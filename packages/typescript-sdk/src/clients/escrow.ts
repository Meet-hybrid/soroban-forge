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
 * A single three-party escrow record.
 *
 * ## Accounting fields
 *
 * * `amount` — total tokens deposited by the buyer; never changes after
 * `deposit`.
 * * `released` — cumulative tokens already transferred to the seller via
 * `release_partial`; `0` before any partial release is made.
 * * `remaining` (derived) — `amount - released`; the balance currently
 * held in custody. `refund` and `resolve` pay this amount; `release`
 * and the final `release_partial` must consume it entirely.
 *
 * ## Storage compatibility
 *
 * Records written by contract versions prior to the partial-release feature
 * do not contain a `released` field. They are loaded via a backward-compat
 * decode path that defaults `released` to `0` (see `load_escrow` internals
 * and the module-level storage compatibility note).
 */
export interface EscrowData {
  /**
 * Total amount of `token` deposited by the buyer. Never changes after
 * `deposit`.
 */
amount: i128;
  /**
 * Neutral party deciding disputes. Recorded at creation; authorizes
 * only `resolve`.
 */
arbiter: string;
  /**
 * Party funding the escrow and the default refund recipient.
 */
buyer: string;
  /**
 * Unix timestamp of creation.
 */
created_at: u64;
  /**
 * Stable id, never reused.
 */
escrow_id: u64;
  /**
 * Cumulative amount already transferred to the seller via
 * `release_partial`. `0` until the first partial release.
 */
released: i128;
  /**
 * Party paid on release.
 */
seller: string;
  /**
 * Current lifecycle state.
 */
status: EscrowStatus;
  /**
 * Seconds after `created_at` at which the buyer may self-refund.
 */
timeout: u64;
  /**
 * SEP-41 token contract custodied by this escrow.
 */
token: string;
}


/**
 * One leg of a basket escrow: a single SEP-41 token and its accounting.
 *
 * The field names mirror [`EscrowData`]'s single-token accounting exactly so
 * the two record kinds express one rule rather than two: `remaining` is
 * `amount - released`, `refund`/`resolve` pay `remaining`, and a full release
 * consumes it.
 */
export interface EscrowAsset {
  /**
 * Total amount of `token` this leg escrows. Fixed at creation and
 * unchanged by `deposit_basket`.
 */
amount: i128;
  /**
 * Cumulative amount of `token` already paid to the seller via
 * `release_partial_basket`. `0` until the first partial release.
 */
released: i128;
  /**
 * SEP-41 token contract for this leg.
 */
token: string;
}

/**
 * Lifecycle state of an escrow.
 */
export type EscrowStatus = {tag: "Pending", values: void} | {tag: "Funded", values: void} | {tag: "Completed", values: void} | {tag: "Refunded", values: void} | {tag: "Disputed", values: void} | {tag: "Cancelled", values: void};


/**
 * A basket escrow record: the same three-party lifecycle as [`EscrowData`],
 * with an ordered list of legs instead of one token.
 *
 * Stored at [`DataKey::Basket`], a separate key class from
 * [`DataKey::Escrow`], so the single-token record's stored shape and the
 * events that carry it are untouched. Ids come from the same
 * [`DataKey::Count`] sequence, so an id belongs to exactly one escrow of
 * exactly one kind.
 */
export interface BasketEscrowData {
  /**
 * Neutral party deciding disputes. Recorded at creation; authorizes only
 * `resolve_basket`.
 */
arbiter: string;
  /**
 * The basket: one leg per token, in the order the buyer declared. No two
 * legs name the same token.
 */
assets: Array<EscrowAsset>;
  /**
 * Party funding the basket and the default refund recipient.
 */
buyer: string;
  /**
 * Unix timestamp of creation.
 */
created_at: u64;
  /**
 * Stable id, never reused. Shares the sequence with single-token escrows.
 */
escrow_id: u64;
  /**
 * Party paid on release.
 */
seller: string;
  /**
 * Current lifecycle state. Identical semantics to [`EscrowStatus`].
 */
status: EscrowStatus;
  /**
 * Seconds after `created_at` at which the buyer may self-refund.
 */
timeout: u64;
}


/**
 * A page of escrow ids involving a participant.
 *
 * Returned by [`SorobanForgeEscrow::escrows_for_participant`]; powered by
 * the per-party persistent index written at creation.
 */
export interface ParticipantEscrowsPage {
  /**
 * Escrow ids on this page, in creation order.
 */
ids: Array<u64>;
  /**
 * Offset for the next page, or `None` when `ids` ends the participant's
 * list. Replay it until `None` to iterate the whole list.
 */
next_cursor: Option<u32>;
  /**
 * Total escrows involving the participant across all pages.
 */
total: u32;
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
  15: {message:"ProposerCooldown"}
}

