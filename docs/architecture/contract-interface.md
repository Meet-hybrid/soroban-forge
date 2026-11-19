# Architecture — Contract Interfaces

Every contract exposes a stable interface using `#[contract]` and `#[contractimpl]`.

Public methods follow the pattern: `verb_noun` (e.g., `create_escrow`, `release_funds`, `cancel_subscription`).

All inputs are validated at the boundary. Errors use the shared `ForgeError` enum.

## Contract-spec compatibility

Use `soroban-forge compare <old.wasm> <new.wasm>` to extract the `contractspecv0` sections and compare functions, events, and user-defined types from two local WASM artifacts. Add `--json` for a stable machine-readable report.

The report distinguishes:

- **additive** changes: added functions, events, or types;
- **source-breaking** changes: removed or renamed functions and function signature/type changes;
- **wire/storage-risky** changes: event or user-defined type changes that can affect encoding or persisted application data.

Exit status is nonzero for source-breaking and wire/storage-risky changes. Identical specs emit an empty change list.

This check proves neither XDR compatibility nor storage compatibility. XDR wire format can change independently of the contract spec, and storage compatibility depends on key layout, enum discriminants, serialized custom types, migration logic, and TTL behavior. Review XDR schemas and persisted-key layouts separately; no automatic upgrade or network deployment is performed.
