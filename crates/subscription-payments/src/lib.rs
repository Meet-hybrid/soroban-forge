#![no_std]

//! # Soroban Forge — Subscription Payments contract
//!
//! Recurring, on-chain subscription billing: a subscriber authorises a
//! provider to pull a fixed `amount` per `period` (seconds) from a token
//! balance. The contract tracks subscription state and billing cadence; the
//! provider pulls each period by calling [`SorobanForgeSubscriptionPayments::charge`].
//!
//! Lifecycle:
//!
//! ```text
//! subscribe -> Active --(period elapses)--> charge bills amount, advances
//!            -> cancel -> Cancelled (no further charges)
//! ```
//!
//! Authorization model:
//! - `subscribe` requires the subscriber (who authorises the agreement).
//! - `charge` requires the provider (who pulls payment) and only bills when a
//!   full period has elapsed since the last charge.
//! - `cancel` requires the subscriber.
//! - `get_subscription` is a read-only view.
//!
//! The `PastDue` status is reserved for a failed-payment retry model that
//! lands in a follow-up; it is not reachable through the current public
//! interface. Token settlement (SAC transfers) is intentionally out of scope
//! for this iteration: the contract tracks state and authorisation, not
//! balances.

#[cfg(test)]
extern crate std;

use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env};

/// Public interface for the Soroban Forge subscription payments contract.
#[contractclient(name = "SorobanForgeSubscriptionPaymentsClient")]
pub trait SorobanForgeSubscriptionPayments {
    /// Subscribe `subscriber` to `provider`'s service at `amount` per
    /// `period`. Returns the stable subscription id.
    fn subscribe(
        env: Env,
        subscriber: Address,
        provider: Address,
        token: Address,
        amount: i128,
        period: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Charge the next due payment for `subscription_id`.
    ///
    /// Returns the billed amount, or `0` when no full period has elapsed since
    /// the last charge.
    fn charge(
        env: Env,
        subscription_id: u64,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Cancel `subscription_id`, preventing further charges.
    fn cancel(env: Env, subscription_id: u64)
        -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Permissionless TTL keeper: extend a stored subscription entry without
    /// mutating its state.
    fn touch_ttl(
        env: Env,
        subscription_id: u64,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Read a stored subscription by id (read-only view).
    fn get_subscription(
        env: Env,
        subscription_id: u64,
    ) -> Result<Subscription, soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a subscription.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    /// Active and chargeable.
    Active,
    /// Cancelled; no further charges.
    Cancelled,
    /// Payment failed and the subscription is in arrears.
    PastDue,
}

/// A recurring payment agreement.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscription {
    /// Stable identifier assigned at creation.
    pub subscription_id: u64,
    /// Account being charged.
    pub subscriber: Address,
    /// Account receiving payments.
    pub provider: Address,
    /// Token contract used for settlement.
    pub token: Address,
    /// Amount charged per period.
    pub amount: i128,
    /// Length of one billing period, in seconds.
    pub period: u64,
    /// Ledger timestamp of the last successful charge.
    pub last_charged: u64,
    /// Current state.
    pub status: SubscriptionStatus,
}

/// Ledger-time constants for TTL bumps. One ledger closes roughly every 5
/// seconds, so 17,280 ledgers ~= 1 day. `BUMP_AMOUNT` is the lifetime written
/// on every touch; `BUMP_THRESHOLD` is how close to expiry an entry must be
/// before we extend it. This keeps long-lived subscriptions alive without
/// requiring a custom storage migration before every billing cycle.
mod ttl {
    pub const DAY_IN_LEDGERS: u32 = 17_280;
    pub const BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
    pub const BUMP_THRESHOLD: u32 = BUMP_AMOUNT - DAY_IN_LEDGERS;
}

/// Instance-storage keys.
#[contracttype]
enum DataKey {
    /// The subscription record for `u64` id.
    Subscription(u64),
    /// Monotonic subscription id counter.
    Count,
}

/// The deployable subscription payments contract.
#[contract]
pub struct SubscriptionPayments;

#[contractimpl]
impl SubscriptionPayments {
    /// Create a new subscription and return its stable id.
    ///
    /// Requires `amount > 0` and `period > 0`. The subscriber is authorized at
    /// creation time; billing starts from the moment of subscription.
    pub fn subscribe(
        env: Env,
        subscriber: Address,
        provider: Address,
        token: Address,
        amount: i128,
        period: u64,
    ) -> Result<u64, ForgeError> {
        if amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        if period == 0 {
            return Err(ForgeError::InvalidInput);
        }
        subscriber.require_auth();

        let subscription_id = Self::next_id(&env)?;
        let subscription = Subscription {
            subscription_id,
            subscriber,
            provider,
            token,
            amount,
            period,
            last_charged: env.ledger().timestamp(),
            status: SubscriptionStatus::Active,
        };
        let key = DataKey::Subscription(subscription_id);
        env.storage().persistent().set(&key, &subscription);
        bump_entry(&env, &key);
        Ok(subscription_id)
    }

    /// Bill one due period.
    ///
    /// Requires the provider. If a full period has not elapsed since the last
    /// charge, returns `0` and leaves the subscription untouched. Otherwise
    /// advances the billing point by one period and returns the billed amount.
    /// Repeating the call catches up at most one period at a time.
    pub fn charge(env: Env, subscription_id: u64) -> Result<i128, ForgeError> {
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status != SubscriptionStatus::Active {
            return Err(ForgeError::InvalidInput);
        }
        subscription.provider.require_auth();

        let next_due = subscription
            .last_charged
            .checked_add(subscription.period)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        if env.ledger().timestamp() < next_due {
            return Ok(0);
        }

        subscription.last_charged = next_due;
        let key = DataKey::Subscription(subscription_id);
        env.storage().persistent().set(&key, &subscription);
        bump_entry(&env, &key);
        Ok(subscription.amount)
    }

    /// Cancel a subscription, preventing further charges.
    ///
    /// Requires the subscriber. Cancelling an already-cancelled subscription is
    /// rejected.
    pub fn cancel(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status != SubscriptionStatus::Active {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();

        subscription.status = SubscriptionStatus::Cancelled;
        let key = DataKey::Subscription(subscription_id);
        env.storage().persistent().set(&key, &subscription);
        bump_entry(&env, &key);
        Ok(())
    }

    /// Permissionless keeper: extend a stored subscription entry without
    /// mutating its state.
    pub fn touch_ttl(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let key = DataKey::Subscription(subscription_id);
        if !env.storage().persistent().has(&key) {
            return Err(ForgeError::NotFound);
        }
        bump_entry(&env, &key);
        Ok(())
    }

    /// Read a stored subscription by id (read-only view).
    pub fn get_subscription(env: Env, subscription_id: u64) -> Result<Subscription, ForgeError> {
        Self::get_subscription_impl(&env, subscription_id)
    }

    /// Allocate the next monotonic subscription id.
    fn next_id(env: &Env) -> Result<u64, ForgeError> {
        let count: u64 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let id = count.checked_add(1).ok_or(ForgeError::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::Count, &id);
        Ok(id)
    }

    fn get_subscription_impl(env: &Env, subscription_id: u64) -> Result<Subscription, ForgeError> {
        env.storage()
            .persistent()
            .get(&DataKey::Subscription(subscription_id))
            .ok_or(ForgeError::NotFound)
    }
}

/// Bump a persistent entry's TTL to the [`ttl::BUMP_AMOUNT`] horizon when it
/// falls inside [`ttl::BUMP_THRESHOLD`]. This mirrors the escrow and multi-sig
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
    const PERIOD: u64 = 1_000;
    const AMOUNT: i128 = 250;

    /// Build a fresh env with mocked auths, a registered contract, a
    /// subscription, and named accounts.
    macro_rules! setup {
        () => {{
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(START);
            let contract_id = env.register(SubscriptionPayments, ());
            let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);
            let subscription_id = client.subscribe(
                &accounts.user1,
                &accounts.validator,
                &accounts.deployer,
                &AMOUNT,
                &PERIOD,
            );
            (env, client, accounts, subscription_id)
        }};
    }

    #[test]
    fn subscribe_succeeds_and_is_active() {
        let (_env, client, accounts, subscription_id) = setup!();
        let subscription = client.get_subscription(&subscription_id);
        assert_eq!(subscription.subscriber, accounts.user1);
        assert_eq!(subscription.provider, accounts.validator);
        assert_eq!(subscription.amount, AMOUNT);
        assert_eq!(subscription.status, SubscriptionStatus::Active);
        assert_eq!(subscription.last_charged, START);
    }

    #[test]
    fn subscribe_assigns_distinct_ids() {
        let (_env, client, accounts, subscription_id) = setup!();
        let id2 = client.subscribe(
            &accounts.user2,
            &accounts.validator,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        assert_ne!(subscription_id, id2);
    }

    #[test]
    fn subscribe_rejects_zero_amount() {
        let (_env, client, accounts, _id) = setup!();
        let err = client
            .try_subscribe(
                &accounts.user1,
                &accounts.validator,
                &accounts.deployer,
                &0_i128,
                &PERIOD,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn subscribe_rejects_zero_period() {
        let (_env, client, accounts, _id) = setup!();
        let err = client
            .try_subscribe(
                &accounts.user1,
                &accounts.validator,
                &accounts.deployer,
                &AMOUNT,
                &0_u64,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn charge_before_period_returns_zero() {
        let (env, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD - 1);
        assert_eq!(client.charge(&subscription_id), 0);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START
        );
    }

    #[test]
    fn charge_at_period_bills_full_amount() {
        let (env, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD);
        assert_eq!(client.charge(&subscription_id), AMOUNT);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START + PERIOD
        );
    }

    #[test]
    fn charge_catches_up_one_period_per_call() {
        let (env, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD * 3);
        // Each call bills a single period and advances the billing point.
        assert_eq!(client.charge(&subscription_id), AMOUNT);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START + PERIOD
        );
    }

    #[test]
    fn charge_missing_subscription_is_not_found() {
        let (_env, client, _accounts, _id) = setup!();
        let err = client.try_charge(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn subscription_record_is_persistent() {
        let (env, client, _accounts, subscription_id) = setup!();
        let record: Subscription = env
            .storage()
            .persistent()
            .get(&DataKey::Subscription(subscription_id))
            .unwrap();
        assert_eq!(record.subscription_id, subscription_id);
        assert_eq!(client.get_subscription(&subscription_id).status, SubscriptionStatus::Active);
    }

    #[test]
    fn touch_ttl_extends_and_keeps_state_intact() {
        let (env, client, _accounts, subscription_id) = setup!();
        client.touch_ttl(&subscription_id);
        assert_eq!(client.get_subscription(&subscription_id).status, SubscriptionStatus::Active);
        assert!(env.storage().persistent().has(&DataKey::Subscription(subscription_id)));
    }

    #[test]
    fn touch_ttl_unknown_subscription_is_not_found() {
        let (_env, client, _accounts, _id) = setup!();
        let err = client.try_touch_ttl(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn cancel_prevents_further_charges() {
        let (env, client, _accounts, subscription_id) = setup!();
        client.cancel(&subscription_id);
        assert_eq!(
            client.get_subscription(&subscription_id).status,
            SubscriptionStatus::Cancelled
        );
        env.ledger().set_timestamp(START + PERIOD);
        let err = client.try_charge(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_twice_is_invalid() {
        let (_env, client, _accounts, subscription_id) = setup!();
        client.cancel(&subscription_id);
        let err = client.try_cancel(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_missing_subscription_is_not_found() {
        let (_env, client, _accounts, _id) = setup!();
        let err = client.try_cancel(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_subscription_missing_is_not_found() {
        let (_env, client, _accounts, _id) = setup!();
        let err = client.try_get_subscription(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }
}
