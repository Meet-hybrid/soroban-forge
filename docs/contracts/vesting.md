# Vesting Contract

Time-locked token release in two shapes: a cliff plus linear release, or an
explicit table of unlock tranches.

## Interface

```rust
// linear (cliff + ramp)
fn create_schedule(funder, beneficiary, token, total_amount, cliff, duration) -> Result<u64, ForgeError>
fn get_schedule(schedule_id) -> Result<VestingSchedule, ForgeError>
fn revoke(schedule_id) -> Result<(), ForgeError>
fn refund_revoked(schedule_id) -> Result<i128, ForgeError>
fn reassign_beneficiary(schedule_id, new_beneficiary) -> Result<(), ForgeError>
// tranche (discrete unlock table)
fn create_tranche_schedule(funder, beneficiary, token, tranches: Vec<Tranche>) -> Result<u64, ForgeError>
fn get_tranche_schedule(schedule_id) -> Result<TrancheSchedule, ForgeError>
fn revoke_tranche_schedule(schedule_id) -> Result<i128, ForgeError>
// both kinds
fn claim(schedule_id) -> Result<i128, ForgeError>
fn claim_for(schedule_id, beneficiary) -> Result<i128, ForgeError>
fn claimable(schedule_id) -> Result<i128, ForgeError>
fn claimable_for(schedule_id, beneficiary) -> Result<i128, ForgeError>
fn get_status(schedule_id) -> Result<VestingStatus, ForgeError>
fn touch_ttl(schedule_id) -> Result<(), ForgeError>
```

`Tranche { unlock_at: u64, amount: i128 }` is one entry of the unlock table:
`unlock_at` is an **offset in seconds from the schedule start** (the ledger
timestamp recorded at creation) and `amount` is what unlocks there.

Both kinds draw ids from one counter, so the id space is shared; the two
records live under distinct storage keys and a linear id is `NotFound` in
`get_tranche_schedule` (and vice versa).

`get_schedule` is the linear kind's read-only record view: it returns the
stored `VestingSchedule` (funder, current beneficiary, token, total amount,
start, cliff, duration, total claimed across beneficiaries, reassignment
snapshot and count, frozen revoked amount, stored status) for an
existing id, mirroring `get_tranche_schedule` and the workspace's other
record views (`get_escrow`, `get_tx`, `get_proposal`,
`get_subscription`). `claimed` is the total already paid across beneficiaries; the stored
`status` is refreshed on claim, and `get_status` derives the current one
from ledger time between claims.

## Timing

Timings in both shapes are **durations in seconds measured from the schedule
start**.

### Linear shape

```text
start ......... start+cliff ................... start+duration
  |             (claims become possible)        (fully vested)
  |  Locked    |            Vesting (linear)  |
```

- `start + cliff` — claims become possible;
- `start + duration` — the schedule is fully vested.

Validation at creation: `total_amount > 0`, `duration > 0`, `cliff <= duration`,
`funder != beneficiary` (failures return `ForgeError::InvalidInput`).

### Tranche shape

```text
       t1            t2                t3
start  |             |                 |
  |    |   tranche 1  |   tranche 2     |   tranche 3
  | Locked  (a1)         (a2)              (a3)
  |    |  vested:      vested:           vested:
  |    |     0  ->     a1  ->            a1+a2  ->  total
  +----+--------------+------------------+--------->
```

A grant agreement of the form "25% at TGE, 25% at +6 months, 50% at +12
months" is a table, not a ramp, so this shape takes it verbatim. Validation at
creation:

- `funder != beneficiary` (`ForgeError::InvalidInput`);
- the table is non-empty and holds at most `MAX_TRANCHES` (32) entries;
- every `amount > 0`;
- `unlock_at` strictly increases (a repeat or a rewind is rejected);
- the amounts sum to at most `i128::MAX` (`ForgeError::ArithmeticOverflow`).

A first tranche at `unlock_at == 0` unlocks at the creation timestamp — the
"TGE tranche" of a typical agreement. The table is validated, stored once, and
immutable afterwards.

## Release Formula

Linear, at ledger time `t`:

