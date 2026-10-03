# Soroban Forge Smart Contracts Reference

Authoritative technical reference for the production smart contracts provided by Soroban Forge. This documentation is synchronized with contract implementations, cryptographic authorization rules, ledger storage schemas, and operational parameters across Soroban SDK v28 (`soroban-sdk = "=28.0.0"`).

---

## Architecture Standards

All contracts in the Soroban Forge workspace adhere to shared architectural principles to guarantee security, predictable ledger fee consumption, and deterministic state transitions.

### 1. Error Model (`ForgeError`)

The shared error enum [`ForgeError`](../../crates/shared-utils/src/errors.rs) defines a unified error space across all contracts. Contract error codes start at `1` (code `0` is reserved by the Soroban host runtime):

| Code | Variant | Description |
|:----:|:--------|:------------|
| `1` | `Unauthorized` | Caller lacks required authorization, or is not the recorded owner/funder/subscriber. |
| `2` | `NotFound` | Target entity (escrow, proposal, schedule, subscription, config) does not exist in storage. |
| `3` | `InvalidInput` | Argument failed validation (e.g. non-positive amount, zero duration, cycle in DAG, duplicate entity). |
| `4` | `InsufficientFunds` | Contract custody balance or subscriber prepaid balance is insufficient for requested operation. |
| `5` | `AlreadyInitialized` | Entity or one-time configuration has already been initialized. |
| `6` | `NotInitialized` | Contract requires initialization or setup before invoking the entrypoint. |
| `7` | `DeadlineReached` | Operation attempted after deadline elapsed, or prerequisite proposal has not completed. |
| `8` | `InsufficientAllowance` | Token allowance is less than requested expenditure. |
| `9` | `ArithmeticOverflow` | Mathematical computation overflowed integer bounds (`u32`, `u64`, or `i128`). |
| `10` | `Custom` | Domain-specific contract error not covered by standard variants. |
| `11` | `TokenTransferFailed` | SEP-41 token transfer failed (insufficient balance, missing trustline, or host abort). Discarded raw token error. |
| `12` | `ContractInvocationFailed` | Cross-contract call reverted or failed during target dispatch. |
| `13` | `WithdrawalLimitExceeded` | Proposed withdrawal exceeds configured rolling-window rate limit. |
| `14` | `SubscriptionPastDue` | Subscription is in arrears and cannot perform action until restored to `Active`. |
| `15` | `ProposerCooldown` | Proposer reached maximum concurrent active proposals limit (`DEFAULT_MAX_ACTIVE_PROPOSALS = 5`). |

### 2. Token Custody & Transfer-Before-State Discipline

All contracts enforcing token movements (Escrow, Vesting, Multi-Sig Wallet, DAO Governance, Subscription Payments) adhere to the **transfer-before-state ordering discipline**:

1. Validate input parameters and verify mathematical bounds.
2. Verify authorization envelopes.
3. Perform external cross-contract SEP-41 token transfers (`transfer_to_contract`, `transfer_from_contract`, or `transfer_tokens`).
4. Commit state mutations to contract storage only after token transfer success.

**Host Frame Rollback Guarantee**: If a token contract invocation fails or reverts, the entire Soroban host invocation frame is rolled back automatically. This eliminates state/balance divergence windows and guarantees that contracts never record commitments for unreceived or unreleased funds.

**Error Bucketing**: All external token failures (insufficient balance, missing or deauthorized trustlines, custom token logic reverts, or undeployed token contracts) are bucketed into `ForgeError::TokenTransferFailed`. Raw discriminants are discarded to avoid collision with contract-native error codes. Diagnostic events remain visible on the transaction call tree.

### 3. Authorization Model

- **Host Enforce Mode**: Entrypoints calling `Address::require_auth()` or `require_auth_for_args()` require matching signatures in the invocation envelope. Missing, surplus, or parameter-mismatched authorizations abort the transaction via `InvokeError::Abort`.
- **Contract Self-Authorization**: The Soroban host implicitly auto-authorizes `require_auth` checks performed by the currently executing contract address. Token releases and refunds from contract custody execute under self-authorization and do not require external user signature sub-invocations.
- **Nested Cross-Contract Authorization**: Operations pulling tokens from user balances into custody (such as `deposit` in Escrow, `propose` in DAO Governance, or `distribute` in Marketplace Royalties) require user authorization covering both the outer contract entrypoint and the nested token `transfer` invocation.

### 4. Storage & TTL Management

Contracts partition data between **Instance Storage** and **Persistent Storage**:

- **Instance Storage**: Used for small, hot, singleton configuration records (e.g. `Owners`, `Threshold`, `Count`, `BondConfig`, `GovernanceToken`). Lifetime is tied to contract instance availability.
- **Persistent Storage**: Used for unbounded, addressable entity records (e.g. `EscrowData`, `BasketEscrowData`, `Proposal`, `WalletTx`, `Balance`, `ParticipantIndex`, `Royalty`, `SettlementSummary`). Scales independently per record.

#### Storage TTL Parameters
Persistent entries are managed via the shared `TTLHelper` policy (`crates/shared-utils/src/ttl.rs`):

