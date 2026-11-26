//! Model-based proptest state machine for a single-token escrow's full
//! lifecycle.
//!
//! Covers every single-escrow entrypoint against a real Stellar Asset Contract
//! under strict per-step authorization enforcement (`mock_auths` / `set_auths`,
//! never blanket `mock_all_auths` during execution):
//!
//! * Creation: `create_escrow`, `create_escrow_with_review`,
//!   `create_escrow_with_arbiter_fee` (both valid and invalid terms/actors).
//! * Custody & cancellation: `deposit` (including missing nested token auth
//!   and insufficient buyer balance), `cancel`.
//! * Direct releases: `release`, `release_partial`.
//! * Review-window flow: `request_release`, `approve_release`,
//!   `execute_release`.
//! * Refunds: `refund` (seller pre-deadline, buyer at/after deadline, blocked
//!   by pending request), `refund_expired` (permissionless strictly after
//!   deadline, blocked by pending request).
//! * Disputes & settlement: `dispute`, `resolve`, `resolve_split` (including
//!   arbiter fee deduction and floor-to-zero split rounding).
//! * Extensions & keeper: `extend_deadline` (before/after deadline, overflow,
//!   non-increasing rejection, preserving pending requests), `touch_ttl`.
//!
//! An independent pure-Rust model predicts each step's exact outcome
//! (`Ok`, `OkU64`, exact `ForgeError`, or host `AuthAbort`) and resulting
//! state. After every step the harness compares on-chain storage, views
//! (`get_escrow`, `get_status`, `get_release_request`, `review_period`,
//! `get_arbiter_fee_bps`, `preflight_deposit`, `get_action_summary`), real SAC
//! balances, terminal-state immutability, and `escrows_for_participant`
//! indexes against the model.

extern crate std;

use crate::{
    ActionIneligibilityReason, DepositPreflightStatus, Escrow, EscrowAction, EscrowData,
    EscrowStatus, ReleaseRequest, SorobanForgeEscrowClient, DEFAULT_REVIEW_PERIOD,
    MAX_ARBITER_FEE_BPS, MAX_REVIEW_PERIOD, MIN_REVIEW_PERIOD,
};
use core::fmt;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, EnvTestConfig, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Vec as SorobanVec};
use std::collections::BTreeMap;
use std::format;
use std::string::String;
use std::vec;
use std::vec::Vec;

const START_TIMESTAMP: u64 = 1_000_000;
const DAY: u64 = 24 * 60 * 60;
const MAX_TIMEOUT: u64 = 14 * DAY;
const MAX_AMOUNT: i128 = 1_000_000_000;

/// Number of state-machine cases run in CI and locally (>= 128).
const LIFECYCLE_CASES: u32 = 160;
/// Maximum number of steps per generated sequence.
const MAX_STEPS: usize = 40;
/// Fixed seed so the exact same sequences run on every machine and in CI.
const LIFECYCLE_SEED: u64 = 0x4553_4352_4f57_4c43; // "ESCROWLC"

// ---------------------------------------------------------------------------
// Actors, targets, and step language (designed for clean shrinking)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Actor {
    Buyer,
    Seller,
    Arbiter,
    Outsider,
}

impl Actor {
    const ALL: [Self; 4] = [Self::Buyer, Self::Seller, Self::Arbiter, Self::Outsider];

