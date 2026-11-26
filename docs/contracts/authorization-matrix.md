# Cross-crate authorization matrix

This matrix is the integrator-facing answer to one question: **who must sign a
state-changing call, what happens when they do not, and who can move tokens?**
It covers the six core contracts. Per-contract guides remain the source of truth
for lifecycle details; this page cross-links their entrypoints to the generated
TypeScript clients.

## Conventions

- Entrypoint names are the exact `snake_case` names used by the Rust
  `#[contractclient]` traits **and** by the generated clients (for example
  `client.create_escrow(...)`).
- `Required signer(s)` lists the addresses whose `require_auth` must be
  satisfied. "none" means permissionless; a keeper may be any account.
- Every `require_auth` failure aborts the Soroban host invocation with an
  authorization error before any state is written. The `Unauthorized` entries
  in the tables are **contract-level identity checks** (for example, a caller
  that is not the recorded proposer), not the host abort.
- `Failure behavior` names `ForgeError` variants. Tokens are always transferred
  before the corresponding state write, so any error rolls the whole invocation
  back: no partial state and no stranded funds.
- `Token transfer authority` describes the SEP-41 movement authorized by the
  call. `→ contract` is a pull (funding/custody); `contract →` is a payout from
  contract custody. A signer's entrypoint authorization also covers the nested
  token invocation; token rejections are bucketed into `TokenTransferFailed`.
  "none" means no token moves.

## Contracts at a glance

| Contract | Crate path | Package | Generated client | Guide |
| --- | --- | --- | --- | --- |
| Escrow | `crates/escrow` | `soroban-forge-escrow` | `packages/typescript-sdk/src/clients/escrow.ts` | [escrow.md](./escrow.md) |
| Vesting | `crates/vesting` | `soroban-forge-vesting` | `packages/typescript-sdk/src/clients/vesting.ts` | [vesting.md](./vesting.md) |
| Multi-Sig Wallet | `crates/multi-sig-wallet` | `soroban-forge-multi-sig-wallet` | `packages/typescript-sdk/src/clients/multisig.ts` | [multi-sig-wallet.md](./multi-sig-wallet.md) |
| DAO Governance | `crates/dao-governance` | `soroban-forge-dao-governance` | `packages/typescript-sdk/src/clients/dao.ts` | [dao-governance.md](./dao-governance.md) |
| Subscription Payments | `crates/subscription-payments` | `soroban-forge-subscription-payments` | `packages/typescript-sdk/src/clients/subscription.ts` | [subscription-payments.md](./subscription-payments.md) |
| Marketplace Royalties | `crates/marketplace-royalties` | `soroban-forge-marketplace-royalties` | `packages/typescript-sdk/src/clients/marketplace.ts` | [marketplace-royalties.md](./marketplace-royalties.md) |

## Escrow

Crate `crates/escrow` (`soroban-forge-escrow`); generated client
`packages/typescript-sdk/src/clients/escrow.ts`. The buyer funds custody; payouts
are contract-owned. The seller and arbiter are recorded without authorizing
creation so they can act later.

### Mutating entrypoints