```text
0                                            when t < start + cliff
total_amount * (t - (start + cliff)) / (duration - cliff)   otherwise, floored
total_amount                                 when t >= start + duration
```

Tranche, at ledger time `t`: the sum of every tranche whose offset has
elapsed, i.e. `0` before the first unlock, `a1` up to the second, `a1 + a2` up
to the third, and the full total from the last unlock on. Nothing accrues
between unlocks.

Integer (floor) division means a linear claim never rounds up, so repeated
claims can never overpay or underpay: for either kind `claim` returns exactly
`unlocked - claimed`, or `0` when nothing is claimable. Tranche progress is
compared on the elapsed offset rather than on `start + unlock_at`, so an offset
of `u64::MAX` cannot overflow — that tranche simply never unlocks.

This also means callers may make partial claims at arbitrary timestamps: each
claim receives only the newly unlocked amount, while any floor-division residue
remains in custody until the claim at or after the vesting end pays the exact
remaining balance.

On a `Revoked` linear schedule the vested amount is frozen at the revocation
timestamp (`revoked_vested`): the release formula returns that frozen amount
for every later time, so `claim` succeeds only up to it and `claimable`
returns `0` once it is claimed. Nothing after the revocation timestamp ever
vests.

On a `Revoked` tranche schedule the cumulative unlocked amount at the
revocation timestamp is similarly frozen (`revoked_unlocked`): `claimable`
returns `revoked_unlocked - claimed` (`0` once drained), nothing further ever
unlocks, and attempting to `claim` locked tranches once no unlocked balance
remains is rejected with `ForgeError::InvalidInput`.

### Reassignment split

`reassign_beneficiary(schedule_id, new_beneficiary)` is funder-authorized and
only applies to a live linear schedule. It snapshots the vested amount at the
current ledger timestamp without changing `start`, `cliff`, `duration`, or
`total_amount`. The outgoing beneficiary's vested-but-unclaimed amount is
credited to that address; the new beneficiary starts with zero accrual at that
same timestamp and receives only subsequent vesting.

The split is exact at the boundary: a claim and reassignment in the same
ledger observes the pre-reassignment vested amount for the outgoing
beneficiary. `claim` and `claimable` address the current beneficiary;
`claim_for` and `claimable_for` address a named beneficiary and include any
unclaimed frozen balance from earlier assignments. Former-beneficiary balances
are keyed by both schedule id and address, so one beneficiary's other schedules
are unaffected. Reassignment is unlimited but increments
`VestingSchedule.reassignment_count` on every successful change.

Each successful change emits `BeneficiaryReassignedFrom` and
`BeneficiaryReassignedTo`, with the schedule id and corresponding beneficiary
as topics, plus the other party, frozen vested amount, and count in event data.
Reassignment to the current beneficiary or funder is rejected. Completed and
revoked schedules cannot be reassigned. If a reassigned schedule is later
revoked, existing former-beneficiary balances remain payable and the current
beneficiary can claim only their accrued portion up to the revocation snapshot.

## Status

Derived from ledger time and the claimed amount (always current between
claims): `Locked` before the first unlock (the cliff, for the linear kind),
`Vesting` from the first unlock until the last amount is claimed, `Completed`
once `claimed == total_amount`. `Revoked` is set by `revoke` (linear) or
`revoke_tranche_schedule` (tranche) and overrides the time/claimed derivation:
a revoked schedule stays `Revoked` even while its frozen remainder is unclaimed
or after it is drained.

## Revocation

### Linear revocation

`revoke(schedule_id)` terminates a linear schedule before completion:
`Locked | Vesting -> Revoked`, permanently stopping further vesting.

- Only the **funder** recorded at creation may revoke (`require_auth` on it);
  the beneficiary cannot. Revoking a `Completed` or already-`Revoked`
  schedule is rejected with `ForgeError::InvalidInput`, so double-revoke is
  impossible.
- The vested amount at the revocation ledger timestamp is frozen into the
  record: everything already vested stays claimable, nothing after that
  timestamp ever vests, and the unvested remainder is returned to the funder
  via `refund_revoked(schedule_id)`.
