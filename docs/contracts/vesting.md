# Vesting Contract Reference

- **Crate**: `soroban-forge-vesting`
- **Wasm Target**: `wasm32v1-none`
- **Rust Client**: `SorobanForgeVestingClient`
- **TypeScript Client**: `@soroban-forge/vesting-client` (`Client`)

---

## 1. Overview & Architectural Role

The Vesting contract manages token release schedules across time. It supports two schedule structures sharing a unified monotonic ID counter and claim execution path:
1. **Linear Vesting**: Continuous accrual following an optional cliff up to a maturity duration, with support for funder revocation and beneficiary reassignment.
2. **Tranche Vesting**: Discrete unlock tables specifying exact token quantities unlocking at specific time offsets (e.g., TGE releases and periodic unlocks).

---

## 2. API Reference

### Schedule Creation

#### `create_schedule`
Creates a linear vesting schedule.
