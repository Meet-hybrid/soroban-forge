# Maintainer application packet — Soroban Forge

> **Draft for submission.** Every factual claim in this packet has been
> verified against the repository as of 2026-08-04. Final submission is the
> maintainer's decision, and campaign approval is a program decision made by
> the organizers; this document does not guarantee acceptance.

## 1. Project summary

Soroban Forge is a community-driven Rust workspace of reusable Soroban smart
contracts and developer tooling for the Stellar ecosystem: escrow, token
vesting, multi-signature wallet, DAO governance, subscription payments, and
marketplace royalties, plus shared utilities, testing helpers, a developer CLI,
a TypeScript SDK, and reference examples.

The project targets developers who would otherwise reimplement these common
patterns independently. Contributors improve correctness, security-review
coverage, examples, tooling, and release hygiene, and the results are reusable
artifacts for other Stellar builders.

**Status (honest):** under active development. The escrow contract (16 tests)
and the vesting contract (21 tests) are implemented and tested in-crate; the
remaining four contracts ship public interfaces and storage types, with
implementations tracked in the issue backlog. Nothing is audited or
production-ready yet.

## 2. Stellar ecosystem value

- Turns repeated contract patterns (escrow, vesting, multisig, governance,
  subscriptions, royalties) into documented, testable modules.
- Shares one error model, storage conventions, and test utilities across all
  contracts, lowering the barrier for new Soroban contributors.
- Ships SDK/CLI/examples so the workspace demonstrates the full
  contract → tooling → app pipeline for Stellar.

## 3. Links

- Repository: https://github.com/Meet-hybrid/soroban-forge
- Documentation: `docs/` (getting started, architecture, contracts, tutorials,
  best practices)
- CI: `.github/workflows/ci.yml` — Rustfmt, Clippy (-D warnings), Build, Test,
  WASM Size Check (per-contract budget), all with `--locked`; plus a separate
  Security Audit workflow. All green on `main`.
- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-Apache`)
- Security policy: `SECURITY.md` (private vulnerability reporting via GitHub)
- Contributing guide: `CONTRIBUTING.md` (fork-first contributor workflow)

## 4. Security / audit status

- No independent audit has been performed; this is stated in the README.
- CI runs dependency auditing (rustsec/audit-check) and a WASM size budget.
  One advisory (`RUSTSEC-2026-0009`, `time` 0.3.44) is pinned in CI with a
  documented rationale: it is a transitive dependency of the pinned soroban-sdk
  21.x chain, is not compiled into the workspace graph, and is removed by the
  soroban-sdk 27 migration (issue #14).
- A security-invariant test suite is planned in the issue backlog
  (issue #21).
- Contracts must not be described as audited or production-ready until an
  independent review supports that claim.

## 5. Initial issue roadmap

Eleven scoped issues are live on GitHub (#14–#24), created from
`docs/maintainers/issue-backlog.md` via `scripts/create-issues.sh`:

| GitHub | Issue | Complexity |
| ------ | ----- | ---------- |
| #14 | chore: migrate to soroban-sdk 27 and stellar-xdr 27 | high |
| #15 | test(escrow): audit and harden the escrow implementation | medium |
| #16 | test(vesting): audit and harden the vesting implementation | medium |
| #17 | feat(multi-sig-wallet): implement authorization and execution | medium |
| #18 | feat(dao-governance): implement proposal/voting state transitions | medium |
| #19 | test: add in-process integration tests for every public method | medium |
| #20 | test(cli): add smoke tests and CI coverage | medium |
| #21 | test: add security invariant tests across contracts | high |
| #22 | chore: cut the v0.1.0 release and changelog | trivial |
| #23 | chore: harden WASM-size and dependency audit checks | medium |
| #24 | docs: document deployment and storage compatibility | trivial |

Each issue has one concrete deliverable, a conventional-commit title,
acceptance criteria, implementation and PR guidelines, affected files, and
verification commands, formatted like the Stellar Wave reference repository.
Contributors work in forks and are assigned before starting. Issues ship
without the `Stellar Wave` campaign label; the label is applied only if the
repository is accepted into the program. The escrow and vesting
implementations serve as the reference baseline for the audit tasks #15 and
#16.

## 6. Maintainer contact and review expectations

- Maintainer: the Meet-hybrid account (GitHub).
- Review turnaround target: acknowledge PRs within 3 business days; first
  review within 7 business days.
- One approval is required by branch protection for contributor pull
  requests (repository administrators are exempt); sensitive or non-trivial
  changes should receive a second maintainer review where practical.
- Unassignments are communicated promptly when work is paused, per
  `docs/maintainers/issue-triage.md`.

## 7. Pre-submission checklist

- [x] Issue backlog created on GitHub (#14–#24, 11 issues, one complexity
      label each, no campaign label).
- [x] Escrow and vesting contracts implemented and tested in-crate
      (37 tests total); CLI wired into the workspace and CI.
- [x] CI is green on `main` for all required checks.
- [x] README, SECURITY, CONTRIBUTING, and docs agree with the repository
      state.
- [x] This packet's factual claims verified against the repository.
- [ ] Final wording confirmed by the maintainer before submission.

## 8. Notes on application paths

- **Drips (Stellar Wave):** approval is an ecosystem/program decision made by
  organizers; onboarding is via the Drips maintainer dashboard after the
  repository is ready.
- **GrantFox:** a separate application and approval path; Drips approval does
  not transfer. Campaign labels from one program are not reused in the other
  until each program accepts the repository.
