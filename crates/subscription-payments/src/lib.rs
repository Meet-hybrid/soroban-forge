#![no_std]

//! # Soroban Forge — Subscription Payments contract
//!
//! Recurring, on-chain subscription billing: a subscriber authorises a
//! provider to pull a fixed `amount` per `period` (seconds) from a token
//! balance using SEP-41 tokens. The contract tracks subscription state,
//! billing cadence, token settlement, pause/resume, arrears retry handling,
//! and metered usage (per-metric quotas with exact overage pricing).
//!
//! Lifecycle:
//!
//! ```text
//! subscribe -> Active <-> Paused
//!                 \         /
//!                  v       v
//!                   cancel -> Cancelled (no further charges)
//! subscribe -> Active --(period elapses, charge succeeds)--> bills amount, advances
//!            -> Active --(period elapses, charge fails)---> PastDue (arrears retry)
//!            -> PastDue --(retry exceeds max retries)-----> Cancelled
//!            -> cancel -> Cancelled (no further charges)
//! ```
//!
//! Authorization model:
//! - `subscribe` requires the subscriber (who authorises the agreement).
//! - `subscribe_with_trial` requires the subscriber and defers the first due
//!   charge until `now + trial_seconds`.
//! - `authorize_provider` requires the subscriber (who consents to the
//!   provider-initiated relationship).
//! - `revoke_provider` requires the subscriber (who withdraws that consent).
//! - `subscribe_on_behalf_of` requires the provider **and** an explicit
//!   opt-in from the subscriber for that provider.
//! - `charge` requires the provider (who pulls payment) and bills when a
//!   full period has elapsed since the last charge, executing a SEP-41 token
//!   transfer from subscriber to provider in pull mode, or from prepaid
//!   contract custody to provider after a subscriber opts in with `deposit`.
//! - `charge_catchup` requires the provider and atomically bills multiple
//!   elapsed periods, bounded by [`MAX_CATCHUP_PERIODS`]. It refuses
//!   `PastDue` subscriptions so arrears retry semantics remain owned by
//!   `charge`.
//! - `set_quotas` requires the subscriber: quotas price the overage the
//!   subscriber is billed, so the account being charged is the one that
//!   consents to the terms.
//! - `record_usage` requires the provider (the party that served the usage
//!   and meters it, mirroring who is authorized to `charge`).
//! - `pause` requires the subscriber.
//! - `resume` requires the subscriber.
//! - `cancel` requires the subscriber (works from `Active`, `Paused`, or `PastDue`).
//! - read-only views (`get_subscription`, `get_subscription_count`,
//!   `subscriptions_for_subscriber`, `subscriptions_for_provider`,
//!   `is_provider_authorized`, `get_trial_end`) require no authorization.
//!
//! | Entrypoint                 | Required authorization             |
//! | -------------------------- | ---------------------------------- |
//! | `subscribe`                | subscriber                         |
//! | `subscribe_with_trial`     | subscriber                         |
//! | `authorize_provider`       | subscriber                         |
//! | `revoke_provider`          | subscriber                         |
//! | `subscribe_on_behalf_of`   | provider + valid subscriber opt-in |
//! | `charge`                   | provider (existing authorization)  |
//! | `deposit`                  | subscriber                         |
//! | `withdraw_balance`         | subscriber                         |
//! | `configure_auto_topup`     | subscriber                         |
//! | `disable_auto_topup`       | subscriber                         |
//! | `set_quotas`               | subscriber (prices the overage)    |
//! | `record_usage`             | provider (meters the service)      |
//! | `pause`                    | subscriber                         |
//! | `resume`                   | subscriber                         |
//! | `cancel`                   | subscriber                         |
//! | read methods               | none                               |
//!
//! ## Prepaid auto-topup
//!
//! A prepaid subscription lapses to `PastDue` when its balance cannot cover
//! the period. `configure_auto_topup(subscription_id, trigger_threshold,
//! topup_amount)` lets the subscriber authorize the contract to pull
//! `topup_amount` from their account into custody during billing instead:
//!
//! - **Trigger.** On a due `charge` in prepaid mode, before the debit, the
//!   contract tops up once when `prepaid_balance < period_amount` (the
//!   charge would otherwise fail) or `prepaid_balance < trigger_threshold`
//!   (the subscriber's safety floor). At most one top-up runs per charge;
//!   if the topped-up balance still cannot cover the period, the charge
//!   follows the usual arrears path and the topped-up funds stay credited
//!   to the prepaid balance.
//! - **Transfer before state.** The top-up transfer (subscriber → custody)
//!   runs before the debit transfer and before the subscription is written,
//!   and emits `AutoTopupTriggered` with the amount and resulting balance.
//! - **Safe fallback.** A top-up the token rejects (insufficient subscriber
//!   funds, a deauthorized account) moves nothing and writes nothing; the
//!   charge then proceeds on the unchanged balance — a debit when it still
//!   covers the period, otherwise `PastDue` exactly as without auto-topup.
//!   A top-up that would overflow the balance is skipped the same way.
//! - **Validation.** `topup_amount > 0` and `trigger_threshold >= 0`; a
//!   cancelled subscription cannot be configured. The configuration lives
//!   under its own [`DataKey::AutoTopup`] key, so the stored `Subscription`
//!   shape is unchanged; `disable_auto_topup` removes it, and
//!   `get_auto_topup` reads it. Pull-mode billing and `charge_catchup`
//!   (which refuses prepaid subscriptions) never top up.
//!
//! ## Provider opt-in (explicit consent)
//!
//! A provider-initiated subscription (`subscribe_on_behalf_of`) is only
//! accepted for a provider the subscriber has **explicitly authorized** on
//! chain. The consent lives in instance storage under
//! [`DataKey::ProviderOptIn`], keyed by `(subscriber, provider)`; it is
//! written by `authorize_provider` and removed by `revoke_provider`, both
//! of which require the subscriber's authorization.
//!
//! The security property this preserves: a provider cannot create a
//! subscription that can later pull subscriber funds without the
//! subscriber's explicit on-chain consent. `charge` still authorizes
//! through the provider, so a subscription created by `subscribe` (subscriber
//! authorized at creation) remains fully chargeable, while a provider-created
//! subscription can only exist for a relationship the subscriber chose.
//!
//! The opt-in is a narrow, per-relationship flag — not a general permission
//! framework. Both path's records are stored in the same
//! [`Subscription`] shape with the same sequential id counter, so
//! [`get_subscription`] cannot distinguish a subscriber-created from a
//! provider-created record, and no consent-origin field is needed.
//!
//! ## Metered usage: quotas and overage
//!
//! A flat `amount` per period is a one-size-fits-all price. Production
//! products also price *usage*: a base period fee plus included units per
//! metric, with a per-bucket overage price for what exceeds the inclusion
//! (for example "10k API calls included, then 5 stroops per 1k over").
//!
//! - A subscription carries a declared quota list, [`Vec<MetricQuota>`]. An
//!   **empty list is the flat flow**: [`period_amount`] short-circuits to
//!   `amount`, the charged value and every event payload are bit-identical to
//!   a contract without this feature, and the existing regression suite is
//!   unchanged.
//! - The provider meters usage with `record_usage(subscription_id, metric,
//!   units)`, which accumulates **raw units** for the open period only, under
//!   [`DataKey::Usage`]. Recording is rejected for a metric the subscription
//!   does not declare, so a provider can never meter usage the subscriber has
//!   not already been asked to pay for.
//! - The amount a period bills is derived in one place — the pure
//!   [`period_amount`] helper, called by `charge`, by every period of
//!   `charge_catchup`, and by the `quote_period` view:
//!
//!   ```text
//!   amount = base
//!          + Σ over metric of ceil(min(max(0, units - included), cap) / bucket) * overage_price
//!   ```
//!
//!   with `cap` omitted for an uncapped quota. A started bucket is billed in
//!   full and the price is **per bucket**, never per unit, so no sub-unit
//!   price precision is ever lost.
//! - **No rounding drift is possible across periods.** The accumulator holds
//!   raw units and never a bucket count or a carried-over remainder, so each
//!   period's amount is a pure function of that period's units. Charging `N`
//!   periods bills exactly `N` times the per-period derivation — there is no
//!   state in which rounding can compound.
//! - Meters are dropped atomically with the charge that closes the period, in
//!   the same frame that advances `last_charged`. A **failed** transfer
//!   returns before that point, so every meter survives untouched and the
//!   retry re-derives a byte-identical amount.
//! - `charge_catchup` settles the open period **last**: usage can only be
//!   metered into the open (latest) period, so earlier unsettled periods bill
//!   the base amount and only the final one carries overage.
//! - `record_usage` requires an `Active` subscription, which is what freezes
//!   the meter at the attempted amount once a charge has failed and makes the
//!   arrears retry identical to the attempt that failed.
//!
//! ### Design decisions
//!
//! 1. **Cap policy** — a per-metric `max_overage_units` on *billable overage
//!    units*, enforced at settlement by clamping, never by rejecting. A charge
//!    is always billable, so a usage spike cannot wedge a subscription, and
//!    the subscriber's worst case per period is `base + Σ caps`. Rejecting at
//!    `record_usage` time was rejected because refused units would spill into
//!    the next period and break the "current period only" property.
//! 2. **Bucketing / rounding** — integer buckets priced per bucket, rounded
//!    **up** (a started bucket is charged in full), derived from raw units on
//!    every derivation. A cap that is not a multiple of `bucket_units` bills
//!    at most `ceil(cap / bucket_units)` buckets.
//! 3. **Accumulator storage class** — persistent storage
//!    ([`DataKey::Usage`]), one entry per `(subscription, metric)`, stamped
//!    with the `last_charged` it belongs to. Successful writes renew the
//!    workspace TTL horizon; the migration keeps the old instance records
//!    readable and upgrades them lazily on the next state-changing call.

//! ## Optional subscription policies
//!
//! Subscribers may set grace to 1..=MAX_GRACE_SECONDS (30 days). The first
//! failed charge starts a fixed deadline. Repeated attempts increase the
//! retry count without moving last_charged or extending the deadline. A
//! payment strictly before the deadline restores Active. At or after the
//! deadline, charge cancels and uses the existing prepaid refund path.
//! Zero disables grace and restores the three-attempt cancellation policy.
//!
//! Provider rotation requires a subscriber proposal and authorization by
//! the nominated provider on acceptance. Proposals may be replaced or
//! cancelled. Acceptance moves the provider index without changing billing
//! timing, balances, usage or arrears. Subsequent provider-authorized calls
//! require the accepted provider.
//!
//! Each metric can carry unused units up to a subscriber-selected cap.
//! Carried units are consumed before the new
//! allocation and reduce overage. Base amount, overage prices and cadence
//! retain their configured values. Rollover closes only on successful
//! settlement. Quota replacement clears the old rollover policy. The default
//! cap is zero. Policies are separate persistent entries, so existing
//! subscription and quota records remain decodable.

#[cfg(test)]
extern crate std;

use soroban_forge_shared_utils::{bump_entry, ForgeError};
use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, token, Address, Env,
    Symbol, TryFromVal, Val, Vec,
};

/// Maximum configurable grace duration, in seconds (30 days).
pub const MAX_GRACE_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Maximum failed attempts when grace is disabled.
const MAX_RETRIES: u32 = 3;

/// Hard upper bound for one catch-up invocation.
///
/// Keeping the loop bounded protects Soroban instruction limits while still
/// covering practical keeper downtime without requiring one transaction per
/// missed period.
const MAX_CATCHUP_PERIODS: u32 = 32;

/// Maximum free-trial duration in seconds (90 days).
///
/// Trials longer than this cap are rejected by `subscribe_with_trial` so the
/// billing gap stays bounded and accidental inputs (such as passing a Unix
/// timestamp instead of a duration) fail fast.
pub const TRIAL_MAX_SECONDS: u64 = 90 * 86_400;

/// Maximum number of per-metric quotas one subscription may declare.
///
/// Every charge derives its amount by walking the quota list, so the list is
/// bounded to keep that derivation — and the instance storage a metered
/// subscription uses — inside Soroban's instruction and state budgets.
const MAX_QUOTAS: u32 = 16;

/// Public interface for the Soroban Forge subscription payments contract.
#[contractclient(name = "SorobanForgeSubscriptionPaymentsClient")]
pub trait SorobanForgeSubscriptionPayments {
    /// Configure subscriber-approved grace, capped at MAX_GRACE_SECONDS.
    fn set_grace_period(env: Env, id: u64, seconds: u64) -> Result<(), ForgeError>;
    fn grace_deadline(env: Env, id: u64) -> Result<Option<u64>, ForgeError>;
    /// Propose or replace a subscription-specific provider change.
    fn propose_provider(env: Env, id: u64, provider: Address) -> Result<(), ForgeError>;
    fn cancel_provider_change(env: Env, id: u64) -> Result<(), ForgeError>;
    fn pending_provider(env: Env, id: u64) -> Result<Option<Address>, ForgeError>;
    /// The nominated provider accepts the subscriber's proposal.
    fn accept_provider(env: Env, id: u64, provider: Address) -> Result<(), ForgeError>;
    /// Configure a cap whose sum with included units fits in u64.
    fn set_rollover_cap(env: Env, id: u64, metric: Symbol, cap: u64) -> Result<(), ForgeError>;
    fn rollover_remaining(env: Env, id: u64, metric: Symbol) -> Result<u64, ForgeError>;

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

