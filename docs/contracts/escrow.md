# Escrow Contract

Secure fund custody for buyer-seller transactions with optional arbiter dispute resolution.

## Interface

```rust
fn create_escrow(buyer, seller, arbiter, token, amount, timeout) -> Result<u64, ForgeError>
fn create_escrow_with_review(buyer, seller, arbiter, token, amount, timeout, review_period) -> Result<u64, ForgeError>
fn deposit(escrow_id) -> Result<(), ForgeError>

// Seller is paid after a review window (see below)
fn request_release(escrow_id, amount) -> Result<u64, ForgeError>    // seller; returns release_at
fn approve_release(escrow_id) -> Result<(), ForgeError>             // buyer: pay now
fn execute_release(escrow_id) -> Result<(), ForgeError>             // anyone, after release_at
fn get_release_request(escrow_id) -> Option<ReleaseRequest>
fn review_period(escrow_id) -> u64

fn release(escrow_id) -> Result<(), ForgeError>                     // buyer pays directly
fn release_partial(escrow_id, amount) -> Result<(), ForgeError>     // buyer pays directly
fn refund(escrow_id) -> Result<(), ForgeError>
fn refund_expired(escrow_id) -> Result<(), ForgeError>
fn dispute(escrow_id, claimant) -> Result<(), ForgeError>
fn resolve(escrow_id, in_favor_of_seller) -> Result<(), ForgeError>
fn cancel(escrow_id) -> Result<(), ForgeError>
fn get_status(escrow_id) -> Result<EscrowStatus, ForgeError>   // either record kind
fn get_escrow(escrow_id) -> Result<EscrowData, ForgeError>     // single-token records
fn escrows_for_participant(participant, cursor, limit) -> ParticipantEscrowsPage
fn touch_ttl(escrow_id) -> Result<(), ForgeError>              // either record kind

// Multi-asset baskets (additive; the single-token interface above is unchanged)
fn create_basket(buyer, seller, arbiter, assets: Vec<EscrowAsset>, timeout) -> Result<u64, ForgeError>
fn deposit_basket(escrow_id) -> Result<(), ForgeError>
fn request_release_basket(escrow_id) -> Result<u64, ForgeError>     // seller; every leg
fn release_basket(escrow_id) -> Result<(), ForgeError>              // buyer pays directly
fn release_partial_basket(escrow_id, token, amount) -> Result<(), ForgeError>
fn refund_basket(escrow_id) -> Result<(), ForgeError>
fn dispute_basket(escrow_id, claimant) -> Result<(), ForgeError>
fn resolve_basket(escrow_id, in_favor_of_seller) -> Result<(), ForgeError>
fn cancel_basket(escrow_id) -> Result<(), ForgeError>
fn get_basket(escrow_id) -> Result<BasketEscrowData, ForgeError> // basket records
```

## Multi-Asset Baskets

A basket is an escrow over an ordered list of legs
(`EscrowAsset { token, amount, released }`) instead of a single
`(token, amount)` pair. It is **additive**: `EscrowData`, `DataKey::Escrow(id)`,
and every single-token event keep their exact shape, so the single-token flow,
its ABI consumers, and its stored records are untouched.

- **Shared id space.** Baskets draw from the same monotonic `DataKey::Count`
  sequence as single-token escrows, so an id names exactly one record of one
  kind. `get_status` / `touch_ttl` work for either kind; `get_escrow` reads
  single-token records and `get_basket` reads baskets (`NotFound` for the
  other kind). The participant index is shared and deduplicated per address.
- **Basket bounds.** 1..=8 legs (`MAX_BASKET_ASSETS`), each `amount > 0`, no
  token repeated (duplicates are rejected, not merged). `create_basket` is
  buyer-authorized, list order is preserved, and the assets vector is part of
  the approved invocation.
- **All-or-nothing deposit.** `deposit_basket` pulls every leg in list order;
  a failure on any leg rolls the whole frame back (host rollback), so custody
  never holds a partial basket and the basket stays `Pending` and retryable.
  The buyer must authorize **one transfer per leg**.
- **Per-leg accounting.** `remaining = amount - released` per leg. A basket
  reaches `Completed` only when **every** leg is fully released, so emptying
  one leg leaves the basket `Funded` with a zero-balance leg.
  `release_partial_basket(escrow_id, token, amount)` pays one named leg
  (unknown token → `NotFound`).
- **Terminal paths pay every leg.** `release_basket` / `refund_basket` /
  `resolve_basket` move each leg's remaining balance through the same payout
  loop, one direction for the whole basket. `refund_basket` keeps the
  single-token refund party rule (seller pre-deadline, buyer post-deadline).
  Auth is per entrypoint as with the single-token paths (see the lifecycle
  diagram below).