| Entrypoint | Required signer(s) | Failure behavior | Token transfer authority |
| --- | --- | --- | --- |
| `create_escrow` | buyer | `InvalidInput` (non-positive amount or zero timeout); `ArithmeticOverflow` (id counter) | none |
| `create_escrow_with_arbiter_fee` | buyer | `InvalidInput` (base validation or fee above 1000 bps); `ArithmeticOverflow` | none |
| `create_escrow_with_review` | buyer | `InvalidInput` (base validation or review period outside 60 seconds to 30 days); `ArithmeticOverflow` | none |
| `deposit` | buyer | `NotFound` (unknown id); `InvalidInput` (not `Pending`); `TokenTransferFailed` | buyer → contract, all-or-nothing |
| `release` | buyer | `NotFound`; `InvalidInput` (not `Funded`); `TokenTransferFailed` | contract → seller, full remaining balance |
| `release_partial` | buyer | `NotFound`; `InvalidInput` (not `Funded`, non-positive amount, amount above remaining); `ArithmeticOverflow`; `TokenTransferFailed` | contract → seller, one partial leg |
| `refund` | seller before deadline; buyer after deadline | `NotFound`; `InvalidInput` (not `Funded`); `Unauthorized` (wrong party for the phase); `ReleasePending` (buyer reclaim with a request waiting); `TokenTransferFailed` | contract → buyer, remaining balance |
| `refund_expired` | none (permissionless, strictly after deadline) | `NotFound`; `InvalidInput` (not `Funded`); `DeadlineReached` (deadline not passed); `ReleasePending`; `ArithmeticOverflow`; `TokenTransferFailed` | contract → buyer, remaining balance |
| `dispute` | claimant explicitly named as buyer or seller | `NotFound`; `InvalidInput` (not `Funded` or claimant is neither party); `Unauthorized` (claimant signature missing) | none; freezes the escrow |
| `resolve` | arbiter recorded on the escrow | `NotFound`; `InvalidInput` (not `Disputed`); `Unauthorized` (not the arbiter); `TokenTransferFailed` | contract → arbiter fee, then contract → seller or buyer |
| `resolve_split` | arbiter recorded on the escrow | `NotFound`; `InvalidInput` (not `Disputed` or seller bps above 10,000); `Unauthorized`; `ArithmeticOverflow`; `TokenTransferFailed` | contract → arbiter fee, then contract → seller and buyer by basis points |
| `request_release` | seller | `NotFound`; `InvalidInput` (not `Funded` or invalid amount); `ReleasePending`; `DeadlineReached` | none; starts the review window only |
| `request_release_basket` | seller | `NotFound`; `InvalidInput` (not `Funded`); `ReleasePending`; `DeadlineReached` | none; starts the review window only |
| `approve_release` | buyer | `NotFound` (no pending request); `TokenTransferFailed` | contract → seller, all legs when requested by basket |
| `execute_release` | none (permissionless after the review window) | `NotFound` (no pending request); `DeadlineReached` (window not ended); `TokenTransferFailed` | contract → seller |
| `cancel` | buyer | `NotFound`; `InvalidInput` (not `Pending`) | none; nothing was funded |
| `create_basket` | buyer | `InvalidInput` (zero timeout, empty or oversized basket, non-positive leg, duplicate token); `ArithmeticOverflow` | none |
| `deposit_basket` | buyer | `NotFound`; `InvalidInput` (not `Pending`); `TokenTransferFailed` | buyer → contract for every leg, all-or-nothing |
| `release_basket` | buyer | `NotFound`; `InvalidInput` (not `Funded`); `TokenTransferFailed` | contract → seller for every leg |
| `release_partial_basket` | buyer | `NotFound` (no basket or no such token leg); `InvalidInput` (not `Funded`, non-positive amount, amount above leg remaining); `ArithmeticOverflow`; `TokenTransferFailed` | contract → seller for one leg |
| `refund_basket` | seller before deadline; buyer after deadline | `NotFound`; `InvalidInput` (not `Funded`); `Unauthorized`; `ReleasePending`; `TokenTransferFailed` | contract → buyer for every remaining leg |
| `dispute_basket` | claimant explicitly named as buyer or seller | `NotFound`; `InvalidInput` (not `Funded` or claimant is neither party); `Unauthorized` | none; freezes every leg |
| `resolve_basket` | arbiter recorded on the basket | `NotFound`; `InvalidInput` (not `Disputed`); `Unauthorized`; `TokenTransferFailed` | contract → seller or buyer for every leg, one direction |
| `cancel_basket` | buyer | `NotFound`; `InvalidInput` (not `Pending`) | none; nothing was funded |
| `replace_arbiter` | buyer and seller | `NotFound`; `InvalidState` (not `Pending` or `Funded`); `InvalidInput` (new arbiter equals buyer, seller, or current arbiter) | none |
| `replace_basket_arbiter` | buyer and seller | `NotFound`; `InvalidState` (not `Pending` or `Funded`); `InvalidInput` (new arbiter equals a party or the current arbiter) | none |
| `extend_deadline` | buyer | `NotFound`; `InvalidInput` (not `Funded` or new timeout not greater); `ArithmeticOverflow` | none |
| `extend_basket_deadline` | buyer | `NotFound`; `InvalidInput` (not `Funded` or new timeout not greater); `ArithmeticOverflow` | none |
| `touch_ttl` | none (keeper) | `NotFound` (no escrow record) | none; extends the persistent entry TTL only |

### Read-only queries

