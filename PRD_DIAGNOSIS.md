# PRD: #205: feat(escrow): multi-asset basket escrows with per-asset conservation and all-or-nothing settlement

## 1. Executive Summary
`crates/escrow` currently supports single SEP-41 token escrows. Issue #205 introduces multi-asset basket escrows as an additive, non-breaking extension. A basket escrow custodies $N \ge 1$ distinct SEP-41 assets under all-or-nothing settlement semantics. Every asset in the basket is conserved exactly across deposit and terminal payout (release, refund, or arbitration resolve). A dispute freezes all assets in the basket, and an arbitration resolve awards the entire basket to either buyer or seller. If any token transfer fails during deposit or terminal settlement, the transaction reverts atomically via Soroban host rollback, ensuring no partial state or stranded balances.

## 2. In-Thread Design Decisions
1. **Partial-Deposit Policy**: All-or-nothing. A basket escrow transitions from `Pending` to `Funded` if and only if all $N$ assets are successfully transferred into the contract. No intermediate partial funding states exist.
2. **Basket Size Bound**: $1 \le N \le 10$. This upper bound bounds CPU instructions, memory footprint, and cross-contract call limits on Soroban host execution.
3. **Duplicate-Token Policy**: Strict rejection. Passing duplicate token addresses in a basket returns `ForgeError::InvalidInput` immediately. Merging duplicate entries silently is prohibited to prevent ambiguous accounting.
4. **TTL / Keeper Semantics**: Per-escrow expiration. Persistent entries (`DataKey::BasketEscrow(id)`) and participant index entries are bumped to `ttl::BUMP_AMOUNT` when within `ttl::BUMP_THRESHOLD`. `touch_ttl` supports basket escrows.

## 3. Data Model & Storage Compatibility
- **Additive Storage Key**: `DataKey::BasketEscrow(u64)` is added to `DataKey`. Existing `DataKey::Escrow(u64)` remains unchanged, avoiding XDR map-length mismatches.
- **Contract Types**:
  ```rust
  #[contracttype]
  #[derive(Clone, Debug, Eq, PartialEq)]
  pub struct BasketAsset {
      pub token: Address,
      pub amount: i128,
  }

  #[contracttype]
  #[derive(Clone, Debug)]
  pub struct BasketEscrowData {
      pub escrow_id: u64,
      pub buyer: Address,
      pub seller: Address,
      pub arbiter: Address,
      pub assets: Vec<BasketAsset>,
      pub timeout: u64,
      pub status: EscrowStatus,
      pub created_at: u64,
  }
  ```
- **Shared ID Space**: The global counter `DataKey::Count` allocates monotonic IDs across both single-token and basket escrows.
- **Participant Index**: Participant indexes (`DataKey::ParticipantIndex(Address)`) track IDs of both single and basket escrows.

## 4. Public API Specification
- `create_escrow_multi(env: Env, buyer: Address, seller: Address, arbiter: Address, assets: Vec<BasketAsset>, timeout: u64) -> Result<u64, ForgeError>`
- `deposit_multi(env: Env, escrow_id: u64) -> Result<(), ForgeError>`
- `release_multi(env: Env, escrow_id: u64) -> Result<(), ForgeError>`
- `refund_multi(env: Env, escrow_id: u64) -> Result<(), ForgeError>`
- `dispute_multi(env: Env, escrow_id: u64, claimant: Address) -> Result<(), ForgeError>`
- `resolve_multi(env: Env, escrow_id: u64, in_favor_of_seller: bool) -> Result<(), ForgeError>`
- `cancel_multi(env: Env, escrow_id: u64) -> Result<(), ForgeError>`
- `get_basket_escrow(env: Env, escrow_id: u64) -> Result<BasketEscrowData, ForgeError>`
- `get_status(env: Env, escrow_id: u64) -> Result<EscrowStatus, ForgeError>` (handles single and basket escrows)
- `touch_ttl(env: Env, escrow_id: u64) -> Result<(), ForgeError>` (handles single and basket escrows)

## 5. Settlement & Ordering Discipline
- **Transfer-Before-State**: All token transfers across the basket are executed in sequence before persistent storage is updated.
- **Atomic Rollback**: If transfer $k$ of $N$ fails, the invocation returns `ForgeError::TokenTransferFailed`, causing the Soroban host to revert all previous token transfers in the invocation and leaving contract storage untouched.
- **Event Visibility**: Because state is written and events are emitted only after all transfers succeed, failed invocations never emit partial events.

## 6. Events Module
Dedicated events for basket escrows with `escrow_id` topic:
- `BasketEscrowCreated { escrow_id: u64, data: BasketEscrowData }`
- `BasketDeposited { escrow_id: u64, data: BasketEscrowData }`
- `BasketReleased { escrow_id: u64, data: BasketEscrowData }`
- `BasketRefunded { escrow_id: u64, data: BasketEscrowData }`
- `BasketDisputed { escrow_id: u64, data: BasketEscrowData }`
- `BasketResolved { escrow_id: u64, data: BasketEscrowData, in_favor_of_seller: bool }`
- `BasketCancelled { escrow_id: u64, data: BasketEscrowData }`

## 7. Worked Example (2-Asset Basket Lifecycle)
- **Creation**: Buyer creates escrow #42 with Asset A (1,000 units) and Asset B (500 units), timeout 3,600s. State is `Pending`.
- **Deposit**: Buyer authorizes `deposit_multi(42)`. Contract pulls 1,000 units of A and 500 units of B. State transitions to `Funded`.
- **Dispute**: Seller calls `dispute_multi(42, seller)`. State transitions to `Disputed`. Both Asset A and Asset B are frozen.
- **Resolve**: Arbiter resolves `resolve_multi(42, in_favor_of_seller = true)`. Contract transfers 1,000 units of A and 500 units of B to seller. State transitions to `Completed`. Contract holds 0 units of A and 0 units of B.

## 8. Verification & Acceptance Criteria
- `cargo fmt --all -- --check` passes cleanly.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` emits zero warnings.
- `cargo test -p soroban-forge-escrow --all-targets --locked` passes all tests (including new proptest cases and negative-auth tests).
- `cargo test --workspace --all-targets --locked` completes with zero failures.