# Multi-Signature Wallet Contract

Multi-owner wallet with configurable approval threshold and transaction queue.

- **Source:** `crates/multi-sig-wallet`
- **Client:** `SorobanForgeMultiSigWalletClient` (generated)
- -**Related:** [Contract index](./index.md), [Feature Status Matrix](../FEATURE-STATUS.md), [Known Limitations](../KNOWN-LIMITATIONS.md), [DAO Governance](./dao-governance.md)

## Interface

```rust
fn initialize(owners: Vec<Address>, threshold: u32) -> Result<(), ForgeError>
fn submit(submitter: Address, target: Address, payload: Bytes, expiry: Option<u64>) -> Result<u64, ForgeError>
fn submit_call(submitter: Address, target: Address, fn_name: Symbol, args: Vec<Val>, expiry: Option<u64>) -> Result<u64, ForgeError>
fn confirm(tx_id: u64, signer: Address) -> Result<(), ForgeError>
fn reject(tx_id: u64, signer: Address) -> Result<(), ForgeError>
fn execute(tx_id: u64) -> Result<(), ForgeError>
fn get_tx(tx_id: u64) -> Result<WalletTx, ForgeError>
fn get_tx_preview(tx_id: u64) -> Result<TxPreview, ForgeError>
fn is_live(tx_id: u64) -> bool
fn is_tx_live(tx_id: u64) -> bool
fn add_owner(proposer: Address, new_owner: Address) -> Result<u64, ForgeError>
fn remove_owner(proposer: Address, owner: Address) -> Result<u64, ForgeError>
fn set_threshold(proposer: Address, new_threshold: u32) -> Result<u64, ForgeError>
```

The wallet is configured **once** via `initialize(owners, threshold)`
(first caller wins; re-initialisation is rejected). `submit` records a
`target` contract and an opaque payload with an optional `expiry` timestamp (`None` for immortal transactions); `submit_call` does the same for typed contract invocations. `confirm` collects approvals until
the threshold is reached; `execute` then performs a real cross-contract
invocation to the recorded target (an opaque `TxKind::Data` tx) or moves
real tokens (a typed `TxKind::Withdrawal` tx). A target revert surfaces as
`ForgeError::ContractInvocationFailed` and leaves the tx `Pending` and
retryable. Any owner may `reject` a pending tx; one rejection immediately
makes it `Rejected` and terminal. The rejector and any existing confirmations
remain on the record for audit, and `get_tx` continues to expose it.
Confirming, rejecting, or executing a rejected transaction returns
`ForgeError::InvalidInput` without changing the record.
`ForgeError::ContractInvocationFailed` and leaves the tx `Pending` and retryable. `reject` records a formal objection; any rejection blocks
execution, and reaching the rejection threshold makes the tx `Rejected`
(terminal). If a transaction expires before meeting threshold, its status transitions lazily to `Expired` and further confirmations are refused.

## Transaction metadata

Transactions carry optional metadata for treasury context (accounting
references, human-readable descriptions, off-chain documentation links):

- `memo: String` - optional short human-readable description, max 128 chars.
- `metadata: String` - optional extended information blob, max 512 chars.

Both fields are stored on the transaction record at submission time and
preserved through the transaction lifecycle (`Pending` -> `Executed` /
`Rejected`). They are returned by `get_tx` and the queue views. Submission
rejects over-length values with `ForgeError::InvalidInput`. Metadata is not
encrypted and no on-chain document storage is performed; use external
IPFS/URLs for large payloads. The `submit` event includes a metadata hash
for verification.

## Transaction query views

The wallet exposes read-only views for loading the transaction queue without
one contract call per transaction:

```rust
fn get_transactions(offset: u32, limit: u32) -> Result<Vec<WalletTx>, ForgeError>
fn get_transactions_by_status(
    status: TxStatus,
    offset: u32,
    limit: u32,
) -> Result<Vec<WalletTx>, ForgeError>
fn get_tx_preview(tx_id: u64) -> Result<TxPreview, ForgeError>
```

