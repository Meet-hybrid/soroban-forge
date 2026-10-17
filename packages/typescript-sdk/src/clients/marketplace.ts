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
 * A royalty configuration for a single collection.
 */
export interface Royalty {
  /**
 * Royalty rate in basis points (100 bps = 1%).
 */
bps: u32;
  /**
 * Collection (NFT contract) this configuration applies to.
 */
collection: string;
  /**
 * Address entitled to royalty payments.
 */
recipient: string;
  /**
 * Whether the configuration is currently enforced.
 */
status: RoyaltyStatus;
}


/**
 * The split one sale of a collection would apply at settlement time, as
 * returned by [`MarketplaceRoyalties::quote_sale`]. For a split config,
 * `royalty_amount` sums the independently floored share for each recipient.
 *
 * Per-sale counterpart of [`SettlementSummary`]: where the summary
 * accumulates what *was* settled, a quote derives what one settlement
 * *would* move. `royalty_amount + seller_net == gross` exactly — the
 * floor-rounding remainder stays with the seller, as in every settled
 * sale.
 */
export interface SaleQuote {
  /**
 * The sale amount the quote was computed for.
 */
gross: i128;
  /**
 * Royalty share: `gross * royalty_bps / 10_000`, floored — the exact
 * amount a settlement would transfer to the configured recipient.
 */
royalty_amount: i128;
  /**
 * Effective royalty rate applied, in basis points. Zero for a
 * `Disabled` configuration — matching `settle_sale`, which settles
 * such a sale in full to the seller.
 */
royalty_bps: u32;
  /**
 * Seller's net: `gross - royalty_amount` — the exact amount a
 * settlement would transfer to the seller.
 */
seller_net: i128;
}


/**
 * The aggregate royalty amount and seller amount for one atomic settlement:
 * one `settle_sale` invocation, or one sale of a `settle_sales` batch.
 * `royalty_share` aggregates every recipient's separately floored payment.
 */
export interface Settlement {
  /**
 * Total amount transferred to all configured royalty recipients.
 */
royalty_share: i128;
  /**
 * Amount transferred to the seller.
 */
seller_net: i128;
}


/**
 * One recipient and its basis-point rate in a collection split.
 */
export interface RoyaltyShare {
  bps: u32;
  recipient: string;
}

/**
 * Lifecycle state of a registered royalty configuration.
 */
export type RoyaltyStatus = {tag: "Active", values: void} | {tag: "Disabled", values: void};


/**
 * Cumulative settlement totals for one collection.
 */
