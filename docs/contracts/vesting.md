# Vesting Contract

Time-locked token release with cliff and customizable schedules.

## Interface

```rust
fn create_schedule(beneficiary, start, cliff, end, total_amount) -> Result<u64, ForgeError>
fn release(schedule_id) -> Result<i128, ForgeError>
fn revoke(schedule_id) -> Result<i128, ForgeError>
fn get_schedule(schedule_id) -> Result<VestingSchedule, ForgeError>
fn get_releasable_amount(schedule_id) -> Result<i128, ForgeError>
```

## Release Formula

Linear release between `cliff` and `end`.
