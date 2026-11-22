import { Buffer } from "buffer";
import { Address } from "@stellar/stellar-sdk";
import { Client as EscrowClient } from "./clients/escrow.js";
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
export * from "./indexer.js";
export * from "./event-decoders.js";
export * from "./helpers.js";

if (typeof window !== "undefined") {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId: "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
  },
} as const;

/**
 * A single three-party escrow record.
 */
export interface EscrowData {
  /**
   * Amount of `token` held in custody.
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
export type EscrowStatus =
  | { tag: "Pending"; values: void }
  | { tag: "Funded"; values: void }
  | { tag: "Completed"; values: void }
  | { tag: "Refunded"; values: void }
  | { tag: "Disputed"; values: void }
  | { tag: "Cancelled"; values: void };

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
   * A SEP-41 token invocation failed (insufficient balance, missing or
   * deauthorized trustline, undeployed token contract, or token-logic
   * rejection). The raw token error discriminant is intentionally not
   * forwarded — callers cannot tell which contract produced a forwarded
   * code, so it is bucketed; the root cause remains visible in the
   * transaction's diagnostic events.
   */
  11: { message: "TokenTransferFailed" },
  /**
   * A cross-contract invocation failed (target reverted or host
   * abort). The target reverts are surfaced here so the caller can
   * distinguish them from token-transfer failures, and the invoking
   * transaction is left un-executed.
   */
  12: { message: "ContractInvocationFailed" },
  /**
   * The requested withdrawal would push a token's rolling-window total
   * past its configured withdrawal limit. Kept distinct from
   * [`ForgeError::InvalidInput`] so a caller can tell a policy rejection
   * (a valid withdrawal that is too large right now) from a malformed
   * argument.
   */
  13: { message: "WithdrawalLimitExceeded" },
  /**
   * A subscription is in the `PastDue` lapsed state and the requested
   * operation cannot be performed until a catch-up charge restores it to
   * `Active`. Kept distinct from [`ForgeError::DeadlineReached`] so a caller
   * can tell a lapsed billing state from an argument that was merely
   * too early.
   */
  14: { message: "SubscriptionPastDue" },
  /**
   * A proposer has reached the maximum allowed concurrent active proposals
   * or is within the proposer cooldown window.
   */
  15: { message: "ProposerCooldown" },
};

/**
 * A single escrow contract result, decoded from the Stellar SDK's `Result<T>`
 * shape. A successful invocation includes the typed payload; a reverted call
 * includes a structured error object that can be inspected by callers without
 * spelunking through raw JSON.
 */
export type EscrowResult<T> =
  | { ok: true; value: T }
  | {
      ok: false;
      error: {
        code?: number;
        message?: string;
        raw?: unknown;
      };
    };

/**
 * Decode a generated escrow invocation result into a small, inspectable
 * JavaScript shape. This is intentionally narrow and escrow-specific: it
 * handles the exact success/error payloads returned by the generated client
 * without modifying the client surface itself.
 */
