#!/usr/bin/env bash
# Create the initial maintainer issue backlog on GitHub.
#
# Usage:
#   ./scripts/create-issues.sh                  # dry run: print what would be created
#   ./scripts/create-issues.sh --apply          # create the issues (skips existing titles)
#   ./scripts/create-issues.sh --apply OWNER/REPO
#
# Safety:
#   - Dry run by default; nothing is created without --apply.
#   - Never applies the `Stellar Wave` label (added only after program acceptance).
#   - Skips any title that already exists on the repository.
set -euo pipefail

APPLY=false
[[ "${1:-}" == "--apply" ]] && APPLY=true && shift
REPO="${1:-${GITHUB_REPOSITORY:-Meet-hybrid/soroban-forge}}"

if [[ "$APPLY" == true ]] && ! gh auth status >/dev/null 2>&1; then
  echo "error: gh is not authenticated" >&2
  exit 1
fi

existing_titles() {
  gh issue list --repo "$REPO" --state all --limit 200 --json title --jq '.[].title' 2>/dev/null || true
}

create_issue() {
  local title="$1"
  local labels="$2"
  local body_file="$3"

  if [[ "$APPLY" == false ]]; then
    echo "would create: $title"
    echo "  labels:     $labels"
    return
  fi

  if existing_titles | grep -Fxq "$title"; then
    echo "skip (exists): $title"
    return
  fi

  gh issue create --repo "$REPO" \
    --title "$title" \
    --label "$labels" \
    --body-file "$body_file"
}

BODY_DIR="$(mktemp -d)"
trap 'rm -rf "$BODY_DIR"' EXIT

echo "Target repository: $REPO"

# ---------------------------------------------------------------- Issue 11
cat > "$BODY_DIR/i11.md" <<'EOF'
### Description

Dependabot PRs #2 and #5 propose a six-major-version jump: soroban-sdk
21.5.1 → 27.0.4 (and soroban-sdk-macros with it). The workspace pins
`=21.5.1`, and a blind bump will not compile: contract macros, the XDR layer,
and error handling moved several majors. This is a platform migration, not a
routine bump, and it matters more here than in most projects: these contracts
are measured artifacts, and the upgrade must not silently change on-chain
behavior.

### What "done" looks like

- `cargo build --workspace --all-targets --locked` and
  `cargo test --workspace --all-targets --locked` pass against soroban-sdk 27.x.
- All six contract crates compile for `wasm32-unknown-unknown` and the WASM
  Size Check stays green; before/after sizes are stated in the PR.
- Any public interface or storage-format changes are documented in
  `CHANGELOG.md` and `docs/contracts/`.
- The dependency bump is done as one deliberate PR, not via the raw dependabot
  branches.

### Implementation guidelines

- Create a branch: `git checkout -b chore/migrate-soroban-sdk-27`.
- Reproduce the failures first: merge `main` into one of the dependabot
  branches and run `cargo clippy --workspace --all-targets --locked`.
- Verify rather than assume: after re-pointing imports, confirm the storage
  and error types still serialize identically before trusting a green build.
- Update `Cargo.lock` intentionally; keep `--locked` CI working.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.
- State the before/after WASM sizes and any behavior changes explicitly.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```
EOF
create_issue \
  "chore: migrate to soroban-sdk 27 and stellar-xdr 27" \
  "chore,dependencies,complexity: high" \
  "$BODY_DIR/i11.md"

# ------------------------------------------------------------------ Issue 1
cat > "$BODY_DIR/i01.md" <<'EOF'
### Description

`crates/escrow` currently ships only its public interface and storage types.
Implement `create_escrow`, `deposit`, `release`, `refund`, `cancel`, and
`get_status` so the contract enforces the state machine
`Pending → Funded → Completed | Refunded | Cancelled`, with `Disputed`
reserved for a follow-up dispute method. The reference implementation pattern
(authorization via `require_auth`, `ForgeError`-typed failures, in-crate
`Env`-based tests) is already demonstrated by the code that landed for this
issue — treat it as the baseline to keep consistent.

### What "done" looks like

- All six public methods implemented with `ForgeError`-typed failures.
- Invalid transitions return the appropriate error, never panic.
- Unit tests cover every documented transition, missing-escrow `NotFound`,
  and deadline expiry (`complexity: high` reflects the dispute/timeout logic).
- WASM build stays under the size budget; `make lint` and `make test` pass.

### Implementation guidelines

- Match the storage pattern: instance storage keyed by `DataKey::Escrow(u64)`
  plus a monotonic `Count`.
- Keep authorization on specific addresses via `require_auth`; the SDK has no
  caller-identity API, so design "buyer OR seller" flows by state, not caller.
- Reuse `crates/test-utils` (`new_env`, `TestAccounts`) in the test module.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p soroban-forge-escrow --all-targets --locked
```
EOF
create_issue \
  "feat(escrow): implement and test the escrow lifecycle" \
  "enhancement,complexity: high" \
  "$BODY_DIR/i01.md"

