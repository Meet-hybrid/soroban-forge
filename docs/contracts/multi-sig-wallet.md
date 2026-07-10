# Multi-Signature Wallet Contract

Multi-owner wallet with configurable approval threshold and transaction queue.

## Interface

```rust
fn submit_transaction(proposer, destination, function_name, args) -> Result<u64, ForgeError>
fn approve(tx_id, approver) -> Result<(), ForgeError>
fn revoke(tx_id, approver) -> Result<(), ForgeError>
fn execute(tx_id) -> Result<(), ForgeError>
fn get_transaction(tx_id) -> Result<Transaction, ForgeError>
fn add_owner(owner) -> Result<(), ForgeError>
fn remove_owner(owner) -> Result<(), ForgeError>
fn update_threshold(new_threshold) -> Result<(), ForgeError>
```

## States

- `Pending` — Awaiting approvals
- `Approved` — Threshold reached, ready for execution
- `Executed` — Transaction completed
- `Rejected` — Revoked or expired
- `Expired` — Timed out
