# Subscription Payments Contract

Recurring, on-chain subscription billing: a subscriber authorizes a provider to
pull a fixed `amount` per `period` (seconds). This iteration tracks subscription
state and billing cadence; token settlement is out of scope.

## Interface

```rust
fn subscribe(subscriber, provider, token, amount, period) -> Result<u64, ForgeError>
fn charge(subscription_id) -> Result<i128, ForgeError>
fn cancel(subscription_id) -> Result<(), ForgeError>
fn get_subscription(subscription_id) -> Result<Subscription, ForgeError>
fn get_subscription_count() -> u64
fn subscriptions_for_subscriber(subscriber, offset, limit) -> Result<Vec<Subscription>, ForgeError>
fn subscriptions_for_provider(provider, offset, limit) -> Result<Vec<Subscription>, ForgeError>
```

- `subscribe` requires the subscriber and returns a stable, monotonic id.
- `charge` requires the provider and bills at most one full period per call.
- `cancel` requires the subscriber and prevents further charges.

## Subscription States

- `Active` — chargeable
- `Cancelled` — no further charges
- `PastDue` — reserved for a failed-payment retry model in a follow-up

## Secondary Indices

Each subscription is indexed in two secondary lists, written on the `subscribe`
success path:

- `SubscriberSubscriptions(subscriber)` — creation-order subscription ids for
  which the address is the subscriber.
- `ProviderSubscriptions(provider)` — creation-order subscription ids for which
  the address is the provider.

The indices store lightweight `Vec<u64>` id lists; full `Subscription` records
are only loaded for the requested page slice.

## View Methods

- `get_subscription_count()` returns the total number of subscriptions created
  (the monotonic id counter; cancellation never lowers it).
- `subscriptions_for_subscriber(subscriber, offset, limit)` returns a page of
  `Subscription` records for the subscriber, in creation order.
- `subscriptions_for_provider(provider, offset, limit)` returns a page of
  `Subscription` records for the provider, in creation order.

Both enumeration methods are read-only (no authorization, no storage writes).
`offset` skips the first `offset` subscriptions and `limit` caps the page size:
an empty index or an out-of-bounds offset yields an empty `Vec`, and a `limit`
of `0` fails with `ForgeError::InvalidInput`.

## Storage

- `Subscription(id)` — the record for id `u64`.
- `Count` — monotonic subscription id counter.
- `SubscriberSubscriptions(address)` — subscriber id index.
- `ProviderSubscriptions(address)` — provider id index.

## Payment Semantics

`charge` returns the billed amount, or `0` when no full period has elapsed since
the last charge. Repeating the call catches up at most one period at a time.