| Entrypoint | Auth | Notes / failures |
| --- | --- | --- |
| `preflight_deposit` | none | No writes; unknown id returns an `UnknownEscrow` status rather than an error |
| `get_release_request` | none | Returns `Option<ReleaseRequest>`; no storage write |
| `review_period` | none | Returns the review window in seconds |
| `get_arbiter_fee_bps` | none | Returns `0` when no fee was configured |
| `get_action_summary` | none | Caller-specific action eligibility; `NotFound` for an unknown id |
| `get_basket_action_summary` | none | Basket-specific eligibility; no mutation |
| `get_status` | none | `NotFound` for an unknown id; covers both record kinds |
| `get_escrow` | none | `NotFound` for an unknown id or a basket id |
| `get_basket` | none | `NotFound` for an unknown id or a single-token id |
| `escrows_for_participant` | none | Paged creation-order ids; unknown participants read as an empty page |
| `ttl_info` | none | `NotFound` for an unknown or archived id; no TTL extension |

## Vesting

Crate `crates/vesting` (`soroban-forge-vesting`); generated client
`packages/typescript-sdk/src/clients/vesting.ts`. Single schedules record a
funder who does not sign creation; only the batch path pulls tokens at creation.

### Mutating entrypoints

| Entrypoint | Required signer(s) | Failure behavior | Token transfer authority |
| --- | --- | --- | --- |
| `create_schedule` | beneficiary (the `funder` argument is recorded, not authorized) | `InvalidInput` (non-positive amount, zero duration, cliff after duration, funder equals beneficiary); `ArithmeticOverflow` (id counter) | none; the grant must be funded into custody out of band |
| `create_tranche_schedule` | beneficiary | `InvalidInput` (empty table, more than 32 tranches, non-positive amount, non-increasing offsets); `ArithmeticOverflow` | none; the grant must be funded into custody out of band |
| `create_schedules_batch` | funder | `InvalidInput` (empty or more than 10 entries, mixed tokens, invalid entry); `ArithmeticOverflow` (aggregate); `TokenTransferFailed` | funder → contract, aggregate transfer before any record is written |
| `revoke` | funder recorded on the linear schedule | `NotFound` (unknown or tranche id); `InvalidInput` (already `Completed` or `Revoked`); authorization failure for a non-funder | none; freezes the vested amount only |
| `refund_revoked` | funder recorded on the linear schedule | `NotFound`; `InvalidInput` (not `Revoked`); `ArithmeticOverflow`; `TokenTransferFailed` | contract → funder, unvested remainder |
| `reassign_beneficiary` | funder recorded on the linear schedule | `NotFound`; `InvalidInput` (revoked or invalid new beneficiary); authorization failure for a non-funder | none; moves future accrual, not tokens |
| `revoke_tranche_schedule` | funder recorded on the tranche schedule | `NotFound` (unknown or linear id); `InvalidInput` (`Completed`, already `Revoked`, or every tranche already unlocked); `ArithmeticOverflow`; `TokenTransferFailed` (schedule left unchanged) | contract → funder, locked remainder, transferred before the schedule is written |
| `propose_reassignment` | funder recorded on the linear schedule | `NotFound`; `InvalidInput` (revoked or completed schedule, or invalid new beneficiary); authorization failure for a non-funder | none; stores a pending proposal, the current beneficiary keeps accruing |
| `accept_reassignment` | pending beneficiary | `NotFound`; `InvalidInput` (no pending proposal or schedule no longer reassignable); `ArithmeticOverflow`; authorization failure for anyone else | none; vested balance stays with the former beneficiary, future accrual moves |
| `cancel_reassignment` | funder recorded on the linear schedule | `NotFound`; `InvalidInput` (no pending proposal); authorization failure for a non-funder | none; clears the pending proposal |
| `claim` | current beneficiary | `NotFound`; `TokenTransferFailed`; `ArithmeticOverflow`; a zero claim returns before any transfer | contract → current beneficiary |
| `claim_for` | explicitly named beneficiary, including a former beneficiary | `NotFound`; `TokenTransferFailed`; `ArithmeticOverflow` | contract → named beneficiary |

### Read-only queries

