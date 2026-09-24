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
//! - `touch_ttl` is permissionless (keeper entrypoint).
//!
//! The `Revoked` status is reserved for a revocation method that lands in a
//! follow-up; it is not reachable through the current public interface. Token
//! settlement (SAC transfers) is intentionally out of scope for this
//! iteration: the contract tracks state and authorization, not balances.
//!
//! ## Storage and TTL
//!
//! Each schedule is its own **persistent** entry (`DataKey::Schedule(id)`)
//! so the byte budget scales per record and a schedule's lifetime does not
//! depend on the contract instance's. The only instance entry is the id
//! counter. Every write bumps the entry's TTL with the standard
//! threshold/extend-to pattern (same constants as escrow), and `touch_ttl`
//! is a permissionless keeper entrypoint for schedules that sit idle near
//! expiry. Read-only views never extend a TTL.

#[cfg(test)]
extern crate std;

use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env};

/// Ledger-time constants for TTL bumps.
///
/// One ledger closes roughly every 5 seconds, so 17,280 ledgers ≈ 1 day.
/// `BUMP_AMOUNT` is the lifetime written on every touch; `BUMP_THRESHOLD`
/// is how close to expiry an entry must be before a bump applies. Vesting
/// windows routinely exceed 30 days, so long-idle schedules rely on
/// `touch_ttl` (or a claim) to stay live.
mod ttl {
    pub const DAY_IN_LEDGERS: u32 = 17_280;
    /// Lifetime applied on every TTL touch.
    pub const BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
    /// Bump only when the entry is within this window of expiring.
    pub const BUMP_THRESHOLD: u32 = BUMP_AMOUNT - DAY_IN_LEDGERS;
}

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
    /// Requires the beneficiary. Returns the exact vested-but-unclaimed
    /// amount, or `0` when there is nothing to claim.
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

    /// Permissionless TTL keeper: bumps the schedule entry's TTL to the
    /// [`ttl::BUMP_AMOUNT`] horizon when it falls inside
    /// [`ttl::BUMP_THRESHOLD`]. Call periodically for schedules that must
    /// outlive their entry's current TTL. Costs fees; changes nothing
    /// else.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::NotFound`] — no schedule with this id.
    fn touch_ttl(env: Env, schedule_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
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

/// Storage keys. Schedules are per-id **persistent** entries so the byte
/// budget scales per record; only the id counter lives in instance storage
/// (one small entry, written once per creation).
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
            .persistent()
            .set(&DataKey::Schedule(id), &schedule);
        bump_entry(&env, &DataKey::Schedule(id));
        Ok(id)
    }

    /// Claim the vested-but-unclaimed amount.
    ///
    /// Requires the beneficiary. Returns exactly what vested since the last
    /// claim (or `0` when nothing is claimable), so repeated claims can never
    /// overpay or underpay.
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

        schedule.claimed = schedule
            .claimed
            .checked_add(amount)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        schedule.status = Self::current_status(&schedule, now)?;
        env.storage()
            .persistent()
            .set(&DataKey::Schedule(schedule_id), &schedule);
        bump_entry(&env, &DataKey::Schedule(schedule_id));
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

    /// Permissionless keeper: bump the schedule entry's TTL without changing
    /// any state. The existence check is deliberate — touching a missing id
    /// must fail loudly so a keeper can distinguish "extended" from "no such
    /// schedule".
    pub fn touch_ttl(env: Env, schedule_id: u64) -> Result<(), ForgeError> {
        Self::get_schedule(&env, schedule_id)?;
        bump_entry(&env, &DataKey::Schedule(schedule_id));
        Ok(())
    }

    /// Allocate the next monotonic schedule id. Instance storage: one small
    /// entry, written once per creation.
    fn next_id(env: &Env) -> Result<u64, ForgeError> {
        let count: u64 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let id = count.checked_add(1).ok_or(ForgeError::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::Count, &id);
        Ok(id)
    }

    /// Load a schedule by id. A missing id is `NotFound`; the lookup never
    /// writes, so it cannot create an entry as a side effect.
    fn get_schedule(env: &Env, schedule_id: u64) -> Result<VestingSchedule, ForgeError> {
        env.storage()
            .persistent()
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

/// Bump a persistent entry's TTL to the [`ttl::BUMP_AMOUNT`] horizon when
/// it falls inside [`ttl::BUMP_THRESHOLD`]. The standard threshold/extend
/// pattern: cheap no-op while the entry is fresh, decisive near expiry.
fn bump_entry(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, ttl::BUMP_THRESHOLD, ttl::BUMP_AMOUNT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_forge_test_utils::TestAccounts;
    use soroban_sdk::testutils::Ledger as _;
    use soroban_sdk::Env;

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
            let contract_id = env.register(Vesting, ());
            let client = SorobanForgeVestingClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);
            (env, client, accounts)
        }};
    }

    // NOTE: negative authorization tests (calling `require_auth` without a
    // matching signature) are not runnable in-process with soroban-sdk 21.5.1:
    // the host raises a non-unwinding panic that aborts the test binary. They
    // are tracked in the security-invariant test backlog (Issue 7).

    fn create(client: &SorobanForgeVestingClient<'_>, accounts: &TestAccounts) -> u64 {
        client.create_schedule(
            &accounts.user1,
            &accounts.validator,
            &TOTAL,
            &CLIFF,
            &DURATION,
        )
    }

    #[test]
    fn create_schedule_succeeds_and_is_locked() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        assert_eq!(client.get_status(&id), VestingStatus::Locked);
        assert_eq!(client.claimable(&id), 0);
    }

    #[test]
    fn create_schedule_assigns_distinct_ids() {
        let (_env, client, accounts) = setup!();
        let id1 = create(&client, &accounts);
        let id2 = create(&client, &accounts);
        assert_ne!(id1, id2);
    }

    #[test]
    fn create_schedule_without_cliff_starts_vesting() {
        let (_env, client, accounts) = setup!();
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
        let (_env, client, accounts) = setup!();
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
        let (_env, client, accounts) = setup!();
        let err = client
            .try_create_schedule(&accounts.user1, &accounts.validator, &TOTAL, &CLIFF, &0_u64)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn create_schedule_rejects_cliff_after_duration() {
        let (_env, client, accounts) = setup!();
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
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        // Halfway between start and the cliff.
        env.ledger().set_timestamp(START + CLIFF / 2);
        assert_eq!(client.claimable(&id), 0);
    }

    #[test]
    fn claimable_at_cliff_is_zero() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        env.ledger().set_timestamp(START + CLIFF);
        assert_eq!(client.claimable(&id), 0);
    }

    #[test]
    fn claim_before_cliff_returns_zero() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        env.ledger().set_timestamp(START + CLIFF / 2);
        assert_eq!(client.claim(&id), 0);
    }

    #[test]
    fn claimable_midway_is_half() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        // Halfway through the vesting window (cliff .. duration).
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claimable(&id), TOTAL / 2);
    }

    #[test]
    fn claimable_at_duration_is_full() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        env.ledger().set_timestamp(START + DURATION);
        assert_eq!(client.claimable(&id), TOTAL);
    }

    #[test]
    fn claimable_after_duration_is_full() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        env.ledger().set_timestamp(START + DURATION + 1);
        assert_eq!(client.claimable(&id), TOTAL);
    }

    #[test]
    fn claim_pays_exact_amount() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
        assert_eq!(client.claim(&id), TOTAL / 2);
    }

    #[test]
    fn repeated_claims_never_overpay_or_underpay() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);

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
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        env.ledger().set_timestamp(START + DURATION + 1);
        assert_eq!(client.get_status(&id), VestingStatus::Vesting);
        assert_eq!(client.claim(&id), TOTAL);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
    }

    #[test]
    fn claim_without_cliff_vests_from_start() {
        let (env, client, accounts) = setup!();
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
        let (env, client, accounts) = setup!();
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
        let (_env, client, _accounts) = setup!();
        let err = client.try_claim(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn claimable_missing_schedule_is_not_found() {
        let (_env, client, _accounts) = setup!();
        let err = client.try_claimable(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_status_missing_schedule_is_not_found() {
        let (_env, client, _accounts) = setup!();
        let err = client.try_get_status(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn claimable_overflow_is_reported() {
        let (env, client, accounts) = setup!();
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
    // Persistent storage and TTL
    // -------------------------------------------------------------------

    use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};

    /// Remaining TTL (in ledgers) of the schedule's persistent entry.
    fn schedule_ttl(env: &Env, client: &SorobanForgeVestingClient<'_>, id: u64) -> u32 {
        env.as_contract(&client.address, || {
            env.storage().persistent().get_ttl(&DataKey::Schedule(id))
        })
    }

    /// Remaining TTL (in ledgers) of the contract instance entry.
    fn instance_ttl(env: &Env, client: &SorobanForgeVestingClient<'_>) -> u32 {
        env.as_contract(&client.address, || env.storage().instance().get_ttl())
    }

    /// Read the raw stored record, bypassing the contract entrypoints.
    fn stored_schedule(
        env: &Env,
        client: &SorobanForgeVestingClient<'_>,
        id: u64,
    ) -> Option<VestingSchedule> {
        env.as_contract(&client.address, || {
            env.storage().persistent().get(&DataKey::Schedule(id))
        })
    }

    // How TTL is observed here: in test mode the SDK 27 host
    // (soroban-env-host 27.0.1, `Storage::handle_maybe_expired_entry`, run
    // from `prepare_read_only_access`) auto-restores an expired persistent
    // entry on *any* access -- including `get_ttl` itself -- resetting its
    // TTL to the network minimum. So "is it still readable" alone cannot tell
    // a live entry from a restored one. Instead:
    // - a restored entry reports exactly `restored_ttl()`;
    // - a remaining TTL exactly equal to the untouched decay proves the entry
    //   stayed live and was never expired-and-restored.
    // The contract instance is also auto-restored when a client call hits it
    // after its TTL; that is orthogonal to the per-schedule TTL measured here.

    /// The TTL the test host reports for an entry it just auto-restored.
    fn restored_ttl(env: &Env) -> u32 {
        env.ledger().get().min_persistent_entry_ttl - 1
    }

    /// Advance the ledger sequence by `ledgers`.
    fn advance(env: &Env, ledgers: u32) {
        let seq = env.ledger().sequence();
        env.ledger().set_sequence_number(seq + ledgers);
    }

    /// Field-by-field equality (`VestingSchedule` intentionally does not
    /// derive `PartialEq`; adding it would widen the public type's API).
    fn assert_same_schedule(a: &VestingSchedule, b: &VestingSchedule) {
        assert_eq!(a.beneficiary, b.beneficiary);
        assert_eq!(a.token, b.token);
        assert_eq!(a.total_amount, b.total_amount);
        assert_eq!(a.start, b.start);
        assert_eq!(a.cliff, b.cliff);
        assert_eq!(a.duration, b.duration);
        assert_eq!(a.claimed, b.claimed);
        assert_eq!(a.status, b.status);
    }

    #[test]
    fn create_writes_persistent_entry_with_full_ttl() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);

        env.as_contract(&client.address, || {
            assert!(env.storage().persistent().has(&DataKey::Schedule(id)));
            // Nothing schedule-shaped is left in instance storage; only the
            // id counter lives there.
            assert!(!env.storage().instance().has(&DataKey::Schedule(id)));
            assert_eq!(
                env.storage().instance().get::<_, u64>(&DataKey::Count),
                Some(id)
            );
        });
        assert_eq!(schedule_ttl(&env, &client, id), ttl::BUMP_AMOUNT);
    }

    #[test]
    fn schedule_survives_past_instance_ttl_boundary() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        let before = stored_schedule(&env, &client, id).unwrap();

        // Under the old layout the schedule lived inside the instance entry
        // and shared its TTL. Jump one ledger past the instance's expiry
        // (see `instance_entry_is_expired_at_that_boundary`).
        let instance_left = instance_ttl(&env, &client);
        assert!(instance_left < ttl::BUMP_AMOUNT);
        advance(&env, instance_left + 1);

        // The schedule entry has decayed by exactly the jump, so it stayed
        // live on its own TTL and was never restored.
        let remaining = ttl::BUMP_AMOUNT - (instance_left + 1);
        assert_ne!(remaining, restored_ttl(&env));
        assert_eq!(schedule_ttl(&env, &client, id), remaining);
        let after = stored_schedule(&env, &client, id).unwrap();
        assert_same_schedule(&before, &after);

        // End to end through the public interface.
        env.ledger().set_timestamp(START + DURATION);
        assert_eq!(client.claimable(&id), TOTAL);
        assert_eq!(client.claim(&id), TOTAL);
        assert_eq!(client.get_status(&id), VestingStatus::Completed);
    }

    /// Negative control for `schedule_survives_past_instance_ttl_boundary`:
    /// at the ledger it jumps to, the instance entry (where schedules used to
    /// live) really has expired -- the host had to restore it.
    #[test]
    fn instance_entry_is_expired_at_that_boundary() {
        let (env, client, accounts) = setup!();
        create(&client, &accounts);
        let instance_left = instance_ttl(&env, &client);
        advance(&env, instance_left + 1);
        // Advancing `instance_left + 1` ledgers leaves no live TTL, yet the
        // read reports a fresh minimum TTL: the entry was auto-restored.
        assert_eq!(instance_ttl(&env, &client), restored_ttl(&env));
    }

    /// Negative control for the TTL assertions: an untouched schedule entry
    /// really does expire at its own TTL (and is then auto-restored with the
    /// minimum TTL, not the decayed one).
    #[test]
    fn untouched_schedule_expires_at_its_own_ttl() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        advance(&env, ttl::BUMP_AMOUNT + 1);
        assert_eq!(schedule_ttl(&env, &client, id), restored_ttl(&env));
    }

    #[test]
    fn read_only_views_do_not_extend_ttl() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        advance(&env, 2 * ttl::DAY_IN_LEDGERS);
        let ttl_before = schedule_ttl(&env, &client, id);
        assert!(ttl_before < ttl::BUMP_THRESHOLD);

        client.claimable(&id);
        client.get_status(&id);

        assert_eq!(schedule_ttl(&env, &client, id), ttl_before);
    }

    #[test]
    fn claim_extends_ttl() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        advance(&env, 2 * ttl::DAY_IN_LEDGERS);
        assert!(schedule_ttl(&env, &client, id) < ttl::BUMP_THRESHOLD);

        env.ledger().set_timestamp(START + DURATION);
        assert_eq!(client.claim(&id), TOTAL);

        assert_eq!(schedule_ttl(&env, &client, id), ttl::BUMP_AMOUNT);
    }

    #[test]
    fn touch_ttl_extends_without_changing_state() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);

        // Mid-vesting with a partial claim, so `claimed` and `status` are
        // non-default and any accidental rewrite would show.
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 4);
        client.claim(&id);
        env.ledger()
            .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);

        advance(&env, 2 * ttl::DAY_IN_LEDGERS);
        let ttl_before = schedule_ttl(&env, &client, id);
        assert!(ttl_before < ttl::BUMP_THRESHOLD);

        let schedule_before = stored_schedule(&env, &client, id).unwrap();
        let claimable_before = client.claimable(&id);
        let status_before = client.get_status(&id);
        assert!(claimable_before > 0);

        client.touch_ttl(&id);
        // Permissionless: no authorization was demanded.
        assert!(env.auths().is_empty());

        assert_eq!(schedule_ttl(&env, &client, id), ttl::BUMP_AMOUNT);
        let schedule_after = stored_schedule(&env, &client, id).unwrap();
        assert_same_schedule(&schedule_before, &schedule_after);
        assert_eq!(client.claimable(&id), claimable_before);
        assert_eq!(client.get_status(&id), status_before);
    }

    #[test]
    fn touch_ttl_keeps_schedule_live_past_original_expiry() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        let original_ttl = schedule_ttl(&env, &client, id);

        // Touch once inside the bump window, then jump one ledger past the
        // entry's original expiry.
        let touched_at = 2 * ttl::DAY_IN_LEDGERS;
        advance(&env, touched_at);
        client.touch_ttl(&id);
        advance(&env, original_ttl + 1 - touched_at);

        // Exactly the TTL the touch granted, minus the elapsed ledgers:
        // never expired, never restored.
        let remaining = ttl::BUMP_AMOUNT - (original_ttl + 1 - touched_at);
        assert_ne!(remaining, restored_ttl(&env));
        assert_eq!(schedule_ttl(&env, &client, id), remaining);
        assert_eq!(client.get_status(&id), VestingStatus::Locked);
    }

    #[test]
    fn touch_ttl_when_fresh_is_a_no_op() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        advance(&env, 10);
        let ttl_before = schedule_ttl(&env, &client, id);

        client.touch_ttl(&id);

        // Outside the bump window the threshold check skips the extension.
        assert_eq!(schedule_ttl(&env, &client, id), ttl_before);
    }

    #[test]
    fn missing_schedule_is_not_found_and_creates_no_storage() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        let missing = id + 998;

        for err in [
            client.try_touch_ttl(&missing).unwrap_err().unwrap(),
            client.try_claim(&missing).unwrap_err().unwrap(),
            client.try_claimable(&missing).unwrap_err().unwrap(),
            client.try_get_status(&missing).unwrap_err().unwrap(),
        ] {
            assert_eq!(err, ForgeError::NotFound);
        }

        env.as_contract(&client.address, || {
            assert!(!env.storage().persistent().has(&DataKey::Schedule(missing)));
            assert!(!env.storage().instance().has(&DataKey::Schedule(missing)));
            assert!(!env.storage().temporary().has(&DataKey::Schedule(missing)));
            // The id counter was not advanced by the failed lookups.
            assert_eq!(
                env.storage().instance().get::<_, u64>(&DataKey::Count),
                Some(id)
            );
        });
    }
}
