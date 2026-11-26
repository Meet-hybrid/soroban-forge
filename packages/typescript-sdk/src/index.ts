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
export * from "./clients/index.js";

if (typeof window !== "undefined") {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId: "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
  }
} as const


/**
 * A single three-party escrow record.
 */
export interface EscrowData {
  /**
 * Absolute Unix timestamp (seconds) of the seller acceptance deadline
 * (#728). `None` — no acceptance deadline: the legacy `timeout` deadline
 * rules apply.
 */
  acceptance_deadline: Option<u64>;
  /**
 * Amount of `token` held in custody.
 */
amount: i128;
  /**
 * Tamper-evident hash commitment over the dispute evidence (#727).
 * `None` — no commitment was supplied at dispute time.
 */
  evidence_commitment: Option<Buffer>;
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
 * Lifecycle state of an escrow.
 */
export type EscrowStatus = {tag: "Pending", values: void} | {tag: "Funded", values: void} | {tag: "Completed", values: void} | {tag: "Refunded", values: void} | {tag: "Disputed", values: void} | {tag: "Cancelled", values: void};

/**
 * Settlement view of one basket component leg (#729).
 */
export interface BasketComponentSummary {
  /**
   * The leg's token.
   */
  token: string;
  /**
   * The leg's escrowed amount (never changes after creation).
   */
  amount: i128;
  /**
   * The leg's cumulative amount paid to the seller (`released`).
   */
  released: i128;
  /**
   * The leg's balance currently in custody (`amount - released`).
   */
  held: i128;
  /**
   * The leg's value returned to the buyer (zero unless the basket is
   * `Refunded`).
   */
  refunded: i128;
}

/**
 * Read-only settlement summary of a basket (#729): per-component entries
 * plus disjoint totals (`held + released + refunded == total_amount`).
 */
export interface BasketSettlementSummary {
  /**
   * The basket's id.
   */
  escrow_id: u64;
  /**
   * The basket's current lifecycle status.
   */
  status: EscrowStatus;
  /**
   * The basket's acceptance deadline, if it has one (#728).
   */
  acceptance_deadline: Option<u64>;
  /**
   * Total value ever escrowed: `sum(leg.amount)` across components.
   */
  total_amount: i128;
  /**
   * Value in custody: `sum(leg.remaining())` across components.
   */
  held: i128;
  /**
   * Value already paid to the seller: `sum(leg.released)` across
   * components (zero when the basket is `Refunded`).
   */
  released: i128;
  /**
   * Value returned to the buyer (`total_amount` when the basket is
   * `Refunded`, else zero).
   */
  refunded: i128;
  /**
   * One entry per basket component, in the basket's leg order.
   */
  components: Array<BasketComponentSummary>;
}

/**
 * Lifecycle state of a review-window release request (#726).
 */
export type ReleaseRequestState = {tag: "Pending", values: void} | {tag: "Approved", values: void} | {tag: "Expired", values: void};

/**
 * A review-window release request on a funded escrow (#726).
 */
export interface ReleaseRequest {
  /**
   * Absolute Unix timestamp (seconds) at which the request expires.
   */
  expiry: u64;
  /**
   * Current state of the request.
   */
  state: ReleaseRequestState;
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
 * A participant in a multi-party flow (escrow, governance, ...).
 */
export interface Party {
  /**
 * On-chain address of the participant.
 */
address: string;
  /**
 * Whether this party has granted approval for the current action.
 */
approved: boolean;
  /**
 * Human-readable role label, e.g. `"buyer"` or `"arbiter"`.
 */
role: string;
}


/**
 * Inclusive time window expressed as Unix timestamps (seconds).
 * 
 * Stored as plain `u64` because `soroban_sdk` models time as `u64`; a
 * dedicated newtype would add conversions without benefit.
 */
export interface TimeBounds {
  /**
 * Latest moment (inclusive) at which the window is active.
 */
end: u64;
  /**
 * Earliest moment (inclusive) at which the window is active.
 */
start: u64;
}


/**
 * A page of results plus the cursor needed to fetch the next page.
 * 
 * Items are stored as serialized `Bytes` so the helper is agnostic to the
 * concrete value type a contract paginates. Callers decode each item into
 * their domain type. `Debug` is omitted because the SDK collection does not
 * implement it for this contract type.
 */
export interface PaginatedResult {
  /**
 * Cursor describing the next page (offset advanced by `limit`).
 */
cursor: PaginationCursor;
  /**
 * Items on this page.
 */
items: Array<Buffer>;
  /**
 * Total number of items across all pages.
 */
total: u32;
}


/**
 * Offset/limit pair for paginated reads.
 */
export interface PaginationCursor {
  /**
 * Maximum number of items to return.
 */
limit: u32;
  /**
 * Number of items to skip from the start of the full result set.
 */
offset: u32;
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


/**
 * Audit metadata attached to a persisted value.
 * 
 * Contracts store domain data in Soroban instance storage; wrapping it with
 * this record lets callers (and off-chain indexers) see when a value was last
 * written. The payload is stored as opaque serialized bytes so the record is
 * valid as a Soroban contract type without coupling it to a concrete domain
 * value.
 */
export interface StorageEntry {
  /**
 * Unix timestamp (seconds) at which the entry was last written.
 */
updated_at: u64;
  /**
 * The stored payload, serialized as opaque Soroban bytes.
 */
value: Buffer;
}

export { Client } from "./clients/escrow.js";