export interface SettlementSummary {
  /**
 * Sum of every settled sale amount.
 */
gross_volume: i128;
  /**
 * Sum of every royalty share transferred across all recipients.
 */
royalties_paid: i128;
  /**
 * Number of sales settled so far.
 */
sales: u32;
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
   * Construct and simulate a touch_ttl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Permissionless keeper: bump the royalty and summary entries' TTL without changing
   * any state.
   *
   * Returns `ForgeError::NotFound` if no royalty configuration exists for `collection`.
   */
  touch_ttl: ({collection}: {collection: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a distribute transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Distribute the royalty share of `amount` from a sale of `collection`
   * in `token`.
   *
   * Requires the collection's authorization (the payer's authorization
   * covers the nested token transfer, exactly as `settle_sale`) and
   * `amount > 0`. Computes the split with
   * `split` — the same math as `settle_sale` — then transfers only the
   * royalty share from `payer` to the configured recipient **before any
   * settlement state is committed**. The `seller`'s net is *not*
   * transferred here: this is a standalone royalty settlement for cases
   * where the underlying sale/payment is handled outside `settle_sale`,
   * so the caller is responsible for paying the seller separately. The
   * net is still returned so the caller knows what the seller is owed.
   * A `Disabled` or zero-bps configuration transfers nothing (share is
   * zero) and settles in full to the seller by returning the full
   * amount. Token failures are bucketed into
   * [`ForgeError::TokenTransferFailed`], and any returned error rolls
   * the whole invocation back — a failed transfer never commits total
   */
  distribute: ({collection, token, payer, seller, amount}: {collection: string, token: string, payer: string, seller: string, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a quote_sale transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Quote the exact split a settlement of `amount` for `collection`
   * would apply (read-only view).
   *
   * Settle-parity: the quote runs the same validation order and the same
   * split derivation as the settlement entrypoints — configuration load
   * (`NotFound`), `amount > 0` (`InvalidInput`, mirroring
   * `distribute`/`settle_sale`), then `effective_bps` + `split` —
   * so the quote never succeeds where `settle_sale` would fail, and the
   * returned numbers are the settlement's own, rounding included. A
   * `Disabled` configuration quotes at zero bps, matching `settle_sale`'s
   * settle-in-full behavior. No storage mutation, no authorization, no
   * events.
   */
  quote_sale: ({collection, amount}: {collection: string, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<SaleQuote>>>

  /**
   * Construct and simulate a get_royalty transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the stored royalty configuration for `collection` (read-only view).
   */
  get_royalty: ({collection}: {collection: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Royalty>>>

  /**
   * Construct and simulate a set_royalty transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Register or update a royalty configuration for `collection`.
   *
   * Requires the collection's authorization and `bps <= 10_000`
   * (100%). Re-registration updates the existing configuration in place.
   */
  set_royalty: ({collection, recipient, bps}: {collection: string, recipient: string, bps: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a settle_sale transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Settle a sale of `collection` atomically in `token`.
   *
   * Requires the collection's authorization (as `distribute` does)
   * and the `payer`'s, which covers both nested token transfers. Requires
   * `amount > 0` and a stored configuration. The split is computed with
   * checked arithmetic, and both transfers run **before any settlement
   * state is committed**: the seller's net first, the royalty recipient
   * last, so a failed transfer can never leave the royalty recipient
   * partially paid. A `Disabled` or zero-bps configuration settles the
   * full amount to the seller in a single transfer. Token failures are
   * bucketed into [`ForgeError::TokenTransferFailed`], and any returned
   * error rolls the whole invocation back — including an earlier
   * successful transfer — so retrying after a failure never double-pays.
   * Use `settle_sales` to settle a batch of sales in one invocation.
   */
  settle_sale: ({collection, token, payer, seller, amount}: {collection: string, token: string, payer: string, seller: string, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Settlement>>>

  /**
   * Construct and simulate a settle_sales transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Settle a batch of sales of `collection` atomically in `token`.
   *
   * Same trust model as `settle_sale`: the collection's authorization and
   * the `payer`'s are each required once, and the payer's single
   * authorization covers **every** nested token transfer in the batch —
   * there is no per-sale re-authorization.
   *
   * Validation runs to completion before the first transfer: the
   * configuration must exist ([`ForgeError::NotFound`]), `sales` must be
   * non-empty and at most [`MAX_SETTLE_SALES`] long
   * ([`ForgeError::InvalidInput`]), every `amount` must be positive
   * ([`ForgeError::InvalidInput`]), and every split plus the batch
   * aggregate — including the checked add against the stored summary —
   * must be overflow-free ([`ForgeError::ArithmeticOverflow`]). Only
   * then do transfers run, in sale order, each sale seller-then-recipient
   * with the recipient skipped when its share is zero, exactly as
   * `settle_sale` does. A failure in any sale rolls the whole invocation
   * back: no sale is half-settled, no balance moves, and the summary is
   * untouched
   */
  settle_sales: ({collection, token, payer, sales}: {collection: string, token: string, payer: string, sales: Array<readonly [string, i128]>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Array<Settlement>>>>

  /**
   * Construct and simulate a set_royalty_splits transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Replace a collection's split atomically. Zero-rate recipients are valid.
   */
  set_royalty_splits: ({collection, recipients}: {collection: string, recipients: Array<RoyaltyShare>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_settlement_summary transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the cumulative settlement totals for `collection` (read-only
   * view). `NotFound` until the collection settles its first sale.
   */
  get_settlement_summary: ({collection}: {collection: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<SettlementSummary>>>

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
      new ContractSpec([ "AAAAAQAAADBBIHJveWFsdHkgY29uZmlndXJhdGlvbiBmb3IgYSBzaW5nbGUgY29sbGVjdGlvbi4AAAAAAAAAB1JveWFsdHkAAAAABAAAACxSb3lhbHR5IHJhdGUgaW4gYmFzaXMgcG9pbnRzICgxMDAgYnBzID0gMSUpLgAAAANicHMAAAAABAAAADhDb2xsZWN0aW9uIChORlQgY29udHJhY3QpIHRoaXMgY29uZmlndXJhdGlvbiBhcHBsaWVzIHRvLgAAAApjb2xsZWN0aW9uAAAAAAATAAAAJUFkZHJlc3MgZW50aXRsZWQgdG8gcm95YWx0eSBwYXltZW50cy4AAAAAAAAJcmVjaXBpZW50AAAAAAAAEwAAADBXaGV0aGVyIHRoZSBjb25maWd1cmF0aW9uIGlzIGN1cnJlbnRseSBlbmZvcmNlZC4AAAAGc3RhdHVzAAAAAAfQAAAADVJveWFsdHlTdGF0dXMAAAA=",
        "AAAAAQAAAepUaGUgc3BsaXQgb25lIHNhbGUgb2YgYSBjb2xsZWN0aW9uIHdvdWxkIGFwcGx5IGF0IHNldHRsZW1lbnQgdGltZSwgYXMKcmV0dXJuZWQgYnkgW2BNYXJrZXRwbGFjZVJveWFsdGllczo6cXVvdGVfc2FsZWBdLiBGb3IgYSBzcGxpdCBjb25maWcsCmByb3lhbHR5X2Ftb3VudGAgc3VtcyB0aGUgaW5kZXBlbmRlbnRseSBmbG9vcmVkIHNoYXJlIGZvciBlYWNoIHJlY2lwaWVudC4KClBlci1zYWxlIGNvdW50ZXJwYXJ0IG9mIFtgU2V0dGxlbWVudFN1bW1hcnlgXTogd2hlcmUgdGhlIHN1bW1hcnkKYWNjdW11bGF0ZXMgd2hhdCAqd2FzKiBzZXR0bGVkLCBhIHF1b3RlIGRlcml2ZXMgd2hhdCBvbmUgc2V0dGxlbWVudAoqd291bGQqIG1vdmUuIGByb3lhbHR5X2Ftb3VudCArIHNlbGxlcl9uZXQgPT0gZ3Jvc3NgIGV4YWN0bHkg4oCUIHRoZQpmbG9vci1yb3VuZGluZyByZW1haW5kZXIgc3RheXMgd2l0aCB0aGUgc2VsbGVyLCBhcyBpbiBldmVyeSBzZXR0bGVkCnNhbGUuAAAAAAAAAAAACVNhbGVRdW90ZQAAAAAAAAQAAAArVGhlIHNhbGUgYW1vdW50IHRoZSBxdW90ZSB3YXMgY29tcHV0ZWQgZm9yLgAAAAAFZ3Jvc3MAAAAAAAALAAAAhFJveWFsdHkgc2hhcmU6IGBncm9zcyAqIHJveWFsdHlfYnBzIC8gMTBfMDAwYCwgZmxvb3JlZCDigJQgdGhlIGV4YWN0CmFtb3VudCBhIHNldHRsZW1lbnQgd291bGQgdHJhbnNmZXIgdG8gdGhlIGNvbmZpZ3VyZWQgcmVjaXBpZW50LgAAAA5yb3lhbHR5X2Ftb3VudAAAAAAACwAAAKFFZmZlY3RpdmUgcm95YWx0eSByYXRlIGFwcGxpZWQsIGluIGJhc2lzIHBvaW50cy4gWmVybyBmb3IgYQpgRGlzYWJsZWRgIGNvbmZpZ3VyYXRpb24g4oCUIG1hdGNoaW5nIGBzZXR0bGVfc2FsZWAsIHdoaWNoIHNldHRsZXMKc3VjaCBhIHNhbGUgaW4gZnVsbCB0byB0aGUgc2VsbGVyLgAAAAAAAAtyb3lhbHR5X2JwcwAAAAAEAAAAZlNlbGxlcidzIG5ldDogYGdyb3NzIC0gcm95YWx0eV9hbW91bnRgIOKAlCB0aGUgZXhhY3QgYW1vdW50IGEKc2V0dGxlbWVudCB3b3VsZCB0cmFuc2ZlciB0byB0aGUgc2VsbGVyLgAAAAAACnNlbGxlcl9uZXQAAAAAAAs=",
        "AAAAAQAAANdUaGUgYWdncmVnYXRlIHJveWFsdHkgYW1vdW50IGFuZCBzZWxsZXIgYW1vdW50IGZvciBvbmUgYXRvbWljIHNldHRsZW1lbnQ6Cm9uZSBgc2V0dGxlX3NhbGVgIGludm9jYXRpb24sIG9yIG9uZSBzYWxlIG9mIGEgYHNldHRsZV9zYWxlc2AgYmF0Y2guCmByb3lhbHR5X3NoYXJlYCBhZ2dyZWdhdGVzIGV2ZXJ5IHJlY2lwaWVudCdzIHNlcGFyYXRlbHkgZmxvb3JlZCBwYXltZW50LgAAAAAAAAAAClNldHRsZW1lbnQAAAAAAAIAAAA+VG90YWwgYW1vdW50IHRyYW5zZmVycmVkIHRvIGFsbCBjb25maWd1cmVkIHJveWFsdHkgcmVjaXBpZW50cy4AAAAAAA1yb3lhbHR5X3NoYXJlAAAAAAAACwAAACFBbW91bnQgdHJhbnNmZXJyZWQgdG8gdGhlIHNlbGxlci4AAAAAAAAKc2VsbGVyX25ldAAAAAAACw==",
        "AAAAAQAAAD1PbmUgcmVjaXBpZW50IGFuZCBpdHMgYmFzaXMtcG9pbnQgcmF0ZSBpbiBhIGNvbGxlY3Rpb24gc3BsaXQuAAAAAAAAAAAAAAxSb3lhbHR5U2hhcmUAAAACAAAAAAAAAANicHMAAAAABAAAAAAAAAAJcmVjaXBpZW50AAAAAAAAEw==",
        "AAAAAgAAADZMaWZlY3ljbGUgc3RhdGUgb2YgYSByZWdpc3RlcmVkIHJveWFsdHkgY29uZmlndXJhdGlvbi4AAAAAAAAAAAANUm95YWx0eVN0YXR1cwAAAAAAAAIAAAAAAAAAHEFjdGl2ZSBhbmQgYXBwbGllZCB0byBzYWxlcy4AAAAGQWN0aXZlAAAAAAAAAAAALURpc2FibGVkOyBzYWxlcyBzZXR0bGUgdG8gdGhlIHNlbGxlciBpbiBmdWxsLgAAAAAAAAhEaXNhYmxlZA==",
        "AAAAAQAAADBDdW11bGF0aXZlIHNldHRsZW1lbnQgdG90YWxzIGZvciBvbmUgY29sbGVjdGlvbi4AAAAAAAAAEVNldHRsZW1lbnRTdW1tYXJ5AAAAAAAAAwAAACFTdW0gb2YgZXZlcnkgc2V0dGxlZCBzYWxlIGFtb3VudC4AAAAAAAAMZ3Jvc3Nfdm9sdW1lAAAACwAAAD1TdW0gb2YgZXZlcnkgcm95YWx0eSBzaGFyZSB0cmFuc2ZlcnJlZCBhY3Jvc3MgYWxsIHJlY2lwaWVudHMuAAAAAAAADnJveWFsdGllc19wYWlkAAAAAAALAAAAH051bWJlciBvZiBzYWxlcyBzZXR0bGVkIHNvIGZhci4AAAAABXNhbGVzAAAAAAAABA==",
        "AAAAAAAAALFQZXJtaXNzaW9ubGVzcyBrZWVwZXI6IGJ1bXAgdGhlIHJveWFsdHkgYW5kIHN1bW1hcnkgZW50cmllcycgVFRMIHdpdGhvdXQgY2hhbmdpbmcKYW55IHN0YXRlLgoKUmV0dXJucyBgRm9yZ2VFcnJvcjo6Tm90Rm91bmRgIGlmIG5vIHJveWFsdHkgY29uZmlndXJhdGlvbiBleGlzdHMgZm9yIGBjb2xsZWN0aW9uYC4AAAAAAAAJdG91Y2hfdHRsAAAAAAAAAQAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAABABEaXN0cmlidXRlIHRoZSByb3lhbHR5IHNoYXJlIG9mIGBhbW91bnRgIGZyb20gYSBzYWxlIG9mIGBjb2xsZWN0aW9uYAppbiBgdG9rZW5gLgoKUmVxdWlyZXMgdGhlIGNvbGxlY3Rpb24ncyBhdXRob3JpemF0aW9uICh0aGUgcGF5ZXIncyBhdXRob3JpemF0aW9uCmNvdmVycyB0aGUgbmVzdGVkIHRva2VuIHRyYW5zZmVyLCBleGFjdGx5IGFzIGBzZXR0bGVfc2FsZWApIGFuZApgYW1vdW50ID4gMGAuIENvbXB1dGVzIHRoZSBzcGxpdCB3aXRoCmBzcGxpdGAg4oCUIHRoZSBzYW1lIG1hdGggYXMgYHNldHRsZV9zYWxlYCDigJQgdGhlbiB0cmFuc2ZlcnMgb25seSB0aGUKcm95YWx0eSBzaGFyZSBmcm9tIGBwYXllcmAgdG8gdGhlIGNvbmZpZ3VyZWQgcmVjaXBpZW50ICoqYmVmb3JlIGFueQpzZXR0bGVtZW50IHN0YXRlIGlzIGNvbW1pdHRlZCoqLiBUaGUgYHNlbGxlcmAncyBuZXQgaXMgKm5vdCoKdHJhbnNmZXJyZWQgaGVyZTogdGhpcyBpcyBhIHN0YW5kYWxvbmUgcm95YWx0eSBzZXR0bGVtZW50IGZvciBjYXNlcwp3aGVyZSB0aGUgdW5kZXJseWluZyBzYWxlL3BheW1lbnQgaXMgaGFuZGxlZCBvdXRzaWRlIGBzZXR0bGVfc2FsZWAsCnNvIHRoZSBjYWxsZXIgaXMgcmVzcG9uc2libGUgZm9yIHBheWluZyB0aGUgc2VsbGVyIHNlcGFyYXRlbHkuIFRoZQpuZXQgaXMgc3RpbGwgcmV0dXJuZWQgc28gdGhlIGNhbGxlciBrbm93cyB3aGF0IHRoZSBzZWxsZXIgaXMgb3dlZC4KQSBgRGlzYWJsZWRgIG9yIHplcm8tYnBzIGNvbmZpZ3VyYXRpb24gdHJhbnNmZXJzIG5vdGhpbmcgKHNoYXJlIGlzCnplcm8pIGFuZCBzZXR0bGVzIGluIGZ1bGwgdG8gdGhlIHNlbGxlciBieSByZXR1cm5pbmcgdGhlIGZ1bGwKYW1vdW50LiBUb2tlbiBmYWlsdXJlcyBhcmUgYnVja2V0ZWQgaW50bwpbYEZvcmdlRXJyb3I6OlRva2VuVHJhbnNmZXJGYWlsZWRgXSwgYW5kIGFueSByZXR1cm5lZCBlcnJvciByb2xscwp0aGUgd2hvbGUgaW52b2NhdGlvbiBiYWNrIOKAlCBhIGZhaWxlZCB0cmFuc2ZlciBuZXZlciBjb21taXRzIHRvdGFsAAAACmRpc3RyaWJ1dGUAAAAAAAUAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAFcGF5ZXIAAAAAAAATAAAAAAAAAAZzZWxsZXIAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAnRRdW90ZSB0aGUgZXhhY3Qgc3BsaXQgYSBzZXR0bGVtZW50IG9mIGBhbW91bnRgIGZvciBgY29sbGVjdGlvbmAKd291bGQgYXBwbHkgKHJlYWQtb25seSB2aWV3KS4KClNldHRsZS1wYXJpdHk6IHRoZSBxdW90ZSBydW5zIHRoZSBzYW1lIHZhbGlkYXRpb24gb3JkZXIgYW5kIHRoZSBzYW1lCnNwbGl0IGRlcml2YXRpb24gYXMgdGhlIHNldHRsZW1lbnQgZW50cnlwb2ludHMg4oCUIGNvbmZpZ3VyYXRpb24gbG9hZAooYE5vdEZvdW5kYCksIGBhbW91bnQgPiAwYCAoYEludmFsaWRJbnB1dGAsIG1pcnJvcmluZwpgZGlzdHJpYnV0ZWAvYHNldHRsZV9zYWxlYCksIHRoZW4gYGVmZmVjdGl2ZV9icHNgICsgYHNwbGl0YCDigJQKc28gdGhlIHF1b3RlIG5ldmVyIHN1Y2NlZWRzIHdoZXJlIGBzZXR0bGVfc2FsZWAgd291bGQgZmFpbCwgYW5kIHRoZQpyZXR1cm5lZCBudW1iZXJzIGFyZSB0aGUgc2V0dGxlbWVudCdzIG93biwgcm91bmRpbmcgaW5jbHVkZWQuIEEKYERpc2FibGVkYCBjb25maWd1cmF0aW9uIHF1b3RlcyBhdCB6ZXJvIGJwcywgbWF0Y2hpbmcgYHNldHRsZV9zYWxlYCdzCnNldHRsZS1pbi1mdWxsIGJlaGF2aW9yLiBObyBzdG9yYWdlIG11dGF0aW9uLCBubyBhdXRob3JpemF0aW9uLCBubwpldmVudHMuAAAACnF1b3RlX3NhbGUAAAAAAAIAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAH0AAAAAlTYWxlUXVvdGUAAAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAEhSZWFkIHRoZSBzdG9yZWQgcm95YWx0eSBjb25maWd1cmF0aW9uIGZvciBgY29sbGVjdGlvbmAgKHJlYWQtb25seSB2aWV3KS4AAAALZ2V0X3JveWFsdHkAAAAAAQAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAEAAAPpAAAH0AAAAAdSb3lhbHR5AAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAL5SZWdpc3RlciBvciB1cGRhdGUgYSByb3lhbHR5IGNvbmZpZ3VyYXRpb24gZm9yIGBjb2xsZWN0aW9uYC4KClJlcXVpcmVzIHRoZSBjb2xsZWN0aW9uJ3MgYXV0aG9yaXphdGlvbiBhbmQgYGJwcyA8PSAxMF8wMDBgCigxMDAlKS4gUmUtcmVnaXN0cmF0aW9uIHVwZGF0ZXMgdGhlIGV4aXN0aW5nIGNvbmZpZ3VyYXRpb24gaW4gcGxhY2UuAAAAAAALc2V0X3JveWFsdHkAAAAAAwAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAAAAAAJcmVjaXBpZW50AAAAAAAAEwAAAAAAAAADYnBzAAAAAAQAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAA1dTZXR0bGUgYSBzYWxlIG9mIGBjb2xsZWN0aW9uYCBhdG9taWNhbGx5IGluIGB0b2tlbmAuCgpSZXF1aXJlcyB0aGUgY29sbGVjdGlvbidzIGF1dGhvcml6YXRpb24gKGFzIGBkaXN0cmlidXRlYCBkb2VzKQphbmQgdGhlIGBwYXllcmAncywgd2hpY2ggY292ZXJzIGJvdGggbmVzdGVkIHRva2VuIHRyYW5zZmVycy4gUmVxdWlyZXMKYGFtb3VudCA+IDBgIGFuZCBhIHN0b3JlZCBjb25maWd1cmF0aW9uLiBUaGUgc3BsaXQgaXMgY29tcHV0ZWQgd2l0aApjaGVja2VkIGFyaXRobWV0aWMsIGFuZCBib3RoIHRyYW5zZmVycyBydW4gKipiZWZvcmUgYW55IHNldHRsZW1lbnQKc3RhdGUgaXMgY29tbWl0dGVkKio6IHRoZSBzZWxsZXIncyBuZXQgZmlyc3QsIHRoZSByb3lhbHR5IHJlY2lwaWVudApsYXN0LCBzbyBhIGZhaWxlZCB0cmFuc2ZlciBjYW4gbmV2ZXIgbGVhdmUgdGhlIHJveWFsdHkgcmVjaXBpZW50CnBhcnRpYWxseSBwYWlkLiBBIGBEaXNhYmxlZGAgb3IgemVyby1icHMgY29uZmlndXJhdGlvbiBzZXR0bGVzIHRoZQpmdWxsIGFtb3VudCB0byB0aGUgc2VsbGVyIGluIGEgc2luZ2xlIHRyYW5zZmVyLiBUb2tlbiBmYWlsdXJlcyBhcmUKYnVja2V0ZWQgaW50byBbYEZvcmdlRXJyb3I6OlRva2VuVHJhbnNmZXJGYWlsZWRgXSwgYW5kIGFueSByZXR1cm5lZAplcnJvciByb2xscyB0aGUgd2hvbGUgaW52b2NhdGlvbiBiYWNrIOKAlCBpbmNsdWRpbmcgYW4gZWFybGllcgpzdWNjZXNzZnVsIHRyYW5zZmVyIOKAlCBzbyByZXRyeWluZyBhZnRlciBhIGZhaWx1cmUgbmV2ZXIgZG91YmxlLXBheXMuClVzZSBgc2V0dGxlX3NhbGVzYCB0byBzZXR0bGUgYSBiYXRjaCBvZiBzYWxlcyBpbiBvbmUgaW52b2NhdGlvbi4AAAAAC3NldHRsZV9zYWxlAAAAAAUAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAFcGF5ZXIAAAAAAAATAAAAAAAAAAZzZWxsZXIAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAH0AAAAApTZXR0bGVtZW50AAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAABABTZXR0bGUgYSBiYXRjaCBvZiBzYWxlcyBvZiBgY29sbGVjdGlvbmAgYXRvbWljYWxseSBpbiBgdG9rZW5gLgoKU2FtZSB0cnVzdCBtb2RlbCBhcyBgc2V0dGxlX3NhbGVgOiB0aGUgY29sbGVjdGlvbidzIGF1dGhvcml6YXRpb24gYW5kCnRoZSBgcGF5ZXJgJ3MgYXJlIGVhY2ggcmVxdWlyZWQgb25jZSwgYW5kIHRoZSBwYXllcidzIHNpbmdsZQphdXRob3JpemF0aW9uIGNvdmVycyAqKmV2ZXJ5KiogbmVzdGVkIHRva2VuIHRyYW5zZmVyIGluIHRoZSBiYXRjaCDigJQKdGhlcmUgaXMgbm8gcGVyLXNhbGUgcmUtYXV0aG9yaXphdGlvbi4KClZhbGlkYXRpb24gcnVucyB0byBjb21wbGV0aW9uIGJlZm9yZSB0aGUgZmlyc3QgdHJhbnNmZXI6IHRoZQpjb25maWd1cmF0aW9uIG11c3QgZXhpc3QgKFtgRm9yZ2VFcnJvcjo6Tm90Rm91bmRgXSksIGBzYWxlc2AgbXVzdCBiZQpub24tZW1wdHkgYW5kIGF0IG1vc3QgW2BNQVhfU0VUVExFX1NBTEVTYF0gbG9uZwooW2BGb3JnZUVycm9yOjpJbnZhbGlkSW5wdXRgXSksIGV2ZXJ5IGBhbW91bnRgIG11c3QgYmUgcG9zaXRpdmUKKFtgRm9yZ2VFcnJvcjo6SW52YWxpZElucHV0YF0pLCBhbmQgZXZlcnkgc3BsaXQgcGx1cyB0aGUgYmF0Y2gKYWdncmVnYXRlIOKAlCBpbmNsdWRpbmcgdGhlIGNoZWNrZWQgYWRkIGFnYWluc3QgdGhlIHN0b3JlZCBzdW1tYXJ5IOKAlAptdXN0IGJlIG92ZXJmbG93LWZyZWUgKFtgRm9yZ2VFcnJvcjo6QXJpdGhtZXRpY092ZXJmbG93YF0pLiBPbmx5CnRoZW4gZG8gdHJhbnNmZXJzIHJ1biwgaW4gc2FsZSBvcmRlciwgZWFjaCBzYWxlIHNlbGxlci10aGVuLXJlY2lwaWVudAp3aXRoIHRoZSByZWNpcGllbnQgc2tpcHBlZCB3aGVuIGl0cyBzaGFyZSBpcyB6ZXJvLCBleGFjdGx5IGFzCmBzZXR0bGVfc2FsZWAgZG9lcy4gQSBmYWlsdXJlIGluIGFueSBzYWxlIHJvbGxzIHRoZSB3aG9sZSBpbnZvY2F0aW9uCmJhY2s6IG5vIHNhbGUgaXMgaGFsZi1zZXR0bGVkLCBubyBiYWxhbmNlIG1vdmVzLCBhbmQgdGhlIHN1bW1hcnkgaXMKdW50b3VjaGVkAAAADHNldHRsZV9zYWxlcwAAAAQAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAFcGF5ZXIAAAAAAAATAAAAAAAAAAVzYWxlcwAAAAAAA+oAAAPtAAAAAgAAABMAAAALAAAAAQAAA+kAAAPqAAAH0AAAAApTZXR0bGVtZW50AAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAEhSZXBsYWNlIGEgY29sbGVjdGlvbidzIHNwbGl0IGF0b21pY2FsbHkuIFplcm8tcmF0ZSByZWNpcGllbnRzIGFyZSB2YWxpZC4AAAASc2V0X3JveWFsdHlfc3BsaXRzAAAAAAACAAAAAAAAAApjb2xsZWN0aW9uAAAAAAATAAAAAAAAAApyZWNpcGllbnRzAAAAAAPqAAAH0AAAAAxSb3lhbHR5U2hhcmUAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAIBSZWFkIHRoZSBjdW11bGF0aXZlIHNldHRsZW1lbnQgdG90YWxzIGZvciBgY29sbGVjdGlvbmAgKHJlYWQtb25seQp2aWV3KS4gYE5vdEZvdW5kYCB1bnRpbCB0aGUgY29sbGVjdGlvbiBzZXR0bGVzIGl0cyBmaXJzdCBzYWxlLgAAABZnZXRfc2V0dGxlbWVudF9zdW1tYXJ5AAAAAAABAAAAAAAAAApjb2xsZWN0aW9uAAAAAAATAAAAAQAAA+kAAAfQAAAAEVNldHRsZW1lbnRTdW1tYXJ5AAAAAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAABQAAAAAAAAAAAAAAC1NhbGVTZXR0bGVkAAAAAAEAAAAMc2FsZV9zZXR0bGVkAAAACAAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAEAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAAAAAABXBheWVyAAAAAAAAEwAAAAAAAAAAAAAABnNlbGxlcgAAAAAAEwAAAAAAAAAAAAAAEXJveWFsdHlfcmVjaXBpZW50AAAAAAAAEwAAAAAAAAAAAAAADGdyb3NzX2Ftb3VudAAAAAsAAAAAAAAAAAAAAApzZWxsZXJfbmV0AAAAAAALAAAAAAAAAAAAAAANcm95YWx0eV9zaGFyZQAAAAAAAAsAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAEVJveWFsdHlDb25maWd1cmVkAAAAAAAAAQAAABJyb3lhbHR5X2NvbmZpZ3VyZWQAAAAAAAMAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAABAAAAAAAAAAlyZWNpcGllbnQAAAAAAAATAAAAAAAAAAAAAAADYnBzAAAAAAQAAAAAAAAAAg==",
        "AAAABAAAAZxTaGFyZWQgZXJyb3IgdHlwZSB1c2VkIGFjcm9zcyBhbGwgU29yb2JhbiBGb3JnZSBjb250cmFjdHMuCgpEZWZpbmluZyBhIHNpbmdsZSBlcnJvciBlbnVtIGluIGBzaGFyZWQtdXRpbHNgIGtlZXBzIHRoZSBvbi1jaGFpbiBlcnJvcgpzcGFjZSBjb25zaXN0ZW50IGFuZCBpbnRlbGxpZ2libGUgdG8gU0RLIGNvbnN1bWVycywgYW5kIGF2b2lkcyBldmVyeQpjb250cmFjdCByZS1kZWNsYXJpbmcgdGhlIHNhbWUgZmFpbHVyZSBtb2Rlcy4gQ29udHJhY3QgY3JhdGVzIG1heSBleHBvc2UKdGhlaXIgb3duIGRvbWFpbi1zcGVjaWZpYyBlcnJvcnMsIGJ1dCBzaG91bGQgcHJlZmVyIHRoZXNlIHdoZXJlIHRoZXkgZml0LgoKRXJyb3IgY29kZXMgc3RhcnQgYXQgMTsgY29kZSAwIGlzIHJlc2VydmVkIGJ5IHRoZSBTb3JvYmFuIGhvc3QuAAAAAAAAAApGb3JnZUVycm9yAAAAAAAPAAAAM1RoZSBjYWxsZXIgaXMgbm90IHBlcm1pdHRlZCB0byBwZXJmb3JtIHRoaXMgYWN0aW9uLgAAAAAMVW5hdXRob3JpemVkAAAAAQAAAEpUaGUgcmVxdWVzdGVkIGVudGl0eSAoZXNjcm93LCBwcm9wb3NhbCwgc3Vic2NyaXB0aW9uLCAuLi4pIGRvZXMgbm90IGV4aXN0LgAAAAAACE5vdEZvdW5kAAAAAgAAAEhPbmUgb3IgbW9yZSBhcmd1bWVudHMgZmFpbGVkIHZhbGlkYXRpb24gKGUuZy4gemVybyBhbW91bnQsIGJhZCBhZGRyZXNzKS4AAAAMSW52YWxpZElucHV0AAAAAwAAAENUaGUgY29udHJhY3QgZG9lcyBub3QgaG9sZCBlbm91Z2ggYmFsYW5jZSB0byBzYXRpc2Z5IHRoZSBvcGVyYXRpb24uAAAAABFJbnN1ZmZpY2llbnRGdW5kcwAAAAAAAAQAAABCVGhlIGVudGl0eSB3YXMgYWxyZWFkeSBpbml0aWFsaXNlZDsgcmUtaW5pdGlhbGlzYXRpb24gaXMgcmVqZWN0ZWQuAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAAFAAAANlRoZSBlbnRpdHkgd2FzIGV4cGVjdGVkIHRvIGJlIGluaXRpYWxpc2VkIGJ1dCB3YXMgbm90LgAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAGAAAANkFuIG9wZXJhdGlvbiB3YXMgYXR0ZW1wdGVkIGFmdGVyIGl0cyBkZWFkbGluZSBlbGFwc2VkLgAAAAAAD0RlYWRsaW5lUmVhY2hlZAAAAAAHAAAAQUEgcmVxdWlyZWQgdG9rZW4gYWxsb3dhbmNlIHdhcyBsb3dlciB0aGFuIHRoZSBhbW91bnQgYmVpbmcgc3BlbnQuAAAAAAAAFUluc3VmZmljaWVudEFsbG93YW5jZQAAAAAAAAgAAAAjQW4gYXJpdGhtZXRpYyBvcGVyYXRpb24gb3ZlcmZsb3dlZC4AAAAAEkFyaXRobWV0aWNPdmVyZmxvdwAAAAAACQAAAEJDb250cmFjdC1zcGVjaWZpYyBlcnJvciB0aGF0IGRvZXMgbm90IG1hcCB0byB0aGUgY2F0ZWdvcmllcyBhYm92ZS4AAAAAAAZDdXN0b20AAAAAAAoAAAFsQSBTRVAtNDEgdG9rZW4gaW52b2NhdGlvbiBmYWlsZWQgKGluc3VmZmljaWVudCBiYWxhbmNlLCBtaXNzaW5nIG9yCmRlYXV0aG9yaXplZCB0cnVzdGxpbmUsIHVuZGVwbG95ZWQgdG9rZW4gY29udHJhY3QsIG9yIHRva2VuLWxvZ2ljCnJlamVjdGlvbikuIFRoZSByYXcgdG9rZW4gZXJyb3IgZGlzY3JpbWluYW50IGlzIGludGVudGlvbmFsbHkgbm90CmZvcndhcmRlZCDigJQgY2FsbGVycyBjYW5ub3QgdGVsbCB3aGljaCBjb250cmFjdCBwcm9kdWNlZCBhIGZvcndhcmRlZApjb2RlLCBzbyBpdCBpcyBidWNrZXRlZDsgdGhlIHJvb3QgY2F1c2UgcmVtYWlucyB2aXNpYmxlIGluIHRoZQp0cmFuc2FjdGlvbidzIGRpYWdub3N0aWMgZXZlbnRzLgAAABNUb2tlblRyYW5zZmVyRmFpbGVkAAAAAAsAAADbQSBjcm9zcy1jb250cmFjdCBpbnZvY2F0aW9uIGZhaWxlZCAodGFyZ2V0IHJldmVydGVkIG9yIGhvc3QKYWJvcnQpLiBUaGUgdGFyZ2V0IHJldmVydHMgYXJlIHN1cmZhY2VkIGhlcmUgc28gdGhlIGNhbGxlciBjYW4KZGlzdGluZ3Vpc2ggdGhlbSBmcm9tIHRva2VuLXRyYW5zZmVyIGZhaWx1cmVzLCBhbmQgdGhlIGludm9raW5nCnRyYW5zYWN0aW9uIGlzIGxlZnQgdW4tZXhlY3V0ZWQuAAAAABhDb250cmFjdEludm9jYXRpb25GYWlsZWQAAAAMAAABDFRoZSByZXF1ZXN0ZWQgd2l0aGRyYXdhbCB3b3VsZCBwdXNoIGEgdG9rZW4ncyByb2xsaW5nLXdpbmRvdyB0b3RhbApwYXN0IGl0cyBjb25maWd1cmVkIHdpdGhkcmF3YWwgbGltaXQuIEtlcHQgZGlzdGluY3QgZnJvbQpbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIHNvIGEgY2FsbGVyIGNhbiB0ZWxsIGEgcG9saWN5IHJlamVjdGlvbgooYSB2YWxpZCB3aXRoZHJhd2FsIHRoYXQgaXMgdG9vIGxhcmdlIHJpZ2h0IG5vdykgZnJvbSBhIG1hbGZvcm1lZAphcmd1bWVudC4AAAAXV2l0aGRyYXdhbExpbWl0RXhjZWVkZWQAAAAADQAAARtBIHN1YnNjcmlwdGlvbiBpcyBpbiB0aGUgYFBhc3REdWVgIGxhcHNlZCBzdGF0ZSBhbmQgdGhlIHJlcXVlc3RlZApvcGVyYXRpb24gY2Fubm90IGJlIHBlcmZvcm1lZCB1bnRpbCBhIGNhdGNoLXVwIGNoYXJnZSByZXN0b3JlcyBpdCB0bwpgQWN0aXZlYC4gS2VwdCBkaXN0aW5jdCBmcm9tIFtgRm9yZ2VFcnJvcjo6RGVhZGxpbmVSZWFjaGVkYF0gc28gYSBjYWxsZXIKY2FuIHRlbGwgYSBsYXBzZWQgYmlsbGluZyBzdGF0ZSBmcm9tIGFuIGFyZ3VtZW50IHRoYXQgd2FzIG1lcmVseQp0b28gZWFybHkuAAAAABNTdWJzY3JpcHRpb25QYXN0RHVlAAAAAA4AAABxQSBwcm9wb3NlciBoYXMgcmVhY2hlZCB0aGUgbWF4aW11bSBhbGxvd2VkIGNvbmN1cnJlbnQgYWN0aXZlIHByb3Bvc2FscwpvciBpcyB3aXRoaW4gdGhlIHByb3Bvc2VyIGNvb2xkb3duIHdpbmRvdy4AAAAAAAAQUHJvcG9zZXJDb29sZG93bgAAAA8=" ]),
      options
    )
  }
  public readonly fromJSON = {
    touch_ttl: this.txFromJSON<Result<void>>,
        distribute: this.txFromJSON<Result<i128>>,
        quote_sale: this.txFromJSON<Result<SaleQuote>>,
        get_royalty: this.txFromJSON<Result<Royalty>>,
        set_royalty: this.txFromJSON<Result<void>>,
        settle_sale: this.txFromJSON<Result<Settlement>>,
        settle_sales: this.txFromJSON<Result<Array<Settlement>>>,
        set_royalty_splits: this.txFromJSON<Result<void>>,
        get_settlement_summary: this.txFromJSON<Result<SettlementSummary>>
  }
}