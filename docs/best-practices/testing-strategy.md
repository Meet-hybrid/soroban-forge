# Testing Strategy

## Unit Tests

Each contract should have inline unit tests for core business logic.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // ... tests
}
```

## Integration Tests

Located in `tests/integration/`. Spin up a soroban environment and interact with deployed WASM.

## Fuzzing

Consider using `cargo-fuzz` for parsing inputs and complex state machines.

## Coverage

Target >= 90% line coverage for stable contracts.

## CI Gates

All PRs must pass:
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