# ------------------------------------------------------------------ Issue 2
cat > "$BODY_DIR/i02.md" <<'EOF'
### Description

The vesting contract was implemented (`create_schedule`, `claim`, `claimable`,
`get_status`) with 21 in-crate tests. This issue is an **independent audit
and hardening pass**: verify the implementation against the documented
specification, add the edge cases that are not yet covered, and document the
storage schema. Treat the existing implementation as the baseline — seek out
what is missing rather than rewriting it.

### What "done" looks like

- An independent review of the release math (floor division, cliff/duration
  boundaries, `cliff == duration`, the never-overpay guarantee) with any
  defects found fixed and tested.
- Edge cases beyond the current 21 tests, for example: timestamps/amounts
  near `u64::MAX` / `i128::MAX` (overflow paths), many interleaved partial
  claims, and repeated `claimable` calls between claims.
- The negative-authorization gap (host abort on soroban-sdk 21.x) documented
  explicitly in the test module, with each invariant covered by a reachable
  error path where possible.
- The storage schema (`DataKey::Schedule(u64)` and `Count`) documented in
  `docs/contracts/vesting.md`, including an upgrade-compatibility note.
- `cargo test -p soroban-forge-vesting --all-targets --locked` and
  `make lint` pass.

### Implementation guidelines

- Read `crates/vesting/src/lib.rs` and its tests first. Do **not** change the
  public interface (`create_schedule`/`claim`/`claimable`/`get_status`) —
  that is a breaking change and out of scope.
- Reuse `crates/test-utils` rather than duplicating helpers.
- Follow the house test style (`setup!` macro, `try_<method>` client
  variants, `.unwrap_err().unwrap()`).

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p soroban-forge-vesting --all-targets --locked
```
EOF
create_issue \
  "test(vesting): audit and harden the vesting implementation" \
  "test,complexity: medium" \
  "$BODY_DIR/i02.md"

# ------------------------------------------------------------------ Issue 3
cat > "$BODY_DIR/i03.md" <<'EOF'
### Description

Implement `submit`, `confirm`, and `execute` so a transaction only executes
once the configured owner threshold is met. Owner-set and threshold storage,
duplicate-confirmation rejection, and execute-once semantics are the core
invariants to get right.

### What "done" looks like

- Confirmation below threshold keeps the transaction `Pending`.
- Executing before the threshold, or twice, returns the documented error.
- Duplicate confirmations and non-owner confirmations are rejected.
- Unit tests cover the threshold boundary; `make lint` and `make test` pass.

### Implementation guidelines

- The payload is opaque `Bytes`; no external-call dispatch in this iteration.
- Match the escrow test style (`env.register_contract` + generated client).

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p soroban-forge-multi-sig-wallet --all-targets --locked
```
EOF
create_issue \
  "feat(multi-sig-wallet): implement authorization and execution" \
  "enhancement,complexity: medium" \
  "$BODY_DIR/i03.md"

# ------------------------------------------------------------------ Issue 4
cat > "$BODY_DIR/i04.md" <<'EOF'
### Description

Implement `propose`, `vote`, and `execute` with the
`Active → Succeeded | Defeated` lifecycle and vote tallying: id assignment,
single-vote-per-address enforcement, deadline enforcement, and execute-only-
after-deadline semantics.

### What "done" looks like

- Votes cast after the voting deadline are rejected.
- Double voting by the same address is rejected.
- State transitions are exactly `Active → Succeeded | Defeated`.
- Unit tests cover deadline, double-vote, and tally correctness; `make lint`
  and `make test` pass.

### Implementation guidelines

- The action payload is opaque `Bytes`; execution just finalizes state.
- Reuse the escrow storage/error pattern.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p soroban-forge-dao-governance --all-targets --locked
```
EOF
create_issue \
  "feat(dao-governance): implement proposal/voting state transitions" \
  "enhancement,complexity: medium" \
  "$BODY_DIR/i04.md"

# ------------------------------------------------------------------ Issue 5
cat > "$BODY_DIR/i05.md" <<'EOF'
### Description

Add Soroban `Env`-based in-crate test modules (the standard
`env.register_contract` + generated `*Client` pattern) exercising every public
method of every contract — happy paths, error paths, and state-machine
transitions. The escrow crate shows the house style; extend it to the other
five contracts.

### What "done" looks like

- `cargo test --workspace --all-targets --locked` runs a growing, nonzero
  suite with every public method invoked at least once.
- Shared helpers are reused from `crates/test-utils`, not duplicated.
- CI `Test` job output shows the suite executing.

### Implementation guidelines

- Use `try_<method>` client variants to assert exact `ForgeError` codes.
- Note: negative authorization tests are not runnable in-process on
  soroban-sdk 21.x (host panics abort); document any gaps in the test module
  rather than leaving silent holes.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```
EOF
create_issue \
  "test: add in-process integration tests for every public contract method" \
  "test,complexity: medium" \
  "$BODY_DIR/i05.md"

