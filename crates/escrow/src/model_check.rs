//! State-machine model checking for the escrow contract lifecycle.
//!
//! Validates the escrow contract against an independent state-machine model:
//! - Tracks statuses: Uncreated -> Pending -> Funded -> (Disputed) -> Completed / Refunded / Cancelled
//! - Enforces token conservation: buyer + seller + arbiter + contract == total_minted
//! - Verifies terminal states: contract balance drops to 0 and terminal state is irreversible
//! - Guarantees non-mutation: invalid operations are rejected without modifying storage or balances

extern crate std;

use crate::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use core::fmt;
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::model::{
    assert_model_check, LifecycleModel, ModelDriver, ModelRng, OpGenerator,
};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::format;
use std::string::{String, ToString};

const START_TIMESTAMP: u64 = 1_000_000;
const INITIAL_MINT: i128 = 10_000_000;
const ESCROW_AMOUNT: i128 = 1_000_000;
const TIMEOUT: u64 = 86_400;

/// Abstract lifecycle model state for escrow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowModelState {
    Uncreated,
    Pending {
        amount: i128,
        timeout: u64,
    },
    Funded {
        amount: i128,
        released: i128,
        timeout: u64,
    },
    Disputed {
        amount: i128,
        released: i128,
        timeout: u64,
    },
    Completed {
        total_amount: i128,
        released: i128,
    },
    Refunded {
        total_amount: i128,
        released: i128,
        refunded: i128,
    },
    Cancelled,
}

/// Abstract operation applied to the escrow model and contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowModelOp {
    Create { amount: i128, timeout: u64 },
    Deposit,
    ReleaseFull,
    ReleasePartial { amount: i128 },
    Refund,
    RefundExpired,
    Dispute { outsider: bool },
    Resolve { in_favor_of_seller: bool },
    Cancel,
    AdvanceTime { seconds: u64 },
    UnknownEscrowOp,
}

impl fmt::Display for EscrowModelOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create { amount, timeout } => {
                write!(f, "Create(amount={amount}, timeout={timeout})")
            }
            Self::Deposit => write!(f, "Deposit"),
            Self::ReleaseFull => write!(f, "ReleaseFull"),
            Self::ReleasePartial { amount } => write!(f, "ReleasePartial(amount={amount})"),
            Self::Refund => write!(f, "Refund"),
            Self::RefundExpired => write!(f, "RefundExpired"),
            Self::Dispute { outsider } => write!(f, "Dispute(outsider={outsider})"),
            Self::Resolve { in_favor_of_seller } => {
                write!(f, "Resolve(in_favor_of_seller={in_favor_of_seller})")
            }
            Self::Cancel => write!(f, "Cancel"),
            Self::AdvanceTime { seconds } => write!(f, "AdvanceTime({seconds}s)"),
            Self::UnknownEscrowOp => write!(f, "UnknownEscrowOp"),
        }
    }
}

/// On-chain state snapshot captured before and after each operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowSnapshot {
    pub status: Option<EscrowStatus>,
    pub released: i128,
    pub contract_balance: i128,
    pub buyer_balance: i128,
    pub seller_balance: i128,
    pub arbiter_balance: i128,
    pub current_time: u64,
}

pub struct EscrowLifecycleModel {
    pub initial_time: u64,
}

impl LifecycleModel for EscrowLifecycleModel {
    type State = (EscrowModelState, u64); // (model_state, current_time)
    type Op = EscrowModelOp;
    type Snapshot = EscrowSnapshot;

    fn initial_state(&self) -> Self::State {
        (EscrowModelState::Uncreated, self.initial_time)
    }

    fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String> {
        let (current_state, current_time) = state;

        match op {
            EscrowModelOp::AdvanceTime { seconds } => {
                Ok((current_state.clone(), current_time.saturating_add(*seconds)))
            }
            EscrowModelOp::Create { amount, timeout } => match current_state {
                EscrowModelState::Uncreated => {
                    if *amount <= 0 {
                        Err("amount must be positive".into())
                    } else if *timeout == 0 {
                        Err("timeout must be positive".into())
                    } else {
                        Ok((
                            EscrowModelState::Pending {
                                amount: *amount,
                                timeout: *timeout,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("escrow already created".into()),
            },
            EscrowModelOp::Deposit => match current_state {
                EscrowModelState::Pending { amount, timeout } => Ok((
                    EscrowModelState::Funded {
                        amount: *amount,
                        released: 0,
                        timeout: *timeout,
                    },
                    *current_time,
                )),
                _ => Err("can only deposit while Pending".into()),
            },
            EscrowModelOp::ReleaseFull => match current_state {
                EscrowModelState::Funded { amount, .. } => Ok((
                    EscrowModelState::Completed {
                        total_amount: *amount,
                        released: *amount,
                    },
                    *current_time,
                )),
                _ => Err("can only release full while Funded".into()),
            },
            EscrowModelOp::ReleasePartial { amount } => match current_state {
                EscrowModelState::Funded {
                    amount: total,
                    released,
                    timeout,
                } => {
                    let remaining = total.saturating_sub(*released);
                    if *amount <= 0 || *amount > remaining {
                        Err(format!(
                            "partial amount {amount} invalid for remaining {remaining}"
                        ))
                    } else if *amount == remaining {
                        Ok((
                            EscrowModelState::Completed {
                                total_amount: *total,
                                released: *total,
                            },
                            *current_time,
                        ))
                    } else {
                        Ok((
                            EscrowModelState::Funded {
                                amount: *total,
                                released: released.saturating_add(*amount),
                                timeout: *timeout,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("can only release partial while Funded".into()),
            },
            EscrowModelOp::Refund => match current_state {
                EscrowModelState::Funded {
                    amount: total,
                    released,
                    ..
                } => {
                    let refunded = total.saturating_sub(*released);
                    Ok((
                        EscrowModelState::Refunded {
                            total_amount: *total,
                            released: *total,
                            refunded,
                        },
                        *current_time,
                    ))
                }
                _ => Err("can only refund while Funded".into()),
            },
            EscrowModelOp::RefundExpired => match current_state {
                EscrowModelState::Funded {
                    amount: total,
                    released,
                    timeout,
                } => {
                    let deadline = self.initial_time.saturating_add(*timeout);
                    if *current_time <= deadline {
                        Err(format!(
                            "cannot refund_expired before deadline {deadline} (current {current_time})"
                        ))
                    } else {
                        let refunded = total.saturating_sub(*released);
                        Ok((
                            EscrowModelState::Refunded {
                                total_amount: *total,
                                released: *total,
                                refunded,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("can only refund_expired while Funded".into()),
            },
            EscrowModelOp::Dispute { outsider } => {
                if *outsider {
                    Err("outsider dispute rejected".into())
                } else {
                    match current_state {
                        EscrowModelState::Funded {
                            amount,
                            released,
                            timeout,
                        } => Ok((
                            EscrowModelState::Disputed {
                                amount: *amount,
                                released: *released,
                                timeout: *timeout,
                            },
                            *current_time,
                        )),
                        _ => Err("can only dispute while Funded".into()),
                    }
                }
            }
            EscrowModelOp::Resolve { in_favor_of_seller } => match current_state {
                EscrowModelState::Disputed {
                    amount: total,
                    released,
                    ..
                } => {
                    if *in_favor_of_seller {
                        Ok((
                            EscrowModelState::Completed {
                                total_amount: *total,
                                released: *total,
                            },
                            *current_time,
                        ))
                    } else {
                        let refunded = total.saturating_sub(*released);
                        Ok((
                            EscrowModelState::Refunded {
                                total_amount: *total,
                                released: *total,
                                refunded,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("can only resolve while Disputed".into()),
            },
            EscrowModelOp::Cancel => match current_state {
                EscrowModelState::Pending { .. } => {
                    Ok((EscrowModelState::Cancelled, *current_time))
                }
                _ => Err("can only cancel while Pending".into()),
            },
            EscrowModelOp::UnknownEscrowOp => Err("unknown escrow target".into()),
        }
    }

    fn check_invariants(
        &self,
        state: &Self::State,
        snapshot: &Self::Snapshot,
    ) -> Result<(), String> {
        let (current_state, current_time) = state;

        // 1. Ledger time correspondence
        if *current_time != snapshot.current_time {
            return Err(format!(
                "time mismatch: model={current_time}, on-chain={}",
                snapshot.current_time
            ));
        }

        // 2. Value conservation: total tokens across accounts + contract == INITIAL_MINT
        let total_tokens = snapshot
            .contract_balance
            .saturating_add(snapshot.buyer_balance)
            .saturating_add(snapshot.seller_balance)
            .saturating_add(snapshot.arbiter_balance);

        if total_tokens != INITIAL_MINT {
            return Err(format!(
                "Token conservation violated! Expected {INITIAL_MINT}, got {total_tokens}"
            ));
        }

        // 3. Status and terminal state rules
        match current_state {
            EscrowModelState::Uncreated => {
                if snapshot.status.is_some() || snapshot.contract_balance != 0 {
                    return Err("Uncreated escrow has on-chain record or balance".into());
                }
            }
            EscrowModelState::Pending { .. } => {
                if snapshot.status != Some(EscrowStatus::Pending) {
                    return Err(format!(
                        "Expected Pending status, got {:?}",
                        snapshot.status
                    ));
                }
                if snapshot.contract_balance != 0 {
                    return Err("Pending escrow must have 0 contract custody".into());
                }
            }
            EscrowModelState::Funded {
                amount, released, ..
            } => {
                if snapshot.status != Some(EscrowStatus::Funded) {
                    return Err(format!("Expected Funded status, got {:?}", snapshot.status));
                }
                let expected_remaining = amount.saturating_sub(*released);
                if snapshot.contract_balance != expected_remaining {
                    return Err(format!(
                        "Contract custody mismatch in Funded: expected {expected_remaining}, got {}",
                        snapshot.contract_balance
                    ));
                }
                if snapshot.released != *released {
                    return Err(format!(
                        "Released accounting mismatch: model={released}, on-chain={}",
                        snapshot.released
                    ));
                }
            }
            EscrowModelState::Disputed {
                amount, released, ..
            } => {
                if snapshot.status != Some(EscrowStatus::Disputed) {
                    return Err(format!(
                        "Expected Disputed status, got {:?}",
                        snapshot.status
                    ));
                }
                let expected_remaining = amount.saturating_sub(*released);
                if snapshot.contract_balance != expected_remaining {
                    return Err(format!(
                        "Contract custody mismatch in Disputed: expected {expected_remaining}, got {}",
                        snapshot.contract_balance
                    ));
                }
            }
            EscrowModelState::Completed { released, .. } => {
                if snapshot.status != Some(EscrowStatus::Completed) {
                    return Err(format!(
                        "Expected Completed status, got {:?}",
                        snapshot.status
                    ));
                }
                if snapshot.contract_balance != 0 {
                    return Err("Completed escrow must hold zero balance in contract".into());
                }
                if snapshot.released != *released {
                    return Err("Released accounting mismatch in Completed".into());
                }
            }
            EscrowModelState::Refunded { .. } => {
                if snapshot.status != Some(EscrowStatus::Refunded) {
                    return Err(format!(
                        "Expected Refunded status, got {:?}",
                        snapshot.status
                    ));
                }
                if snapshot.contract_balance != 0 {
                    return Err("Refunded escrow must hold zero balance in contract".into());
                }
            }
            EscrowModelState::Cancelled => {
                if snapshot.status != Some(EscrowStatus::Cancelled) {
                    return Err(format!(
                        "Expected Cancelled status, got {:?}",
                        snapshot.status
                    ));
                }
                if snapshot.contract_balance != 0 {
                    return Err("Cancelled escrow must hold zero balance in contract".into());
                }
            }
        }

        Ok(())
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(
            state.0,
            EscrowModelState::Completed { .. }
                | EscrowModelState::Refunded { .. }
                | EscrowModelState::Cancelled
        )
    }
}

pub struct EscrowModelDriver {
    pub contract_id: Address,
    pub token: Address,
    pub accounts: TestAccounts,
    pub escrow_id: Option<u64>,
}

impl EscrowModelDriver {
    pub fn new(env: &Env) -> Self {
        env.mock_all_auths();
        env.ledger().set_timestamp(START_TIMESTAMP);

        let admin = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let accounts = TestAccounts::generate(env);
        StellarAssetClient::new(env, &token).mint(&accounts.user1, &INITIAL_MINT);

        let contract_id = env.register(Escrow, ());

        Self {
            contract_id,
            token,
            accounts,
            escrow_id: None,
        }
    }

    fn client<'a>(&self, env: &'a Env) -> SorobanForgeEscrowClient<'a> {
        SorobanForgeEscrowClient::new(env, &self.contract_id)
    }

    fn token_client<'a>(&self, env: &'a Env) -> TokenClient<'a> {
        TokenClient::new(env, &self.token)
    }
}

impl ModelDriver<EscrowLifecycleModel> for EscrowModelDriver {
    fn execute_op(&mut self, env: &Env, op: &EscrowModelOp) -> Result<(), String> {
        let client = self.client(env);

        match op {
            EscrowModelOp::AdvanceTime { seconds } => {
                let current = env.ledger().timestamp();
                env.ledger().set_timestamp(current.saturating_add(*seconds));
                Ok(())
            }
            EscrowModelOp::Create { amount, timeout } => {
                if self.escrow_id.is_some() {
                    return Err("already created".into());
                }
                // Try create escrow
                let res = client.try_create_escrow(
                    &self.accounts.user1,
                    &self.accounts.user2,
                    &self.accounts.arbiter,
                    &self.token,
                    amount,
                    timeout,
                );
                match res {
                    Ok(Ok(id)) => {
                        self.escrow_id = Some(id);
                        Ok(())
                    }
                    Ok(Err(_)) => Err("conversion error".into()),
                    Err(Ok(forge_err)) => Err(format!("{forge_err:?}")),
                    Err(Err(_)) => Err("invoke error".into()),
                }
            }
            EscrowModelOp::Deposit => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                map_result(client.try_deposit(&id))
            }
            EscrowModelOp::ReleaseFull => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                map_result(client.try_release(&id))
            }
            EscrowModelOp::ReleasePartial { amount } => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                map_result(client.try_release_partial(&id, amount))
            }
            EscrowModelOp::Refund => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                map_result(client.try_refund(&id))
            }
            EscrowModelOp::RefundExpired => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                map_result(client.try_refund_expired(&id))
            }
            EscrowModelOp::Dispute { outsider } => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                let claimant = if *outsider {
                    self.accounts.user3.clone()
                } else {
                    self.accounts.user1.clone()
                };
                map_result(client.try_dispute(&id, &claimant))
            }
            EscrowModelOp::Resolve { in_favor_of_seller } => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                map_result(client.try_resolve(&id, in_favor_of_seller))
            }
            EscrowModelOp::Cancel => {
                let id = self
                    .escrow_id
                    .ok_or_else(|| "no escrow created".to_string())?;
                map_result(client.try_cancel(&id))
            }
            EscrowModelOp::UnknownEscrowOp => {
                let invalid_id = 999_999_u64;
                map_result(client.try_deposit(&invalid_id))
            }
        }
    }

    fn snapshot(&self, env: &Env) -> EscrowSnapshot {
        let tc = self.token_client(env);
        let client = self.client(env);

        let (status, released) = if let Some(id) = self.escrow_id {
            match client.try_get_escrow(&id) {
                Ok(Ok(data)) => (Some(data.status), data.released),
                _ => (None, 0),
            }
        } else {
            (None, 0)
        };

        EscrowSnapshot {
            status,
            released,
            contract_balance: tc.balance(&self.contract_id),
            buyer_balance: tc.balance(&self.accounts.user1),
            seller_balance: tc.balance(&self.accounts.user2),
            arbiter_balance: tc.balance(&self.accounts.arbiter),
            current_time: env.ledger().timestamp(),
        }
    }

    fn reset(&mut self, env: &Env) {
        *self = Self::new(env);
    }
}

fn map_result<T>(
    res: Result<
        Result<T, soroban_sdk::ConversionError>,
        Result<ForgeError, soroban_sdk::InvokeError>,
    >,
) -> Result<(), String> {
    match res {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => Err("Conversion error".into()),
        Err(Ok(forge_err)) => Err(format!("{forge_err:?}")),
        Err(Err(_)) => Err("Invoke error".into()),
    }
}

pub struct EscrowOpGenerator;

impl OpGenerator<EscrowLifecycleModel> for EscrowOpGenerator {
    fn generate_op(
        &mut self,
        state: &(EscrowModelState, u64),
        rng: &mut ModelRng,
    ) -> EscrowModelOp {
        let (current_state, _time) = state;

        // Mix valid and invalid operations dynamically
        let choice = rng.bounded(12);
        match current_state {
            EscrowModelState::Uncreated => match choice {
                0..=8 => EscrowModelOp::Create {
                    amount: ESCROW_AMOUNT,
                    timeout: TIMEOUT,
                },
                9 => EscrowModelOp::Create {
                    amount: 0, // Invalid: amount 0
                    timeout: TIMEOUT,
                },
                10 => EscrowModelOp::Deposit,    // Invalid: uncreated
                _ => EscrowModelOp::ReleaseFull, // Invalid: uncreated
            },
            EscrowModelState::Pending { amount, .. } => match choice {
                0..=5 => EscrowModelOp::Deposit,
                6..=7 => EscrowModelOp::Cancel,
                8 => EscrowModelOp::ReleaseFull, // Invalid while Pending
                9 => EscrowModelOp::Refund,      // Invalid while Pending
                10 => EscrowModelOp::Create {
                    amount: *amount,
                    timeout: TIMEOUT,
                }, // Invalid: already created
                _ => EscrowModelOp::AdvanceTime {
                    seconds: rng.gen_range_u64(100, 10_000),
                },
            },
            EscrowModelState::Funded {
                amount, released, ..
            } => {
                let remaining = amount.saturating_sub(*released);
                match choice {
                    0..=2 => {
                        let frac = rng.bounded(3) + 1; // 1/3, 2/3, 3/3
                        let part = (remaining / 3) * (frac as i128);
                        let p = if part == 0 {
                            remaining
                        } else {
                            part.min(remaining)
                        };
                        EscrowModelOp::ReleasePartial { amount: p }
                    }
                    3 => EscrowModelOp::ReleaseFull,
                    4 => EscrowModelOp::Dispute { outsider: false },
                    5 => EscrowModelOp::Dispute { outsider: true }, // Invalid outsider
                    6 => EscrowModelOp::Refund,
                    7 => EscrowModelOp::RefundExpired, // Valid if timeout expired, invalid otherwise
                    8 => EscrowModelOp::Deposit,       // Invalid: already deposited
                    9 => EscrowModelOp::Cancel,        // Invalid: cannot cancel when Funded
                    10 => EscrowModelOp::ReleasePartial {
                        amount: remaining + 1, // Invalid: over remaining
                    },
                    _ => EscrowModelOp::AdvanceTime {
                        seconds: rng.gen_range_u64(1_000, 100_000),
                    },
                }
            }
            EscrowModelState::Disputed { .. } => match choice {
                0..=4 => EscrowModelOp::Resolve {
                    in_favor_of_seller: rng.gen_bool(),
                },
                5..=6 => EscrowModelOp::ReleaseFull, // Invalid while Disputed
                7 => EscrowModelOp::Deposit,         // Invalid while Disputed
                8 => EscrowModelOp::Cancel,          // Invalid while Disputed
                9 => EscrowModelOp::Refund,          // Invalid while Disputed
                _ => EscrowModelOp::AdvanceTime { seconds: 1_000 },
            },
            EscrowModelState::Completed { .. }
            | EscrowModelState::Refunded { .. }
            | EscrowModelState::Cancelled => {
                // Terminal states: verify no operations can mutate on-chain state
                match choice {
                    0 => EscrowModelOp::Deposit,
                    1 => EscrowModelOp::ReleaseFull,
                    2 => EscrowModelOp::Refund,
                    3 => EscrowModelOp::Dispute { outsider: false },
                    4 => EscrowModelOp::Resolve {
                        in_favor_of_seller: true,
                    },
                    5 => EscrowModelOp::Cancel,
                    6 => EscrowModelOp::ReleasePartial { amount: 100 },
                    7 => EscrowModelOp::UnknownEscrowOp,
                    _ => EscrowModelOp::AdvanceTime { seconds: 500 },
                }
            }
        }
    }
}

#[test]
fn test_escrow_model_check_deterministic_seeds() {
    let seeds = [101_u64, 202, 303, 404, 505, 606, 707, 808];
    for seed in seeds {
        let env = Env::default();
        let model = EscrowLifecycleModel {
            initial_time: START_TIMESTAMP,
        };
        let mut driver = EscrowModelDriver::new(&env);
        let mut generator = EscrowOpGenerator;

        assert_model_check(&env, &model, &mut driver, &mut generator, seed, 25);
    }
}
