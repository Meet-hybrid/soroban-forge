import { Buffer } from "buffer";
import { Address } from "@stellar/stellar-sdk";
import {
  AssembledTransaction,
  Client as ContractClient,
  ClientOptions as ContractClientOptions,
  MethodOptions,
  Result,
  Spec as ContractSpec,
} from "@stellar/stellar-sdk/contract";
import type {
  u32,
  i32,
  u64,
  i64,
  u128,
  i128,
  u256,
  i256,
  Option,
  Timepoint,
  Duration,
} from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";

if (typeof window !== "undefined") {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}





/**
 * A single governance proposal.
 */
export interface Proposal {
  /**
 * Encoded action to execute on success.
 */
action: Buffer;
  /**
 * Tally of "against" votes (in governance-token units).
 */
against_votes: i128;
  /**
 * Bond amount posted at creation; the exact figure refunded or
 * forfeited on the terminal transition.
 */
bond_amount: i128;
  /**
 * Lifecycle of this proposal's bond. Moves in the same frame as the
 * terminal proposal state, exactly once.
 */
bond_state: BondState;
  /**
 * SEP-41 token this proposal's bond was posted in (the configured
 * token at creation time).
 */
bond_token: string;
  /**
 * Proposal whose execution blocks this one.
 */
conflicts_with: Option<u64>;
  /**
 * Tally of "for" votes (in governance-token units).
 */
for_votes: i128;
  /**
 * Stable identifier assigned at creation.
 */
proposal_id: u64;
  /**
 * Address that created the proposal.
 */
proposer: string;
  /**
 * Proposals that must execute before this one.
 */
requires: Array<u64>;
  /**
 * Current state.
 */
state: ProposalState;
  /**
 * Target contract to invoke on successful execution.
 */
target: string;
  /**
 * Ledger timestamp at which voting closes.
 */
voting_ends: u64;
}

/**
 * Lifecycle state of the bond posted for a proposal.
 */
export type BondState = {tag: "Posted", values: void} | {tag: "Refunded", values: void} | {tag: "Forfeited", values: void};


/**
 * The one-time proposal-bond configuration.
 *
 * Written once by [`DaoGovernance::configure_bond`] and immutable
 * afterwards; read by `propose` (what to pull) and by the terminal
 * transitions (where a forfeit goes).
 */
export interface BondConfig {
  /**
 * Amount of `token` pulled from the proposer on `propose`.
 */
amount: i128;
  /**
 * SEP-41 token every proposal must post as a bond.
 */
token: string;
  /**
 * Receives bonds forfeited by defeated proposals. Never receives
 * refunds and holds no other authority.
 */
treasury: string;
}


/**
 * Immutable receipt for a single proposal vote.
 *
 * `support` and `weight` are the values recorded at vote time. They are never
 * recalculated from the voter's current token balance.
 */
export interface VoteReceipt {
  /**
 * Whether this voter cast a vote on the proposal.
 */
has_voted: boolean;
  /**
 * Recorded direction: `Some(true)` for, `Some(false)` against.
 * `None` means no vote was recorded. Abstain is not supported by `vote`.
 */
support: Option<boolean>;
  /**
 * Governance-token balance recorded when the vote was cast.
 */
weight: i128;
}

/**
 * Lifecycle state of a governance proposal.
 */
export type ProposalState = {tag: "Active", values: void} | {tag: "Succeeded", values: void} | {tag: "Defeated", values: void} | {tag: "Executed", values: void} | {tag: "Queued", values: void} | {tag: "Cancelled", values: void};


/**
 * Dependency edges exposed to indexers independently of the full proposal.
 */
export interface DependencyView {
  /**
 * Proposal whose execution blocks this proposal.
 */
conflicts_with: Option<u64>;
  /**
 * Proposals that must execute first.
 */
requires: Array<u64>;
}


/**
 * A read-only summary of a proposal's readiness state.
 *
 * Returned by [`SorobanForgeDaoGovernance::get_proposal_status`].
 * Distinguishes executable, retryable, expired, rejected, and terminal
 * states where applicable.
 */
export interface ProposalStatus {
  /**
 * Tally of "against" votes in governance-token units.
 */
against_votes: i128;
  /**
 * Tally of "for" votes in governance-token units.
 */
for_votes: i128;
  /**
 * Whether the proposal can be executed now (i.e. `Succeeded` with
 * dependencies met and voting deadline passed).
 */
is_executable: boolean;
  /**
 * Whether the proposal is in a retryable state (e.g. `Succeeded` whose
 * target invocation previously failed; bond still in custody).
 */
is_retryable: boolean;
  /**
 * Unique proposal identifier.
 */
proposal_id: u64;
  /**
 * Current lifecycle state of the proposal.
 */
state: ProposalState;
  /**
 * Ledger timestamp at which voting closes (0 if proposal is not `Active`).
 */
voting_ends: u64;
}






/**
 * Shared error type used across all Soroban Forge contracts.
 *
 * Defining a single error enum in `shared-utils` keeps the on-chain error
 * space consistent and intelligible to SDK consumers, and avoids every
 * contract re-declaring the same failure modes. Contract crates may expose
 * their own domain-specific errors, but should prefer these where they fit.
 *
 * Error codes start at 1; code 0 is reserved by the Soroban host.
 */
export const ForgeError = {
  /**
   * The caller is not permitted to perform this action.
   */
  1: {message:"Unauthorized"},
  /**
   * The requested entity (escrow, proposal, subscription, ...) does not exist.
   */
  2: {message:"NotFound"},
  /**
   * One or more arguments failed validation (e.g. zero amount, bad address).
   */
  3: {message:"InvalidInput"},
  /**
   * The contract does not hold enough balance to satisfy the operation.
   */
  4: {message:"InsufficientFunds"},
  /**
   * The entity was already initialised; re-initialisation is rejected.
   */
  5: {message:"AlreadyInitialized"},
  /**
   * The entity was expected to be initialised but was not.
   */
  6: {message:"NotInitialized"},
  /**
   * An operation was attempted after its deadline elapsed.
   */
  7: {message:"DeadlineReached"},
  /**
   * A required token allowance was lower than the amount being spent.
   */
  8: {message:"InsufficientAllowance"},
  /**
   * An arithmetic operation overflowed.
   */
  9: {message:"ArithmeticOverflow"},
  /**
   * Contract-specific error that does not map to the categories above.
   */
  10: {message:"Custom"},
  /**
   * A SEP-41 token invocation failed (insufficient balance, missing or
   * deauthorized trustline, undeployed token contract, or token-logic
   * rejection). The raw token error discriminant is intentionally not
   * forwarded — callers cannot tell which contract produced a forwarded
   * code, so it is bucketed; the root cause remains visible in the
   * transaction's diagnostic events.
   */
  11: {message:"TokenTransferFailed"},
  /**
   * A cross-contract invocation failed (target reverted or host
   * abort). The target reverts are surfaced here so the caller can
   * distinguish them from token-transfer failures, and the invoking
   * transaction is left un-executed.
   */
  12: {message:"ContractInvocationFailed"},
  /**
   * The requested withdrawal would push a token's rolling-window total
   * past its configured withdrawal limit. Kept distinct from
   * [`ForgeError::InvalidInput`] so a caller can tell a policy rejection
   * (a valid withdrawal that is too large right now) from a malformed
   * argument.
   */
  13: {message:"WithdrawalLimitExceeded"},
  /**
   * A subscription is in the `PastDue` lapsed state and the requested
   * operation cannot be performed until a catch-up charge restores it to
   * `Active`. Kept distinct from [`ForgeError::DeadlineReached`] so a caller
   * can tell a lapsed billing state from an argument that was merely
   * too early.
   */
  14: {message:"SubscriptionPastDue"},
  /**
   * A proposer has reached the maximum allowed concurrent active proposals
   * or is within the proposer cooldown window.
   */
  15: {message:"ProposerCooldown"},
  /**
   * A seller's release request is waiting out its review window, so the
   * buyer cannot take the funds back by refunding. The buyer can approve
   * the release or raise a dispute instead. Kept distinct from
   * [`ForgeError::InvalidInput`] so a wallet can explain why the refund
   * was refused and offer the dispute path.
   */
  16: {message:"ReleasePending"},
  /**
   * The current lifecycle state does not allow the operation.
   */
  17: {message:"InvalidState"}
}

