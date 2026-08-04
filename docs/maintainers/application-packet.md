# Maintainer application packet — Soroban Forge

> **Draft for review.** This packet is a template for a Drips (Stellar Wave)
> and/or GrantFox application. It must not be submitted until the
> implementation gaps in the issue backlog are closed and the maintainer has
> confirmed the wording. Campaign approval is a program decision; this document
> does not guarantee acceptance.

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

**Status (honest):** under active development. Contract crates currently ship
public interfaces and storage types; implementations and tests are being
delivered through the initial issue backlog. Nothing is audited or
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
- CI: `.github/workflows/ci.yml` — Rustfmt, Clippy, Build, Test, Security
  Audit, WASM Size Check
- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-Apache`)
- Security policy: `SECURITY.md` (private vulnerability reporting via GitHub)
- Contributing guide: `CONTRIBUTING.md`

## 4. Security / audit status

- No independent audit has been performed. This is stated in the README.
- CI runs dependency auditing (`cargo audit` via rustsec/audit-check) and a
  WASM size budget.
- A security-invariant test suite is planned in the issue backlog (Issue 7).
- Contracts must not be described as audited or production-ready until an
  independent review supports that claim.

## 5. Initial issue roadmap

See `docs/maintainers/issue-backlog.md` for the full drafted issues. Summary:

| # | Issue | Complexity |
| - | ----- | ---------- |
| 11 | Migrate to soroban-sdk 27 (dependabot-proposed) | high |
| 1 | Escrow lifecycle implementation + tests | high |
| 2 | Vesting edge cases implementation + tests | medium |
| 3 | Multisig authorization and execution | medium |
| 4 | DAO proposal/voting state transitions | medium |
| 5 | In-process integration tests for every public method | medium |
| 6 | CLI smoke tests and CI coverage | medium |
| 7 | Security invariant tests | high |
| 8 | First release and changelog | trivial |
| 9 | WASM-size and dependency audit hardening | medium |
| 10 | Deployment and storage-compatibility docs | trivial |

Each issue has one concrete deliverable, a conventional-commit title,
acceptance criteria, implementation and PR guidelines, affected files, and
verification commands, formatted like the Stellar Wave reference repository.
Issues ship without the `Stellar Wave` campaign label; the label is applied
only if the repository is accepted into the program.

## 6. Maintainer contact and review expectations

- Maintainer: the Meet-hybrid account (GitHub).
- Review turnaround target: acknowledge PRs within 3 business days; first
  review within 7 business days.
- Two reviews are required for security-sensitive changes; one approval is
  required by branch protection for other changes.
- Unassignees are communicated promptly when work is paused, per
  `docs/maintainers/issue-triage.md`.

## 7. Pre-submission checklist

- [ ] Issue backlog confirmed and created on GitHub.
- [ ] Integration test harness and CLI wired into CI (Issues 5–6; both
      already landed locally — keep the issues as regression tasks).
- [ ] At least the highest-risk contracts implemented and tested (Issues 1–4
      in progress).
- [ ] CI is green on `main` for all required checks.
- [ ] README, SECURITY, CONTRIBUTING, and docs agree with the repository state.
- [ ] This packet reviewed and finalized.

## 8. Notes on application paths

- **Drips (Stellar Wave):** approval is an ecosystem/program decision made by
  organizers; onboarding is via the Drips maintainer dashboard after the
  repository is ready.
- **GrantFox:** a separate application and approval path; Drips approval does
  not transfer. Campaign labels from one program are not reused in the other
  until each program accepts the repository.
