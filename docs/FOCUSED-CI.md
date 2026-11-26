# Focused Crate CI Validation

The [Focused Crate Checks](../.github/workflows/focused-checks.yml) workflow detects which crates and client dependencies changed on a push or pull request and runs the smallest relevant `cargo fmt`, `cargo test`, `cargo clippy`, `shared-utils`, and generated-client checks.

The full [CI](../.github/workflows/ci.yml) workflow remains intact for repository-wide verification and branch-protection status checks; it also runs workflow linting and path-matrix regression tests on every PR inside the `repo-hygiene` job.

## How Path-to-Matrix Resolution Works

[detect-changed-crates.py](../scripts/ci/detect-changed-crates.py) reads `[workspace].members` and every member `Cargo.toml` directly using Python's `tomllib`, builds the workspace dependency graph, and classifies changed paths as follows:

| Changed Path | Focused Crate Matrix (`fmt`, `test`, `clippy`) | `shared-utils` & Error Registry | Generated Clients & TS SDK | Full-Workspace Fallback |
| --- | --- | --- | --- | --- |
| `crates/<contract>/**` (any of the 6 `cdylib` contracts) | `<contract>` + `soroban-forge-composition-tests` | No | Yes | No |
| `crates/shared-utils/**` | All 6 contracts + `soroban-forge-test-utils` + `soroban-forge-composition-tests` (excludes `soroban-forge-cli`) | Yes | Yes | No |
| `crates/test-utils/**` | `soroban-forge-test-utils` + all 6 contracts + `soroban-forge-composition-tests` | No | No | No |
| `crates/cli/**`, `templates/**` | `soroban-forge-cli` | No | No | No |
| `crates/composition-tests/**` | `soroban-forge-composition-tests` | No | No | No |
| `errors.json`, `scripts/generate-error-registry.sh`, `scripts/check-error-registry-fresh.sh`, `packages/typescript-sdk/src/errors.generated.ts` | _(none — handled by `shared-utils-checks`)_ | Yes | Yes | No |
| `packages/typescript-sdk/**`, `packages/*-client/**`, `scripts/generate-clients.sh` | _(none)_ | No | Yes | No |
| `indexer/fixtures/<contract>-events.json` | `<contract>` + `soroban-forge-composition-tests` | No | Yes | No |
| `Cargo.lock`, `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `Makefile`, `rustfmt.toml`, `.rustfmt.toml`, `clippy.toml`, `.clippy.toml`, `.cargo/**`, `.github/workflows/**`, `scripts/ci/**`, unrecognized `crates/<dir>/**` | _(handled by `full-workspace`)_ | Yes | Yes | **Yes** |
| Docs / unrelated files (`README.md`, `docs/**`, `CHANGELOG.md`, `packages/nextjs-example/**`) | _(skipped)_ | No | No | No |

### Notes on Dependencies & Fallbacks

- **Reverse dependencies come from `Cargo.toml`**: Any crate that depends (normal, build, or dev) on a changed crate is automatically added to the focused test matrix.
- **Generated-client impact tracks `cdylib` reachability**: Only changes to `cdylib` contract crates, their normal dependencies (`shared-utils`), generated client packages, SDK sources/scripts, error-registry files, indexer fixtures, or fallback root files trigger the `generated-clients` job (`npm run verify:generated`, `npm test`, `npm run build`). Dev-only utilities like `crates/test-utils` do not trigger client regeneration.
- **Safe fallback on uncertain diffs**: Initial branch pushes (`before == 0000...`), unavailable base SHAs, `workflow_dispatch` with `full_workspace: true`, or unrecognized paths under `crates/` automatically fall back to the full workspace check (`cargo fmt --all -- --check`, `cargo test --workspace --all-targets --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `bash scripts/check-error-registry-fresh.sh`, and generated-client verification).

## Running Workflow Lint & Path-Matrix Tests Locally

Run the static workflow linter across all `.github/workflows/*.yml` files (also invokes `actionlint` if installed on `PATH`):

```bash
python3 scripts/ci/lint-workflows.py
```

Run the path-matrix and workflow-lint regression test suite:

```bash
python3 scripts/ci/test-path-matrix.py
```

Preview the validation plan for any list of changed paths or git revision range:

```bash
# Preview specific changed files
python3 scripts/ci/detect-changed-crates.py --files crates/escrow/src/lib.rs

# Preview against a base branch using PR merge-base diffing
python3 scripts/ci/detect-changed-crates.py --base origin/develop --head HEAD --diff-mode three-dot
```