    const fn is_participant(self) -> bool {
        matches!(self, Self::Buyer | Self::Seller | Self::Arbiter)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CreateVariant {
    Standard,
    WithReview(u64),
    WithArbiterFee(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InitialEscrow {
    variant: CreateVariant,
    amount: i128,
    timeout: u64,
    /// Extra balance minted to the buyer beyond the initial escrow's amount.
    /// Keeping this bounded means later `Create*` escrows may either be
    /// fundable or hit real insufficient-balance `TokenTransferFailed`.
    extra_buyer_mint: i128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EscrowTarget {
    /// Escrow id 1 (the primary escrow created at setup).
    Primary,
    /// The most recently created escrow id.
    Latest,
    /// A literal id (e.g. an earlier escrow or an unknown id like 999).
    Id(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AmountSpec {
    /// Literal amount (covers negative, zero, and small edge numbers).
    Exact(i128),
    /// Basis points (`1..=10_000`) of the target's current remaining balance
    /// (at least 1 when remaining > 0; 1 when remaining == 0).
    FractionOfRemainingBps(u16),
    /// Full remaining balance (or 1 when remaining == 0).
    FullRemaining,
    /// `remaining + 1` (always exceeds remaining).
    OverRemaining,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimeoutSpec {
    /// `current_timeout + delta` (strictly later when `delta > 0`).
    Add(u64),
    /// Equal to current timeout (rejected with `InvalidInput`).
    Same,
    /// One second shorter than current timeout (rejected with `InvalidInput`).
    Shorter,
    /// `u64::MAX` (overflows `created_at + new_timeout` -> `ArithmeticOverflow`).
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimeJump {
    /// Advance ledger time by `seconds`.
    Seconds(u64),
    /// Move to `deadline - 1` (no-op if already at or past it).
    JustBeforeDeadline,
    /// Move to exact `deadline` (no-op if already past it).
    AtDeadline,
    /// Move to `deadline + 1` (no-op if already past it).
    JustAfterDeadline,
    /// Move to `pending.release_at - 1` (no-op if no pending request or past).
    JustBeforeReleaseAt,
    /// Move to exact `pending.release_at` (no-op if no pending request or past).
    AtReleaseAt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    CreateEscrow {
        actor: Actor,
        amount: i128,
        timeout: u64,
    },
    CreateEscrowWithReview {
        actor: Actor,
        amount: i128,
        timeout: u64,
        review_period: u64,
    },
    CreateEscrowWithArbiterFee {
        actor: Actor,
        amount: i128,
        timeout: u64,
        arbiter_fee_bps: u32,
    },
    Deposit {
        actor: Actor,
        target: EscrowTarget,
        authorize_transfer: bool,
    },
    Cancel {
        actor: Actor,
        target: EscrowTarget,
    },
    Release {
        actor: Actor,
        target: EscrowTarget,
    },
    ReleasePartial {
        actor: Actor,
        target: EscrowTarget,
        amount: AmountSpec,
    },
    RequestRelease {
        actor: Actor,
        target: EscrowTarget,
        amount: AmountSpec,
    },
    ApproveRelease {
        actor: Actor,
        target: EscrowTarget,
    },
    ExecuteRelease {
        target: EscrowTarget,
    },
    Refund {
        actor: Actor,
        target: EscrowTarget,
    },
    RefundExpired {
        target: EscrowTarget,
    },
    Dispute {
        actor: Actor,
        claimant: Actor,
        target: EscrowTarget,
    },
    Resolve {
        actor: Actor,
        target: EscrowTarget,
        in_favor_of_seller: bool,
    },
    ResolveSplit {
        actor: Actor,
        target: EscrowTarget,
        seller_bps: u32,
    },
    ExtendDeadline {
        actor: Actor,
        target: EscrowTarget,
        timeout: TimeoutSpec,
    },
    TouchTtl {
        target: EscrowTarget,
    },
    AdvanceTime {
        target: EscrowTarget,
        jump: TimeJump,
    },
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateEscrow {
                actor,
                amount,
                timeout,
            } => write!(
                f,
                "create_escrow(actor={actor:?}, amount={amount}, timeout={timeout})"
            ),
            Self::CreateEscrowWithReview {
                actor,
                amount,
                timeout,
                review_period,
            } => write!(
                f,
                "create_escrow_with_review(actor={actor:?}, amount={amount}, timeout={timeout}, review={review_period})"
            ),
            Self::CreateEscrowWithArbiterFee {
                actor,
                amount,
                timeout,
                arbiter_fee_bps,
            } => write!(
                f,
                "create_escrow_with_arbiter_fee(actor={actor:?}, amount={amount}, timeout={timeout}, fee_bps={arbiter_fee_bps})"
            ),
            Self::Deposit {
                actor,
                target,
                authorize_transfer,
            } => write!(
                f,
                "deposit(actor={actor:?}, target={target:?}, auth_transfer={authorize_transfer})"
            ),
            Self::Cancel { actor, target } => {
                write!(f, "cancel(actor={actor:?}, target={target:?})")
            }
            Self::Release { actor, target } => {
                write!(f, "release(actor={actor:?}, target={target:?})")
            }
            Self::ReleasePartial {
                actor,
                target,
                amount,
            } => write!(
                f,
                "release_partial(actor={actor:?}, target={target:?}, amount={amount:?})"
            ),
            Self::RequestRelease {
                actor,
                target,
                amount,
            } => write!(
                f,
                "request_release(actor={actor:?}, target={target:?}, amount={amount:?})"
            ),
            Self::ApproveRelease { actor, target } => {
                write!(f, "approve_release(actor={actor:?}, target={target:?})")
            }
            Self::ExecuteRelease { target } => write!(f, "execute_release(target={target:?})"),
            Self::Refund { actor, target } => {
                write!(f, "refund(actor={actor:?}, target={target:?})")
            }
            Self::RefundExpired { target } => write!(f, "refund_expired(target={target:?})"),
            Self::Dispute {
                actor,
                claimant,
                target,
            } => write!(
                f,
                "dispute(actor={actor:?}, claimant={claimant:?}, target={target:?})"
            ),
            Self::Resolve {
                actor,
                target,
                in_favor_of_seller,
            } => write!(
                f,
                "resolve(actor={actor:?}, target={target:?}, in_favor_of_seller={in_favor_of_seller})"
            ),
            Self::ResolveSplit {
                actor,
                target,
                seller_bps,
            } => write!(
                f,
                "resolve_split(actor={actor:?}, target={target:?}, seller_bps={seller_bps})"
            ),
            Self::ExtendDeadline {
                actor,
                target,
                timeout,
            } => write!(
                f,
                "extend_deadline(actor={actor:?}, target={target:?}, timeout={timeout:?})"
            ),
            Self::TouchTtl { target } => write!(f, "touch_ttl(target={target:?})"),
            Self::AdvanceTime { target, jump } => {
                write!(f, "advance_time(target={target:?}, jump={jump:?})")
            }
        }
    }
}

fn format_trace(initial: InitialEscrow, steps: &[Step]) -> String {
    let mut out = format!("initial: {initial:?}\n");
    for (idx, step) in steps.iter().enumerate() {
        out.push_str(&format!("  {:>2}. {step}\n", idx + 1));
    }
    out
}

// ---------------------------------------------------------------------------
// Exact predicted outcome
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepOutcome {
    Ok,
    OkU64(u64),
    ContractError(ForgeError),
    AuthAbort,
}

fn classify_unit<E: fmt::Debug>(
    res: Result<Result<(), E>, Result<ForgeError, InvokeError>>,
) -> StepOutcome {
    match res {
        Ok(Ok(())) => StepOutcome::Ok,
        Err(Ok(err)) => StepOutcome::ContractError(err),
        Err(Err(InvokeError::Abort)) => StepOutcome::AuthAbort,
        other => panic!("unexpected invocation result: {other:?}"),
    }
}

fn classify_u64<E: fmt::Debug>(
    res: Result<Result<u64, E>, Result<ForgeError, InvokeError>>,
) -> StepOutcome {
    match res {
        Ok(Ok(val)) => StepOutcome::OkU64(val),
        Err(Ok(err)) => StepOutcome::ContractError(err),
        Err(Err(InvokeError::Abort)) => StepOutcome::AuthAbort,
        other => panic!("unexpected u64 invocation result: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust reference model
// ---------------------------------------------------------------------------

/// Frozen snapshot captured the instant an escrow enters a terminal state,
/// used to assert that no subsequent step ever mutates it again.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalSnapshot {
    status: EscrowStatus,
    amount: i128,
    released: i128,
    timeout: u64,
    created_at: u64,
    pending_request: Option<ReleaseRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EscrowModel {
    id: u64,
    amount: i128,
    /// Mirrors `EscrowData::released`: cumulative payouts while Funded/Disputed,
    /// equal to `amount` once Completed or Refunded, `0` while Pending/Cancelled.
    contract_released: i128,
    timeout: u64,
    created_at: u64,
    status: EscrowStatus,
    review_period: u64,
    arbiter_fee_bps: u32,
    pending_request: Option<ReleaseRequest>,
    /// True once `deposit` succeeded on this escrow.
    was_deposited: bool,
    /// Actual cumulative tokens paid to the seller from this escrow.
    seller_received: i128,
    /// Actual cumulative tokens refunded to the buyer from this escrow.
    buyer_refunded: i128,
    /// Actual cumulative arbiter fees paid to the arbiter from this escrow.
    arbiter_received: i128,
    /// Snapshot captured when this escrow first became terminal.
    terminal_snapshot: Option<TerminalSnapshot>,
}

impl EscrowModel {
    const fn new(
        id: u64,
        amount: i128,
        timeout: u64,
        created_at: u64,
        review_period: u64,
        arbiter_fee_bps: u32,
    ) -> Self {
        Self {
            id,
            amount,
            contract_released: 0,
            timeout,
            created_at,
            status: EscrowStatus::Pending,
            review_period,
            arbiter_fee_bps,
            pending_request: None,
            was_deposited: false,
            seller_received: 0,
            buyer_refunded: 0,
            arbiter_received: 0,
            terminal_snapshot: None,
        }
    }

    /// Mirrors `EscrowData::remaining()` (`amount - released`).
    const fn contract_remaining(&self) -> i128 {
        self.amount - self.contract_released
    }

    /// Tokens actually held in custody by the contract for this escrow.
    const fn funds_held(&self) -> i128 {
        match self.status {
            EscrowStatus::Funded | EscrowStatus::Disputed => self.contract_remaining(),
            EscrowStatus::Pending
            | EscrowStatus::Completed
            | EscrowStatus::Refunded
            | EscrowStatus::Cancelled => 0,
        }
    }

    const fn deposited_amount(&self) -> i128 {
        if self.was_deposited {
            self.amount
        } else {
            0
        }
    }

    const fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            EscrowStatus::Completed | EscrowStatus::Refunded | EscrowStatus::Cancelled
        )
    }

    fn record_terminal_if_needed(&mut self) {
        if self.is_terminal() && self.terminal_snapshot.is_none() {
            self.terminal_snapshot = Some(TerminalSnapshot {
                status: self.status.clone(),
                amount: self.amount,
                released: self.contract_released,
                timeout: self.timeout,
                created_at: self.created_at,
                pending_request: self.pending_request.clone(),
            });
        }
    }
}

fn model_split_amount(amount: i128, bps: u32) -> Result<(i128, i128), ForgeError> {
    let denom = 10_000_i128;
    let bps_i = i128::from(bps);
    let whole = amount
        .checked_div(denom)
        .ok_or(ForgeError::ArithmeticOverflow)?;
    let rem = amount
        .checked_rem(denom)
        .ok_or(ForgeError::ArithmeticOverflow)?;
    let share = whole
        .checked_mul(bps_i)
        .and_then(|v| {
            rem.checked_mul(bps_i)
                .and_then(|frac| v.checked_add(frac / denom))
        })
        .ok_or(ForgeError::ArithmeticOverflow)?;
    let rest = amount
        .checked_sub(share)
        .ok_or(ForgeError::ArithmeticOverflow)?;
    Ok((share, rest))
}

#[derive(Clone, Debug)]
struct StateModel {
    now: u64,
    next_id: u64,
    initial_buyer_mint: i128,
    buyer_balance: i128,
    escrows: BTreeMap<u64, EscrowModel>,
}

impl StateModel {
    const fn new(now: u64, initial_buyer_mint: i128) -> Self {
        Self {
            now,
            next_id: 1,
            initial_buyer_mint,
            buyer_balance: initial_buyer_mint,
            escrows: BTreeMap::new(),
        }
    }

    fn resolve_target(&self, target: EscrowTarget) -> u64 {
        match target {
            EscrowTarget::Primary => 1,
            EscrowTarget::Latest => self.escrows.keys().next_back().copied().unwrap_or(1),
            EscrowTarget::Id(id) => id,
        }
    }

    fn resolve_amount(&self, target_id: u64, spec: AmountSpec) -> i128 {
        let remaining = self
            .escrows
            .get(&target_id)
            .map_or(0, EscrowModel::contract_remaining);
        match spec {
            AmountSpec::Exact(v) => v,
            AmountSpec::FractionOfRemainingBps(bps) => {
                if remaining <= 0 {
                    1
                } else {
                    ((remaining * i128::from(bps)) / 10_000)
                        .max(1)
                        .min(remaining)
                }
            }
            AmountSpec::FullRemaining => remaining.max(1),
            AmountSpec::OverRemaining => remaining.saturating_add(1),
        }
    }

    fn resolve_timeout(&self, target_id: u64, spec: TimeoutSpec) -> u64 {
        let current = self.escrows.get(&target_id).map_or(100, |e| e.timeout);
        match spec {
            TimeoutSpec::Add(delta) => current.saturating_add(delta),
            TimeoutSpec::Same => current,
            TimeoutSpec::Shorter => current.saturating_sub(1),
            TimeoutSpec::Overflow => u64::MAX,
        }
    }

    fn resolve_next_timestamp(&self, target_id: u64, jump: TimeJump) -> u64 {
        let target_ts = match jump {
            TimeJump::Seconds(delta) => self.now.saturating_add(delta),
            TimeJump::JustBeforeDeadline => self
                .escrows
                .get(&target_id)
                .and_then(|e| e.created_at.checked_add(e.timeout))
                .map_or(self.now, |d| d.saturating_sub(1)),
            TimeJump::AtDeadline => self
                .escrows
                .get(&target_id)
                .and_then(|e| e.created_at.checked_add(e.timeout))
                .unwrap_or(self.now),
            TimeJump::JustAfterDeadline => self
                .escrows
                .get(&target_id)
                .and_then(|e| e.created_at.checked_add(e.timeout))
                .map_or(self.now, |d| d.saturating_add(1)),
            TimeJump::JustBeforeReleaseAt => self
                .escrows
                .get(&target_id)
                .and_then(|e| e.pending_request.as_ref())
                .map_or(self.now, |r| r.release_at.saturating_sub(1)),
            TimeJump::AtReleaseAt => self
                .escrows
                .get(&target_id)
                .and_then(|e| e.pending_request.as_ref())
                .map_or(self.now, |r| r.release_at),
        };
        // Ledger timestamps are monotonically non-decreasing.
        target_ts.max(self.now)
    }

    fn apply_create(
        &mut self,
        actor: Actor,
        amount: i128,
        timeout: u64,
        variant: CreateVariant,
    ) -> StepOutcome {
        let (review_period, arbiter_fee_bps) = match variant {
            CreateVariant::Standard => (DEFAULT_REVIEW_PERIOD, 0),
            CreateVariant::WithReview(review_period) => {
                if !(MIN_REVIEW_PERIOD..=MAX_REVIEW_PERIOD).contains(&review_period) {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                (review_period, 0)
            }
            CreateVariant::WithArbiterFee(arbiter_fee_bps) => {
                if arbiter_fee_bps > MAX_ARBITER_FEE_BPS {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                (DEFAULT_REVIEW_PERIOD, arbiter_fee_bps)
            }
        };
        if amount <= 0 || timeout == 0 {
            return StepOutcome::ContractError(ForgeError::InvalidInput);
        }
        if actor != Actor::Buyer {
            return StepOutcome::AuthAbort;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.escrows.insert(
            id,
            EscrowModel::new(
                id,
                amount,
                timeout,
                self.now,
                review_period,
                arbiter_fee_bps,
            ),
        );
        StepOutcome::OkU64(id)
    }

    fn apply_pay_request(&mut self, id: u64) -> StepOutcome {
        let Some(e) = self.escrows.get_mut(&id) else {
            return StepOutcome::ContractError(ForgeError::NotFound);
        };
        let Some(req) = e.pending_request.clone() else {
            return StepOutcome::ContractError(ForgeError::NotFound);
        };
        if e.status != EscrowStatus::Funded {
            return StepOutcome::ContractError(ForgeError::InvalidInput);
        }
        let remaining = e.contract_remaining();
        if req.amount <= 0 || req.amount > remaining {
            return StepOutcome::ContractError(ForgeError::InvalidInput);
        }
        e.contract_released += req.amount;
        e.seller_received += req.amount;
        if req.amount == remaining {
            e.status = EscrowStatus::Completed;
        }
        e.pending_request = None;
        e.record_terminal_if_needed();
        StepOutcome::Ok
    }

    /// Predict the outcome of `step` and mutate the model accordingly.
    fn apply(&mut self, step: Step) -> StepOutcome {
        match step {
            Step::CreateEscrow {
                actor,
                amount,
                timeout,
            } => self.apply_create(actor, amount, timeout, CreateVariant::Standard),
            Step::CreateEscrowWithReview {
                actor,
                amount,
                timeout,
                review_period,
            } => self.apply_create(
                actor,
                amount,
                timeout,
                CreateVariant::WithReview(review_period),
            ),
            Step::CreateEscrowWithArbiterFee {
                actor,
                amount,
                timeout,
                arbiter_fee_bps,
            } => self.apply_create(
                actor,
                amount,
                timeout,
                CreateVariant::WithArbiterFee(arbiter_fee_bps),
            ),
            Step::Deposit {
                actor,
                target,
                authorize_transfer,
            } => {
                let id = self.resolve_target(target);
                let Some(e) = self.escrows.get(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                // `deposit` checks buyer auth BEFORE status.
                if actor != Actor::Buyer {
                    return StepOutcome::AuthAbort;
                }
                if e.status != EscrowStatus::Pending {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if !authorize_transfer || self.buyer_balance < e.amount {
                    return StepOutcome::ContractError(ForgeError::TokenTransferFailed);
                }
                let amount = e.amount;
                self.buyer_balance -= amount;
                let e_mut = self.escrows.get_mut(&id).expect("escrow exists");
                e_mut.status = EscrowStatus::Funded;
                e_mut.was_deposited = true;
                StepOutcome::Ok
            }
            Step::Cancel { actor, target } => {
                let id = self.resolve_target(target);
                let Some(e) = self.escrows.get_mut(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Pending {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if actor != Actor::Buyer {
                    return StepOutcome::AuthAbort;
                }
                e.status = EscrowStatus::Cancelled;
                e.record_terminal_if_needed();
                StepOutcome::Ok
            }
            Step::Release { actor, target } => {
                let id = self.resolve_target(target);
                let Some(e) = self.escrows.get_mut(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                // `release` checks buyer auth BEFORE status.
                if actor != Actor::Buyer {
                    return StepOutcome::AuthAbort;
                }
                if e.status != EscrowStatus::Funded {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                let remaining = e.contract_remaining();
                e.seller_received += remaining;
                e.contract_released = e.amount;
                e.status = EscrowStatus::Completed;
                e.pending_request = None;
                e.record_terminal_if_needed();
                StepOutcome::Ok
            }
            Step::ReleasePartial {
                actor,
                target,
                amount,
            } => {
                let id = self.resolve_target(target);
                let resolved_amount = self.resolve_amount(id, amount);
                let Some(e) = self.escrows.get_mut(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                // `release_partial` checks buyer auth BEFORE status/amount.
                if actor != Actor::Buyer {
                    return StepOutcome::AuthAbort;
                }
                if e.status != EscrowStatus::Funded {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                let remaining = e.contract_remaining();
                if resolved_amount <= 0 || resolved_amount > remaining {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                e.seller_received += resolved_amount;
                e.contract_released += resolved_amount;
                if resolved_amount == remaining {
                    e.status = EscrowStatus::Completed;
                }
                e.pending_request = None;
                e.record_terminal_if_needed();
                StepOutcome::Ok
            }
            Step::RequestRelease {
                actor,
                target,
                amount,
            } => {
                let id = self.resolve_target(target);
                let resolved_amount = self.resolve_amount(id, amount);
                let now = self.now;
                let Some(e) = self.escrows.get_mut(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Funded {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if resolved_amount <= 0 || resolved_amount > e.contract_remaining() {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if actor != Actor::Seller {
                    return StepOutcome::AuthAbort;
                }
                let Some(deadline) = e.created_at.checked_add(e.timeout) else {
                    return StepOutcome::ContractError(ForgeError::ArithmeticOverflow);
                };
                if e.pending_request.is_some() {
                    return StepOutcome::ContractError(ForgeError::ReleasePending);
                }
                if now > deadline {
                    return StepOutcome::ContractError(ForgeError::DeadlineReached);
                }
                let Some(release_at) = now.checked_add(e.review_period) else {
                    return StepOutcome::ContractError(ForgeError::ArithmeticOverflow);
                };
                e.pending_request = Some(ReleaseRequest {
                    amount: resolved_amount,
                    requested_at: now,
                    release_at,
                });
                StepOutcome::OkU64(release_at)
            }
            Step::ApproveRelease { actor, target } => {
                let id = self.resolve_target(target);
                let has_req = self
                    .escrows
                    .get(&id)
                    .is_some_and(|e| e.pending_request.is_some());
                if !has_req {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                }
                if actor != Actor::Buyer {
                    return StepOutcome::AuthAbort;
                }
                self.apply_pay_request(id)
            }
            Step::ExecuteRelease { target } => {
                let id = self.resolve_target(target);
                let Some(req) = self
                    .escrows
                    .get(&id)
                    .and_then(|e| e.pending_request.as_ref())
                else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if self.now < req.release_at {
                    return StepOutcome::ContractError(ForgeError::DeadlineReached);
                }
                self.apply_pay_request(id)
            }
            Step::Refund { actor, target } => {
                let id = self.resolve_target(target);
                let now = self.now;
                let Some(e) = self.escrows.get(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Funded {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                let Some(deadline) = e.created_at.checked_add(e.timeout) else {
                    return StepOutcome::ContractError(ForgeError::ArithmeticOverflow);
                };
                let authorizer = if now >= deadline {
                    Actor::Buyer
                } else {
                    Actor::Seller
                };
                if authorizer == Actor::Buyer && e.pending_request.is_some() {
                    return StepOutcome::ContractError(ForgeError::ReleasePending);
                }
                if actor != authorizer {
                    return StepOutcome::AuthAbort;
                }
                let remaining = e.contract_remaining();
                self.buyer_balance += remaining;
                let e_mut = self.escrows.get_mut(&id).expect("escrow exists");
                e_mut.buyer_refunded += remaining;
                e_mut.contract_released = e_mut.amount;
                e_mut.status = EscrowStatus::Refunded;
                e_mut.pending_request = None;
                e_mut.record_terminal_if_needed();
                StepOutcome::Ok
            }
            Step::RefundExpired { target } => {
                let id = self.resolve_target(target);
                let now = self.now;
                let Some(e) = self.escrows.get(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Funded {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                let Some(deadline) = e.created_at.checked_add(e.timeout) else {
                    return StepOutcome::ContractError(ForgeError::ArithmeticOverflow);
                };
                if now <= deadline {
                    return StepOutcome::ContractError(ForgeError::DeadlineReached);
                }
                if e.pending_request.is_some() {
                    return StepOutcome::ContractError(ForgeError::ReleasePending);
                }
                let remaining = e.contract_remaining();
                self.buyer_balance += remaining;
                let e_mut = self.escrows.get_mut(&id).expect("escrow exists");
                e_mut.buyer_refunded += remaining;
                e_mut.contract_released = e_mut.amount;
                e_mut.status = EscrowStatus::Refunded;
                e_mut.record_terminal_if_needed();
                StepOutcome::Ok
            }
            Step::Dispute {
                actor,
                claimant,
                target,
            } => {
                let id = self.resolve_target(target);
                let Some(e) = self.escrows.get_mut(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Funded {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if claimant != Actor::Buyer && claimant != Actor::Seller {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if actor != claimant {
                    return StepOutcome::AuthAbort;
                }
                e.status = EscrowStatus::Disputed;
                e.pending_request = None;
                StepOutcome::Ok
            }
            Step::Resolve {
                actor,
                target,
                in_favor_of_seller,
            } => {
                let id = self.resolve_target(target);
                let Some(e) = self.escrows.get(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Disputed {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if actor != Actor::Arbiter {
                    return StepOutcome::AuthAbort;
                }
                let remaining = e.contract_remaining();
                let (arbiter_fee, award) = match model_split_amount(remaining, e.arbiter_fee_bps) {
                    Ok(pair) => pair,
                    Err(err) => return StepOutcome::ContractError(err),
                };
                if !in_favor_of_seller {
                    self.buyer_balance += award;
                }
                let e_mut = self.escrows.get_mut(&id).expect("escrow exists");
                e_mut.arbiter_received += arbiter_fee;
                if in_favor_of_seller {
                    e_mut.seller_received += award;
                    e_mut.status = EscrowStatus::Completed;
                } else {
                    e_mut.buyer_refunded += award;
                    e_mut.status = EscrowStatus::Refunded;
                }
                e_mut.contract_released = e_mut.amount;
                e_mut.record_terminal_if_needed();
                StepOutcome::Ok
            }
            Step::ResolveSplit {
                actor,
                target,
                seller_bps,
            } => {
                let id = self.resolve_target(target);
                let Some(e) = self.escrows.get(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Disputed {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if seller_bps > 10_000 {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if actor != Actor::Arbiter {
                    return StepOutcome::AuthAbort;
                }
                let remaining = e.contract_remaining();
                let (arbiter_fee, distributable) =
                    match model_split_amount(remaining, e.arbiter_fee_bps) {
                        Ok(pair) => pair,
                        Err(err) => return StepOutcome::ContractError(err),
                    };
                let (seller_share, buyer_share) =
                    match model_split_amount(distributable, seller_bps) {
                        Ok(pair) => pair,
                        Err(err) => return StepOutcome::ContractError(err),
                    };
                self.buyer_balance += buyer_share;
                let e_mut = self.escrows.get_mut(&id).expect("escrow exists");
                e_mut.arbiter_received += arbiter_fee;
                e_mut.seller_received += seller_share;
                e_mut.buyer_refunded += buyer_share;
                e_mut.contract_released = e_mut.amount;
                e_mut.status = if seller_share == 0 {
                    EscrowStatus::Refunded
                } else {
                    EscrowStatus::Completed
                };
                e_mut.record_terminal_if_needed();
                StepOutcome::Ok
            }
            Step::ExtendDeadline {
                actor,
                target,
                timeout,
            } => {
                let id = self.resolve_target(target);
                let new_timeout = self.resolve_timeout(id, timeout);
                let Some(e) = self.escrows.get_mut(&id) else {
                    return StepOutcome::ContractError(ForgeError::NotFound);
                };
                if e.status != EscrowStatus::Funded || new_timeout <= e.timeout {
                    return StepOutcome::ContractError(ForgeError::InvalidInput);
                }
                if e.created_at.checked_add(e.timeout).is_none()
                    || e.created_at.checked_add(new_timeout).is_none()
                {
                    return StepOutcome::ContractError(ForgeError::ArithmeticOverflow);
                }
                if actor != Actor::Buyer {
                    return StepOutcome::AuthAbort;
                }
                e.timeout = new_timeout;
                StepOutcome::Ok
            }
            Step::TouchTtl { target } => {
                let id = self.resolve_target(target);
                if self.escrows.contains_key(&id) {
                    StepOutcome::Ok
                } else {
                    StepOutcome::ContractError(ForgeError::NotFound)
                }
            }
            Step::AdvanceTime { target, jump } => {
                let id = self.resolve_target(target);
                self.now = self.resolve_next_timestamp(id, jump);
                StepOutcome::Ok
            }
        }
    }

    /// Predict the expected `get_action_summary` eligibility for one escrow
    /// and caller.
    fn expected_action_eligibility(
        &self,
        e: &EscrowModel,
        caller: Actor,
    ) -> [(EscrowAction, bool, ActionIneligibilityReason); 12] {
        use ActionIneligibilityReason as R;
        use EscrowAction as A;

        let deadline = e.created_at + e.timeout;
        let has_req = e.pending_request.is_some();

        let (dep_ok, dep_r) = if e.status != EscrowStatus::Pending {
            (false, R::WrongStatus)
        } else if caller != Actor::Buyer {
            (false, R::WrongRole)
        } else {
            (true, R::Eligible)
        };

        let (req_ok, req_r) = if e.status != EscrowStatus::Funded {
            (false, R::WrongStatus)
        } else if caller != Actor::Seller {
            (false, R::WrongRole)
        } else if has_req {
            (false, R::ReleasePending)
        } else if self.now > deadline {
            (false, R::AfterDeadline)
        } else {
            (true, R::Eligible)
        };

        let (app_ok, app_r) = if e.status != EscrowStatus::Funded {
            (false, R::WrongStatus)
        } else if !has_req {
            (false, R::NoPendingRequest)
        } else if caller != Actor::Buyer {
            (false, R::WrongRole)
        } else {
            (true, R::Eligible)
        };

        let (exe_ok, exe_r) = if e.status != EscrowStatus::Funded {
            (false, R::WrongStatus)
        } else {
            match e.pending_request.as_ref() {
                None => (false, R::NoPendingRequest),
                Some(r) if self.now < r.release_at => (false, R::ReviewWindowNotEnded),
                Some(_) => (true, R::Eligible),
            }
        };

        let (rel_ok, rel_r) = if e.status != EscrowStatus::Funded {
            (false, R::WrongStatus)
        } else if caller != Actor::Buyer {
            (false, R::WrongRole)
        } else {
            (true, R::Eligible)
        };

        let (ref_ok, ref_r) = if e.status != EscrowStatus::Funded {
            (false, R::WrongStatus)
        } else {
            let refund_party = if self.now >= deadline {
                Actor::Buyer
            } else {
                Actor::Seller
            };
            if caller != refund_party {
                (false, R::WrongRole)
            } else if refund_party == Actor::Buyer && has_req {
                (false, R::ReleasePending)
            } else {
                (true, R::Eligible)
            }
        };

        let (exp_ok, exp_r) = if e.status != EscrowStatus::Funded {
            (false, R::WrongStatus)
        } else if self.now <= deadline {
            (false, R::BeforeDeadline)
        } else if has_req {
            (false, R::ReleasePending)
        } else {
            (true, R::Eligible)
        };

        let (dis_ok, dis_r) = if e.status != EscrowStatus::Funded {
            (false, R::WrongStatus)
        } else if caller != Actor::Buyer && caller != Actor::Seller {
            (false, R::WrongRole)
        } else {
            (true, R::Eligible)
        };

        let (res_ok, res_r) = if e.status != EscrowStatus::Disputed {
            (false, R::WrongStatus)
        } else if caller != Actor::Arbiter {
            (false, R::WrongRole)
        } else {
            (true, R::Eligible)
        };

        [
            (A::Deposit, dep_ok, dep_r.clone()),
            (A::RequestRelease, req_ok, req_r),
            (A::ApproveRelease, app_ok, app_r),
            (A::ExecuteRelease, exe_ok, exe_r),
            (A::Release, rel_ok, rel_r.clone()),
            (A::ReleasePartial, rel_ok, rel_r),
            (A::Refund, ref_ok, ref_r),
            (A::RefundExpired, exp_ok, exp_r),
            (A::Dispute, dis_ok, dis_r),
            (A::Resolve, res_ok, res_r.clone()),
            (A::ResolveSplit, res_ok, res_r),
            (A::Cancel, dep_ok, dep_r),
        ]
    }
}

// ---------------------------------------------------------------------------
// On-chain harness executing each step under strict per-actor authorization
// ---------------------------------------------------------------------------

struct World {
    env: Env,
    token: Address,
    contract: Address,
    buyer: Address,
    seller: Address,
    arbiter: Address,
    outsider: Address,
}

impl World {
    fn new(initial_buyer_mint: i128) -> Self {
        let env = Env::new_with_config(EnvTestConfig {
            capture_snapshot_at_drop: false,
        });
        env.mock_all_auths();
        env.ledger().set_timestamp(START_TIMESTAMP);

        let admin = Address::generate(&env);
        let token = env.register_stellar_asset_contract_v2(admin).address();
        let contract = env.register(Escrow, ());
        let buyer = Address::generate(&env);
        let seller = Address::generate(&env);
        let arbiter = Address::generate(&env);
        let outsider = Address::generate(&env);

        if initial_buyer_mint > 0 {
            StellarAssetClient::new(&env, &token).mint(&buyer, &initial_buyer_mint);
        }
        // Disable blanket mocking immediately after minting: every contract
        // step arms its own exact per-actor MockAuth or empty auth envelope.
        env.set_auths(&[]);

        Self {
            env,
            token,
            contract,
            buyer,
            seller,
            arbiter,
            outsider,
        }
    }

    fn client(&self) -> SorobanForgeEscrowClient<'_> {
        SorobanForgeEscrowClient::new(&self.env, &self.contract)
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }

    const fn addr(&self, actor: Actor) -> &Address {
        match actor {
            Actor::Buyer => &self.buyer,
            Actor::Seller => &self.seller,
            Actor::Arbiter => &self.arbiter,
            Actor::Outsider => &self.outsider,
        }
    }

    fn arm_simple_auth(
        &self,
        actor: Actor,
        fn_name: &'static str,
        args: SorobanVec<soroban_sdk::Val>,
    ) {
        self.env.mock_auths(&[MockAuth {
            address: self.addr(actor),
            invoke: &MockAuthInvoke {
                contract: &self.contract,
                fn_name,
                args,
                sub_invokes: &[],
            },
        }]);
    }

    /// Execute one `Step` on the contract, using `model` only to resolve
    /// relative parameter specs (`EscrowTarget`, `AmountSpec`, `TimeoutSpec`,
    /// `TimeJump`) identically to what `model.apply(step)` will see.
    fn execute_step(&self, model: &StateModel, step: Step) -> StepOutcome {
        let client = self.client();
        match step {
            Step::CreateEscrow {
                actor,
                amount,
                timeout,
            } => {
                self.arm_simple_auth(
                    actor,
                    "create_escrow",
                    (
                        &self.buyer,
                        &self.seller,
                        &self.arbiter,
                        &self.token,
                        amount,
                        timeout,
                    )
                        .into_val(&self.env),
                );
                classify_u64(client.try_create_escrow(
                    &self.buyer,
                    &self.seller,
                    &self.arbiter,
                    &self.token,
                    &amount,
                    &timeout,
                ))
            }
            Step::CreateEscrowWithReview {
                actor,
                amount,
                timeout,
                review_period,
            } => {
                self.arm_simple_auth(
                    actor,
                    "create_escrow_with_review",
                    (
                        &self.buyer,
                        &self.seller,
                        &self.arbiter,
                        &self.token,
                        amount,
                        timeout,
                        review_period,
                    )
                        .into_val(&self.env),
                );
                classify_u64(client.try_create_escrow_with_review(
                    &self.buyer,
                    &self.seller,
                    &self.arbiter,
                    &self.token,
                    &amount,
                    &timeout,
                    &review_period,
                ))
            }
            Step::CreateEscrowWithArbiterFee {
                actor,
                amount,
                timeout,
                arbiter_fee_bps,
            } => {
                self.arm_simple_auth(
                    actor,
                    "create_escrow_with_arbiter_fee",
                    (
                        &self.buyer,
                        &self.seller,
                        &self.arbiter,
                        &self.token,
                        amount,
                        timeout,
                        arbiter_fee_bps,
                    )
                        .into_val(&self.env),
                );
                classify_u64(client.try_create_escrow_with_arbiter_fee(
                    &self.buyer,
                    &self.seller,
                    &self.arbiter,
                    &self.token,
                    &amount,
                    &timeout,
                    &arbiter_fee_bps,
                ))
            }
            Step::Deposit {
                actor,
                target,
                authorize_transfer,
            } => {
                let id = model.resolve_target(target);
                let signer = self.addr(actor);
                if authorize_transfer {
                    if let Some(e) = model.escrows.get(&id) {
                        let transfer_args = (self.buyer.clone(), self.contract.clone(), e.amount)
                            .into_val(&self.env);
                        self.env.mock_auths(&[MockAuth {
                            address: signer,
                            invoke: &MockAuthInvoke {
                                contract: &self.contract,
                                fn_name: "deposit",
                                args: (id,).into_val(&self.env),
                                sub_invokes: &[MockAuthInvoke {
                                    contract: &self.token,
                                    fn_name: "transfer",
                                    args: transfer_args,
                                    sub_invokes: &[],
                                }],
                            },
                        }]);
                    } else {
                        self.arm_simple_auth(actor, "deposit", (id,).into_val(&self.env));
                    }
                } else {
                    self.arm_simple_auth(actor, "deposit", (id,).into_val(&self.env));
                }
                classify_unit(client.try_deposit(&id))
            }
            Step::Cancel { actor, target } => {
                let id = model.resolve_target(target);
                self.arm_simple_auth(actor, "cancel", (id,).into_val(&self.env));
                classify_unit(client.try_cancel(&id))
            }
            Step::Release { actor, target } => {
                let id = model.resolve_target(target);
                self.arm_simple_auth(actor, "release", (id,).into_val(&self.env));
                classify_unit(client.try_release(&id))
            }
            Step::ReleasePartial {
                actor,
                target,
                amount,
            } => {
                let id = model.resolve_target(target);
                let resolved_amount = model.resolve_amount(id, amount);
                self.arm_simple_auth(
                    actor,
                    "release_partial",
                    (id, resolved_amount).into_val(&self.env),
                );
                classify_unit(client.try_release_partial(&id, &resolved_amount))
            }
            Step::RequestRelease {
                actor,
                target,
                amount,
            } => {
                let id = model.resolve_target(target);
                let resolved_amount = model.resolve_amount(id, amount);
                self.arm_simple_auth(
                    actor,
                    "request_release",
                    (id, resolved_amount).into_val(&self.env),
                );
                classify_u64(client.try_request_release(&id, &resolved_amount))
            }
            Step::ApproveRelease { actor, target } => {
                let id = model.resolve_target(target);
                self.arm_simple_auth(actor, "approve_release", (id,).into_val(&self.env));
                classify_unit(client.try_approve_release(&id))
            }
            Step::ExecuteRelease { target } => {
                let id = model.resolve_target(target);
                // Permissionless: blank auth envelope proves no signature is needed.
                self.env.set_auths(&[]);
                classify_unit(client.try_execute_release(&id))
            }
            Step::Refund { actor, target } => {
                let id = model.resolve_target(target);
                self.arm_simple_auth(actor, "refund", (id,).into_val(&self.env));
                classify_unit(client.try_refund(&id))
            }
            Step::RefundExpired { target } => {
                let id = model.resolve_target(target);
                // Permissionless: blank auth envelope proves no signature is needed.
                self.env.set_auths(&[]);
                classify_unit(client.try_refund_expired(&id))
            }
            Step::Dispute {
                actor,
                claimant,
                target,
            } => {
                let id = model.resolve_target(target);
                let claimant_addr = self.addr(claimant);
                self.arm_simple_auth(actor, "dispute", (id, claimant_addr).into_val(&self.env));
                classify_unit(client.try_dispute(&id, claimant_addr))
            }
            Step::Resolve {
                actor,
                target,
                in_favor_of_seller,
            } => {
                let id = model.resolve_target(target);
                self.arm_simple_auth(
                    actor,
                    "resolve",
                    (id, in_favor_of_seller).into_val(&self.env),
                );
                classify_unit(client.try_resolve(&id, &in_favor_of_seller))
            }
            Step::ResolveSplit {
                actor,
                target,
                seller_bps,
            } => {
                let id = model.resolve_target(target);
                self.arm_simple_auth(actor, "resolve_split", (id, seller_bps).into_val(&self.env));
                classify_unit(client.try_resolve_split(&id, &seller_bps))
            }
            Step::ExtendDeadline {
                actor,
                target,
                timeout,
            } => {
                let id = model.resolve_target(target);
                let new_timeout = model.resolve_timeout(id, timeout);
                self.arm_simple_auth(
                    actor,
                    "extend_deadline",
                    (id, new_timeout).into_val(&self.env),
                );
                classify_unit(client.try_extend_deadline(&id, &new_timeout))
            }
            Step::TouchTtl { target } => {
                let id = model.resolve_target(target);
                self.env.set_auths(&[]);
                classify_unit(client.try_touch_ttl(&id))
            }
            Step::AdvanceTime { target, jump } => {
                let id = model.resolve_target(target);
                let next_ts = model.resolve_next_timestamp(id, jump);
                self.env.ledger().set_timestamp(next_ts);
                StepOutcome::Ok
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Post-step model comparison and invariant verification
// ---------------------------------------------------------------------------

fn assert_state_and_invariants(
    world: &World,
    model: &StateModel,
    initial: InitialEscrow,
    executed_steps: &[Step],
    focus_id: u64,
    focus_actor: Actor,
) -> Result<(), TestCaseError> {
    let client = world.client();
    let tc = world.token_client();
    let trace = || format_trace(initial, executed_steps);

    prop_assert_eq!(
        world.env.ledger().timestamp(),
        model.now,
        "ledger timestamp diverged from model\n{}",
        trace()
    );

    // Unallocated id must never exist in storage.
    prop_assert!(
        matches!(
            client.try_get_escrow(&model.next_id),
            Err(Ok(ForgeError::NotFound))
        ),
        "unallocated id {} must not exist\n{}",
        model.next_id,
        trace()
    );

    let mut total_deposited = 0_i128;
    let mut total_held = 0_i128;
    let mut total_seller_received = 0_i128;
    let mut total_buyer_refunded = 0_i128;
    let mut total_arbiter_received = 0_i128;
    let mut all_fee_free = true;

    for (&id, expected) in &model.escrows {
        let on_chain: EscrowData = client.get_escrow(&id);
        prop_assert_eq!(
            on_chain.escrow_id,
            expected.id,
            "escrow {} id mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            &on_chain.buyer,
            &world.buyer,
            "escrow {} buyer mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            &on_chain.seller,
            &world.seller,
            "escrow {} seller mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            &on_chain.arbiter,
            &world.arbiter,
            "escrow {} arbiter mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            &on_chain.token,
            &world.token,
            "escrow {} token mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            on_chain.amount,
            expected.amount,
            "escrow {} amount mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            on_chain.released,
            expected.contract_released,
            "escrow {} released mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            on_chain.remaining(),
            expected.contract_remaining(),
            "escrow {} remaining mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            on_chain.timeout,
            expected.timeout,
            "escrow {} timeout mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            &on_chain.status,
            &expected.status,
            "escrow {} status mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            on_chain.created_at,
            expected.created_at,
            "escrow {} created_at mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            &client.get_status(&id),
            &expected.status,
            "escrow {} get_status mismatch\n{}",
            id,
            trace()
        );

        let on_chain_req = client.get_release_request(&id);
        prop_assert_eq!(
            &on_chain_req,
            &expected.pending_request,
            "escrow {} pending release request mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            client.review_period(&id),
            expected.review_period,
            "escrow {} review_period mismatch\n{}",
            id,
            trace()
        );
        prop_assert_eq!(
            client.get_arbiter_fee_bps(&id),
            expected.arbiter_fee_bps,
            "escrow {} arbiter_fee_bps mismatch\n{}",
            id,
            trace()
        );

        // Terminal-state immutability invariant.
        if let Some(snap) = &expected.terminal_snapshot {
            prop_assert_eq!(
                &on_chain.status,
                &snap.status,
                "terminal escrow {} changed status\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                on_chain.amount,
                snap.amount,
                "terminal escrow {} changed amount\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                on_chain.released,
                snap.released,
                "terminal escrow {} changed released\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                on_chain.timeout,
                snap.timeout,
                "terminal escrow {} changed timeout\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                on_chain.created_at,
                snap.created_at,
                "terminal escrow {} changed created_at\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                &on_chain_req,
                &snap.pending_request,
                "terminal escrow {} changed pending_request\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                expected.funds_held(),
                0,
                "terminal escrow {} must hold zero funds\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                on_chain_req,
                None,
                "terminal escrow {} must have no pending request\n{}",
                id,
                trace()
            );
        }

        // Per-escrow value conservation.
        let held = expected.funds_held();
        let deposited = expected.deposited_amount();
        if expected.arbiter_fee_bps == 0 {
            prop_assert_eq!(
                expected.arbiter_received,
                0,
                "fee-free escrow {} paid arbiter\n{}",
                id,
                trace()
            );
            prop_assert_eq!(
                expected.seller_received + expected.buyer_refunded + held,
                deposited,
                "escrow {}: seller_received + buyer_refunded + held != deposited\n{}",
                id,
                trace()
            );
        } else {
            all_fee_free = false;
            prop_assert_eq!(
                expected.seller_received
                    + expected.buyer_refunded
                    + expected.arbiter_received
                    + held,
                deposited,
                "escrow {}: seller + buyer_refunded + arbiter_fee + held != deposited\n{}",
                id,
                trace()
            );
        }

        // Deposit preflight consistency.
        let preflight = client.preflight_deposit(&id);
        let expected_preflight = if expected.status != EscrowStatus::Pending {
            DepositPreflightStatus::InvalidInput
        } else if model.buyer_balance >= expected.amount {
            DepositPreflightStatus::Sufficient
        } else {
            DepositPreflightStatus::InsufficientBalance
        };
        prop_assert_eq!(
            preflight.status,
            expected_preflight,
            "escrow {} preflight_deposit mismatch\n{}",
            id,
            trace()
        );

        total_deposited += deposited;
        total_held += held;
        total_seller_received += expected.seller_received;
        total_buyer_refunded += expected.buyer_refunded;
        total_arbiter_received += expected.arbiter_received;
    }

    // Action summary consistency for the focused escrow & actor (if it exists).
    if let Some(focused) = model.escrows.get(&focus_id) {
        let summary = client.get_action_summary(&focus_id, world.addr(focus_actor));
        prop_assert_eq!(
            &summary.status,
            &focused.status,
            "action summary status mismatch\n{}",
            trace()
        );
        prop_assert_eq!(
            summary.deadline,
            focused.created_at + focused.timeout,
            "action summary deadline mismatch\n{}",
            trace()
        );
        prop_assert_eq!(
            summary.has_pending_request,
            focused.pending_request.is_some(),
            "action summary has_pending_request mismatch\n{}",
            trace()
        );
        let expected_actions = model.expected_action_eligibility(focused, focus_actor);
        prop_assert_eq!(
            summary.actions.len(),
            u32::try_from(expected_actions.len()).expect("12 fits in u32"),
            "action summary length mismatch\n{}",
            trace()
        );
        for (idx, (exp_action, exp_eligible, exp_reason)) in
            expected_actions.into_iter().enumerate()
        {
            let got = summary
                .actions
                .get_unchecked(u32::try_from(idx).expect("idx fits in u32"));
            prop_assert_eq!(
                &got.action,
                &exp_action,
                "action summary entry {} action mismatch\n{}",
                idx,
                trace()
            );
            prop_assert_eq!(
                got.eligible,
                exp_eligible,
                "action summary {:?} eligibility mismatch for {:?}\n{}",
                exp_action,
                focus_actor,
                trace()
            );
            prop_assert_eq!(
                &got.reason,
                &exp_reason,
                "action summary {:?} reason mismatch for {:?}\n{}",
                exp_action,
                focus_actor,
                trace()
            );
        }
    }

    // Real SAC token balance invariants.
    let contract_bal = tc.balance(&world.contract);
    let buyer_bal = tc.balance(&world.buyer);
    let seller_bal = tc.balance(&world.seller);
    let arbiter_bal = tc.balance(&world.arbiter);
    let outsider_bal = tc.balance(&world.outsider);

    prop_assert_eq!(
        contract_bal,
        total_held,
        "contract token balance must equal total escrow funds held\n{}",
        trace()
    );
    prop_assert_eq!(
        seller_bal,
        total_seller_received,
        "seller balance must match model\n{}",
        trace()
    );
    prop_assert_eq!(
        buyer_bal,
        model.buyer_balance,
        "buyer balance must match model\n{}",
        trace()
    );
    prop_assert_eq!(
        buyer_bal,
        model.initial_buyer_mint - total_deposited + total_buyer_refunded,
        "buyer balance accounting identity failed\n{}",
        trace()
    );
    prop_assert_eq!(
        arbiter_bal,
        total_arbiter_received,
        "arbiter balance must match model\n{}",
        trace()
    );
    prop_assert_eq!(
        outsider_bal,
        0,
        "outsider must never receive tokens\n{}",
        trace()
    );
    if all_fee_free {
        prop_assert_eq!(
            seller_bal + total_buyer_refunded + contract_bal,
            total_deposited,
            "seller received + buyer refunded + held must equal deposited\n{}",
            trace()
        );
    } else {
        prop_assert_eq!(
            seller_bal + total_buyer_refunded + arbiter_bal + contract_bal,
            total_deposited,
            "seller received + buyer refunded + arbiter received + held must equal deposited\n{}",
            trace()
        );
    }
    prop_assert_eq!(
        buyer_bal + seller_bal + arbiter_bal + outsider_bal + contract_bal,
        model.initial_buyer_mint,
        "total minted token supply must be conserved\n{}",
        trace()
    );

    // Participant index invariant: matches all non-cancelled stored escrows in
    // creation order for every actor.
    let live_ids: Vec<u64> = model
        .escrows
        .iter()
        .filter(|(_, e)| e.status != EscrowStatus::Cancelled)
        .map(|(&id, _)| id)
        .collect();

    for actor in Actor::ALL {
        let expected_ids: &[u64] = if actor.is_participant() {
            &live_ids
        } else {
            &[]
        };
        let page = client.escrows_for_participant(world.addr(actor), &0, &u32::MAX);
        let got_ids: Vec<u64> = page.ids.iter().collect();
        prop_assert_eq!(
            &got_ids,
            &expected_ids,
            "participant index for {:?} diverged from stored escrows\n{}",
            actor,
            trace()
        );
        prop_assert_eq!(
            page.total,
            u32::try_from(expected_ids.len()).expect("id count fits in u32"),
            "participant index total for {:?} mismatch\n{}",
            actor,
            trace()
        );
        prop_assert_eq!(
            page.next_cursor,
            None,
            "full participant index read must have no next_cursor\n{}",
            trace()
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

fn arb_valid_amount() -> impl Strategy<Value = i128> {
    prop_oneof![
        3 => 1_i128..=25,
        4 => 26_i128..=10_000,
        3 => 10_001_i128..=MAX_AMOUNT,
    ]
}

fn arb_create_amount() -> impl Strategy<Value = i128> {
    prop_oneof![
        7 => arb_valid_amount(),
        1 => Just(0_i128),
        1 => -100_i128..=-1,
    ]
}

fn arb_valid_timeout() -> impl Strategy<Value = u64> {
    prop_oneof![
        2 => 1_u64..=120,
        4 => 121_u64..=DAY,
        4 => (DAY + 1)..=MAX_TIMEOUT,
    ]
}

fn arb_create_timeout() -> impl Strategy<Value = u64> {
    prop_oneof![
        8 => arb_valid_timeout(),
        1 => Just(0_u64),
    ]
}

fn arb_valid_review_period() -> impl Strategy<Value = u64> {
    prop_oneof![
        2 => Just(MIN_REVIEW_PERIOD),
        4 => (MIN_REVIEW_PERIOD + 1)..=DAY,
        3 => (DAY + 1)..=MAX_REVIEW_PERIOD,
        1 => Just(MAX_REVIEW_PERIOD),
    ]
}

fn arb_review_period_any() -> impl Strategy<Value = u64> {
    prop_oneof![
        7 => arb_valid_review_period(),
        1 => 0_u64..MIN_REVIEW_PERIOD,
        1 => (MAX_REVIEW_PERIOD + 1)..=(MAX_REVIEW_PERIOD + 3_600),
    ]
}

fn arb_valid_arbiter_fee_bps() -> impl Strategy<Value = u32> {
    prop_oneof![
        2 => Just(0_u32),
        5 => 1_u32..MAX_ARBITER_FEE_BPS,
        2 => Just(MAX_ARBITER_FEE_BPS),
    ]
}

fn arb_arbiter_fee_bps_any() -> impl Strategy<Value = u32> {
    prop_oneof![
        7 => arb_valid_arbiter_fee_bps(),
        2 => (MAX_ARBITER_FEE_BPS + 1)..=(MAX_ARBITER_FEE_BPS + 2_000),
    ]
}

fn arb_initial_escrow() -> impl Strategy<Value = InitialEscrow> {
    let variant = prop_oneof![
        4 => Just(CreateVariant::Standard),
        3 => arb_valid_review_period().prop_map(CreateVariant::WithReview),
        3 => arb_valid_arbiter_fee_bps().prop_map(CreateVariant::WithArbiterFee),
    ];
    let extra_mint = prop_oneof![
        3 => Just(0_i128),
        4 => 1_i128..=10_000,
        3 => 10_001_i128..=(MAX_AMOUNT * 2),
    ];
    (variant, arb_valid_amount(), arb_valid_timeout(), extra_mint).prop_map(
        |(variant, amount, timeout, extra_buyer_mint)| InitialEscrow {
            variant,
            amount,
            timeout,
            extra_buyer_mint,
        },
    )
}

fn arb_actor_biased(primary: Actor) -> impl Strategy<Value = Actor> {
    let others: [Actor; 3] = match primary {
        Actor::Buyer => [Actor::Seller, Actor::Arbiter, Actor::Outsider],
        Actor::Seller => [Actor::Buyer, Actor::Arbiter, Actor::Outsider],
        Actor::Arbiter => [Actor::Buyer, Actor::Seller, Actor::Outsider],
        Actor::Outsider => [Actor::Buyer, Actor::Seller, Actor::Arbiter],
    };
    prop_oneof![
        7 => Just(primary),
        1 => Just(others[0]),
        1 => Just(others[1]),
        1 => Just(others[2]),
    ]
}

fn arb_refund_actor() -> impl Strategy<Value = Actor> {
    prop_oneof![
        4 => Just(Actor::Buyer),
        4 => Just(Actor::Seller),
        1 => Just(Actor::Arbiter),
        1 => Just(Actor::Outsider),
    ]
}

fn arb_dispute_actors() -> impl Strategy<Value = (Actor, Actor)> {
    prop_oneof![
        4 => Just((Actor::Buyer, Actor::Buyer)),
        4 => Just((Actor::Seller, Actor::Seller)),
        1 => Just((Actor::Buyer, Actor::Seller)),
        1 => Just((Actor::Seller, Actor::Buyer)),
        1 => Just((Actor::Outsider, Actor::Buyer)),
        1 => Just((Actor::Outsider, Actor::Outsider)),
        1 => Just((Actor::Arbiter, Actor::Arbiter)),
    ]
}

fn arb_target() -> impl Strategy<Value = EscrowTarget> {
    prop_oneof![
        6 => Just(EscrowTarget::Primary),
        5 => Just(EscrowTarget::Latest),
        1 => Just(EscrowTarget::Id(2)),
        1 => Just(EscrowTarget::Id(999)),
    ]
}

fn arb_amount_spec() -> impl Strategy<Value = AmountSpec> {
    prop_oneof![
        5 => (1_u16..=9_999).prop_map(AmountSpec::FractionOfRemainingBps),
        3 => Just(AmountSpec::FullRemaining),
        1 => Just(AmountSpec::OverRemaining),
        1 => prop_oneof![Just(0_i128), Just(-1_i128), Just(1_i128)].prop_map(AmountSpec::Exact),
    ]
}

fn arb_timeout_spec() -> impl Strategy<Value = TimeoutSpec> {
    prop_oneof![
        6 => (1_u64..=(7 * DAY)).prop_map(TimeoutSpec::Add),
        1 => Just(TimeoutSpec::Same),
        1 => Just(TimeoutSpec::Shorter),
        1 => Just(TimeoutSpec::Overflow),
    ]
}

fn arb_time_jump() -> impl Strategy<Value = TimeJump> {
    prop_oneof![
        3 => (0_u64..=(3 * DAY)).prop_map(TimeJump::Seconds),
        2 => Just(TimeJump::JustBeforeDeadline),
        2 => Just(TimeJump::AtDeadline),
        2 => Just(TimeJump::JustAfterDeadline),
        2 => Just(TimeJump::JustBeforeReleaseAt),
        3 => Just(TimeJump::AtReleaseAt),
    ]
}

fn arb_seller_bps() -> impl Strategy<Value = u32> {
    prop_oneof![
        2 => Just(0_u32),
        2 => Just(1_u32),
        4 => 2_u32..9_999,
        2 => Just(10_000_u32),
        2 => 10_001_u32..=15_000,
    ]
}

fn arb_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        // Funding & cancellation
        5 => (
            arb_actor_biased(Actor::Buyer),
            arb_target(),
            prop_oneof![5 => Just(true), 1 => Just(false)],
        ).prop_map(|(actor, target, authorize_transfer)| Step::Deposit {
            actor,
            target,
            authorize_transfer,
        }),
        1 => (arb_actor_biased(Actor::Buyer), arb_target())
            .prop_map(|(actor, target)| Step::Cancel { actor, target }),

        // Direct releases (full & partial)
        2 => (arb_actor_biased(Actor::Buyer), arb_target())
            .prop_map(|(actor, target)| Step::Release { actor, target }),
        4 => (arb_actor_biased(Actor::Buyer), arb_target(), arb_amount_spec())
            .prop_map(|(actor, target, amount)| Step::ReleasePartial {
                actor,
                target,
                amount,
            }),

        // Review-window release flow
        4 => (arb_actor_biased(Actor::Seller), arb_target(), arb_amount_spec())
            .prop_map(|(actor, target, amount)| Step::RequestRelease {
                actor,
                target,
                amount,
            }),
        3 => (arb_actor_biased(Actor::Buyer), arb_target())
            .prop_map(|(actor, target)| Step::ApproveRelease { actor, target }),
        3 => arb_target().prop_map(|target| Step::ExecuteRelease { target }),

        // Refunds & expiry refunds
        3 => (arb_refund_actor(), arb_target())
            .prop_map(|(actor, target)| Step::Refund { actor, target }),
        3 => arb_target().prop_map(|target| Step::RefundExpired { target }),

        // Disputes & resolutions (boolean + basis-point split)
        3 => (arb_dispute_actors(), arb_target())
            .prop_map(|((actor, claimant), target)| Step::Dispute {
                actor,
                claimant,
                target,
            }),
        2 => (
            arb_actor_biased(Actor::Arbiter),
            arb_target(),
            prop::bool::ANY,
        ).prop_map(|(actor, target, in_favor_of_seller)| Step::Resolve {
            actor,
            target,
            in_favor_of_seller,
        }),
        3 => (
            arb_actor_biased(Actor::Arbiter),
            arb_target(),
            arb_seller_bps(),
        ).prop_map(|(actor, target, seller_bps)| Step::ResolveSplit {
            actor,
            target,
            seller_bps,
        }),

        // Extensions, keeper TTL, and time progression
        3 => (arb_actor_biased(Actor::Buyer), arb_target(), arb_timeout_spec())
            .prop_map(|(actor, target, timeout)| Step::ExtendDeadline {
                actor,
                target,
                timeout,
            }),
        1 => arb_target().prop_map(|target| Step::TouchTtl { target }),
        6 => (arb_target(), arb_time_jump())
            .prop_map(|(target, jump)| Step::AdvanceTime { target, jump }),

        // Additional single-escrow creation (valid & invalid terms/actors)
        1 => (
            arb_actor_biased(Actor::Buyer),
            arb_create_amount(),
            arb_create_timeout(),
        ).prop_map(|(actor, amount, timeout)| Step::CreateEscrow {
            actor,
            amount,
            timeout,
        }),
        1 => (
            arb_actor_biased(Actor::Buyer),
            arb_create_amount(),
            arb_create_timeout(),
            arb_review_period_any(),
        ).prop_map(|(actor, amount, timeout, review_period)| Step::CreateEscrowWithReview {
            actor,
            amount,
            timeout,
            review_period,
        }),
        1 => (
            arb_actor_biased(Actor::Buyer),
            arb_create_amount(),
            arb_create_timeout(),
            arb_arbiter_fee_bps_any(),
        ).prop_map(|(actor, amount, timeout, arbiter_fee_bps)| Step::CreateEscrowWithArbiterFee {
            actor,
            amount,
            timeout,
            arbiter_fee_bps,
        }),
    ]
}

// ---------------------------------------------------------------------------
// State-machine execution
// ---------------------------------------------------------------------------

const fn step_focus(model: &StateModel, step: Step) -> (u64, Actor) {
    match step {
        Step::CreateEscrow { actor, .. }
        | Step::CreateEscrowWithReview { actor, .. }
        | Step::CreateEscrowWithArbiterFee { actor, .. } => (1, actor),
        Step::Deposit { actor, target, .. }
        | Step::Cancel { actor, target }
        | Step::Release { actor, target }
        | Step::ReleasePartial { actor, target, .. }
        | Step::RequestRelease { actor, target, .. }
        | Step::ApproveRelease { actor, target }
        | Step::Refund { actor, target }
        | Step::Dispute { actor, target, .. }
        | Step::Resolve { actor, target, .. }
        | Step::ResolveSplit { actor, target, .. }
        | Step::ExtendDeadline { actor, target, .. } => (
            match target {
                EscrowTarget::Primary => 1,
                EscrowTarget::Latest => match model.next_id.checked_sub(1) {
                    Some(id) if id >= 1 => id,
                    _ => 1,
                },
                EscrowTarget::Id(id) => id,
            },
            actor,
        ),
        Step::ExecuteRelease { target }
        | Step::RefundExpired { target }
        | Step::TouchTtl { target }
        | Step::AdvanceTime { target, .. } => (
            match target {
                EscrowTarget::Primary => 1,
                EscrowTarget::Latest => match model.next_id.checked_sub(1) {
                    Some(id) if id >= 1 => id,
                    _ => 1,
                },
                EscrowTarget::Id(id) => id,
            },
            Actor::Outsider,
        ),
    }
}

fn run_lifecycle_case(initial: InitialEscrow, steps: &[Step]) -> Result<(), TestCaseError> {
    let initial_buyer_mint = initial.amount + initial.extra_buyer_mint;
    let world = World::new(initial_buyer_mint);
    let mut model = StateModel::new(START_TIMESTAMP, initial_buyer_mint);

    // Step 0: create the initial single-token escrow via the chosen create entrypoint.
    let create_step = match initial.variant {
        CreateVariant::Standard => Step::CreateEscrow {
            actor: Actor::Buyer,
            amount: initial.amount,
            timeout: initial.timeout,
        },
        CreateVariant::WithReview(review_period) => Step::CreateEscrowWithReview {
            actor: Actor::Buyer,
            amount: initial.amount,
            timeout: initial.timeout,
            review_period,
        },
        CreateVariant::WithArbiterFee(arbiter_fee_bps) => Step::CreateEscrowWithArbiterFee {
            actor: Actor::Buyer,
            amount: initial.amount,
            timeout: initial.timeout,
            arbiter_fee_bps,
        },
    };
    let actual_init = world.execute_step(&model, create_step);
    let expected_init = model.apply(create_step);
    prop_assert_eq!(
        actual_init,
        expected_init,
        "initial creation outcome mismatch\n{}",
        format_trace(initial, &[])
    );
    assert_state_and_invariants(&world, &model, initial, &[], 1, Actor::Buyer)?;

    // Execute every generated step and compare outcome, state, and invariants.
    for (idx, &step) in steps.iter().enumerate() {
        let actual = world.execute_step(&model, step);
        let expected = model.apply(step);
        if actual != expected {
            let trace = format_trace(initial, &steps[..=idx]);
            prop_assert_eq!(
                actual,
                expected,
                "step {} outcome mismatch: {}\n{}",
                idx + 1,
                step,
                trace
            );
        }
        let (focus_id, focus_actor) = step_focus(&model, step);
        assert_state_and_invariants(
            &world,
            &model,
            initial,
            &steps[..=idx],
            focus_id,
            focus_actor,
        )?;
    }

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: LIFECYCLE_CASES,
        rng_seed: RngSeed::Fixed(LIFECYCLE_SEED),
        ..ProptestConfig::default()
    })]

    /// Full single-escrow lifecycle model-based state machine:
    /// runs 160 deterministic cases of up to 40 steps covering every
    /// single-escrow entrypoint, random actors, amounts, and time jumps.
    #[test]
    fn single_escrow_full_lifecycle_matches_model(
        initial in arb_initial_escrow(),
        steps in prop::collection::vec(arb_step(), 1..=MAX_STEPS),
    ) {
        run_lifecycle_case(initial, &steps)?;
    }
}

// ---------------------------------------------------------------------------
// Deterministic smoke / regression tests for tricky multi-step interactions
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_model_covers_review_extension_partial_dispute_split_and_terminal_lock() {
    let initial = InitialEscrow {
        variant: CreateVariant::WithArbiterFee(500), // 5% arbiter fee
        amount: 1_000,
        timeout: 1_000,
        extra_buyer_mint: 5_000,
    };
    let steps = [
        // Wrong actor cannot deposit; missing nested token auth fails cleanly.
        Step::Deposit {
            actor: Actor::Seller,
            target: EscrowTarget::Primary,
            authorize_transfer: true,
        },
        Step::Deposit {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
            authorize_transfer: false,
        },
        Step::Deposit {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
            authorize_transfer: true,
        },
        // Partial release, then request release, then jump past deadline:
        // buyer refund and refund_expired are both blocked by ReleasePending.
        Step::ReleasePartial {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
            amount: AmountSpec::Exact(200),
        },
        Step::RequestRelease {
            actor: Actor::Seller,
            target: EscrowTarget::Primary,
            amount: AmountSpec::Exact(300),
        },
        Step::AdvanceTime {
            target: EscrowTarget::Primary,
            jump: TimeJump::JustAfterDeadline,
        },
        Step::Refund {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
        },
        Step::RefundExpired {
            target: EscrowTarget::Primary,
        },
        // Buyer extends deadline even after it passed; pending request stays intact.
        Step::ExtendDeadline {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
            timeout: TimeoutSpec::Add(10_000),
        },
        // Execute before review window ends -> DeadlineReached; at release_at -> Ok.
        Step::AdvanceTime {
            target: EscrowTarget::Primary,
            jump: TimeJump::JustBeforeReleaseAt,
        },
        Step::ExecuteRelease {
            target: EscrowTarget::Primary,
        },
        Step::AdvanceTime {
            target: EscrowTarget::Primary,
            jump: TimeJump::AtReleaseAt,
        },
        Step::ExecuteRelease {
            target: EscrowTarget::Primary,
        },
        // Dispute remaining 500 and resolve via basis-point split with arbiter fee.
        Step::Dispute {
            actor: Actor::Buyer,
            claimant: Actor::Buyer,
            target: EscrowTarget::Primary,
        },
        Step::ResolveSplit {
            actor: Actor::Arbiter,
            target: EscrowTarget::Primary,
            seller_bps: 3_333,
        },
        // Terminal escrow rejects every subsequent mutation without changing state.
        Step::Release {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
        },
        Step::RefundExpired {
            target: EscrowTarget::Primary,
        },
        Step::ExtendDeadline {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
            timeout: TimeoutSpec::Add(5_000),
        },
        Step::Cancel {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
        },
        // Create a second escrow with custom review window and cancel it;
        // verify participant index retains id 1 and removes id 2.
        Step::CreateEscrowWithReview {
            actor: Actor::Buyer,
            amount: 500,
            timeout: 2_000,
            review_period: 120,
        },
        Step::Cancel {
            actor: Actor::Buyer,
            target: EscrowTarget::Latest,
        },
        Step::TouchTtl {
            target: EscrowTarget::Primary,
        },
    ];
    run_lifecycle_case(initial, &steps).expect("deterministic lifecycle sequence must pass");
}

#[test]
fn lifecycle_trace_formats_as_readable_step_list() {
    let initial = InitialEscrow {
        variant: CreateVariant::WithReview(300),
        amount: 100,
        timeout: 600,
        extra_buyer_mint: 0,
    };
    let steps = [
        Step::Deposit {
            actor: Actor::Buyer,
            target: EscrowTarget::Primary,
            authorize_transfer: true,
        },
        Step::AdvanceTime {
            target: EscrowTarget::Primary,
            jump: TimeJump::JustAfterDeadline,
        },
        Step::RefundExpired {
            target: EscrowTarget::Primary,
        },
    ];
    let rendered = format_trace(initial, &steps);
    assert!(rendered.contains("1. deposit(actor=Buyer"));
    assert!(rendered.contains("2. advance_time(target=Primary, jump=JustAfterDeadline)"));
    assert!(rendered.contains("3. refund_expired(target=Primary)"));
}

/// Verify that the fixed-seed 160-case strategy actually reaches the success
/// path of every single-escrow entrypoint as well as every contract error class
/// and authorization abort (runs on the pure-Rust model in milliseconds).
#[test]
fn lifecycle_generator_covers_every_entrypoint_and_error_class() {
    use proptest::strategy::ValueTree as _;
    use proptest::test_runner::TestRunner;

    let mut runner = TestRunner::new(ProptestConfig {
        cases: LIFECYCLE_CASES,
        rng_seed: RngSeed::Fixed(LIFECYCLE_SEED),
        ..ProptestConfig::default()
    });
    let strat = (
        arb_initial_escrow(),
        prop::collection::vec(arb_step(), 1..=MAX_STEPS),
    );

    let mut ok_create = 0_u32;
    let mut ok_create_review = 0_u32;
    let mut ok_create_fee = 0_u32;
    let mut ok_deposit = 0_u32;
    let mut ok_cancel = 0_u32;
    let mut ok_release = 0_u32;
    let mut ok_release_partial = 0_u32;
    let mut ok_request_release = 0_u32;
    let mut ok_approve_release = 0_u32;
    let mut ok_execute_release = 0_u32;
    let mut ok_refund = 0_u32;
    let mut ok_refund_expired = 0_u32;
    let mut ok_dispute = 0_u32;
    let mut ok_resolve = 0_u32;
    let mut ok_resolve_split = 0_u32;
    let mut ok_extend_deadline = 0_u32;
    let mut ok_touch_ttl = 0_u32;

    let mut err_not_found = 0_u32;
    let mut err_invalid_input = 0_u32;
    let mut err_deadline_reached = 0_u32;
    let mut err_release_pending = 0_u32;
    let mut err_token_transfer_failed = 0_u32;
    let mut err_overflow = 0_u32;
    let mut err_auth_abort = 0_u32;

    for _ in 0..LIFECYCLE_CASES {
        let (initial, steps) = strat
            .new_tree(&mut runner)
            .expect("generate lifecycle case")
            .current();
        let initial_buyer_mint = initial.amount + initial.extra_buyer_mint;
        let mut model = StateModel::new(START_TIMESTAMP, initial_buyer_mint);

        let init_step = match initial.variant {
            CreateVariant::Standard => Step::CreateEscrow {
                actor: Actor::Buyer,
                amount: initial.amount,
                timeout: initial.timeout,
            },
            CreateVariant::WithReview(review_period) => Step::CreateEscrowWithReview {
                actor: Actor::Buyer,
                amount: initial.amount,
                timeout: initial.timeout,
                review_period,
            },
            CreateVariant::WithArbiterFee(arbiter_fee_bps) => Step::CreateEscrowWithArbiterFee {
                actor: Actor::Buyer,
                amount: initial.amount,
                timeout: initial.timeout,
                arbiter_fee_bps,
            },
        };
        let _ = model.apply(init_step);

        for step in steps {
            let outcome = model.apply(step);
            match outcome {
                StepOutcome::Ok | StepOutcome::OkU64(_) => match step {
                    Step::CreateEscrow { .. } => ok_create += 1,
                    Step::CreateEscrowWithReview { .. } => ok_create_review += 1,
                    Step::CreateEscrowWithArbiterFee { .. } => ok_create_fee += 1,
                    Step::Deposit { .. } => ok_deposit += 1,
                    Step::Cancel { .. } => ok_cancel += 1,
                    Step::Release { .. } => ok_release += 1,
                    Step::ReleasePartial { .. } => ok_release_partial += 1,
                    Step::RequestRelease { .. } => ok_request_release += 1,
                    Step::ApproveRelease { .. } => ok_approve_release += 1,
                    Step::ExecuteRelease { .. } => ok_execute_release += 1,
                    Step::Refund { .. } => ok_refund += 1,
                    Step::RefundExpired { .. } => ok_refund_expired += 1,
                    Step::Dispute { .. } => ok_dispute += 1,
                    Step::Resolve { .. } => ok_resolve += 1,
                    Step::ResolveSplit { .. } => ok_resolve_split += 1,
                    Step::ExtendDeadline { .. } => ok_extend_deadline += 1,
                    Step::TouchTtl { .. } => ok_touch_ttl += 1,
                    Step::AdvanceTime { .. } => {}
                },
                StepOutcome::ContractError(ForgeError::NotFound) => err_not_found += 1,
                StepOutcome::ContractError(ForgeError::InvalidInput) => err_invalid_input += 1,
                StepOutcome::ContractError(ForgeError::DeadlineReached) => {
                    err_deadline_reached += 1;
                }
                StepOutcome::ContractError(ForgeError::ReleasePending) => err_release_pending += 1,
                StepOutcome::ContractError(ForgeError::TokenTransferFailed) => {
                    err_token_transfer_failed += 1;
                }
                StepOutcome::ContractError(ForgeError::ArithmeticOverflow) => err_overflow += 1,
                StepOutcome::AuthAbort => err_auth_abort += 1,
                StepOutcome::ContractError(other) => panic!("unexpected error class: {other:?}"),
            }
        }
    }

    assert!(ok_create > 0, "create_escrow step never succeeded");
    assert!(
        ok_create_review > 0,
        "create_escrow_with_review step never succeeded"
    );
    assert!(
        ok_create_fee > 0,
        "create_escrow_with_arbiter_fee step never succeeded"
    );
    assert!(ok_deposit > 0, "deposit never succeeded");
    assert!(ok_cancel > 0, "cancel never succeeded");
    assert!(ok_release > 0, "release never succeeded");
    assert!(ok_release_partial > 0, "release_partial never succeeded");
    assert!(ok_request_release > 0, "request_release never succeeded");
    assert!(ok_approve_release > 0, "approve_release never succeeded");
    assert!(ok_execute_release > 0, "execute_release never succeeded");
    assert!(ok_refund > 0, "refund never succeeded");
    assert!(ok_refund_expired > 0, "refund_expired never succeeded");
    assert!(ok_dispute > 0, "dispute never succeeded");
    assert!(ok_resolve > 0, "resolve never succeeded");
    assert!(ok_resolve_split > 0, "resolve_split never succeeded");
    assert!(ok_extend_deadline > 0, "extend_deadline never succeeded");
    assert!(ok_touch_ttl > 0, "touch_ttl never succeeded");

    assert!(err_not_found > 0, "NotFound never triggered");
    assert!(err_invalid_input > 0, "InvalidInput never triggered");
    assert!(err_deadline_reached > 0, "DeadlineReached never triggered");
    assert!(err_release_pending > 0, "ReleasePending never triggered");
    assert!(
        err_token_transfer_failed > 0,
        "TokenTransferFailed never triggered"
    );
    assert!(err_overflow > 0, "ArithmeticOverflow never triggered");
    assert!(err_auth_abort > 0, "AuthAbort never triggered");
}

/// Verify that a failing multi-step case shrinks via proptest's `TestRunner`
/// to a minimal, human-readable step list.
#[test]
fn failing_lifecycle_case_shrinks_to_minimal_readable_steps() {
    use proptest::test_runner::{TestError, TestRunner};

    let mut runner = TestRunner::new(ProptestConfig {
        cases: LIFECYCLE_CASES,
        rng_seed: RngSeed::Fixed(LIFECYCLE_SEED),
        ..ProptestConfig::default()
    });
    let strat = (
        arb_initial_escrow(),
        prop::collection::vec(arb_step(), 1..=MAX_STEPS),
    );

    // Synthetic failing property: "the primary escrow never reaches Completed".
    let err = runner
        .run(&strat, |(initial, steps)| {
            let mut model =
                StateModel::new(START_TIMESTAMP, initial.amount + initial.extra_buyer_mint);
            let _ = model.apply(Step::CreateEscrow {
                actor: Actor::Buyer,
                amount: initial.amount,
                timeout: initial.timeout,
            });
            for step in &steps {
                let _ = model.apply(*step);
                if model
                    .escrows
                    .get(&1)
                    .is_some_and(|e| e.status == EscrowStatus::Completed)
                {
                    return Err(TestCaseError::fail("completed"));
                }
            }
            Ok(())
        })
        .expect_err("synthetic property must fail and shrink");

    match err {
        TestError::Fail(_, (initial, shrunk_steps)) => {
            assert!(
                shrunk_steps.len() <= 3,
                "expected minimal shrunk trace (<= 3 steps), got {} steps:\n{}",
                shrunk_steps.len(),
                format_trace(initial, &shrunk_steps)
            );
            let rendered = format_trace(initial, &shrunk_steps);
            assert!(rendered.contains("1. deposit("));
        }
        TestError::Abort(reason) => panic!("unexpected abort: {reason}"),
    }
}