- Linear `revoke` is pure state-machine logic — it writes the frozen amount and
  the status and moves no tokens — so it stays compatible with the SEP-41
  settlement path (issue #50) and the storage-tier layout (issue #55).

### Tranche revocation

`revoke_tranche_schedule(schedule_id)` terminates a tranche schedule before all
tranches have unlocked: `Locked | Vesting -> Revoked`, freezing the schedule at
the current ledger timestamp and refunding the locked remainder to the funder
atomically in the same call.

- **Authorization**: Only the **funder** recorded at creation may call
  `revoke_tranche_schedule` (`require_auth` on `funder`); the beneficiary and
  third parties are rejected at the host.
- **Atomic refund (transfer-before-state)**: The cumulative amount unlocked at
  the revocation timestamp (`unlocked_at_revocation`) is frozen in
  `revoked_unlocked`, and the locked remainder (`total_amount -
  unlocked_at_revocation`) is transferred from the contract to `funder` before
  the updated record is written. If the token transfer fails, the call returns
  `ForgeError::TokenTransferFailed` and leaves the schedule unchanged.
- **Unlocked tranches stay claimable**: The beneficiary may still claim
  `revoked_unlocked - claimed` via `claim` / `claim_for`. `claimable` returns
  `0` once that frozen remainder is drained (or immediately if zero tranches
  had unlocked at revocation), and no later tranches ever unlock.
- **Views**: `get_tranche_schedule` returns the updated record with
  `status == VestingStatus::Revoked`, `revoked_unlocked == Some(unlocked)`, and
  `refunded_amount == total_amount - unlocked`; `get_status` returns
  `VestingStatus::Revoked`.
- **Documented error rejections**:
  - `ForgeError::NotFound`: unknown id or a linear schedule id.
  - `ForgeError::InvalidInput`: revoking twice (already `Revoked`), revoking a
    `Completed` schedule, revoking after every tranche has already unlocked
    (`unlocked_at_revocation >= total_amount`), or calling `claim` /
    `claim_for` against locked tranches on a `Revoked` tranche schedule when no
    unlocked balance remains.
  - `ForgeError::TokenTransferFailed`: the refund transfer to the funder failed.

## Authorization

- `create_schedule` and `create_tranche_schedule` require the beneficiary;
  the `funder` argument is recorded on the schedule without authorizing
  (mirroring escrow's non-consenting parties).
- `revoke`, `refund_revoked`, and `revoke_tranche_schedule` require the funder
  recorded at creation.
- `reassign_beneficiary` requires the funder recorded at creation.
- `claim` requires the current beneficiary; `claim_for` requires the named
  beneficiary.
- `claimable`, `claimable_for`, `get_status`, `get_schedule`, and
  `get_tranche_schedule` are read-only views.

## Settlement

## Events

`create_schedules_batch` publishes one `ScheduleCreated` event per created
linear schedule. `reassign_beneficiary` publishes `BeneficiaryReassignedFrom`
and `BeneficiaryReassignedTo`. `refund_revoked` publishes `RevokedRefunded`
when a non-zero linear refund executes. `revoke_tranche_schedule` publishes
`TrancheRevoked` with topics `(schedule_id, funder)` and data
`(unlocked_at_revocation, refunded_to_funder)`. Read-only calls and failed
invocations publish no vesting lifecycle event.

The contract custodies the SEP-41 token configured on the schedule, and
`claim` settles through it for both kinds; `claim_for` settles linear claims
for an explicit beneficiary:

- the newly claimable amount is transferred from the contract to the
  beneficiary **before** the schedule is written (escrow's
  transfer-before-state pattern); a failed transfer returns
  `ForgeError::TokenTransferFailed` with `claimed` and `status` unchanged;
- zero-claim calls (before the first unlock, or nothing newly unlocked)
  return `0` and issue **no** token transfer;
- floor-division residue on the linear kind is never lost: it stays claimable
  between claims and is paid out by the final claim — on a revoked schedule,
  by a claim up to the frozen amount.

## Storage Layout & Upgrade Compatibility

The vesting contract uses instance storage partitioned into four key variants:

```rust
enum DataKey {
    /// Holds a `VestingSchedule` record keyed by monotonic schedule id.
    Schedule(u64),
    /// Holds a `TrancheSchedule` record keyed by the same id space.
    TrancheSchedule(u64),
    /// Monotonic id counter (`u64`), initialized at 0 and incremented on each schedule creation.
    Count,
    /// Frozen, unclaimed amount owed to a former beneficiary.
    FormerBeneficiaryClaim(u64, Address),
}
```

### Storage Model

- **`DataKey::Count`**: Stores a single `u64` representing the highest allocated schedule id. Next id allocation uses checked addition (`checked_add(1)`), returning `ForgeError::ArithmeticOverflow` on counter saturation.
- **`DataKey::Schedule(u64)`**: Stores the `VestingSchedule` struct containing funder, current beneficiary, token address, total amount, start timestamp, cliff duration, total duration, total claimed amount, the current assignment's vested snapshot and claimed amount, reassignment count, optional frozen vested amount (`revoked_vested`), and derived lifecycle status.
- **`DataKey::FormerBeneficiaryClaim(u64, Address)`**: Stores a former beneficiary's frozen, unclaimed balance for that schedule. The schedule id is part of the key, so balances from separate grants never mix.
- **`DataKey::TrancheSchedule(u64)`**: Stores the `TrancheSchedule` struct containing funder, beneficiary, token address, total amount, start timestamp, the immutable unlock table, claimed amount, optional frozen unlocked amount (`revoked_unlocked`), refunded amount (`refunded_amount`), and derived lifecycle status.
- Instance storage lifetime is bound to the contract instance. In environments with storage TTL expiration, the contract instance TTL must be maintained to prevent storage eviction.

### Upgrade Compatibility

- **Key Segregation**: Because `DataKey::Schedule(u64)` and `DataKey::TrancheSchedule(u64)` use tuple variants and `DataKey::Count` is a unit variant, keys occupy non-overlapping namespaces within instance storage.
- **Record Schema Evolution**:
  - The `VestingSchedule` and `TrancheSchedule` structs are serialized via Soroban's `#[contracttype]`, where every struct field is a required key. The revocation/reassignment additions (`funder`, `revoked_vested`, `reassignment_vested`, `beneficiary_claimed`, and `reassignment_count` on `VestingSchedule`, plus `funder`, `revoked_unlocked`, and `refunded_amount` on `TrancheSchedule`) make this a **storage-breaking upgrade**: records written by an older build do not deserialize into the new types. Deployments must migrate or reset their instance storage before upgrading. The former-beneficiary key and `RevokedRefund` key are additive.
  - Future additions to either struct must maintain backwards deserialization compatibility (e.g. using `Option<T>` for newly introduced optional fields, or migrating storage records upon upgrade) — note that `Option<T>` helps only for reading absent fields into an optional, not for the required-key encoding itself.
  - Storage key enums must preserve existing discriminant ordering if extended (e.g. adding new key variants for administrative roles or persistent storage migration).
- **Storage Tier Migration**: If migrating from instance storage to persistent storage with per-schedule TTL management (mirroring the Escrow contract architecture), schedule records and `FormerBeneficiaryClaim` entries can be migrated to persistent storage while retaining `DataKey::Count` in instance storage. Reassignment adds a key variant but no storage tier.

Tests live in-crate (`crates/vesting/src/lib.rs`) and run with
`cargo test -p soroban-forge-vesting --all-targets --locked`.

## Discover schedules by beneficiary or funder

Call `schedules_for_beneficiary(beneficiary, offset, limit)` or
`schedules_for_funder(funder, offset, limit)` to list IDs in creation order.
Offsets are zero-based and the page limit is 1 through 100, inclusive. Invalid
limits return `InvalidInput`. An unknown address or offset beyond the index
returns an empty list. These calls require no authorization and do not change
storage or publish events.

Reassignment moves the ID between beneficiary indexes and keeps its creation
position in the resulting list. The funder index stays the same. An index
includes schedules after claims or revocation. Tranche creation records both a
funder and an authorizing beneficiary, so the tranche ID appears under each
respective address in the funder and beneficiary indexes. On upgraded
deployments, the queries read pre-index schedules from their original records.
