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

## Storage-layout diff

Use `soroban-forge storage-diff --source <WASM_A> --target <WASM_B>` to compare
the `contractspecv0` custom section of two local artifacts before an upgrade.
Add `--json` for a stable machine-readable report suitable for CI (schema
version `1`; all keys and arrays are emitted in deterministic order, and
classifications are counted in a fixed `classification_counts` object).

The terminal report groups every finding into three explicit classifications:

- **spec-established**: a deletion or discriminant change directly observed in
  the spec XDR;
- **potential-risk**: a field type change, field reordering, or union payload
  change whose storage impact depends on encoding and usage;
- **manual-review-required**: an added key, field, or variant that the spec
  alone cannot prove safe for existing stored data.

Each finding also carries a severity (`backward-compatible`,
`potentially-breaking`, or `breaking`). Breaking findings produce explicit
`MIGRATION WARNING` lines in terminal output and `migration_warnings` in JSON.
Following the `soroban-forge compare` convention, the command exits nonzero
when at least one breaking change is found; the report is printed first, and
diagnostics (including warnings and the limitation note) go to stderr in
`--json` mode.

**Critical limitation:** contract specs do not necessarily expose the full
internal persistent or instance storage layout. The tool compares storage-relevant
UDTs only. Absence of detected spec differences does **not** guarantee a safe
upgrade; persisted key encodings, XDR wire changes, migrations, and TTL behavior
still require manual review. The command never executes a migration or modifies
contract storage.
