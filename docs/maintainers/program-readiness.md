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

## First contribution campaign

The initial issue set should prioritize verifiable maintenance rather than
promising every contract at once:

1. Contract-level unit and integration coverage for the highest-risk public
   methods.
2. Security invariants and authorization tests for escrow, vesting, and
   multisig.
3. CLI and TypeScript SDK examples that build in CI.
4. Documentation corrections and runnable deployment examples.
5. Release, dependency, and WASM-size checks.

Each issue should use the Maintainer Task template, have one complexity label,
and name its acceptance criteria. Do not label an issue for a campaign until
the repository has been accepted by that campaign's organizers.

## Evidence to include in an application

- repository URL and license;
- a short demo or runnable quick start;
- CI status and the commands contributors should run;
- documentation and security policy links;
- the first 5–10 scoped issues with acceptance criteria;
- maintainer contact and expected review turnaround;
- a statement identifying any components that are not yet audited or
  production-ready.

## Important accuracy note

Soroban Forge must not describe contracts as audited or production-ready until
an independent audit and release-readiness review actually support those
claims. Campaign approval is not a security audit or a guarantee that a
contribution will be funded.
