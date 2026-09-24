#![no_std]

//! # Soroban Forge — Vesting contract
//!
//! A token-vesting contract that releases a beneficiary's tokens linearly
//! over time, optionally behind a cliff.
//!
//! Timings are expressed as **durations in seconds measured from the schedule
//! start** (the ledger timestamp recorded at creation):
//!
//! ```text
//! start ......... start+cliff ................... start+duration
//!   |             (claims become possible)        (fully vested)
//!   |  Locked    |            Vesting (linear)  |
//! ```
//!
//! The vested amount at ledger time `t` is:
//! - `0` when `t < start + cliff`,
//! - `total_amount` when `t >= start + duration`,
//! - otherwise `total_amount * (t - (start + cliff)) / (duration - cliff)`,
//!   using integer (floor) division so claims never round up.
//!
//! Authorization model:
//! - `create_schedule` requires the beneficiary.
//! - `claim` requires the beneficiary.
//! - `claimable` and `get_status` are read-only views.
//!
//! ## Settlement (load-bearing)
//!
//! The contract custodies the configured SEP-41 token and `claim` settles
//! through it: the newly claimable amount is transferred from this contract
//! to the beneficiary, and the schedule is written only after that transfer
//! succeeds. The transfer-before-state ordering mirrors
//! `crates/escrow/src/lib.rs` — a failed transfer (empty contract balance,
//! undeployed token) returns [`ForgeError::TokenTransferFailed`] with
//! `claimed` and `status` untouched. A zero-claim call exits before the
//! transfer, so no empty transfers are ever issued. Soroban's frame rollback
//! is the outer atomicity guarantee: any `Err` returned from `claim` reverts
//! the whole invocation, including sub-invocations.
//!
//! The `Revoked` status is reserved for a revocation method that lands in a
//! follow-up; it is not reachable through the current public interface.

#[cfg(test)]
extern crate std;

use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::{contract, contractclient, contractimpl, contracttype, token, Address, Env};