| Entrypoint | Auth | Notes / failures |
| --- | --- | --- |
| `schedules_for_beneficiary` | none | Paged ids in creation order |
| `schedules_for_funder` | none | Paged ids in creation order |
| `get_schedule` | none | `NotFound` for an unknown id or a tranche id |
| `get_tranche_schedule` | none | `NotFound` for an unknown id or a linear id |
| `claimable` | none | Current beneficiary's claimable amount; kind-aware |
| `claimable_for` | none | Named beneficiary's claimable amount, including frozen balances |
| `get_status` | none | `NotFound` for an unknown id |
| `get_schedule_status` | none | Derived status summary of a linear schedule (cliff, claimable, claimed); `NotFound` for an unknown or tranche id; `ArithmeticOverflow` |
| `pending_beneficiary` | none | Proposed beneficiary, if any; `NotFound` for an unknown id |

## Multi-Sig Wallet

Crate `crates/multi-sig-wallet` (`soroban-forge-multi-sig-wallet`); generated
client `packages/typescript-sdk/src/clients/multisig.ts`. Submission and
confirmation are owner-gated proposals; only `execute` moves funds, after the
current threshold of distinct owners has approved and with no rejection.

### Mutating entrypoints

| Entrypoint | Required signer(s) | Failure behavior | Token transfer authority |
| --- | --- | --- | --- |
| `initialize` | none (first caller; one time only) | `AlreadyInitialized`; `InvalidInput` (empty owner set, duplicates, threshold zero or above owner count) | none |
| `pause_wallet` | threshold of distinct owners in `approvers` | `NotInitialized`; `InvalidInput` (already paused, duplicate/non-owner approver, threshold not met) | none |
| `resume_wallet` | threshold of distinct owners in `approvers` | `NotInitialized`; `InvalidInput` (not paused, duplicate/non-owner approver, threshold not met) | none |
| `submit` | submitter (an owner) | `NotInitialized`; `Unauthorized` (not an owner); `DeadlineReached` (expiry in the past) | none; records an opaque pending tx |
| `submit_call` | submitter (an owner) | `NotInitialized`; `Unauthorized`; `InvalidInput`; `DeadlineReached` | none; records a typed pending call |
| `confirm` | signer (an owner who has not already confirmed) | `NotFound`; `InvalidInput` (terminal tx, already signalled, duplicate); `Unauthorized`; `DeadlineReached` | none |
| `confirm_batch` | signer (an owner) for every listed transaction | `NotFound`; `InvalidInput` (empty or oversized batch, invalid entry); `Unauthorized`; `DeadlineReached` | none |
| `reject` | signer (an owner who has not already signalled) | `NotFound`; `InvalidInput` (already confirmed or rejected, terminal tx); `Unauthorized`; `DeadlineReached` | none; one rejection blocks execution |
| `cancel_tx` | recorded submitter only | `NotFound`; `Unauthorized` (not the submitter); `InvalidInput` (not `Pending`) | none |
| `expire_tx` | none (permissionless once the expiry timestamp is reached) | `NotFound`; `InvalidInput` (no expiry, not yet expired, or already `Executed`, `Rejected` or `Cancelled`); idempotent once `Expired` | none; persists `Expired` and clears confirmations without moving funds |
| `execute` | none (permissionless after threshold, with no rejection, while unpaused) | `NotInitialized`; `InvalidInput` (below threshold, terminal, paused); `DeadlineReached` (tx expired); `InsufficientFunds`; `WithdrawalLimitExceeded`; `ContractInvocationFailed` (target reverted) | contract → withdrawal destination; target calls run as the contract |
| `deposit` | `from` (any depositor, owner or third party) | `NotInitialized`; `InvalidInput` (non-positive amount); `TokenTransferFailed` | from → contract, into per-token custody |
| `submit_withdrawal` | submitter (an owner) | `NotInitialized`; `Unauthorized`; `InvalidInput` (non-positive amount); `ArithmeticOverflow`; `WithdrawalLimitExceeded` | none; intent only, re-checked at execution |
| `set_withdrawal_limit` | submitter (an owner) | `NotInitialized`; `Unauthorized`; `InvalidInput` (non-positive amount or zero window) | none; takes effect at execution |
| `remove_withdrawal_limit` | submitter (an owner) | `NotInitialized`; `Unauthorized` | none; takes effect at execution |
| `add_owner` | submitter (an owner) | `NotInitialized`; `Unauthorized`; `InvalidInput` (already an owner) | none; insertion happens at execution |
| `remove_owner` | submitter (an owner) | `NotInitialized`; `Unauthorized`; `InvalidInput` (non-member or final owner) | none; removal happens at execution |
| `set_threshold` | submitter (an owner) | `NotInitialized`; `Unauthorized`; `InvalidInput` (zero or above owner count); `ArithmeticOverflow` | none; change happens at execution under the old threshold |
| `touch_ttl` | none (keeper) | `NotFound` (no balance entry for the token) | none; extends the balance entry TTL only |
| `touch_tx_ttl` | none (keeper) | `NotFound` (no tx record or it expired) | none; extends the transaction entry TTL only |

