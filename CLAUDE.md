# Task Workspace Context Protocol

## Workspace Sandbox Root
`/Users/fangqq/.bounty_agent_platform/workspaces/gh-5608023608`

## Task Info
- Repository: Meet-hybrid/soroban-forge
- Issue: #205
- Title: feat(escrow): multi-asset basket escrows with per-asset conservation and all-or-nothing settlement

## Git Branch Protocol
- Feature Branch: `feat/issue-205`
- Target Base Branch: `main`

## Issue Description


## Step 2 PRD Specifications
- Root Cause: `crates/escrow` is strictly single-asset in its storage model (`EscrowData` containing a single `token: Address`, `amount: i128`, and `released: i128`), its entrypoints (`create_escrow`, `deposit`, `release`, `refund`, `resolve`), its event payloads, and its property tests. There is no abstraction for multi-asset baskets `(Address, i128)`, no multi-token atomic transfer settlement ensuring per-asset conservation across all terminal execution paths, no validation against duplicate basket assets or bounded basket sizes, and no multi-asset storage key or record variant that preserves backward compatibility with existing single-token escrow records.
- Proposed Fix: Implement an additive, non-breaking multi-asset basket escrow architecture in `crates/escrow`: (1) Define `BasketAsset` and `BasketEscrowData` contract types and `DataKey::BasketEscrow(u64)` persistent storage key to isolate storage and prevent XDR map-length deserialization panics on existing `EscrowData` records; (2) Implement `create_escrow_multi`, `deposit_multi`, `release_multi`, `refund_multi`, `dispute_multi`, `resolve_multi`, `cancel_multi`, and `get_basket_escrow`, updating `get_status` and `touch_ttl` to handle both single and basket escrow IDs; (3) Enforce strict validation: basket size bounded to 1..=10, explicit rejection of duplicate token addresses (`ForgeError::InvalidInput`), and positive amounts per asset; (4) Enforce transfer-before-state ordering across all assets with all-or-nothing settlement relying on Soroban host rollback upon any failed token transfer; (5) Emit dedicated basket events (`BasketEscrowCreated`, `BasketDeposited`, `BasketReleased`, `BasketRefunded`, `BasketDisputed`, `BasketResolved`, `BasketCancelled`) to preserve indexer event contracts; (6) Integrate basket escrows into the existing participant index using the shared monotonic ID counter; (7) Extend proptest invariant suites for multi-asset conservation, negative-auth test coverage for all new entrypoints, and documentation in `docs/contracts/escrow.md` and `docs/FEATURE-STATUS.md`.
- Test Command/Strategy: cargo test

## Demand-to-Code Mapping
- Requirement: Multi-asset basket creation and validation with non-breaking API
  Affected: ["crates/escrow/src/lib.rs"]
  Action: Define `BasketAsset` and `BasketEscrowData` structs; introduce `create_escrow_multi(env, buyer, seller, arbiter, assets: Vec<BasketAsset>, timeout)` validating `1 <= assets.len() <= 10`, no duplicate tokens, and positive amounts.
- Requirement: All-or-nothing deposit with transfer-before-state ordering
  Affected: ["crates/escrow/src/lib.rs"]
  Action: Implement `deposit_multi` to iterate through all basket assets, executing `transfer_to_contract` for each prior to committing the `Funded` status to persistent storage, guaranteeing complete host rollback on any transfer failure.
- Requirement: Per-asset exact conservation across terminal payouts (release, refund, resolve)
  Affected: ["crates/escrow/src/lib.rs", "crates/escrow/src/props.rs"]
  Action: Implement `release_multi`, `refund_multi`, and `resolve_multi` to transfer all basket assets to the recipient using transfer-before-state ordering; generalize property tests in `props.rs` to verify per-asset conservation over random baskets.
- Requirement: Dispute freezes all basket assets; resolve settles all-or-nothing
  Affected: ["crates/escrow/src/lib.rs", "crates/escrow/src/tests.rs"]
  Action: Implement `dispute_multi(env, escrow_id, claimant)` freezing all basket assets under `EscrowStatus::Disputed`; implement `resolve_multi(env, escrow_id, in_favor_of_seller)` paying the entire basket all-or-nothing to seller (`true`) or buyer (`false`).
