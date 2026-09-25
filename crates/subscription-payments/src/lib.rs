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
//! subscribe -> Active <-> Paused
//!                 \         /
//!                  v       v
//!                   cancel -> Cancelled (no further charges)
//! ```
//!
//! Authorization model:
//! - `subscribe` requires the subscriber (who authorises the agreement).
//! - `charge` requires the provider (who pulls payment) and only bills when a
//!   full period has elapsed since the last charge.
//! - `pause` requires the subscriber.
//! - `resume` requires the subscriber.
//! - `cancel` requires the subscriber.
//! - `get_subscription` is a read-only view.
//!
//! ## Resume Semantics
//!
//! When a subscription is resumed from `Paused`, `last_charged` is advanced by
//! the exact elapsed paused duration (`ledger_timestamp - paused_at`). This
//! pushes the next due date (`last_charged + period`) forward by the paused
//! duration so that the paused interval is never billed, and any remaining
//! time in the unbilled cycle is preserved deterministically.
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

    /// Pause an active subscription, preventing further charges while paused.
    ///
    /// Requires the subscriber. Only valid when `Active`.
    fn pause(env: Env, subscription_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Resume a paused subscription, advancing the next due date by the
    /// elapsed paused duration.
    ///
    /// Requires the subscriber. Only valid when `Paused`.
    fn resume(env: Env, subscription_id: u64)
        -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Cancel `subscription_id`, preventing further charges.
    ///
    /// Requires the subscriber. Valid when `Active` or `Paused`.
    fn cancel(env: Env, subscription_id: u64)
        -> Result<(), soroban_forge_shared_utils::ForgeError>;

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
    /// Temporarily paused; no charges can be made until resumed.
    Paused,
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
    /// Ledger timestamp when paused, if currently paused.
    pub paused_at: Option<u64>,
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
            paused_at: None,
        };
        env.storage()
            .instance()
            .set(&DataKey::Subscription(subscription_id), &subscription);
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

    /// Pause an active subscription, preventing charges while paused.
    ///
    /// Requires the subscriber. Only valid when `Active`.
    pub fn pause(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status != SubscriptionStatus::Active {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();

        subscription.status = SubscriptionStatus::Paused;
        subscription.paused_at = Some(env.ledger().timestamp());
        env.storage()
            .instance()
            .set(&DataKey::Subscription(subscription_id), &subscription);
        Ok(())
    }

    /// Resume a paused subscription, advancing the next due date by the
    /// elapsed paused duration so that paused periods are not billed.
    ///
    /// Requires the subscriber. Only valid when `Paused`.
    pub fn resume(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status != SubscriptionStatus::Paused {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();

        let now = env.ledger().timestamp();
        let paused_at = subscription.paused_at.unwrap_or(now);
        let elapsed = now
            .checked_sub(paused_at)
            .ok_or(ForgeError::ArithmeticOverflow)?;

        subscription.last_charged = subscription
            .last_charged
            .checked_add(elapsed)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        subscription.status = SubscriptionStatus::Active;
        subscription.paused_at = None;
        env.storage()
            .instance()
            .set(&DataKey::Subscription(subscription_id), &subscription);
        Ok(())
    }

    /// Cancel a subscription, preventing further charges.
    ///
    /// Requires the subscriber. Valid when `Active` or `Paused`. Cancelling
    /// an already-cancelled subscription is rejected.
    pub fn cancel(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status != SubscriptionStatus::Active
            && subscription.status != SubscriptionStatus::Paused
        {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();

        subscription.status = SubscriptionStatus::Cancelled;
        subscription.paused_at = None;
        env.storage()
            .instance()
            .set(&DataKey::Subscription(subscription_id), &subscription);
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
            .instance()
            .get(&DataKey::Subscription(subscription_id))
            .ok_or(ForgeError::NotFound)
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
        assert_eq!(subscription.paused_at, None);
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
    fn pause_active_subscription_succeeds() {
        let (env, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + 250);
        client.pause(&subscription_id);

        let sub = client.get_subscription(&subscription_id);
        assert_eq!(sub.status, SubscriptionStatus::Paused);
        assert_eq!(sub.paused_at, Some(START + 250));
    }

    #[test]
    fn pause_already_paused_is_invalid() {
        let (_env, client, _accounts, subscription_id) = setup!();
        client.pause(&subscription_id);
        let err = client.try_pause(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn pause_cancelled_is_invalid() {
        let (_env, client, _accounts, subscription_id) = setup!();
        client.cancel(&subscription_id);
        let err = client.try_pause(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn pause_missing_is_not_found() {
        let (_env, client, _accounts, _id) = setup!();
        let err = client.try_pause(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn charge_paused_subscription_fails_and_does_not_advance() {
        let (env, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + 100);
        client.pause(&subscription_id);

        env.ledger().set_timestamp(START + PERIOD * 2);
        let err = client.try_charge(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);

        let sub = client.get_subscription(&subscription_id);
        assert_eq!(sub.last_charged, START);
        assert_eq!(sub.status, SubscriptionStatus::Paused);
    }

    #[test]
    fn resume_paused_subscription_advances_due_date_by_elapsed() {
        let (env, client, _accounts, subscription_id) = setup!();
        // Pause at START + 300 (300s into 1000s period)
        env.ledger().set_timestamp(START + 300);
        client.pause(&subscription_id);

        // Resume at START + 800 (elapsed pause = 500s)
        env.ledger().set_timestamp(START + 800);
        client.resume(&subscription_id);

        let sub = client.get_subscription(&subscription_id);
        assert_eq!(sub.status, SubscriptionStatus::Active);
        assert_eq!(sub.paused_at, None);
        // last_charged should be shifted by 500s: START + 500
        assert_eq!(sub.last_charged, START + 500);

        // At original due date (START + 1000), only 500s of active cycle elapsed: charge -> 0
        env.ledger().set_timestamp(START + 1000);
        assert_eq!(client.charge(&subscription_id), 0);

        // At new due date (START + 500 + 1000 = START + 1500), charge succeeds
        env.ledger().set_timestamp(START + 1500);
        assert_eq!(client.charge(&subscription_id), AMOUNT);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START + 1500
        );
    }

    #[test]
    fn pause_resume_zero_length_no_drift() {
        let (env, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + 300);
        client.pause(&subscription_id);
        client.resume(&subscription_id);

        let sub = client.get_subscription(&subscription_id);
        assert_eq!(sub.status, SubscriptionStatus::Active);
        assert_eq!(sub.last_charged, START);
        assert_eq!(sub.paused_at, None);

        // Charge at exactly START + PERIOD succeeds as normal
        env.ledger().set_timestamp(START + PERIOD);
        assert_eq!(client.charge(&subscription_id), AMOUNT);
    }

    #[test]
    fn pause_across_multiple_periods_preserves_remaining_cycle() {
        let (env, client, _accounts, subscription_id) = setup!();
        // 100s into cycle (900s remaining)
        env.ledger().set_timestamp(START + 100);
        client.pause(&subscription_id);

        // Stay paused across multiple periods: 5 periods (5000s) elapsed while paused
        env.ledger().set_timestamp(START + 5100);
        client.resume(&subscription_id);

        let sub = client.get_subscription(&subscription_id);
        // last_charged shifted by 5000: START + 5000
        assert_eq!(sub.last_charged, START + 5000);
        // next due is START + 6000 (which is resume_time + 900s remaining)

        env.ledger().set_timestamp(START + 5999);
        assert_eq!(client.charge(&subscription_id), 0);

        env.ledger().set_timestamp(START + 6000);
        assert_eq!(client.charge(&subscription_id), AMOUNT);
    }

    #[test]
    fn resume_active_subscription_is_invalid() {
        let (_env, client, _accounts, subscription_id) = setup!();
        let err = client.try_resume(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn resume_cancelled_subscription_is_invalid() {
        let (_env, client, _accounts, subscription_id) = setup!();
        client.cancel(&subscription_id);
        let err = client.try_resume(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn resume_missing_is_not_found() {
        let (_env, client, _accounts, _id) = setup!();
        let err = client.try_resume(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn cancel_paused_subscription_succeeds_and_cannot_resume() {
        let (env, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + 200);
        client.pause(&subscription_id);

        // Cancel while paused
        client.cancel(&subscription_id);
        let sub = client.get_subscription(&subscription_id);
        assert_eq!(sub.status, SubscriptionStatus::Cancelled);
        assert_eq!(sub.paused_at, None);

        // Cannot resume cancelled subscription
        let err = client.try_resume(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);

        // Cannot charge cancelled subscription
        env.ledger().set_timestamp(START + PERIOD * 2);
        let err = client.try_charge(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);

        // Cannot pause cancelled subscription
        let err = client.try_pause(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);

        // Cannot cancel already cancelled subscription
        let err = client.try_cancel(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }
}