### Read-only queries

| Entrypoint | Auth | Notes / failures |
| --- | --- | --- |
| `is_paused` | none | No errors |
| `get_withdrawal_limit` | none | Returns `None` when no limit is configured |
| `get_withdrawal_window` | none | Empty window for unknown token or uninitialized wallet; no pruning |
| `check_withdrawal` | none | Simulates submission policy and current funding; no storage write |
| `get_window_usage` | none | In-window total for a token; zero when no limit is configured |
| `balance` | none | Contract custody for a token; zero when never deposited |
| `get_threshold` | none | `NotInitialized` when no owner set exists |
| `get_tx` | none | `NotFound` for an unknown or expired transaction |
| `get_tx_preview` | none | `NotFound`; `NotInitialized` |
| `get_transactions` | none | `InvalidInput` for zero limit; `ArithmeticOverflow` on page range |
| `get_transactions_by_status` | none | `InvalidInput` for zero limit; `ArithmeticOverflow`; `NotInitialized` |
| `get_owners` | none | `NotInitialized` when no owner set exists |
| `is_owner` | none | Uninitialized wallet reads as `false` |
| `is_live` | none | `Pending` and not expired reads as `true` |
| `is_tx_live` | none | Read-only alias of `is_live` |
| `is_expired` | none | `true` when the effective status is `Expired`; `NotFound` for an unknown id |
| `get_confirmations` | none | `NotFound` for an unknown transaction |
| `get_rejections` | none | `NotFound` for an unknown transaction |
| `get_tx_count` | none | No errors |

## DAO Governance

Crate `crates/dao-governance` (`soroban-forge-dao-governance`); generated client
`packages/typescript-sdk/src/clients/dao.ts`. Configuration and finalisation are
permissionless; proposals and votes are bound to the named proposer or voter.
The proposal bond is the only token custody in this contract.

### Mutating entrypoints

| Entrypoint | Required signer(s) | Failure behavior | Token transfer authority |
| --- | --- | --- | --- |
| `initialize` | none (first caller fixes the governance token) | `AlreadyInitialized` | none |
| `configure_bond` | none (first caller fixes token, amount, and treasury) | `AlreadyInitialized`; `InvalidInput` (non-positive bond) | none |
| `set_governance_address` | current governance address (the contract itself until one is set) | host authorization failure for anyone else; returns no `ForgeError` of its own and never `NotInitialized`, since governance defaults to the contract address | none |
| `set_governance_policy` | governance address (the contract itself until one is set) | `InvalidInput` (quorum above 10 000 bps, negative minimum participation, zero voting period); authorization failure | none; applies to proposals created afterwards |
| `propose` | proposer | `NotInitialized` (no bond configured); `InvalidInput` (zero duration or invalid dependency edge); `NotFound` (unknown dependency); `ProposerCooldown`; `ArithmeticOverflow`; `TokenTransferFailed` | proposer → contract, bond pulled before any state write |
| `propose_with_delay` | proposer | Same failures as `propose`, plus `InvalidInput` for an invalid voting delay | proposer → contract, bond pulled before the proposal is stored |
| `vote` | voter | `NotInitialized`; `InvalidInput` (inactive proposal, duplicate vote, zero token balance); `DeadlineReached`; `ArithmeticOverflow` | none |
| `revise_vote` | voter who already voted | `InvalidState` (proposal not `Active`); `InvalidInput` (no prior vote or same direction); `DeadlineReached`; `ArithmeticOverflow` | none; moves the weight locked at first vote between tally sides |
| `execute` | none (permissionless after voting ends) | `NotFound`; `InvalidInput` (voting open, terminal, or conflict executed); `DeadlineReached` (required proposal not yet executed); `ContractInvocationFailed` (target reverted or dependency blocked); `TokenTransferFailed` (bond refund or forfeit); `ArithmeticOverflow` | contract → proposer on success or cancel, contract → treasury on defeat |
| `cancel_proposal` | original proposer only | `NotFound`; `Unauthorized` (not the proposer); `InvalidInput` (not `Active`); `TokenTransferFailed` | contract → proposer, bond refunded before the state write |
| `touch_ttl` | none (keeper) | `NotFound` (no proposal record) | none; extends the persistent entry TTL only |

