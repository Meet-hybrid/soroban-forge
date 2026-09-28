# Escrow Contract

Secure fund custody for buyer-seller transactions with optional arbiter dispute resolution.

## Interface

```rust
fn create_escrow(buyer, seller, arbiter, token, amount, timeout) -> Result<u64, ForgeError>
fn deposit(escrow_id) -> Result<(), ForgeError>
fn release(escrow_id) -> Result<(), ForgeError>
fn release_partial(escrow_id, amount) -> Result<(), ForgeError>
fn refund(escrow_id) -> Result<(), ForgeError>
fn dispute(escrow_id, claimant) -> Result<(), ForgeError>
fn resolve(escrow_id, in_favor_of_seller) -> Result<(), ForgeError>
fn cancel(escrow_id) -> Result<(), ForgeError>
fn create_escrow_multi(buyer, seller, arbiter, assets, timeout) -> Result<u64, ForgeError>
fn deposit_multi(escrow_id) -> Result<(), ForgeError>
fn release_multi(escrow_id) -> Result<(), ForgeError>
fn refund_multi(escrow_id) -> Result<(), ForgeError>
fn dispute_multi(escrow_id, claimant) -> Result<(), ForgeError>
fn resolve_multi(escrow_id, in_favor_of_seller) -> Result<(), ForgeError>
fn cancel_multi(escrow_id) -> Result<(), ForgeError>
fn get_status(escrow_id) -> Result<EscrowStatus, ForgeError>
fn get_escrow(escrow_id) -> Result<EscrowData, ForgeError>
fn get_basket_escrow(escrow_id) -> Result<BasketEscrowData, ForgeError>
fn escrows_for_participant(participant, cursor, limit) -> ParticipantEscrowsPage
fn touch_ttl(escrow_id) -> Result<(), ForgeError>
```

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

## Partial Release

`release_partial(escrow_id, amount)` allows the seller to receive the escrow
balance incrementally while the escrow remains `Funded`.

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

`release_partial` is seller-authorized. Invalid requests (wrong status,
non-positive amount, amount exceeding remaining) return `InvalidInput` and
do not modify any storage.

### Events

`release_partial` emits `PartiallyReleased` (not `Released`). The event
carries `partial_amount` (the incremental transfer) and the full `EscrowData`
(including updated `released` and `status`). The existing `Released` event
remains exclusively for the `release` entrypoint and signals terminal
completion to indexers.

## Multi-Asset Basket Escrows

Multi-asset basket escrows support custody and settlement of baskets containing $1 \le N \le 10$ distinct SEP-41 assets under all-or-nothing settlement semantics.

### Basket Asset Types & Validation

- `BasketAsset { token: Address, amount: i128 }`
- `BasketEscrowData { escrow_id, buyer, seller, arbiter, assets, timeout, status, created_at }`
- **Validation**:
  - Basket size bounded to `1..=10` assets.
  - Duplicate token addresses are strictly rejected with `ForgeError::InvalidInput`.
  - Every asset must have `amount > 0`.
  - `timeout > 0`.

### Settlement & Atomicity Semantics

- **All-or-Nothing Funding**: `deposit_multi` transfers all basket assets into the contract. It transitions from `Pending` to `Funded` if and only if all transfers succeed.
- **Transfer-Before-State Discipline**: All token transfers across the basket are executed in sequence before persistent storage state is mutated. If any transfer fails, the Soroban host reverts the entire invocation, restoring all token balances and leaving contract storage untouched.
- **Per-Asset Conservation**: Across all terminal payout paths (`release_multi`, `refund_multi`, `resolve_multi`), all $N$ assets are transferred to the recipient (seller or buyer), ensuring exact per-asset balance equality with contract balances ending at zero.
- **Dispute & Arbitration**: `dispute_multi` freezes all assets in the basket under `EscrowStatus::Disputed`. `resolve_multi` pays out all basket assets all-or-nothing to the seller (`true`) or back to the buyer (`false`).
- **Cancellation**: `cancel_multi` cancels a `Pending` basket escrow and cleans up participant index entries while preserving survivor order.

### Basket Events

Dedicated events with `escrow_id` as the topic:
- `BasketEscrowCreated { escrow_id, data }`
- `BasketDeposited { escrow_id, data }`
- `BasketReleased { escrow_id, data }`
- `BasketRefunded { escrow_id, data }`
- `BasketDisputed { escrow_id, data }`
- `BasketResolved { escrow_id, data, in_favor_of_seller }`
- `BasketCancelled { escrow_id, data }`

### Storage & Shared ID Space

- Isolated storage key `DataKey::BasketEscrow(u64)` prevents schema interference or XDR map mismatches with single-asset `DataKey::Escrow(u64)`.
- Monotonic counter `DataKey::Count` allocates ids sequentially across both single and basket escrows.
- `get_status(id)` and `touch_ttl(id)` transparently support both single and basket escrow ids.
- `get_basket_escrow(id)` retrieves the full basket record.

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

## WASM Budget

Current size: ~28 KB  
Limit: < 150 KB

## Feature Flags

- `test-utils` — enables test-only helpers (proptest, authz tests)
