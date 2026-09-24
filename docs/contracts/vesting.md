# Vesting Contract

Time-locked token release with a cliff and linear release.

## Interface

```rust
fn create_schedule(beneficiary, token, total_amount, cliff, duration) -> Result<u64, ForgeError>
fn claim(schedule_id) -> Result<i128, ForgeError>
fn claimable(schedule_id) -> Result<i128, ForgeError>
fn get_status(schedule_id) -> Result<VestingStatus, ForgeError>
fn touch_ttl(schedule_id) -> Result<(), ForgeError>
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
- `touch_ttl` is permissionless (keeper entrypoint).

## Storage and TTL

| Key | Storage | Contents | TTL handling |
|---|---|---|---|
| `DataKey::Schedule(u64)` | **persistent**, one entry per schedule | `VestingSchedule` | Extended on `create_schedule`, on every `claim` that writes, and by `touch_ttl` |
| `DataKey::Count` | instance | `u64` last-issued id | Lives with the contract instance (same as escrow) |

This follows the escrow layout (`crates/escrow`): per-record persistent
entries so the byte budget scales per schedule and a schedule's lifetime is
independent of the contract instance's, with only the id counter in instance
storage.

TTL extension uses `extend_ttl(key, BUMP_THRESHOLD, BUMP_AMOUNT)` with the
escrow constants: `BUMP_AMOUNT = 30 days` (518,400 ledgers at ~5 s per
ledger) and `BUMP_THRESHOLD = 29 days`. An entry is only extended once its
remaining TTL falls below the threshold, so a touch on a fresh entry costs
almost nothing.

- **State-changing calls extend the TTL.** `create_schedule` and a `claim`
  that pays out a non-zero amount write the schedule and then extend its
  TTL. A `claim` that returns `0` writes nothing and does not extend.
- **Read-only calls never extend the TTL.** This covers `claimable` and
  `get_status`.
- **`touch_ttl(schedule_id)`** is a permissionless keeper entrypoint (no
  authorization). It extends an existing schedule's TTL and changes nothing
  else: no claimable amount, field or status changes. An unknown id returns
  `ForgeError::NotFound`.

Vesting windows are often longer than 30 days. A schedule that goes more
than 30 days without a payout or a `touch_ttl` falls out of its TTL. Keepers
should call `touch_ttl` within the 30-day horizon. Once a schedule's TTL
runs out, it must be restored (Soroban persistent-entry restoration) before
it can be used again. Nothing is deleted.

Missing ids: every entrypoint that takes a `schedule_id` returns
`ForgeError::NotFound` for an unknown id. The lookup is a plain
`persistent().get` that never writes, so a failed lookup creates no storage
entry and does not advance the id counter.

## Upgrade compatibility

Earlier builds stored schedules in **instance** storage under the same
`DataKey::Schedule(u64)` key. Persistent and instance storage are separate
namespaces, so this build cannot read schedules written by an earlier build.
**No migration path is provided, and none is needed:**

- The vesting contract has never been deployed. It is built for provenance
  (`scripts/provenance.sh`) but is not deployed by `scripts/demo-testnet.sh`
  or `scripts/deploy-mainnet.sh`. The project is testnet-only (see
  [Known Limitations](../KNOWN-LIMITATIONS.md)).
- The contract has no upgrade entrypoint (`update_current_contract_wasm`).
  A deployed instance can never run new code against old storage. A new
  version is always a new deployment with empty storage.

The id counter (`DataKey::Count`) stays in instance storage with the same key
and encoding.

If an upgrade entrypoint is ever added, any schedules still in instance
storage would need an explicit, one-time move into persistent entries before
this layout could read them.

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