- Requirement: Participant index invariants and cancellation support for baskets
  Affected: ["crates/escrow/src/lib.rs", "crates/escrow/src/tests.rs", "crates/escrow/src/props.rs"]
  Action: Index distinct participants on `create_escrow_multi` using the shared monotonic ID counter; implement `cancel_multi` to remove basket ID from distinct participant indexes while preserving survivor order.
- Requirement: TTL keeper semantics defined for baskets
  Affected: ["crates/escrow/src/lib.rs"]
  Action: Update `touch_ttl` (or add `touch_ttl_multi`) to check and bump `DataKey::BasketEscrow(id)` entries to `ttl::BUMP_AMOUNT` when within `ttl::BUMP_THRESHOLD`.
- Requirement: Negative-authorization and replay protection coverage
  Affected: ["crates/escrow/src/authz.rs"]
  Action: Add negative-auth tests covering wrong signers on `create_escrow_multi`, `deposit_multi`, `release_multi`, `refund_multi`, `dispute_multi`, `resolve_multi`, and `cancel_multi`, as well as replay over altered basket payloads.
- Requirement: Documentation and status matrix synchronization
  Affected: ["docs/contracts/escrow.md", "docs/FEATURE-STATUS.md"]
  Action: Document multi-asset basket API, validation rules, all-or-nothing semantics, and events in `docs/contracts/escrow.md`; update `docs/FEATURE-STATUS.md` matrix.

## Actionable Task Checklist
1. Task 1: Define `BasketAsset`, `BasketEscrowData`, and `DataKey::BasketEscrow(u64)` in `crates/escrow/src/lib.rs`.
2. Task 2: Define basket events (`BasketEscrowCreated`, `BasketDeposited`, `BasketReleased`, `BasketRefunded`, `BasketDisputed`, `BasketResolved`, `BasketCancelled`) in `events` module.
3. Task 3: Implement `validate_basket_assets` helper enforcing `1 <= len <= 10`, duplicate address rejection, and positive amounts.
4. Task 4: Implement `create_escrow_multi` in `SorobanForgeEscrow` trait and `Escrow` contract impl, recording distinct participant indices.
5. Task 5: Implement `deposit_multi` enforcing buyer auth, Pending status, and atomic transfer-before-state pull of all basket tokens.
6. Task 6: Implement `release_multi` enforcing seller auth, Funded status, and atomic transfer-before-state payout of all basket tokens.
7. Task 7: Implement `refund_multi` enforcing pre-deadline seller auth or post-deadline buyer auth, and atomic payout of all basket tokens.
8. Task 8: Implement `dispute_multi` and `resolve_multi` enforcing arbiter auth and all-or-nothing payout to seller or buyer.
9. Task 9: Implement `cancel_multi` enforcing buyer auth, Pending status, and participant index cleanup.
10. Task 10: Implement `get_basket_escrow` and update `get_status` and `touch_ttl` to support basket escrow records.
11. Task 11: Write unit tests in `tests.rs` for full lifecycle, edge cases (empty, oversized, duplicates), and atomicity rollback on failed token transfer.
12. Task 12: Write negative-authorization tests in `authz.rs` for all multi-asset entrypoints and argument-tampering replay rejection.
13. Task 13: Extend property tests in `props.rs` with multi-asset per-asset conservation (P1) and participant index consistency (P4).
14. Task 14: Update `docs/contracts/escrow.md` and `docs/FEATURE-STATUS.md` with multi-asset basket specifications.
15. Task 15: Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --all-targets --locked` to verify complete workspace compliance.

## Strict Guidelines
1. All modifications MUST be made inside `/Users/fangqq/.bounty_agent_platform/workspaces/gh-5608023608`.
2. Implement complete code with ZERO stubs or placeholders.
3. Run `cargo test` to verify all tests pass.
