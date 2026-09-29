
# Escrow Contract

## Keeper Integration Guide

### Permissionless Expiry Refunds

The escrow contract now supports permissionless expiry refunds through the `refund_expired` entrypoint. This allows keeper infrastructure to automatically settle expired escrows without requiring party authorization.

#### Boundary Semantics

- **Deadline Check**: Refunds are only permitted after the exact deadline timestamp.
- **State Check**: Only escrows in `Active` state are refundable. Disputed escrows remain protected.
- **Terminal States**: Escrows already in `Refunded` or `Resolved` states cannot be refunded again.

#### Keeper Workflow

1. **Monitor Escrows**: Keepers should track escrow deadlines and states.
2. **Trigger Refunds**: Call `refund_expired` with the escrow ID after the deadline.
3. **Event Listening**: Listen for `RefundExpired` events to confirm successful settlements.

#### Anti-Griefing Guarantees

- **Dispute Protection**: Escrows in dispute cannot be refunded by third parties.
- **State Integrity**: The contract maintains strict state transitions to prevent reentrancy or double-spending.
- **Boundary Safety**: The exact-at-deadline boundary is enforced to prevent front-running.

#### Example

```rust
// After deadline has passed
client.refund_expired(&env, escrow_id, keeper_address);
```

#### Event Structure

The `RefundExpired` event includes:
- `escrow_id`: The unique identifier of the escrow
- `buyer`: The recipient of the refund
- `amount`: The full escrow amount being refunded
- `timestamp`: When the refund was processed

This provides clear auditability for keeper operations.
