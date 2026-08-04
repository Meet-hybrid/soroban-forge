# Maintainer-program readiness

## Project summary

Soroban Forge is an open-source Rust workspace of reusable Soroban contracts,
shared utilities, testing helpers, a CLI, and TypeScript examples. It targets
developers who would otherwise reimplement escrow, vesting, multisig,
governance, subscriptions, and royalty flows independently.

## Why this is useful to the Stellar ecosystem

The project turns repeated contract patterns into documented, testable modules.
Community work can improve correctness, security review coverage, examples,
developer tooling, and release hygiene while producing reusable artifacts for
other Stellar builders.

## Current state (verified 2026-08-04)

- Eleven scoped issues are live on GitHub (#14–#24), each with one complexity
  label, acceptance criteria, and verification commands, and none with a
  campaign label.
- Escrow (16 tests) and vesting (21 tests) are implemented in-crate as the
  reference quality bar; the remaining four contracts are interface-only and
  tracked in the backlog.
- CI is green on `main`: Rustfmt, Clippy, Build, Test, WASM Size Check (all
  `--locked`) plus a Security Audit workflow.
- A `v0.1.0` tag is pushed and the Release workflow drafts a GitHub release
  with all six contract WASM artifacts.
- Contribution model is fork-first; maintainers assign issues and review fork
  pull requests per `issue-triage.md`.

## First contribution campaign

The live issue set prioritizes verifiable maintenance rather than promising
every contract at once:

1. Contract-level unit and integration coverage for the highest-risk public
   methods.
2. Security invariants and authorization tests for escrow, vesting, and
   multisig.
3. CLI and TypeScript SDK examples that build in CI.
4. Documentation corrections and runnable deployment examples.
5. Release, dependency, and WASM-size checks.

A campaign would assign issues from this backlog. Each issue uses the
Maintainer Task template, carries one complexity label, and names its
acceptance criteria. Do not label an issue for a campaign until the
repository has been accepted by that campaign's organizers.

## Evidence to include in an application

- repository URL and license;
- a short demo or runnable quick start;
- CI status and the commands contributors should run;
- documentation and security policy links;
- the 11 live scoped issues (#14–#24) with acceptance criteria;
- maintainer contact and expected review turnaround;
- a statement identifying any components that are not yet audited or
  production-ready.

## Important accuracy note

Soroban Forge must not describe contracts as audited or production-ready until
an independent audit and release-readiness review actually support those
claims. Campaign approval is not a security audit or a guarantee that a
contribution will be funded.