- **Dispute freezes all legs.** `dispute_basket` freezes every leg until the
  arbiter resolves (`resolve_basket`, arbiter-only, final).
- **Shared internals.** The single-token paths run through the *same* transfer
  primitives as baskets (a single-token escrow is a one-leg basket), so the
  payout loop and its conservation rule literally cannot drift between kinds.
- **TTL per escrow.** A basket is one persistent entry, extended as a unit;
  legs can never expire independently.

### Basket events

`BasketCreated`, `BasketDeposited`, `BasketReleased`,
`BasketPartiallyReleased` (carries `token` + `partial_amount`), `BasketRefunded`,
`BasketDisputed`, `BasketResolved` (carries `in_favor_of_seller`),
`BasketCancelled` — each carries the id as topic and the full
`BasketEscrowData` payload. Single-token event payloads are unchanged.
## Participant Index and Pagination

Each distinct buyer, seller, and arbiter receives a persistent participant
index in creation order. Creating an escrow adds its id once to each distinct
party's list. Cancelling a `Pending` escrow removes its id from all of those
lists after the status and buyer-authorization checks succeed. Removal keeps
the remaining ids in their original relative order. `release`, `refund`, and
`resolve` do not remove ids: those terminal escrow records remain addressable
through `get_escrow` and participant views. Thus the index contains every
non-cancelled escrow record, whether pending, funded, or terminal.

`escrows_for_participant` uses live offset pagination over the current
compacted list. `cursor` is an index offset at the moment of the call, not a
snapshot token. If cancellation removes an id before a cursor returned by a
previous page, later ids shift left and continuing from that saved cursor can
skip an id. Clients that need a complete view after an intervening mutation
should restart at cursor `0`. Without mutation, replaying `next_cursor` yields
every indexed id exactly once in creation order. `limit == 0`, cursors past
the end, and `u32::MAX` cursor/limit values return an empty terminal page
without overflow.

The invariant is maintained in the successful cancel path only: missing ids,
non-`Pending` records, and failed authorization do not alter either party's
index. No entrypoint signatures or generated TypeScript ABI changed.

## Lifecycle

```text
Pending --deposit--> Funded --request_release--> review window --execute_release / approve_release--> paid
                    |                              (dispute during the window freezes it instead)
Pending --deposit--> Funded --release_partial (×n)--> Funded  (partial)
                    |                                  |
                    |                                  +--> Completed (final partial)
                    |        --release--> Completed (full, direct)
                    |        --refund--> Refunded   (buyer back, remaining only)
                    |        --dispute--> Disputed --resolve--> Completed | Refunded
         --cancel--> Cancelled (before funding only)
```

## States

- `Pending` — Created but not funded
- `Funded` — Funds deposited; partial releases may be applied
- `Completed` — Released to seller (full release or final partial release)
- `Refunded` — Remaining balance returned to buyer
- `Disputed` — Under arbitration (remaining balance frozen)
- `Cancelled` — Cancelled before funding

## Release With a Review Window

The seller is paid in two steps, so neither side can take the money alone:

1. **The seller asks:** `request_release(escrow_id, amount)` (seller-signed).
   `amount` is the whole remaining balance or part of it for a milestone.
   Nothing moves; the call returns `release_at = now + review window`.
2. **The buyer reviews** until `release_at`, and can:
   - `approve_release(escrow_id)` (buyer-signed): the seller is paid now;
   - `dispute(escrow_id, buyer)`: the escrow freezes and the arbiter decides.
3. **The window ends** with no dispute: anyone calls
   `execute_release(escrow_id)` and the seller is paid.

| Rule | Why |
| --- | --- |
| The review window is set at creation: `create_escrow_with_review(..., review_period)` (1 minute to 30 days), or 3 days with `create_escrow`. Escrows from before this version read as 3 days | Both parties know the terms before funding |
| One request at a time; a second returns `ReleasePending` | The buyer always reviews one known amount |
| A request must be made **before** the escrow's deadline (`DeadlineReached` after) | A seller cannot block a refund that is already due |
| While a request is pending, the buyer's after-deadline `refund` and `refund_expired` return `ReleasePending` | A buyer cannot sit on a delivered order and take the money back after the deadline; they approve or dispute |
| Any payout, refund or dispute clears the pending request | A stale request can never pay out later |
| `execute_release` before `release_at` returns `DeadlineReached` | Nothing is paid early |