### Read-only queries

| Entrypoint | Auth | Notes / failures |
| --- | --- | --- |
| `get_bond_config` | none | `NotInitialized` when no bond was configured |
| `get_dependencies` | none | `NotFound` for an unknown proposal |
| `get_proposal` | none | `NotFound` for an unknown proposal |
| `get_proposal_status` | none | `NotFound`; `DeadlineReached` on lazy status evaluation |
| `get_proposal_count` | none | No errors |
| `get_proposals` | none | `InvalidInput` for zero limit; `ArithmeticOverflow`; `NotFound` |
| `has_voted` | none | `NotFound`; `NotInitialized` |
| `get_vote_receipt` | none | `NotFound` for an unknown proposal; no-vote receipt otherwise |
| `get_vote_snapshot` | none | Snapshot ledger, timestamp and weight; zero weight for a non-voter; `NotFound` for an unknown proposal |
| `get_voter_weight` | none | Recorded weight, `0` for a non-voter; `NotFound` for an unknown proposal |
| `get_vote_revision_count` | none | Number of `revise_vote` calls; `NotFound` for an unknown proposal |
| `get_proposal_snapshot` | none | Snapshot ledger and timestamp taken at creation; `NotFound` |
| `get_proposal_policy` | none | Governance policy snapshotted onto the proposal; `NotFound` |
| `get_voting_start` | none | Voting start timestamp (`0` when not delayed); `NotFound` |
| `get_governance_policy` | none | Currently configured governance policy |
| `get_active_proposal_count` | none | No errors |

## Subscription Payments

Crate `crates/subscription-payments` (`soroban-forge-subscription-payments`);
generated client `packages/typescript-sdk/src/clients/subscription.ts`.
Subscribers own the agreement and prepaid balance; providers meter usage and
initiate charges. Provider-initiated creation requires a stored subscriber
opt-in.

### Mutating entrypoints