export function decodeEscrowResult<T>(raw: unknown): EscrowResult<T> {
  if (!raw || typeof raw !== "object" || !("ok" in raw)) {
    return {
      ok: false,
      error: {
        message: "Malformed escrow result payload",
        raw,
      },
    };
  }

  const result = raw as Record<string, unknown>;

  if (result.ok === true) {
    return {
      ok: true,
      value: result.value as T,
    };
  }

  const error = result.error;
  if (error && typeof error === "object") {
    const errRecord = error as Record<string, unknown>;
    const code =
      typeof errRecord.code === "number" ? errRecord.code : undefined;
    const message =
      typeof errRecord.message === "string"
        ? errRecord.message
        : "Unknown error";

    return {
      ok: false,
      error: {
        code,
        message,
        raw: error,
      },
    };
  }

  return {
    ok: false,
    error: {
      message: "Unknown error",
      raw: result.error ?? raw,
    },
  };
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

export interface Client {
  /**
   * Construct and simulate a cancel transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cancel a `Pending` escrow. Requires the buyer. Nothing has moved,
   * so no token transfer occurs.
   */
  cancel: (
    { escrow_id }: { escrow_id: u64 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Construct and simulate a refund transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Refund the buyer.
   *
   * Pre-deadline: seller authorizes (voluntary refund). Post-deadline:
   * buyer authorizes (reclaim of unfulfilled funds).
   */
  refund: (
    { escrow_id }: { escrow_id: u64 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Construct and simulate a deposit transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Fund the escrow, pulling tokens from the buyer into this contract.
   *
   * Ordering: transfer **first**, state write **second** — see the
   * module docs for why the inverse would be a fund-safety bug.
   */
  deposit: (
    { escrow_id }: { escrow_id: u64 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Construct and simulate a dispute transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Raise a dispute: the claimant (buyer or seller) authorizes, while
   * `Funded`. Freezes all payout paths until the arbiter resolves.
   */
  dispute: (
    { escrow_id, claimant }: { escrow_id: u64; claimant: string },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Construct and simulate a release transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Release funds to the seller. Seller-authorized: delivery
   * confirmation by the paid party, not the paying one.
   */
  release: (
    { escrow_id }: { escrow_id: u64 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Construct and simulate a resolve transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Resolve a dispute: arbiter only, final. Pays the full amount to the
   * seller (`true`) or refunds the buyer (`false`).
   */
  resolve: (
    {
      escrow_id,
      in_favor_of_seller,
    }: { escrow_id: u64; in_favor_of_seller: boolean },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Construct and simulate a touch_ttl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Permissionless keeper: bump the escrow entry's TTL without changing
   * any state. The existence check is deliberate — touching a missing
   * id must fail loudly so a keeper can distinguish "extended" from
   * "no such escrow".
   */
  touch_ttl: (
    { escrow_id }: { escrow_id: u64 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<void>>>;

  /**
   * Construct and simulate a get_escrow transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the full escrow record.
   */
  get_escrow: (
    { escrow_id }: { escrow_id: u64 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<EscrowData>>>;

  /**
   * Construct and simulate a get_status transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the current lifecycle status.
   */
  get_status: (
    { escrow_id }: { escrow_id: u64 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<EscrowStatus>>>;

  /**
   * Construct and simulate a create_escrow transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new escrow and return its stable id.
   *
   * Only the buyer authorizes at creation. The arbiter does not
   * authorize either: they must be able to `resolve` later even if
   * they never participated in creation.
   */
  create_escrow: (
    {
      buyer,
      seller,
      arbiter,
      token,
      amount,
      timeout,
    }: {
      buyer: string;
      seller: string;
      arbiter: string;
      token: string;
      amount: i128;
      timeout: u64;
    },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<Result<u64>>>;

  /**
   * Construct and simulate a escrows_for_participant transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the creation-order escrow ids one page at a time for a
   * participant. Read-only: never mutates storage and never errors — an
   * unknown address or one with no escrows simply yields an empty page.
   */
  escrows_for_participant: (
    {
      participant,
      cursor,
      limit,
    }: { participant: string; cursor: u32; limit: u32 },
    options?: MethodOptions,
  ) => Promise<AssembledTransaction<ParticipantEscrowsPage>>;
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
      },
  ): Promise<AssembledTransaction<T>> {
    return ContractClient.deploy(null, options);
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec(new EscrowClient(options).spec.entries),
      options,
    );
  }
  public readonly fromJSON = {
    cancel: this.txFromJSON<Result<void>>,
    refund: this.txFromJSON<Result<void>>,
    deposit: this.txFromJSON<Result<void>>,
    dispute: this.txFromJSON<Result<void>>,
    release: this.txFromJSON<Result<void>>,
    resolve: this.txFromJSON<Result<void>>,
    touch_ttl: this.txFromJSON<Result<void>>,
    get_escrow: this.txFromJSON<Result<EscrowData>>,
    get_status: this.txFromJSON<Result<EscrowStatus>>,
    create_escrow: this.txFromJSON<Result<u64>>,
    escrows_for_participant: this.txFromJSON<ParticipantEscrowsPage>,
  };
}
