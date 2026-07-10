# Subscription Payments Contract

Recurring payment plans with plan management, billing, and pause/resume.

## Interface

```rust
fn create_plan(creator, name, amount, interval, interval_count) -> Result<u64, ForgeError>
fn subscribe(plan_id, subscriber) -> Result<u64, ForgeError>
fn charge(subscription_id) -> Result<i128, ForgeError>
fn cancel(subscription_id) -> Result<(), ForgeError>
fn pause(plan_id) -> Result<(), ForgeError>
fn resume(plan_id) -> Result<(), ForgeError>
```

## Plan States

- `Active`
- `Paused`
- `Deprecated`

## Subscription States

- `Active`
- `Paused`
- `Canceled`
- `Expired`
