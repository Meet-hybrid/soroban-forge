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

Tests live in-crate (`crates/vesting/src/lib.rs`) and run with
`cargo test -p soroban-forge-vesting --all-targets --locked`.