export interface Client {
  /**
   * Construct and simulate a cancel transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cancel a `Pending` escrow. Requires the buyer. Nothing has moved,
   * so no token transfer occurs.
   */
  cancel: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a refund transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Refund the buyer.
   *
   * Pre-deadline: seller authorizes (voluntary refund). Post-deadline:
   * buyer authorizes (reclaim of unfulfilled funds). Refunds only the
   * **remaining** balance (`amount - released`).
   */
  refund: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a deposit transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Fund the escrow, pulling tokens from the buyer into this contract.
   *
   * Ordering: transfer **first**, state write **second** — see the
   * module docs for why the inverse would be a fund-safety bug.
   */
  deposit: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a dispute transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Raise a dispute: the claimant (buyer or seller) authorizes, while
   * `Funded`. Freezes the remaining balance until the arbiter resolves.
   */
  dispute: ({escrow_id, claimant}: {escrow_id: u64, claimant: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a release transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Release the full remaining balance to the seller. Seller-authorized:
   * delivery confirmation by the paid party, not the paying one.
   *
   * Backward-compatible: behaves identically to the pre-partial-release
   * `release` when `released == 0`. When partial releases have already
   * been made, only the **remaining** balance (`amount - released`) is
   * transferred — maintaining the conservation invariant.
   */
  release: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a resolve transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Resolve a dispute: arbiter only, final. Pays the **remaining**
   * balance to the seller (`true`) or refunds it to the buyer (`false`).
   */
  resolve: ({escrow_id, in_favor_of_seller}: {escrow_id: u64, in_favor_of_seller: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a ttl_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the remaining ledger count before the escrow record expires.
   *
   * `NotFound` means the id never existed or its persistent entry is
   * already archived. This view performs no TTL extension.
   */
  ttl_info: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u32>>>

  /**
   * Construct and simulate a touch_ttl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Permissionless keeper: bump the escrow entry's TTL without changing
   * any state. The existence check is deliberate — touching a missing
   * id must fail loudly so a keeper can distinguish "extended" from
   * "no such escrow".
   */
  touch_ttl: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read a basket record.
   */
  get_basket: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<BasketEscrowData>>>

  /**
   * Construct and simulate a get_escrow transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the full escrow record. The returned `EscrowData` exposes
   * `released` (cumulative seller payments) and `remaining()` (balance
   * in custody). For records created before the partial-release feature
   * was deployed, `released` will be `0`.
   */
  get_escrow: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<EscrowData>>>

  /**
   * Construct and simulate a get_status transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the current lifecycle status of either record kind.
   */
  get_status: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<EscrowStatus>>>

  /**
   * Construct and simulate a cancel_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cancel a `Pending` basket. Buyer-authorized; nothing has moved, so no
   * transfer occurs.
   */
  cancel_basket: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a create_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a basket escrow. Buyer-authorized only, exactly as
   * [`Self::create_escrow`], and drawing from the same id sequence.
   */
  create_basket: ({buyer, seller, arbiter, assets, timeout}: {buyer: string, seller: string, arbiter: string, assets: Array<EscrowAsset>, timeout: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a create_escrow transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new escrow and return its stable id.
   *
   * Only the buyer authorizes at creation. The arbiter does not
   * authorize either: they must be able to `resolve` later even if
   * they never participated in creation.
   */
  create_escrow: ({buyer, seller, arbiter, token, amount, timeout}: {buyer: string, seller: string, arbiter: string, token: string, amount: i128, timeout: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a refund_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pay every leg's remaining balance back to the buyer. Seller-authorized
   * before the deadline, buyer-authorized after, as for a single-token
   * escrow.
   */
  refund_basket: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a resolve_split transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Resolve a dispute by splitting the escrowed amount between the seller
   * and buyer according to `seller_bps` out of 10_000.
   *
   * The seller share is floored at `amount * seller_bps / 10_000` and the
   * remainder stays with the buyer. `seller_bps == 0` resolves to a
   * buyer-only refund and `seller_bps == 10_000` resolves to a full seller
   * payout. A failed second payout rolls back the whole invocation.
   */
  resolve_split: ({escrow_id, seller_bps}: {escrow_id: u64, seller_bps: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a deposit_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Fund a basket: pull every leg from the buyer. All-or-nothing.
   */
  deposit_basket: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a dispute_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Raise a dispute on a basket. The claimant must be a party and must
   * authorize the call; freezes every leg until the arbiter resolves.
   */
  dispute_basket: ({escrow_id, claimant}: {escrow_id: u64, claimant: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a refund_expired transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Permissionlessly refund the buyer after the escrow deadline.
   *
   * The strict-after boundary leaves the exact deadline to the existing
   * party-authorized refund path. Disputed escrows remain frozen, and the
   * token transfer precedes the state update so a failed payout is atomic.
   */
  refund_expired: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a release_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pay every leg's remaining balance to the seller. Seller-authorized.
   */
  release_basket: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a resolve_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Resolve a basket dispute: arbiter only, final, one direction for every
   * leg. Payout goes through the same loop as `release_basket`, so a
   * resolve can never settle legs differently than a release.
   */
  resolve_basket: ({escrow_id, in_favor_of_seller}: {escrow_id: u64, in_favor_of_seller: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a release_partial transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Release a partial `amount` to the seller. Seller-authorized.
   *
   * Only valid while `Funded`. `amount` must be positive and must not
   * exceed the remaining balance. When `amount == remaining`, the escrow
   * transitions to `Completed` in the same call.
   *
   * Transfer-before-state ordering is preserved: the token payout happens
   * before any storage write, so a failed transfer leaves no partial state.
   */
  release_partial: ({escrow_id, amount}: {escrow_id: u64, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a release_partial_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pay `amount` of one leg to the seller. Seller-authorized.
   *
   * All validation before any state change or transfer, mirroring
   * `release_partial`.
   */
  release_partial_basket: ({escrow_id, token, amount}: {escrow_id: u64, token: string, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a escrows_for_participant transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the current creation-order escrow ids for a participant one page
   * at a time. Cancellation removes ids from the compacted index. A cursor
   * is an offset into the current list; if an earlier id is removed between
   * pages, later ids may shift before the cursor. Restart at zero after
   * mutations to enumerate the current list completely. This view itself
   * never mutates storage and never errors — an unknown address or one with
   * no escrows simply yields an empty page.
   */
  escrows_for_participant: ({participant, cursor, limit}: {participant: string, cursor: u32, limit: u32}, options?: MethodOptions) => Promise<AssembledTransaction<ParticipantEscrowsPage>>

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
      new ContractSpec([ "AAAAAAAAAF5DYW5jZWwgYSBgUGVuZGluZ2AgZXNjcm93LiBSZXF1aXJlcyB0aGUgYnV5ZXIuIE5vdGhpbmcgaGFzIG1vdmVkLApzbyBubyB0b2tlbiB0cmFuc2ZlciBvY2N1cnMuAAAAAAAGY2FuY2VsAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAMRSZWZ1bmQgdGhlIGJ1eWVyLgoKUHJlLWRlYWRsaW5lOiBzZWxsZXIgYXV0aG9yaXplcyAodm9sdW50YXJ5IHJlZnVuZCkuIFBvc3QtZGVhZGxpbmU6CmJ1eWVyIGF1dGhvcml6ZXMgKHJlY2xhaW0gb2YgdW5mdWxmaWxsZWQgZnVuZHMpLiBSZWZ1bmRzIG9ubHkgdGhlCioqcmVtYWluaW5nKiogYmFsYW5jZSAoYGFtb3VudCAtIHJlbGVhc2VkYCkuAAAABnJlZnVuZAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAMBGdW5kIHRoZSBlc2Nyb3csIHB1bGxpbmcgdG9rZW5zIGZyb20gdGhlIGJ1eWVyIGludG8gdGhpcyBjb250cmFjdC4KCk9yZGVyaW5nOiB0cmFuc2ZlciAqKmZpcnN0KiosIHN0YXRlIHdyaXRlICoqc2Vjb25kKiog4oCUIHNlZSB0aGUKbW9kdWxlIGRvY3MgZm9yIHdoeSB0aGUgaW52ZXJzZSB3b3VsZCBiZSBhIGZ1bmQtc2FmZXR5IGJ1Zy4AAAAHZGVwb3NpdAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAIVSYWlzZSBhIGRpc3B1dGU6IHRoZSBjbGFpbWFudCAoYnV5ZXIgb3Igc2VsbGVyKSBhdXRob3JpemVzLCB3aGlsZQpgRnVuZGVkYC4gRnJlZXplcyB0aGUgcmVtYWluaW5nIGJhbGFuY2UgdW50aWwgdGhlIGFyYml0ZXIgcmVzb2x2ZXMuAAAAAAAAB2Rpc3B1dGUAAAAAAgAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAAAAAAIY2xhaW1hbnQAAAATAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAYRSZWxlYXNlIHRoZSBmdWxsIHJlbWFpbmluZyBiYWxhbmNlIHRvIHRoZSBzZWxsZXIuIFNlbGxlci1hdXRob3JpemVkOgpkZWxpdmVyeSBjb25maXJtYXRpb24gYnkgdGhlIHBhaWQgcGFydHksIG5vdCB0aGUgcGF5aW5nIG9uZS4KCkJhY2t3YXJkLWNvbXBhdGlibGU6IGJlaGF2ZXMgaWRlbnRpY2FsbHkgdG8gdGhlIHByZS1wYXJ0aWFsLXJlbGVhc2UKYHJlbGVhc2VgIHdoZW4gYHJlbGVhc2VkID09IDBgLiBXaGVuIHBhcnRpYWwgcmVsZWFzZXMgaGF2ZSBhbHJlYWR5CmJlZW4gbWFkZSwgb25seSB0aGUgKipyZW1haW5pbmcqKiBiYWxhbmNlIChgYW1vdW50IC0gcmVsZWFzZWRgKSBpcwp0cmFuc2ZlcnJlZCDigJQgbWFpbnRhaW5pbmcgdGhlIGNvbnNlcnZhdGlvbiBpbnZhcmlhbnQuAAAAB3JlbGVhc2UAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAINSZXNvbHZlIGEgZGlzcHV0ZTogYXJiaXRlciBvbmx5LCBmaW5hbC4gUGF5cyB0aGUgKipyZW1haW5pbmcqKgpiYWxhbmNlIHRvIHRoZSBzZWxsZXIgKGB0cnVlYCkgb3IgcmVmdW5kcyBpdCB0byB0aGUgYnV5ZXIgKGBmYWxzZWApLgAAAAAHcmVzb2x2ZQAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAAAAABJpbl9mYXZvcl9vZl9zZWxsZXIAAAAAAAEAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAALxSZXR1cm4gdGhlIHJlbWFpbmluZyBsZWRnZXIgY291bnQgYmVmb3JlIHRoZSBlc2Nyb3cgcmVjb3JkIGV4cGlyZXMuCgpgTm90Rm91bmRgIG1lYW5zIHRoZSBpZCBuZXZlciBleGlzdGVkIG9yIGl0cyBwZXJzaXN0ZW50IGVudHJ5IGlzCmFscmVhZHkgYXJjaGl2ZWQuIFRoaXMgdmlldyBwZXJmb3JtcyBubyBUVEwgZXh0ZW5zaW9uLgAAAAh0dGxfaW5mbwAAAAEAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAD6QAAAAQAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAANlQZXJtaXNzaW9ubGVzcyBrZWVwZXI6IGJ1bXAgdGhlIGVzY3JvdyBlbnRyeSdzIFRUTCB3aXRob3V0IGNoYW5naW5nCmFueSBzdGF0ZS4gVGhlIGV4aXN0ZW5jZSBjaGVjayBpcyBkZWxpYmVyYXRlIOKAlCB0b3VjaGluZyBhIG1pc3NpbmcKaWQgbXVzdCBmYWlsIGxvdWRseSBzbyBhIGtlZXBlciBjYW4gZGlzdGluZ3Vpc2ggImV4dGVuZGVkIiBmcm9tCiJubyBzdWNoIGVzY3JvdyIuAAAAAAAACXRvdWNoX3R0bAAAAAAAAAEAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAABVSZWFkIGEgYmFza2V0IHJlY29yZC4AAAAAAAAKZ2V0X2Jhc2tldAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAH0AAAABBCYXNrZXRFc2Nyb3dEYXRhAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAOtSZWFkIHRoZSBmdWxsIGVzY3JvdyByZWNvcmQuIFRoZSByZXR1cm5lZCBgRXNjcm93RGF0YWAgZXhwb3NlcwpgcmVsZWFzZWRgIChjdW11bGF0aXZlIHNlbGxlciBwYXltZW50cykgYW5kIGByZW1haW5pbmcoKWAgKGJhbGFuY2UKaW4gY3VzdG9keSkuIEZvciByZWNvcmRzIGNyZWF0ZWQgYmVmb3JlIHRoZSBwYXJ0aWFsLXJlbGVhc2UgZmVhdHVyZQp3YXMgZGVwbG95ZWQsIGByZWxlYXNlZGAgd2lsbCBiZSBgMGAuAAAAAApnZXRfZXNjcm93AAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAfQAAAACkVzY3Jvd0RhdGEAAAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAADhSZWFkIHRoZSBjdXJyZW50IGxpZmVjeWNsZSBzdGF0dXMgb2YgZWl0aGVyIHJlY29yZCBraW5kLgAAAApnZXRfc3RhdHVzAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAfQAAAADEVzY3Jvd1N0YXR1cwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAQAAAv5BIHNpbmdsZSB0aHJlZS1wYXJ0eSBlc2Nyb3cgcmVjb3JkLgoKIyMgQWNjb3VudGluZyBmaWVsZHMKCiogYGFtb3VudGAg4oCUIHRvdGFsIHRva2VucyBkZXBvc2l0ZWQgYnkgdGhlIGJ1eWVyOyBuZXZlciBjaGFuZ2VzIGFmdGVyCmBkZXBvc2l0YC4KKiBgcmVsZWFzZWRgIOKAlCBjdW11bGF0aXZlIHRva2VucyBhbHJlYWR5IHRyYW5zZmVycmVkIHRvIHRoZSBzZWxsZXIgdmlhCmByZWxlYXNlX3BhcnRpYWxgOyBgMGAgYmVmb3JlIGFueSBwYXJ0aWFsIHJlbGVhc2UgaXMgbWFkZS4KKiBgcmVtYWluaW5nYCAoZGVyaXZlZCkg4oCUIGBhbW91bnQgLSByZWxlYXNlZGA7IHRoZSBiYWxhbmNlIGN1cnJlbnRseQpoZWxkIGluIGN1c3RvZHkuIGByZWZ1bmRgIGFuZCBgcmVzb2x2ZWAgcGF5IHRoaXMgYW1vdW50OyBgcmVsZWFzZWAKYW5kIHRoZSBmaW5hbCBgcmVsZWFzZV9wYXJ0aWFsYCBtdXN0IGNvbnN1bWUgaXQgZW50aXJlbHkuCgojIyBTdG9yYWdlIGNvbXBhdGliaWxpdHkKClJlY29yZHMgd3JpdHRlbiBieSBjb250cmFjdCB2ZXJzaW9ucyBwcmlvciB0byB0aGUgcGFydGlhbC1yZWxlYXNlIGZlYXR1cmUKZG8gbm90IGNvbnRhaW4gYSBgcmVsZWFzZWRgIGZpZWxkLiBUaGV5IGFyZSBsb2FkZWQgdmlhIGEgYmFja3dhcmQtY29tcGF0CmRlY29kZSBwYXRoIHRoYXQgZGVmYXVsdHMgYHJlbGVhc2VkYCB0byBgMGAgKHNlZSBgbG9hZF9lc2Nyb3dgIGludGVybmFscwphbmQgdGhlIG1vZHVsZS1sZXZlbCBzdG9yYWdlIGNvbXBhdGliaWxpdHkgbm90ZSkuAAAAAAAAAAAACkVzY3Jvd0RhdGEAAAAAAAoAAABOVG90YWwgYW1vdW50IG9mIGB0b2tlbmAgZGVwb3NpdGVkIGJ5IHRoZSBidXllci4gTmV2ZXIgY2hhbmdlcyBhZnRlcgpgZGVwb3NpdGAuAAAAAAAGYW1vdW50AAAAAAALAAAAUU5ldXRyYWwgcGFydHkgZGVjaWRpbmcgZGlzcHV0ZXMuIFJlY29yZGVkIGF0IGNyZWF0aW9uOyBhdXRob3JpemVzCm9ubHkgYHJlc29sdmVgLgAAAAAAAAdhcmJpdGVyAAAAABMAAAA6UGFydHkgZnVuZGluZyB0aGUgZXNjcm93IGFuZCB0aGUgZGVmYXVsdCByZWZ1bmQgcmVjaXBpZW50LgAAAAAABWJ1eWVyAAAAAAAAEwAAABtVbml4IHRpbWVzdGFtcCBvZiBjcmVhdGlvbi4AAAAACmNyZWF0ZWRfYXQAAAAAAAYAAAAYU3RhYmxlIGlkLCBuZXZlciByZXVzZWQuAAAACWVzY3Jvd19pZAAAAAAAAAYAAABvQ3VtdWxhdGl2ZSBhbW91bnQgYWxyZWFkeSB0cmFuc2ZlcnJlZCB0byB0aGUgc2VsbGVyIHZpYQpgcmVsZWFzZV9wYXJ0aWFsYC4gYDBgIHVudGlsIHRoZSBmaXJzdCBwYXJ0aWFsIHJlbGVhc2UuAAAAAAhyZWxlYXNlZAAAAAsAAAAWUGFydHkgcGFpZCBvbiByZWxlYXNlLgAAAAAABnNlbGxlcgAAAAAAEwAAABhDdXJyZW50IGxpZmVjeWNsZSBzdGF0ZS4AAAAGc3RhdHVzAAAAAAfQAAAADEVzY3Jvd1N0YXR1cwAAAD5TZWNvbmRzIGFmdGVyIGBjcmVhdGVkX2F0YCBhdCB3aGljaCB0aGUgYnV5ZXIgbWF5IHNlbGYtcmVmdW5kLgAAAAAAB3RpbWVvdXQAAAAABgAAAC9TRVAtNDEgdG9rZW4gY29udHJhY3QgY3VzdG9kaWVkIGJ5IHRoaXMgZXNjcm93LgAAAAAFdG9rZW4AAAAAAAAT",
        "AAAAAQAAATBPbmUgbGVnIG9mIGEgYmFza2V0IGVzY3JvdzogYSBzaW5nbGUgU0VQLTQxIHRva2VuIGFuZCBpdHMgYWNjb3VudGluZy4KClRoZSBmaWVsZCBuYW1lcyBtaXJyb3IgW2BFc2Nyb3dEYXRhYF0ncyBzaW5nbGUtdG9rZW4gYWNjb3VudGluZyBleGFjdGx5IHNvCnRoZSB0d28gcmVjb3JkIGtpbmRzIGV4cHJlc3Mgb25lIHJ1bGUgcmF0aGVyIHRoYW4gdHdvOiBgcmVtYWluaW5nYCBpcwpgYW1vdW50IC0gcmVsZWFzZWRgLCBgcmVmdW5kYC9gcmVzb2x2ZWAgcGF5IGByZW1haW5pbmdgLCBhbmQgYSBmdWxsIHJlbGVhc2UKY29uc3VtZXMgaXQuAAAAAAAAAAtFc2Nyb3dBc3NldAAAAAADAAAAXlRvdGFsIGFtb3VudCBvZiBgdG9rZW5gIHRoaXMgbGVnIGVzY3Jvd3MuIEZpeGVkIGF0IGNyZWF0aW9uIGFuZAp1bmNoYW5nZWQgYnkgYGRlcG9zaXRfYmFza2V0YC4AAAAAAAZhbW91bnQAAAAAAAsAAAB6Q3VtdWxhdGl2ZSBhbW91bnQgb2YgYHRva2VuYCBhbHJlYWR5IHBhaWQgdG8gdGhlIHNlbGxlciB2aWEKYHJlbGVhc2VfcGFydGlhbF9iYXNrZXRgLiBgMGAgdW50aWwgdGhlIGZpcnN0IHBhcnRpYWwgcmVsZWFzZS4AAAAAAAhyZWxlYXNlZAAAAAsAAAAjU0VQLTQxIHRva2VuIGNvbnRyYWN0IGZvciB0aGlzIGxlZy4AAAAABXRva2VuAAAAAAAAEw==",
        "AAAAAgAAAB1MaWZlY3ljbGUgc3RhdGUgb2YgYW4gZXNjcm93LgAAAAAAAAAAAAAMRXNjcm93U3RhdHVzAAAABgAAAAAAAAAXQ3JlYXRlZCBidXQgbm90IGZ1bmRlZC4AAAAAB1BlbmRpbmcAAAAAAAAAABxUb2tlbnMgaGVsZCBieSB0aGUgY29udHJhY3QuAAAABkZ1bmRlZAAAAAAAAAAAAD9SZWxlYXNlZCB0byB0aGUgc2VsbGVyIChmdWxsIHJlbGVhc2Ugb3IgZmluYWwgcGFydGlhbCByZWxlYXNlKS4AAAAACUNvbXBsZXRlZAAAAAAAAAAAAAAWUmVmdW5kZWQgdG8gdGhlIGJ1eWVyLgAAAAAACFJlZnVuZGVkAAAAAAAAADxBIHBhcnR5IHJhaXNlZCBhIGRpc3B1dGU7IGZyb3plbiB1bnRpbCB0aGUgYXJiaXRlciByZXNvbHZlcy4AAAAIRGlzcHV0ZWQAAAAAAAAAGUNhbmNlbGxlZCBiZWZvcmUgZnVuZGluZy4AAAAAAAAJQ2FuY2VsbGVkAAAA",
        "AAAAAAAAAFZDYW5jZWwgYSBgUGVuZGluZ2AgYmFza2V0LiBCdXllci1hdXRob3JpemVkOyBub3RoaW5nIGhhcyBtb3ZlZCwgc28gbm8KdHJhbnNmZXIgb2NjdXJzLgAAAAAADWNhbmNlbF9iYXNrZXQAAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAHlDcmVhdGUgYSBiYXNrZXQgZXNjcm93LiBCdXllci1hdXRob3JpemVkIG9ubHksIGV4YWN0bHkgYXMKW2BTZWxmOjpjcmVhdGVfZXNjcm93YF0sIGFuZCBkcmF3aW5nIGZyb20gdGhlIHNhbWUgaWQgc2VxdWVuY2UuAAAAAAAADWNyZWF0ZV9iYXNrZXQAAAAAAAAFAAAAAAAAAAVidXllcgAAAAAAABMAAAAAAAAABnNlbGxlcgAAAAAAEwAAAAAAAAAHYXJiaXRlcgAAAAATAAAAAAAAAAZhc3NldHMAAAAAA+oAAAfQAAAAC0VzY3Jvd0Fzc2V0AAAAAAAAAAAHdGltZW91dAAAAAAGAAAAAQAAA+kAAAAGAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAM5DcmVhdGUgYSBuZXcgZXNjcm93IGFuZCByZXR1cm4gaXRzIHN0YWJsZSBpZC4KCk9ubHkgdGhlIGJ1eWVyIGF1dGhvcml6ZXMgYXQgY3JlYXRpb24uIFRoZSBhcmJpdGVyIGRvZXMgbm90CmF1dGhvcml6ZSBlaXRoZXI6IHRoZXkgbXVzdCBiZSBhYmxlIHRvIGByZXNvbHZlYCBsYXRlciBldmVuIGlmCnRoZXkgbmV2ZXIgcGFydGljaXBhdGVkIGluIGNyZWF0aW9uLgAAAAAADWNyZWF0ZV9lc2Nyb3cAAAAAAAAGAAAAAAAAAAVidXllcgAAAAAAABMAAAAAAAAABnNlbGxlcgAAAAAAEwAAAAAAAAAHYXJiaXRlcgAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAAAAAAHdGltZW91dAAAAAAGAAAAAQAAA+kAAAAGAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAJFQYXkgZXZlcnkgbGVnJ3MgcmVtYWluaW5nIGJhbGFuY2UgYmFjayB0byB0aGUgYnV5ZXIuIFNlbGxlci1hdXRob3JpemVkCmJlZm9yZSB0aGUgZGVhZGxpbmUsIGJ1eWVyLWF1dGhvcml6ZWQgYWZ0ZXIsIGFzIGZvciBhIHNpbmdsZS10b2tlbgplc2Nyb3cuAAAAAAAADXJlZnVuZF9iYXNrZXQAAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAYZSZXNvbHZlIGEgZGlzcHV0ZSBieSBzcGxpdHRpbmcgdGhlIGVzY3Jvd2VkIGFtb3VudCBiZXR3ZWVuIHRoZSBzZWxsZXIKYW5kIGJ1eWVyIGFjY29yZGluZyB0byBgc2VsbGVyX2Jwc2Agb3V0IG9mIDEwXzAwMC4KClRoZSBzZWxsZXIgc2hhcmUgaXMgZmxvb3JlZCBhdCBgYW1vdW50ICogc2VsbGVyX2JwcyAvIDEwXzAwMGAgYW5kIHRoZQpyZW1haW5kZXIgc3RheXMgd2l0aCB0aGUgYnV5ZXIuIGBzZWxsZXJfYnBzID09IDBgIHJlc29sdmVzIHRvIGEKYnV5ZXItb25seSByZWZ1bmQgYW5kIGBzZWxsZXJfYnBzID09IDEwXzAwMGAgcmVzb2x2ZXMgdG8gYSBmdWxsIHNlbGxlcgpwYXlvdXQuIEEgZmFpbGVkIHNlY29uZCBwYXlvdXQgcm9sbHMgYmFjayB0aGUgd2hvbGUgaW52b2NhdGlvbi4AAAAAAA1yZXNvbHZlX3NwbGl0AAAAAAAAAgAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAAAAAAKc2VsbGVyX2JwcwAAAAAABAAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAD1GdW5kIGEgYmFza2V0OiBwdWxsIGV2ZXJ5IGxlZyBmcm9tIHRoZSBidXllci4gQWxsLW9yLW5vdGhpbmcuAAAAAAAADmRlcG9zaXRfYmFza2V0AAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAIRSYWlzZSBhIGRpc3B1dGUgb24gYSBiYXNrZXQuIFRoZSBjbGFpbWFudCBtdXN0IGJlIGEgcGFydHkgYW5kIG11c3QKYXV0aG9yaXplIHRoZSBjYWxsOyBmcmVlemVzIGV2ZXJ5IGxlZyB1bnRpbCB0aGUgYXJiaXRlciByZXNvbHZlcy4AAAAOZGlzcHV0ZV9iYXNrZXQAAAAAAAIAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAAAAAAACGNsYWltYW50AAAAEwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAQ5QZXJtaXNzaW9ubGVzc2x5IHJlZnVuZCB0aGUgYnV5ZXIgYWZ0ZXIgdGhlIGVzY3JvdyBkZWFkbGluZS4KClRoZSBzdHJpY3QtYWZ0ZXIgYm91bmRhcnkgbGVhdmVzIHRoZSBleGFjdCBkZWFkbGluZSB0byB0aGUgZXhpc3RpbmcKcGFydHktYXV0aG9yaXplZCByZWZ1bmQgcGF0aC4gRGlzcHV0ZWQgZXNjcm93cyByZW1haW4gZnJvemVuLCBhbmQgdGhlCnRva2VuIHRyYW5zZmVyIHByZWNlZGVzIHRoZSBzdGF0ZSB1cGRhdGUgc28gYSBmYWlsZWQgcGF5b3V0IGlzIGF0b21pYy4AAAAAAA5yZWZ1bmRfZXhwaXJlZAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAENQYXkgZXZlcnkgbGVnJ3MgcmVtYWluaW5nIGJhbGFuY2UgdG8gdGhlIHNlbGxlci4gU2VsbGVyLWF1dGhvcml6ZWQuAAAAAA5yZWxlYXNlX2Jhc2tldAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAMFSZXNvbHZlIGEgYmFza2V0IGRpc3B1dGU6IGFyYml0ZXIgb25seSwgZmluYWwsIG9uZSBkaXJlY3Rpb24gZm9yIGV2ZXJ5CmxlZy4gUGF5b3V0IGdvZXMgdGhyb3VnaCB0aGUgc2FtZSBsb29wIGFzIGByZWxlYXNlX2Jhc2tldGAsIHNvIGEKcmVzb2x2ZSBjYW4gbmV2ZXIgc2V0dGxlIGxlZ3MgZGlmZmVyZW50bHkgdGhhbiBhIHJlbGVhc2UuAAAAAAAADnJlc29sdmVfYmFza2V0AAAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAAAAABJpbl9mYXZvcl9vZl9zZWxsZXIAAAAAAAEAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAYBSZWxlYXNlIGEgcGFydGlhbCBgYW1vdW50YCB0byB0aGUgc2VsbGVyLiBTZWxsZXItYXV0aG9yaXplZC4KCk9ubHkgdmFsaWQgd2hpbGUgYEZ1bmRlZGAuIGBhbW91bnRgIG11c3QgYmUgcG9zaXRpdmUgYW5kIG11c3Qgbm90CmV4Y2VlZCB0aGUgcmVtYWluaW5nIGJhbGFuY2UuIFdoZW4gYGFtb3VudCA9PSByZW1haW5pbmdgLCB0aGUgZXNjcm93CnRyYW5zaXRpb25zIHRvIGBDb21wbGV0ZWRgIGluIHRoZSBzYW1lIGNhbGwuCgpUcmFuc2Zlci1iZWZvcmUtc3RhdGUgb3JkZXJpbmcgaXMgcHJlc2VydmVkOiB0aGUgdG9rZW4gcGF5b3V0IGhhcHBlbnMKYmVmb3JlIGFueSBzdG9yYWdlIHdyaXRlLCBzbyBhIGZhaWxlZCB0cmFuc2ZlciBsZWF2ZXMgbm8gcGFydGlhbCBzdGF0ZS4AAAAPcmVsZWFzZV9wYXJ0aWFsAAAAAAIAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAQAAAZFBIGJhc2tldCBlc2Nyb3cgcmVjb3JkOiB0aGUgc2FtZSB0aHJlZS1wYXJ0eSBsaWZlY3ljbGUgYXMgW2BFc2Nyb3dEYXRhYF0sCndpdGggYW4gb3JkZXJlZCBsaXN0IG9mIGxlZ3MgaW5zdGVhZCBvZiBvbmUgdG9rZW4uCgpTdG9yZWQgYXQgW2BEYXRhS2V5OjpCYXNrZXRgXSwgYSBzZXBhcmF0ZSBrZXkgY2xhc3MgZnJvbQpbYERhdGFLZXk6OkVzY3Jvd2BdLCBzbyB0aGUgc2luZ2xlLXRva2VuIHJlY29yZCdzIHN0b3JlZCBzaGFwZSBhbmQgdGhlCmV2ZW50cyB0aGF0IGNhcnJ5IGl0IGFyZSB1bnRvdWNoZWQuIElkcyBjb21lIGZyb20gdGhlIHNhbWUKW2BEYXRhS2V5OjpDb3VudGBdIHNlcXVlbmNlLCBzbyBhbiBpZCBiZWxvbmdzIHRvIGV4YWN0bHkgb25lIGVzY3JvdyBvZgpleGFjdGx5IG9uZSBraW5kLgAAAAAAAAAAAAAQQmFza2V0RXNjcm93RGF0YQAAAAgAAABYTmV1dHJhbCBwYXJ0eSBkZWNpZGluZyBkaXNwdXRlcy4gUmVjb3JkZWQgYXQgY3JlYXRpb247IGF1dGhvcml6ZXMgb25seQpgcmVzb2x2ZV9iYXNrZXRgLgAAAAdhcmJpdGVyAAAAABMAAABgVGhlIGJhc2tldDogb25lIGxlZyBwZXIgdG9rZW4sIGluIHRoZSBvcmRlciB0aGUgYnV5ZXIgZGVjbGFyZWQuIE5vIHR3bwpsZWdzIG5hbWUgdGhlIHNhbWUgdG9rZW4uAAAABmFzc2V0cwAAAAAD6gAAB9AAAAALRXNjcm93QXNzZXQAAAAAOlBhcnR5IGZ1bmRpbmcgdGhlIGJhc2tldCBhbmQgdGhlIGRlZmF1bHQgcmVmdW5kIHJlY2lwaWVudC4AAAAAAAVidXllcgAAAAAAABMAAAAbVW5peCB0aW1lc3RhbXAgb2YgY3JlYXRpb24uAAAAAApjcmVhdGVkX2F0AAAAAAAGAAAAR1N0YWJsZSBpZCwgbmV2ZXIgcmV1c2VkLiBTaGFyZXMgdGhlIHNlcXVlbmNlIHdpdGggc2luZ2xlLXRva2VuIGVzY3Jvd3MuAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAFlBhcnR5IHBhaWQgb24gcmVsZWFzZS4AAAAAAAZzZWxsZXIAAAAAABMAAABBQ3VycmVudCBsaWZlY3ljbGUgc3RhdGUuIElkZW50aWNhbCBzZW1hbnRpY3MgdG8gW2BFc2Nyb3dTdGF0dXNgXS4AAAAAAAAGc3RhdHVzAAAAAAfQAAAADEVzY3Jvd1N0YXR1cwAAAD5TZWNvbmRzIGFmdGVyIGBjcmVhdGVkX2F0YCBhdCB3aGljaCB0aGUgYnV5ZXIgbWF5IHNlbGYtcmVmdW5kLgAAAAAAB3RpbWVvdXQAAAAABg==",
        "AAAAAAAAAItQYXkgYGFtb3VudGAgb2Ygb25lIGxlZyB0byB0aGUgc2VsbGVyLiBTZWxsZXItYXV0aG9yaXplZC4KCkFsbCB2YWxpZGF0aW9uIGJlZm9yZSBhbnkgc3RhdGUgY2hhbmdlIG9yIHRyYW5zZmVyLCBtaXJyb3JpbmcKYHJlbGVhc2VfcGFydGlhbGAuAAAAABZyZWxlYXNlX3BhcnRpYWxfYmFza2V0AAAAAAADAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAQAAAKpBIHBhZ2Ugb2YgZXNjcm93IGlkcyBpbnZvbHZpbmcgYSBwYXJ0aWNpcGFudC4KClJldHVybmVkIGJ5IFtgU29yb2JhbkZvcmdlRXNjcm93Ojplc2Nyb3dzX2Zvcl9wYXJ0aWNpcGFudGBdOyBwb3dlcmVkIGJ5CnRoZSBwZXItcGFydHkgcGVyc2lzdGVudCBpbmRleCB3cml0dGVuIGF0IGNyZWF0aW9uLgAAAAAAAAAAABZQYXJ0aWNpcGFudEVzY3Jvd3NQYWdlAAAAAAADAAAAK0VzY3JvdyBpZHMgb24gdGhpcyBwYWdlLCBpbiBjcmVhdGlvbiBvcmRlci4AAAAAA2lkcwAAAAPqAAAABgAAAH1PZmZzZXQgZm9yIHRoZSBuZXh0IHBhZ2UsIG9yIGBOb25lYCB3aGVuIGBpZHNgIGVuZHMgdGhlIHBhcnRpY2lwYW50J3MKbGlzdC4gUmVwbGF5IGl0IHVudGlsIGBOb25lYCB0byBpdGVyYXRlIHRoZSB3aG9sZSBsaXN0LgAAAAAAAAtuZXh0X2N1cnNvcgAAAAPoAAAABAAAADlUb3RhbCBlc2Nyb3dzIGludm9sdmluZyB0aGUgcGFydGljaXBhbnQgYWNyb3NzIGFsbCBwYWdlcy4AAAAAAAAFdG90YWwAAAAAAAAE",
        "AAAAAAAAAc9SZWFkIHRoZSBjdXJyZW50IGNyZWF0aW9uLW9yZGVyIGVzY3JvdyBpZHMgZm9yIGEgcGFydGljaXBhbnQgb25lIHBhZ2UKYXQgYSB0aW1lLiBDYW5jZWxsYXRpb24gcmVtb3ZlcyBpZHMgZnJvbSB0aGUgY29tcGFjdGVkIGluZGV4LiBBIGN1cnNvcgppcyBhbiBvZmZzZXQgaW50byB0aGUgY3VycmVudCBsaXN0OyBpZiBhbiBlYXJsaWVyIGlkIGlzIHJlbW92ZWQgYmV0d2VlbgpwYWdlcywgbGF0ZXIgaWRzIG1heSBzaGlmdCBiZWZvcmUgdGhlIGN1cnNvci4gUmVzdGFydCBhdCB6ZXJvIGFmdGVyCm11dGF0aW9ucyB0byBlbnVtZXJhdGUgdGhlIGN1cnJlbnQgbGlzdCBjb21wbGV0ZWx5LiBUaGlzIHZpZXcgaXRzZWxmCm5ldmVyIG11dGF0ZXMgc3RvcmFnZSBhbmQgbmV2ZXIgZXJyb3JzIOKAlCBhbiB1bmtub3duIGFkZHJlc3Mgb3Igb25lIHdpdGgKbm8gZXNjcm93cyBzaW1wbHkgeWllbGRzIGFuIGVtcHR5IHBhZ2UuAAAAABdlc2Nyb3dzX2Zvcl9wYXJ0aWNpcGFudAAAAAADAAAAAAAAAAtwYXJ0aWNpcGFudAAAAAATAAAAAAAAAAZjdXJzb3IAAAAAAAQAAAAAAAAABWxpbWl0AAAAAAAABAAAAAEAAAfQAAAAFlBhcnRpY2lwYW50RXNjcm93c1BhZ2UAAA==",
        "AAAABQAAAAAAAAAAAAAACERpc3B1dGVkAAAAAQAAAAhkaXNwdXRlZAAAAAIAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAAAAAAAARkYXRhAAAH0AAAAApFc2Nyb3dEYXRhAAAAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAACFJlZnVuZGVkAAAAAQAAAAhyZWZ1bmRlZAAAAAIAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAAAAAAAARkYXRhAAAH0AAAAApFc2Nyb3dEYXRhAAAAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAACFJlbGVhc2VkAAAAAQAAAAhyZWxlYXNlZAAAAAIAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAAAAAAAARkYXRhAAAH0AAAAApFc2Nyb3dEYXRhAAAAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAACFJlc29sdmVkAAAAAQAAAAhyZXNvbHZlZAAAAAMAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAAAAAAAARkYXRhAAAH0AAAAApFc2Nyb3dEYXRhAAAAAAAAAAAAAAAAABJpbl9mYXZvcl9vZl9zZWxsZXIAAAAAAAEAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAACUNhbmNlbGxlZAAAAAAAAAEAAAAJY2FuY2VsbGVkAAAAAAAAAgAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAAAAAAABGRhdGEAAAfQAAAACkVzY3Jvd0RhdGEAAAAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAACURlcG9zaXRlZAAAAAAAAAEAAAAJZGVwb3NpdGVkAAAAAAAAAgAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAAAAAAABGRhdGEAAAfQAAAACkVzY3Jvd0RhdGEAAAAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAADUJhc2tldENyZWF0ZWQAAAAAAAABAAAADmJhc2tldF9jcmVhdGVkAAAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAQQmFza2V0RXNjcm93RGF0YQAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAADUVzY3Jvd0NyZWF0ZWQAAAAAAAABAAAADmVzY3Jvd19jcmVhdGVkAAAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAKRXNjcm93RGF0YQAAAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAADVJlZnVuZEV4cGlyZWQAAAAAAAABAAAADnJlZnVuZF9leHBpcmVkAAAAAAADAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAPcmVmdW5kZWRfYW1vdW50AAAAAAsAAAAAAAAAAAAAAAl0aW1lc3RhbXAAAAAAAAAGAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAADVJlc29sdmVkU3BsaXQAAAAAAAABAAAADnJlc29sdmVkX3NwbGl0AAAAAAAFAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAKRXNjcm93RGF0YQAAAAAAAAAAAAAAAAAKc2VsbGVyX2JwcwAAAAAABAAAAAAAAAAAAAAADHNlbGxlcl9zaGFyZQAAAAsAAAAAAAAAAAAAAAtidXllcl9zaGFyZQAAAAALAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAADkJhc2tldERpc3B1dGVkAAAAAAABAAAAD2Jhc2tldF9kaXNwdXRlZAAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAQQmFza2V0RXNjcm93RGF0YQAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAADkJhc2tldFJlZnVuZGVkAAAAAAABAAAAD2Jhc2tldF9yZWZ1bmRlZAAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAQQmFza2V0RXNjcm93RGF0YQAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAADkJhc2tldFJlbGVhc2VkAAAAAAABAAAAD2Jhc2tldF9yZWxlYXNlZAAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAQQmFza2V0RXNjcm93RGF0YQAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAADkJhc2tldFJlc29sdmVkAAAAAAABAAAAD2Jhc2tldF9yZXNvbHZlZAAAAAADAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAQQmFza2V0RXNjcm93RGF0YQAAAAAAAAAAAAAAEmluX2Zhdm9yX29mX3NlbGxlcgAAAAAAAQAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAD0Jhc2tldENhbmNlbGxlZAAAAAABAAAAEGJhc2tldF9jYW5jZWxsZWQAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAQQmFza2V0RXNjcm93RGF0YQAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAD0Jhc2tldERlcG9zaXRlZAAAAAABAAAAEGJhc2tldF9kZXBvc2l0ZWQAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAAAAAAAEZGF0YQAAB9AAAAAQQmFza2V0RXNjcm93RGF0YQAAAAAAAAAC",
        "AAAABQAAAS1FbWl0dGVkIGJ5IGByZWxlYXNlX3BhcnRpYWxgIGZvciBldmVyeSBpbmNyZW1lbnRhbCBzZWxsZXIgcGF5b3V0LgoKYHBhcnRpYWxfYW1vdW50YCBpcyB0aGUgYW1vdW50IHRyYW5zZmVycmVkIGluIHRoaXMgY2FsbDsgYGRhdGFgCmNhcnJpZXMgdGhlIHBvc3QtdXBkYXRlIGBFc2Nyb3dEYXRhYCAoaW5jbHVkaW5nIHVwZGF0ZWQgYHJlbGVhc2VkYAphbmQgdGhlIG5ldyBgc3RhdHVzYCkuIENoZWNrIGBkYXRhLnN0YXR1c2AgdG8gZGV0ZXJtaW5lIHdoZXRoZXIKdGhpcyBwYXJ0aWFsIHJlbGVhc2Ugd2FzIHRoZSBmaW5hbCBvbmUuAAAAAAAAAAAAABFQYXJ0aWFsbHlSZWxlYXNlZAAAAAAAAAEAAAAScGFydGlhbGx5X3JlbGVhc2VkAAAAAAADAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAAD1UaGUgYW1vdW50IHRyYW5zZmVycmVkIHRvIHRoZSBzZWxsZXIgaW4gdGhpcyBwYXJ0aWN1bGFyIGNhbGwuAAAAAAAADnBhcnRpYWxfYW1vdW50AAAAAAALAAAAAAAAAGBGdWxsIGVzY3JvdyByZWNvcmQgYWZ0ZXIgdGhpcyBwYXJ0aWFsIHJlbGVhc2UsIGluY2x1ZGluZyB1cGRhdGVkCmByZWxlYXNlZGAgYW5kIGBzdGF0dXNgIGZpZWxkcy4AAAAEZGF0YQAAB9AAAAAKRXNjcm93RGF0YQAAAAAAAAAAAAI=",
        "AAAABQAAARNFbWl0dGVkIGJ5IGByZWxlYXNlX3BhcnRpYWxfYmFza2V0YCBmb3IgZXZlcnkgaW5jcmVtZW50YWwgcGF5b3V0IG9mIG9uZQpsZWcuIGB0b2tlbmAgYW5kIGBwYXJ0aWFsX2Ftb3VudGAgZGVzY3JpYmUgdGhlIHRyYW5zZmVycmVkIGxlZzsgYGRhdGFgCmNhcnJpZXMgdGhlIHBvc3QtdXBkYXRlIGBCYXNrZXRFc2Nyb3dEYXRhYCAocGVyLWxlZyBgcmVsZWFzZWRgIGFuZCB0aGUKcG9zc2libGUgYENvbXBsZXRlZGAgc3RhdHVzIG9uY2UgKipldmVyeSoqIGxlZyBpcyBleGhhdXN0ZWQpLgAAAAAAAAAAF0Jhc2tldFBhcnRpYWxseVJlbGVhc2VkAAAAAAEAAAAZYmFza2V0X3BhcnRpYWxseV9yZWxlYXNlZAAAAAAAAAQAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAAI1RoZSBsZWcgdGhhdCB3YXMgcGFpZCBpbiB0aGlzIGNhbGwuAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAPVRoZSBhbW91bnQgdHJhbnNmZXJyZWQgdG8gdGhlIHNlbGxlciBpbiB0aGlzIHBhcnRpY3VsYXIgY2FsbC4AAAAAAAAOcGFydGlhbF9hbW91bnQAAAAAAAsAAAAAAAAALkZ1bGwgYmFza2V0IHJlY29yZCBhZnRlciB0aGlzIHBhcnRpYWwgcmVsZWFzZS4AAAAAAARkYXRhAAAH0AAAABBCYXNrZXRFc2Nyb3dEYXRhAAAAAAAAAAI=",
        "AAAABAAAAZxTaGFyZWQgZXJyb3IgdHlwZSB1c2VkIGFjcm9zcyBhbGwgU29yb2JhbiBGb3JnZSBjb250cmFjdHMuCgpEZWZpbmluZyBhIHNpbmdsZSBlcnJvciBlbnVtIGluIGBzaGFyZWQtdXRpbHNgIGtlZXBzIHRoZSBvbi1jaGFpbiBlcnJvcgpzcGFjZSBjb25zaXN0ZW50IGFuZCBpbnRlbGxpZ2libGUgdG8gU0RLIGNvbnN1bWVycywgYW5kIGF2b2lkcyBldmVyeQpjb250cmFjdCByZS1kZWNsYXJpbmcgdGhlIHNhbWUgZmFpbHVyZSBtb2Rlcy4gQ29udHJhY3QgY3JhdGVzIG1heSBleHBvc2UKdGhlaXIgb3duIGRvbWFpbi1zcGVjaWZpYyBlcnJvcnMsIGJ1dCBzaG91bGQgcHJlZmVyIHRoZXNlIHdoZXJlIHRoZXkgZml0LgoKRXJyb3IgY29kZXMgc3RhcnQgYXQgMTsgY29kZSAwIGlzIHJlc2VydmVkIGJ5IHRoZSBTb3JvYmFuIGhvc3QuAAAAAAAAAApGb3JnZUVycm9yAAAAAAAPAAAAM1RoZSBjYWxsZXIgaXMgbm90IHBlcm1pdHRlZCB0byBwZXJmb3JtIHRoaXMgYWN0aW9uLgAAAAAMVW5hdXRob3JpemVkAAAAAQAAAEpUaGUgcmVxdWVzdGVkIGVudGl0eSAoZXNjcm93LCBwcm9wb3NhbCwgc3Vic2NyaXB0aW9uLCAuLi4pIGRvZXMgbm90IGV4aXN0LgAAAAAACE5vdEZvdW5kAAAAAgAAAEhPbmUgb3IgbW9yZSBhcmd1bWVudHMgZmFpbGVkIHZhbGlkYXRpb24gKGUuZy4gemVybyBhbW91bnQsIGJhZCBhZGRyZXNzKS4AAAAMSW52YWxpZElucHV0AAAAAwAAAENUaGUgY29udHJhY3QgZG9lcyBub3QgaG9sZCBlbm91Z2ggYmFsYW5jZSB0byBzYXRpc2Z5IHRoZSBvcGVyYXRpb24uAAAAABFJbnN1ZmZpY2llbnRGdW5kcwAAAAAAAAQAAABCVGhlIGVudGl0eSB3YXMgYWxyZWFkeSBpbml0aWFsaXNlZDsgcmUtaW5pdGlhbGlzYXRpb24gaXMgcmVqZWN0ZWQuAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAAFAAAANlRoZSBlbnRpdHkgd2FzIGV4cGVjdGVkIHRvIGJlIGluaXRpYWxpc2VkIGJ1dCB3YXMgbm90LgAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAGAAAANkFuIG9wZXJhdGlvbiB3YXMgYXR0ZW1wdGVkIGFmdGVyIGl0cyBkZWFkbGluZSBlbGFwc2VkLgAAAAAAD0RlYWRsaW5lUmVhY2hlZAAAAAAHAAAAQUEgcmVxdWlyZWQgdG9rZW4gYWxsb3dhbmNlIHdhcyBsb3dlciB0aGFuIHRoZSBhbW91bnQgYmVpbmcgc3BlbnQuAAAAAAAAFUluc3VmZmljaWVudEFsbG93YW5jZQAAAAAAAAgAAAAjQW4gYXJpdGhtZXRpYyBvcGVyYXRpb24gb3ZlcmZsb3dlZC4AAAAAEkFyaXRobWV0aWNPdmVyZmxvdwAAAAAACQAAAEJDb250cmFjdC1zcGVjaWZpYyBlcnJvciB0aGF0IGRvZXMgbm90IG1hcCB0byB0aGUgY2F0ZWdvcmllcyBhYm92ZS4AAAAAAAZDdXN0b20AAAAAAAoAAAFsQSBTRVAtNDEgdG9rZW4gaW52b2NhdGlvbiBmYWlsZWQgKGluc3VmZmljaWVudCBiYWxhbmNlLCBtaXNzaW5nIG9yCmRlYXV0aG9yaXplZCB0cnVzdGxpbmUsIHVuZGVwbG95ZWQgdG9rZW4gY29udHJhY3QsIG9yIHRva2VuLWxvZ2ljCnJlamVjdGlvbikuIFRoZSByYXcgdG9rZW4gZXJyb3IgZGlzY3JpbWluYW50IGlzIGludGVudGlvbmFsbHkgbm90CmZvcndhcmRlZCDigJQgY2FsbGVycyBjYW5ub3QgdGVsbCB3aGljaCBjb250cmFjdCBwcm9kdWNlZCBhIGZvcndhcmRlZApjb2RlLCBzbyBpdCBpcyBidWNrZXRlZDsgdGhlIHJvb3QgY2F1c2UgcmVtYWlucyB2aXNpYmxlIGluIHRoZQp0cmFuc2FjdGlvbidzIGRpYWdub3N0aWMgZXZlbnRzLgAAABNUb2tlblRyYW5zZmVyRmFpbGVkAAAAAAsAAADbQSBjcm9zcy1jb250cmFjdCBpbnZvY2F0aW9uIGZhaWxlZCAodGFyZ2V0IHJldmVydGVkIG9yIGhvc3QKYWJvcnQpLiBUaGUgdGFyZ2V0IHJldmVydHMgYXJlIHN1cmZhY2VkIGhlcmUgc28gdGhlIGNhbGxlciBjYW4KZGlzdGluZ3Vpc2ggdGhlbSBmcm9tIHRva2VuLXRyYW5zZmVyIGZhaWx1cmVzLCBhbmQgdGhlIGludm9raW5nCnRyYW5zYWN0aW9uIGlzIGxlZnQgdW4tZXhlY3V0ZWQuAAAAABhDb250cmFjdEludm9jYXRpb25GYWlsZWQAAAAMAAABDFRoZSByZXF1ZXN0ZWQgd2l0aGRyYXdhbCB3b3VsZCBwdXNoIGEgdG9rZW4ncyByb2xsaW5nLXdpbmRvdyB0b3RhbApwYXN0IGl0cyBjb25maWd1cmVkIHdpdGhkcmF3YWwgbGltaXQuIEtlcHQgZGlzdGluY3QgZnJvbQpbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIHNvIGEgY2FsbGVyIGNhbiB0ZWxsIGEgcG9saWN5IHJlamVjdGlvbgooYSB2YWxpZCB3aXRoZHJhd2FsIHRoYXQgaXMgdG9vIGxhcmdlIHJpZ2h0IG5vdykgZnJvbSBhIG1hbGZvcm1lZAphcmd1bWVudC4AAAAXV2l0aGRyYXdhbExpbWl0RXhjZWVkZWQAAAAADQAAARtBIHN1YnNjcmlwdGlvbiBpcyBpbiB0aGUgYFBhc3REdWVgIGxhcHNlZCBzdGF0ZSBhbmQgdGhlIHJlcXVlc3RlZApvcGVyYXRpb24gY2Fubm90IGJlIHBlcmZvcm1lZCB1bnRpbCBhIGNhdGNoLXVwIGNoYXJnZSByZXN0b3JlcyBpdCB0bwpgQWN0aXZlYC4gS2VwdCBkaXN0aW5jdCBmcm9tIFtgRm9yZ2VFcnJvcjo6RGVhZGxpbmVSZWFjaGVkYF0gc28gYSBjYWxsZXIKY2FuIHRlbGwgYSBsYXBzZWQgYmlsbGluZyBzdGF0ZSBmcm9tIGFuIGFyZ3VtZW50IHRoYXQgd2FzIG1lcmVseQp0b28gZWFybHkuAAAAABNTdWJzY3JpcHRpb25QYXN0RHVlAAAAAA4AAABxQSBwcm9wb3NlciBoYXMgcmVhY2hlZCB0aGUgbWF4aW11bSBhbGxvd2VkIGNvbmN1cnJlbnQgYWN0aXZlIHByb3Bvc2FscwpvciBpcyB3aXRoaW4gdGhlIHByb3Bvc2VyIGNvb2xkb3duIHdpbmRvdy4AAAAAAAAQUHJvcG9zZXJDb29sZG93bgAAAA8=" ]),
      options
    )
  }
  public readonly fromJSON = {
    cancel: this.txFromJSON<Result<void>>,
        refund: this.txFromJSON<Result<void>>,
        deposit: this.txFromJSON<Result<void>>,
        dispute: this.txFromJSON<Result<void>>,
        release: this.txFromJSON<Result<void>>,
        resolve: this.txFromJSON<Result<void>>,
        ttl_info: this.txFromJSON<Result<u32>>,
        touch_ttl: this.txFromJSON<Result<void>>,
        get_basket: this.txFromJSON<Result<BasketEscrowData>>,
        get_escrow: this.txFromJSON<Result<EscrowData>>,
        get_status: this.txFromJSON<Result<EscrowStatus>>,
        cancel_basket: this.txFromJSON<Result<void>>,
        create_basket: this.txFromJSON<Result<u64>>,
        create_escrow: this.txFromJSON<Result<u64>>,
        refund_basket: this.txFromJSON<Result<void>>,
        resolve_split: this.txFromJSON<Result<void>>,
        deposit_basket: this.txFromJSON<Result<void>>,
        dispute_basket: this.txFromJSON<Result<void>>,
        refund_expired: this.txFromJSON<Result<void>>,
        release_basket: this.txFromJSON<Result<void>>,
        resolve_basket: this.txFromJSON<Result<void>>,
        release_partial: this.txFromJSON<Result<void>>,
        release_partial_basket: this.txFromJSON<Result<void>>,
        escrows_for_participant: this.txFromJSON<ParticipantEscrowsPage>
  }
}

/**
 * Automatically pages through participant escrows using a page provider function.
 * Stops cleanly when next_cursor is null/undefined and preserves on-chain creation order.
 */
export async function paginateParticipantEscrows(
  fetchPage: (cursor: u32, limit: u32) => Promise<ParticipantEscrowsPage>,
  limit: u32 = 50,
): Promise<Array<u64>> {
  const ids: Array<u64> = [];
  let cursor: u32 | undefined = 0;

  while (cursor !== undefined && cursor !== null) {
    const page = await fetchPage(cursor, limit);
    ids.push(...page.ids);
    if (page.next_cursor === null || page.next_cursor === undefined) {
      break;
    }
    cursor = page.next_cursor;
  }

  return ids;
}
