# Smart Contract Security Best Practices

## Input Validation

Validate all inputs at the contract boundary. Never trust caller-supplied values.

```rust
fn ensure_non_negative(amount: i128) -> Result<(), ForgeError> {
    if amount < 0 {
        return Err(ForgeError::InvalidInput);
    }
    Ok(())
}
```

## Authorization

Every public function must perform explicit authorization checks using `require_auth` or `require_auth_for_all_non_creator_auths`.

## Reentrancy

Avoid calling external contracts in the middle of a state transition. Use checks-effects-interactions pattern.

## Overflow Protection

Rust's `checked_*` family of operations should be used for all arithmetic. Never use `wrapping_*` or `saturating_*` in contract logic unless explicitly documented.

## Upgrade Safety

If a contract supports upgrades, document the exact migration path and limit the set of callers who can trigger the upgrade.

## Secrets

Never embed private keys, admin secrets, or environment variables in the WASM binary. All configuration must be provided at deploy time or via instance storage.

## Audits

Contracts in `crates/` are expected to undergo independent security audits before being marked stable in a release.
