# Vesting Contract Reference

- **Crate**: `crates/vesting`
- **Package**: `soroban-forge-vesting`
- **Client**: `SorobanForgeVestingClient`
- **Contract Type**: `Vesting`
- **Trait**: `SorobanForgeVesting`

---

## Overview

The Vesting contract manages token release schedules over time for employees, founders, advisors, and ecosystem participants. The contract supports two distinct schedule models under a single identifier space:

1. **Linear Vesting (`VestingSchedule`)**: A straight-line continuous vesting curve with an optional initial lockup cliff.
2. **Tranche Vesting (`TrancheSchedule`)**: An explicit schedule of discrete unlock events (e.g. 25% at TGE, 25% after 6 months, 50% after 12 months).

Both models hold SEP-41 tokens in contract custody, release funds incrementally via `claim()`, and support introspection of unlocked balances and lifecycle status.

---

## Vesting Models & Mathematics

### 1. Linear Model

Linear schedules parameterize vesting using `cliff` and `duration` (in seconds elapsed since `start`).