/// Public interface for the Soroban Forge vesting contract.
///
/// Declared as a `contractclient` trait so SDK consumers (and the TypeScript
/// SDK generator) get a strongly-typed client without coupling to the
/// implementation crate.
#[contractclient(name = "SorobanForgeVestingClient")]
pub trait SorobanForgeVesting {
    /// Create a new vesting schedule for `beneficiary`.
    ///
    /// `cliff` and `duration` are seconds measured from creation
    /// (`cliff <= duration`, `duration > 0`, `total_amount > 0`). Returns the
    /// stable schedule id.
    fn create_schedule(
        env: Env,
        beneficiary: Address,
        token: Address,
        total_amount: i128,
        cliff: u64,
        duration: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Claim tokens that have vested as of the current ledger time.
    ///
    /// Requires the beneficiary. Transfers the exact vested-but-unclaimed
    /// amount from this contract to the beneficiary, then records the claim;
    /// returns `0` without issuing a transfer when nothing is claimable.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::NotFound`] — no schedule with this id.
    /// * [`ForgeError::TokenTransferFailed`] — the token contract rejected
    ///   the payout (insufficient contract balance, undeployed token).
    /// * [`ForgeError::ArithmeticOverflow`] — the claimed total overflowed.
    fn claim(env: Env, schedule_id: u64) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Return the amount currently claimable by `schedule_id` (read-only).
    fn claimable(
        env: Env,
        schedule_id: u64,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Read the current lifecycle status of `schedule_id` (read-only).
    fn get_status(
        env: Env,
        schedule_id: u64,
    ) -> Result<VestingStatus, soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a vesting schedule.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VestingStatus {
    /// Before the cliff has been reached.
    Locked,
    /// Past the cliff; tokens are vesting linearly.
    Vesting,
    /// Fully vested and claimed.
    Completed,
    /// Schedule was terminated before completion (reserved).
    Revoked,
}

/// A single token-vesting schedule.
#[contracttype]
#[derive(Clone, Debug)]
pub struct VestingSchedule {
    /// Recipient of the vested tokens.
    pub beneficiary: Address,
    /// Token contract whose balance is drawn down.
    pub token: Address,
    /// Total amount to vest linearly between `cliff` and `duration`.
    pub total_amount: i128,
    /// Ledger timestamp at which vesting begins (creation time).
    pub start: u64,
    /// Seconds after `start` at which claims become possible.
    pub cliff: u64,
    /// Seconds after `start` at which the schedule is fully vested.
    pub duration: u64,
    /// Amount already claimed by the beneficiary.
    pub claimed: i128,
    /// Current lifecycle state.
    pub status: VestingStatus,
}

/// Instance-storage keys.
#[contracttype]
enum DataKey {
    /// The vesting record for `u64` id.
    Schedule(u64),
    /// Monotonic id counter.
    Count,
}

/// The deployable vesting contract.
#[contract]
pub struct Vesting;

#[contractimpl]
impl Vesting {
    /// Create a new vesting schedule and return its stable id.
    ///
    /// Requires `total_amount > 0`, `duration > 0`, and `cliff <= duration`.
    /// The beneficiary is authorized at creation time.
    pub fn create_schedule(
        env: Env,
        beneficiary: Address,
        token: Address,
        total_amount: i128,
        cliff: u64,
        duration: u64,
    ) -> Result<u64, ForgeError> {
        if total_amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        if duration == 0 {
            return Err(ForgeError::InvalidInput);
        }
        if cliff > duration {
            return Err(ForgeError::InvalidInput);
        }
        beneficiary.require_auth();

        let id = Self::next_id(&env)?;
        let start = env.ledger().timestamp();
        let mut schedule = VestingSchedule {
            beneficiary,
            token,
            total_amount,
            start,
            cliff,
            duration,
            claimed: 0,
            status: VestingStatus::Locked,
        };
        // Derive the initial status from time (cliff == 0 starts `Vesting`).
        schedule.status = Self::current_status(&schedule, start)?;
        env.storage()
            .instance()
            .set(&DataKey::Schedule(id), &schedule);
        Ok(id)
    }

    /// Claim the vested-but-unclaimed amount.
    ///
    /// Requires the beneficiary. Returns exactly what vested since the last
    /// claim (or `0` when nothing is claimable), so repeated claims can never
    /// overpay or underpay.
    ///
    /// Ordering: the SEP-41 transfer runs **before** the schedule write —
    /// see the module docs. A zero-claim call returns before either.
    pub fn claim(env: Env, schedule_id: u64) -> Result<i128, ForgeError> {
        let mut schedule = Self::get_schedule(&env, schedule_id)?;
        // NOTE: when a revocation method lands, `claim` must be gated on
        // `schedule.status != Revoked`; the status is currently unreachable.
        schedule.beneficiary.require_auth();

        let now = env.ledger().timestamp();
        let amount = Self::claimable_amount(&schedule, now)?;
        if amount == 0 {
            return Ok(0);
        }

        // Pay the beneficiary before recording anything: a failed transfer
        // returns TokenTransferFailed with `claimed`/`status` untouched.
        transfer_from_contract(&env, &schedule.token, &schedule.beneficiary, amount)?;

        schedule.claimed = schedule
            .claimed
            .checked_add(amount)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        schedule.status = Self::current_status(&schedule, now)?;
        env.storage()
            .instance()
            .set(&DataKey::Schedule(schedule_id), &schedule);
        Ok(amount)
    }

    /// Amount currently claimable (read-only view; no state change).
    pub fn claimable(env: Env, schedule_id: u64) -> Result<i128, ForgeError> {
        let schedule = Self::get_schedule(&env, schedule_id)?;
        Self::claimable_amount(&schedule, env.ledger().timestamp())
    }

    /// Read the current lifecycle status (read-only view).
    ///
    /// The status is derived from the ledger time and claimed amount rather
    /// than the stored field, so it is always current between claims.
    pub fn get_status(env: Env, schedule_id: u64) -> Result<VestingStatus, ForgeError> {
        let schedule = Self::get_schedule(&env, schedule_id)?;
        Self::current_status(&schedule, env.ledger().timestamp())
    }

    /// Allocate the next monotonic schedule id.
    fn next_id(env: &Env) -> Result<u64, ForgeError> {
        let count: u64 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let id = count.checked_add(1).ok_or(ForgeError::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::Count, &id);
        Ok(id)
    }

    fn get_schedule(env: &Env, schedule_id: u64) -> Result<VestingSchedule, ForgeError> {
        env.storage()
            .instance()
            .get(&DataKey::Schedule(schedule_id))
            .ok_or(ForgeError::NotFound)
    }

    /// Derive the lifecycle status from ledger time and claimed amount.
    fn current_status(schedule: &VestingSchedule, now: u64) -> Result<VestingStatus, ForgeError> {
        if schedule.claimed >= schedule.total_amount {
            return Ok(VestingStatus::Completed);
        }
        let cliff_time = schedule
            .start
            .checked_add(schedule.cliff)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        if now < cliff_time {
            return Ok(VestingStatus::Locked);
        }
        Ok(VestingStatus::Vesting)
    }

    /// Vested amount at ledger time `now`, using floor division so claims
    /// never round up.
    fn vested_amount(schedule: &VestingSchedule, now: u64) -> Result<i128, ForgeError> {
        let cliff_time = schedule
            .start
            .checked_add(schedule.cliff)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        if now < cliff_time {
            return Ok(0);
        }
        let end_time = schedule
            .start
            .checked_add(schedule.duration)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        if now >= end_time {
            return Ok(schedule.total_amount);
        }

        // `cliff <= duration` is enforced at creation, so the period is
        // non-negative; a zero period (cliff == duration) means everything
        // vests at once, which the `now >= end_time` branch above already
        // returned. Guard defensively against division by zero.
        let period = end_time - cliff_time;
        if period == 0 {
            return Ok(schedule.total_amount);
        }
        let elapsed = now - cliff_time;
        let vested = schedule
            .total_amount
            .checked_mul(elapsed as i128)
            .ok_or(ForgeError::ArithmeticOverflow)?
            / period as i128;
        Ok(vested)
    }

    /// Claimable amount at ledger time `now` (vested minus claimed).
    fn claimable_amount(schedule: &VestingSchedule, now: u64) -> Result<i128, ForgeError> {
        let vested = Self::vested_amount(schedule, now)?;
        // By construction `claimed` never exceeds `vested`, so the subtraction
        // cannot underflow; use checked arithmetic to fail loudly if the
        // invariant is ever broken.
        vested
            .checked_sub(schedule.claimed)
            .ok_or(ForgeError::ArithmeticOverflow)
    }
}

/// Move `amount` of `token` from this contract to `to`.
///
/// Token failures are bucketed into [`ForgeError::TokenTransferFailed`]
/// rather than forwarded — the same policy as escrow: a client receiving
/// `Error(Contract, #N)` cannot know whether `N` came from the token or this
/// contract, and the root cause remains visible in the transaction's
/// diagnostic events.
fn transfer_from_contract(
    env: &Env,
    token: &Address,
    to: &Address,
    amount: i128,
) -> Result<(), ForgeError> {
    match token::TokenClient::new(env, token).try_transfer(
        &env.current_contract_address(),
        to,
        &amount,
    ) {
        Ok(Ok(())) => Ok(()),
        // Token returned a typed error (insufficient balance, custom token
        // logic) or the host aborted (most commonly an undeployed token
        // address). The raw discriminant is intentionally discarded.
        _ => Err(ForgeError::TokenTransferFailed),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod authz;

#[cfg(test)]
mod props;
