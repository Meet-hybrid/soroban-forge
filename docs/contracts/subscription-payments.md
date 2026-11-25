# Subscription Payments Contract

Recurring, on-chain subscription billing: a subscriber authorizes a provider to
pull a fixed `amount` per `period` (seconds) through SEP-41 token transfers.

## Interface

```rust
fn subscribe(subscriber, provider, token, amount, period) -> Result<u64, ForgeError>
fn subscribe_on_behalf_of(provider, subscriber, token, amount, period) -> Result<u64, ForgeError>
fn subscribe_with_trial(subscriber, provider, token, amount, period, trial_seconds: u64) -> Result<u64, ForgeError>
fn get_trial_end(subscription_id) -> Option<u64>
fn authorize_provider(subscriber, provider) -> Result<(), ForgeError>
fn revoke_provider(subscriber, provider) -> Result<(), ForgeError>
fn is_provider_authorized(subscriber, provider) -> bool
fn charge(subscription_id) -> Result<i128, ForgeError>
fn deposit(subscription_id, amount) -> Result<i128, ForgeError>
fn withdraw_balance(subscription_id, amount) -> Result<i128, ForgeError>
fn configure_auto_topup(subscription_id, trigger_threshold: i128, topup_amount: i128) -> Result<(), ForgeError>
fn disable_auto_topup(subscription_id) -> Result<(), ForgeError>
fn get_auto_topup(subscription_id) -> Option<AutoTopup>
fn charge_catchup(subscription_id, max_periods: u32) -> Result<i128, ForgeError>
fn pause(subscription_id) -> Result<(), ForgeError>
fn resume(subscription_id) -> Result<(), ForgeError>
fn set_quotas(subscription_id, quotas: Vec<MetricQuota>) -> Result<(), ForgeError>
fn record_usage(subscription_id, metric: Symbol, units: u64) -> Result<(), ForgeError>
fn quote_period(subscription_id) -> Result<i128, ForgeError>
fn cancel(subscription_id) -> Result<(), ForgeError>
fn get_subscription(subscription_id) -> Result<Subscription, ForgeError>
fn get_usage(subscription_id, metric: Symbol) -> Result<UsageRecord, ForgeError>
fn get_subscription_count() -> u64
fn subscriptions_for_subscriber(subscriber, offset, limit) -> Result<Vec<Subscription>, ForgeError>
fn subscriptions_for_provider(provider, offset, limit) -> Result<Vec<Subscription>, ForgeError>
// Plan registry (issue #116)
fn create_plan(provider, token, amount, period, quotas: Vec<MetricQuota>) -> Result<u64, ForgeError>
fn subscribe_to_plan(plan_id, subscriber) -> Result<u64, ForgeError>
fn get_plan(plan_id) -> Result<Plan, ForgeError>
fn plan_count() -> u64
```

- `subscribe` requires the subscriber and returns a stable, monotonic id.
- `charge` requires the provider and settles at most one full period per call.
- `deposit` requires the subscriber. The first successful deposit opts that
  subscription into prepaid mode; deposits pull the exact amount into contract
  custody before writing the new balance. `get_subscription` exposes
  `prepaid_balance`: `None` is pull mode, while `Some(0)` remains opted in.
- In prepaid mode, `charge` still requires the provider and transfers exactly
  one derived period amount from contract custody to the provider. If the
  balance cannot cover it, no partial transfer occurs and the existing retry
  policy moves the subscription to `PastDue` (and auto-cancels after three
  failed attempts). Depositing while `PastDue` allows the next `charg` retry
  failed attempts). Depositing while `PastDue` allows the next `charge` retry
  to recover it. `charge_catchup` rejects prepaid subscriptions so it cannot
  bypass this one-period lapse policy by pulling from the subscriber.
- `withdraw_balance` requires the subscriber and returns a requested amount
  before cancellation. It does not turn prepaid mode off. `cancel` returns the
  exact remaining balance to the subscriber before recording cancellation;
  retry-limit auto-cancellation also returns the remainder.
- `charge_catchup` requires the provider and settles `min(max_periods,
elapsed_periods)` periods in one atomic invocation. `max_periods == 0` is a
  no-op; values above the hard cap of 32 are rejected. The cap bounds Soroban
  instruction use and token outlay for keeper calls.