| Parameter | Value (Ledgers) | Metric Equivalent (at ~5s/ledger) |
|:----------|:---------------:|:----------------------------------|
| `DAY_IN_LEDGERS` | `17,280` | ~24 hours |
| `BUMP_AMOUNT` | `518,400` | ~30 days |
| `BUMP_THRESHOLD` | `501,120` | ~29 days (bumping when ≤ 1 day remaining) |

Each persistent write or explicit `touch_ttl` call extends the entry TTL to `BUMP_AMOUNT` whenever remaining life falls below `BUMP_THRESHOLD`.

---

## Contract Catalog

| Contract | Package | Description | Reference Link |
|:---------|:--------|:------------|:---------------|
| **Escrow** | `soroban-forge-escrow` | Three-party conditional token custody with dispute arbitration, milestone partial releases, and multi-asset basket support. | [escrow.md](./escrow.md) |
| **Vesting** | `soroban-forge-vesting` | Token vesting schedules supporting linear straight-line ramps with cliffs, discrete tranche unlock tables, funder revocation, and beneficiary reassignment. | [vesting.md](./vesting.md) |
| **Multi-Sig Wallet** | `soroban-forge-multi-sig-wallet` | M-of-N multi-signature treasury wallet supporting opaque payloads, typed calls, token custody, single-rejection vetoes, rolling withdrawal limits, and on-chain owner governance. | [multi-sig-wallet.md](./multi-sig-wallet.md) |
| **DAO Governance** | `soroban-forge-dao-governance` | Token-weighted proposal voting with anti-spam proposal bonds, DAG execution order dependencies, proposer cooldowns, and cross-contract target execution. | [dao-governance.md](./dao-governance.md) |
| **Subscription Payments** | `soroban-forge-subscription-payments` | Recurring subscription payments supporting pull and prepaid custody modes, provider opt-in consent, multi-period catch-up billing, pause/resume, and metered usage quotas with overage pricing. | [subscription-payments.md](./subscription-payments.md) |
| **Marketplace Royalties** | `soroban-forge-marketplace-royalties` | Secondary NFT marketplace sales settlement supporting multi-recipient royalty splits, floor-rounding dust retention, atomic batch settlement, and quote derivation. | [marketplace-royalties.md](./marketplace-royalties.md) |

---

## Global Limitations & Boundary Constraints

| Contract | Constant / Metric | Constraint Bound | Behavior on Violation |
|:---------|:------------------|:-----------------|:----------------------|
| **Escrow** | `MAX_BASKET_ASSETS` | Maximum `8` asset legs per basket | Returns `ForgeError::InvalidInput` at creation. |
| **Escrow** | `seller_bps` | Maximum `10_000` (100.00%) | Returns `ForgeError::InvalidInput` on dispute split. |
| **Vesting** | `MAX_TRANCHES` | Maximum `32` tranches per schedule | Returns `ForgeError::InvalidInput` at creation. |
| **Vesting** | Tranche Timestamps | Strictly increasing offsets (`t[i] > t[i-1]`) | Returns `ForgeError::InvalidInput` if duplicate or decreasing. |
| **Multi-Sig Wallet** | `threshold` | `1 <= threshold <= owners.len()` | Returns `ForgeError::InvalidInput` at initialization or proposal. |
| **Multi-Sig Wallet** | Rejections | `1` objection vetoes transaction | Transitions immediately to `Rejected`; blocks `execute`. |
| **Multi-Sig Wallet** | Owners Removal | Cannot remove final owner; cannot leave `threshold > owners.len()` | Returns `ForgeError::InvalidInput` at submission or execution. |
| **DAO Governance** | `DEFAULT_MAX_ACTIVE_PROPOSALS` | Maximum `5` concurrent active proposals per proposer | Returns `ForgeError::ProposerCooldown` on `propose`. |
| **DAO Governance** | Dependency DAG | Cycles detected via iterative DFS | Returns `ForgeError::InvalidInput` at proposal time. |
| **DAO Governance** | Voting Majority | Strict majority required (`for_votes > against_votes`) | Ties (`for == against`) or zero-vote outcomes resolve to `Defeated`. |
| **Subscription Payments** | `MAX_RETRIES` | Maximum `3` consecutive failed payment attempts | Transitions from `PastDue` to `Cancelled`; refunds prepaid balance. |
| **Subscription Payments** | `MAX_CATCHUP_PERIODS` | Maximum `32` periods per batch invocation | Returns `ForgeError::InvalidInput` if `max_periods > 32`. |
| **Subscription Payments** | `MAX_QUOTAS` | Maximum `16` metric quotas per subscription | Returns `ForgeError::InvalidInput` on `set_quotas`. |
| **Marketplace Royalties** | `MAX_SETTLE_SALES` | Maximum `20` sales per batch settlement | Returns `ForgeError::InvalidInput` if `sales.len() > 20`. |
| **Marketplace Royalties** | `MAX_ROYALTY_RECIPIENTS` | Maximum `5` recipients per collection split | Returns `ForgeError::InvalidInput` on split configuration. |
| **Marketplace Royalties** | Total Royalty BPS | Maximum `10_000` (100.00%) | Returns `ForgeError::InvalidInput` if sum exceeds 10,000 bps. |