| Entrypoint | Required signer(s) | Failure behavior | Token transfer authority |
| --- | --- | --- | --- |
| `set_grace_period` | subscriber | `NotFound`; `InvalidInput` (not `Active` or grace above the cap) | none |
| `propose_provider` | subscriber | `NotFound`; `InvalidInput` (cancelled or same provider) | none |
| `cancel_provider_change` | subscriber | `NotFound`; no error when no proposal exists | none |
| `accept_provider` | nominated provider | `NotFound`; `InvalidInput` (cancelled or no matching proposal) | none |
| `set_rollover_cap` | subscriber | `NotFound`; `InvalidInput` (not `Active`, usage recorded, cap overflow, unknown metric) | none |
| `subscribe` | subscriber | `InvalidInput` (non-positive amount or period) | none at creation; later pulls are subscriber-authorized |
| `subscribe_with_trial` | subscriber | `InvalidInput` (non-positive amount or period, zero trial, or a trial longer than 90 days); `ArithmeticOverflow` (trial end overflows) | none at creation; nothing is billed before the trial ends |
| `authorize_provider` | subscriber | `NotFound`; `Unauthorized` (not the subscriber); idempotent | none; records provider opt-in |
| `revoke_provider` | subscriber | `NotFound`; `Unauthorized` (not the subscriber); idempotent | none; removes provider opt-in |
| `subscribe_on_behalf_of` | provider, plus a prior subscriber opt-in for that provider | `Unauthorized` (not the provider or no opt-in); `InvalidInput` (non-positive amount or period) | none at creation; later pulls are subscriber-authorized |
| `change_plan` | subscriber and provider | `NotFound`; `InvalidInput` (invalid amount or state) | none |
| `charge` | provider | `NotFound`; `InvalidInput` (not due, paused, cancelled, or past due); `ArithmeticOverflow`; `TokenTransferFailed` | pull mode: subscriber → provider; prepaid mode: contract → provider |
| `deposit` | subscriber | `NotFound`; `InvalidInput` (cancelled or non-positive amount); `ArithmeticOverflow`; `TokenTransferFailed` | subscriber → contract, credited as prepaid balance |
| `withdraw_balance` | subscriber | `NotFound`; `InsufficientFunds`; `InvalidInput` (non-positive amount); `TokenTransferFailed` | contract → subscriber |
| `configure_auto_topup` | subscriber | `NotFound`; `Unauthorized`; `InvalidInput` (non-positive topup, negative threshold, cancelled) | none now; authorizes a subscriber → contract pull during a later charge |
| `disable_auto_topup` | subscriber | `NotFound`; `Unauthorized`; idempotent | none |
| `charge_catchup` | provider | `NotFound`; `InvalidInput` (past due, bad period count, cancelled, paused); `ArithmeticOverflow`; `TokenTransferFailed` | pull mode: subscriber → provider; prepaid mode: contract → provider |
| `set_quotas` | subscriber | `NotFound`; `InvalidInput` (not `Active`, usage already recorded, invalid quota); `Unauthorized` | none |
| `record_usage` | provider | `NotFound`; `InvalidInput` (zero units, not `Active`, undeclared metric); `Unauthorized`; `ArithmeticOverflow` | none |
| `pause` | subscriber | `NotFound`; `InvalidInput` (not `Active`) | none |
| `resume` | subscriber | `NotFound`; `InvalidInput` (not `Paused`); `ArithmeticOverflow` | none |
| `cancel` | subscriber | `NotFound`; `InvalidInput` (already `Cancelled`) | none; prepaid balance is not refunded here |
| `cancel_by_provider` | provider | `NotFound`; `InvalidInput` (already `Cancelled`); `TokenTransferFailed` | contract → subscriber, unspent prepaid balance |
| `touch_ttl` | none (keeper) | `NotFound` (no subscription record) | none; extends the persistent entry TTL only |

### Read-only queries

| Entrypoint | Auth | Notes / failures |
| --- | --- | --- |
| `grace_deadline` | none | `NotFound` for an unknown subscription |
| `pending_provider` | none | `NotFound`; returns `None` when no change is pending |
| `rollover_remaining` | none | `NotFound`; `InvalidInput` for an unknown metric |
| `is_provider_authorized` | none | No errors |
| `preview_catchup` | none | `NotFound`; `ArithmeticOverflow` on projected totals |
| `get_auto_topup` | none | Returns `None` when auto-topup is not configured |
| `get_usage` | none | `NotFound`; an unused metric reads as a zeroed record |
| `quote_period` | none | `NotFound`; uses the same derivation as `charge` |
| `get_subscription` | none | `NotFound` for an unknown subscription |
| `get_trial_end` | none | Trial end timestamp, or `None` when the subscription has no trial |
| `get_subscription_count` | none | No errors |
| `subscriptions_for_subscriber` | none | `InvalidInput` for zero limit; empty page when out of bounds |
| `subscriptions_for_provider` | none | `InvalidInput` for zero limit; empty page when out of bounds |

## Marketplace Royalties

Crate `crates/marketplace-royalties` (`soroban-forge-marketplace-royalties`);
generated client `packages/typescript-sdk/src/clients/marketplace.ts`. The
collection authorizes configuration; the collection and payer both authorize
settlement. One payer signature covers every nested token transfer in a batch.

### Mutating entrypoints

