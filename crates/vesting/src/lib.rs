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
mod tests {
    use super::*;
    use soroban_forge_test_utils::TestAccounts;
    use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
    use soroban_sdk::{Env, IntoVal, InvokeError};

    const START: u64 = 1_000_000;
    const CLIFF: u64 = 1_000;
    const DURATION: u64 = 4_000;
    const TOTAL: i128 = 10_000;

    /// Build a fresh env with mocked auths, a registered contract, and named
    /// accounts. The generated client borrows the env, so it cannot be
    /// returned from a helper.
    macro_rules! setup {
        () => {{
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(START);

            // Real Stellar Asset Contract — the same fixture escrow uses.
            let admin = Address::generate(&env);
            let sac = env.register_stellar_asset_contract_v2(admin);
            let token = sac.address();
            let token_admin = StellarAssetClient::new(&env, &token);
            let token_client = TokenClient::new(&env, &token);
            let contract_id = env.register(Vesting, ());
            let client = SorobanForgeVestingClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);
            // Fund the schedule contract up front with the full allocation,
            // the way a deployment is topped up before schedules run.
            token_admin.mint(&contract_id, &TOTAL);
            (env, token, token_client, contract_id, client, accounts)
        }};
    }

    /// Assert that a `try_` call aborted on authorization (the host aborts the
    /// whole invocation when the armed envelope does not match).
    macro_rules! assert_auth_abort {
        ($res:expr) => {
            assert!(
                matches!($res, Err(Err(InvokeError::Abort))),
                "expected auth abort, got {:?}",
                $res
            );
        };
    }

    // NOTE: In soroban-sdk 27.0.6, entrypoint `require_auth` checks are verified
    // via `env.mock_auths` (enforce mode). An unauthorized caller or a signature
    // with mismatched arguments aborts the invocation with `Err(Err(InvokeError::Abort))`.
    // Contract self-authorization for token settlement is implicit in the host:
    // the executing contract's own transfer is auto-approved and requires no
    // separate user signature frame.

    fn create(
        client: &SorobanForgeVestingClient<'_>,
        token: &Address,
        accounts: &TestAccounts,
    ) -> u64 {
        client.create_schedule(&accounts.user1, token, &TOTAL, &CLIFF, &DURATION)
    }

    #[test]
    fn create_schedule_succeeds_and_is_locked() {
        let (_env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        assert_eq!(client.get_status(&id), VestingStatus::Locked);
        assert_eq!(client.claimable(&id), 0);
    }

    #[test]
    fn create_schedule_assigns_distinct_ids() {
        let (_env, token, _tc, _cid, client, accounts) = setup!();
        let id1 = create(&client, &token, &accounts);
        let id2 = create(&client, &token, &accounts);
        assert_ne!(id1, id2);
    }

    #[test]
    fn create_schedule_without_cliff_starts_vesting() {
        let (_env, _token, _tc, _cid, client, accounts) = setup!();
        let id = client.create_schedule(
            &accounts.user1,
            &accounts.validator,
            &TOTAL,
            &0_u64,
            &DURATION,
        );
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    }

    #[test]
    fn create_schedule_rejects_zero_total() {
        let (_env, _token, _tc, _cid, client, accounts) = setup!();
        let err = client
            .try_create_schedule(
                &accounts.user1,
                &accounts.validator,
                &0_i128,
                &CLIFF,
                &DURATION,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn create_schedule_rejects_zero_duration() {
        let (_env, _token, _tc, _cid, client, accounts) = setup!();
        let err = client
            .try_create_schedule(&accounts.user1, &accounts.validator, &TOTAL, &CLIFF, &0_u64)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn create_schedule_rejects_cliff_after_duration() {
        let (_env, _token, _tc, _cid, client, accounts) = setup!();
        let err = client
            .try_create_schedule(
                &accounts.user1,
                &accounts.validator,
                &TOTAL,
                &5_000_u64,
                &4_000_u64,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn claimable_before_cliff_is_zero() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        // Halfway between start and the cliff.
        env.ledger().set_timestamp(START + CLIFF / 2);
        assert_eq!(client.claimable(&id), 0);
    }

    #[test]
    fn claimable_at_cliff_is_zero() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger().set_timestamp(START + CLIFF);
        assert_eq!(client.claimable(&id), 0);
    }

    #[test]
    fn claim_before_cliff_returns_zero() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger().set_timestamp(START + CLIFF / 2);
        assert_eq!(client.claim(&id), 0);
    }

    #[test]
    fn claimable_midway_is_half() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        // Halfway through the vesting window (cliff .. duration).
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claimable(&id), TOTAL / 2);
    }

    #[test]
    fn claimable_at_duration_is_full() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger().set_timestamp(START + DURATION);
        assert_eq!(client.claimable(&id), TOTAL);
    }

    #[test]
    fn claimable_after_duration_is_full() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger().set_timestamp(START + DURATION + 1);
        assert_eq!(client.claimable(&id), TOTAL);
    }

    #[test]
    fn claim_pays_exact_amount() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claim(&id), TOTAL / 2);
    }

    #[test]
    fn repeated_claims_never_overpay_or_underpay() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // Claim half at the midway point.
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claim(&id), TOTAL / 2);
        assert_eq!(client.claimable(&id), 0);

        // Advance past the end; the remaining half becomes claimable.
        env.ledger().set_timestamp(START + DURATION + 100);
        assert_eq!(client.claim(&id), TOTAL - TOTAL / 2);
        assert_eq!(client.claimable(&id), 0);

        // A further claim is a no-op.
        assert_eq!(client.claim(&id), 0);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
    }

    #[test]
    fn claim_after_end_completes_status() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger().set_timestamp(START + DURATION + 1);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claim(&id), TOTAL);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
    }

    #[test]
    fn claim_without_cliff_vests_from_start() {
        let (env, _token, _tc, _cid, client, accounts) = setup!();
        let id = client.create_schedule(
            &accounts.user1,
            &accounts.validator,
            &TOTAL,
            &0_u64,
            &DURATION,
        );
        env.ledger().set_timestamp(START + DURATION / 2);
        assert_eq!(client.claimable(&id), TOTAL / 2);
    }

    #[test]
    fn cliff_equals_duration_vests_at_once() {
        let (env, _token, _tc, _cid, client, accounts) = setup!();
        let id = client.create_schedule(
            &accounts.user1,
            &accounts.validator,
            &TOTAL,
            &DURATION,
            &DURATION,
        );
        env.ledger().set_timestamp(START + DURATION - 1);
        assert_eq!(client.claimable(&id), 0);
        env.ledger().set_timestamp(START + DURATION);
        assert_eq!(client.claimable(&id), TOTAL);
    }

    #[test]
    fn claim_missing_schedule_is_not_found() {
        let (_env, _token, _tc, _cid, client, _accounts) = setup!();
        let err = client.try_claim(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn claimable_missing_schedule_is_not_found() {
        let (_env, _token, _tc, _cid, client, _accounts) = setup!();
        let err = client.try_claimable(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_status_missing_schedule_is_not_found() {
        let (_env, _token, _tc, _cid, client, _accounts) = setup!();
        let err = client.try_get_status(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn claimable_overflow_is_reported() {
        let (env, _token, _tc, _cid, client, accounts) = setup!();
        // A huge total with a non-trivial elapsed time overflows the
        // intermediate `total * elapsed` product.
        let id = client.create_schedule(
            &accounts.user1,
            &accounts.validator,
            &i128::MAX,
            &0_u64,
            &1_000_u64,
        );
        env.ledger().set_timestamp(START + 500);
        let err = client.try_claimable(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::ArithmeticOverflow);
    }

    // -------------------------------------------------------------------
    // Settlement: claim pays real SEP-41 tokens
    // -------------------------------------------------------------------

    #[test]
    fn claim_transfers_vested_tokens_to_beneficiary() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // Halfway through the vesting window: 5_000 claimable.
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claim(&id), TOTAL / 2);

        // Tokens actually moved, and the schedule recorded the claim.
        assert_eq!(tc.balance(&accounts.user1), TOTAL / 2);
        assert_eq!(tc.balance(&contract_id), TOTAL - TOTAL / 2);
        assert_eq!(client.claimable(&id), 0);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    }

    #[test]
    fn final_claim_settles_remainder_and_completes() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claim(&id), TOTAL / 2);

        env.ledger().set_timestamp(START + DURATION + 100);
        assert_eq!(client.claim(&id), TOTAL - TOTAL / 2);

        // Full allocation paid out; contract drained; schedule completed.
        assert_eq!(tc.balance(&accounts.user1), TOTAL);
        assert_eq!(tc.balance(&contract_id), 0);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
        assert_eq!(client.claimable(&id), 0);

        // A repeated claim after completion is a silent no-op: no transfer.
        assert_eq!(client.claim(&id), 0);
        assert_eq!(tc.balance(&accounts.user1), TOTAL);
        assert_eq!(tc.balance(&contract_id), 0);
    }

    #[test]
    fn repeated_claims_settle_token_balances_each_time() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // Three separate claims across the window; each moves exactly the
        // newly claimable amount and nothing more.
        env.ledger().set_timestamp(START + CLIFF + 1_000);
        assert_eq!(client.claim(&id), 3_333);
        assert_eq!(tc.balance(&accounts.user1), 3_333);

        env.ledger().set_timestamp(START + CLIFF + 2_000);
        assert_eq!(client.claim(&id), 3_333);
        assert_eq!(tc.balance(&accounts.user1), 6_666);

        env.ledger().set_timestamp(START + DURATION + 1);
        assert_eq!(client.claim(&id), 3_334);
        assert_eq!(tc.balance(&accounts.user1), TOTAL);
        assert_eq!(tc.balance(&contract_id), 0);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
    }

    #[test]
    fn failed_transfer_leaves_claim_unchanged() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        // Schedule promises double what the contract actually holds.
        let id = client.create_schedule(&accounts.user1, &token, &(TOTAL * 2), &0_u64, &DURATION);
        env.ledger().set_timestamp(START + DURATION + 1);
        assert_eq!(client.claimable(&id), TOTAL * 2);

        let err = client.try_claim(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::TokenTransferFailed);

        // Nothing moved, nothing recorded: balances and schedule unchanged.
        assert_eq!(tc.balance(&accounts.user1), 0);
        assert_eq!(tc.balance(&contract_id), TOTAL);
        assert_eq!(client.claimable(&id), TOTAL * 2);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    }

    #[test]
    fn zero_claim_issues_no_token_transfer() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // Before the cliff: returns 0, moves nothing.
        env.ledger().set_timestamp(START + CLIFF / 2);
        assert_eq!(client.claim(&id), 0);
        assert_eq!(tc.balance(&accounts.user1), 0);
        assert_eq!(tc.balance(&contract_id), TOTAL);

        // A second immediate claim right after a payout is also a no-op.
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claim(&id), TOTAL / 2);
        assert_eq!(client.claim(&id), 0);
        assert_eq!(tc.balance(&accounts.user1), TOTAL / 2);
        assert_eq!(tc.balance(&contract_id), TOTAL - TOTAL / 2);
    }

    #[test]
    fn floor_division_residue_stays_claimable_until_final_claim() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // 1_000 / 3_000 through the window:
        // 10_000 * 1_000 / 3_000 = 3_333 (floored) — one stroop of residue
        // stays behind, and the final claim pays it out in full.
        env.ledger().set_timestamp(START + CLIFF + 1_000);
        let first = client.claim(&id);
        assert_eq!(first, 3_333);

        env.ledger().set_timestamp(START + DURATION + 1);
        let rest = client.claim(&id);
        assert_eq!(first + rest, TOTAL);
        assert_eq!(tc.balance(&accounts.user1), TOTAL);
        assert_eq!(tc.balance(&contract_id), 0);
    }

    // -------------------------------------------------------------------
    // Authorization & Negative-Auth Verification
    // -------------------------------------------------------------------

    #[test]
    fn create_schedule_accepts_beneficiary_auth() {
        let (env, token, _tc, contract_id, client, accounts) = setup!();
        let beneficiary = &accounts.user1;
        env.mock_auths(&[MockAuth {
            address: beneficiary,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "create_schedule",
                args: (beneficiary, &token, &TOTAL, &CLIFF, &DURATION).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let id = client
            .try_create_schedule(beneficiary, &token, &TOTAL, &CLIFF, &DURATION)
            .expect("outer ok")
            .expect("create_schedule ok");
        assert_eq!(id, 1);
    }

    #[test]
    fn create_schedule_rejects_non_beneficiary_auth() {
        let (env, token, _tc, contract_id, client, accounts) = setup!();
        let beneficiary = &accounts.user1;
        let attacker = &accounts.user2;

        env.mock_auths(&[MockAuth {
            address: attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "create_schedule",
                args: (beneficiary, &token, &TOTAL, &CLIFF, &DURATION).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let res = client.try_create_schedule(beneficiary, &token, &TOTAL, &CLIFF, &DURATION);
        assert_auth_abort!(res);
    }

    #[test]
    fn claim_accepts_beneficiary_auth() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger().set_timestamp(START + DURATION);

        env.mock_auths(&[MockAuth {
            address: &accounts.user1,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "claim",
                args: (id,).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let res = client.try_claim(&id).expect("outer ok").expect("claim ok");
        assert_eq!(res, TOTAL);
        assert_eq!(tc.balance(&accounts.user1), TOTAL);
    }

    #[test]
    fn claim_rejects_non_beneficiary_auth() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);
        env.ledger().set_timestamp(START + DURATION);

        env.mock_auths(&[MockAuth {
            address: &accounts.user2,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "claim",
                args: (id,).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let res = client.try_claim(&id);
        assert_auth_abort!(res);
        assert_eq!(tc.balance(&accounts.user1), 0);
        assert_eq!(tc.balance(&contract_id), TOTAL);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claimable(&id), TOTAL);
    }

    // -------------------------------------------------------------------
    // Boundary, Cliff == Duration, and Timing Tests
    // -------------------------------------------------------------------

    #[test]
    fn boundary_cliff_and_duration_step_progression() {
        let (env, token, _tc, _cid, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // At creation start:
        env.ledger().set_timestamp(START);
        assert_eq!(client.get_status(&id), VestingStatus::Locked);
        assert_eq!(client.claimable(&id), 0);

        // 1 second before cliff:
        env.ledger().set_timestamp(START + CLIFF - 1);
        assert_eq!(client.get_status(&id), VestingStatus::Locked);
        assert_eq!(client.claimable(&id), 0);

        // At cliff boundary:
        env.ledger().set_timestamp(START + CLIFF);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claimable(&id), 0);

        // 1 second after cliff: floor(10_000 * 1 / 3_000) = 3
        env.ledger().set_timestamp(START + CLIFF + 1);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claimable(&id), 3);

        // 1 second before full duration: floor(10_000 * 2_999 / 3_000) = 9_996
        env.ledger().set_timestamp(START + DURATION - 1);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claimable(&id), 9_996);

        // Exactly at duration:
        env.ledger().set_timestamp(START + DURATION);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claimable(&id), TOTAL);

        // After duration:
        env.ledger().set_timestamp(START + DURATION + 10_000);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claimable(&id), TOTAL);
    }

    #[test]
    fn interleaved_partial_claims_across_arbitrary_timestamps() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // Five sequential claim points:
        // Period is 3_000 seconds, total is 10_000.
        // Step 1: elapsed 300 -> 10_000 * 300 / 3_000 = 1_000.
        env.ledger().set_timestamp(START + CLIFF + 300);
        assert_eq!(client.claim(&id), 1_000);
        assert_eq!(tc.balance(&accounts.user1), 1_000);
        assert_eq!(client.claimable(&id), 0);

        // Step 2: elapsed 750 -> 10_000 * 750 / 3_000 = 2_500 vested. Claimable = 2_500 - 1_000 = 1_500.
        env.ledger().set_timestamp(START + CLIFF + 750);
        assert_eq!(client.claim(&id), 1_500);
        assert_eq!(tc.balance(&accounts.user1), 2_500);
        assert_eq!(client.claimable(&id), 0);

        // Step 3: elapsed 1_500 -> 10_000 * 1_500 / 3_000 = 5_000 vested. Claimable = 5_000 - 2_500 = 2_500.
        env.ledger().set_timestamp(START + CLIFF + 1_500);
        assert_eq!(client.claim(&id), 2_500);
        assert_eq!(tc.balance(&accounts.user1), 5_000);
        assert_eq!(client.claimable(&id), 0);

        // Step 4: elapsed 2_250 -> 10_000 * 2_250 / 3_000 = 7_500 vested. Claimable = 7_500 - 5_000 = 2_500.
        env.ledger().set_timestamp(START + CLIFF + 2_250);
        assert_eq!(client.claim(&id), 2_500);
        assert_eq!(tc.balance(&accounts.user1), 7_500);
        assert_eq!(client.claimable(&id), 0);

        // Step 5: elapsed 3_000 (end of duration) -> full 10_000 vested. Remainder = 2_500.
        env.ledger().set_timestamp(START + DURATION);
        assert_eq!(client.claim(&id), 2_500);
        assert_eq!(tc.balance(&accounts.user1), TOTAL);
        assert_eq!(tc.balance(&contract_id), 0);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
        assert_eq!(client.claimable(&id), 0);
    }

    #[test]
    fn repeated_zero_claims_at_and_before_cliff() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // Repeated claims before cliff:
        env.ledger().set_timestamp(START + 100);
        assert_eq!(client.claim(&id), 0);
        assert_eq!(client.claim(&id), 0);
        assert_eq!(client.claim(&id), 0);
        assert_eq!(tc.balance(&accounts.user1), 0);
        assert_eq!(tc.balance(&contract_id), TOTAL);

        // Repeated claims at cliff:
        env.ledger().set_timestamp(START + CLIFF);
        assert_eq!(client.claim(&id), 0);
        assert_eq!(client.claim(&id), 0);
        assert_eq!(tc.balance(&accounts.user1), 0);
        assert_eq!(tc.balance(&contract_id), TOTAL);
    }

    #[test]
    fn timestamp_u64_max_edge_case() {
        let (env, token, tc, contract_id, client, accounts) = setup!();
        let id = create(&client, &token, &accounts);

        // Ledger timestamp at maximum u64 value:
        env.ledger().set_timestamp(u64::MAX);
        assert_eq!(client.claimable(&id), TOTAL);
        assert_eq!(client.claim(&id), TOTAL);
        assert_eq!(tc.balance(&accounts.user1), TOTAL);
        assert_eq!(tc.balance(&contract_id), 0);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
    }

    #[test]
    fn total_amount_i128_max_at_duration_succeeds() {
        let (env, _token, _tc, _cid, client, accounts) = setup!();
        let id = client.create_schedule(
            &accounts.user1,
            &accounts.validator,
            &i128::MAX,
            &0_u64,
            &1_000_u64,
        );
        // At or after duration, vested_amount returns total_amount directly
        // without computing total_amount * elapsed / period, so it does not overflow.
        env.ledger().set_timestamp(START + 1_000);
        assert_eq!(client.claimable(&id), i128::MAX);

        env.ledger().set_timestamp(START + 2_000);
        assert_eq!(client.claimable(&id), i128::MAX);
    }

    #[test]
    fn start_plus_duration_u64_overflow_reported() {
        let (env, _token, _tc, _cid, client, accounts) = setup!();
        // A schedule whose duration would cause start + duration to overflow u64.
        let id = client.create_schedule(
            &accounts.user1,
            &accounts.validator,
            &TOTAL,
            &0_u64,
            &u64::MAX,
        );
        env.ledger().set_timestamp(START + 100);
        let err = client.try_claimable(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::ArithmeticOverflow);

        let claim_err = client.try_claim(&id).unwrap_err().unwrap();
        assert_eq!(claim_err, ForgeError::ArithmeticOverflow);
    }

    #[test]
    fn monotonic_id_counter_overflow_reported() {
        let (env, token, _tc, contract_id, client, accounts) = setup!();
        // Set Count key to u64::MAX directly in contract storage:
        env.as_contract(&contract_id, || {
            env.storage().instance().set(&DataKey::Count, &u64::MAX);
        });

        let err = client
            .try_create_schedule(&accounts.user1, &token, &TOTAL, &CLIFF, &DURATION)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::ArithmeticOverflow);
    }
}