Both return transactions in ascending `tx_id` order. For
`get_transactions`, `offset` is zero-based in the complete sequence (so
offset `0` starts at transaction id `1`). For
`get_transactions_by_status`, `offset` is zero-based among transactions
matching the requested status. Each returns at most `limit` records; ranges
past the end return an empty vector or the remaining records. A `limit` of
zero returns `ForgeError::InvalidInput`. An uninitialized or empty wallet
returns an empty vector for a positive limit. These views do not require
authorization and do not modify contract state.

`get_tx_preview` returns a `TxPreview` with the transaction `status`, current
wallet `threshold`, number of recorded distinct `approvals`, and
`approvals_remaining` (zero once the threshold is met). Its status follows the
same lazy expiry evaluation as `get_tx`; an unknown transaction ID returns
`ForgeError::NotFound`. This view is read-only and requires no authorization.

Additional views: `get_threshold`, `get_owners`, `is_owner`,
`get_confirmations`, `get_rejections`, `get_tx_count`, `get_tx`, and the
per-token views `get_withdrawal_limit` / `get_withdrawal_window` /
`get_window_usage` / `check_withdrawal` / `balance`.

## Withdrawal-limit views

```rust
fn get_withdrawal_limit(token: Address) -> Option<WithdrawalLimit>
fn get_withdrawal_window(token: Address) -> WindowState
fn check_withdrawal(token: Address, amount: i128) -> CheckResult
```

These views require no authorization and do not modify or prune storage.
`get_withdrawal_limit` returns `None` when no policy is configured. The
window view reports the active total (including pending withdrawals), the
oldest active submission time as `window_start`, and that entry's projected
expiry as `reset_at`. Entries expire at `submitted_at + window_seconds`, so
they still count one second before reset and are excluded exactly at reset.
As entries expire at different times, `reset_at` describes the next expiry,
not a time when the entire total necessarily becomes zero. With no configured
limit, no active entries, an unknown token, or an uninitialized wallet, the
window is empty (`total = 0`, timestamps absent). Removing a limit makes the
limit and window views return `None` and an empty window; retained history is
still used if a limit is configured again.

`check_withdrawal` returns `CheckResult::Allowed` or a denial variant:
`WalletNotInitialized`, `InvalidAmount`, `LimitExceeded`,
`InsufficientFunds`, or `ArithmeticOverflow`. It checks initialization,
positive amount, the same rolling-limit arithmetic as submission, then the
wallet's recorded custody balance. This is a snapshot at the current ledger
timestamp, not a promise about later execution: submission reserves window
capacity, and both the rolling limit and custody funding are rechecked when
the approved transaction executes (see below). The generated TypeScript client has not been regenerated for these
new views; future client regeneration will include them.

Example: with a 1,000-unit limit per 600 seconds, a 700-unit withdrawal
submitted at timestamp 10,000 produces `total = 700`, `window_start = 10,000`,
and `reset_at = 10,600`. A check for 400 at 10,599 returns
`CheckResult::LimitExceeded`, matching submission enforcement. At 10,600 the
700-unit entry is expired; the same check is allowed if the wallet has enough
recorded balance.

### Execution-time limit re-check

`execute` re-checks a withdrawal against the token's limit **in force at
execution**, before custody funding, so a proposal approved under an older,
higher limit cannot settle past a lowered one. A withdrawal is never counted
twice:

- Admitted at submission and still inside the current window — it is
  already in the window total, so execution requires `total <= limit`.
- Not counted — submitted while the token had no limit, or more than one
  window ago — it is checked like a new submission (`total + amount <=
  limit`) and recorded in the window when it executes.
- No limit configured at execution — no limit check.

A failed re-check returns `WithdrawalLimitExceeded` and leaves the
transaction `Pending` with nothing moved; owners can raise the limit, wait
for the window to clear, or `reject` it. Which withdrawals were admitted is
tracked per transaction under the persistent `WithdrawalAdmission(tx_id)` key
(removed on execution), so the `WalletTx` and `WindowEntry` shapes are
unchanged.

