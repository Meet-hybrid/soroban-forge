# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `docs/maintainers/release-checklist.md` — maintainer release procedure.

### In progress
- The vesting, multi-sig-wallet, dao-governance, subscription-payments, and
  marketplace-royalties contracts ship public interfaces and storage types
  only; implementations are tracked in the
  [issue backlog](docs/maintainers/issue-backlog.md).

## [0.1.0] - 2026-08-04

### Added
- Virtual Cargo workspace of nine crates with shared `workspace.package`
  metadata (version, edition, rust-version, license, repository).
- `shared-utils` crate: `ForgeError`, `StorageEntry` storage pattern, and
  shared types (`Party`, `TimeBounds`, `PaginatedResult`, `PaginationCursor`).
- `test-utils` crate: Soroban `Env` test harness (`new_env`) and mock account
  sets (`TestAccounts`).
- **Escrow contract** (implemented): `create_escrow`, `deposit`, `release`,
  `refund`, `cancel`, and `get_status` with `require_auth` authorization,
  deadline-based refunds, checked arithmetic, and 16 in-crate `Env`-based
  tests covering the full state machine and error paths.
- Public interfaces and storage types for the vesting, multi-sig-wallet,
  dao-governance, subscription-payments, and marketplace-royalties contracts
  (implementations land via the issue backlog).
- Developer CLI (`soroban-forge-cli`) with `build`, `test`, `lint`, and
  `deploy` subcommands, wired into the workspace.
- Language bindings and examples under `packages/`: TypeScript SDK, Next.js
  reference application, and deployment templates (not covered by workspace
  CI).
- Continuous integration (`.github/workflows/ci.yml`): Rustfmt, Clippy
  (`-D warnings`), Build, Test, Security Audit, and a WASM Size Check with a
  per-contract size budget; a `Release` workflow that builds WASM artifacts
  and drafts a GitHub release on `v*` tags.
- Maintainer tooling and docs: 11-issue backlog, application packet, issue
  triage guide, program-readiness notes, and reusable `scripts/` for label
  and issue creation.
- MIT OR Apache-2.0 dual license, security policy, and contribution
  guidelines.

### Changed
- Committed `Cargo.lock` for reproducible builds; all CI cargo commands use
  `--locked`.
- Replaced raw `Val` fields used under `#[contracttype]` with serializable
  `Bytes` (`StorageEntry`, `PaginatedResult`, `WalletTx`, `Proposal`).
- Corrected repository URLs and the CI badge from `teachlink` to
  `Meet-hybrid`.
- Pinned the toolchain to Rust 1.96.0 (`rust-toolchain.toml` and CI): the
  pinned soroban-sdk 21.x does not compile on newer stable toolchains.
- Main branch protection: one approving review required (repository
  administrators exempt), six required status checks, linear history.
- Dependabot updates grouped to reduce pull-request noise; risky major
  bumps (soroban-sdk, rand) intentionally ungrouped for triage.

### Fixed
- CI WASM-size check built no WASM artifacts silently; it now builds with
  `--target wasm32-unknown-unknown` and fails when no artifacts are produced.
- Escrow WASM build failure (missing `#![no_std]`).
- `audit.yml` invalid YAML and missing `checks: write` permission.
- CLI crate did not compile and was excluded from the workspace; rewired,
  clippy-clean, and smoke-tested.

### Security
- `unsafe` code is forbidden workspace-wide.
- `cargo audit` runs in CI. `RUSTSEC-2026-0009` (`time` 0.3.44) is a
  transitive dependency of the pinned soroban-sdk 21.x chain, is not compiled
  into the workspace graph, and is ignored in CI with rationale until the
  soroban-sdk 27 migration (issue #14) removes it.