# ------------------------------------------------------------------ Issue 6
cat > "$BODY_DIR/i06.md" <<'EOF'
### Description

The CLI is already wired into the workspace and builds
(`soroban-forge --help` runs; clippy-clean). This issue adds the missing
regression coverage: unit tests for argument parsing and command dispatch, a
smoke test that executes the built binary, and an explicit CI step that runs
`soroban-forge --help`.

### What "done" looks like

- `cargo test -p soroban-forge-cli --all-targets --locked` runs arg-parsing
  and smoke tests.
- A CI step runs `soroban-forge --help` and fails on a nonzero exit.
- `make lint` and `make test` pass.

### Implementation guidelines

- Test through `std::process::Command` (or `assert_cmd`) against the built
  binary; assert the subcommand list is present.
- Add the CI step to the existing `test` job.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p soroban-forge-cli --all-targets --locked
```
EOF
create_issue \
  "test(cli): add smoke tests and CI coverage" \
  "test,complexity: medium" \
  "$BODY_DIR/i06.md"

# ------------------------------------------------------------------ Issue 7
cat > "$BODY_DIR/i07.md" <<'EOF'
### Description

Add a shared security-test suite asserting invariants across escrow, vesting,
multisig, and governance: authorization enforcement, no double-spend or
over-claim, integer overflow resistance, and timestamp edge cases.

### What "done" looks like

- Each contract has at least three security-focused tests.
- Tests fail loudly when an authorization or overflow check is removed.
- `make test` passes with the full suite enabled.

### Implementation guidelines

- Parameterize with `Env` auths; where the SDK's non-unwinding panics prevent
  in-process negative-auth tests, document the gap and cover the invariant by
  a reachable error path instead.
- Cover `checked_add`-style overflow paths that the implementation relies on.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```
EOF
create_issue \
  "test: add security invariant tests across contracts" \
  "test,complexity: high" \
  "$BODY_DIR/i07.md"

# ------------------------------------------------------------------ Issue 8
cat > "$BODY_DIR/i08.md" <<'EOF'
### Description

Cut `v0.1.0`: complete `CHANGELOG.md` from existing history, verify the
release workflow tags and drafts a GitHub release, and document the release
checklist for maintainers.

### What "done" looks like

- `CHANGELOG.md` covers all merged user-facing changes.
- `cargo metadata --no-deps --format-version 1` shows one consistent `0.1.0`
  across all workspace members.
- Release workflow runs green on a dry-run tag.

### Implementation guidelines

- Follow the Keep a Changelog format already started in `CHANGELOG.md`.
- Do not push tags during the PR; the workflow handles tagging on merge.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```
EOF
create_issue \
  "chore: cut the v0.1.0 release and changelog" \
  "chore,complexity: trivial,good first issue" \
  "$BODY_DIR/i08.md"

# ------------------------------------------------------------------ Issue 9
cat > "$BODY_DIR/i09.md" <<'EOF'
### Description

Harden CI: keep the WASM Size Check failing when no artifacts are produced,
add per-contract size budgets, and add a dependency policy (`cargo-deny` or
`cargo audit` config) with explicit allow/deny rules.

### What "done" looks like

- WASM Size Check fails if no `.wasm` files are produced.
- The chosen audit tool passes with a pinned, documented policy.
- Size budgets and audit commands are documented in `docs/`.

### Implementation guidelines

- Baseline: the size job already builds the six contract crates with
  `--target wasm32-unknown-unknown`; extend, don't rewrite.
- Keep the new contract-crate list in sync with workspace members (see the
  comment in `ci.yml`).

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```
EOF
create_issue \
  "chore: harden WASM-size and dependency audit checks" \
  "chore,complexity: medium" \
  "$BODY_DIR/i09.md"

# ----------------------------------------------------------------- Issue 10
cat > "$BODY_DIR/i10.md" <<'EOF'
### Description

Write deployment guidance (testnet → mainnet, account/key setup, verifying
deployed WASM) and document each contract's storage schema plus what a
storage-breaking upgrade looks like.

### What "done" looks like

- Every contract links to its storage layout and deploy commands.
- A short "upgrade compatibility" section explains when storage changes break
  upgrades.
- No dead links in the edited docs.

### Implementation guidelines

- Use the actual `stellar contract` commands that work with the current
  contract set; verify, don't copy from memory.
- Cross-link the storage keys defined in each contract crate.

### PR guidelines

- Get assigned before starting.
- PR description must include: `Closes #<this issue>`.

### 📋 Before you start

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```
EOF
create_issue \
  "docs: document deployment and storage compatibility" \
  "documentation,complexity: trivial,good first issue" \
  "$BODY_DIR/i10.md"

if [[ "$APPLY" == false ]]; then
  echo
  echo "Dry run complete. Re-run with --apply to create the issues."
  echo "Target repository: $REPO"
else
  echo
  echo "Done. Verify with: gh issue list --repo $REPO"
fi