    /// Explicitly authorize `provider` to create subscriptions on
    /// `subscriber`'s behalf (see the module docs on provider opt-in).
    ///
    /// Requires the subscriber. Idempotent: authorizing a provider that is
    /// already authorized succeeds.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::Unauthorized`] — the caller is not `subscriber`.
    fn authorize_provider(
        env: Env,
        subscriber: Address,
        provider: Address,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Withdraw `subscriber`'s explicit authorization of `provider` (see the
    /// module docs on provider opt-in).
    ///
    /// Requires the subscriber. Idempotent: revoking a provider that is not
    /// authorized succeeds and leaves the opt-in absent.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::Unauthorized`] — the caller is not `subscriber`.
    fn revoke_provider(
        env: Env,
        subscriber: Address,
        provider: Address,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Read whether `subscriber` has explicitly authorized `provider`
    /// (read-only view; requires no authorization).
    fn is_provider_authorized(env: Env, subscriber: Address, provider: Address) -> bool;

    /// Subscribe `subscriber` to `provider`'s service on the provider's
    /// initiative, at `amount` per `period`. Returns the stable
    /// subscription id.
    ///
    /// Requires the provider and an explicit subscriber opt-in for that
    /// provider (see [`authorize_provider`]). The subscriber is **not**
    /// authorized at creation: the opt-in is the consent. Creates a normal
    /// subscription record indistinguishable from one created by
    /// [`subscribe`], sharing the same sequential id counter.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::Unauthorized`] — the caller is not `provider`, or the
    ///   subscriber has not authorized the provider.
    /// * [`ForgeError::InvalidInput`] — `amount` or `period` is not positive.
    fn subscribe_on_behalf_of(
        env: Env,
        provider: Address,
        subscriber: Address,
        token: Address,
        amount: i128,
        period: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    fn change_plan(env: Env, subscription_id: u64, new_amount: i128) -> Result<i128, ForgeError>;
    fn preview_catchup(
        env: Env,
        subscription_id: u64,
        max_periods: u32,
    ) -> Result<CatchupPreview, ForgeError>;

    /// Subscribe `subscriber` to `provider`'s service with a free trial whose
    /// first charge is due at `now + trial_seconds`. Returns the stable
    /// subscription id.
    ///
    /// `trial_seconds` must be positive and at most [`TRIAL_MAX_SECONDS`]
    /// (90 days). `charge` and `charge_catchup` bill nothing while
    /// `now < trial_end`; at `now >= trial_end` the first charge settles the
    /// first paid period without retroactively charging trial time, emitting
    /// `TrialConverted`. Cancelling during the trial costs nothing and refunds
    /// any prepaid balance in full; pausing during the trial extends
    /// `trial_end` on resume by the paused duration.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::InvalidInput`] — `amount <= 0`, `period == 0`,
    ///   `trial_seconds == 0`, or `trial_seconds > TRIAL_MAX_SECONDS`.
    /// * [`ForgeError::ArithmeticOverflow`] — `now + trial_seconds` overflows
    ///   `u64`.
    fn subscribe_with_trial(
        env: Env,
        subscriber: Address,
        provider: Address,
        token: Address,
        amount: i128,
        period: u64,
        trial_seconds: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Read the trial end timestamp for `subscription_id`, or `None` when the
    /// subscription has no trial (read-only view; requires no authorization).
    fn get_trial_end(env: Env, subscription_id: u64) -> Option<u64>;

    /// Charge the next due payment for `subscription_id`.
    ///
    /// Returns the billed amount, or `0` when no full period has elapsed since
    /// the last charge.
    fn charge(
        env: Env,
        subscription_id: u64,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Opt this subscription into prepaid mode and deposit `amount` tokens.
    /// Requires the subscriber; the exact SEP-41 transfer occurs before the
    /// balance is updated. A cancelled subscription cannot be funded.
    fn deposit(
        env: Env,
        subscription_id: u64,
        amount: i128,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Withdraw prepaid funds before cancellation. Requires the subscriber.
    /// The subscription remains in prepaid mode, including at a zero balance.
    fn withdraw_balance(
        env: Env,
        subscription_id: u64,
        amount: i128,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Authorize automatic prepaid top-ups for `subscription_id` (see the
    /// module docs on prepaid auto-topup), replacing any earlier setting.
    ///
    /// During a due prepaid `charge`, when the balance is below the period
    /// amount or below `trigger_threshold`, the contract first transfers
    /// `topup_amount` from the subscriber into custody.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::InvalidInput`] — `topup_amount <= 0`,
    ///   `trigger_threshold < 0`, or the subscription is `Cancelled`.
    /// * [`ForgeError::NotFound`] — no subscription with this id.
    /// * [`ForgeError::Unauthorized`] — the subscriber did not authorize.
    fn configure_auto_topup(
        env: Env,
        subscription_id: u64,
        trigger_threshold: i128,
        topup_amount: i128,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Stop automatic prepaid top-ups for `subscription_id`. Requires the
    /// subscriber; idempotent when none is configured.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::NotFound`] — no subscription with this id.
    /// * [`ForgeError::Unauthorized`] — the subscriber did not authorize.
    fn disable_auto_topup(
        env: Env,
        subscription_id: u64,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Read the auto-topup setting of `subscription_id`, or `None` when it is
    /// not configured (read-only view).
    fn get_auto_topup(env: Env, subscription_id: u64) -> Option<AutoTopup>;

    /// Atomically charge up to `max_periods` elapsed periods.
    ///
    /// Returns the total billed amount. `max_periods` must not exceed the
    /// contract's hard catch-up bound. A transfer failure returns
    /// [`ForgeError::TokenTransferFailed`] and rolls back all transfers,
    /// subscription state, and usage meters. `PastDue` subscriptions must use
    /// `charge` to preserve the existing retry policy.
    fn charge_catchup(
        env: Env,
        subscription_id: u64,
        max_periods: u32,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Declare (or replace) the metered-usage quotas of `subscription_id`.
    ///
    /// Requires the subscriber: a quota prices the overage the subscriber is
    /// billed, so the account that is charged is the one that consents to the
    /// terms. An empty list returns the subscription to flat pricing.
    ///
    /// Only valid while `Active` and only while the open period has no
    /// recorded usage, so already-metered units can never be repriced
    /// mid-period. Validated before authorization: at most [`MAX_QUOTAS`]
    /// quotas, no duplicate metrics, `bucket_units > 0`, and
    /// `overage_price >= 0`.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::InvalidInput`] — the subscription is not `Active`, the
    ///   open period already has recorded usage, or a quota failed validation.
    /// * [`ForgeError::Unauthorized`] — the caller is not the subscriber.
    /// * [`ForgeError::NotFound`] — no such subscription.
    fn set_quotas(
        env: Env,
        subscription_id: u64,
        quotas: soroban_sdk::Vec<MetricQuota>,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Record `units` of `metric` consumed in the subscription's open period.
    ///
    /// Requires the provider (the party that served the usage, mirroring who
    /// is authorized to `charge`). Accumulates into the open period's raw unit
    /// counter; the meters are dropped atomically by the charge that closes
    /// the period.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::NotFound`] — no such subscription.
    /// * [`ForgeError::InvalidInput`] — `units` is zero, the subscription is
    ///   not `Active`, or the subscription does not declare `metric`.
    /// * [`ForgeError::Unauthorized`] — the caller is not the provider.
    /// * [`ForgeError::ArithmeticOverflow`] — the period's unit counter would
    ///   overflow `u64`.
    fn record_usage(
        env: Env,
        subscription_id: u64,
        metric: soroban_sdk::Symbol,
        units: u64,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Read the usage recorded for `metric` in the open period (read-only
    /// view; requires no authorization).
    ///
    /// A metric with no recorded usage reads back as a zeroed record stamped
    /// with the current `last_charged`, so callers can poll one metric without
    /// branching on "never used".
    fn get_usage(
        env: Env,
        subscription_id: u64,
        metric: soroban_sdk::Symbol,
    ) -> Result<UsageRecord, soroban_forge_shared_utils::ForgeError>;

    /// The amount the open period would bill right now: `amount` plus the
    /// overage for every declared metric (read-only view; requires no
    /// authorization).
    ///
    /// This is the same derivation `charge` and `charge_catchup` settle, so a
    /// quote and the resulting charge cannot disagree. For a flat subscription
    /// it is exactly `amount`.
    fn quote_period(
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
    /// Requires the subscriber. Valid when `Active`, `Paused`, or `PastDue`.
    fn cancel(env: Env, subscription_id: u64)
        -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Cancel `subscription_id` on the provider's side, returning any unspent
    /// prepaid balance to the subscriber.
    ///
    /// Requires the provider. Valid when `Active`, `Paused`, or `PastDue`.
    /// A positive `prepaid_balance` is transferred back to the subscriber
    /// before the cancellation is stored, leaving `Some(0)`.
    ///
    /// # Errors
    ///
    /// * [`ForgeError::NotFound`] — no subscription with this id.
    /// * [`ForgeError::InvalidInput`] — the subscription is already
    ///   `Cancelled`.
    /// * [`ForgeError::TokenTransferFailed`] — the token contract rejected
    ///   the refund; nothing is changed.
    fn cancel_by_provider(
        env: Env,
        subscription_id: u64,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Read a stored subscription by id (read-only view).
    fn get_subscription(
        env: Env,
        subscription_id: u64,
    ) -> Result<Subscription, soroban_forge_shared_utils::ForgeError>;

    /// Extend the persistent TTL of one subscription record.
    fn touch_ttl(
        env: Env,
        subscription_id: u64,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

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
    /// Temporarily paused; no charges can be made until resumed.
    Paused,
}

/// Expected token amounts if the provider successfully settles the selected periods.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatchupPreview {
    pub chargeable_amount: i128,
    pub prepaid_consumed: i128,
    pub arrears_remaining: i128,
    pub resulting_status: SubscriptionStatus,
}

/// A recurring payment agreement.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subscription {
    /// Stable identifier assigned at creation.
    pub subscription_id: u64,
    /// Account being charged.
    pub subscriber: Address,
    /// Account receiving payments.
    pub provider: Address,
    /// Token contract used for settlement.
    pub token: Address,
    /// Amount charged per period, before usage overage.
    pub amount: i128,
    /// Length of one billing period, in seconds.
    pub period: u64,
    /// Ledger timestamp of the last successful charge.
    pub last_charged: u64,
    /// Current state.
    pub status: SubscriptionStatus,
    /// Ledger timestamp when paused, if currently paused.
    pub paused_at: Option<u64>,
    /// Number of consecutive failed billing attempts.
    pub failed_attempts: u32,
    /// Ledger timestamp at which the subscription first became overdue for the
    /// current open period, or `None` while the subscription is current.
    /// Set when a charge observes an elapsed due time without a successful
    /// payment; cleared on a successful charge.
    pub past_due_since: Option<u64>,
    /// Per-metric usage quotas priced on top of `amount`.
    ///
    /// Empty is the flat flow: the period bills exactly `amount`. The list is
    /// shaped so it can be embedded verbatim in a plan record (a plan copies
    /// its quotas here when a subscriber joins it), keeping the metering core
    /// independent of where the terms were published.
    pub quotas: Vec<MetricQuota>,
    /// `None` means pull mode; `Some(balance)` means prepaid mode is enabled.
    /// A zero balance remains `Some(0)` so topping up never changes modes.
    pub prepaid_balance: Option<i128>,
}

/// Subscription record written before prepaid balances were introduced.
///
/// Legacy records remain in instance storage after the persistent-storage
/// migration. They are decoded by `get_subscription_impl` and are rewritten
/// in the current shape on their next state-changing operation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct SubscriptionV1 {
    subscription_id: u64,
    subscriber: Address,
    provider: Address,
    token: Address,
    amount: i128,
    period: u64,
    last_charged: u64,
    status: SubscriptionStatus,
    paused_at: Option<u64>,
    failed_attempts: u32,
    past_due_since: Option<u64>,
    quotas: Vec<MetricQuota>,
}

impl From<SubscriptionV1> for Subscription {
    fn from(legacy: SubscriptionV1) -> Self {
        Self {
            subscription_id: legacy.subscription_id,
            subscriber: legacy.subscriber,
            provider: legacy.provider,
            token: legacy.token,
            amount: legacy.amount,
            period: legacy.period,
            last_charged: legacy.last_charged,
            status: legacy.status,
            paused_at: legacy.paused_at,
            failed_attempts: legacy.failed_attempts,
            past_due_since: legacy.past_due_since,
            quotas: legacy.quotas,
            prepaid_balance: None,
        }
    }
}

/// One metered dimension's pricing terms: units included in the base period
/// price plus the per-bucket overage price for what exceeds the inclusion.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricQuota {
    /// The metered dimension, e.g. `api_calls` or `storage_bytes`.
    pub metric: Symbol,
    /// Units covered by the base `amount`; usage up to and including this
    /// many units bills nothing extra.
    pub included_units: u64,
    /// Price of one started bucket of overage units, in the subscription's
    /// token. Per bucket, never per unit, so no sub-unit price precision is
    /// lost to integer division.
    pub overage_price: i128,
    /// Units per overage bucket. Must be greater than zero; a started bucket
    /// is billed in full (rounding is up).
    pub bucket_units: u64,
    /// Cap on billable overage units for one period, or `None` for uncapped.
    ///
    /// The cap clamps overage units before they are bucketed, so a period can
    /// bill at most `ceil(cap / bucket_units)` buckets for this metric.
    pub max_overage_units: Option<u64>,
}

/// Usage metered for one metric in one billing period.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageRecord {
    /// Subscription the usage was recorded against.
    pub subscription_id: u64,
    /// The metered dimension.
    pub metric: Symbol,
    /// Raw units accumulated in the open period. Always a unit count, never a
    /// bucket count, so no rounding state can survive into the next period.
    pub units: u64,
    /// The `last_charged` this meter belongs to. A meter whose stamp no
    /// longer matches the subscription's open period is not counted, and the
    /// next `record_usage` restarts it.
    pub period_start: u64,
}

/// A subscriber's authorization for automatic prepaid top-ups (see the
/// module docs on prepaid auto-topup).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoTopup {
    /// Top up when the prepaid balance is below this floor at billing time
    /// (in addition to whenever it cannot cover the period). Never negative.
    pub trigger_threshold: i128,
    /// Amount pulled from the subscriber into custody per top-up. Positive.
    pub topup_amount: i128,
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
    /// Explicit subscriber opt-in for a provider-initiated subscription:
    /// `(subscriber, provider)`. Written by `authorize_provider`, removed by
    /// `revoke_provider`, consulted by `subscribe_on_behalf_of`.
    ProviderOptIn(Address, Address),
    /// The open period's unit counter for `(subscription, metric)`. Written by
    /// `record_usage`, removed by the charge that closes the period, read by
    /// the amount derivation.
    Usage(u64, Symbol),
    /// The subscriber's [`AutoTopup`] authorization for a subscription.
    /// Persistent; written by `configure_auto_topup`, removed by
    /// `disable_auto_topup`, read by `charge`.
    AutoTopup(u64),
    PlanCredit(u64),
    GracePeriod(u64),
    GraceDeadline(u64),
    PendingProvider(u64),
    RolloverCap(u64, Symbol),
    RolloverUnits(u64, Symbol),
    /// Trial end timestamp for a subscription created via
    /// `subscribe_with_trial`. Stored under its own persistent key so the
    /// `Subscription` record layout remains compatible.
    TrialEnd(u64),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct RolloverBalance {
    period_start: u64,
    units: u64,
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

        Self::create_subscription(&env, subscriber, provider, token, amount, period)
    }

    /// Explicitly authorize `provider` to create subscriptions on
    /// `subscriber`'s behalf (see the module docs on provider opt-in).
    ///
    /// Requires the subscriber. Idempotent: authorizing a provider that is
    /// already authorized succeeds.
    pub fn authorize_provider(
        env: Env,
        subscriber: Address,
        provider: Address,
    ) -> Result<(), ForgeError> {
        subscriber.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::ProviderOptIn(subscriber, provider), &true);
        Ok(())
    }

    /// Withdraw `subscriber`'s explicit authorization of `provider` (see the
    /// module docs on provider opt-in).
    ///
    /// Requires the subscriber. Idempotent: revoking a provider that is not
    /// authorized succeeds and leaves the opt-in absent.
    pub fn revoke_provider(
        env: Env,
        subscriber: Address,
        provider: Address,
    ) -> Result<(), ForgeError> {
        subscriber.require_auth();

        env.storage()
            .instance()
            .remove(&DataKey::ProviderOptIn(subscriber, provider));
        Ok(())
    }

    /// Read whether `subscriber` has explicitly authorized `provider`
    /// (read-only view; requires no authorization).
    pub fn is_provider_authorized(env: Env, subscriber: Address, provider: Address) -> bool {
        Self::is_provider_authorized_impl(&env, &subscriber, &provider)
    }

    fn is_provider_authorized_impl(env: &Env, subscriber: &Address, provider: &Address) -> bool {
        env.storage().instance().has(&DataKey::ProviderOptIn(
            subscriber.clone(),
            provider.clone(),
        ))
    }

    /// Subscribe `subscriber` to `provider`'s service on the provider's
    /// initiative (see the module docs on provider opt-in).
    ///
    /// Requires `amount > 0`, `period > 0`, the provider's authorization, and
    /// an explicit subscriber opt-in for the provider (checked before the
    /// provider is authorized, so a missing opt-in surfaces without spending
    /// the provider's signature). The subscriber is **not** authorized at
    /// creation. The record is created through the same internal path as
    /// [`subscribe`], sharing the same sequential id counter.
    pub fn subscribe_on_behalf_of(
        env: Env,
        provider: Address,
        subscriber: Address,
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
        // The opt-in is the subscriber's consent: without it, a provider
        // must not be able to stand up a subscription that later pulls
        // subscriber funds.
        if !Self::is_provider_authorized_impl(&env, &subscriber, &provider) {
            return Err(ForgeError::Unauthorized);
        }
        provider.require_auth();

        Self::create_subscription(&env, subscriber, provider, token, amount, period)
    }

    /// Opt into grace of 1 second to 30 days. Zero restores the legacy
    /// three-attempt policy. Terms may only change while Active.
    pub fn set_grace_period(env: Env, id: u64, seconds: u64) -> Result<(), ForgeError> {
        let sub = Self::get_subscription_impl(&env, id)?;
        sub.subscriber.require_auth();
        if sub.status != SubscriptionStatus::Active || seconds > MAX_GRACE_SECONDS {
            return Err(ForgeError::InvalidInput);
        }
        let key = DataKey::GracePeriod(id);
        if seconds == 0 {
            env.storage().persistent().remove(&key);
        } else {
            env.storage().persistent().set(&key, &seconds);
            bump_entry(&env, &key);
        }
        Ok(())
    }

    pub fn grace_deadline(env: Env, id: u64) -> Result<Option<u64>, ForgeError> {
        Self::get_subscription_impl(&env, id)?;
        Ok(env.storage().persistent().get(&DataKey::GraceDeadline(id)))
    }

    /// Subscriber consent is bound to this subscription and proposed address.
    pub fn propose_provider(env: Env, id: u64, provider: Address) -> Result<(), ForgeError> {
        let sub = Self::get_subscription_impl(&env, id)?;
        sub.subscriber.require_auth();
        if sub.status == SubscriptionStatus::Cancelled || provider == sub.provider {
            return Err(ForgeError::InvalidInput);
        }
        let key = DataKey::PendingProvider(id);
        env.storage().persistent().set(&key, &provider);
        bump_entry(&env, &key);
        Ok(())
    }

    pub fn cancel_provider_change(env: Env, id: u64) -> Result<(), ForgeError> {
        let sub = Self::get_subscription_impl(&env, id)?;
        sub.subscriber.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::PendingProvider(id));
        Ok(())
    }

    pub fn pending_provider(env: Env, id: u64) -> Result<Option<Address>, ForgeError> {
        Self::get_subscription_impl(&env, id)?;
        Ok(env
            .storage()
            .persistent()
            .get(&DataKey::PendingProvider(id)))
    }

    pub fn accept_provider(env: Env, id: u64, provider: Address) -> Result<(), ForgeError> {
        let mut sub = Self::get_subscription_impl(&env, id)?;
        if sub.status == SubscriptionStatus::Cancelled
            || env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::PendingProvider(id))
                != Some(provider.clone())
        {
            return Err(ForgeError::InvalidInput);
        }
        provider.require_auth();
        let old_key = DataKey::ProviderSubscriptions(sub.provider.clone());
        let mut old_ids = Self::index_ids(&env, &old_key);
        if let Some(index) = old_ids.first_index_of(id) {
            old_ids.remove(index);
        }
        env.storage().persistent().set(&old_key, &old_ids);
        bump_entry(&env, &old_key);
        // Insert by creation id to preserve the documented index ordering.
        let new_key = DataKey::ProviderSubscriptions(provider.clone());
        let mut new_ids = Self::index_ids(&env, &new_key);
        let at = new_ids
            .iter()
            .position(|existing| existing > id)
            .unwrap_or(new_ids.len() as usize);
        new_ids.insert(at as u32, id);
        env.storage().persistent().set(&new_key, &new_ids);
        bump_entry(&env, &new_key);
        sub.provider = provider;
        Self::store_subscription(&env, id, &sub);
        env.storage()
            .persistent()
            .remove(&DataKey::PendingProvider(id));
        Ok(())
    }

    /// Cap unused units carried forward. Cap plus included units must fit
    /// in u64. Changing terms requires an unused Active period.
    pub fn set_rollover_cap(env: Env, id: u64, metric: Symbol, cap: u64) -> Result<(), ForgeError> {
        let sub = Self::get_subscription_impl(&env, id)?;
        sub.subscriber.require_auth();
        let quota = sub
            .quotas
            .iter()
            .find(|quota| quota.metric == metric)
            .ok_or(ForgeError::InvalidInput)?;
        if sub.status != SubscriptionStatus::Active
            || Self::has_recorded_usage(&env, &sub)
            || cap.checked_add(quota.included_units).is_none()
        {
            return Err(ForgeError::InvalidInput);
        }
        let key = DataKey::RolloverCap(id, metric.clone());
        if cap == 0 {
            env.storage().persistent().remove(&key);
        } else {
            env.storage().persistent().set(&key, &cap);
            bump_entry(&env, &key);
        }
        env.storage()
            .persistent()
            .remove(&DataKey::RolloverUnits(id, metric));
        Ok(())
    }

    /// Remaining carried units after recorded usage, before base allocation.
    pub fn rollover_remaining(env: Env, id: u64, metric: Symbol) -> Result<u64, ForgeError> {
        let sub = Self::get_subscription_impl(&env, id)?;
        if !declares_metric(&sub, &metric) {
            return Err(ForgeError::InvalidInput);
        }
        let used =
            Self::read_open_period_usage(&env, &sub, &metric).map_or(0, |record| record.units);
        Ok(Self::opening_rollover(&env, &sub, &metric).saturating_sub(used))
    }

    fn opening_rollover(env: &Env, sub: &Subscription, metric: &Symbol) -> u64 {
        env.storage()
            .persistent()
            .get::<_, RolloverBalance>(&DataKey::RolloverUnits(sub.subscription_id, metric.clone()))
            .filter(|record| record.period_start == sub.last_charged)
            .map_or(0, |record| record.units)
    }

    fn renew_policies(env: &Env, sub: &Subscription) {
        for key in [
            DataKey::GracePeriod(sub.subscription_id),
            DataKey::GraceDeadline(sub.subscription_id),
            DataKey::PendingProvider(sub.subscription_id),
        ] {
            if env.storage().persistent().has(&key) {
                bump_entry(env, &key);
            }
        }
        for quota in sub.quotas.iter() {
            for key in [
                DataKey::RolloverCap(sub.subscription_id, quota.metric.clone()),
                DataKey::RolloverUnits(sub.subscription_id, quota.metric),
            ] {
                if env.storage().persistent().has(&key) {
                    bump_entry(env, &key);
                }
            }
        }
    }

    /// Create a subscription with a free trial whose first charge is due at
    /// `now + trial_seconds` (see trait docs).
    pub fn subscribe_with_trial(
        env: Env,
        subscriber: Address,
        provider: Address,
        token: Address,
        amount: i128,
        period: u64,
        trial_seconds: u64,
    ) -> Result<u64, ForgeError> {
        if amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        if period == 0 {
            return Err(ForgeError::InvalidInput);
        }
        if trial_seconds == 0 || trial_seconds > TRIAL_MAX_SECONDS {
            return Err(ForgeError::InvalidInput);
        }
        let trial_end = env
            .ledger()
            .timestamp()
            .checked_add(trial_seconds)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        subscriber.require_auth();

        let subscription_id =
            Self::create_subscription(&env, subscriber, provider, token, amount, period)?;
        Self::store_trial_end(&env, subscription_id, trial_end);
        events::trial_started(&env, subscription_id, trial_end);
        Ok(subscription_id)
    }

    /// Read the trial end timestamp for `subscription_id` (read-only view).
    ///
    /// Returns `None` when the subscription has no trial.
    pub fn get_trial_end(env: Env, subscription_id: u64) -> Option<u64> {
        Self::read_trial_end(&env, subscription_id)
    }

    fn read_trial_end(env: &Env, subscription_id: u64) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::TrialEnd(subscription_id))
    }

    fn store_trial_end(env: &Env, subscription_id: u64, trial_end: u64) {
        let key = DataKey::TrialEnd(subscription_id);
        env.storage().persistent().set(&key, &trial_end);
        bump_entry(env, &key);
    }

    /// Return `Some(trial_end)` while the subscription's trial has not yet
    /// converted to paid billing (`last_charged < trial_end`).
    fn pending_trial_end(env: &Env, subscription: &Subscription) -> Option<u64> {
        let trial_end = Self::read_trial_end(env, subscription.subscription_id)?;
        if subscription.last_charged < trial_end {
            Some(trial_end)
        } else {
            None
        }
    }

    /// Bill one due period.
    ///
    /// Requires the provider. If a full period has not elapsed since the last
    /// charge, returns `0` and leaves the subscription untouched. Otherwise
    /// attempts to transfer the period's derived amount of `token` from
    /// `subscriber` to `provider`.
    ///
    /// The derived amount is `amount` plus the overage for every declared
    /// metric (see the module docs on metered usage) — exactly `amount` when no
    /// quotas are declared. It is computed before the transfer, so an
    /// arithmetic failure never moves funds.
    ///
    /// - On successful payment: advances `last_charged` by one period, drops
    ///   the period's usage meters, resets `failed_attempts` to 0, transitions
    ///   status to `Active`, and returns the derived amount.
    /// - On failed payment: `last_charged` is NOT advanced and the usage
    ///   meters are NOT dropped, so the retry re-derives the same amount.
    ///   Increments `failed_attempts`. If `failed_attempts >= MAX_RETRIES`
    ///   (3), status becomes `Cancelled` unless grace is configured. During grace
    ///   failures remain `PastDue` until the fixed deadline.
    ///   Returns `0`.
    /// - In prepaid mode with auto-topup configured, a low balance is first
    ///   topped up from the subscriber (see the module docs on prepaid
    ///   auto-topup); a rejected top-up falls back to the rules above.
    pub fn charge(env: Env, subscription_id: u64) -> Result<i128, ForgeError> {
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status == SubscriptionStatus::Cancelled
            || subscription.status == SubscriptionStatus::Paused
        {
            return Err(ForgeError::InvalidInput);
        }
        subscription.provider.require_auth();

        let pending_trial = Self::pending_trial_end(&env, &subscription);
        let next_due = match pending_trial {
            Some(trial_end) => trial_end,
            None => subscription
                .last_charged
                .checked_add(subscription.period)
                .ok_or(ForgeError::ArithmeticOverflow)?,
        };
        if env.ledger().timestamp() < next_due {
            return Ok(0);
        }

        if let Some(deadline) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::GraceDeadline(subscription_id))
        {
            if env.ledger().timestamp() >= deadline {
                Self::settle_cancellation(&env, subscription_id, subscription)?;
                return Ok(0);
            }
        }

        // Derived before the transfer: a usage or arithmetic failure leaves
        // the subscription untouched and no funds in flight.
        let (amount, credit_left) = apply_credit(
            Self::derive_period_amount(&env, &subscription)?,
            Self::available_credit(&env, subscription_id),
        );

        let (payer, prepaid_after) = if let Some(mut balance) = subscription.prepaid_balance {
            if let Some(topup) = Self::auto_topup_impl(&env, subscription_id) {
                if amount > 0 && (balance < amount || balance < topup.trigger_threshold) {
                    balance = Self::run_auto_topup(&env, &mut subscription, &topup, balance);
                }
            }
            if balance < amount {
                Self::record_failed_charge(&env, &mut subscription)?;
                return Ok(0);
            }
            (env.current_contract_address(), Some(balance - amount))
        } else {
            (subscription.subscriber.clone(), None)
        };
        if !Self::transfer_charge(&env, &subscription, &payer, amount) {
            Self::record_failed_charge(&env, &mut subscription)?;
            return Ok(0);
        }

        Self::close_credit_period(&env, subscription_id, credit_left);
        subscription.prepaid_balance = prepaid_after;
        subscription.last_charged = next_due;
        subscription.failed_attempts = 0;
        subscription.past_due_since = None;
        env.storage()
            .persistent()
            .remove(&DataKey::GraceDeadline(subscription_id));
        subscription.status = SubscriptionStatus::Active;
        subscription.paused_at = None;
        Self::rollover_usage(&env, &subscription);
        Self::store_subscription(&env, subscription_id, &subscription);
        if let Some(trial_end) = pending_trial {
            events::trial_converted(&env, subscription_id, trial_end);
        }
        events::charged(&env, &subscription, amount);
        if let Some(remaining) = prepaid_after {
            events::balance_debited(&env, subscription_id, amount, remaining);
        }
        Ok(amount)
    }

    fn transfer_charge(env: &Env, sub: &Subscription, payer: &Address, amount: i128) -> bool {
        amount == 0
            || matches!(
                token::TokenClient::new(env, &sub.token).try_transfer(
                    payer,
                    &sub.provider,
                    &amount
                ),
                Ok(Ok(()))
            )
    }

    fn record_failed_charge(env: &Env, subscription: &mut Subscription) -> Result<(), ForgeError> {
        if let Some(seconds) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::GracePeriod(subscription.subscription_id))
        {
            let key = DataKey::GraceDeadline(subscription.subscription_id);
            let deadline = match env.storage().persistent().get::<_, u64>(&key) {
                Some(deadline) => deadline,
                None => env
                    .ledger()
                    .timestamp()
                    .checked_add(seconds)
                    .ok_or(ForgeError::ArithmeticOverflow)?,
            };
            if env.ledger().timestamp() >= deadline {
                return Self::settle_cancellation(
                    env,
                    subscription.subscription_id,
                    subscription.clone(),
                );
            }
            subscription.failed_attempts = subscription.failed_attempts.saturating_add(1);
            subscription.status = SubscriptionStatus::PastDue;
            subscription.past_due_since = Some(deadline - seconds);
            env.storage().persistent().set(&key, &deadline);
            Self::store_subscription(env, subscription.subscription_id, subscription);
            return Ok(());
        }
        let failed = subscription.failed_attempts.saturating_add(1);
        subscription.failed_attempts = failed;
        subscription.status = if failed >= MAX_RETRIES {
            SubscriptionStatus::Cancelled
        } else {
            SubscriptionStatus::PastDue
        };
        let refund = if subscription.status == SubscriptionStatus::Cancelled {
            if let Some(balance) = subscription.prepaid_balance {
                if balance > 0 {
                    let transfer_result = token::TokenClient::new(env, &subscription.token)
                        .try_transfer(
                            &env.current_contract_address(),
                            &subscription.subscriber,
                            &balance,
                        );
                    if !matches!(transfer_result, Ok(Ok(()))) {
                        return Err(ForgeError::TokenTransferFailed);
                    }
                    subscription.prepaid_balance = Some(0);
                    Some(balance)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        Self::store_subscription(env, subscription.subscription_id, subscription);
        if let Some(amount) = refund {
            events::balance_refunded(env, subscription.subscription_id, amount, 0);
        }
        Ok(())
    }

    fn auto_topup_impl(env: &Env, subscription_id: u64) -> Option<AutoTopup> {
        let key = DataKey::AutoTopup(subscription_id);
        let topup = env.storage().persistent().get(&key);
        if topup.is_some() {
            bump_entry(env, &key);
        }
        topup
    }

    /// Pull one configured top-up from the subscriber into custody and
    /// return the resulting prepaid balance.
    ///
    /// Transfer before state: the in-memory balance changes only after the
    /// transfer succeeded. A rejected transfer or an overflowing balance
    /// skips the top-up and returns `balance` unchanged, so the caller's
    /// ordinary sufficient/insufficient-balance path applies untouched.
    fn run_auto_topup(
        env: &Env,
        subscription: &mut Subscription,
        topup: &AutoTopup,
        balance: i128,
    ) -> i128 {
        let Some(updated) = balance.checked_add(topup.topup_amount) else {
            return balance;
        };
        let transfer_result = token::TokenClient::new(env, &subscription.token).try_transfer(
            &subscription.subscriber,
            env.current_contract_address(),
            &topup.topup_amount,
        );
        if !matches!(transfer_result, Ok(Ok(()))) {
            return balance;
        }
        subscription.prepaid_balance = Some(updated);
        events::auto_topup_triggered(
            env,
            subscription.subscription_id,
            topup.topup_amount,
            updated,
        );
        updated
    }

    /// Change the base price by mutual consent and return newly granted credit.
    pub fn change_plan(
        env: Env,
        subscription_id: u64,
        new_amount: i128,
    ) -> Result<i128, ForgeError> {
        let mut sub = Self::get_subscription_impl(&env, subscription_id)?;
        if sub.status != SubscriptionStatus::Active || new_amount <= 0 || new_amount == sub.amount {
            return Err(ForgeError::InvalidInput);
        }
        sub.subscriber.require_auth();
        sub.provider.require_auth();
        let key = DataKey::PlanCredit(subscription_id);
        let (balance, credited): (i128, bool) =
            env.storage().persistent().get(&key).unwrap_or_default();
        let elapsed = env
            .ledger()
            .timestamp()
            .checked_sub(sub.last_charged)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        let remaining = sub.period.saturating_sub(elapsed);
        // Splitting off the quotient avoids overflowing old_price * remaining.
        // Both factors of the remainder product fit in u64.
        let period = i128::from(sub.period);
        let credit = if credited {
            0
        } else {
            let fraction =
                ((sub.amount % period) as u128 * u128::from(remaining)) / u128::from(sub.period);
            (sub.amount / period)
                .checked_mul(i128::from(remaining))
                .and_then(|whole| whole.checked_add(fraction as i128))
                .ok_or(ForgeError::ArithmeticOverflow)?
        };
        let available = balance
            .checked_add(credit)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        env.storage().persistent().set(&key, &(available, true));
        let old_amount = sub.amount;
        sub.amount = new_amount;
        Self::store_subscription(&env, subscription_id, &sub);
        events::PlanChanged {
            subscription_id,
            old_amount,
            new_amount,
            credit,
        }
        .publish(&env);
        Ok(credit)
    }

    fn available_credit(env: &Env, id: u64) -> i128 {
        env.storage()
            .persistent()
            .get::<_, (i128, bool)>(&DataKey::PlanCredit(id))
            .unwrap_or_default()
            .0
    }

    fn close_credit_period(env: &Env, id: u64, remaining: i128) {
        let key = DataKey::PlanCredit(id);
        if remaining > 0 {
            env.storage().persistent().set(&key, &(remaining, false));
        } else if env.storage().persistent().has(&key) {
            env.storage().persistent().remove(&key);
        }
    }

    /// Pull tokens into custody and enable prepaid mode only after success.
    pub fn deposit(env: Env, subscription_id: u64, amount: i128) -> Result<i128, ForgeError> {
        if amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status == SubscriptionStatus::Cancelled {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();
        let current = subscription.prepaid_balance.unwrap_or(0);
        let updated = current
            .checked_add(amount)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        let transfer_result = token::TokenClient::new(&env, &subscription.token).try_transfer(
            &subscription.subscriber,
            env.current_contract_address(),
            &amount,
        );
        if !matches!(transfer_result, Ok(Ok(()))) {
            return Err(ForgeError::TokenTransferFailed);
        }
        subscription.prepaid_balance = Some(updated);
        Self::store_subscription(&env, subscription_id, &subscription);
        events::deposited(&env, subscription_id, amount, updated);
        Ok(updated)
    }

    /// Return prepaid funds to the subscriber while retaining prepaid mode.
    pub fn withdraw_balance(
        env: Env,
        subscription_id: u64,
        amount: i128,
    ) -> Result<i128, ForgeError> {
        if amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status == SubscriptionStatus::Cancelled {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();
        let balance = subscription
            .prepaid_balance
            .ok_or(ForgeError::InsufficientFunds)?;
        // `checked_sub` on i128 only catches overflow, not a negative result,
        // so compare explicitly: otherwise an over-withdrawal would be paid out
        // of other subscriptions' escrowed funds.
        if amount > balance {
            return Err(ForgeError::InsufficientFunds);
        }
        let remaining = balance - amount;
        let transfer_result = token::TokenClient::new(&env, &subscription.token).try_transfer(
            &env.current_contract_address(),
            &subscription.subscriber,
            &amount,
        );
        if !matches!(transfer_result, Ok(Ok(()))) {
            return Err(ForgeError::TokenTransferFailed);
        }
        subscription.prepaid_balance = Some(remaining);
        Self::store_subscription(&env, subscription_id, &subscription);
        events::balance_refunded(&env, subscription_id, amount, remaining);
        Ok(remaining)
    }

    /// Authorize automatic prepaid top-ups (see the trait docs).
    pub fn configure_auto_topup(
        env: Env,
        subscription_id: u64,
        trigger_threshold: i128,
        topup_amount: i128,
    ) -> Result<(), ForgeError> {
        if topup_amount <= 0 || trigger_threshold < 0 {
            return Err(ForgeError::InvalidInput);
        }
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status == SubscriptionStatus::Cancelled {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();
        let key = DataKey::AutoTopup(subscription_id);
        env.storage().persistent().set(
            &key,
            &AutoTopup {
                trigger_threshold,
                topup_amount,
            },
        );
        bump_entry(&env, &key);
        Ok(())
    }

    /// Stop automatic prepaid top-ups (see the trait docs).
    pub fn disable_auto_topup(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        subscription.subscriber.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::AutoTopup(subscription_id));
        Ok(())
    }

    /// Read the auto-topup setting, if any (read-only view).
    pub fn get_auto_topup(env: Env, subscription_id: u64) -> Option<AutoTopup> {
        Self::auto_topup_impl(&env, subscription_id)
    }

    /// Atomically bill multiple elapsed periods.
    pub fn charge_catchup(
        env: Env,
        subscription_id: u64,
        max_periods: u32,
    ) -> Result<i128, ForgeError> {
        let (mut subscription, elapsed) = Self::catchup_terms(&env, subscription_id, max_periods)?;
        subscription.provider.require_auth();
        let periods = elapsed.min(u64::from(max_periods));
        let pending_trial = Self::pending_trial_end(&env, &subscription);
        let mut credit = Self::available_credit(&env, subscription_id);
        let mut total = 0_i128;
        for index in 0..periods {
            let converting_trial = index == 0 && pending_trial.is_some();
            let next_due = if let Some(trial_end) = pending_trial.filter(|_| index == 0) {
                trial_end
            } else {
                subscription
                    .last_charged
                    .checked_add(subscription.period)
                    .ok_or(ForgeError::ArithmeticOverflow)?
            };
            // Usage can only be metered into the open (latest) period, and the
            // meters are dropped as each period closes, so the first iteration
            // is the only one that can carry overage: every period that follows
            // derives the base amount. Same helper, same arithmetic, per period.
            let (amount, left) = Self::catchup_period(&env, &subscription, index, credit)?;
            credit = left;
            if !Self::transfer_charge(&env, &subscription, &subscription.subscriber, amount) {
                return Err(ForgeError::TokenTransferFailed);
            }
            total = total
                .checked_add(amount)
                .ok_or(ForgeError::ArithmeticOverflow)?;
            subscription.last_charged = next_due;
            Self::rollover_usage(&env, &subscription);
            if converting_trial {
                if let Some(trial_end) = pending_trial {
                    events::trial_converted(&env, subscription_id, trial_end);
                }
            }
            events::charged(&env, &subscription, amount);
        }

        if periods > 0 {
            Self::close_credit_period(&env, subscription_id, credit);
            subscription.failed_attempts = 0;
            subscription.status = SubscriptionStatus::Active;
            subscription.paused_at = None;
            Self::store_subscription(&env, subscription_id, &subscription);
        }
        Ok(total)
    }

    fn catchup_period(
        env: &Env,
        sub: &Subscription,
        index: u64,
        credit: i128,
    ) -> Result<(i128, i128), ForgeError> {
        let gross = if index == 0 {
            Self::derive_period_amount(env, sub)?
        } else {
            sub.amount
        };
        Ok(apply_credit(gross, credit))
    }

    fn catchup_terms(env: &Env, id: u64, limit: u32) -> Result<(Subscription, u64), ForgeError> {
        if limit > MAX_CATCHUP_PERIODS {
            return Err(ForgeError::InvalidInput);
        }
        let sub = Self::get_subscription_impl(env, id)?;
        if sub.status != SubscriptionStatus::Active || sub.prepaid_balance.is_some() {
            return Err(ForgeError::InvalidInput);
        }
        let now = env.ledger().timestamp();
        let periods = match Self::pending_trial_end(env, &sub) {
            Some(trial_end) => {
                if now < trial_end {
                    0
                } else {
                    ((now - trial_end) / sub.period).saturating_add(1)
                }
            }
            None => {
                let elapsed = match now.checked_sub(sub.last_charged) {
                    Some(seconds) => seconds,
                    None if limit == 0 => 0,
                    None => return Err(ForgeError::ArithmeticOverflow),
                };
                elapsed / sub.period
            }
        };
        Ok((sub, periods))
    }

    /// Quote a successful catch-up, assuming the token transfers are authorized.
    pub fn preview_catchup(
        env: Env,
        subscription_id: u64,
        max_periods: u32,
    ) -> Result<CatchupPreview, ForgeError> {
        let (sub, elapsed) = Self::catchup_terms(&env, subscription_id, max_periods)?;
        let periods = elapsed.min(u64::from(max_periods));
        let mut credit = Self::available_credit(&env, subscription_id);
        let mut chargeable_amount = 0_i128;
        let mut arrears_remaining = 0_i128;
        for index in 0..periods {
            let (amount, left) = Self::catchup_period(&env, &sub, index, credit)?;
            chargeable_amount = chargeable_amount
                .checked_add(amount)
                .ok_or(ForgeError::ArithmeticOverflow)?;
            credit = left;
        }
        if elapsed > periods {
            let (first, left) = Self::catchup_period(&env, &sub, periods, credit)?;
            let (rest, _) = flat_periods_due(sub.amount, elapsed - periods - 1, left)?;
            arrears_remaining = first
                .checked_add(rest)
                .ok_or(ForgeError::ArithmeticOverflow)?;
        }
        Ok(CatchupPreview {
            chargeable_amount,
            prepaid_consumed: 0,
            arrears_remaining,
            resulting_status: sub.status,
        })
    }

    /// Declare (or replace) the metered-usage quotas of `subscription_id`.
    ///
    /// Requires the subscriber — see the module docs on metered usage for why
    /// the account being charged consents to the overage price. Rejected unless
    /// the subscription is `Active` and the open period has no recorded usage,
    /// so already-metered units can never be repriced mid-period.
    pub fn set_quotas(
        env: Env,
        subscription_id: u64,
        quotas: Vec<MetricQuota>,
    ) -> Result<(), ForgeError> {
        validate_quotas(&quotas)?;
        let mut subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status != SubscriptionStatus::Active {
            return Err(ForgeError::InvalidInput);
        }
        if Self::has_recorded_usage(&env, &subscription) {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();

        for quota in subscription.quotas.iter() {
            env.storage()
                .persistent()
                .remove(&DataKey::RolloverCap(subscription_id, quota.metric.clone()));
            env.storage()
                .persistent()
                .remove(&DataKey::RolloverUnits(subscription_id, quota.metric));
        }
        subscription.quotas = quotas;
        Self::store_subscription(&env, subscription_id, &subscription);
        events::quotas_set(&env, &subscription);
        Ok(())
    }

    /// Record `units` of `metric` consumed in the subscription's open period.
    ///
    /// Requires the provider. See the module docs on metered usage for the
    /// period-boundary, cap, and retry semantics.
    pub fn record_usage(
        env: Env,
        subscription_id: u64,
        metric: Symbol,
        units: u64,
    ) -> Result<(), ForgeError> {
        if units == 0 {
            return Err(ForgeError::InvalidInput);
        }
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status != SubscriptionStatus::Active {
            return Err(ForgeError::InvalidInput);
        }
        if !declares_metric(&subscription, &metric) {
            return Err(ForgeError::InvalidInput);
        }
        subscription.provider.require_auth();

        // A meter belongs to the open period only. `charge` and `charge_catchup`
        // drop the meters when they close a period, so the stamp normally
        // already matches; the check also covers a window moved by `resume`.
        let open = Self::read_open_period_usage(&env, &subscription, &metric);
        let units_before = open.as_ref().map_or(0, |record| record.units);
        let period_units = units_before
            .checked_add(units)
            .ok_or(ForgeError::ArithmeticOverflow)?;

        let recorded = UsageRecord {
            subscription_id,
            metric: metric.clone(),
            units: period_units,
            period_start: subscription.last_charged,
        };
        let usage_key = DataKey::Usage(subscription_id, metric);
        env.storage().persistent().set(&usage_key, &recorded);
        bump_entry(&env, &usage_key);
        Self::renew_policies(&env, &subscription);
        events::usage_recorded(&env, &recorded, units);
        Ok(())
    }

    /// Read the usage recorded for `metric` in the open period (read-only
    /// view).
    pub fn get_usage(
        env: Env,
        subscription_id: u64,
        metric: Symbol,
    ) -> Result<UsageRecord, ForgeError> {
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        Ok(
            Self::read_open_period_usage(&env, &subscription, &metric).unwrap_or(UsageRecord {
                subscription_id,
                metric,
                units: 0,
                period_start: subscription.last_charged,
            }),
        )
    }

    /// The amount the open period would bill right now (read-only view).
    pub fn quote_period(env: Env, subscription_id: u64) -> Result<i128, ForgeError> {
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        Ok(apply_credit(
            Self::derive_period_amount(&env, &subscription)?,
            Self::available_credit(&env, subscription_id),
        )
        .0)
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
        Self::store_subscription(&env, subscription_id, &subscription);
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

        let pending_trial = Self::pending_trial_end(&env, &subscription);
        let new_last_charged = subscription
            .last_charged
            .checked_add(elapsed)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        let new_trial_end = match pending_trial {
            Some(trial_end) => Some(
                trial_end
                    .checked_add(elapsed)
                    .ok_or(ForgeError::ArithmeticOverflow)?,
            ),
            None => None,
        };

        subscription.last_charged = new_last_charged;
        subscription.status = SubscriptionStatus::Active;
        subscription.paused_at = None;
        if let Some(updated_trial_end) = new_trial_end {
            Self::store_trial_end(&env, subscription_id, updated_trial_end);
        }
        Self::store_subscription(&env, subscription_id, &subscription);
        Ok(())
    }

    /// Cancel a subscription, preventing further charges.
    ///
    /// Requires the subscriber. Valid when `Active`, `Paused`, or `PastDue`. Cancelling
    /// an already-cancelled subscription is rejected.
    pub fn cancel(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status == SubscriptionStatus::Cancelled {
            return Err(ForgeError::InvalidInput);
        }
        subscription.subscriber.require_auth();
        Self::settle_cancellation(&env, subscription_id, subscription)
    }

    /// Cancel a subscription on the provider's side, refunding any unspent
    /// prepaid balance to the subscriber.
    ///
    /// Requires the provider. Valid when `Active`, `Paused`, or `PastDue`;
    /// cancelling an already-cancelled subscription is rejected. Settles
    /// exactly like the subscriber's `cancel`: the refund transfer runs
    /// before the cancellation is stored.
    pub fn cancel_by_provider(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        if subscription.status == SubscriptionStatus::Cancelled {
            return Err(ForgeError::InvalidInput);
        }
        subscription.provider.require_auth();
        Self::settle_cancellation(&env, subscription_id, subscription)
    }

    /// Read a stored subscription by id (read-only view).
    pub fn get_subscription(env: Env, subscription_id: u64) -> Result<Subscription, ForgeError> {
        Self::get_subscription_impl(&env, subscription_id)
    }

    /// Extend the persistent TTL of a subscription record without changing
    /// its business state. This is intended for permissionless keeper calls.
    pub fn touch_ttl(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        let key = DataKey::Subscription(subscription_id);
        if !env.storage().persistent().has(&key) {
            let subscription = Self::get_subscription_impl(&env, subscription_id)?;
            Self::store_subscription(&env, subscription_id, &subscription);
        } else {
            env.storage().persistent().extend_ttl(
                &key,
                soroban_forge_shared_utils::ttl::BUMP_THRESHOLD,
                soroban_forge_shared_utils::ttl::BUMP_AMOUNT,
            );
        }
        Self::renew_credit(&env, subscription_id);
        let subscription = Self::get_subscription_impl(&env, subscription_id)?;
        Self::renew_policies(&env, &subscription);
        let trial_key = DataKey::TrialEnd(subscription_id);
        if env.storage().persistent().has(&trial_key) {
            bump_entry(&env, &trial_key);
        }
        Ok(())
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

    /// Refund any positive prepaid balance to the subscriber, then store the
    /// subscription as `Cancelled` with `paused_at` cleared.
    ///
    /// Shared by `cancel` and `cancel_by_provider`; callers have already
    /// checked the status and authorized. The refund transfer runs before the
    /// state write, so a rejected transfer leaves the record unchanged.
    fn settle_cancellation(
        env: &Env,
        subscription_id: u64,
        mut subscription: Subscription,
    ) -> Result<(), ForgeError> {
        let mut refund = None;
        if let Some(balance) = subscription.prepaid_balance {
            if balance > 0 {
                let transfer_result = token::TokenClient::new(env, &subscription.token)
                    .try_transfer(
                        &env.current_contract_address(),
                        &subscription.subscriber,
                        &balance,
                    );
                if !matches!(transfer_result, Ok(Ok(()))) {
                    return Err(ForgeError::TokenTransferFailed);
                }
                subscription.prepaid_balance = Some(0);
                refund = Some(balance);
            }
        }

        env.storage()
            .persistent()
            .remove(&DataKey::GraceDeadline(subscription_id));
        env.storage()
            .persistent()
            .remove(&DataKey::PendingProvider(subscription_id));
        subscription.status = SubscriptionStatus::Cancelled;
        subscription.paused_at = None;
        Self::store_subscription(env, subscription_id, &subscription);
        if let Some(amount) = refund {
            events::balance_refunded(env, subscription_id, amount, 0);
        }
        events::cancelled(env, &subscription);
        Ok(())
    }

    /// Create a new subscription through the single shared creation path used
    /// by both `subscribe` and `subscribe_on_behalf_of`.
    ///
    /// Callers must have completed validation and authorization; this helper
    /// allocates the id, writes the record, and appends both indexes. Index
    /// writes join the success path after every fallible step (validation,
    /// `require_auth`, id allocation), so they cannot observe or create
    /// partial state. Records created through either entrypoint are
    /// indistinguishable from `get_subscription`'s perspective and draw
    /// from the same sequential counter.
    fn create_subscription(
        env: &Env,
        subscriber: Address,
        provider: Address,
        token: Address,
        amount: i128,
        period: u64,
    ) -> Result<u64, ForgeError> {
        let subscription_id = Self::next_id(env)?;
        let subscription = Subscription {
            subscription_id,
            subscriber: subscriber.clone(),
            provider: provider.clone(),
            token,
            amount,
            period,
            last_charged: env.ledger().timestamp(),
            status: SubscriptionStatus::Active,
            paused_at: None,
            failed_attempts: 0,
            past_due_since: None,
            // Flat by default: a subscription is metered only when the
            // subscriber declares quotas (today via `set_quotas`, and later
            // copied from the plan it joins).
            quotas: Vec::new(env),
            prepaid_balance: None,
        };
        Self::store_subscription(env, subscription_id, &subscription);
        Self::append_index(
            env,
            &DataKey::SubscriberSubscriptions(subscriber),
            subscription_id,
        );
        Self::append_index(
            env,
            &DataKey::ProviderSubscriptions(provider),
            subscription_id,
        );
        Ok(subscription_id)
    }

    /// Allocate the next monotonic subscription id.
    fn next_id(env: &Env) -> Result<u64, ForgeError> {
        let count: u64 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let id = count.checked_add(1).ok_or(ForgeError::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::Count, &id);
        Ok(id)
    }

    fn get_subscription_impl(env: &Env, subscription_id: u64) -> Result<Subscription, ForgeError> {
        let key = DataKey::Subscription(subscription_id);
        if let Some(raw) = env.storage().persistent().get::<DataKey, Val>(&key) {
            if let Ok(subscription) = Subscription::try_from_val(env, &raw) {
                return Ok(subscription);
            }
        }
        if let Some(raw) = env.storage().instance().get::<DataKey, Val>(&key) {
            if let Ok(subscription) = Subscription::try_from_val(env, &raw) {
                return Ok(subscription);
            }
            if let Ok(legacy) = SubscriptionV1::try_from_val(env, &raw) {
                return Ok(legacy.into());
            }
        }
        Err(ForgeError::NotFound)
    }

    /// Persist the current subscription shape and renew its TTL.
    fn store_subscription(env: &Env, subscription_id: u64, subscription: &Subscription) {
        let key = DataKey::Subscription(subscription_id);
        env.storage().persistent().set(&key, subscription);
        bump_entry(env, &key);
        Self::renew_credit(env, subscription_id);
        Self::renew_policies(env, subscription);
    }

    fn renew_credit(env: &Env, id: u64) {
        let key = DataKey::PlanCredit(id);
        if env.storage().persistent().has(&key) {
            bump_entry(env, &key);
        }
    }

    /// Append `subscription_id` to an index, carrying the address the index
    /// belongs to in the key.
    fn append_index(env: &Env, key: &DataKey, subscription_id: u64) {
        let mut ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(key)
            .or_else(|| env.storage().instance().get(key))
            .unwrap_or_else(|| Vec::new(env));
        ids.push_back(subscription_id);
        env.storage().persistent().set(key, &ids);
        bump_entry(env, key);
    }

    /// Read an index's id list, defaulting to empty when the address has no
    /// entries.
    fn index_ids(env: &Env, key: &DataKey) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(key)
            .or_else(|| env.storage().instance().get(key))
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

    /// Read this subscription's open-period amount: `amount` plus the overage
    /// for every declared metric.
    ///
    /// The single storage-aware entry point to the amount derivation. A flat
    /// subscription short-circuits to `amount` without touching the meters, so
    /// the no-quota path is bit-identical to the pre-metering contract.
    fn derive_period_amount(env: &Env, subscription: &Subscription) -> Result<i128, ForgeError> {
        if subscription.quotas.is_empty() {
            return Ok(subscription.amount);
        }
        let mut usage = Vec::new(env);
        for quota in subscription.quotas.iter() {
            if let Some(mut record) = Self::read_open_period_usage(env, subscription, &quota.metric)
            {
                record.units = record.units.saturating_sub(Self::opening_rollover(
                    env,
                    subscription,
                    &quota.metric,
                ));
                usage.push_back(record);
            }
        }
        period_amount(subscription.amount, &subscription.quotas, &usage)
    }

    /// Read the open period's meter for `metric`, or `None` when the metric is
    /// unused in the open period.
    ///
    /// A stored meter stamped for a different `last_charged` is ignored: the
    /// charge paths drop meters when they close a period, so a mismatched
    /// stamp can only come from a window moved by `resume`, whose paused
    /// window is not billable and therefore not metered. `record_usage`
    /// restarts the meter from the new open period.
    fn read_open_period_usage(
        env: &Env,
        subscription: &Subscription,
        metric: &Symbol,
    ) -> Option<UsageRecord> {
        let key = DataKey::Usage(subscription.subscription_id, metric.clone());
        let record: UsageRecord = env
            .storage()
            .persistent()
            .get(&key)
            .or_else(|| env.storage().instance().get(&key))?;
        if record.period_start == subscription.last_charged {
            Some(record)
        } else {
            None
        }
    }

    /// Whether the open period has any recorded usage, across the declared
    /// metrics. Every meter key belongs to a currently declared metric, so
    /// walking the quota list is exhaustive.
    fn has_recorded_usage(env: &Env, subscription: &Subscription) -> bool {
        subscription.quotas.iter().any(|quota| {
            Self::read_open_period_usage(env, subscription, &quota.metric)
                .is_some_and(|record| record.units > 0)
        })
    }

    /// Drop every open-period meter for `subscription`.
    ///
    /// Called only on the settlement success path, in the same frame that
    /// advances `last_charged`, which is what makes the period rollover atomic
    /// with the period close. A failed transfer returns before this point, so
    /// the meters survive and the retry charges the same amount again.
    fn rollover_usage(env: &Env, subscription: &Subscription) {
        for quota in subscription.quotas.iter() {
            let key = DataKey::Usage(subscription.subscription_id, quota.metric.clone());
            let cap_key = DataKey::RolloverCap(subscription.subscription_id, quota.metric.clone());
            if let Some(cap) = env.storage().persistent().get::<_, u64>(&cap_key) {
                let previous = subscription.last_charged - subscription.period;
                let usage = env
                    .storage()
                    .persistent()
                    .get::<_, UsageRecord>(&key)
                    .or_else(|| env.storage().instance().get(&key))
                    .filter(|record| record.period_start == previous)
                    .map_or(0, |record| record.units);
                let roll_key =
                    DataKey::RolloverUnits(subscription.subscription_id, quota.metric.clone());
                let old = env
                    .storage()
                    .persistent()
                    .get::<_, RolloverBalance>(&roll_key)
                    .filter(|record| record.period_start == previous)
                    .map_or(0, |record| record.units);
                let unused = quota
                    .included_units
                    .saturating_sub(usage.saturating_sub(old));
                let remaining_old = old.saturating_sub(usage);
                let units = unused.saturating_add(remaining_old).min(cap);
                env.storage().persistent().set(
                    &roll_key,
                    &RolloverBalance {
                        period_start: subscription.last_charged,
                        units,
                    },
                );
                bump_entry(env, &roll_key);
            }
            env.storage().persistent().remove(&key);
            env.storage().instance().remove(&key);
        }
    }
}

fn apply_credit(amount: i128, credit: i128) -> (i128, i128) {
    let used = amount.min(credit);
    (amount - used, credit - used)
}

// Consume fully credited periods before multiplying so an intermediate gross
// total cannot overflow when the net total is representable.
fn flat_periods_due(amount: i128, periods: u64, credit: i128) -> Result<(i128, i128), ForgeError> {
    let periods = i128::from(periods);
    let covered = (credit / amount).min(periods);
    let credit = credit - covered * amount;
    if covered == periods {
        return Ok((0, credit));
    }
    let tail = (periods - covered - 1)
        .checked_mul(amount)
        .ok_or(ForgeError::ArithmeticOverflow)?;
    Ok((
        tail.checked_add(amount - credit)
            .ok_or(ForgeError::ArithmeticOverflow)?,
        0,
    ))
}

/// Derive the amount billable for one period: the base `amount` plus the
/// overage of every declared metric, from the open period's raw unit counters.
///
/// This is the **only** place period amounts are computed — `charge`, each
/// settled period of `charge_catchup`, and `quote_period` all call it — so a
/// quote, a single charge, and a catch-up bill can never disagree. It is pure:
/// no storage, no ledger access, no authorization, and no rounding state
/// carried between calls, which is what keeps multi-period totals exact.
///
/// Per metric, with `over = max(0, units - included_units)`:
///
/// ```text
/// overage = ceil(min(over, max_overage_units) / bucket_units) * overage_price
/// ```
///
/// where an uncapped quota skips the `min`. Rounding is up: a started bucket
/// is billed in full. Overflow surfaces as [`ForgeError::ArithmeticOverflow`]
/// before any transfer runs, so an unrepresentable bill can never move funds.
fn period_amount(
    base: i128,
    quotas: &Vec<MetricQuota>,
    usage: &Vec<UsageRecord>,
) -> Result<i128, ForgeError> {
    let mut total = base;
    for quota in quotas.iter() {
        let overage = metric_overage(usage_units(usage, &quota.metric), &quota)?;
        total = total
            .checked_add(overage)
            .ok_or(ForgeError::ArithmeticOverflow)?;
    }
    Ok(total)
}

/// Overage owed for one metric's `units`: overage units past the inclusion,
/// clamped to the quota's cap, rounded up to whole buckets, priced per bucket.
fn metric_overage(units: u64, quota: &MetricQuota) -> Result<i128, ForgeError> {
    let over_units = units.saturating_sub(quota.included_units);
    let buckets = billable_buckets(over_units, quota.max_overage_units, quota.bucket_units);
    // `u64 -> i128` is lossless, so the multiply is the only step that can
    // overflow, and it is checked so an unrepresentable price is an error
    // rather than a wrapped (possibly negative) bill.
    let buckets = i128::from(buckets);
    buckets
        .checked_mul(quota.overage_price)
        .ok_or(ForgeError::ArithmeticOverflow)
}

/// Billable bucket count for `over_units` under a cap and bucket size.
///
/// The cap is applied to overage units **before** bucketing, so a cap bills at
/// most `ceil(cap / bucket_units)` buckets. A zero cap bills nothing and a zero
/// bucket size bills nothing, so no input can divide by zero or wrap.
fn billable_buckets(over_units: u64, max_overage_units: Option<u64>, bucket_units: u64) -> u64 {
    if bucket_units == 0 {
        // Unreachable through `set_quotas`, which rejects a zero bucket size;
        // guarded here so the helper is total and cannot divide by zero.
        return 0;
    }
    let capped = match max_overage_units {
        Some(cap) => over_units.min(cap),
        None => over_units,
    };
    capped.div_ceil(bucket_units)
}

/// Read a metric's recorded units out of a period's meter list, defaulting to
/// zero for a metric that was never used.
fn usage_units(usage: &Vec<UsageRecord>, metric: &Symbol) -> u64 {
    let mut at = 0;
    while at < usage.len() {
        let record = usage.get_unchecked(at);
        if &record.metric == metric {
            return record.units;
        }
        at += 1;
    }
    0
}

/// Validate a declared quota list: at most [`MAX_QUOTAS`] entries, no metric
/// declared twice (a duplicate would make the per-metric sum
/// order-dependent), a non-zero bucket size, and a non-negative overage price.
fn validate_quotas(quotas: &Vec<MetricQuota>) -> Result<(), ForgeError> {
    if quotas.len() > MAX_QUOTAS {
        return Err(ForgeError::InvalidInput);
    }
    let mut at = 0;
    while at < quotas.len() {
        let quota = quotas.get(at).ok_or(ForgeError::InvalidInput)?;
        if quota.bucket_units == 0 || quota.overage_price < 0 {
            return Err(ForgeError::InvalidInput);
        }
        let mut earlier = 0;
        while earlier < at {
            let previous = quotas.get(earlier).ok_or(ForgeError::InvalidInput)?;
            if previous.metric == quota.metric {
                return Err(ForgeError::InvalidInput);
            }
            earlier += 1;
        }
        at += 1;
    }
    Ok(())
}

/// Whether `subscription` prices `metric`, i.e. whether the provider is
/// allowed to meter it at all.
fn declares_metric(subscription: &Subscription, metric: &Symbol) -> bool {
    subscription
        .quotas
        .iter()
        .any(|quota| quota.metric == *metric)
}

/// Lifecycle events emitted by the subscription payments contract.
mod events {
    use super::*;

    #[contractevent]
    pub struct PlanChanged {
        #[topic]
        pub subscription_id: u64,
        pub old_amount: i128,
        pub new_amount: i128,
        pub credit: i128,
    }

    #[contractevent]
    pub struct Charged {
        #[topic]
        pub subscription_id: u64,
        /// Amount actually transferred for the settled period: the derived
        /// `base + overage` amount, not the base alone.
        pub amount: i128,
        pub last_charged: u64,
        pub next_charge_at: u64,
    }

    /// Usage metered for one metric in the open period. Indexers can rebuild
    /// every settled period's overage from these plus the plan's quotas.
    #[contractevent]
    pub struct UsageRecorded {
        #[topic]
        pub subscription_id: u64,
        #[topic]
        pub metric: Symbol,
        /// Units added by this call.
        pub units: u64,
        /// Running total for the open period, including this call.
        pub period_units: u64,
        /// The billing window these units belong to.
        pub period_start: u64,
    }

    /// The subscription's declared quota list was replaced.
    #[contractevent]
    pub struct QuotasSet {
        #[topic]
        pub subscription_id: u64,
        pub quotas: Vec<MetricQuota>,
    }

    #[contractevent]
    pub struct Cancelled {
        #[topic]
        pub subscription_id: u64,
        pub subscriber: Address,
    }

    #[contractevent]
    pub struct Deposited {
        #[topic]
        pub subscription_id: u64,
        pub amount: i128,
        pub balance_after: i128,
    }

    #[contractevent]
    pub struct BalanceDebited {
        #[topic]
        pub subscription_id: u64,
        pub amount: i128,
        pub balance_after: i128,
    }

    #[contractevent]
    pub struct BalanceRefunded {
        #[topic]
        pub subscription_id: u64,
        pub amount: i128,
        pub balance_after: i128,
    }

    /// A prepaid balance was topped up from the subscriber during `charge`.
    #[contractevent]
    pub struct AutoTopupTriggered {
        #[topic]
        pub subscription_id: u64,
        /// Amount pulled from the subscriber into custody.
        pub amount: i128,
        /// Prepaid balance after the top-up, before the period's debit.
        pub balance_after: i128,
    }

    pub fn auto_topup_triggered(
        env: &Env,
        subscription_id: u64,
        amount: i128,
        balance_after: i128,
    ) {
        AutoTopupTriggered {
            subscription_id,
            amount,
            balance_after,
        }
        .publish(env);
    }

    pub fn deposited(env: &Env, subscription_id: u64, amount: i128, balance_after: i128) {
        Deposited {
            subscription_id,
            amount,
            balance_after,
        }
        .publish(env);
    }

    pub fn balance_debited(env: &Env, subscription_id: u64, amount: i128, balance_after: i128) {
        BalanceDebited {
            subscription_id,
            amount,
            balance_after,
        }
        .publish(env);
    }

    pub fn balance_refunded(env: &Env, subscription_id: u64, amount: i128, balance_after: i128) {
        BalanceRefunded {
            subscription_id,
            amount,
            balance_after,
        }
        .publish(env);
    }

    pub fn charged(env: &Env, subscription: &Subscription, amount: i128) {
        let next_charge_at = subscription
            .last_charged
            .saturating_add(subscription.period);
        Charged {
            subscription_id: subscription.subscription_id,
            amount,
            last_charged: subscription.last_charged,
            next_charge_at,
        }
        .publish(env);
    }

    pub fn usage_recorded(env: &Env, record: &UsageRecord, units: u64) {
        UsageRecorded {
            subscription_id: record.subscription_id,
            metric: record.metric.clone(),
            units,
            period_units: record.units,
            period_start: record.period_start,
        }
        .publish(env);
    }

    pub fn quotas_set(env: &Env, subscription: &Subscription) {
        QuotasSet {
            subscription_id: subscription.subscription_id,
            quotas: subscription.quotas.clone(),
        }
        .publish(env);
    }

    /// A subscription with a free trial was created.
    #[contractevent]
    pub struct TrialStarted {
        #[topic]
        pub subscription_id: u64,
        /// Ledger timestamp at which the trial ends and the first charge is due.
        pub trial_end: u64,
    }

    /// A trial subscription converted into paid billing on its first successful
    /// post-trial charge.
    #[contractevent]
    pub struct TrialConverted {
        #[topic]
        pub subscription_id: u64,
        /// The trial end timestamp at which paid billing began.
        pub trial_end: u64,
    }

    pub fn cancelled(env: &Env, subscription: &Subscription) {
        Cancelled {
            subscription_id: subscription.subscription_id,
            subscriber: subscription.subscriber.clone(),
        }
        .publish(env);
    }

    pub fn trial_started(env: &Env, subscription_id: u64, trial_end: u64) {
        TrialStarted {
            subscription_id,
            trial_end,
        }
        .publish(env);
    }

    pub fn trial_converted(env: &Env, subscription_id: u64, trial_end: u64) {
        TrialConverted {
            subscription_id,
            trial_end,
        }
        .publish(env);
    }
}

#[cfg(test)]
mod authz;
#[cfg(test)]
mod auto_topup;
#[cfg(test)]
mod conservation;
#[cfg(test)]
mod lifecycle_props;
#[cfg(test)]
mod metering;
#[cfg(test)]
mod prepaid;
#[cfg(test)]
mod props;
#[cfg(test)]
mod settlement;
#[cfg(test)]
mod trial;

#[cfg(test)]
mod indexer_fixtures;

#[cfg(test)]
mod resource_report;

#[cfg(test)]
mod model_check;

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_forge_test_utils::{
        FailureBoundary, FailureSchedule, FailureToken, FailureTokenClient, TestAccounts,
    };
    use soroban_sdk::testutils::{storage::Persistent, Address as _, Events as _, Ledger as _};
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
    use soroban_sdk::{Address, Env};

    const START: u64 = 1_000_000;
    const PERIOD: u64 = 1_000;
    const AMOUNT: i128 = 250;

    /// Build a fresh env with mocked auths, a registered contract, a real SEP-41
    /// token with minted funds to subscriber, a subscription, and named accounts.
    macro_rules! setup {
        () => {{
            let env = Env::default();
            env.mock_all_auths_allowing_non_root_auth();
            env.ledger().set_timestamp(START);

            let admin = Address::generate(&env);
            let sac = env.register_stellar_asset_contract_v2(admin);
            let token = sac.address();
            let token_admin = StellarAssetClient::new(&env, &token);
            let token_client = TokenClient::new(&env, &token);

            let contract_id = env.register(SubscriptionPayments, ());
            let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);

            token_admin.mint(&accounts.user1, &10_000_i128);

            let subscription_id = client.subscribe(
                &accounts.user1,
                &accounts.validator,
                &token,
                &AMOUNT,
                &PERIOD,
            );
            (
                env,
                token,
                token_client,
                contract_id,
                client,
                accounts,
                subscription_id,
            )
        }};
    }

    #[test]
    fn subscribe_succeeds_and_is_active() {
        let (_env, _token, _tc, _contract_id, client, accounts, subscription_id) = setup!();
        let subscription = client.get_subscription(&subscription_id);
        assert_eq!(subscription.subscriber, accounts.user1);
        assert_eq!(subscription.provider, accounts.validator);
        assert_eq!(subscription.amount, AMOUNT);
        assert_eq!(subscription.status, SubscriptionStatus::Active);
        assert_eq!(subscription.last_charged, START);
        assert_eq!(subscription.paused_at, None);
        assert_eq!(subscription.failed_attempts, 0);
    }

    #[test]
    fn failure_token_seeded_catchup_failure_rolls_back_all_periods() {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_timestamp(START);
        let token = env.register(FailureToken, ());
        let token_client = FailureTokenClient::new(&env, &token);
        let contract_id = env.register(SubscriptionPayments, ());
        let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        token_client.mint(&accounts.user1, &10_000_i128);
        let subscription_id = client.subscribe(
            &accounts.user1,
            &accounts.validator,
            &token,
            &AMOUNT,
            &PERIOD,
        );
        let schedule = FailureSchedule::seeded(1, FailureBoundary::BeforeBatchStep);
        let step = schedule.failing_step().expect("seeded step");
        token_client.set_fail_at_transfer(&step);
        let before = client.get_subscription(&subscription_id);
        let subscriber_before = token_client.balance(&accounts.user1);
        let provider_before = token_client.balance(&accounts.validator);
        env.ledger().set_timestamp(START + PERIOD * 3);

        let err = client
            .try_charge_catchup(&subscription_id, &3)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::TokenTransferFailed);
        let after = client.get_subscription(&subscription_id);
        assert_eq!(after.last_charged, before.last_charged);
        assert_eq!(after.status, before.status);
        assert_eq!(after.failed_attempts, before.failed_attempts);
        assert_eq!(after.prepaid_balance, before.prepaid_balance);
        assert_eq!(token_client.balance(&accounts.user1), subscriber_before);
        assert_eq!(token_client.balance(&accounts.validator), provider_before);
        assert!(token_client.transfer_trace().is_empty());
        assert_eq!(
            schedule.attempt_trace(step),
            (1..=step).collect::<std::vec::Vec<u32>>()
        );
        assert!(env.events().all().events().is_empty());
    }

    #[test]
    fn subscribe_assigns_distinct_ids() {
        let (_env, token, _tc, _contract_id, client, accounts, subscription_id) = setup!();
        let id2 = client.subscribe(
            &accounts.user2,
            &accounts.validator,
            &token,
            &AMOUNT,
            &PERIOD,
        );
        assert_ne!(subscription_id, id2);
    }

    #[test]
    fn subscribe_on_behalf_of_creates_equivalent_active_subscription() {
        let (_env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        client.authorize_provider(&accounts.user1, &accounts.validator);
        let id = client.subscribe_on_behalf_of(
            &accounts.validator,
            &accounts.user1,
            &token,
            &AMOUNT,
            &PERIOD,
        );

        let sub = client.get_subscription(&id);
        assert_eq!(sub.subscriber, accounts.user1);
        assert_eq!(sub.provider, accounts.validator);
        assert_eq!(sub.amount, AMOUNT);
        assert_eq!(sub.status, SubscriptionStatus::Active);
        assert_eq!(sub.last_charged, client.env.ledger().timestamp());
        // Both creation paths feed the same indexes.
        assert_eq!(
            client
                .subscriptions_for_subscriber(&accounts.user1, &0, &10)
                .len(),
            2
        );
        assert_eq!(
            client
                .subscriptions_for_provider(&accounts.validator, &0, &10)
                .len(),
            2
        );
    }

    #[test]
    fn subscribe_on_behalf_of_requires_opt_in() {
        let (_env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        let err = client
            .try_subscribe_on_behalf_of(
                &accounts.validator,
                &accounts.user1,
                &token,
                &AMOUNT,
                &PERIOD,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::Unauthorized);

        // Revoking consent blocks later provider-initiated creations too.
        client.authorize_provider(&accounts.user1, &accounts.validator);
        client.revoke_provider(&accounts.user1, &accounts.validator);
        let err = client
            .try_subscribe_on_behalf_of(
                &accounts.validator,
                &accounts.user1,
                &token,
                &AMOUNT,
                &PERIOD,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::Unauthorized);
    }

    #[test]
    fn subscribe_on_behalf_of_rejects_invalid_amount_and_period() {
        let (_env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        client.authorize_provider(&accounts.user1, &accounts.validator);

        let err = client
            .try_subscribe_on_behalf_of(
                &accounts.validator,
                &accounts.user1,
                &token,
                &0_i128,
                &PERIOD,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);

        let err = client
            .try_subscribe_on_behalf_of(
                &accounts.validator,
                &accounts.user1,
                &token,
                &AMOUNT,
                &0_u64,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
        // No partial record from the failed calls.
        assert_eq!(client.get_subscription_count(), 1);
    }

    #[test]
    fn subscription_ids_are_sequential_across_both_paths() {
        let (_env, token, _tc, _contract_id, client, accounts, subscription_id) = setup!();
        assert_eq!(subscription_id, 1);

        client.authorize_provider(&accounts.user2, &accounts.validator);
        let id2 = client.subscribe_on_behalf_of(
            &accounts.validator,
            &accounts.user2,
            &token,
            &AMOUNT,
            &PERIOD,
        );
        assert_eq!(id2, 2);

        let id3 = client.subscribe(
            &accounts.user3,
            &accounts.arbiter,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        assert_eq!(id3, 3);

        client.authorize_provider(&accounts.user1, &accounts.arbiter);
        let id4 = client.subscribe_on_behalf_of(
            &accounts.arbiter,
            &accounts.user1,
            &accounts.deployer,
            &AMOUNT,
            &PERIOD,
        );
        assert_eq!(id4, 4);
        assert_eq!(client.get_subscription_count(), 4);
    }

    #[test]
    fn authorize_provider_is_idempotent() {
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
        client.authorize_provider(&accounts.user1, &accounts.validator);
        client.authorize_provider(&accounts.user1, &accounts.validator);
        assert!(client.is_provider_authorized(&accounts.user1, &accounts.validator));
    }

    #[test]
    fn revoke_provider_is_idempotent() {
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
        client.revoke_provider(&accounts.user1, &accounts.validator);
        assert!(!client.is_provider_authorized(&accounts.user1, &accounts.validator));
    }

    #[test]
    fn subscribe_rejects_zero_amount() {
        let (_env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        let err = client
            .try_subscribe(
                &accounts.user1,
                &accounts.validator,
                &token,
                &0_i128,
                &PERIOD,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn subscribe_rejects_zero_period() {
        let (_env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        let err = client
            .try_subscribe(
                &accounts.user1,
                &accounts.validator,
                &token,
                &AMOUNT,
                &0_u64,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn charge_before_period_returns_zero() {
        let (env, _token, tc, _contract_id, client, accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD - 1);
        assert_eq!(client.charge(&subscription_id), 0);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START
        );
        assert_eq!(tc.balance(&accounts.user1), 10_000);
        assert_eq!(tc.balance(&accounts.validator), 0);
    }

    #[test]
    fn charge_at_period_bills_full_amount_and_transfers_tokens() {
        let (env, _token, tc, _contract_id, client, accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD);
        assert_eq!(client.charge(&subscription_id), AMOUNT);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START + PERIOD
        );
        assert_eq!(tc.balance(&accounts.user1), 10_000 - AMOUNT);
        assert_eq!(tc.balance(&accounts.validator), AMOUNT);
    }

    #[test]
    fn charge_catches_up_one_period_per_call() {
        let (env, _token, tc, _contract_id, client, accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD * 3);
        // First call bills one period
        assert_eq!(client.charge(&subscription_id), AMOUNT);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START + PERIOD
        );
        assert_eq!(tc.balance(&accounts.validator), AMOUNT);

        // Second call bills second period
        assert_eq!(client.charge(&subscription_id), AMOUNT);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START + PERIOD * 2
        );
        assert_eq!(tc.balance(&accounts.validator), AMOUNT * 2);
    }

    #[test]
    fn charge_catchup_bills_elapsed_periods_and_respects_max_periods() {
        let (env, _token, tc, _contract_id, client, accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD * 3);

        assert_eq!(client.charge_catchup(&subscription_id, &2), AMOUNT * 2);
        let subscription = client.get_subscription(&subscription_id);
        assert_eq!(subscription.last_charged, START + PERIOD * 2);
        assert_eq!(tc.balance(&accounts.validator), AMOUNT * 2);

        assert_eq!(client.charge_catchup(&subscription_id, &2), AMOUNT);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START + PERIOD * 3
        );
        assert_eq!(tc.balance(&accounts.validator), AMOUNT * 3);
    }

    #[test]
    fn charge_catchup_zero_and_before_due_are_noops() {
        let (env, _token, tc, _contract_id, client, accounts, subscription_id) = setup!();
        assert_eq!(client.charge_catchup(&subscription_id, &0), 0);
        env.ledger().set_timestamp(START + PERIOD - 1);
        assert_eq!(client.charge_catchup(&subscription_id, &2), 0);
        assert_eq!(
            client.get_subscription(&subscription_id).last_charged,
            START
        );
        assert_eq!(tc.balance(&accounts.validator), 0);
    }

    #[test]
    fn charge_catchup_rejects_values_over_hard_cap() {
        let (_env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        let err = client
            .try_charge_catchup(&subscription_id, &33)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn charge_catchup_handles_u64_max_timestamp_without_overflow() {
        let (env, token, tc, _contract_id, client, accounts, _id) = setup!();
        let subscription_id = client.subscribe(
            &accounts.user2,
            &accounts.validator,
            &token,
            &1_i128,
            &u64::MAX,
        );
        StellarAssetClient::new(&env, &token).mint(&accounts.user2, &1_i128);
        env.ledger().set_timestamp(u64::MAX);

        assert_eq!(client.charge_catchup(&subscription_id, &1), 0);
        assert_eq!(tc.balance(&accounts.validator), 0);
    }

    #[test]
    fn charge_catchup_refuses_past_due_and_preserves_retry_state() {
        let (env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        let broke_user = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&broke_user, &10_i128);
        let sub_id = client.subscribe(&broke_user, &accounts.validator, &token, &AMOUNT, &PERIOD);
        env.ledger().set_timestamp(START + PERIOD);
        assert_eq!(client.charge(&sub_id), 0);

        let before = client.get_subscription(&sub_id);
        let err = client.try_charge_catchup(&sub_id, &2).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
        assert_eq!(
            client.try_pause(&sub_id).unwrap_err().unwrap(),
            ForgeError::InvalidInput
        );
        assert_eq!(
            client.try_resume(&sub_id).unwrap_err().unwrap(),
            ForgeError::InvalidInput
        );
        assert_eq!(client.get_subscription(&sub_id), before);
    }

    #[test]
    fn charge_catchup_transfer_failure_rolls_back_prior_periods() {
        let (env, token, tc, _contract_id, client, accounts, _id) = setup!();
        let broke_user = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&broke_user, &(AMOUNT + 1));
        let sub_id = client.subscribe(&broke_user, &accounts.validator, &token, &AMOUNT, &PERIOD);
        env.ledger().set_timestamp(START + PERIOD * 2);

        let err = client.try_charge_catchup(&sub_id, &2).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::TokenTransferFailed);
        assert_eq!(tc.balance(&broke_user), AMOUNT + 1);
        assert_eq!(tc.balance(&accounts.validator), 0);
        assert_eq!(client.get_subscription(&sub_id).last_charged, START);
    }

    #[test]
    fn charge_catchup_missing_subscription_is_not_found() {
        let (_env, _token, _tc, _contract_id, client, _accounts, _id) = setup!();
        let err = client.try_charge_catchup(&999, &1).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn charge_failure_transitions_to_past_due_and_retry_restores_active() {
        let (env, token, tc, _contract_id, client, accounts, _id) = setup!();
        let broke_user = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&broke_user, &10_i128);
        let sub_id = client.subscribe(&broke_user, &accounts.validator, &token, &AMOUNT, &PERIOD);

        env.ledger().set_timestamp(START + PERIOD);

        // First charge attempt fails due to insufficient balance in real SAC token
        let billed = client.charge(&sub_id);
        assert_eq!(billed, 0);

        let sub_past_due = client.get_subscription(&sub_id);
        assert_eq!(sub_past_due.status, SubscriptionStatus::PastDue);
        assert_eq!(sub_past_due.failed_attempts, 1);
        assert_eq!(sub_past_due.last_charged, START); // Not advanced!

        // Subscriber gets funds minted now
        StellarAssetClient::new(&env, &token).mint(&broke_user, &10_000_i128);

        // Retry charge succeeds!
        let billed = client.charge(&sub_id);
        assert_eq!(billed, AMOUNT);

        let sub_active = client.get_subscription(&sub_id);
        assert_eq!(sub_active.status, SubscriptionStatus::Active);
        assert_eq!(sub_active.failed_attempts, 0);
        assert_eq!(sub_active.last_charged, START + PERIOD);
        assert_eq!(tc.balance(&accounts.validator), AMOUNT);
    }

    #[test]
    fn max_retries_exceeded_transitions_to_cancelled() {
        let (env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        let broke_user = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&broke_user, &10_i128);
        let sub_id = client.subscribe(&broke_user, &accounts.validator, &token, &AMOUNT, &PERIOD);

        env.ledger().set_timestamp(START + PERIOD);

        // Attempt 1 -> PastDue (failed_attempts = 1)
        assert_eq!(client.charge(&sub_id), 0);
        assert_eq!(
            client.get_subscription(&sub_id).status,
            SubscriptionStatus::PastDue
        );

        // Attempt 2 -> PastDue (failed_attempts = 2)
        assert_eq!(client.charge(&sub_id), 0);
        assert_eq!(
            client.get_subscription(&sub_id).status,
            SubscriptionStatus::PastDue
        );

        // Attempt 3 -> Cancelled (failed_attempts = 3)
        assert_eq!(client.charge(&sub_id), 0);
        assert_eq!(
            client.get_subscription(&sub_id).status,
            SubscriptionStatus::Cancelled
        );

        // Attempt 4 -> InvalidInput because status is now Cancelled
        assert_eq!(
            client.try_charge(&sub_id).unwrap_err().unwrap(),
            ForgeError::InvalidInput
        );
    }

    #[test]
    fn cancel_from_past_due_succeeds() {
        let (env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        let broke_user = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&broke_user, &10_i128);
        let sub_id = client.subscribe(&broke_user, &accounts.validator, &token, &AMOUNT, &PERIOD);

        env.ledger().set_timestamp(START + PERIOD);
        let _ = client.charge(&sub_id);
        assert_eq!(
            client.get_subscription(&sub_id).status,
            SubscriptionStatus::PastDue
        );

        // Cancel from PastDue
        client.cancel(&sub_id);
        assert_eq!(
            client.get_subscription(&sub_id).status,
            SubscriptionStatus::Cancelled
        );
    }

    #[test]
    fn charge_missing_subscription_is_not_found() {
        let (_env, _token, _tc, _contract_id, client, _accounts, _id) = setup!();
        let err = client.try_charge(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn cancel_prevents_further_charges() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
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
        let (_env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        client.cancel(&subscription_id);
        let err = client.try_cancel(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_missing_subscription_is_not_found() {
        let (_env, _token, _tc, _contract_id, client, _accounts, _id) = setup!();
        let err = client.try_cancel(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_subscription_missing_is_not_found() {
        let (_env, _token, _tc, _contract_id, client, _accounts, _id) = setup!();
        let err = client.try_get_subscription(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_subscription_count_tracks_creations() {
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
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
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
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
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
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
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
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
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
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
    fn pause_active_subscription_succeeds() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + 250);
        client.pause(&subscription_id);

        let sub = client.get_subscription(&subscription_id);
        assert_eq!(sub.status, SubscriptionStatus::Paused);
        assert_eq!(sub.paused_at, Some(START + 250));
    }

    #[test]
    fn pause_already_paused_is_invalid() {
        let (_env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        client.pause(&subscription_id);
        let err = client.try_pause(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn pause_cancelled_is_invalid() {
        let (_env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        client.cancel(&subscription_id);
        let err = client.try_pause(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn pause_missing_is_not_found() {
        let (_env, _token, _tc, _contract_id, client, _accounts, _id) = setup!();
        let err = client.try_pause(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn charge_paused_subscription_fails_and_does_not_advance() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
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
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
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
    fn out_of_bounds_offset_returns_empty() {
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
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
        let (_env, _token, _tc, _contract_id, client, accounts, _id) = setup!();
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

    #[test]
    fn pause_resume_zero_length_no_drift() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
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
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
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
        let (_env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        let err = client.try_resume(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn resume_cancelled_subscription_is_invalid() {
        let (_env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        client.cancel(&subscription_id);
        let err = client.try_resume(&subscription_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn resume_missing_is_not_found() {
        let (_env, _token, _tc, _contract_id, client, _accounts, _id) = setup!();
        let err = client.try_resume(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn cancel_paused_subscription_succeeds_and_cannot_resume() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
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
    #[test]
    fn events_emitted_on_lifecycle_actions() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD);
        client.charge(&subscription_id);
        client.cancel(&subscription_id);

        let all_events = env.events().all();
        assert!(!all_events.events().is_empty());
    }

    #[test]
    fn charge_catchup_emits_charged_event_per_settled_period() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD * 3);

        let total = client.charge_catchup(&subscription_id, &3);
        assert_eq!(total, AMOUNT * 3);

        let events = env.events().all();
        assert!(
            !events.events().is_empty(),
            "Events should be emitted during catchup"
        );
    }

    #[test]
    fn charge_catchup_periods_zero_emits_no_events() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();

        let total = client.charge_catchup(&subscription_id, &0);
        assert_eq!(total, 0);

        let events = env.events().all();
        assert!(
            events.events().is_empty(),
            "No new events should be emitted when charging 0 periods"
        );
    }

    #[test]
    fn charge_catchup_failed_transfer_rolls_back_events() {
        let (env, token, _tc, _contract_id, client, accounts, _id) = setup!();
        let broke_user = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&broke_user, &AMOUNT);

        let sub_id = client.subscribe(&broke_user, &accounts.validator, &token, &AMOUNT, &PERIOD);
        env.ledger().set_timestamp(START + PERIOD * 3);

        let res = client.try_charge_catchup(&sub_id, &3);
        assert!(res.is_err());

        let events = env.events().all();
        assert!(
            events.events().is_empty(),
            "Failed transfer should roll back emitted events"
        );
    }

    #[test]
    fn charge_catchup_advances_last_charged_and_event_payloads() {
        let (env, _token, _tc, _contract_id, client, _accounts, subscription_id) = setup!();
        env.ledger().set_timestamp(START + PERIOD * 2);

        client.charge_catchup(&subscription_id, &2);

        let sub = client.get_subscription(&subscription_id);
        assert_eq!(sub.last_charged, START + PERIOD * 2);
    }

    #[test]
    fn legacy_instance_subscription_is_read_and_upgraded_on_write() {
        let (env, token, _tc, contract_id, client, accounts, id) = setup!();
        let current = client.get_subscription(&id);
        let legacy = SubscriptionV1 {
            subscription_id: current.subscription_id,
            subscriber: current.subscriber,
            provider: current.provider,
            token: current.token,
            amount: current.amount,
            period: current.period,
            last_charged: current.last_charged,
            status: current.status,
            paused_at: current.paused_at,
            failed_attempts: current.failed_attempts,
            past_due_since: current.past_due_since,
            quotas: current.quotas,
        };

        env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .remove(&DataKey::Subscription(id));
            env.storage()
                .instance()
                .set(&DataKey::Subscription(id), &legacy);
        });

        let decoded = client.get_subscription(&id);
        assert_eq!(decoded.subscription_id, id);
        assert_eq!(decoded.subscriber, accounts.user1);
        assert_eq!(decoded.provider, accounts.validator);
        assert_eq!(decoded.token, token);
        assert_eq!(decoded.prepaid_balance, None);

        client.pause(&id);
        env.as_contract(&contract_id, || {
            assert!(env.storage().persistent().has(&DataKey::Subscription(id)));
            assert_eq!(
                env.storage()
                    .persistent()
                    .get::<_, Subscription>(&DataKey::Subscription(id))
                    .unwrap()
                    .status,
                SubscriptionStatus::Paused
            );
        });
    }

    #[test]
    fn touch_ttl_extends_persistent_subscription_without_changing_state() {
        let (env, _token, _tc, contract_id, client, _accounts, id) = setup!();
        let before = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get_ttl(&DataKey::Subscription(id))
        });

        client.touch_ttl(&id);

        let after = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get_ttl(&DataKey::Subscription(id))
        });
        assert!(after >= before);
        assert_eq!(client.get_subscription(&id).subscription_id, id);
    }
}

#[cfg(test)]
mod policy_tests;
