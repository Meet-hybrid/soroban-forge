# Initial issue backlog (ready to paste)

This backlog mirrors the issue style used by
[soroban-budget-assert](https://github.com/Tollcraft/soroban-budget-assert)
(Stellar Wave reference repo): conventional-commit titles, a narrative
Description, a **What "done" looks like** section, **Implementation
guidelines**, **PR guidelines**, and the **📋 Before you start** verification
block. Each issue carries type labels plus exactly **one** complexity label.

Deliberately **no** `Stellar Wave` label: that campaign label is added only
after the repository is accepted by the program organizers.

Label convention (create these exact labels in GitHub):

| Label | Description (paste as-is) |
| --- | --- |
| `complexity: trivial` | `Wave: typos, small bug fixes, minor copy changes (100 pts)` |
| `complexity: medium` | `Wave: standard features or involved bug fixes (150 pts)` |
| `complexity: high` | `Wave: complex features, refactors, or new integrations (200 pts)` |
| `enhancement` | `New feature or request` |
| `bug` | `Something isn't working` |
| `documentation` | `Improvements or additions to documentation` |
| `refactor` | `A code change that neither fixes a bug nor adds a feature` |
| `test` | `Adding missing tests or correcting existing tests` |
| `chore` | `Maintenance tasks and build/CI changes` |
| `dependencies` | `Pull requests that update a dependency file` |
| `good first issue` | `Good for newcomers` |
| `Stellar Wave` | `Issues in the Stellar wave program` *(add only after acceptance)* |

Suggested issue creation order: create **11** (SDK migration) first, then **5**
and **6** (test coverage), then **1–4** (implementations), then **8–10**
(housekeeping) in parallel.

---

## Issue 11 — chore: migrate to soroban-sdk 27 and stellar-xdr 27

- **Labels:** `chore`, `dependencies`, `complexity: high`
- **Affected files:** workspace `Cargo.toml`, `Cargo.lock`, all contract crates, CI

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

---

## Issue 1 — test(escrow): audit and harden the escrow implementation

- **Labels:** `test`, `complexity: medium`
- **Affected crates:** `crates/escrow`, `crates/shared-utils`

### Description

The escrow contract was implemented (`create_escrow`, `deposit`, `release`,
`refund`, `cancel`, `get_status`) with 16 in-crate tests. This issue is an
**independent audit and hardening pass**: verify the implementation against
the documented specification, add the edge cases that are not yet covered,
and document the storage schema. Treat the existing implementation as the
baseline — seek out what is missing rather than rewriting it.

### What "done" looks like

- An independent review of the state machine
  (`Pending → Funded → Completed | Refunded | Cancelled`) and the
  deadline-based refund authorization (seller before the deadline, buyer
  after) with any defects found fixed and tested.
- Edge cases beyond the current 16 tests, for example: refund exactly at the
  deadline boundary, the invalid-transition matrix (deposit/release/refund/
  cancel from every wrong state), timestamps near `u64::MAX` (overflow
  paths), and repeated operations after terminal states.
- The negative-authorization gap (host abort on soroban-sdk 21.x) documented
  explicitly in the test module, with each invariant covered by a reachable
  error path where possible.
- The storage schema (`DataKey::Escrow(u64)` and `Count`) documented in
  `docs/contracts/escrow.md`, including an upgrade-compatibility note.
- `cargo test -p soroban-forge-escrow --all-targets --locked` and
  `make lint` pass.

### Implementation guidelines

- Read `crates/escrow/src/lib.rs` and its tests first. Do **not** change the
  public interface (`create_escrow`/`deposit`/`release`/`refund`/`cancel`/
  `get_status`) — that is a breaking change and out of scope.
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
cargo test -p soroban-forge-escrow --all-targets --locked
```

---

## Issue 2 — test(vesting): audit and harden the vesting implementation

- **Labels:** `test`, `complexity: medium`
- **Affected crates:** `crates/vesting`, `crates/shared-utils`

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

---

## Issue 3 — feat(multi-sig-wallet): implement authorization and execution

- **Labels:** `enhancement`, `complexity: medium`
- **Affected crates:** `crates/multi-sig-wallet`, `crates/shared-utils`

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

---

## Issue 4 — feat(dao-governance): implement proposal/voting state transitions

- **Labels:** `enhancement`, `complexity: medium`
- **Affected crates:** `crates/dao-governance`, `crates/shared-utils`

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

---

## Issue 5 — test: add in-process integration tests for every public contract method

- **Labels:** `test`, `complexity: medium`
- **Affected crates:** all contract crates, `crates/test-utils`

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

---

## Issue 6 — test(cli): add smoke tests and CI coverage

- **Labels:** `test`, `complexity: medium`
- **Affected crates:** `crates/cli`, `.github/workflows/ci.yml`

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

---

## Issue 7 — test: add security invariant tests across contracts

- **Labels:** `test`, `complexity: high`
- **Affected crates:** all contract crates, `crates/test-utils`

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

---

## Issue 8 — chore: cut the v0.1.0 release and changelog

- **Labels:** `chore`, `complexity: trivial`, `good first issue`
- **Affected files:** `CHANGELOG.md`, `Cargo.toml`, `.github/workflows/release.yml`

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

---

## Issue 9 — chore: harden WASM-size and dependency audit checks

- **Labels:** `chore`, `complexity: medium`
- **Affected files:** `.github/workflows/ci.yml`, `Makefile`, `deny.toml` (new)

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

---

## Issue 10 — docs: document deployment and storage compatibility

- **Labels:** `documentation`, `complexity: trivial`, `good first issue`
- **Affected files:** `docs/tutorials/deploying-to-testnet.md`, `docs/architecture/storage-patterns.md`, `docs/contracts/index.md`

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

---

## Dependabot triage (do this before/while creating issues)

Your PR list is currently 100% dependabot. Program reviewers will look at the
Issues tab, not just PRs — real, labeled issues are what make the repo look
maintainer-ready. While creating the backlog:

1. **Do not merge #2/#5** (soroban-sdk → 27.0.4) or #1 (rand 0.10) blindly —
   those are covered by **Issue 11** (migration) and should land as one
   deliberate PR with re-measured artifacts.
2. **Close or let expire** the small JS bumps (#3, #4, #6–#11) if they are
   irrelevant to the core Rust workspace, or configure dependabot to group
   them so they stop dominating the PR list.
3. Reopen `dependabot` branches under a migration branch only as part of
   Issue 11.

## Backlog ordering suggestion

Create **11** (migration) first — it unblocks the workspace health story.
Then **5** and **6** (test coverage) alongside the implementation PRs (1–4)
so those PRs land on a CI-visible test surface. **8–10** can be created and
taken in parallel at any time. **7** (security invariants) is most valuable
after 1–4 land.

> Note: the CLI is already wired into the workspace, and the escrow (16 tests)
> and vesting (21 tests) contracts are implemented ahead of issues 1, 2, and 6
> — those issues remain valid as regression/coverage tasks, and their bodies
> reflect the current baseline. Issue 2 (vesting) is scoped as an audit and
> hardening pass over the existing implementation.