`release` and `release_partial` still exist and pay immediately, but they are
now **buyer-signed**: the paying party approving payment. Until this version
they were seller-signed, which let a seller pay themselves before the buyer
could raise a dispute. Baskets follow the same rules with
`request_release_basket` (always every leg) and buyer-signed
`release_basket` / `release_partial_basket`.

Events: `request_release*` emits `ReleaseRequested { escrow_id, amount,
release_at }` (`amount` is `0` for a basket). Paying a request emits the same
event as a direct payment: `Released` when it pays everything left,
`PartiallyReleased` for a milestone, `BasketReleased` for a basket.

Storage: `DataKey::ReleaseRequest(id)` holds the pending request and
`DataKey::ReviewPeriod(id)` the window. Both are new keys; `EscrowData` and
`BasketEscrowData` keep their stored shape.

### Testnet proof

[`docs/testnet/review-window-receipt.json`](../testnet/review-window-receipt.json)
records a live run on 13 November 2026 against a fresh deployment
(`CAGIVLP2G4GUJFAX4UHNQDZ7UZ6OFHT76ZZMMLKDEIHAHOBMISVNFCKU`) with native XLM and
a 60-second window, from `scripts/review-window-proof.py`:

| Round | Result |
| --- | --- |
| Request, window ends, a third party executes | Executing early was refused (`DeadlineReached`) and the contract kept the 10 XLM; after the window the seller was paid |
| Request, buyer approves | Seller paid at once |
| Request, buyer disputes | Executing was refused (`NotFound`); the arbiter refunded the buyer |
| Milestones | 3 XLM through the window (escrow stayed `Funded`), then 7 XLM approved (`Completed`) |
| Request just before the deadline | After the deadline, `refund` and `refund_expired` were both refused with `ReleasePending`; the seller was paid when the window ended |
| Seller calls `release` directly | Refused; the buyer's direct `release` still works |

The contract held nothing extra afterwards. The earlier escrow deployment
(`CC227UDF…NBZHSB`) predates review windows and keeps seller-confirmed release.

## Partial Release

`release_partial(escrow_id, amount)` lets the buyer pay the seller part of the
balance directly while the escrow remains `Funded`. Sellers request milestone
payments with `request_release(escrow_id, amount)` instead.

### Accounting

```text
remaining = deposited - released
deposited = amount field set at create_escrow
released  = cumulative amount transferred to seller via release_partial
```

- Each successful `release_partial` call:
  1. Validates `status == Funded`, `amount > 0`, `amount <= remaining`
  2. Transfers exactly `amount` to the seller (transfer-before-state)
  3. Increments `released` by `amount`
  4. If `amount == remaining`: transitions to `Completed`
- `release` (the original full-release entrypoint) pays the **remaining** balance
  in one call and is backward-compatible — it behaves identically to before
  when no partial releases have been made.
- `refund` and `resolve` operate on the **remaining** balance only.
- `dispute` freezes the **remaining** balance.

### Authorization

`release_partial` is buyer-authorized. Invalid requests (wrong status,
non-positive amount, amount exceeding remaining) return `InvalidInput` and
do not modify any storage.

### Events

`release_partial` emits `PartiallyReleased` (not `Released`). The event
carries `partial_amount` (the incremental transfer) and the full `EscrowData`
(including updated `released` and `status`). The existing `Released` event
remains exclusively for the `release` entrypoint and signals terminal
completion to indexers.

## Storage Compatibility

`EscrowData` now carries a `released: i128` field. Records written by
earlier contract versions do not contain this field.

**Strategy**: two-type fallback decode in `load_escrow`.
1. Attempt deserialization as current `EscrowData` (10 fields including `released`).
2. On failure, attempt as `EscrowDataV1` (9 fields, no `released`).
3. Convert `EscrowDataV1` → `EscrowData` by defaulting `released = 0`.

This works because Soroban `#[contracttype]` structs are stored as XDR
symbol-keyed maps. The host rejects deserialization when the map's entry count
differs from the struct's field count; old 9-field records fail step 1 and
succeed at step 2. `EscrowDataV1` is a read-only migration type; new code
never writes it. Lazy migration: the first state-changing call on an old record
writes the current schema back.

`DataKey::Escrow(id)` is unchanged.

`DataKey::ReleaseRequest(id)` and `DataKey::ReviewPeriod(id)` are new keys;
old escrows have no entry, which reads as "no pending request" and the
3-day default window.

## WASM Budget

Current size: ~48 KB (unoptimized release build)  
Limit: < 150 KB

## Feature Flags

- `test-utils` — enables test-only helpers (proptest, authz tests)
