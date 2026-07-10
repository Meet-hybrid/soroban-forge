# Escrow Contract

Secure fund custody for buyer-seller transactions with optional arbiter dispute resolution.

## Interface

```rust
fn create_escrow(buyer, seller, arbiter, amount, timeout) -> Result<u64, ForgeError>
fn deposit(escrow_id) -> Result<(), ForgeError>
fn release(escrow_id) -> Result<(), ForgeError>
fn refund(escrow_id) -> Result<(), ForgeError>
fn cancel(escrow_id) -> Result<(), ForgeError>
fn get_status(escrow_id) -> Result<EscrowStatus, ForgeError>
```

## States

- `Pending` — Created but not funded
- `Funded` — Funds deposited
- `Completed` — Released to seller
- `Refunded` — Returned to buyer
- `Disputed` — Under arbitration
- `Cancelled` — Cancelled before funding

## WASM Budget

Target: < 100KB

## Feature Flags

- `test-utils` — enables test-only helpers
