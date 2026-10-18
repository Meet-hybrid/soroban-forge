# PR: Sync CHANGELOG [Unreleased] with FEATURE-STATUS matrix

Closes #263

## Description

Synchronizes `CHANGELOG.md`'s `[Unreleased]` section with features already documented in `docs/FEATURE-STATUS.md` and implemented across the workspace contracts. 

Specifically, adds structured Keep-a-Changelog entries under `[Unreleased]` for:
1. **Escrow Milestone Partial Release** (`release_partial`): buyer/arbiter partial release with remaining balance tracking.
2. **DAO Proposal Bonds**: SEP-41 token bond deposit upon `propose`, refundable on execution/quorum and forfeited/slashed on rejection/cancellation.
3. **Vesting Claim Settlement**: real token settlement via SEP-41 transfers during `claim` / `claim_tranche`.
4. **Subscription Pause/Resume** (`pause_subscription`, `resume_subscription`): caller/provider pause controls with billing interval preservation.

---

## Changes Made to `CHANGELOG.md`

### Added
- **Escrow milestone partial release** (`release_partial`): allows partial disbursement of funds to the seller prior to final settlement while keeping the remaining amount secured under the escrow contract until completion or resolution.
- **DAO proposal bond lifecycle**: enforces SEP-41 bond transfer from proposer on proposal creation; automatically refunds bonds on successful proposal execution and forfeits/slashes bonds on defeated/cancelled proposals.
- **Vesting claim SEP-41 settlement**: linear and tranche vesting `claim` operations execute real token transfers to beneficiaries, updating claimed amounts and verifying contract custody.
- **Subscription pause and resume** (`pause_subscription`, `resume_subscription`): subscriber/provider-authorized state transition that halts recurring charges while preserving billing periods, plan parameters, and unspent prepaid balances.

---

## Verification

- [x] All entrypoint names, error behaviors, and contract paths verified directly against source code and `docs/FEATURE-STATUS.md`.
- [x] No previously released version sections modified.
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`

---

## Type of Change

- [x] Documentation update (Keep-a-Changelog synchronization)
