# Escrow Contract

Secure fund custody for buyer-seller transactions with optional arbiter dispute resolution.

## Interface

```rust
fn create_escrow(buyer, seller, arbiter, token, amount, timeout) -> Result<u64, ForgeError>
fn deposit(escrow_id) -> Result<(), ForgeError>
fn release(escrow_id) -> Result<(), ForgeError>
fn release_partial(escrow_id, amount) -> Result<(), ForgeError>
fn schedule_release(escrow_id, release_at) -> Result<(), ForgeError>
fn execute_scheduled(escrow_id) -> Result<(), ForgeError>
fn refund(escrow_id) -> Result<(), ForgeError>
fn dispute(escrow_id, claimant) -> Result<(), ForgeError>
fn resolve(escrow_id, in_favor_of_seller) -> Result<(), ForgeError>
fn cancel(escrow_id) -> Result<(), ForgeError>
fn get_status(escrow_id) -> Result<EscrowStatus, ForgeError>
fn get_escrow(escrow_id) -> Result<EscrowData, ForgeError>
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
                    |        --schedule_release--> Funded (scheduled)
                    |        --execute_scheduled--> Completed (after time-lock)
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

## Time-Lock Release

`schedule_release(escrow_id, release_at)` lets the buyer schedule a future
release of the escrow's remaining balance. The escrow stays `Funded` while
scheduled; no funds move until `execute_scheduled` is called after the
time-lock expires.

### Scheduling

- `schedule_release` is buyer-authorized.
- `release_at` is a ledger timestamp (seconds) in the future; it must be
  strictly greater than the current ledger timestamp.
- The escrow must be `Funded` and must not already have a scheduled release.
- Scheduling does not move funds and does not change `status`.
- The scheduled release time is stored per escrow id.

### Execution

- `execute_scheduled` is permissionless: anyone may call it once the
  time-lock has expired.
- It validates that a schedule exists and that
  `ledger.timestamp() >= release_at`.
- On success it transfers the **remaining** balance to the seller and
  transitions the escrow to `Completed`, identical to `release`.
- The scheduled release entry is cleared on execution.

### Cancellation and Overrides

- `release` and `release_partial` may be called while a schedule is pending;
  doing so clears the pending schedule.
- `refund`, `dispute`, and `cancel` also clear any pending schedule.
- A schedule cannot be created on a `Completed`, `Refunded`, `Disputed`, or
  `Cancelled` escrow, nor on one whose `release_at` is in the past.

### Events

- `schedule_release` emits `ReleaseScheduled` carrying `escrow_id` and
  `release_at`.
- `execute_scheduled` emits `Released` (same shape as the direct `release`
  entrypoint) so indexers treat both terminal paths uniformly.

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

A separate `DataKey::ScheduledRelease(id)` entry stores the pending
`release_at` timestamp. It is written by `schedule_release` and removed on
execution or on any state-changing call that clears the schedule.

## WASM Budget

Current size: ~28 KB  
Limit: < 150 KB

## Feature Flags

- `test-utils` — enables test-only helpers (proptest, authz tests)

## Time-Lock Patterns

- **Scheduled payment**: buyer calls `schedule_release` with `release_at =
  now + N days`; seller (or any party) calls `execute_scheduled` after the
  window to settle.
- **Cool-off period**: buyer schedules release immediately after funding with
  a short delay, retaining the ability to `refund` or `dispute` before the
  window elapses.
- **Regulatory reversal window**: schedule with a 48-hour delay to satisfy
  compliance requirements before funds become irrevocably the seller's.
- **Multi-step workflows**: combine `schedule_release` with off-chain
  approvals; on-chain execution remains gated solely by the time-lock.