- `charge_catchup` refuses `PastDue`; call `charge` to use the existing retry
  policy. A failed catch-up transfer returns `TokenTransferFailed` and rolls
  back all transfers and `last_charged` through Soroban frame rollback.
- `pause` requires the subscriber and is valid only for `Active` subscriptions;
  it prevents `charge` and `charge_catchup` while paused.
- `resume` requires the subscriber and is valid only for `Paused` subscriptions;
  it advances `last_charged` by the paused duration so paused time is not billed.
- `cancel` requires the subscriber and prevents further charges.
- `set_quotas`, `record_usage`, and the metering views are described under
  [Metered usage](#metered-usage-quotas-and-overage).

## Free Trials

`subscribe_with_trial(subscriber, provider, token, amount, period, trial_seconds)`
requires the subscriber and creates a subscription whose **first charge is due
at `trial_end = now + trial_seconds`**. The trial end timestamp is stored under
its own persistent `TrialEnd(subscription_id)` key so the serialized
`Subscription` record layout remains unchanged.

### Parameters and cap

- `trial_seconds` must be strictly positive (`> 0`) and at most
  `TRIAL_MAX_SECONDS` (`7_776_000` seconds / 90 days).
- Values of `0` or above `TRIAL_MAX_SECONDS`, or non-positive `amount` /
  `period`, fail with `ForgeError::InvalidInput` before authorization.
- If `now + trial_seconds` overflows `u64`, `ForgeError::ArithmeticOverflow` is
  returned before any subscription state is written.

### Lifecycle rules

| Situation | Behaviour |
|---|---|
| `charge` or `charge_catchup` while `now < trial_end` | Returns `0`; no tokens move, no state changes, no events emitted. |
| First successful `charge` at `now >= trial_end` | Bills one period (pull or prepaid), sets `last_charged = trial_end`, and emits `TrialConverted` and `Charged`. Subsequent charges follow the normal cadence (`trial_end + period`, `trial_end + 2 * period`, …) without retroactively billing trial time. |
| Failed first `charge` at `now >= trial_end` | Follows the existing arrears retry path (`PastDue` / `Cancelled`) without advancing `last_charged` or emitting `TrialConverted`; retrying once funded converts the trial and bills the period due at `trial_end`. |
| `charge_catchup` at `now >= trial_end` | Bills `min(max_periods, 1 + (now - trial_end) / period)` periods starting from `trial_end`, emitting `TrialConverted` once and `Charged` per settled period. Trial time is never retroactively billed. `max_periods == 0` is a no-op and does not convert the trial. |
| `cancel` / `cancel_by_provider` during the trial | Costs nothing and refunds any prepaid balance in full before recording `Cancelled`. |
| `pause` during the trial, then `resume` | Extends `trial_end` (and `last_charged`) by the elapsed paused duration, matching how `resume` shifts due dates on regular subscriptions. Pausing after trial conversion shifts `last_charged` only and leaves the historical `trial_end` unchanged. |

`get_trial_end(subscription_id)` is a read-only view returning `Some(trial_end)`
for trial subscriptions (reflecting any pause extensions made during the trial)
and `None` for subscriptions created without a trial.

## Metered usage: quotas and overage

A subscription can declare up to 16 per-metric quotas, so a period bills
`amount` plus overage for real consumption. **No quotas means the original flat
flow**: the charged amount and every event payload are identical to a contract
without this feature.

```rust
struct MetricQuota {
    metric: Symbol,             // e.g. api_calls
    included_units: u64,         // covered by the base `amount`
    overage_price: i128,        // per started bucket
    bucket_units: u64,             // units per bucket, must be > 0
    max_overage_units: Option<u64>, // cap on billable overage units
}
```

The billable amount for one period is derived in a single pure helper used by
`charg`, by every period of `charge_catchup`, and by `quote_period`:

```text
amount = base
       + Σ over metric of ceil(min(max(0, units - included), cap) / bucket) * overage_price
```

with the `min` skipped for an uncapped quota.

- `set_quotas` requires the **subscriber** — a quota prices what the subscriber
  is billed, so the charged account consents to the terms. It is rejected unless
  the subscription is `Active` and the open period has no recorded usage, so
  already-metered units can never be repriced mid-period. An empty list returns
  the subscription to flat pricing.
- `record_usage` requires the **provider**, mirroring who may `charge`, and
  rejects a metric the subscription does not declare, so usage the subscriber
  has not been asked to pay for can never be billed.
- `record_usage` accumulates **raw units** for the open period only, so each
  period's amount is a pure function of that period's units: charging `N
  periods bills exactly `N` times the per-period derivation, and no rounding can
  compound across periods. A bucket is billed once started (rounded up), and the
  price is per bucket, never per unit.
- Meters are dropped atomically with the charge that closes the period, in
  the same frame that advances `last_charged`. A failed transfer returns before that
  point, so every meter survives and the arrears retry re-derives an identical
  amount.
- `charge_catchup` settles the open period **last**: usage can only be metered
  into the open (latest) period, so earlier unsettled periods bill the base
  amount and only the final one carries overage.
- `record_usage` requires `Active`, which freezes the meter at the attempted
  amount once a charge has failed, so the `PastDue` retry is identical to the
  attempt that failed.
- Caps are enforced by clamping at settlement, never by rejecting, so a usage
  spike cannot wedge a subscription; the subscriber's worst case per period is
  `base + Σ caps`. A `u64` unit-counter overflow surfaces as
  `ArithmeticOverflow` rather than wrapping into a silent underbill.

## Subscription States

- `Active` — chargeable
- `PastDue` — the open period has elapsed without a successful charge; chargeable
  through the catch-up path
- `Paused` — temporarily not chargeable; `charge` and `charge_catchup` are
  refused, while the subscriber may `resume` or `cancel`
- `Cancelled` — no further charges

The contract maintains an explicit lapsed state so integrators can build
dunning (reminders, retries, suspension) on top of the contract. The lifecycle is
<code>Active → PastDue → Active</code>, with the following transition table:

| From | Event | To | Notes |
|------|-------|----|-------|
| `Active` | Open period ends without a successful charge (time-derived) | `PastDue` | Entered as soon as the due timestamp is observable; no call required to observe it via the view |
| `Active` | Successful `charge` within the grace window | `Active` | Due timestamp advances exactly one period |
| `PastDue` | Successful `charg` (catch-up) | `Active` | Bills the overdue period with identical amount math; due timestamp advances exactly one period |
| `PastDue` | Passage of time alone | `PastDue` | Never silently reverts to `Active` |
| `PastDue` | `cancel` | `Cancelled` | Refunds any prepaid remainder exactly as from `Active` |
| `PastDue` | `pause` | `Paused` | Lapse bookkeeping is preserved and resumes with the subscription |
| `Paused` | `resume` while the open period has elapsed | `PastDue` | The lapse is re-derived from the due timestamp on resume |
| `PastDue` | Successful catch-up charge while paused | `Paused` | Catch-up advances the due timestamp but the status remains `Paused` until resume |

- `PastDue` means the open period has elapsed without a successful charge. It is
  reachable through the public interface: the trigger is time-derived, so a
  subscription whose due timestamp has elapsed is reported as `PastDue` by
  `get_subscription` and the `is_past_due` view without anyone calling
  `charg`.
- A successful catch-up `charg` on a `PastDue` subscription bills the
  overdue period with identical amount math, restores `Active`, and advances
  the due timestamp exactly one period — the one-period-per-call invariant is
  preserved.
- `PastDue` is recoverable only through a successful charge; it never
  silently reverts to `Active` by the passage of time alone.
- The grace window is a contract constant. There is no admin or principal in
  this contract, so the window is not configurable at runtime. A charge that
  lands after the due timestamp but within the grace window still bills the
  normal one-period amount and keeps the subscription `Active`.
- Charging a `Cancelled` subscription behaves exactly as today: it fails with
  `ForgeError::NotActive`. Double-charging remains impossible.
- Auto-cancellation after N lapsed periods is out of scope for this state;
  the lapse bookkeeping is exposed so a future follow-up can build on it.

The implemented transitions are `Active -> Paused -> Active` for pause/resume.
Cancellation is allowed from `Active`, `Paused`, and `PastDue`; `Cancelled` is
terminal. Pausing records the ledger timestamp, and resuming shifts the next
charge window by the elapsed pause duration.

## Prepaid money flow

Only a successful `deposit` opts in. Subscriptions that never deposit retain
the existing pull flow and `prepaid_balance == None`.

| State / operation | Token movement | Balance effect |
|---|---|---|
| Pull mode `charge` | subscriber → provider, exact derived period amount | no prepaid balance |
| Prepaid `deposit` | subscriber → contract, exact requested amount | add amount after transfer succeeds |
| Prepaid `charge`, sufficient balance | contract → provider, exact derived period amount | subtract amount after transfer succeeds |
| Prepaid `charge`, insufficient balance | none | unchanged; retry state advances to `PastDue` or retry-limit cancellation |
| `withdraw_balance` | contract → subscriber, exact requested amount | subtract amount after transfer succeeds |
| `cancel` / retry-limit auto-cancel | contract → subscriber, full remaining balance | set to `Some(0)` after refund succeeds |

The conservation invariant is `Σdeposits ‒ Σperiod debits ‒ Σwithdrawals ‒
Σcancellation refunds == prepaid_balance` after every successful operation.
The randomized lifecycle property test compares each operation with an
independent balance/state mirror and also checks the SEP-41 contract balance.
Every successful deposit, period debit, and refund emits `Deposited`,
`BalanceDebited`, or `BalanceRefunded` with `amount` and `balance_after`.

Prepaid mode introduces a custody trust surface: deposited tokens stay in the
contract until periods are charged or the subscriber withdraws/cancels. The
pull mode remains direct subscriber-to-provider settlement. The subscription
record gains an optional `prepaid_balance` field; ABI clients must be regenerated
for this record shape change.

## Prepaid money flow

Only a successful `deposit` opts in. Subscriptions that never deposit retain
the existing pull flow and `prepaid_balance == None`.

| State / operation | Token movement | Balance effect |
|---|---|---|
| Pull mode `charge` | subscriber → provider, exact derived period amount | no prepaid balance |
| Prepaid `deposit` | subscriber → contract, exact requested amount | add amount after transfer succeeds |
| Prepaid `charge`, sufficient balance | contract → provider, exact derived period amount | subtract amount after transfer succeeds |
| Prepaid `charge`, insufficient balance | none | unchanged; retry state advances to `PastDue` or retry-limit cancellation |
| `withdraw_balance` | contract → subscriber, exact requested amount | subtract amount after transfer succeeds |
| `cancel` / retry-limit auto-cancel | contract → subscriber, full remaining balance | set to `Some(0)` after refund succeeds |

The conservation invariant is `Σdeposits − Σperiod debits ‒ Σwithdrawals ‒
Σcancellation refunds == prepaid_balance` after every successful operation.
The randomized lifecycle property test compares each operation with an
independent balance/state mirror and also checks the SEP-41 contract balance.
Every successful deposit, period debit, and refund emits `Deposited`,
`BalanceDebited`, or `BalanceRefunded` with `amount` and `balance_after`.

Prepaid mode introduces a custody trust surface: deposited tokens stay in the
contract until periods are charged or the subscriber withdraws/cancels. The
pull mode remains direct subscriber-to-provider settlement. The subscription
record gains an optional `prepaid_balance` field; ABI clients must be regenerated
for this record shape change.

## Prepaid auto-topup

`configure_auto_topup(subscription_id, trigger_threshold, topup_amount)`
requires the subscriber and authorizes the contract to pull `topup_amount`
from the subscriber into custody during billing. It validates
`topup_amount > 0` and `trigger_threshold >= 0`, rejects a `Cancelled`
subscription, and replaces any earlier setting. `disable_auto_topup` (also
subscriber-only, idempotent) removes it; `get_auto_topup` reads it.

| Due prepaid `charge` | Token movement | Result |
|---|---|---|
| `prepaid_balance < period_amount` or `< trigger_threshold`, top-up succeeds | subscriber → contract `topup_amount`, then contract → provider period amount | `AutoTopupTriggered { amount, balance_after }`, then the normal debit |
| Topped-up balance still below the period amount | subscriber → contract `topup_amount` only | credited; usual `PastDue` / retry-limit cancellation (which refunds the whole balance) |
| Top-up rejected (insufficient funds, deauthorized account) | none | nothing written by the top-up; debit if the old balance still covers the period, otherwise `PastDue` exactly as without auto-topup |

At most one top-up runs per charge, and it runs before the debit transfer
and before the subscription is stored (transfer before state). Pull mode and
`charge_catchup` (which refuses prepaid subscriptions) never top up. With
auto-topup, prepaid conservation reads `Σdeposits + Σtop-ups − Σperiod
debits − Σwithdrawals − Σcancellation refunds == prepaid_balance`.

## Secondary Indices

Each subscription is indexed in two secondary lists, written on the `subscribe`Jsuccess path:

-  `SubscriberSubscriptions(subscriber)` — creation-order subscription ids for
  which the address is the subscriber.
-  `ProviderSubscriptions(provider)` — creation-order subscription ids for which
  the address is the provider.

The indices store lightweight `Vec<u64>` id lists; full `Subscription` records
are only loaded for the requested page slice.

## View Methods

-  `get_subscription_count()` returns the total number of subscriptions created
  (the monotonic id counter; cancellation never lowers it).
-  `subscriptions_for_subscriber(subscriber, offset, limit)` returns a page of
  `Subscription` records for the subscriber, in creation order.
-  `subscriptions_for_provider(provider, offset, limit)` returns a page of
  `Subscription` records for the provider, in creation order.
-  `is_past_due(subscription_id)` returns whether the subscription is currently
  lapsed. It is a pure read of the due timestamp and the grace window, aligned
  with the next-due / over-due queries of issue #71.

All enumeration methods are read-only (no authorization, no storage writes).
`offset` skips the first `offset` subscriptions and `limit` caps the page size:
an empty index or an out-of-bounds offset yields an empty `Vec`, and a `limit`
of `0` fails with `ForgeError::InvalidInput`.

## Storage

- `Subscription(id)` — a persistent record for id `u64`. Persistent entries
  are TTL-managed and are refreshed on writes; `touch_ttl(id)` can refresh an
  existing record without changing its lifecycle state.
- `Count` — monotonic subscription id counter.
- `SubscriberSubscriptions(address)` and `ProviderSubscriptions(address)` —
  persistent, creation-ordered id indexes used by the paginated enumeration
  methods. New writes refresh their TTLs.
- `RenewalPolicy(id)` — separate instance-storage record; the `Subscription`
  wire shape remains unchanged.
- `AutoTopup(subscription_id)` — persistent `AutoTopup { trigger_threshold,
  topup_amount }` record; a separate key, so the `Subscription` wire shape is
  unchanged.
- `TrialEnd(subscription_id)` — persistent `u64` trial end timestamp written by
  `subscribe_with_trial` and extended by `resume` when paused during a trial;
  stored separately so the `Subscription` wire shape is unchanged, and refreshed
  by `touch_ttl`.
- `Usage(subscription_id, metric)` — a persistent open-period raw unit counter
  for one metric, stamped with the `last_charged` window it belongs to; removed
  by the charge that closes the period.

### Storage migration and TTL compatibility

Older deployments may contain subscription records and indexes in instance
storage. Reads first use the persistent shape and then fall back to the legacy
instance entry, including the pre-`prepaid_balance` record shape. A subsequent
lifecycle write upgrades that record to the current persistent shape; the
contract does not silently create a default subscription when an entry is
missing. Index reads also retain the legacy instance fallback while new index
writes use persistent storage. This makes the migration incremental and keeps
pagination deterministic.

`touch_ttl` is an explicit maintenance operation for integrations that want to
refresh a subscription record ahead of its normal write path. It only extends
the persistent record's TTL and does not charge, pause, resume, cancel, or
otherwise change subscription state.

## Payment Semantics

`charge` returns the billed amount, or `0` when no full period has elapsed since
the last charge. `charge_catchup` returns the total billed amount and advances
`last_charged` once per successfully settled period. Period and total arithmetic
uses checked operations. For a metered subscription the billed amount is the
derived `base + overage` amount rather than `amount` alone; it is computed
before the transfer, so an unrepresentable bill never moves funds.

When a subscription is `PastDue`, a charge bills the overdue period with the
identical amount math as a normal charge and restores `Active`. The due
timestamp advances exactly one period, so the one-period-per-call invariant holds
even when catching up. Timestamp arithmetic on due and catch-up math uses
`checked_add` and surfaces `ArithmeticOverflow` at the `u64` boundaries.

## Events

The contract emits typed on-chain lifecycle events for indexers and off-chain monitoring:

- `Subscribed` (topic: `subscription_id: u64`) — emitted when a subscription is created via `subscribe`. Contains `subscriber`, `provider`, `token`, `amount`, and `period`.
- `Charged` (topic: `subscription_id: u64`) — emitted on successful billing via `charge` (one event) or `charge_catchup` (one event per settled period). Contains `amount`, `last_charged`, and `next_charge_at`. For a metered subscription `amount` is the derived `base + overage` amount, not the base alone.
- `PastDueEntered` (topic: `subscription_id: u64`) — emitted once when a
  subscription becomes lapsed. Contains the `due_since` timestamp and the
  `missed_periods` counter, so an indexer can drive dunning without reading
  storage. It is never emitted twice for the same lapse.
- `PastDueRecovered` (topic: `subscription_id: u64`) — emitted when a
  successful catch-up `charg` restores a `PastDue` subscription to `Active`.
  Contains the catch-up `amount` and the new `last_charged` / `next_charge_at`.
- `UsageRecorded` (topics: `subscription_id: u64`, `metric: Symbol`) — emitted per `record_usage` call. Contains the `units` added, the running `period_units` for the open period, and the `period_start` window those units belong to, so an indexer can rebuild each settled period's overage.
- `QuotasSet` (topic: `subscription_id: u64`) — emitted when `set_quotas` replaces the declared terms. Contains the full `quotas` list, so the pricing an indexer needs travels with the event.
- `Deposited` (topic: `subscription_id: u64`) — successful prepaid deposit; contains `amount` and `balance_after`.
- `BalanceDebited` (topic: `subscription_id: u64`) — successful prepaid period settlement; contains the exact `amount` and `balance_after`.
- `BalanceRefunded` (topic: `subscription_id: u64`) — successful subscriber withdrawal or cancellation refund; contains `amount` and `balance_after`.
- `AutoTopupTriggered` (topic: `subscription_id: u64`) — a prepaid balance was topped up from the subscriber during `charge`; contains the top-up `amount` and the `balance_after` it, before the period's debit.
- `TrialStarted` (topic: `subscription_id: u64`) — emitted by `subscribe_with_trial` on success; contains `trial_end`, the timestamp when the first charge becomes due.
- `TrialConverted` (topic: `subscription_id: u64`) — emitted once when the first successful post-trial `charge` or `charge_catchup` converts the subscription into paid billing; contains `trial_end`.
- `Cancelled` (topic: `subscription_id: u64`) — emitted when a subscription is cancelled via `cancel`. Contains `subscriber`.
- `Charged` (topic: `subscription_id: u64`) — emitted on successful billing via `charge` (one event) or `charge_catchup` (one event per settled period). Contains `amount`, `last_charged`, and `next_charge_at`.
- `Cancelled` (topic: `subscription_id: u64`) — emitted when a subscription is cancelled via `cancel`. Contains `subscriber`.
- `RenewalPolicyChanged` reports subscriber policy changes; `max_renewals = 0`
  means unlimited.
- `Renewed` reports successful permissionless renewals and the completed
  renewal count; the ordinary `Charged` event is also emitted.

`pause` and `resume` currently emit no contract events. `Paused`/`Resumed`
events are planned in [issue #12](https://github.com/Meet-hybrid/soroban-forge/issues/12);
until that work lands, indexers should observe the status through the stored
subscription record rather than expecting lifecycle events for these calls.

## Automatic Renewal

The subscriber enables or disables policy with `set_renewal_policy`.
`renew(subscription_id)` settles one elapsed period while the subscription
is active, policy is enabled, and the call lands between the due timestamp
and seven days after it (inclusive). A configured maximum is enforced; zero
means unlimited. A failed transfer returns `TokenTransferFailed` and leaves
the subscription and renewal counter unchanged. `get_renewal_policy` reports
the next due timestamp, current eligibility, and allowance expiry ledger.
Enabling policy approves this contract as a SEP-41 spender; finite renewal
counts are enforced independently by each subscription policy. Disabling a
policy closes that subscription's renewal gate, and the shared allowance is
revoked when no other active policy for the same subscriber and token remains.
The token's temporary allowance lives through its
maximum TTL, so the subscriber must re-enable policy after that ledger window
to continue automatic renewals. Policy uses a parallel key so the serialized
`Subscription` record does not change. Manual charges and renewals advance the
same `last_charged` value, preventing double settlement.

## Plan Registry

A provider can publish a reusable billing plan once and share its `plan_id`
with subscribers, who then self-serve without needing the token, amount, and
period out-of-band.

### Creating a plan

```rust
fn create_plan(provider, token, amount, period, quotas) -> Result<u64, ForgeError>
```

- Requires the **provider's authorization**.
- Validates `amount > 0` and `period > 0` before auth, so invalid inputs never
  spend the provider's signature.
- `quotas` follows the same validation as `set_quotas`: at most 16 entries, no
  duplicate metrics, `bucket_units > 0`, `overage_price >= 0`. Pass an empty
  `Vec` for flat pricing.
- Returns a stable `plan_id` allocated from a **separate monotonic counter**
  (`DataKey::PlanCount`) so plan ids and subscription ids are in distinct
  namespaces and can never be confused.
- Plan ids are sequential starting from 1 and stable — they never change after
  creation.

### Subscribing to a plan

```rust
fn subscribe_to_plan(plan_id, subscriber) -> Result<u64, ForgeError>
```

- Requires the **subscriber's authorization**.
- Looks up the plan; returns `ForgeError::NotFound` for an unknown `plan_id`.
- Creates a `Subscription` record with the plan's token, amount, period, and
  quotas copied verbatim. The subscription id is allocated from the **same**
  counter as `subscribe`, so all subscription ids are globally unique regardless
  of creation path.
- **A subscriber can hold multiple subscriptions to the same plan.** Each call
  creates a distinct record with its own `subscription_id`, `last_charged`
  timestamp, and independent lifecycle. This mirrors the existing behaviour of
  `subscribe`, which also creates a new record on every call even with the same
  provider.
- The resulting subscription is indistinguishable from one created via
  `subscribe`: it appears in both subscriber and provider secondary indexes,
  supports `charge`, `charge_catchup`, `pause`, `resume`, `cancel`,
  `set_quotas`, `record_usage`, and all views.

### Reading plan state

```rust
fn get_plan(plan_id) -> Result<Plan, ForgeError>  // no auth required
fn plan_count() -> u64                            // no auth required
```

- `get_plan` returns `ForgeError::NotFound` for an unknown id. No
  authorization required.
- `plan_count` returns the total number of plans created (the monotonic counter;
  it never decreases). No authorization required.

### Plan struct

```rust
struct Plan {
    plan_id:  u64,
    provider: Address,
    token:    Address,
    amount:   i128,
    period:   u64,
    quotas:   Vec<MetricQuota>,  // empty = flat pricing
}
```

### Authorization table (plan entrypoints)

| Entrypoint          | Required authorization |
| ------------------- | ---------------------- |
| `create_plan`       | provider               |
| `subscribe_to_plan` | subscriber             |
| `get_plan`          | none                   |
| `plan_count`        | none                   |

### Storage keys (plan registry)

- `Plan(plan_id: u64)` — the plan record.
- `PlanCount` — monotonic plan id counter (separate from `Count` used for
  subscriptions).

### Change the recurring price

The subscriber and provider both authorize `change_plan(subscription_id,
new_amount)` for an active subscription. The new base price must be positive
and different. The billing timestamp and period length stay fixed. The call
returns the newly granted credit and emits `PlanChanged` with the subscription
ID, old price, new price and credit, all amounts in token base units.

Credit is the old base price multiplied by the unused seconds in the current
period, divided by the period length and rounded down. A change at the start
credits the full old price. A change at or after the due time credits zero.
Repricing again before billing can change the next price, but the same unused
period earns credit only once. A separate persistent key tracks this credit,
so existing subscription records need no migration.

`quote_period` includes available credit. Each successful charge applies as
much credit as the bill can absorb and carries any excess into later periods.
For example, with 90 credit and a price of 30, three due periods can settle
with zero transferred. The billing timestamp still advances. A failed charge
preserves the credit. Pull and prepaid charges use the same deduction.

### Preview catch-up

Call `preview_catchup(subscription_id, max_periods)` to quote a successful
`charge_catchup` at the current ledger time. The same maximum of 32 applies.
The response reports the token amount payable for the selected periods,
`prepaid_consumed`, the token amount still overdue after the selected periods,
and the resulting status. A limit of zero shows all arrears under
`arrears_remaining`. The preview reads the open period's usage and credits
without transferring tokens, writing records or extending TTLs.

Catch-up accepts active pull subscriptions. For accepted calls,
`prepaid_consumed` is zero and the resulting status is active. A prepaid,
paused, cancelled or past-due subscription returns `InvalidInput` from both
preview and settlement. A quote assumes that the token transfers will succeed.
If a later transfer fails, catch-up rolls back the entire call. Repeated
single-period charges retain earlier successful periods and record a failed
attempt for the first unpaid one.