| Entrypoint | Required signer(s) | Failure behavior | Token transfer authority |
| --- | --- | --- | --- |
| `set_royalty` | collection | `InvalidInput` (bps above 10,000); replaces any multi-recipient split | none |
| `set_royalty_splits` | collection | `InvalidInput` (empty or oversized split, invalid bps) | none |
| `set_royalty_tiers` | collection | `InvalidInput` (first minimum not `1`, non-increasing minimums, bps above 10,000 or the collection's cap, too many tiers); an empty table clears tiers | none |
| `clear_royalty_tiers` | collection | host authorization failure only; returns no `ForgeError` (clearing with no tiers set is not `NotFound`) | none |
| `propose_recipient_rotation` | current royalty recipient | `NotFound` (`current` is not a recipient of the collection); `InvalidInput` (new recipient equals current); replaces an earlier pending rotation | none |
| `accept_recipient_rotation` | proposed new recipient | `NotFound` (no pending rotation or no royalty for the collection); `InvalidInput` (the new recipient already holds another split entry) | none; rewrites the split entries, keeping bps and order |
| `cancel_recipient_rotation` | current royalty recipient | `NotFound` (no pending rotation) | none |
| `set_max_royalty_cap` | collection | `InvalidInput` (bps above 10,000) | none |
| `distribute` | collection and payer | `NotFound` (no royalty config); `InvalidInput` (non-positive amount); `TokenTransferFailed` | payer → royalty recipient; the seller net is not transferred here |
| `settle_sale` | collection and payer | `NotFound`; `InvalidInput` (non-positive amount); `ArithmeticOverflow`; `TokenTransferFailed` | payer → seller, then payer → each royalty recipient |
| `settle_sales` | collection and payer (one authorization covers the batch) | `NotFound`; `InvalidInput` (empty or oversized batch, non-positive amount); `ArithmeticOverflow`; `TokenTransferFailed` | payer → every seller, then payer → each royalty recipient; all-or-nothing |
| `touch_ttl` | none (keeper) | `NotFound` (no royalty config for the collection) | none; extends persistent entry TTLs only |

### Read-only queries

| Entrypoint | Auth | Notes / failures |
| --- | --- | --- |
| `get_max_royalty_cap` | none | Returns 10,000 bps when no cap is configured |
| `get_royalty` | none | `NotFound` for an unknown collection; legacy view of a split |
| `get_royalty_splits` | none | `NotFound` for an unknown collection |
| `get_royalty_tiers` | none | Configured price-bracket tiers; empty when none are set |
| `get_pending_rotation` | none | Proposed new recipient, or `None` when no rotation is pending |
| `get_settlement_summary` | none | `NotFound` until the first successful settlement |
| `quote_sale` | none | `NotFound`; `InvalidInput` (non-positive amount); `ArithmeticOverflow` |
| `preview_settlement` | none | `NotFound`; `InvalidInput` (non-positive amount) |

## Token transfer authority summary

| Custody model | Contracts | Who can move tokens out | Authority source |
| --- | --- | --- | --- |
| Escrow holds the funded amount | `crates/escrow` | buyer, arbiter, or a permissionless post-deadline keeper | buyer on `deposit` and direct releases; arbiter on `resolve*`; no signer on `refund_expired` and `execute_release` |
| Vesting holds granted tokens | `crates/vesting` | current or former beneficiary, and funder on a revoked refund | beneficiary on `claim`/`claim_for`; funder on `refund_revoked`; funder on `create_schedules_batch` for the initial pull |
| Wallet holds arbitrary SEP-41 balances | `crates/multi-sig-wallet` | threshold-approved withdrawal only | any depositor signs `deposit`; `execute` is permissionless but enforces the current threshold and withdrawal limit |
| DAO holds proposal bonds | `crates/dao-governance` | proposer on success or cancel, treasury on defeat | proposer on `propose`; `execute` and `cancel_proposal` settle custody |
| Subscription holds optional prepaid balances | `crates/subscription-payments` | provider on charge, subscriber on withdraw or provider-initiated cancel | provider on `charge`/`charge_catchup`; subscriber on `deposit`, `withdraw_balance`, and opt-in |
| Marketplace has no custody | `crates/marketplace-royalties` | payer funds every sale transfer | collection and payer on `distribute`, `settle_sale`, and `settle_sales` |

No contract has an admin key that can withdraw another account's funds. A
failed SEP-41 transfer returns `TokenTransferFailed` and reverts the invocation;
diagnostic events retain the token's original error.

## Shared failures

All six contracts use `ForgeError` from `crates/shared-utils/src/errors.rs`:
`Unauthorized`, `NotFound`, `InvalidInput`, `InsufficientFunds`,
`AlreadyInitialized`, `NotInitialized`, `DeadlineReached`,
`InsufficientAllowance`, `ArithmeticOverflow`, `Custom`, `TokenTransferFailed`,
`ContractInvocationFailed`, `WithdrawalLimitExceeded`, `SubscriptionPastDue`,
`ProposerCooldown`, `ReleasePending`, and `InvalidState`.
