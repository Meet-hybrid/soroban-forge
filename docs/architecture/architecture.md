# Architecture

## Overview

Soroban Forge is designed around **modularity**, **security**, and **developer experience**.

```
crates/
├── shared-utils/       # Common types, errors, storage patterns
├── test-utils/         # Harnesses and mocks
├── <contract>/         # One crate per contract
│   ├── src/
│   │   ├── lib.rs      # Contract entry point
│   │   ├── contract.rs # Contract implementation
│   │   ├── types.rs    # Domain types and events
│   │   ├── errors.rs   # Contract-specific errors
│   │   └── tests.rs    # Unit and integration tests
│   └── Cargo.toml
```

## Design Principles

- **One crate per contract**: Independent lifecycle and testing.
- **Shared crate for common logic**: Error handling, auth helpers, pagination.
- **Test-only dependencies**: Separate dev-dependencies for unit tests.
- **WASM size budget**: Target < 100KB per contract.
- **No runtime dependencies**: Only compile to `cdylib`.

## Storage Pattern

All state is accessed through explicit `env.storage().persistent()` or `env.storage().temporary()` calls. We avoid global mutable state and require well-defined TTLs.
