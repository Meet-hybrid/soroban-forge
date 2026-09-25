# Vesting Contract

Time-locked token release with a cliff and linear release.

## Interface

```rust
fn create_schedule(beneficiary, token, total_amount, cliff, duration) -> Result<u64, ForgeError>
fn claim(schedule_id) -> Result<i128, ForgeError>
fn claimable(schedule_id) -> Result<i128, ForgeError>
fn get_status(schedule_id) -> Result<VestingStatus, ForgeError>
```

## Timing

`cliff` and `duration` are **durations in seconds measured from the schedule
start** (the ledger timestamp recorded at creation):

- `start + cliff` — claims become possible;
- `start + duration` — the schedule is fully vested.

Validation at creation: `total_amount > 0`, `duration > 0`, `cliff <= duration`
(failures return `ForgeError::InvalidInput`).

## Release Formula

The vested amount at ledger time `t` is:

```text
0                                            when t < start + cliff
total_amount * (t - (start + cliff)) / (duration - cliff)   otherwise, floored
total_amount                                 when t >= start + duration
```

Integer (floor) division means a claim never rounds up, so repeated claims can
never overpay or underpay: `claim` returns exactly `vested - claimed`, or `0`
when nothing is claimable.

## Status

Derived from ledger time and the claimed amount (always current between
claims): `Locked` before the cliff, `Vesting` after the cliff, `Completed`
once fully claimed. `Revoked` is reserved for a follow-up revocation method.

## Authorization

- `create_schedule` requires the beneficiary.
- `claim` requires the beneficiary.
- `claimable` and `get_status` are read-only views.

## Settlement

The contract custodies the SEP-41 token configured on the schedule, and
`claim` settles through it:

- the newly claimable amount is transferred from the contract to the
  beneficiary **before** the schedule is written (escrow's
  transfer-before-state pattern); a failed transfer returns
  `ForgeError::TokenTransferFailed` with `claimed` and `status` unchanged;
- zero-claim calls (before the cliff, or nothing newly vested) return `0`
  and issue **no** token transfer;
- floor-division residue is never lost: it stays claimable between claims
  and is paid out by the final claim.

## Storage Layout & Upgrade Compatibility

The vesting contract uses instance storage partitioned into two distinct keys:

```rust
enum DataKey {
    /// Holds a `VestingSchedule` record keyed by monotonic schedule id.
    Schedule(u64),
    /// Monotonic id counter (`u64`), initialized at 0 and incremented on each schedule creation.
    Count,
}
```

### Storage Model

- **`DataKey::Count`**: Stores a single `u64` representing the highest allocated schedule id. Next id allocation uses checked addition (`checked_add(1)`), returning `ForgeError::ArithmeticOverflow` on counter saturation.
- **`DataKey::Schedule(u64)`**: Stores the `VestingSchedule` struct containing beneficiary, token address, total amount, start timestamp, cliff duration, total duration, claimed amount, and derived lifecycle status.
- Instance storage lifetime is bound to the contract instance. In environments with storage TTL expiration, the contract instance TTL must be maintained to prevent storage eviction.

### Upgrade Compatibility

- **Key Segregation**: Because `DataKey::Schedule(u64)` uses a tuple variant and `DataKey::Count` is a unit variant, keys occupy non-overlapping namespaces within instance storage.
- **Record Schema Evolution**:
  - The `VestingSchedule` struct is serialized via Soroban's `#[contracttype]`. Adding new fields to `VestingSchedule` in future contract versions must maintain backwards deserialization compatibility (e.g. using `Option<T>` for newly introduced optional fields, or migrating storage records upon upgrade).
  - Storage key enums must preserve existing discriminant ordering if extended (e.g. adding new key variants for administrative roles or persistent storage migration).
- **Storage Tier Migration**: If migrating from instance storage to persistent storage with per-schedule TTL management (mirroring the Escrow contract architecture), `DataKey::Schedule(u64)` entries can be migrated to persistent storage while retaining `DataKey::Count` in instance storage.

Tests live in-crate (`crates/vesting/src/lib.rs`) and run with
`cargo test -p soroban-forge-vesting --all-targets --locked`.
