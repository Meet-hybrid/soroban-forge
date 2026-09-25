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
use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env, Vec};

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

    /// Read a stored subscription by id (read-only view).
    fn get_subscription(
        env: Env,
        subscription_id: u64,
    ) -> Result<Subscription, soroban_forge_shared_utils::ForgeError>;

    /// Total number of subscriptions created so far (read-only view).
    fn get_subscription_count(env: Env) -> u64;

    /// List `subscriber`'s subscriptions in creation order, one page at a
    /// time (read-only view).
    ///
    /// `offset` skips the first `offset` subscriptions and `limit` caps the
    /// page size. Returns a `Result` so a `limit` of `0` fails with
    /// [`ForgeError::InvalidInput`]; an empty index or an out-of-bounds
    /// offset yields an empty `Vec`, not an error.
    fn subscriptions_for_subscriber(
        env: Env,
        subscriber: Address,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Subscription>, soroban_forge_shared_utils::ForgeError>;

    /// List `provider`'s subscriptions in creation order, one page at a time
    /// (read-only view).
    ///
    /// `offset` skips the first `offset` subscriptions and `limit` caps the
    /// page size. Returns a `Result` so a `limit` of `0` fails with
    /// [`ForgeError::InvalidInput`]; an empty index or an out-of-bounds
    /// offset yields an empty `Vec`, not an error.
    fn subscriptions_for_provider(
        env: Env,
        provider: Address,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Subscription>, soroban_forge_shared_utils::ForgeError>;
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

/// Instance-storage keys.
#[contracttype]
enum DataKey {
    /// The subscription record for `u64` id.
    Subscription(u64),
    /// Monotonic subscription id counter.
    Count,
    /// Creation-order subscription ids for which the `Address` is the
    /// subscriber, in id order. Written once per `subscribe`.
    SubscriberSubscriptions(Address),
    /// Creation-order subscription ids for which the `Address` is the
    /// provider, in id order. Written once per `subscribe`.
    ProviderSubscriptions(Address),
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
            subscriber: subscriber.clone(),
            provider: provider.clone(),
            token,
            amount,
            period,
            last_charged: env.ledger().timestamp(),
            status: SubscriptionStatus::Active,
        };
        env.storage()
            .instance()
            .set(&DataKey::Subscription(subscription_id), &subscription);
        // Index writes join the success path after every fallible step
        // (validation, `require_auth`, id allocation), so they cannot
        // observe or create partial state.
        Self::append_index(
            &env,
            &DataKey::SubscriberSubscriptions(subscriber),
            subscription_id,
        );
        Self::append_index(
            &env,
            &DataKey::ProviderSubscriptions(provider),
            subscription_id,
        );
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
        env.storage()
            .instance()
            .set(&DataKey::Subscription(subscription_id), &subscription);
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
        env.storage()
            .instance()
            .set(&DataKey::Subscription(subscription_id), &subscription);
        Ok(())
    }

    /// Read a stored subscription by id (read-only view).
    pub fn get_subscription(env: Env, subscription_id: u64) -> Result<Subscription, ForgeError> {
        Self::get_subscription_impl(&env, subscription_id)
    }

    /// Total number of subscriptions created so far (read-only view).
    ///
    /// This is the monotonic id counter, which only `subscribe` advances, so
    /// it never decreases and equals the number of live ids returned by the
    /// enumeration views.
    pub fn get_subscription_count(env: Env) -> u64 {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    }

    /// List `subscriber`'s subscriptions in creation order, one page at a
    /// time (read-only view).
    ///
    /// Requires no authorization and never mutates storage. An address with
    /// no subscriptions — or an offset at or past the end of its list —
    /// returns an empty `Vec`, not an error, so clients can back "my
    /// subscriptions" views without an off-chain indexer.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::InvalidInput`] — `limit` is zero.
    pub fn subscriptions_for_subscriber(
        env: Env,
        subscriber: Address,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Subscription>, ForgeError> {
        if limit == 0 {
            return Err(ForgeError::InvalidInput);
        }
        let ids = Self::index_ids(&env, &DataKey::SubscriberSubscriptions(subscriber));
        Self::resolve_page(&env, &ids, offset, limit)
    }

    /// List `provider`'s subscriptions in creation order, one page at a time
    /// (read-only view).
    ///
    /// Requires no authorization and never mutates storage. An address with
    /// no subscriptions — or an offset at or past the end of its list —
    /// returns an empty `Vec`, not an error, so clients can back "my
    /// subscribers" views without an off-chain indexer.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::InvalidInput`] — `limit` is zero.
    pub fn subscriptions_for_provider(
        env: Env,
        provider: Address,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Subscription>, ForgeError> {
        if limit == 0 {
            return Err(ForgeError::InvalidInput);
        }
        let ids = Self::index_ids(&env, &DataKey::ProviderSubscriptions(provider));
        Self::resolve_page(&env, &ids, offset, limit)
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
            .instance()
            .get(&DataKey::Subscription(subscription_id))
            .ok_or(ForgeError::NotFound)
    }

    /// Append `subscription_id` to an index, carrying the address the index
    /// belongs to in the key.
    fn append_index(env: &Env, key: &DataKey, subscription_id: u64) {
        let mut ids: Vec<u64> = env
            .storage()
            .instance()
            .get(key)
            .unwrap_or_else(|| Vec::new(env));
        ids.push_back(subscription_id);
        env.storage().instance().set(key, &ids);
    }

    /// Read an index's id list, defaulting to empty when the address has no
    /// entries.
    fn index_ids(env: &Env, key: &DataKey) -> Vec<u64> {
        env.storage()
            .instance()
            .get(key)
            .unwrap_or_else(|| Vec::new(env))
    }

    /// Resolve a contiguous slice of `ids` into `Subscription` records.
    ///
    /// `offset` is clamped to the list length and `end` saturates, so an
    /// out-of-bounds offset yields an empty `Vec` rather than a bounds panic.
    fn resolve_page(
        env: &Env,
        ids: &Vec<u64>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Subscription>, ForgeError> {
        let total = ids.len();
        let (mut at, end) = (offset.min(total), offset.saturating_add(limit).min(total));
        let mut subscriptions = Vec::new(env);
        while at < end {
            subscriptions.push_back(Self::get_subscription_impl(env, ids.get_unchecked(at))?);
            at += 1;
        }
        Ok(subscriptions)
    }
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

    #[test]
    fn get_subscription_count_tracks_creations() {
        let (_env, client, accounts, _id) = setup!();
        assert_eq!(client.get_subscription_count(), 1);
        let id2 = client.subscribe(
            &accounts.user2,
            &accounts.validator,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        assert_eq!(client.get_subscription_count(), 2);
        // Cancelling does not lower the creation count.
        client.cancel(&id2);
        assert_eq!(client.get_subscription_count(), 2);
    }

    #[test]
    fn subscriber_index_tracks_multiple_providers() {
        let (_env, client, accounts, _id) = setup!();
        let id2 = client.subscribe(
            &accounts.user1,
            &accounts.arbiter,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        let page = client.subscriptions_for_subscriber(&accounts.user1, &0, &10);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get_unchecked(0).subscription_id, 1);
        assert_eq!(page.get_unchecked(0).provider, accounts.validator);
        assert_eq!(page.get_unchecked(1).subscription_id, id2);
        assert_eq!(page.get_unchecked(1).provider, accounts.arbiter);
    }

    #[test]
    fn provider_index_tracks_multiple_subscribers() {
        let (_env, client, accounts, _id) = setup!();
        let id2 = client.subscribe(
            &accounts.user2,
            &accounts.validator,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        let page = client.subscriptions_for_provider(&accounts.validator, &0, &10);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get_unchecked(0).subscriber, accounts.user1);
        assert_eq!(page.get_unchecked(0).subscription_id, 1);
        assert_eq!(page.get_unchecked(1).subscriber, accounts.user2);
        assert_eq!(page.get_unchecked(1).subscription_id, id2);
    }

    #[test]
    fn pagination_slices_across_pages() {
        let (_env, client, accounts, _id) = setup!();
        // user1 adds four more subscriptions, for ids 2..=5.
        client.subscribe(
            &accounts.user1,
            &accounts.arbiter,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        client.subscribe(
            &accounts.user1,
            &accounts.user3,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        client.subscribe(
            &accounts.user1,
            &accounts.user2,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        client.subscribe(
            &accounts.user1,
            &accounts.validator,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );

        let first = client.subscriptions_for_subscriber(&accounts.user1, &0, &2);
        assert_eq!(first.len(), 2);
        assert_eq!(first.get_unchecked(0).subscription_id, 1);
        assert_eq!(first.get_unchecked(1).subscription_id, 2);

        let mid = client.subscriptions_for_subscriber(&accounts.user1, &2, &2);
        assert_eq!(mid.len(), 2);
        assert_eq!(mid.get_unchecked(0).subscription_id, 3);
        assert_eq!(mid.get_unchecked(1).subscription_id, 4);

        let last = client.subscriptions_for_subscriber(&accounts.user1, &4, &10);
        assert_eq!(last.len(), 1);
        assert_eq!(last.get_unchecked(0).subscription_id, 5);
    }

    #[test]
    fn subscriptions_for_unknown_address_are_empty() {
        let (_env, client, accounts, _id) = setup!();
        assert_eq!(
            client
                .subscriptions_for_subscriber(&accounts.arbiter, &0, &10)
                .len(),
            0
        );
        assert_eq!(
            client
                .subscriptions_for_provider(&accounts.arbiter, &0, &10)
                .len(),
            0
        );
    }

    #[test]
    fn out_of_bounds_offset_returns_empty() {
        let (_env, client, accounts, _id) = setup!();
        assert_eq!(
            client
                .subscriptions_for_subscriber(&accounts.user1, &1, &10)
                .len(),
            0
        );
        assert_eq!(
            client
                .subscriptions_for_provider(&accounts.validator, &2, &10)
                .len(),
            0
        );
    }

    #[test]
    fn zero_limit_is_invalid_input() {
        let (_env, client, accounts, _id) = setup!();
        let err = client
            .try_subscriptions_for_subscriber(&accounts.user1, &0, &0)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
        let err = client
            .try_subscriptions_for_provider(&accounts.validator, &0, &0)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }
}