export interface Client {
  /**
   * Construct and simulate a vote transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cast a vote on an active proposal.
   *
   * Requires the voter. Each voter may vote exactly once; voting is closed
   * once the deadline (`voting_ends`) passes.
   */
  vote: ({proposal_id, voter, support}: {proposal_id: u64, voter: string, support: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a execute transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Finalise a proposal once voting has ended, and execute passed proposals.
   *
   * Callable by anyone after the deadline (permissionless execution), and
   * — on the paths that release a bond — permissionless in the token
   * sense too: contract self-authorization covers the outgoing transfer.
   *
   * - An `Active` proposal past deadline transitions to `Succeeded` on a
   * strict majority of `for` votes (the bond stays in custody until a
   * terminal transition), or to `Defeated` otherwise — forfeiting the
   * bond to the treasury **before** the state write.
   * - A `Succeeded` proposal performs a real cross-contract call to `target`
   * with `action` (`target.execute(action)`). On successful invocation,
   * it refunds the bond to the proposer, then transitions to the
   * terminal `Executed` state and emits an `Executed` event.
   * - If the target invocation reverts, `ForgeError::ContractInvocationFailed`
   * is returned and the proposal remains `Succeeded` (not `Executed`),
   * leaving target state unchanged and the bond in custody.
   * - If a bond refund/forfeit trans
   */
  execute: ({proposal_id}: {proposal_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a propose transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new proposal and return its stable id.
   *
   * Requires `duration > 0` and a configured bond. The proposer is
   * authorized at creation time, and their authorization covers the
   * nested bond pull.
   *
   * Ordering (load-bearing): the bond transfer runs **before** the id
   * counter, the proposal record, and the custody total are written, so a
   * failed transfer leaves no proposal record — see the module docs.
   */
  propose: ({proposer, target, action, duration, requires, conflicts_with}: {proposer: string, target: string, action: Buffer, duration: u64, requires: Array<u64>, conflicts_with: Option<u64>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a has_voted transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Check whether `voter` has voted on `proposal_id` (read-only view).
   *
   * Returns [`ForgeError::NotFound`] if `proposal_id` does not exist.
   */
  has_voted: ({proposal_id, voter}: {proposal_id: u64, voter: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<boolean>>>

  /**
   * Construct and simulate a touch_ttl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Permissionless keeper: bump the proposal entry's TTL without changing
   * any state. The existence check is deliberate — touching a missing
   * id must fail loudly with `ForgeError::NotFound`.
   */
  touch_ttl: ({proposal_id}: {proposal_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Configure the governance token exactly once.
   */
  initialize: ({governance_token}: {governance_token: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_proposal transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read a stored proposal by id (read-only view).
   */
  get_proposal: ({proposal_id}: {proposal_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Proposal>>>

  /**
   * Construct and simulate a get_proposals transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read a paginated slice of proposals ordered by proposal ID (read-only view).
   *
   * Bounds clamping:
   * - `limit == 0` returns `ForgeError::InvalidInput`.
   * - If `offset >= total`, returns an empty `Vec`.
   * - Returns at most `limit` items without overflowing.
   */
  get_proposals: ({offset, limit}: {offset: u32, limit: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Array<Proposal>>>>

  /**
   * Construct and simulate a configure_bond transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Configure the proposal bond for the first and only time.
   *
   * Permissionless one-shot (see [`SorobanForgeDaoGovernance::configure_bond`]):
   * the deployer calls it in the deploy transaction and the configuration
   * never changes afterwards, so no privileged role exists.
   */
  configure_bond: ({token, amount, treasury}: {token: string, amount: i128, treasury: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a cancel_proposal transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Withdraw an active proposal before it is executed.
   *
   * Requires the original proposer. A proposal may be cancelled even after
   * voting ends and quorum is met, as long as it has not been executed (or
   * already cancelled). A cancelled proposal is terminal: further votes and
   * execution are rejected.
   *
   * The bond is refunded to the proposer in this same call: transfer
   * first, then the `Cancelled` state write — a failed refund surfaces
   * `TokenTransferFailed` with the proposal still `Active`.
   *
   * * [`ForgeError::NotFound`] — no proposal with this id.
   * * [`ForgeError::Unauthorized`] — `proposer` is not the original proposer.
   * * [`ForgeError::InvalidInput`] — the proposal is no longer `Active`.
   * * [`ForgeError::TokenTransferFailed`] — the bond refund failed.
   */
  cancel_proposal: ({proposal_id, proposer}: {proposal_id: u64, proposer: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_bond_config transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the bond configuration (read-only view).
   *
   * Returns `ForgeError::NotInitialized` while the contract has no bond
   * configuration — the same precondition `propose` enforces.
   */
  get_bond_config: (options?: MethodOptions) => Promise<AssembledTransaction<Result<BondConfig>>>

  /**
   * Construct and simulate a get_dependencies transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return a proposal's dependency edges for indexers.
   */
  get_dependencies: ({proposal_id}: {proposal_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<DependencyView>>>

  /**
   * Construct and simulate a get_vote_receipt transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the immutable vote receipt recorded for `voter` on `proposal_id`.
   *
   * This is a read-only view: it never writes storage, extends TTLs, or
   * recalculates voting power. A voter that has not voted receives
   * `has_voted: false`, `support: None`, and `weight: 0`. A malformed
   * `voter` address is rejected by the Soroban host before this query runs
   * (the shared error registry documents bad addresses as `InvalidInput`).
   *
   * # Errors
   *
   * * [`ForgeError::NotFound`] — no proposal with this id.
   */
  get_vote_receipt: ({proposal_id, voter}: {proposal_id: u64, voter: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<VoteReceipt>>>

  /**
   * Construct and simulate a get_proposal_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the total number of proposals created (read-only view).
   */
  get_proposal_count: (options?: MethodOptions) => Promise<AssembledTransaction<u64>>

  /**
   * Construct and simulate a get_proposal_status transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read a read-only proposal status summary (read-only view).
   *
   * Pure view: never writes storage, extends TTLs, or emits events. An
   * unknown proposal yields [`ForgeError::NotFound`]. A `Succeeded`
   * proposal is executable once voting has closed and all dependencies
   * are satisfied; it stays retryable while its bond is still in custody
   * and no dependency has permanently blocked execution.
   */
  get_proposal_status: ({proposal_id}: {proposal_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<ProposalStatus>>>

  /**
   * Construct and simulate a get_active_proposal_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read how many active proposals `proposer` currently has in flight (read-only view).
   */
  get_active_proposal_count: ({proposer}: {proposer: string}, options?: MethodOptions) => Promise<AssembledTransaction<u32>>

}
export class Client extends ContractClient {
  static async deploy<T = Client>(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options: MethodOptions &
      Omit<ContractClientOptions, "contractId"> & {
        /** The hash of the Wasm blob, which must already be installed on-chain. */
        wasmHash: Buffer | string;
        /** Salt used to generate the contract's ID. Passed through to {@link Operation.createCustomContract}. Default: random. */
        salt?: Buffer | Uint8Array;
        /** The format used to decode `wasmHash`, if it's provided as a string. */
        format?: "hex" | "base64";
      }
  ): Promise<AssembledTransaction<T>> {
    return ContractClient.deploy(null, options)
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([ "AAAAAQAAAM9UaGUgb25lLXRpbWUgcHJvcG9zYWwtYm9uZCBjb25maWd1cmF0aW9uLgoKV3JpdHRlbiBvbmNlIGJ5IFtgRGFvR292ZXJuYW5jZTo6Y29uZmlndXJlX2JvbmRgXSBhbmQgaW1tdXRhYmxlCmFmdGVyd2FyZHM7IHJlYWQgYnkgYHByb3Bvc2VgICh3aGF0IHRvIHB1bGwpIGFuZCBieSB0aGUgdGVybWluYWwKdHJhbnNpdGlvbnMgKHdoZXJlIGEgZm9yZmVpdCBnb2VzKS4AAAAAAAAAAApCb25kQ29uZmlnAAAAAAADAAAAOEFtb3VudCBvZiBgdG9rZW5gIHB1bGxlZCBmcm9tIHRoZSBwcm9wb3NlciBvbiBgcHJvcG9zZWAuAAAABmFtb3VudAAAAAAACwAAADBTRVAtNDEgdG9rZW4gZXZlcnkgcHJvcG9zYWwgbXVzdCBwb3N0IGFzIGEgYm9uZC4AAAAFdG9rZW4AAAAAAAATAAAAZFJlY2VpdmVzIGJvbmRzIGZvcmZlaXRlZCBieSBkZWZlYXRlZCBwcm9wb3NhbHMuIE5ldmVyIHJlY2VpdmVzCnJlZnVuZHMgYW5kIGhvbGRzIG5vIG90aGVyIGF1dGhvcml0eS4AAAAIdHJlYXN1cnkAAAAT",
        "AAAAAgAAADJMaWZlY3ljbGUgc3RhdGUgb2YgdGhlIGJvbmQgcG9zdGVkIGZvciBhIHByb3Bvc2FsLgAAAAAAAAAAAAlCb25kU3RhdGUAAAAAAAADAAAAAAAAAFBIZWxkIGJ5IHRoaXMgY29udHJhY3Qgc2luY2UgYHByb3Bvc2VgOyB0aGUgb25seSBzdGF0ZSBhIGxpdmUKcHJvcG9zYWwgY2FuIGJlIGluLgAAAAZQb3N0ZWQAAAAAAAAAAABYUmV0dXJuZWQgdG8gdGhlIHByb3Bvc2VyIHdoZW4gdGhlIHByb3Bvc2FsIHJlYWNoZWQgYEV4ZWN1dGVkYCBvcgpgQ2FuY2VsbGVkYCAodGVybWluYWwpLgAAAAhSZWZ1bmRlZAAAAAAAAABMU2VudCB0byB0aGUgY29uZmlndXJlZCB0cmVhc3VyeSB3aGVuIHRoZSBwcm9wb3NhbCB3YXMgYERlZmVhdGVkYAoodGVybWluYWwpLgAAAAlGb3JmZWl0ZWQAAAA=",
        "AAAAAQAAAEhEZXBlbmRlbmN5IGVkZ2VzIGV4cG9zZWQgdG8gaW5kZXhlcnMgaW5kZXBlbmRlbnRseSBvZiB0aGUgZnVsbCBwcm9wb3NhbC4AAAAAAAAADkRlcGVuZGVuY3lWaWV3AAAAAAACAAAALlByb3Bvc2FsIHdob3NlIGV4ZWN1dGlvbiBibG9ja3MgdGhpcyBwcm9wb3NhbC4AAAAAAA5jb25mbGljdHNfd2l0aAAAAAAD6AAAAAYAAAAiUHJvcG9zYWxzIHRoYXQgbXVzdCBleGVjdXRlIGZpcnN0LgAAAAAACHJlcXVpcmVzAAAD6gAAAAY=",
        "AAAAAQAAAB1BIHNpbmdsZSBnb3Zlcm5hbmNlIHByb3Bvc2FsLgAAAAAAAAAAAAAIUHJvcG9zYWwAAAANAAAAJUVuY29kZWQgYWN0aW9uIHRvIGV4ZWN1dGUgb24gc3VjY2Vzcy4AAAAAAAAGYWN0aW9uAAAAAAAOAAAANVRhbGx5IG9mICJhZ2FpbnN0IiB2b3RlcyAoaW4gZ292ZXJuYW5jZS10b2tlbiB1bml0cykuAAAAAAAADWFnYWluc3Rfdm90ZXMAAAAAAAALAAAAYkJvbmQgYW1vdW50IHBvc3RlZCBhdCBjcmVhdGlvbjsgdGhlIGV4YWN0IGZpZ3VyZSByZWZ1bmRlZCBvcgpmb3JmZWl0ZWQgb24gdGhlIHRlcm1pbmFsIHRyYW5zaXRpb24uAAAAAAALYm9uZF9hbW91bnQAAAAACwAAAGhMaWZlY3ljbGUgb2YgdGhpcyBwcm9wb3NhbCdzIGJvbmQuIE1vdmVzIGluIHRoZSBzYW1lIGZyYW1lIGFzIHRoZQp0ZXJtaW5hbCBwcm9wb3NhbCBzdGF0ZSwgZXhhY3RseSBvbmNlLgAAAApib25kX3N0YXRlAAAAAAfQAAAACUJvbmRTdGF0ZQAAAAAAAFhTRVAtNDEgdG9rZW4gdGhpcyBwcm9wb3NhbCdzIGJvbmQgd2FzIHBvc3RlZCBpbiAodGhlIGNvbmZpZ3VyZWQKdG9rZW4gYXQgY3JlYXRpb24gdGltZSkuAAAACmJvbmRfdG9rZW4AAAAAABMAAAApUHJvcG9zYWwgd2hvc2UgZXhlY3V0aW9uIGJsb2NrcyB0aGlzIG9uZS4AAAAAAAAOY29uZmxpY3RzX3dpdGgAAAAAA+gAAAAGAAAAMVRhbGx5IG9mICJmb3IiIHZvdGVzIChpbiBnb3Zlcm5hbmNlLXRva2VuIHVuaXRzKS4AAAAAAAAJZm9yX3ZvdGVzAAAAAAAACwAAACdTdGFibGUgaWRlbnRpZmllciBhc3NpZ25lZCBhdCBjcmVhdGlvbi4AAAAAC3Byb3Bvc2FsX2lkAAAAAAYAAAAiQWRkcmVzcyB0aGF0IGNyZWF0ZWQgdGhlIHByb3Bvc2FsLgAAAAAACHByb3Bvc2VyAAAAEwAAACxQcm9wb3NhbHMgdGhhdCBtdXN0IGV4ZWN1dGUgYmVmb3JlIHRoaXMgb25lLgAAAAhyZXF1aXJlcwAAA+oAAAAGAAAADkN1cnJlbnQgc3RhdGUuAAAAAAAFc3RhdGUAAAAAAAfQAAAADVByb3Bvc2FsU3RhdGUAAAAAAAAyVGFyZ2V0IGNvbnRyYWN0IHRvIGludm9rZSBvbiBzdWNjZXNzZnVsIGV4ZWN1dGlvbi4AAAAAAAZ0YXJnZXQAAAAAABMAAAAoTGVkZ2VyIHRpbWVzdGFtcCBhdCB3aGljaCB2b3RpbmcgY2xvc2VzLgAAAAt2b3RpbmdfZW5kcwAAAAAG",
        "AAAAAgAAAClMaWZlY3ljbGUgc3RhdGUgb2YgYSBnb3Zlcm5hbmNlIHByb3Bvc2FsLgAAAAAAAAAAAAANUHJvcG9zYWxTdGF0ZQAAAAAAAAYAAAAAAAAAEE9wZW4gZm9yIHZvdGluZy4AAAAGQWN0aXZlAAAAAAAAAAAAIUFwcHJvdmVkIGFuZCByZWFkeSBmb3IgZXhlY3V0aW9uLgAAAAAAAAlTdWNjZWVkZWQAAAAAAAAAAAAAFFJlamVjdGVkIG9yIGV4cGlyZWQuAAAACERlZmVhdGVkAAAAAAAAACpTdWNjZXNzZnVsbHkgZXhlY3V0ZWQgb24tY2hhaW4gKHRlcm1pbmFsKS4AAAAAAAhFeGVjdXRlZAAAAAAAAAAxUXVldWVkIGZvciBkZWxheWVkIGV4ZWN1dGlvbiAob3B0aW9uYWwgdGltZWxvY2spLgAAAAAAAAZRdWV1ZWQAAAAAAAAAAABDV2l0aGRyYXduIGJ5IHRoZSBwcm9wb3NlciBiZWZvcmUgZXhlY3V0aW9uOyB0ZXJtaW5hbCBhbmQgaW1tdXRhYmxlLgAAAAAJQ2FuY2VsbGVkAAAA",
        "AAAAAQAAANNBIHJlYWQtb25seSBzdW1tYXJ5IG9mIGEgcHJvcG9zYWwncyByZWFkaW5lc3Mgc3RhdGUuCgpSZXR1cm5lZCBieSBbYFNvcm9iYW5Gb3JnZURhb0dvdmVybmFuY2U6OmdldF9wcm9wb3NhbF9zdGF0dXNgXS4KRGlzdGluZ3Vpc2hlcyBleGVjdXRhYmxlLCByZXRyeWFibGUsIGV4cGlyZWQsIHJlamVjdGVkLCBhbmQgdGVybWluYWwKc3RhdGVzIHdoZXJlIGFwcGxpY2FibGUuAAAAAAAAAAAOUHJvcG9zYWxTdGF0dXMAAAAAAAcAAAAzVGFsbHkgb2YgImFnYWluc3QiIHZvdGVzIGluIGdvdmVybmFuY2UtdG9rZW4gdW5pdHMuAAAAAA1hZ2FpbnN0X3ZvdGVzAAAAAAAACwAAAC9UYWxseSBvZiAiZm9yIiB2b3RlcyBpbiBnb3Zlcm5hbmNlLXRva2VuIHVuaXRzLgAAAAAJZm9yX3ZvdGVzAAAAAAAACwAAAG1XaGV0aGVyIHRoZSBwcm9wb3NhbCBjYW4gYmUgZXhlY3V0ZWQgbm93IChpLmUuIGBTdWNjZWVkZWRgIHdpdGgKZGVwZW5kZW5jaWVzIG1ldCBhbmQgdm90aW5nIGRlYWRsaW5lIHBhc3NlZCkuAAAAAAAADWlzX2V4ZWN1dGFibGUAAAAAAAABAAAAgVdoZXRoZXIgdGhlIHByb3Bvc2FsIGlzIGluIGEgcmV0cnlhYmxlIHN0YXRlIChlLmcuIGBTdWNjZWVkZWRgIHdob3NlCnRhcmdldCBpbnZvY2F0aW9uIHByZXZpb3VzbHkgZmFpbGVkOyBib25kIHN0aWxsIGluIGN1c3RvZHkpLgAAAAAAAAxpc19yZXRyeWFibGUAAAABAAAAG1VuaXF1ZSBwcm9wb3NhbCBpZGVudGlmaWVyLgAAAAALcHJvcG9zYWxfaWQAAAAABgAAAChDdXJyZW50IGxpZmVjeWNsZSBzdGF0ZSBvZiB0aGUgcHJvcG9zYWwuAAAABXN0YXRlAAAAAAAH0AAAAA1Qcm9wb3NhbFN0YXRlAAAAAAAASExlZGdlciB0aW1lc3RhbXAgYXQgd2hpY2ggdm90aW5nIGNsb3NlcyAoMCBpZiBwcm9wb3NhbCBpcyBub3QgYEFjdGl2ZWApLgAAAAt2b3RpbmdfZW5kcwAAAAAG",
        "AAAAAQAAAK9JbW11dGFibGUgcmVjZWlwdCBmb3IgYSBzaW5nbGUgcHJvcG9zYWwgdm90ZS4KCmBzdXBwb3J0YCBhbmQgYHdlaWdodGAgYXJlIHRoZSB2YWx1ZXMgcmVjb3JkZWQgYXQgdm90ZSB0aW1lLiBUaGV5IGFyZSBuZXZlcgpyZWNhbGN1bGF0ZWQgZnJvbSB0aGUgdm90ZXIncyBjdXJyZW50IHRva2VuIGJhbGFuY2UuAAAAAAAAAAALVm90ZVJlY2VpcHQAAAAAAwAAAC9XaGV0aGVyIHRoaXMgdm90ZXIgY2FzdCBhIHZvdGUgb24gdGhlIHByb3Bvc2FsLgAAAAAJaGFzX3ZvdGVkAAAAAAAAAQAAAINSZWNvcmRlZCBkaXJlY3Rpb246IGBTb21lKHRydWUpYCBmb3IsIGBTb21lKGZhbHNlKWAgYWdhaW5zdC4KYE5vbmVgIG1lYW5zIG5vIHZvdGUgd2FzIHJlY29yZGVkLiBBYnN0YWluIGlzIG5vdCBzdXBwb3J0ZWQgYnkgYHZvdGVgLgAAAAAHc3VwcG9ydAAAAAPoAAAAAQAAADlHb3Zlcm5hbmNlLXRva2VuIGJhbGFuY2UgcmVjb3JkZWQgd2hlbiB0aGUgdm90ZSB3YXMgY2FzdC4AAAAAAAAGd2VpZ2h0AAAAAAAL",
        "AAAAAAAAAu9XaXRoZHJhdyBhbiBhY3RpdmUgcHJvcG9zYWwgYmVmb3JlIGl0IGlzIGV4ZWN1dGVkLgoKUmVxdWlyZXMgdGhlIG9yaWdpbmFsIHByb3Bvc2VyLiBBIHByb3Bvc2FsIG1heSBiZSBjYW5jZWxsZWQgZXZlbiBhZnRlcgp2b3RpbmcgZW5kcyBhbmQgcXVvcnVtIGlzIG1ldCwgYXMgbG9uZyBhcyBpdCBoYXMgbm90IGJlZW4gZXhlY3V0ZWQgKG9yCmFscmVhZHkgY2FuY2VsbGVkKS4gQSBjYW5jZWxsZWQgcHJvcG9zYWwgaXMgdGVybWluYWw6IGZ1cnRoZXIgdm90ZXMgYW5kCmV4ZWN1dGlvbiBhcmUgcmVqZWN0ZWQuCgpUaGUgYm9uZCBpcyByZWZ1bmRlZCB0byB0aGUgcHJvcG9zZXIgaW4gdGhpcyBzYW1lIGNhbGw6IHRyYW5zZmVyCmZpcnN0LCB0aGVuIHRoZSBgQ2FuY2VsbGVkYCBzdGF0ZSB3cml0ZSDigJQgYSBmYWlsZWQgcmVmdW5kIHN1cmZhY2VzCmBUb2tlblRyYW5zZmVyRmFpbGVkYCB3aXRoIHRoZSBwcm9wb3NhbCBzdGlsbCBgQWN0aXZlYC4KCiogW2BGb3JnZUVycm9yOjpOb3RGb3VuZGBdIOKAlCBubyBwcm9wb3NhbCB3aXRoIHRoaXMgaWQuCiogW2BGb3JnZUVycm9yOjpVbmF1dGhvcml6ZWRgXSDigJQgYHByb3Bvc2VyYCBpcyBub3QgdGhlIG9yaWdpbmFsIHByb3Bvc2VyLgoqIFtgRm9yZ2VFcnJvcjo6SW52YWxpZElucHV0YF0g4oCUIHRoZSBwcm9wb3NhbCBpcyBubyBsb25nZXIgYEFjdGl2ZWAuCiogW2BGb3JnZUVycm9yOjpUb2tlblRyYW5zZmVyRmFpbGVkYF0g4oCUIHRoZSBib25kIHJlZnVuZCBmYWlsZWQuAAAAAA9jYW5jZWxfcHJvcG9zYWwAAAAAAgAAAAAAAAALcHJvcG9zYWxfaWQAAAAABgAAAAAAAAAIcHJvcG9zZXIAAAATAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAQRDb25maWd1cmUgdGhlIHByb3Bvc2FsIGJvbmQgZm9yIHRoZSBmaXJzdCBhbmQgb25seSB0aW1lLgoKUGVybWlzc2lvbmxlc3Mgb25lLXNob3QgKHNlZSBbYFNvcm9iYW5Gb3JnZURhb0dvdmVybmFuY2U6OmNvbmZpZ3VyZV9ib25kYF0pOgp0aGUgZGVwbG95ZXIgY2FsbHMgaXQgaW4gdGhlIGRlcGxveSB0cmFuc2FjdGlvbiBhbmQgdGhlIGNvbmZpZ3VyYXRpb24KbmV2ZXIgY2hhbmdlcyBhZnRlcndhcmRzLCBzbyBubyBwcml2aWxlZ2VkIHJvbGUgZXhpc3RzLgAAAA5jb25maWd1cmVfYm9uZAAAAAAAAwAAAAAAAAAFdG9rZW4AAAAAAAATAAAAAAAAAAZhbW91bnQAAAAAAAsAAAAAAAAACHRyZWFzdXJ5AAAAEwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAABABGaW5hbGlzZSBhIHByb3Bvc2FsIG9uY2Ugdm90aW5nIGhhcyBlbmRlZCwgYW5kIGV4ZWN1dGUgcGFzc2VkIHByb3Bvc2Fscy4KCkNhbGxhYmxlIGJ5IGFueW9uZSBhZnRlciB0aGUgZGVhZGxpbmUgKHBlcm1pc3Npb25sZXNzIGV4ZWN1dGlvbiksIGFuZArigJQgb24gdGhlIHBhdGhzIHRoYXQgcmVsZWFzZSBhIGJvbmQg4oCUIHBlcm1pc3Npb25sZXNzIGluIHRoZSB0b2tlbgpzZW5zZSB0b286IGNvbnRyYWN0IHNlbGYtYXV0aG9yaXphdGlvbiBjb3ZlcnMgdGhlIG91dGdvaW5nIHRyYW5zZmVyLgoKLSBBbiBgQWN0aXZlYCBwcm9wb3NhbCBwYXN0IGRlYWRsaW5lIHRyYW5zaXRpb25zIHRvIGBTdWNjZWVkZWRgIG9uIGEKc3RyaWN0IG1ham9yaXR5IG9mIGBmb3JgIHZvdGVzICh0aGUgYm9uZCBzdGF5cyBpbiBjdXN0b2R5IHVudGlsIGEKdGVybWluYWwgdHJhbnNpdGlvbiksIG9yIHRvIGBEZWZlYXRlZGAgb3RoZXJ3aXNlIOKAlCBmb3JmZWl0aW5nIHRoZQpib25kIHRvIHRoZSB0cmVhc3VyeSAqKmJlZm9yZSoqIHRoZSBzdGF0ZSB3cml0ZS4KLSBBIGBTdWNjZWVkZWRgIHByb3Bvc2FsIHBlcmZvcm1zIGEgcmVhbCBjcm9zcy1jb250cmFjdCBjYWxsIHRvIGB0YXJnZXRgCndpdGggYGFjdGlvbmAgKGB0YXJnZXQuZXhlY3V0ZShhY3Rpb24pYCkuIE9uIHN1Y2Nlc3NmdWwgaW52b2NhdGlvbiwKaXQgcmVmdW5kcyB0aGUgYm9uZCB0byB0aGUgcHJvcG9zZXIsIHRoZW4gdHJhbnNpdGlvbnMgdG8gdGhlCnRlcm1pbmFsIGBFeGVjdXRlZGAgc3RhdGUgYW5kIGVtaXRzIGFuIGBFeGVjdXRlZGAgZXZlbnQuCi0gSWYgdGhlIHRhcmdldCBpbnZvY2F0aW9uIHJldmVydHMsIGBGb3JnZUVycm9yOjpDb250cmFjdEludm9jYXRpb25GYWlsZWRgCmlzIHJldHVybmVkIGFuZCB0aGUgcHJvcG9zYWwgcmVtYWlucyBgU3VjY2VlZGVkYCAobm90IGBFeGVjdXRlZGApLApsZWF2aW5nIHRhcmdldCBzdGF0ZSB1bmNoYW5nZWQgYW5kIHRoZSBib25kIGluIGN1c3RvZHkuCi0gSWYgYSBib25kIHJlZnVuZC9mb3JmZWl0IHRyYW5zAAAAB2V4ZWN1dGUAAAAAAQAAAAAAAAALcHJvcG9zYWxfaWQAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAFNSZWFkIGhvdyBtYW55IGFjdGl2ZSBwcm9wb3NhbHMgYHByb3Bvc2VyYCBjdXJyZW50bHkgaGFzIGluIGZsaWdodCAocmVhZC1vbmx5IHZpZXcpLgAAAAAZZ2V0X2FjdGl2ZV9wcm9wb3NhbF9jb3VudAAAAAAAAAEAAAAAAAAACHByb3Bvc2VyAAAAEwAAAAEAAAAE",
        "AAAAAAAAAK5SZWFkIHRoZSBib25kIGNvbmZpZ3VyYXRpb24gKHJlYWQtb25seSB2aWV3KS4KClJldHVybnMgYEZvcmdlRXJyb3I6Ok5vdEluaXRpYWxpemVkYCB3aGlsZSB0aGUgY29udHJhY3QgaGFzIG5vIGJvbmQKY29uZmlndXJhdGlvbiDigJQgdGhlIHNhbWUgcHJlY29uZGl0aW9uIGBwcm9wb3NlYCBlbmZvcmNlcy4AAAAAAA9nZXRfYm9uZF9jb25maWcAAAAAAAAAAAEAAAPpAAAH0AAAAApCb25kQ29uZmlnAAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAADJSZXR1cm4gYSBwcm9wb3NhbCdzIGRlcGVuZGVuY3kgZWRnZXMgZm9yIGluZGV4ZXJzLgAAAAAAEGdldF9kZXBlbmRlbmNpZXMAAAABAAAAAAAAAAtwcm9wb3NhbF9pZAAAAAAGAAAAAQAAA+kAAAfQAAAADkRlcGVuZGVuY3lWaWV3AAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAC5SZWFkIGEgc3RvcmVkIHByb3Bvc2FsIGJ5IGlkIChyZWFkLW9ubHkgdmlldykuAAAAAAAMZ2V0X3Byb3Bvc2FsAAAAAQAAAAAAAAALcHJvcG9zYWxfaWQAAAAABgAAAAEAAAPpAAAH0AAAAAhQcm9wb3NhbAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAD5SZXR1cm4gdGhlIHRvdGFsIG51bWJlciBvZiBwcm9wb3NhbHMgY3JlYXRlZCAocmVhZC1vbmx5IHZpZXcpLgAAAAAAEmdldF9wcm9wb3NhbF9jb3VudAAAAAAAAAAAAAEAAAAG",
        "AAAAAAAAAXtSZWFkIGEgcmVhZC1vbmx5IHByb3Bvc2FsIHN0YXR1cyBzdW1tYXJ5IChyZWFkLW9ubHkgdmlldykuCgpQdXJlIHZpZXc6IG5ldmVyIHdyaXRlcyBzdG9yYWdlLCBleHRlbmRzIFRUTHMsIG9yIGVtaXRzIGV2ZW50cy4gQW4KdW5rbm93biBwcm9wb3NhbCB5aWVsZHMgW2BGb3JnZUVycm9yOjpOb3RGb3VuZGBdLiBBIGBTdWNjZWVkZWRgCnByb3Bvc2FsIGlzIGV4ZWN1dGFibGUgb25jZSB2b3RpbmcgaGFzIGNsb3NlZCBhbmQgYWxsIGRlcGVuZGVuY2llcwphcmUgc2F0aXNmaWVkOyBpdCBzdGF5cyByZXRyeWFibGUgd2hpbGUgaXRzIGJvbmQgaXMgc3RpbGwgaW4gY3VzdG9keQphbmQgbm8gZGVwZW5kZW5jeSBoYXMgcGVybWFuZW50bHkgYmxvY2tlZCBleGVjdXRpb24uAAAAABNnZXRfcHJvcG9zYWxfc3RhdHVzAAAAAAEAAAAAAAAAC3Byb3Bvc2FsX2lkAAAAAAYAAAABAAAD6QAAB9AAAAAOUHJvcG9zYWxTdGF0dXMAAAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAPZSZWFkIGEgcGFnaW5hdGVkIHNsaWNlIG9mIHByb3Bvc2FscyBvcmRlcmVkIGJ5IHByb3Bvc2FsIElEIChyZWFkLW9ubHkgdmlldykuCgpCb3VuZHMgY2xhbXBpbmc6Ci0gYGxpbWl0ID09IDBgIHJldHVybnMgYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGAuCi0gSWYgYG9mZnNldCA+PSB0b3RhbGAsIHJldHVybnMgYW4gZW1wdHkgYFZlY2AuCi0gUmV0dXJucyBhdCBtb3N0IGBsaW1pdGAgaXRlbXMgd2l0aG91dCBvdmVyZmxvd2luZy4AAAAAAA1nZXRfcHJvcG9zYWxzAAAAAAAAAgAAAAAAAAAGb2Zmc2V0AAAAAAAEAAAAAAAAAAVsaW1pdAAAAAAAAAQAAAABAAAD6QAAA+oAAAfQAAAACFByb3Bvc2FsAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAd5SZWFkIHRoZSBpbW11dGFibGUgdm90ZSByZWNlaXB0IHJlY29yZGVkIGZvciBgdm90ZXJgIG9uIGBwcm9wb3NhbF9pZGAuCgpUaGlzIGlzIGEgcmVhZC1vbmx5IHZpZXc6IGl0IG5ldmVyIHdyaXRlcyBzdG9yYWdlLCBleHRlbmRzIFRUTHMsIG9yCnJlY2FsY3VsYXRlcyB2b3RpbmcgcG93ZXIuIEEgdm90ZXIgdGhhdCBoYXMgbm90IHZvdGVkIHJlY2VpdmVzCmBoYXNfdm90ZWQ6IGZhbHNlYCwgYHN1cHBvcnQ6IE5vbmVgLCBhbmQgYHdlaWdodDogMGAuIEEgbWFsZm9ybWVkCmB2b3RlcmAgYWRkcmVzcyBpcyByZWplY3RlZCBieSB0aGUgU29yb2JhbiBob3N0IGJlZm9yZSB0aGlzIHF1ZXJ5IHJ1bnMKKHRoZSBzaGFyZWQgZXJyb3IgcmVnaXN0cnkgZG9jdW1lbnRzIGJhZCBhZGRyZXNzZXMgYXMgYEludmFsaWRJbnB1dGApLgoKIyBFcnJvcnMKCiogW2BGb3JnZUVycm9yOjpOb3RGb3VuZGBdIOKAlCBubyBwcm9wb3NhbCB3aXRoIHRoaXMgaWQuAAAAAAAQZ2V0X3ZvdGVfcmVjZWlwdAAAAAIAAAAAAAAAC3Byb3Bvc2FsX2lkAAAAAAYAAAAAAAAABXZvdGVyAAAAAAAAEwAAAAEAAAPpAAAH0AAAAAtWb3RlUmVjZWlwdAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAIVDaGVjayB3aGV0aGVyIGB2b3RlcmAgaGFzIHZvdGVkIG9uIGBwcm9wb3NhbF9pZGAgKHJlYWQtb25seSB2aWV3KS4KClJldHVybnMgW2BGb3JnZUVycm9yOjpOb3RGb3VuZGBdIGlmIGBwcm9wb3NhbF9pZGAgZG9lcyBub3QgZXhpc3QuAAAAAAAACWhhc192b3RlZAAAAAAAAAIAAAAAAAAAC3Byb3Bvc2FsX2lkAAAAAAYAAAAAAAAABXZvdGVyAAAAAAAAEwAAAAEAAAPpAAAAAQAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAACxDb25maWd1cmUgdGhlIGdvdmVybmFuY2UgdG9rZW4gZXhhY3RseSBvbmNlLgAAAAppbml0aWFsaXplAAAAAAABAAAAAAAAABBnb3Zlcm5hbmNlX3Rva2VuAAAAEwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAY1DcmVhdGUgYSBuZXcgcHJvcG9zYWwgYW5kIHJldHVybiBpdHMgc3RhYmxlIGlkLgoKUmVxdWlyZXMgYGR1cmF0aW9uID4gMGAgYW5kIGEgY29uZmlndXJlZCBib25kLiBUaGUgcHJvcG9zZXIgaXMKYXV0aG9yaXplZCBhdCBjcmVhdGlvbiB0aW1lLCBhbmQgdGhlaXIgYXV0aG9yaXphdGlvbiBjb3ZlcnMgdGhlCm5lc3RlZCBib25kIHB1bGwuCgpPcmRlcmluZyAobG9hZC1iZWFyaW5nKTogdGhlIGJvbmQgdHJhbnNmZXIgcnVucyAqKmJlZm9yZSoqIHRoZSBpZApjb3VudGVyLCB0aGUgcHJvcG9zYWwgcmVjb3JkLCBhbmQgdGhlIGN1c3RvZHkgdG90YWwgYXJlIHdyaXR0ZW4sIHNvIGEKZmFpbGVkIHRyYW5zZmVyIGxlYXZlcyBubyBwcm9wb3NhbCByZWNvcmQg4oCUIHNlZSB0aGUgbW9kdWxlIGRvY3MuAAAAAAAAB3Byb3Bvc2UAAAAABgAAAAAAAAAIcHJvcG9zZXIAAAATAAAAAAAAAAZ0YXJnZXQAAAAAABMAAAAAAAAABmFjdGlvbgAAAAAADgAAAAAAAAAIZHVyYXRpb24AAAAGAAAAAAAAAAhyZXF1aXJlcwAAA+oAAAAGAAAAAAAAAA5jb25mbGljdHNfd2l0aAAAAAAD6AAAAAYAAAABAAAD6QAAAAYAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAALpQZXJtaXNzaW9ubGVzcyBrZWVwZXI6IGJ1bXAgdGhlIHByb3Bvc2FsIGVudHJ5J3MgVFRMIHdpdGhvdXQgY2hhbmdpbmcKYW55IHN0YXRlLiBUaGUgZXhpc3RlbmNlIGNoZWNrIGlzIGRlbGliZXJhdGUg4oCUIHRvdWNoaW5nIGEgbWlzc2luZwppZCBtdXN0IGZhaWwgbG91ZGx5IHdpdGggYEZvcmdlRXJyb3I6Ok5vdEZvdW5kYC4AAAAAAAl0b3VjaF90dGwAAAAAAAABAAAAAAAAAAtwcm9wb3NhbF9pZAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAJRDYXN0IGEgdm90ZSBvbiBhbiBhY3RpdmUgcHJvcG9zYWwuCgpSZXF1aXJlcyB0aGUgdm90ZXIuIEVhY2ggdm90ZXIgbWF5IHZvdGUgZXhhY3RseSBvbmNlOyB2b3RpbmcgaXMgY2xvc2VkCm9uY2UgdGhlIGRlYWRsaW5lIChgdm90aW5nX2VuZHNgKSBwYXNzZXMuAAAABHZvdGUAAAADAAAAAAAAAAtwcm9wb3NhbF9pZAAAAAAGAAAAAAAAAAV2b3RlcgAAAAAAABMAAAAAAAAAB3N1cHBvcnQAAAAAAQAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAABQAAADRBIGJvbmQgZW50ZXJlZCBjdXN0b2R5IHdpdGggdGhlIHByb3Bvc2FsJ3MgY3JlYXRpb24uAAAAAAAAAApCb25kUG9zdGVkAAAAAAABAAAAC2JvbmRfcG9zdGVkAAAAAAMAAAAAAAAAC3Byb3Bvc2FsX2lkAAAAAAYAAAABAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAAAAAAAZhbW91bnQAAAAAAAsAAAAAAAAAAg==",
        "AAAABQAAAINBIGJvbmQgbGVmdCBjdXN0b2R5IG9uIGEgdGVybWluYWwgdHJhbnNpdGlvbjogYmFjayB0byB0aGUgcHJvcG9zZXIKKGBmb3JmZWl0ZWQgPT0gZmFsc2VgKSBvciB0byB0aGUgdHJlYXN1cnkgKGBmb3JmZWl0ZWQgPT0gdHJ1ZWApLgAAAAAAAAAADEJvbmRSZWxlYXNlZAAAAAEAAAANYm9uZF9yZWxlYXNlZAAAAAAAAAUAAAAAAAAAC3Byb3Bvc2FsX2lkAAAAAAYAAAABAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAAAAAAAZhbW91bnQAAAAAAAsAAAAAAAAAAAAAAAJ0bwAAAAAAEwAAAAAAAAAAAAAACWZvcmZlaXRlZAAAAAAAAAEAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAACUZpbmFsaXNlZAAAAAAAAAEAAAAJZmluYWxpc2VkAAAAAAAABAAAAAAAAAALcHJvcG9zYWxfaWQAAAAABgAAAAEAAAAAAAAABXN0YXRlAAAAAAAH0AAAAA1Qcm9wb3NhbFN0YXRlAAAAAAAAAAAAAAAAAAAJZm9yX3ZvdGVzAAAAAAAACwAAAAAAAAAAAAAADWFnYWluc3Rfdm90ZXMAAAAAAAALAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAACFByb3Bvc2VkAAAAAQAAAAhwcm9wb3NlZAAAAAIAAAAAAAAAC3Byb3Bvc2FsX2lkAAAAAAYAAAABAAAAAAAAAARkYXRhAAAH0AAAAAhQcm9wb3NhbAAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAACFZvdGVDYXN0AAAAAQAAAAl2b3RlX2Nhc3QAAAAAAAAEAAAAAAAAAAtwcm9wb3NhbF9pZAAAAAAGAAAAAQAAAAAAAAAFdm90ZXIAAAAAAAATAAAAAAAAAAAAAAAHc3VwcG9ydAAAAAABAAAAAAAAAAAAAAAGd2VpZ2h0AAAAAAALAAAAAAAAAAI=",
        "AAAABAAAAZxTaGFyZWQgZXJyb3IgdHlwZSB1c2VkIGFjcm9zcyBhbGwgU29yb2JhbiBGb3JnZSBjb250cmFjdHMuCgpEZWZpbmluZyBhIHNpbmdsZSBlcnJvciBlbnVtIGluIGBzaGFyZWQtdXRpbHNgIGtlZXBzIHRoZSBvbi1jaGFpbiBlcnJvcgpzcGFjZSBjb25zaXN0ZW50IGFuZCBpbnRlbGxpZ2libGUgdG8gU0RLIGNvbnN1bWVycywgYW5kIGF2b2lkcyBldmVyeQpjb250cmFjdCByZS1kZWNsYXJpbmcgdGhlIHNhbWUgZmFpbHVyZSBtb2Rlcy4gQ29udHJhY3QgY3JhdGVzIG1heSBleHBvc2UKdGhlaXIgb3duIGRvbWFpbi1zcGVjaWZpYyBlcnJvcnMsIGJ1dCBzaG91bGQgcHJlZmVyIHRoZXNlIHdoZXJlIHRoZXkgZml0LgoKRXJyb3IgY29kZXMgc3RhcnQgYXQgMTsgY29kZSAwIGlzIHJlc2VydmVkIGJ5IHRoZSBTb3JvYmFuIGhvc3QuAAAAAAAAAApGb3JnZUVycm9yAAAAAAARAAAAM1RoZSBjYWxsZXIgaXMgbm90IHBlcm1pdHRlZCB0byBwZXJmb3JtIHRoaXMgYWN0aW9uLgAAAAAMVW5hdXRob3JpemVkAAAAAQAAAEpUaGUgcmVxdWVzdGVkIGVudGl0eSAoZXNjcm93LCBwcm9wb3NhbCwgc3Vic2NyaXB0aW9uLCAuLi4pIGRvZXMgbm90IGV4aXN0LgAAAAAACE5vdEZvdW5kAAAAAgAAAEhPbmUgb3IgbW9yZSBhcmd1bWVudHMgZmFpbGVkIHZhbGlkYXRpb24gKGUuZy4gemVybyBhbW91bnQsIGJhZCBhZGRyZXNzKS4AAAAMSW52YWxpZElucHV0AAAAAwAAAENUaGUgY29udHJhY3QgZG9lcyBub3QgaG9sZCBlbm91Z2ggYmFsYW5jZSB0byBzYXRpc2Z5IHRoZSBvcGVyYXRpb24uAAAAABFJbnN1ZmZpY2llbnRGdW5kcwAAAAAAAAQAAABCVGhlIGVudGl0eSB3YXMgYWxyZWFkeSBpbml0aWFsaXNlZDsgcmUtaW5pdGlhbGlzYXRpb24gaXMgcmVqZWN0ZWQuAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAAFAAAANlRoZSBlbnRpdHkgd2FzIGV4cGVjdGVkIHRvIGJlIGluaXRpYWxpc2VkIGJ1dCB3YXMgbm90LgAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAGAAAANkFuIG9wZXJhdGlvbiB3YXMgYXR0ZW1wdGVkIGFmdGVyIGl0cyBkZWFkbGluZSBlbGFwc2VkLgAAAAAAD0RlYWRsaW5lUmVhY2hlZAAAAAAHAAAAQUEgcmVxdWlyZWQgdG9rZW4gYWxsb3dhbmNlIHdhcyBsb3dlciB0aGFuIHRoZSBhbW91bnQgYmVpbmcgc3BlbnQuAAAAAAAAFUluc3VmZmljaWVudEFsbG93YW5jZQAAAAAAAAgAAAAjQW4gYXJpdGhtZXRpYyBvcGVyYXRpb24gb3ZlcmZsb3dlZC4AAAAAEkFyaXRobWV0aWNPdmVyZmxvdwAAAAAACQAAAEJDb250cmFjdC1zcGVjaWZpYyBlcnJvciB0aGF0IGRvZXMgbm90IG1hcCB0byB0aGUgY2F0ZWdvcmllcyBhYm92ZS4AAAAAAAZDdXN0b20AAAAAAAoAAAFsQSBTRVAtNDEgdG9rZW4gaW52b2NhdGlvbiBmYWlsZWQgKGluc3VmZmljaWVudCBiYWxhbmNlLCBtaXNzaW5nIG9yCmRlYXV0aG9yaXplZCB0cnVzdGxpbmUsIHVuZGVwbG95ZWQgdG9rZW4gY29udHJhY3QsIG9yIHRva2VuLWxvZ2ljCnJlamVjdGlvbikuIFRoZSByYXcgdG9rZW4gZXJyb3IgZGlzY3JpbWluYW50IGlzIGludGVudGlvbmFsbHkgbm90CmZvcndhcmRlZCDigJQgY2FsbGVycyBjYW5ub3QgdGVsbCB3aGljaCBjb250cmFjdCBwcm9kdWNlZCBhIGZvcndhcmRlZApjb2RlLCBzbyBpdCBpcyBidWNrZXRlZDsgdGhlIHJvb3QgY2F1c2UgcmVtYWlucyB2aXNpYmxlIGluIHRoZQp0cmFuc2FjdGlvbidzIGRpYWdub3N0aWMgZXZlbnRzLgAAABNUb2tlblRyYW5zZmVyRmFpbGVkAAAAAAsAAADbQSBjcm9zcy1jb250cmFjdCBpbnZvY2F0aW9uIGZhaWxlZCAodGFyZ2V0IHJldmVydGVkIG9yIGhvc3QKYWJvcnQpLiBUaGUgdGFyZ2V0IHJldmVydHMgYXJlIHN1cmZhY2VkIGhlcmUgc28gdGhlIGNhbGxlciBjYW4KZGlzdGluZ3Vpc2ggdGhlbSBmcm9tIHRva2VuLXRyYW5zZmVyIGZhaWx1cmVzLCBhbmQgdGhlIGludm9raW5nCnRyYW5zYWN0aW9uIGlzIGxlZnQgdW4tZXhlY3V0ZWQuAAAAABhDb250cmFjdEludm9jYXRpb25GYWlsZWQAAAAMAAABDFRoZSByZXF1ZXN0ZWQgd2l0aGRyYXdhbCB3b3VsZCBwdXNoIGEgdG9rZW4ncyByb2xsaW5nLXdpbmRvdyB0b3RhbApwYXN0IGl0cyBjb25maWd1cmVkIHdpdGhkcmF3YWwgbGltaXQuIEtlcHQgZGlzdGluY3QgZnJvbQpbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIHNvIGEgY2FsbGVyIGNhbiB0ZWxsIGEgcG9saWN5IHJlamVjdGlvbgooYSB2YWxpZCB3aXRoZHJhd2FsIHRoYXQgaXMgdG9vIGxhcmdlIHJpZ2h0IG5vdykgZnJvbSBhIG1hbGZvcm1lZAphcmd1bWVudC4AAAAXV2l0aGRyYXdhbExpbWl0RXhjZWVkZWQAAAAADQAAARtBIHN1YnNjcmlwdGlvbiBpcyBpbiB0aGUgYFBhc3REdWVgIGxhcHNlZCBzdGF0ZSBhbmQgdGhlIHJlcXVlc3RlZApvcGVyYXRpb24gY2Fubm90IGJlIHBlcmZvcm1lZCB1bnRpbCBhIGNhdGNoLXVwIGNoYXJnZSByZXN0b3JlcyBpdCB0bwpgQWN0aXZlYC4gS2VwdCBkaXN0aW5jdCBmcm9tIFtgRm9yZ2VFcnJvcjo6RGVhZGxpbmVSZWFjaGVkYF0gc28gYSBjYWxsZXIKY2FuIHRlbGwgYSBsYXBzZWQgYmlsbGluZyBzdGF0ZSBmcm9tIGFuIGFyZ3VtZW50IHRoYXQgd2FzIG1lcmVseQp0b28gZWFybHkuAAAAABNTdWJzY3JpcHRpb25QYXN0RHVlAAAAAA4AAABxQSBwcm9wb3NlciBoYXMgcmVhY2hlZCB0aGUgbWF4aW11bSBhbGxvd2VkIGNvbmN1cnJlbnQgYWN0aXZlIHByb3Bvc2FscwpvciBpcyB3aXRoaW4gdGhlIHByb3Bvc2VyIGNvb2xkb3duIHdpbmRvdy4AAAAAAAAQUHJvcG9zZXJDb29sZG93bgAAAA8AAAEvQSBzZWxsZXIncyByZWxlYXNlIHJlcXVlc3QgaXMgd2FpdGluZyBvdXQgaXRzIHJldmlldyB3aW5kb3csIHNvIHRoZQpidXllciBjYW5ub3QgdGFrZSB0aGUgZnVuZHMgYmFjayBieSByZWZ1bmRpbmcuIFRoZSBidXllciBjYW4gYXBwcm92ZQp0aGUgcmVsZWFzZSBvciByYWlzZSBhIGRpc3B1dGUgaW5zdGVhZC4gS2VwdCBkaXN0aW5jdCBmcm9tCltgRm9yZ2VFcnJvcjo6SW52YWxpZElucHV0YF0gc28gYSB3YWxsZXQgY2FuIGV4cGxhaW4gd2h5IHRoZSByZWZ1bmQKd2FzIHJlZnVzZWQgYW5kIG9mZmVyIHRoZSBkaXNwdXRlIHBhdGguAAAAAA5SZWxlYXNlUGVuZGluZwAAAAAAEAAAADlUaGUgY3VycmVudCBsaWZlY3ljbGUgc3RhdGUgZG9lcyBub3QgYWxsb3cgdGhlIG9wZXJhdGlvbi4AAAAAAAAMSW52YWxpZFN0YXRlAAAAEQ==" ]),
      options
    )
  }
  public readonly fromJSON = {
    vote: this.txFromJSON<Result<void>>,
        execute: this.txFromJSON<Result<void>>,
        propose: this.txFromJSON<Result<u64>>,
        has_voted: this.txFromJSON<Result<boolean>>,
        touch_ttl: this.txFromJSON<Result<void>>,
        initialize: this.txFromJSON<Result<void>>,
        get_proposal: this.txFromJSON<Result<Proposal>>,
        get_proposals: this.txFromJSON<Result<Array<Proposal>>>,
        configure_bond: this.txFromJSON<Result<void>>,
        cancel_proposal: this.txFromJSON<Result<void>>,
        get_bond_config: this.txFromJSON<Result<BondConfig>>,
        get_dependencies: this.txFromJSON<Result<DependencyView>>,
        get_vote_receipt: this.txFromJSON<Result<VoteReceipt>>,
        get_proposal_count: this.txFromJSON<u64>,
        get_proposal_status: this.txFromJSON<Result<ProposalStatus>>,
        get_active_proposal_count: this.txFromJSON<u32>
  }
}