## States

- `Pending` — Awaiting approvals
- `Executed` — Threshold met and the transaction completed
- `Rejected` — Vetoed by an owner; terminal; rejector and existing confirmations retained
- `Pending` — Awaiting approvals; liveness view `is_live(tx_id)` reads `true`.
- `Executed` — Threshold met and the transaction completed (terminal).
- `Rejected` — Rejection threshold met (terminal).
- `Expired` — Expiry deadline reached before meeting approval threshold; evaluated lazily on read/access without requiring an external keeper.

### Transaction Lifecycle and Expiry Semantics

Transactions can optionally specify an `expiry: Option<u64>` (ledger timestamp) upon submission (`None` preserves immortal behavior):
- **Submission validation**: Submitting with `expiry <= env.ledger().timestamp()` is rejected with `ForgeError::DeadlineReached`.
- **Lazy evaluation**: When accessing a transaction (`get_tx`, `is_live`, etc.), if `now >= expiry` and confirmations have not reached `threshold`, the transaction status surfaces lazily as `TxStatus::Expired`.
- **Liveness commitment**: A transaction that reaches the confirmation threshold *before* its expiry deadline remains executable even after the deadline passes (`execute` succeeds; status remains `Pending` until execution completes).
- **Boundary-pinned confirmation**: Approvals (`confirm`) attempted at or after the expiry deadline (`now >= expiry`) are rejected with `ForgeError::DeadlineReached`, leaving the transaction's confirmation set unchanged.
- **Terminal guard**: `execute` and `reject` calls on an expired transaction fail with `ForgeError::DeadlineReached`.
- **Queue query**: `get_transactions_by_status(TxStatus::Expired, ...)` paginates expired transactions seamlessly.

| State Transition | Before Expiry | At Expiry Boundary (`now == expiry`) | After Expiry (`now > expiry`) |
|---|---|---|---|
| `Pending` (below threshold) | `Pending`, `is_live == true` | `Expired`, `is_live == false` | `Expired`, `is_live == false` |
| `confirm` | Succeeds, records approval | Fails (`DeadlineReached`) | Fails (`DeadlineReached`) |
| `execute` (below threshold) | Fails (`InvalidInput`) | Fails (`DeadlineReached`) | Fails (`DeadlineReached`) |
| `Pending` (threshold met before expiry) | `Pending`, executable | `Pending`, executable | `Pending`, executable |
| `execute` (threshold met before expiry) | Succeeds -> `Executed` | Succeeds -> `Executed` | Succeeds -> `Executed` |

## Storage & TWL Maintenance

Transaction records (`DataKey::Tx(u64)`) are stored in **persistent
storage**: `submit`, `confirm`, `reject`, `execute`, `submit_withdrawal`,
and the limit-change paths write through `.persistent()` and bump the
entry's TWL to a 30-day horizon on every write. `get_tx` reads from
persistent storage. Owners, threshold, per-token balances, and withdrawal
limits remain in instance storage.

A permissionless public keeper entrypoint `touch_tx_ttl(tx_id)` allows
anyone to bump a transaction's persistent TTL without modifying its state;
an unknown `tx_id` returns `ForgeError::NotFound`. A separate
`touch_ttl(token)` keeper extends the persistent balance entries' TTL.

<div id="task-247"></div>

### Confirm several transactions

Call `confirm_batch(tx_ids, signer)` with 1–16 distinct transaction IDs. One
current owner authorizes the entire list. The wallet validates every ID before
recording approvals, then emits one ordinary `TxConfirmed` per transaction in
input order. An empty list, duplicate ID or oversized list returns
`InvalidInput`. For each entry the same eligibility rules apply as for
`confirm`. An invalid entry leaves every approval and event untouched.

Owners can confirm during a pause. Execution is a separate call and checks
the current threshold, pause, withdrawal limit and available balance. Removing
an owner clears their pending approvals, including those made in a batch.
