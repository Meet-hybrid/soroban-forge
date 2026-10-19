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
 * One recipient and its basis-point rate in a collection split.
 */
export interface RoyaltyShare {
  bps: u32;
  recipient: string;
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
 * The two amounts one atomic settlement transferred: one `settle_sale`
 * invocation, or one sale of a `settle_sales` batch. Shared by both
 * entrypoints so generated clients can reuse the type.
 */
export interface Settlement {
  /**
   * Amount transferred to the configured royalty recipient.
   */
  royalty_share: i128;
  /**
   * Amount transferred to the seller.
   */
  seller_net: i128;
}

/**
 * Lifecycle state of a registered royalty configuration.
 */
export type RoyaltyStatus =
  | { tag: "Active"; values: void }
  | { tag: "Disabled"; values: void };

/**
 * Cumulative settlement totals for one collection.
 */
export interface SettlementSummary {
  /**
   * Sum of every settled sale amount.
   */
  gross_volume: i128;
  /**
   * Sum of every royalty share transferred to the recipient.
   */
  royalties_paid: i128;
  /**
   * Number of sales settled so far.
   */
  sales: u32;
}

/**
 * The split one sale of a collection would apply at settlement time, as
 * returned by `quote_sale`.
 */
export interface SaleQuote {
  /**
   * The sale amount the quote was computed for.
   */
  gross: i128;
  /**
   * Royalty share: `gross * royalty_bps / 10_000`, floored.
   */
  royalty_amount: i128;
  /**
   * Effective royalty rate applied, in basis points.
   */
  royalty_bps: u32;
  /**
   * Seller's net: `gross - royalty_amount`.
   */
  seller_net: i128;
}

/**
 * A participant in a multi-party flow (escrow, governance, ...).
 */
export interface Party {
  address: string;
  approved: boolean;
  role: string;
}

/**
 * Inclusive time window expressed as Unix timestamps (seconds).
 */
export interface TimeBounds {
  end: u64;
  start: u64;
}

/**
 * A page of results plus the cursor needed to fetch the next page.
 */
export interface PaginatedResult {
  cursor: PaginationCursor;
  items: Array<Buffer>;
  total: u32;
}

/**
 * Offset/limit pair for paginated reads.
 */
export interface PaginationCursor {
  limit: u32;
  offset: u32;
}

/**
 * Shared error type used across all Soroban Forge contracts.
 *
 * Error codes start at 1; code 0 is reserved by the Soroban host.
 */
export const ForgeError = {
  /**
   * The caller is not permitted to perform this action.
   */
  1: { message: "Unauthorized" },
  /**
   * The requested entity (escrow, proposal, subscription, ...) does not exist.
   */
  2: { message: "NotFound" },
  /**
   * One or more arguments failed validation (e.g. zero amount, bad address).
   */
  3: { message: "InvalidInput" },
  /**
   * The contract does not hold enough balance to satisfy the operation.
   */
  4: { message: "InsufficientFunds" },
  /**
   * The entity was already initialised; re-initialisation is rejected.
   */
  5: { message: "AlreadyInitialized" },
  /**
   * The entity was expected to be initialised but was not.
   */
  6: { message: "NotInitialized" },
  /**
   * An operation was attempted after its deadline elapsed.
   */
  7: { message: "DeadlineReached" },
  /**
   * A required token allowance was lower than the amount being spent.
   */
  8: { message: "InsufficientAllowance" },
  /**
   * An arithmetic operation overflowed.
   */
  9: { message: "ArithmeticOverflow" },
  /**
   * Contract-specific error that does not map to the categories above.
   */
  10: { message: "Custom" },
  /**
   * A SEP-41 token invocation failed.
   */
  11: { message: "TokenTransferFailed" },
  /**
   * A cross-contract invocation failed.
   */
  12: { message: "ContractInvocationFailed" },
  /**
   * The requested withdrawal would push a token's rolling-window total
   * past its configured withdrawal limit.
   */
  13: { message: "WithdrawalLimitExceeded" },
  /**
   * A subscription is in the `PastDue` lapsed state.
   */
  14: { message: "SubscriptionPastDue" },
  /**
   * A proposer has reached the maximum allowed concurrent active proposals
   * or is within the proposer cooldown window.
   */
  15: { message: "ProposerCooldown" },
};

/**
 * Audit metadata attached to a persisted value.
 */
export interface StorageEntry {
  updated_at: u64;
  value: Buffer;
}

export interface Client {
  /**
   * Permissionless keeper: bump the royalty and summary entries' TTL without changing
   * any state.
   */
  touch_ttl: (
    { collection }: { collection: string },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Distribute the royalty share of `amount` from a sale of `collection` in `token`.
   */
  distribute: (
    {
      collection,
      token,
      payer,
      seller,
      amount,
    }: {
      collection: string;
      token: string;
      payer: string;
      seller: string;
      amount: i128;
    },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<i128>>>;

  /**
   * Read the stored royalty configuration for `collection` (read-only view).
   */
  get_royalty: (
    { collection }: { collection: string },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<Royalty>>>;

  /**
   * Register or update a royalty configuration for `collection`.
   */
  set_royalty: (
    {
      collection,
      recipient,
      bps,
    }: { collection: string; recipient: string; bps: u32 },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Replace a collection's split atomically. Zero-rate recipients are valid.
   */
  set_royalty_splits: (
    {
      collection,
      recipients,
    }: { collection: string; recipients: Array<RoyaltyShare> },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Settle a sale of `collection` atomically in `token`.
   */
  settle_sale: (
    {
      collection,
      token,
      payer,
      seller,
      amount,
    }: {
      collection: string;
      token: string;
      payer: string;
      seller: string;
      amount: i128;
    },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<Settlement>>>;

  /**
   * Settle a batch of sales of `collection` atomically in `token`.
   */
  settle_sales: (
    {
      collection,
      token,
      payer,
      sales,
    }: {
      collection: string;
      token: string;
      payer: string;
      sales: Array<readonly [string, i128]>;
    },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<Array<Settlement>>>>;

  /**
   * Read the cumulative settlement totals for `collection` (read-only view).
   */
  get_settlement_summary: (
    { collection }: { collection: string },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<SettlementSummary>>>;

  /**
   * Quote the exact split a settlement of `amount` for `collection` would apply (read-only view).
   */
  quote_sale: (
    { collection, amount }: { collection: string; amount: i128 },
    options?: MethodOptions
  ) => Promise<AssembledTransaction<Result<SaleQuote>>>;
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
    return ContractClient.deploy(null, options);
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([
        "AAAAAQAAAD1PbmUgcmVjaXBpZW50IGFuZCBpdHMgYmFzaXMtcG9pbnQgcmF0ZSBpbiBhIGNvbGxlY3Rpb24gc3BsaXQuAAAAAAAAAAAAAAxSb3lhbHR5U2hhcmUAAAACAAAAAAAAAANicHMAAAAABAAAAAAAAAAJcmVjaXBpZW50AAAAAAAAEw==",
        "AAAAAQAAADBBIHJveWFsdHkgY29uZmlndXJhdGlvbiBmb3IgYSBzaW5nbGUgY29sbGVjdGlvbi4AAAAAAAAAB1JveWFsdHkAAAAABAAAACxSb3lhbHR5IHJhdGUgaW4gYmFzaXMgcG9pbnRzICgxMDAgYnBzID0gMSUpLgAAAANicHMAAAAABAAAADhDb2xsZWN0aW9uIChORlQgY29udHJhY3QpIHRoaXMgY29uZmlndXJhdGlvbiBhcHBsaWVzIHRvLgAAAApjb2xsZWN0aW9uAAAAAAATAAAAJUFkZHJlc3MgZW50aXRsZWQgdG8gcm95YWx0eSBwYXltZW50cy4AAAAAAAAJcmVjaXBpZW50AAAAAAAAEwAAADBXaGV0aGVyIHRoZSBjb25maWd1cmF0aW9uIGlzIGN1cnJlbnRseSBlbmZvcmNlZC4AAAAGc3RhdHVzAAAAAAfQAAAADVJveWFsdHlTdGF0dXMAAAA=",
        "AAAAAQAAALtUaGUgdHdvIGFtb3VudHMgb25lIGF0b21pYyBzZXR0bGVtZW50IHRyYW5zZmVycmVkOiBvbmUgYHNldHRsZV9zYWxlYAppbnZvY2F0aW9uLCBvciBvbmUgc2FsZSBvZiBhIGBzZXR0bGVfc2FsZXNgIGJhdGNoLiBTaGFyZWQgYnkgYm90aAplbnRyeXBvaW50cyBzbyBnZW5lcmF0ZWQgY2xpZW50cyBjYW4gcmV1c2UgdGhlIHR5cGUuAAAAAAAAAAAKU2V0dGxlbWVudAAAAAAAAgAAADdBbW91bnQgdHJhbnNmZXJyZWQgdG8gdGhlIGNvbmZpZ3VyZWQgcm95YWx0eSByZWNpcGllbnQuAAAAAA1yb3lhbHR5X3NoYXJlAAAAAAAACwAAACFBbW91bnQgdHJhbnNmZXJyZWQgdG8gdGhlIHNlbGxlci4AAAAAAAAKc2VsbGVyX25ldAAAAAAACw==",
        "AAAAAgAAADZMaWZlY3ljbGUgc3RhdGUgb2YgYSByZWdpc3RlcmVkIHJveWFsdHkgY29uZmlndXJhdGlvbi4AAAAAAAAAAAANUm95YWx0eVN0YXR1cwAAAAAAAAIAAAAAAAAAHEFjdGl2ZSBhbmQgYXBwbGllZCB0byBzYWxlcy4AAAAGQWN0aXZlAAAAAAAAAAAALURpc2FibGVkOyBzYWxlcyBzZXR0bGUgdG8gdGhlIHNlbGxlciBpbiBmdWxsLgAAAAAAAAhEaXNhYmxlZA==",
        "AAAAAQAAADBDdW11bGF0aXZlIHNldHRsZW1lbnQgdG90YWxzIGZvciBvbmUgY29sbGVjdGlvbi4AAAAAAAAAEVNldHRsZW1lbnRTdW1tYXJ5AAAAAAAAAwAAACFTdW0gb2YgZXZlcnkgc2V0dGxlZCBzYWxlIGFtb3VudC4AAAAAAAAMZ3Jvc3Nfdm9sdW1lAAAACwAAADhTdW0gb2YgZXZlcnkgcm95YWx0eSBzaGFyZSB0cmFuc2ZlcnJlZCB0byB0aGUgcmVjaXBpZW50LgAAAA5yb3lhbHRpZXNfcGFpZAAAAAAACwAAAB9OdW1iZXIgb2Ygc2FsZXMgc2V0dGxlZCBzbyBmYXIuAAAAAAVzYWxlcwAAAAAAAAQ=",
        "AAAAAQAAAEJUaGUgc3BsaXQgb25lIHNhbGUgb2YgYSBjb2xsZWN0aW9uIHdvdWxkIGFwcGx5IGF0IHNldHRsZW1lbnQgdGltZS4AAAAAAAAAAAAJU2FsZVF1b3RlAAAAAAAABAAAAAAAAAAFZ3Jvc3MAAAAAAAALAAAAAAAAAA5yb3lhbHR5X2Ftb3VudAAAAAAACwAAAAAAAAALcm95YWx0eV9icHMAAAAABAAAAAAAAAAKc2VsbGVyX25ldAAAAAAACw==",
        "AAAAAAAAALFQZXJtaXNzaW9ubGVzcyBrZWVwZXI6IGJ1bXAgdGhlIHJveWFsdHkgYW5kIHN1bW1hcnkgZW50cmllcycgVFRMIHdpdGhvdXQgY2hhbmdpbmcKYW55IHN0YXRlLgoKUmV0dXJucyBgRm9yZ2VFcnJvcjo6Tm90Rm91bmRgIGlmIG5vIHJveWFsdHkgY29uZmlndXJhdGlvbiBleGlzdHMgZm9yIGBjb2xsZWN0aW9uYC4AAAAAAAAJdG91Y2hfdHRsAAAAAAAAAQAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAABABEaXN0cmlidXRlIHRoZSByb3lhbHR5IHNoYXJlIG9mIGBhbW91bnRgIGZyb20gYSBzYWxlIG9mIGBjb2xsZWN0aW9uYAppbiBgdG9rZW5gLgoKUmVxdWlyZXMgdGhlIGNvbGxlY3Rpb24ncyBhdXRob3JpemF0aW9uICh0aGUgcGF5ZXIncyBhdXRob3JpemF0aW9uCmNvdmVycyB0aGUgbmVzdGVkIHRva2VuIHRyYW5zZmVyLCBleGFjdGx5IGFzIGBzZXR0bGVfc2FsZWApIGFuZApgYW1vdW50ID4gMGAuIENvbXB1dGVzIHRoZSBzcGxpdCB3aXRoCltgc3BsaXRgXSDigJQgdGhlIHNhbWUgbWF0aCBhcyBgc2V0dGxlX3NhbGVgIOKAlCB0aGVuIHRyYW5zZmVycyBvbmx5IHRoZQpyb3lhbHR5IHNoYXJlIGZyb20gYHBheWVyYCB0byB0aGUgY29uZmlndXJlZCByZWNpcGllbnQgKipiZWZvcmUgYW55CnNldHRsZW1lbnQgc3RhdGUgaXMgY29tbWl0dGVkKiouIFRoZSBgc2VsbGVyYCdzIG5ldCBpcyAqbm90Kgp0cmFuc2ZlcnJlZCBoZXJlOiB0aGlzIGlzIGEgc3RhbmRhbG9uZSByb3lhbHR5IHNldHRsZW1lbnQgZm9yIGNhc2VzCndoZXJlIHRoZSB1bmRlcmx5aW5nIHNhbGUvcGF5bWVudCBpcyBoYW5kbGVkIG91dHNpZGUgYHNldHRsZV9zYWxlYCwKc28gdGhlIGNhbGxlciBpcyByZXNwb25zaWJsZSBmb3IgcGF5aW5nIHRoZSBzZWxsZXIgc2VwYXJhdGVseS4gVGhlCm5ldCBpcyBzdGlsbCByZXR1cm5lZCBzbyB0aGUgY2FsbGVyIGtub3dzIHdoYXQgdGhlIHNlbGxlciBpcyBvd2VkLgpBIGBEaXNhYmxlZGAgb3IgemVyby1icHMgY29uZmlndXJhdGlvbiB0cmFuc2ZlcnMgbm90aGluZyAoc2hhcmUgaXMKemVybykgYW5kIHNldHRsZXMgaW4gZnVsbCB0byB0aGUgc2VsbGVyIGJ5IHJldHVybmluZyB0aGUgZnVsbAphbW91bnQuIFRva2VuIGZhaWx1cmVzIGFyZSBidWNrZXRlZCBpbnRvCltgRm9yZ2VFcnJvcjo6VG9rZW5UcmFuc2ZlckZhaWxlZGBdLCBhbmQgYW55IHJldHVybmVkIGVycm9yIHJvbGxzCnRoZSB3aG9sZSBpbnZvY2F0aW9uIGJhY2sg4oCUIGEgZmFpbGVkIHRyYW5zZmVyIG5ldmVyIGNvbW1pdHMgdG90AAAACmRpc3RyaWJ1dGUAAAAAAAUAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAFcGF5ZXIAAAAAAAATAAAAAAAAAAZzZWxsZXIAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAEhSZWFkIHRoZSBzdG9yZWQgcm95YWx0eSBjb25maWd1cmF0aW9uIGZvciBgY29sbGVjdGlvbmAgKHJlYWQtb25seSB2aWV3KS4AAAALZ2V0X3JveWFsdHkAAAAAAQAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAEAAAPpAAAH0AAAAAdSb3lhbHR5AAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAL5SZWdpc3RlciBvciB1cGRhdGUgYSByb3lhbHR5IGNvbmZpZ3VyYXRpb24gZm9yIGBjb2xsZWN0aW9uYC4KClJlcXVpcmVzIHRoZSBjb2xsZWN0aW9uJ3MgYXV0aG9yaXphdGlvbiBhbmQgYGJwcyA8PSAxMF8wMDBgCigxMDAlKS4gUmUtcmVnaXN0cmF0aW9uIHVwZGF0ZXMgdGhlIGV4aXN0aW5nIGNvbmZpZ3VyYXRpb24gaW4gcGxhY2UuAAAAAAALc2V0X3JveWFsdHkAAAAAAwAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAAAAAAJcmVjaXBpZW50AAAAAAAAEwAAAAAAAAADYnBzAAAAAAQAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAACZSZXBsYWNlIGEgY29sbGVjdGlvbiBzcGxpdCBhdG9taWNhbGx5LgAAAAAAEnNldF9yb3lhbHR5X3NwbGl0cwAAAAAAAgAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAAAAAAKcmVjaXBpZW50cwAAAAAD6gAAB9AAAAAMUm95YWx0eVNoYXJlAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAA1dTZXR0bGUgYSBzYWxlIG9mIGBjb2xsZWN0aW9uYCBhdG9taWNhbGx5IGluIGB0b2tlbmAuCgpSZXF1aXJlcyB0aGUgY29sbGVjdGlvbidzIGF1dGhvcml6YXRpb24gKGFzIGBkaXN0cmlidXRlYCBkb2VzKQphbmQgdGhlIGBwYXllcmAncywgd2hpY2ggY292ZXJzIGJvdGggbmVzdGVkIHRva2VuIHRyYW5zZmVycy4gUmVxdWlyZXMKYGFtb3VudCA+IDBgIGFuZCBhIHN0b3JlZCBjb25maWd1cmF0aW9uLiBUaGUgc3BsaXQgaXMgY29tcHV0ZWQgd2l0aApjaGVja2VkIGFyaXRobWV0aWMsIGFuZCBib3RoIHRyYW5zZmVycyBydW4gKipiZWZvcmUgYW55IHNldHRsZW1lbnQKc3RhdGUgaXMgY29tbWl0dGVkKio6IHRoZSBzZWxsZXIncyBuZXQgZmlyc3QsIHRoZSByb3lhbHR5IHJlY2lwaWVudApsYXN0LCBzbyBhIGZhaWxlZCB0cmFuc2ZlciBjYW4gbmV2ZXIgbGVhdmUgdGhlIHJveWFsdHkgcmVjaXBpZW50CnBhcnRpYWxseSBwYWlkLiBBIGBEaXNhYmxlZGAgb3IgemVyby1icHMgY29uZmlndXJhdGlvbiBzZXR0bGVzIHRoZQpmdWxsIGFtb3VudCB0byB0aGUgc2VsbGVyIGluIGEgc2luZ2xlIHRyYW5zZmVyLiBUb2tlbiBmYWlsdXJlcyBhcmUKYnVja2V0ZWQgaW50byBbYEZvcmdlRXJyb3I6OlRva2VuVHJhbnNmZXJGYWlsZWRgXSwgYW5kIGFueSByZXR1cm5lZAplcnJvciByb2xscyB0aGUgd2hvbGUgaW52b2NhdGlvbiBiYWNrIOKAlCBpbmNsdWRpbmcgYW4gZWFybGllcgpzdWNjZXNzZnVsIHRyYW5zZmVyIOKAlCBzbyByZXRyeWluZyBhZnRlciBhIGZhaWx1cmUgbmV2ZXIgZG91YmxlLXBheXMuClVzZSBgc2V0dGxlX3NhbGVzYCB0byBzZXR0bGUgYSBiYXRjaCBvZiBzYWxlcyBpbiBvbmUgaW52b2NhdGlvbi4AAAAAC3NldHRsZV9zYWxlAAAAAAUAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAFcGF5ZXIAAAAAAAATAAAAAAAAAAZzZWxsZXIAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAH0AAAAApTZXR0bGVtZW50AAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAABABTZXR0bGUgYSBiYXRjaCBvZiBzYWxlcyBvZiBgY29sbGVjdGlvbmAgYXRvbWljYWxseSBpbiBgdG9rZW5gLgoKU2FtZSB0cnVzdCBtb2RlbCBhcyBgc2V0dGxlX3NhbGVgOiB0aGUgY29sbGVjdGlvbidzIGF1dGhvcml6YXRpb24gYW5kCnRoZSBgcGF5ZXJgJ3MgYXJlIGVhY2ggcmVxdWlyZWQgb25jZSwgYW5kIHRoZSBwYXllcidzIHNpbmdsZQphdXRob3JpemF0aW9uIGNvdmVycyAqKmV2ZXJ5KiogbmVzdGVkIHRva2VuIHRyYW5zZmVyIGluIHRoZSBiYXRjaCDigJQKdGhlcmUgaXMgbm8gcGVyLXNhbGUgcmUtYXV0aG9yaXphdGlvbi4KClZhbGlkYXRpb24gcnVucyB0byBjb21wbGV0aW9uIGJlZm9yZSB0aGUgZmlyc3QgdHJhbnNmZXI6IHRoZQpjb25maWd1cmF0aW9uIG11c3QgZXhpc3QgKFtgRm9yZ2VFcnJvcjo6Tm90Rm91bmRgXSksIGBzYWxlc2AgbXVzdCBiZQpub24tZW1wdHkgYW5kIGF0IG1vc3QgW2BNQVhfU0VUVExFX1NBTEVTYF0gbG9uZwooW2BGb3JnZUVycm9yOjpJbnZhbGlkSW5wdXRgXSksIGV2ZXJ5IGBhbW91bnRgIG11c3QgYmUgcG9zaXRpdmUKKFtgRm9yZ2VFcnJvcjo6SW52YWxpZElucHV0YF0pLCBhbmQgZXZlcnkgc3BsaXQgcGx1cyB0aGUgYmF0Y2gKYWdncmVnYXRlIOKAlCBpbmNsdWRpbmcgdGhlIGNoZWNrZWQgYWRkIGFnYWluc3QgdGhlIHN0b3JlZCBzdW1tYXJ5IOKAlAptdXN0IGJlIG92ZXJmbG93LWZyZWUgKFtgRm9yZ2VFcnJvcjo6QXJpdGhtZXRpY092ZXJmbG93YF0pLiBPbmx5CnRoZW4gZG8gdHJhbnNmZXJzIHJ1biwgaW4gc2FsZSBvcmRlciwgZWFjaCBzYWxlIHNlbGxlci10aGVuLXJlY2lwaWVudAp3aXRoIHRoZSByZWNpcGllbnQgc2tpcHBlZCB3aGVuIGl0cyBzaGFyZSBpcyB6ZXJvLCBleGFjdGx5IGFzCmBzZXR0bGVfc2FsZWAgZG9lcy4gQSBmYWlsdXJlIGluIGFueSBzYWxlIHJvbGxzIHRoZSB3aG9sZSBpbnZvY2F0aW9uCmJhY2s6IG5vIHNhbGUgaXMgaGFsZi1zZXR0bGVkLCBubyBiYWxhbmNlIG1vdmVzLCBhbmQgdGhlIHN1bW1hcnkgaXMKdW50b3VjaGVkAAAADHNldHRsZV9zYWxlcwAAAAQAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAFcGF5ZXIAAAAAAAATAAAAAAAAAAVzYWxlcwAAAAAAA+oAAAPtAAAAAgAAABMAAAALAAAAAQAAA+kAAAPqAAAH0AAAAApTZXR0bGVtZW50AAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAIBSZWFkIHRoZSBjdW11bGF0aXZlIHNldHRsZW1lbnQgdG90YWxzIGZvciBgY29sbGVjdGlvbmAgKHJlYWQtb25seQp2aWV3KS4gYE5vdEZvdW5kYCB1bnRpbCB0aGUgY29sbGVjdGlvbiBzZXR0bGVzIGl0cyBmaXJzdCBzYWxlLgAAABZnZXRfc2V0dGxlbWVudF9zdW1tYXJ5AAAAAAABAAAAAAAAAApjb2xsZWN0aW9uAAAAAAATAAAAAQAAA+kAAAfQAAAAEVNldHRsZW1lbnRTdW1tYXJ5AAAAAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAEhRdW90ZSB0aGUgZXhhY3Qgc3BsaXQgYSBzZXR0bGVtZW50IG9mIGFtb3VudCBmb3IgY29sbGVjdGlvbiB3b3VsZCBhcHBseS4AAAAKcXVvdGVfc2FsZQAAAAAAAgAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAAAAAAGYW1vdW50AAAAAAALAAAAAQAAA+kAAAfQAAAACVNhbGVRdW90ZQAAAAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAABQAAAAAAAAAAAAAAC1NhbGVTZXR0bGVkAAAAAAEAAAAMc2FsZV9zZXR0bGVkAAAACAAAAAAAAAAKY29sbGVjdGlvbgAAAAAAEwAAAAEAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAAAAAABXBheWVyAAAAAAAAEwAAAAAAAAAAAAAABnNlbGxlcgAAAAAAEwAAAAAAAAAAAAAAEXJveWFsdHlfcmVjaXBpZW50AAAAAAAAEwAAAAAAAAAAAAAADGdyb3NzX2Ftb3VudAAAAAsAAAAAAAAAAAAAAApzZWxsZXJfbmV0AAAAAAALAAAAAAAAAAAAAAANcm95YWx0eV9zaGFyZQAAAAAAAAsAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAEVJveWFsdHlDb25maWd1cmVkAAAAAAAAAQAAABJyb3lhbHR5X2NvbmZpZ3VyZWQAAAAAAAMAAAAAAAAACmNvbGxlY3Rpb24AAAAAABMAAAABAAAAAAAAAAlyZWNpcGllbnQAAAAAAAATAAAAAAAAAAAAAAADYnBzAAAAAAQAAAAAAAAAAg==",
        "AAAAAQAAAD5BIHBhcnRpY2lwYW50IGluIGEgbXVsdGktcGFydHkgZmxvdyAoZXNjcm93LCBnb3Zlcm5hbmNlLCAuLi4pLgAAAAAAAAAAAAVQYXJ0eQAAAAAAAAMAAAAkT24tY2hhaW4gYWRkcmVzcyBvZiB0aGUgcGFydGljaXBhbnQuAAAAB2FkZHJlc3MAAAAAEwAAAD9XaGV0aGVyIHRoaXMgcGFydHkgaGFzIGdyYW50ZWQgYXBwcm92YWwgZm9yIHRoZSBjdXJyZW50IGFjdGlvbi4AAAAACGFwcHJvdmVkAAAAAQAAADlIdW1hbi1yZWFkYWJsZSByb2xlIGxhYmVsLCBlLmcuIGAiYnV5ZXIiYCBvciBgImFyYml0ZXIiYC4AAAAAAAAEcm9sZQAAABA=",
        "AAAAAQAAALtJbmNsdXNpdmUgdGltZSB3aW5kb3cgZXhwcmVzc2VkIGFzIFVuaXggdGltZXN0YW1wcyAoc2Vjb25kcykuCgpTdG9yZWQgYXMgcGxhaW4gYHU2NGAgYmVjYXVzZSBgc29yb2Jhbl9zZGtgIG1vZGVscyB0aW1lIGFzIGB1NjRgOyBhCmRlZGljYXRlZCBuZXd0eXBlIHdvdWxkIGFkZCBjb252ZXJzaW9ucyB3aXRob3V0IGJlbmVmaXQuAAAAAAAAAAAKVGltZUJvdW5kcwAAAAAAAgAAADhMYXRlc3QgbW9tZW50IChpbmNsdXNpdmUpIGF0IHdoaWNoIHRoZSB3aW5kb3cgaXMgYWN0aXZlLgAAAANlbmQAAAAABgAAADpFYXJsaWVzdCBtb21lbnQgKGluY2x1c2l2ZSkgYXQgd2hpY2ggdGhlIHdpbmRvdyBpcyBhY3RpdmUuAAAAAAAFc3RhcnQAAAAAAAAG",
        "AAAAAQAAAUBBIHBhZ2Ugb2YgcmVzdWx0cyBwbHVzIHRoZSBjdXJzb3IgbmVlZGVkIHRvIGZldGNoIHRoZSBuZXh0IHBhZ2UuCgpJdGVtcyBhcmUgc3RvcmVkIGFzIHNlcmlhbGl6ZWQgYEJ5dGVzYCBzbyB0aGUgaGVscGVyIGlzIGFnbm9zdGljIHRvIHRoZQpjb25jcmV0ZSB2YWx1ZSB0eXBlIGEgY29udHJhY3QgcGFnaW5hdGVzLiBDYWxsZXJzIGRlY29kZSBlYWNoIGl0ZW0gaW50bwp0aGVpciBkb21haW4gdHlwZS4gYERlYnVnYCBpcyBvbWl0dGVkIGJlY2F1c2UgdGhlIFNESyBjb2xsZWN0aW9uIGRvZXMgbm90CmltcGxlbWVudCBpdCBmb3IgdGhpcyBjb250cmFjdCB0eXBlLgAAAAAAAAAPUGFnaW5hdGVkUmVzdWx0AAAAAAMAAAA9Q3Vyc29yIGRlc2NyaWJpbmcgdGhlIG5leHQgcGFnZSAob2Zmc2V0IGFkdmFuY2VkIGJ5IGBsaW1pdGApLgAAAAAAAAZjdXJzb3IAAAAAB9AAAAAQUGFnaW5hdGlvbkN1cnNvcgAAABNJdGVtcyBvbiB0aGlzIHBhZ2UuAAAAAAVpdGVtcwAAAAAAA+oAAAAOAAAAJ1RvdGFsIG51bWJlciBvZiBpdGVtcyBhY3Jvc3MgYWxsIHBhZ2VzLgAAAAAFdG90YWwAAAAAAAAE",
        "AAAAAQAAACZPZmZzZXQvbGltaXQgcGFpciBmb3IgcGFnaW5hdGVkIHJlYWRzLgAAAAAAAAAAABBQYWdpbmF0aW9uQ3Vyc29yAAAAAgAAACJNYXhpbXVtIG51bWJlciBvZiBpdGVtcyB0byByZXR1cm4uAAAAAAAFbGltaXQAAAAAAAAEAAAAPk51bWJlciBvZiBpdGVtcyB0byBza2lwIGZyb20gdGhlIHN0YXJ0IG9mIHRoZSBmdWxsIHJlc3VsdCBzZXQuAAAAAAAGb2Zmc2V0AAAAAAAE",
        "AAAABAAAADpTaGFyZWQgZXJyb3IgdHlwZSB1c2VkIGFjcm9zcyBhbGwgU29yb2JhbiBGb3JnZSBjb250cmFjdHMuAAAAAAAAAAAACkZvcmdlRXJyb3IAAAAAAA8AAAAAAAAADFVuYXV0aG9yaXplZAAAAAEAAAAAAAAACE5vdEZvdW5kAAAAAgAAAAAAAAAMSW52YWxpZElucHV0AAAAAwAAAAAAAAARSW5zdWZmaWNpZW50RnVuZHMAAAAAAAAEAAAAAAAAABJBbHJlYWR5SW5pdGlhbGl6ZWQAAAAAAAUAAAAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAGAAAAAAAAAA9EZWFkbGluZVJlYWNoZWQAAAAABwAAAAAAAAAVSW5zdWZmaWNpZW50QWxsb3dhbmNlAAAAAAAACAAAAAAAAAASQXJpdGhtZXRpY092ZXJmbG93AAAAAAAJAAAAAAAAAAZDdXN0b20AAAAAAAoAAAAAAAAAE1Rva2VuVHJhbnNmZXJGYWlsZWQAAAAACwAAAAAAAAAYQ29udHJhY3RJbnZvY2F0aW9uRmFpbGVkAAAADAAAAAAAAAAXV2l0aGRyYXdhbExpbWl0RXhjZWVkZWQAAAAADQAAAAAAAAATU3Vic2NyaXB0aW9uUGFzdER1ZQAAAAAOAAAAAAAAABBQcm9wb3NlckNvb2xkb3duAAAADw==",
        "AAAAAQAAAWBBdWRpdCBtZXRhZGF0YSBhdHRhY2hlZCB0byBhIHBlcnNpc3RlZCB2YWx1ZS4KCkNvbnRyYWN0cyBzdG9yZSBkb21haW4gZGF0YSBpbiBTb3JvYmFuIGluc3RhbmNlIHN0b3JhZ2U7IHdyYXBwaW5nIGl0IHdpdGgKdGhpcyByZWNvcmQgbGV0cyBjYWxsZXJzIChhbmQgb2ZmLWNoYWluIGluZGV4ZXJzKSBzZWUgd2hlbiBhIHZhbHVlIHdhcyBsYXN0CndyaXR0ZW4uIFRoZSBwYXlsb2FkIGlzIHN0b3JlZCBhcyBvcGFxdWUgc2VyaWFsaXplZCBieXRlcyBzbyB0aGUgcmVjb3JkIGlzCnZhbGlkIGFzIGEgU29yb2JhbiBjb250cmFjdCB0eXBlIHdpdGhvdXQgY291cGxpbmcgaXQgdG8gYSBjb25jcmV0ZSBkb21haW4KdmFsdWUuAAAAAAAAAAxTdG9yYWdlRW50cnkAAAACAAAAPVVuaXggdGltZXN0YW1wIChzZWNvbmRzKSBhdCB3aGljaCB0aGUgZW50cnkgd2FzIGxhc3Qgd3JpdHRlbi4AAAAAAAAKdXBkYXRlZF9hdAAAAAAABgAAADdUaGUgc3RvcmVkIHBheWxvYWQsIHNlcmlhbGl6ZWQgYXMgb3BhcXVlIFNvcm9iYW4gYnl0ZXMuAAAAAAV2YWx1ZQAAAAAAAA4=",
      ]),
      options
    );
  }
  public readonly fromJSON = {
    touch_ttl: this.txFromJSON<Result<void>>,
    distribute: this.txFromJSON<Result<i128>>,
    get_royalty: this.txFromJSON<Result<Royalty>>,
    set_royalty: this.txFromJSON<Result<void>>,
    set_royalty_splits: this.txFromJSON<Result<void>>,
    settle_sale: this.txFromJSON<Result<Settlement>>,
    settle_sales: this.txFromJSON<Result<Array<Settlement>>>,
    get_settlement_summary: this.txFromJSON<Result<SettlementSummary>>,
    quote_sale: this.txFromJSON<Result<SaleQuote>>,
  };
}