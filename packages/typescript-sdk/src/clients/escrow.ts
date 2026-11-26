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
 * Storage keys. Escrow records are per-id **persistent** entries so the
 * byte budget scales per record; only the id counter lives in instance
 * storage (one small entry, written once per creation).
 */
export type DataKey = {tag: "Escrow", values: readonly [u64]} | {tag: "Basket", values: readonly [u64]} | {tag: "Count", values: void} | {tag: "ParticipantIndex", values: readonly [string]} | {tag: "EscrowExpiration", values: readonly [u64]} | {tag: "ReleaseRequest", values: readonly [u64]};


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
 * Records written by contract versions prior to the acceptance-deadline
 * and evidence-commitment features (#728/#727) do not contain
 * `acceptance_deadline` or `evidence_commitment` — not even in their older
 * `released`-only (schema V2) shape. They are loaded via a backward-compat
 * decode chain: current `EscrowData` → [`EscrowDataV2`] (10 fields, the
 * pre-deadline/evidence shape) → [`EscrowDataV1`] (9 fields, pre-`released`).
 * New records are written as `EscrowData`; the upgrade is lazy on first
 * read, exactly like the older
 */
export interface EscrowData {
  /**
 * Absolute Unix timestamp (seconds) of the seller acceptance deadline
 * (#728). At `now >= deadline` the buyer may recover via
 * [`SorobanForgeEscrow::refund_expired`]. `None` — no acceptance
 * deadline: the legacy `timeout` deadline rules apply.
 */
acceptance_deadline: Option<u64>;
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
 * Tamper-evident hash commitment over the dispute evidence (#727).
 * Set by [`SorobanForgeEscrow::dispute`]; preserved through dispute
 * resolution. `None` — no commitment was supplied at dispute time.
 */
evidence_commitment: Option<Buffer>;
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
 * Pre-partial-release escrow record shape (schema V1).
 *
 * Used **only** by [`load_escrow`] for backward-compatible decoding of
 * storage entries written by contract versions that pre-date the
 * `released` field. New code never writes this type; it is a read-only
 * migration aid.
 *
 * Soroban `#[contracttype]` structs are XDR symbol-keyed maps. The host
 * rejects deserialization when the map's entry count differs from the
 * struct's field count. Old records have 9 fields (no `released`), so they
 * fail to decode as the current `EscrowData`; they fall further back to
 * `EscrowDataV1`, then upgrade with `released = 0` and the newer fields
 * defaulting to `None`. The upgrade chain is current →
 * [`EscrowDataV2`] → this V1.
 */
export interface EscrowDataV1 {
  /**
 * Matches `EscrowData::amount`.
 */
amount: i128;
  /**
 * Matches `EscrowData::arbiter`.
 */
arbiter: string;
  /**
 * Matches `EscrowData::buyer`.
 */
buyer: string;
  /**
 * Matches `EscrowData::created_at`.
 */
created_at: u64;
  /**
 * Matches `EscrowData::escrow_id`.
 */
escrow_id: u64;
  /**
 * Matches `EscrowData::seller`.
 */
seller: string;
  /**
 * Matches `EscrowData::status`.
 */
status: EscrowStatus;
  /**
 * Matches `EscrowData::timeout`.
 */
timeout: u64;
  /**
 * Matches `EscrowData::token`.
 */
token: string;
}


/**
 * Pre-acceptance-deadline escrow record shape (schema V2).
 *
 * The exact 10-field struct that existed before the acceptance-deadline
 * and evidence-commitment features (#728/#727). Used **only** by
 * [`load_escrow`] for backward-compatible decoding of records written by
 * contract versions that have `released` but not the newer fields; new
 * code never writes this type — it is a read-only migration aid.
 */
export interface EscrowDataV2 {
  /**
 * Matches `EscrowData::amount`.
 */
amount: i128;
  /**
 * Matches `EscrowData::arbiter`.
 */
arbiter: string;
  /**
 * Matches `EscrowData::buyer`.
 */
buyer: string;
  /**
 * Matches `EscrowData::created_at`.
 */
created_at: u64;
  /**
 * Matches `EscrowData::escrow_id`.
 */
escrow_id: u64;
  /**
 * Matches `EscrowData::released`.
 */
released: i128;
  /**
 * Matches `EscrowData::seller`.
 */
seller: string;
  /**
 * Matches `EscrowData::status`.
 */
status: EscrowStatus;
  /**
 * Matches `EscrowData::timeout`.
 */
timeout: u64;
  /**
 * Matches `EscrowData::token`.
 */
token: string;
}

/**
 * Lifecycle state of an escrow.
 */
export type EscrowStatus = {tag: "Pending", values: void} | {tag: "Funded", values: void} | {tag: "Completed", values: void} | {tag: "Refunded", values: void} | {tag: "Disputed", values: void} | {tag: "Cancelled", values: void};


/**
 * A review-window release request on a funded escrow (issue #726).
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
 * Absolute Unix timestamp (seconds) of the seller acceptance deadline
 * (#728). At `now >= deadline` the buyer may recover via the basket
 * deadline refund path. `None` — no acceptance deadline.
 */
acceptance_deadline: Option<u64>;
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
 * Tamper-evident hash commitment over dispute evidence (#727). Stored
 * on the basket record by `dispute_basket`; preserved through
 * resolution. `None` — no commitment was supplied.
 */
evidence_commitment: Option<Buffer>;
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
 * A buyer-initiated review-window release request (issue #726).
 *
 * Stored at [`DataKey::ReleaseRequest`]; its lifecycle is `Pending` on
 * creation → `Approved` on buyer approval → `Expired` on settlement of the
 * expiry path.
 */
export type ReleaseRequestState = {tag: "Pending", values: void} | {tag: "Approved", values: void} | {tag: "Expired", values: void};


/**
 * Settlement view of one basket component leg (issue #729).
 *
 * The three totals are disjoint — `held + released + refunded == amount` —
 * and are derived from the leg itself (`remaining()`, `released`, `amount`).
 * On a `Refunded` basket the whole leg's value is reported as `refunded`
 * (`amount`) and both `released` and `held` are zero: the legs' `released`
 * fields are the payout loop's artifacts, so reporting them as released too
 * would double-count the value that came back to the buyer.
 */
export interface BasketComponentSummary {
  /**
 * The leg's escrowed amount (never changes after creation).
 */
amount: i128;
  /**
 * The leg's balance currently in custody (`amount - released`).
 */
held: i128;
  /**
 * The leg's value returned to the buyer (zero unless the basket is
 * `Refunded`).
 */
refunded: i128;
  /**
 * The leg's cumulative amount paid to the seller (`released`).
 */
released: i128;
  /**
 * The leg's token.
 */
token: string;
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
 * Read-only settlement summary of a basket (issue #729).
 *
 * `held + released + refunded == total_amount`: the value a basket ever
 * escrowed partitions exactly between the balance in custody, the amount
 * already paid to the seller, and the amount returned to the buyer. A
 * released value is never also counted as refunded and vice versa.
 */
export interface BasketSettlementSummary {
  /**
 * The basket's acceptance deadline, if it has one (#728).
 */
acceptance_deadline: Option<u64>;
  /**
 * One entry per basket component, in the basket's leg order.
 */
components: Array<BasketComponentSummary>;
  /**
 * The basket's id.
 */
escrow_id: u64;
  /**
 * Value in custody: `sum(leg.remaining())` across components (zero when
 * the basket is `Refunded`, since the payout path zeroes every leg's
 * balance).
 */
held: i128;
  /**
 * Value returned to the buyer: `total_amount` when the basket is
 * `Refunded`, else zero.
 */
refunded: i128;
  /**
 * Value already paid to the seller: `sum(leg.released)` across
 * components (reported as zero when the basket is `Refunded`).
 */
released: i128;
  /**
 * The basket's current lifecycle status.
 */
status: EscrowStatus;
  /**
 * Total value ever escrowed: `sum(leg.amount)` across components.
 */
total_amount: i128;
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

/**
 * Contract executable used for creating a new contract and used in
 * `CreateContractHostFnContext`.
 */
export type ContractExecutable = {tag: "Wasm", values: readonly [Buffer]} | {tag: "ExternalRef", values: readonly [ContractExecutableRef]};


/**
 * Executable referenced via a persistent storage entry owned by a contract,
 * either this contract or another contract.
 *
 * The persistent storage entry owned by the `owner` has the `tag` as its key.
 */
export interface ContractExecutableRef {
  owner: string;
  tag: string;
}

/**
 * Context of a single authorized call performed by an address.
 *
 * Custom account contracts that implement `__check_auth` special function
 * receive a list of `Context` values corresponding to all the calls that
 * need to be authorized.
 */
export type Context = {tag: "Contract", values: readonly [ContractContext]} | {tag: "CreateContractHostFn", values: readonly [CreateContractHostFnContext]} | {tag: "CreateContractWithCtorHostFn", values: readonly [CreateContractWithConstructorHostFnContext]};


/**
 * Authorization context of a single contract call.
 *
 * This struct corresponds to a `require_auth_for_args` call for an address
 * from `contract` function with `fn_name` name and `args` arguments.
 */
export interface ContractContext {
  args: Array<any>;
  contract: string;
  fn_name: string;
}


/**
 * Value of contract node in InvokerContractAuthEntry tree.
 */
export interface SubContractInvocation {
  context: ContractContext;
  sub_invocations: Array<InvokerContractAuthEntry>;
}

/**
 * A node in the tree of authorizations performed on behalf of the current
 * contract as invoker of the contracts deeper in the call stack.
 *
 * This is used as an argument of `authorize_as_current_contract` host function.
 *
 * This tree corresponds `require_auth[_for_args]` calls on behalf of the
 * current contract.
 */
export type InvokerContractAuthEntry = {tag: "Contract", values: readonly [SubContractInvocation]} | {tag: "CreateContractHostFn", values: readonly [CreateContractHostFnContext]} | {tag: "CreateContractWithCtorHostFn", values: readonly [CreateContractWithConstructorHostFnContext]};


/**
 * Authorization context for `create_contract` host function that creates a
 * new contract on behalf of authorizer address.
 */
export interface CreateContractHostFnContext {
  executable: ContractExecutable;
  salt: Buffer;
}


/**
 * Authorization context for `create_contract` host function that creates a
 * new contract on behalf of authorizer address.
 * This is the same as `CreateContractHostFnContext`, but also has
 * contract constructor arguments.
 */
export interface CreateContractWithConstructorHostFnContext {
  constructor_args: Array<any>;
  executable: ContractExecutable;
  salt: Buffer;
}

export type Executable = {tag: "Wasm", values: readonly [Buffer]} | {tag: "StellarAsset", values: void} | {tag: "Account", values: void};

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
  dispute: ({escrow_id, claimant, evidence_commitment}: {escrow_id: u64, claimant: string, evidence_commitment: Option<Buffer>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

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
  create_basket: ({buyer, seller, arbiter, assets, timeout, acceptance_deadline}: {buyer: string, seller: string, arbiter: string, assets: Array<EscrowAsset>, timeout: u64, acceptance_deadline: Option<u64>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a create_escrow transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new escrow and return its stable id.
   *
   * Only the buyer authorizes at creation. The arbiter does not
   * authorize either: they must be able to `resolve` later even if
   * they never participated in creation.
   */
  create_escrow: ({buyer, seller, arbiter, token, amount, timeout, acceptance_deadline}: {buyer: string, seller: string, arbiter: string, token: string, amount: i128, timeout: u64, acceptance_deadline: Option<u64>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

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
   * `evidence_commitment` follows the same rules as `dispute` (issue
   * #727): an empty `Some` is rejected; the commitment is stored on the
   * basket record and in the dispute event, and survives resolution.
   */
  dispute_basket: ({escrow_id, claimant, evidence_commitment}: {escrow_id: u64, claimant: string, evidence_commitment: Option<Buffer>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a refund_expired transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Permissionlessly refund the escrow's remaining balance to the buyer
   * once the deadline has passed. Only valid while `Funded`; a disputed
   * escrow stays frozen. Works for both record kinds (see the trait docs
   * for the boundary semantics).
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
   * Construct and simulate a request_release transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Open a review window on a funded escrow: the buyer requests that the
   * escrow be released within `review_window` seconds; approval goes
   * through the standard release payout, expiry refunds the buyer.
   */
  request_release: ({escrow_id, review_window}: {escrow_id: u64, review_window: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_basket_summary transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the read-only settlement summary of a basket (issue #729):
   * per-component entries plus disjoint totals
   * (`held + released + refunded == total_amount`), derived directly from
   * the component legs so the summary cannot disagree with the basket
   * record.
   *
   * A corrupt or missing component — a leg with negative amount,
   * negative `released`, or `released > amount` — is rejected with
   * [`ForgeError::InvalidInput`] rather than silently ignored.
   *
   * Read-only: nothing is written and no entry's TTL is extended.
   *
   * # Errors
   *
   * * [`ForgeError::NotFound`] — no basket with this id.
   * * [`ForgeError::InvalidInput`] — a component leg is corrupt or
   * missing (see above).
   */
  get_basket_summary: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<BasketSettlementSummary>>>

  /**
   * Construct and simulate a expire_release_request transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Settle an expired, still-pending release request: the buyer is
   * refunded the still-held balance and the escrow transitions to
   * `Refunded`. Transfer-before-state, so a failed payout leaves
   * everything intact; the request is marked `Expired` and cannot settle
   * twice.
   */
  expire_release_request: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a release_partial_basket transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pay `amount` of one leg to the seller. Seller-authorized.
   *
   * All validation before any state change or transfer, mirroring
   * `release_partial`.
   */
  release_partial_basket: ({escrow_id, token, amount}: {escrow_id: u64, token: string, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a approve_release_request transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Approve a pending release request (buyer-only): pay the **remaining**
   * balance to the seller through the standard release path and
   * transition the escrow to `Completed`.
   */
  approve_release_request: ({escrow_id}: {escrow_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

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
export interface Client {
  readonly spec: ContractSpec;
  txFromJSON: <T>(json: string) => AssembledTransaction<T>;
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
      new ContractSpec([ "AAAAAgAAAMBTdG9yYWdlIGtleXMuIEVzY3JvdyByZWNvcmRzIGFyZSBwZXItaWQgKipwZXJzaXN0ZW50KiogZW50cmllcyBzbyB0aGUKYnl0ZSBidWRnZXQgc2NhbGVzIHBlciByZWNvcmQ7IG9ubHkgdGhlIGlkIGNvdW50ZXIgbGl2ZXMgaW4gaW5zdGFuY2UKc3RvcmFnZSAob25lIHNtYWxsIGVudHJ5LCB3cml0dGVuIG9uY2UgcGVyIGNyZWF0aW9uKS4AAAAAAAAAB0RhdGFLZXkAAAAABgAAAAEAAAB4VGhlIGVzY3JvdyByZWNvcmQgZm9yIGB1NjRgIGlkLiBBbHdheXMgYSBzaW5nbGUtdG9rZW4gW2BFc2Nyb3dEYXRhYF0KKG9yIGEgbGVnYWN5IGBFc2Nyb3dEYXRhVjFgIGF3YWl0aW5nIGxhenkgdXBncmFkZSkuAAAABkVzY3JvdwAAAAAAAQAAAAYAAAABAAACEFRoZSBiYXNrZXQgZXNjcm93IHJlY29yZCBmb3IgYHU2NGAgaWQsIGFsd2F5cyBhIFtgQmFza2V0RXNjcm93RGF0YWBdLgoKQSBzZXBhcmF0ZSBrZXkgY2xhc3MgZnJvbSBbYERhdGFLZXk6OkVzY3Jvd2BdIHJhdGhlciB0aGFuIGEgbmV3IHZhcmlhbnQKb2YgdGhlIHNhbWUga2V5J3MgdmFsdWU6IGFuIGVzY3JvdyBpZCBuYW1lcyBleGFjdGx5IG9uZSByZWNvcmQgb2YKZXhhY3RseSBvbmUga2luZCwgc28gdGhlIHR3byBzaGFwZXMgY2FuIG5ldmVyIGJlIGNvbmZ1c2VkIG9uIHJlYWQsIGFuZAp0aGUgc3RvcmVkIGJ5dGVzIG9mIGV2ZXJ5IHByZS1leGlzdGluZyBzaW5nbGUtdG9rZW4gZXNjcm93IOKAlCBhbmQgb2YKZXZlcnkgZXZlbnQgcGF5bG9hZCBjYXJyeWluZyBvbmUg4oCUIHN0YXkgZXhhY3RseSBhcyB0aGV5IHdlcmUuIFRoZSBpZAppdHNlbGYgaXMgZHJhd24gZnJvbSB0aGUgc2hhcmVkIFtgRGF0YUtleTo6Q291bnRgXSBzZXF1ZW5jZSwgc28gdGhlIHR3bwpjbGFzc2VzIGNhbiBuZXZlciBtaW50IHRoZSBzYW1lIGlkLgAAAAZCYXNrZXQAAAAAAAEAAAAGAAAAAAAAABVNb25vdG9uaWMgaWQgY291bnRlci4AAAAAAAAFQ291bnQAAAAAAAABAAAEAENyZWF0aW9uLW9yZGVyIGlkcyBvZiBldmVyeSBub24tY2FuY2VsbGVkIGVzY3JvdyB0aGUgYEFkZHJlc3NgCnBhcnRpY2lwYXRlcyBpbiAoYXMgYnV5ZXIsIHNlbGxlciwgb3IgYXJiaXRlcikuIFdyaXR0ZW4gYXQgY3JlYXRpb24KYW5kIHJlbW92ZWQgb25seSB3aGVuIGEgcGVuZGluZyBlc2Nyb3cgaXMgY2FuY2VsbGVkLgoKUGVyc2lzdGVudCwgbm90IGluc3RhbmNlOiB0aGUgZW50cnkgZ3Jvd3Mgd2l0aCB0aGF0IHBhcnR5J3MgZXNjcm93CmNvdW50IGFuZCBpcyB3cml0dGVuIG9ubHkgb24gdGhlIGNyZWF0ZSBwYXRoLCBzbyBwYXJraW5nIGl0IGluCmluc3RhbmNlIHN0b3JhZ2Ugd291bGQgYmxvYXQgYSBzaGFyZWQgaG90IGVudHJ5IHdpdGggb25lIHBhcnR5J3MKZ3Jvd3RoICh0aGUgc2FtZSBwZXItcmVjb3JkLXNjYWxpbmcgYXJndW1lbnQgdGhhdCBwdXRzIGBFc2Nyb3coaWQpYAppbiBwZXJzaXN0ZW50IHN0b3JhZ2UgcmF0aGVyIHRoYW4gYSBzaW5nbGUgaW5zdGFuY2UgbWFwKS4gSXQgYWxzbwpjYXJyaWVzIGl0cyBvd24gZXh0ZW5zaWJsZSBUVEwgdW5kZXIgdGhlIHN0YW5kYXJkIGBidW1wX2VudHJ5YAp0aHJlc2hvbGQvZXh0ZW5kIHBhdHRlcm4sIG1pcnJvcmluZyBldmVyeSBvdGhlciBwZXJzaXN0ZW50IHdyaXRlLgoKVGhlIHZhbHVlIGlzIGEgc2luZ2xlIGBWZWM8dTY0PmAgYXBwZW5kZWQgaW4gY3JlYXRpb24gb3JkZXIgcmF0aGVyCnRoYW4gc2hhcmRlZCBwZXItcGFydHkga2V5cyAoYFBhcnRpY2lwYW50SW5kZXgoQWRkcmVzcywgdTY0KWApLgpUcmFkZW9mZjogb25lIGVudHJ5IHBlciBwYXJ0eSBrZWVwcyByZWFkcyBjaGVhcCDigJQgYSBwYWdlIGlzIG9uZQplbnRyeSByZWFkIHBsdXMgYW4gaW4tbWVtb3J5IHNsaWNlIOKAlCBhbmQgYXBwZW5kcyBhcmUgb25lIHJlYWQgKyBvbmUKcmV3cml0ZSBvZiB0aGF0IHBhcnR5J3MgKGlkLXNpemVkLCA4IGJ5dGVzIGVhY2gpIGxpc3QuIFRoZSBjb3N0IGlzCnRoYXQgYSBwYXJ0eSdzIGVudHJ5IGdyb3dzIHVuYm91bmRlZGx5IGFuZCBlYWMAAAAQUGFydGljaXBhbnRJbmRleAAAAAEAAAATAAAAAQAAAIFFeHBpcmF0aW9uIGxlZGdlciBtaXJyb3JlZCBmb3IgYW4gZXNjcm93IHJlY29yZCBiZWNhdXNlIFNESyAyNwpleHBvc2VzIFRUTCBpbnRyb3NwZWN0aW9uIHRvIHRlc3QgdXRpbGl0aWVzIGJ1dCBub3QgY29udHJhY3QgY29kZS4AAAAAAAAQRXNjcm93RXhwaXJhdGlvbgAAAAEAAAAGAAAAAQAAAa9BIHJldmlldy13aW5kb3cgcmVsZWFzZSByZXF1ZXN0IGZvciBlc2Nyb3cgYGlkYCAoaXNzdWUgIzcyNikuCgpTZXBhcmF0ZSBmcm9tIHRoZSBlc2Nyb3cgcmVjb3JkIGl0c2VsZjogYSByZXF1ZXN0IGNoYW5nZXMgbm8KYWNjb3VudGluZyB1bnRpbCBhcHByb3ZhbCAoc2VsbGVyIHBheW91dCkgb3IgZXhwaXJ5IChidXllciByZWZ1bmQpLAphbmQgdGhlIHR3byBzZXR0bGUgcGF0aHMgZWFjaCBjaGVjayB0aGUgZXNjcm93J3MgYEZ1bmRlZGAgc3RhdHVzLCBzbwpubyBjb21wb25lbnQgZXNjcm93IHNldHRsZXMgdHdpY2UgdGhyb3VnaCBkaWZmZXJlbnQgbGlmZWN5Y2xlCm9wZXJhdGlvbnMuIFdyaXRpbmcgdGhlIHJlcXVlc3QgaW4gaXRzIG93biBlbnRyeSBrZWVwcyB0aGUKcHJlLWV4aXN0aW5nIGBFc2Nyb3coaWQpYCByZWNvcmQgc2hhcGUgdW50b3VjaGVkLgAAAAAOUmVsZWFzZVJlcXVlc3QAAAAAAAEAAAAG",
        "AAAAAQAABABBIHNpbmdsZSB0aHJlZS1wYXJ0eSBlc2Nyb3cgcmVjb3JkLgoKIyMgQWNjb3VudGluZyBmaWVsZHMKCiogYGFtb3VudGAg4oCUIHRvdGFsIHRva2VucyBkZXBvc2l0ZWQgYnkgdGhlIGJ1eWVyOyBuZXZlciBjaGFuZ2VzIGFmdGVyCmBkZXBvc2l0YC4KKiBgcmVsZWFzZWRgIOKAlCBjdW11bGF0aXZlIHRva2VucyBhbHJlYWR5IHRyYW5zZmVycmVkIHRvIHRoZSBzZWxsZXIgdmlhCmByZWxlYXNlX3BhcnRpYWxgOyBgMGAgYmVmb3JlIGFueSBwYXJ0aWFsIHJlbGVhc2UgaXMgbWFkZS4KKiBgcmVtYWluaW5nYCAoZGVyaXZlZCkg4oCUIGBhbW91bnQgLSByZWxlYXNlZGA7IHRoZSBiYWxhbmNlIGN1cnJlbnRseQpoZWxkIGluIGN1c3RvZHkuIGByZWZ1bmRgIGFuZCBgcmVzb2x2ZWAgcGF5IHRoaXMgYW1vdW50OyBgcmVsZWFzZWAKYW5kIHRoZSBmaW5hbCBgcmVsZWFzZV9wYXJ0aWFsYCBtdXN0IGNvbnN1bWUgaXQgZW50aXJlbHkuCgojIyBTdG9yYWdlIGNvbXBhdGliaWxpdHkKClJlY29yZHMgd3JpdHRlbiBieSBjb250cmFjdCB2ZXJzaW9ucyBwcmlvciB0byB0aGUgYWNjZXB0YW5jZS1kZWFkbGluZQphbmQgZXZpZGVuY2UtY29tbWl0bWVudCBmZWF0dXJlcyAoIzcyOC8jNzI3KSBkbyBub3QgY29udGFpbgpgYWNjZXB0YW5jZV9kZWFkbGluZWAgb3IgYGV2aWRlbmNlX2NvbW1pdG1lbnRgIOKAlCBub3QgZXZlbiBpbiB0aGVpciBvbGRlcgpgcmVsZWFzZWRgLW9ubHkgKHNjaGVtYSBWMikgc2hhcGUuIFRoZXkgYXJlIGxvYWRlZCB2aWEgYSBiYWNrd2FyZC1jb21wYXQKZGVjb2RlIGNoYWluOiBjdXJyZW50IGBFc2Nyb3dEYXRhYCDihpIgW2BFc2Nyb3dEYXRhVjJgXSAoMTAgZmllbGRzLCB0aGUKcHJlLWRlYWRsaW5lL2V2aWRlbmNlIHNoYXBlKSDihpIgW2BFc2Nyb3dEYXRhVjFgXSAoOSBmaWVsZHMsIHByZS1gcmVsZWFzZWRgKS4KTmV3IHJlY29yZHMgYXJlIHdyaXR0ZW4gYXMgYEVzY3Jvd0RhdGFgOyB0aGUgdXBncmFkZSBpcyBsYXp5IG9uIGZpcnN0CnJlYWQsIGV4YWN0bHkgbGlrZSB0aGUgb2xkZXIgAAAAAAAAAApFc2Nyb3dEYXRhAAAAAAAMAAAA8EFic29sdXRlIFVuaXggdGltZXN0YW1wIChzZWNvbmRzKSBvZiB0aGUgc2VsbGVyIGFjY2VwdGFuY2UgZGVhZGxpbmUKKCM3MjgpLiBBdCBgbm93ID49IGRlYWRsaW5lYCB0aGUgYnV5ZXIgbWF5IHJlY292ZXIgdmlhCltgU29yb2JhbkZvcmdlRXNjcm93OjpyZWZ1bmRfZXhwaXJlZGBdLiBgTm9uZWAg4oCUIG5vIGFjY2VwdGFuY2UKZGVhZGxpbmU6IHRoZSBsZWdhY3kgYHRpbWVvdXRgIGRlYWRsaW5lIHJ1bGVzIGFwcGx5LgAAABNhY2NlcHRhbmNlX2RlYWRsaW5lAAAAA+gAAAAGAAAATlRvdGFsIGFtb3VudCBvZiBgdG9rZW5gIGRlcG9zaXRlZCBieSB0aGUgYnV5ZXIuIE5ldmVyIGNoYW5nZXMgYWZ0ZXIKYGRlcG9zaXRgLgAAAAAABmFtb3VudAAAAAAACwAAAFFOZXV0cmFsIHBhcnR5IGRlY2lkaW5nIGRpc3B1dGVzLiBSZWNvcmRlZCBhdCBjcmVhdGlvbjsgYXV0aG9yaXplcwpvbmx5IGByZXNvbHZlYC4AAAAAAAAHYXJiaXRlcgAAAAATAAAAOlBhcnR5IGZ1bmRpbmcgdGhlIGVzY3JvdyBhbmQgdGhlIGRlZmF1bHQgcmVmdW5kIHJlY2lwaWVudC4AAAAAAAVidXllcgAAAAAAABMAAAAbVW5peCB0aW1lc3RhbXAgb2YgY3JlYXRpb24uAAAAAApjcmVhdGVkX2F0AAAAAAAGAAAAGFN0YWJsZSBpZCwgbmV2ZXIgcmV1c2VkLgAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAxVRhbXBlci1ldmlkZW50IGhhc2ggY29tbWl0bWVudCBvdmVyIHRoZSBkaXNwdXRlIGV2aWRlbmNlICgjNzI3KS4KU2V0IGJ5IFtgU29yb2JhbkZvcmdlRXNjcm93OjpkaXNwdXRlYF07IHByZXNlcnZlZCB0aHJvdWdoIGRpc3B1dGUKcmVzb2x1dGlvbi4gYE5vbmVgIOKAlCBubyBjb21taXRtZW50IHdhcyBzdXBwbGllZCBhdCBkaXNwdXRlIHRpbWUuAAAAAAAAE2V2aWRlbmNlX2NvbW1pdG1lbnQAAAAD6AAAAA4AAABvQ3VtdWxhdGl2ZSBhbW91bnQgYWxyZWFkeSB0cmFuc2ZlcnJlZCB0byB0aGUgc2VsbGVyIHZpYQpgcmVsZWFzZV9wYXJ0aWFsYC4gYDBgIHVudGlsIHRoZSBmaXJzdCBwYXJ0aWFsIHJlbGVhc2UuAAAAAAhyZWxlYXNlZAAAAAsAAAAWUGFydHkgcGFpZCBvbiByZWxlYXNlLgAAAAAABnNlbGxlcgAAAAAAEwAAABhDdXJyZW50IGxpZmVjeWNsZSBzdGF0ZS4AAAAGc3RhdHVzAAAAAAfQAAAADEVzY3Jvd1N0YXR1cwAAAD5TZWNvbmRzIGFmdGVyIGBjcmVhdGVkX2F0YCBhdCB3aGljaCB0aGUgYnV5ZXIgbWF5IHNlbGYtcmVmdW5kLgAAAAAAB3RpbWVvdXQAAAAABgAAAC9TRVAtNDEgdG9rZW4gY29udHJhY3QgY3VzdG9kaWVkIGJ5IHRoaXMgZXNjcm93LgAAAAAFdG9rZW4AAAAAAAAT",
        "AAAAAQAAATBPbmUgbGVnIG9mIGEgYmFza2V0IGVzY3JvdzogYSBzaW5nbGUgU0VQLTQxIHRva2VuIGFuZCBpdHMgYWNjb3VudGluZy4KClRoZSBmaWVsZCBuYW1lcyBtaXJyb3IgW2BFc2Nyb3dEYXRhYF0ncyBzaW5nbGUtdG9rZW4gYWNjb3VudGluZyBleGFjdGx5IHNvCnRoZSB0d28gcmVjb3JkIGtpbmRzIGV4cHJlc3Mgb25lIHJ1bGUgcmF0aGVyIHRoYW4gdHdvOiBgcmVtYWluaW5nYCBpcwpgYW1vdW50IC0gcmVsZWFzZWRgLCBgcmVmdW5kYC9gcmVzb2x2ZWAgcGF5IGByZW1haW5pbmdgLCBhbmQgYSBmdWxsIHJlbGVhc2UKY29uc3VtZXMgaXQuAAAAAAAAAAtFc2Nyb3dBc3NldAAAAAADAAAAXlRvdGFsIGFtb3VudCBvZiBgdG9rZW5gIHRoaXMgbGVnIGVzY3Jvd3MuIEZpeGVkIGF0IGNyZWF0aW9uIGFuZAp1bmNoYW5nZWQgYnkgYGRlcG9zaXRfYmFza2V0YC4AAAAAAAZhbW91bnQAAAAAAAsAAAB6Q3VtdWxhdGl2ZSBhbW91bnQgb2YgYHRva2VuYCBhbHJlYWR5IHBhaWQgdG8gdGhlIHNlbGxlciB2aWEKYHJlbGVhc2VfcGFydGlhbF9iYXNrZXRgLiBgMGAgdW50aWwgdGhlIGZpcnN0IHBhcnRpYWwgcmVsZWFzZS4AAAAAAAhyZWxlYXNlZAAAAAsAAAAjU0VQLTQxIHRva2VuIGNvbnRyYWN0IGZvciB0aGlzIGxlZy4AAAAABXRva2VuAAAAAAAAEw==",
        "AAAAAQAAAsJQcmUtcGFydGlhbC1yZWxlYXNlIGVzY3JvdyByZWNvcmQgc2hhcGUgKHNjaGVtYSBWMSkuCgpVc2VkICoqb25seSoqIGJ5IFtgbG9hZF9lc2Nyb3dgXSBmb3IgYmFja3dhcmQtY29tcGF0aWJsZSBkZWNvZGluZyBvZgpzdG9yYWdlIGVudHJpZXMgd3JpdHRlbiBieSBjb250cmFjdCB2ZXJzaW9ucyB0aGF0IHByZS1kYXRlIHRoZQpgcmVsZWFzZWRgIGZpZWxkLiBOZXcgY29kZSBuZXZlciB3cml0ZXMgdGhpcyB0eXBlOyBpdCBpcyBhIHJlYWQtb25seQptaWdyYXRpb24gYWlkLgoKU29yb2JhbiBgI1tjb250cmFjdHR5cGVdYCBzdHJ1Y3RzIGFyZSBYRFIgc3ltYm9sLWtleWVkIG1hcHMuIFRoZSBob3N0CnJlamVjdHMgZGVzZXJpYWxpemF0aW9uIHdoZW4gdGhlIG1hcCdzIGVudHJ5IGNvdW50IGRpZmZlcnMgZnJvbSB0aGUKc3RydWN0J3MgZmllbGQgY291bnQuIE9sZCByZWNvcmRzIGhhdmUgOSBmaWVsZHMgKG5vIGByZWxlYXNlZGApLCBzbyB0aGV5CmZhaWwgdG8gZGVjb2RlIGFzIHRoZSBjdXJyZW50IGBFc2Nyb3dEYXRhYDsgdGhleSBmYWxsIGZ1cnRoZXIgYmFjayB0bwpgRXNjcm93RGF0YVYxYCwgdGhlbiB1cGdyYWRlIHdpdGggYHJlbGVhc2VkID0gMGAgYW5kIHRoZSBuZXdlciBmaWVsZHMKZGVmYXVsdGluZyB0byBgTm9uZWAuIFRoZSB1cGdyYWRlIGNoYWluIGlzIGN1cnJlbnQg4oaSCltgRXNjcm93RGF0YVYyYF0g4oaSIHRoaXMgVjEuAAAAAAAAAAAADEVzY3Jvd0RhdGFWMQAAAAkAAAAdTWF0Y2hlcyBgRXNjcm93RGF0YTo6YW1vdW50YC4AAAAAAAAGYW1vdW50AAAAAAALAAAAHk1hdGNoZXMgYEVzY3Jvd0RhdGE6OmFyYml0ZXJgLgAAAAAAB2FyYml0ZXIAAAAAEwAAABxNYXRjaGVzIGBFc2Nyb3dEYXRhOjpidXllcmAuAAAABWJ1eWVyAAAAAAAAEwAAACFNYXRjaGVzIGBFc2Nyb3dEYXRhOjpjcmVhdGVkX2F0YC4AAAAAAAAKY3JlYXRlZF9hdAAAAAAABgAAACBNYXRjaGVzIGBFc2Nyb3dEYXRhOjplc2Nyb3dfaWRgLgAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAHU1hdGNoZXMgYEVzY3Jvd0RhdGE6OnNlbGxlcmAuAAAAAAAABnNlbGxlcgAAAAAAEwAAAB1NYXRjaGVzIGBFc2Nyb3dEYXRhOjpzdGF0dXNgLgAAAAAAAAZzdGF0dXMAAAAAB9AAAAAMRXNjcm93U3RhdHVzAAAAHk1hdGNoZXMgYEVzY3Jvd0RhdGE6OnRpbWVvdXRgLgAAAAAAB3RpbWVvdXQAAAAABgAAABxNYXRjaGVzIGBFc2Nyb3dEYXRhOjp0b2tlbmAuAAAABXRva2VuAAAAAAAAEw==",
        "AAAAAQAAAYtQcmUtYWNjZXB0YW5jZS1kZWFkbGluZSBlc2Nyb3cgcmVjb3JkIHNoYXBlIChzY2hlbWEgVjIpLgoKVGhlIGV4YWN0IDEwLWZpZWxkIHN0cnVjdCB0aGF0IGV4aXN0ZWQgYmVmb3JlIHRoZSBhY2NlcHRhbmNlLWRlYWRsaW5lCmFuZCBldmlkZW5jZS1jb21taXRtZW50IGZlYXR1cmVzICgjNzI4LyM3MjcpLiBVc2VkICoqb25seSoqIGJ5CltgbG9hZF9lc2Nyb3dgXSBmb3IgYmFja3dhcmQtY29tcGF0aWJsZSBkZWNvZGluZyBvZiByZWNvcmRzIHdyaXR0ZW4gYnkKY29udHJhY3QgdmVyc2lvbnMgdGhhdCBoYXZlIGByZWxlYXNlZGAgYnV0IG5vdCB0aGUgbmV3ZXIgZmllbGRzOyBuZXcKY29kZSBuZXZlciB3cml0ZXMgdGhpcyB0eXBlIOKAlCBpdCBpcyBhIHJlYWQtb25seSBtaWdyYXRpb24gYWlkLgAAAAAAAAAADEVzY3Jvd0RhdGFWMgAAAAoAAAAdTWF0Y2hlcyBgRXNjcm93RGF0YTo6YW1vdW50YC4AAAAAAAAGYW1vdW50AAAAAAALAAAAHk1hdGNoZXMgYEVzY3Jvd0RhdGE6OmFyYml0ZXJgLgAAAAAAB2FyYml0ZXIAAAAAEwAAABxNYXRjaGVzIGBFc2Nyb3dEYXRhOjpidXllcmAuAAAABWJ1eWVyAAAAAAAAEwAAACFNYXRjaGVzIGBFc2Nyb3dEYXRhOjpjcmVhdGVkX2F0YC4AAAAAAAAKY3JlYXRlZF9hdAAAAAAABgAAACBNYXRjaGVzIGBFc2Nyb3dEYXRhOjplc2Nyb3dfaWRgLgAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAH01hdGNoZXMgYEVzY3Jvd0RhdGE6OnJlbGVhc2VkYC4AAAAACHJlbGVhc2VkAAAACwAAAB1NYXRjaGVzIGBFc2Nyb3dEYXRhOjpzZWxsZXJgLgAAAAAAAAZzZWxsZXIAAAAAABMAAAAdTWF0Y2hlcyBgRXNjcm93RGF0YTo6c3RhdHVzYC4AAAAAAAAGc3RhdHVzAAAAAAfQAAAADEVzY3Jvd1N0YXR1cwAAAB5NYXRjaGVzIGBFc2Nyb3dEYXRhOjp0aW1lb3V0YC4AAAAAAAd0aW1lb3V0AAAAAAYAAAAcTWF0Y2hlcyBgRXNjcm93RGF0YTo6dG9rZW5gLgAAAAV0b2tlbgAAAAAAABM=",
        "AAAAAgAAAB1MaWZlY3ljbGUgc3RhdGUgb2YgYW4gZXNjcm93LgAAAAAAAAAAAAAMRXNjcm93U3RhdHVzAAAABgAAAAAAAAAXQ3JlYXRlZCBidXQgbm90IGZ1bmRlZC4AAAAAB1BlbmRpbmcAAAAAAAAAABxUb2tlbnMgaGVsZCBieSB0aGUgY29udHJhY3QuAAAABkZ1bmRlZAAAAAAAAAAAAD9SZWxlYXNlZCB0byB0aGUgc2VsbGVyIChmdWxsIHJlbGVhc2Ugb3IgZmluYWwgcGFydGlhbCByZWxlYXNlKS4AAAAACUNvbXBsZXRlZAAAAAAAAAAAAAAWUmVmdW5kZWQgdG8gdGhlIGJ1eWVyLgAAAAAACFJlZnVuZGVkAAAAAAAAADxBIHBhcnR5IHJhaXNlZCBhIGRpc3B1dGU7IGZyb3plbiB1bnRpbCB0aGUgYXJiaXRlciByZXNvbHZlcy4AAAAIRGlzcHV0ZWQAAAAAAAAAGUNhbmNlbGxlZCBiZWZvcmUgZnVuZGluZy4AAAAAAAAJQ2FuY2VsbGVkAAAA",
        "AAAAAQAAAEBBIHJldmlldy13aW5kb3cgcmVsZWFzZSByZXF1ZXN0IG9uIGEgZnVuZGVkIGVzY3JvdyAoaXNzdWUgIzcyNikuAAAAAAAAAA5SZWxlYXNlUmVxdWVzdAAAAAAAAgAAAD9BYnNvbHV0ZSBVbml4IHRpbWVzdGFtcCAoc2Vjb25kcykgYXQgd2hpY2ggdGhlIHJlcXVlc3QgZXhwaXJlcy4AAAAABmV4cGlyeQAAAAAABgAAAB1DdXJyZW50IHN0YXRlIG9mIHRoZSByZXF1ZXN0LgAAAAAAAAVzdGF0ZQAAAAAAB9AAAAATUmVsZWFzZVJlcXVlc3RTdGF0ZQA=",
        "AAAAAQAAAZFBIGJhc2tldCBlc2Nyb3cgcmVjb3JkOiB0aGUgc2FtZSB0aHJlZS1wYXJ0eSBsaWZlY3ljbGUgYXMgW2BFc2Nyb3dEYXRhYF0sCndpdGggYW4gb3JkZXJlZCBsaXN0IG9mIGxlZ3MgaW5zdGVhZCBvZiBvbmUgdG9rZW4uCgpTdG9yZWQgYXQgW2BEYXRhS2V5OjpCYXNrZXRgXSwgYSBzZXBhcmF0ZSBrZXkgY2xhc3MgZnJvbQpbYERhdGFLZXk6OkVzY3Jvd2BdLCBzbyB0aGUgc2luZ2xlLXRva2VuIHJlY29yZCdzIHN0b3JlZCBzaGFwZSBhbmQgdGhlCmV2ZW50cyB0aGF0IGNhcnJ5IGl0IGFyZSB1bnRvdWNoZWQuIElkcyBjb21lIGZyb20gdGhlIHNhbWUKW2BEYXRhS2V5OjpDb3VudGBdIHNlcXVlbmNlLCBzbyBhbiBpZCBiZWxvbmdzIHRvIGV4YWN0bHkgb25lIGVzY3JvdyBvZgpleGFjdGx5IG9uZSBraW5kLgAAAAAAAAAAAAAQQmFza2V0RXNjcm93RGF0YQAAAAoAAAC+QWJzb2x1dGUgVW5peCB0aW1lc3RhbXAgKHNlY29uZHMpIG9mIHRoZSBzZWxsZXIgYWNjZXB0YW5jZSBkZWFkbGluZQooIzcyOCkuIEF0IGBub3cgPj0gZGVhZGxpbmVgIHRoZSBidXllciBtYXkgcmVjb3ZlciB2aWEgdGhlIGJhc2tldApkZWFkbGluZSByZWZ1bmQgcGF0aC4gYE5vbmVgIOKAlCBubyBhY2NlcHRhbmNlIGRlYWRsaW5lLgAAAAAAE2FjY2VwdGFuY2VfZGVhZGxpbmUAAAAD6AAAAAYAAABYTmV1dHJhbCBwYXJ0eSBkZWNpZGluZyBkaXNwdXRlcy4gUmVjb3JkZWQgYXQgY3JlYXRpb247IGF1dGhvcml6ZXMgb25seQpgcmVzb2x2ZV9iYXNrZXRgLgAAAAdhcmJpdGVyAAAAABMAAABgVGhlIGJhc2tldDogb25lIGxlZyBwZXIgdG9rZW4sIGluIHRoZSBvcmRlciB0aGUgYnV5ZXIgZGVjbGFyZWQuIE5vIHR3bwpsZWdzIG5hbWUgdGhlIHNhbWUgdG9rZW4uAAAABmFzc2V0cwAAAAAD6gAAB9AAAAALRXNjcm93QXNzZXQAAAAAOlBhcnR5IGZ1bmRpbmcgdGhlIGJhc2tldCBhbmQgdGhlIGRlZmF1bHQgcmVmdW5kIHJlY2lwaWVudC4AAAAAAAVidXllcgAAAAAAABMAAAAbVW5peCB0aW1lc3RhbXAgb2YgY3JlYXRpb24uAAAAAApjcmVhdGVkX2F0AAAAAAAGAAAAR1N0YWJsZSBpZCwgbmV2ZXIgcmV1c2VkLiBTaGFyZXMgdGhlIHNlcXVlbmNlIHdpdGggc2luZ2xlLXRva2VuIGVzY3Jvd3MuAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAslRhbXBlci1ldmlkZW50IGhhc2ggY29tbWl0bWVudCBvdmVyIGRpc3B1dGUgZXZpZGVuY2UgKCM3MjcpLiBTdG9yZWQKb24gdGhlIGJhc2tldCByZWNvcmQgYnkgYGRpc3B1dGVfYmFza2V0YDsgcHJlc2VydmVkIHRocm91Z2gKcmVzb2x1dGlvbi4gYE5vbmVgIOKAlCBubyBjb21taXRtZW50IHdhcyBzdXBwbGllZC4AAAAAABNldmlkZW5jZV9jb21taXRtZW50AAAAA+gAAAAOAAAAFlBhcnR5IHBhaWQgb24gcmVsZWFzZS4AAAAAAAZzZWxsZXIAAAAAABMAAABBQ3VycmVudCBsaWZlY3ljbGUgc3RhdGUuIElkZW50aWNhbCBzZW1hbnRpY3MgdG8gW2BFc2Nyb3dTdGF0dXNgXS4AAAAAAAAGc3RhdHVzAAAAAAfQAAAADEVzY3Jvd1N0YXR1cwAAAD5TZWNvbmRzIGFmdGVyIGBjcmVhdGVkX2F0YCBhdCB3aGljaCB0aGUgYnV5ZXIgbWF5IHNlbGYtcmVmdW5kLgAAAAAAB3RpbWVvdXQAAAAABg==",
        "AAAAAgAAAN1BIGJ1eWVyLWluaXRpYXRlZCByZXZpZXctd2luZG93IHJlbGVhc2UgcmVxdWVzdCAoaXNzdWUgIzcyNikuCgpTdG9yZWQgYXQgW2BEYXRhS2V5OjpSZWxlYXNlUmVxdWVzdGBdOyBpdHMgbGlmZWN5Y2xlIGlzIGBQZW5kaW5nYCBvbgpjcmVhdGlvbiDihpIgYEFwcHJvdmVkYCBvbiBidXllciBhcHByb3ZhbCDihpIgYEV4cGlyZWRgIG9uIHNldHRsZW1lbnQgb2YgdGhlCmV4cGlyeSBwYXRoLgAAAAAAAAAAAAATUmVsZWFzZVJlcXVlc3RTdGF0ZQAAAAADAAAAAAAAAEJUaGUgcmVxdWVzdCBpcyBsaXZlOiB0aGUgcmV2aWV3IHdpbmRvdyBpcyBvcGVuLCBubyBzZXR0bGVtZW50IHlldC4AAAAAAAdQZW5kaW5nAAAAAAAAAAA+VGhlIGJ1eWVyIGFwcHJvdmVkIHRoZSByZXF1ZXN0OyB0aGUgc2VsbGVyIHBheW91dCBhbHJlYWR5IHJhbi4AAAAAAAhBcHByb3ZlZAAAAAAAAABTVGhlIHJlcXVlc3QgZXhwaXJlZCB3aXRob3V0IGFwcHJvdmFsOyB0aGUgYnV5ZXIgd2FzIHJlZnVuZGVkIHRoZQpzdGlsbC1oZWxkIGFtb3VudC4AAAAAB0V4cGlyZWQA",
        "AAAAAQAAAeZTZXR0bGVtZW50IHZpZXcgb2Ygb25lIGJhc2tldCBjb21wb25lbnQgbGVnIChpc3N1ZSAjNzI5KS4KClRoZSB0aHJlZSB0b3RhbHMgYXJlIGRpc2pvaW50IOKAlCBgaGVsZCArIHJlbGVhc2VkICsgcmVmdW5kZWQgPT0gYW1vdW50YCDigJQKYW5kIGFyZSBkZXJpdmVkIGZyb20gdGhlIGxlZyBpdHNlbGYgKGByZW1haW5pbmcoKWAsIGByZWxlYXNlZGAsIGBhbW91bnRgKS4KT24gYSBgUmVmdW5kZWRgIGJhc2tldCB0aGUgd2hvbGUgbGVnJ3MgdmFsdWUgaXMgcmVwb3J0ZWQgYXMgYHJlZnVuZGVkYAooYGFtb3VudGApIGFuZCBib3RoIGByZWxlYXNlZGAgYW5kIGBoZWxkYCBhcmUgemVybzogdGhlIGxlZ3MnIGByZWxlYXNlZGAKZmllbGRzIGFyZSB0aGUgcGF5b3V0IGxvb3AncyBhcnRpZmFjdHMsIHNvIHJlcG9ydGluZyB0aGVtIGFzIHJlbGVhc2VkIHRvbwp3b3VsZCBkb3VibGUtY291bnQgdGhlIHZhbHVlIHRoYXQgY2FtZSBiYWNrIHRvIHRoZSBidXllci4AAAAAAAAAAAAWQmFza2V0Q29tcG9uZW50U3VtbWFyeQAAAAAABQAAADlUaGUgbGVnJ3MgZXNjcm93ZWQgYW1vdW50IChuZXZlciBjaGFuZ2VzIGFmdGVyIGNyZWF0aW9uKS4AAAAAAAAGYW1vdW50AAAAAAALAAAAPVRoZSBsZWcncyBiYWxhbmNlIGN1cnJlbnRseSBpbiBjdXN0b2R5IChgYW1vdW50IC0gcmVsZWFzZWRgKS4AAAAAAAAEaGVsZAAAAAsAAABNVGhlIGxlZydzIHZhbHVlIHJldHVybmVkIHRvIHRoZSBidXllciAoemVybyB1bmxlc3MgdGhlIGJhc2tldCBpcwpgUmVmdW5kZWRgKS4AAAAAAAAIcmVmdW5kZWQAAAALAAAAPFRoZSBsZWcncyBjdW11bGF0aXZlIGFtb3VudCBwYWlkIHRvIHRoZSBzZWxsZXIgKGByZWxlYXNlZGApLgAAAAhyZWxlYXNlZAAAAAsAAAAQVGhlIGxlZydzIHRva2VuLgAAAAV0b2tlbgAAAAAAABM=",
        "AAAAAQAAAKpBIHBhZ2Ugb2YgZXNjcm93IGlkcyBpbnZvbHZpbmcgYSBwYXJ0aWNpcGFudC4KClJldHVybmVkIGJ5IFtgU29yb2JhbkZvcmdlRXNjcm93Ojplc2Nyb3dzX2Zvcl9wYXJ0aWNpcGFudGBdOyBwb3dlcmVkIGJ5CnRoZSBwZXItcGFydHkgcGVyc2lzdGVudCBpbmRleCB3cml0dGVuIGF0IGNyZWF0aW9uLgAAAAAAAAAAABZQYXJ0aWNpcGFudEVzY3Jvd3NQYWdlAAAAAAADAAAAK0VzY3JvdyBpZHMgb24gdGhpcyBwYWdlLCBpbiBjcmVhdGlvbiBvcmRlci4AAAAAA2lkcwAAAAPqAAAABgAAAH1PZmZzZXQgZm9yIHRoZSBuZXh0IHBhZ2UsIG9yIGBOb25lYCB3aGVuIGBpZHNgIGVuZHMgdGhlIHBhcnRpY2lwYW50J3MKbGlzdC4gUmVwbGF5IGl0IHVudGlsIGBOb25lYCB0byBpdGVyYXRlIHRoZSB3aG9sZSBsaXN0LgAAAAAAAAtuZXh0X2N1cnNvcgAAAAPoAAAABAAAADlUb3RhbCBlc2Nyb3dzIGludm9sdmluZyB0aGUgcGFydGljaXBhbnQgYWNyb3NzIGFsbCBwYWdlcy4AAAAAAAAFdG90YWwAAAAAAAAE",
        "AAAAAQAAAUlSZWFkLW9ubHkgc2V0dGxlbWVudCBzdW1tYXJ5IG9mIGEgYmFza2V0IChpc3N1ZSAjNzI5KS4KCmBoZWxkICsgcmVsZWFzZWQgKyByZWZ1bmRlZCA9PSB0b3RhbF9hbW91bnRgOiB0aGUgdmFsdWUgYSBiYXNrZXQgZXZlcgplc2Nyb3dlZCBwYXJ0aXRpb25zIGV4YWN0bHkgYmV0d2VlbiB0aGUgYmFsYW5jZSBpbiBjdXN0b2R5LCB0aGUgYW1vdW50CmFscmVhZHkgcGFpZCB0byB0aGUgc2VsbGVyLCBhbmQgdGhlIGFtb3VudCByZXR1cm5lZCB0byB0aGUgYnV5ZXIuIEEKcmVsZWFzZWQgdmFsdWUgaXMgbmV2ZXIgYWxzbyBjb3VudGVkIGFzIHJlZnVuZGVkIGFuZCB2aWNlIHZlcnNhLgAAAAAAAAAAAAAXQmFza2V0U2V0dGxlbWVudFN1bW1hcnkAAAAACAAAADdUaGUgYmFza2V0J3MgYWNjZXB0YW5jZSBkZWFkbGluZSwgaWYgaXQgaGFzIG9uZSAoIzcyOCkuAAAAABNhY2NlcHRhbmNlX2RlYWRsaW5lAAAAA+gAAAAGAAAAOk9uZSBlbnRyeSBwZXIgYmFza2V0IGNvbXBvbmVudCwgaW4gdGhlIGJhc2tldCdzIGxlZyBvcmRlci4AAAAAAApjb21wb25lbnRzAAAAAAPqAAAH0AAAABZCYXNrZXRDb21wb25lbnRTdW1tYXJ5AAAAAAAQVGhlIGJhc2tldCdzIGlkLgAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAklZhbHVlIGluIGN1c3RvZHk6IGBzdW0obGVnLnJlbWFpbmluZygpKWAgYWNyb3NzIGNvbXBvbmVudHMgKHplcm8gd2hlbgp0aGUgYmFza2V0IGlzIGBSZWZ1bmRlZGAsIHNpbmNlIHRoZSBwYXlvdXQgcGF0aCB6ZXJvZXMgZXZlcnkgbGVnJ3MKYmFsYW5jZSkuAAAAAAAEaGVsZAAAAAsAAABVVmFsdWUgcmV0dXJuZWQgdG8gdGhlIGJ1eWVyOiBgdG90YWxfYW1vdW50YCB3aGVuIHRoZSBiYXNrZXQgaXMKYFJlZnVuZGVkYCwgZWxzZSB6ZXJvLgAAAAAAAAhyZWZ1bmRlZAAAAAsAAAB5VmFsdWUgYWxyZWFkeSBwYWlkIHRvIHRoZSBzZWxsZXI6IGBzdW0obGVnLnJlbGVhc2VkKWAgYWNyb3NzCmNvbXBvbmVudHMgKHJlcG9ydGVkIGFzIHplcm8gd2hlbiB0aGUgYmFza2V0IGlzIGBSZWZ1bmRlZGApLgAAAAAAAAhyZWxlYXNlZAAAAAsAAAAmVGhlIGJhc2tldCdzIGN1cnJlbnQgbGlmZWN5Y2xlIHN0YXR1cy4AAAAAAAZzdGF0dXMAAAAAB9AAAAAMRXNjcm93U3RhdHVzAAAAP1RvdGFsIHZhbHVlIGV2ZXIgZXNjcm93ZWQ6IGBzdW0obGVnLmFtb3VudClgIGFjcm9zcyBjb21wb25lbnRzLgAAAAAMdG90YWxfYW1vdW50AAAACw==",
        "AAAAAAAAAF5DYW5jZWwgYSBgUGVuZGluZ2AgZXNjcm93LiBSZXF1aXJlcyB0aGUgYnV5ZXIuIE5vdGhpbmcgaGFzIG1vdmVkLApzbyBubyB0b2tlbiB0cmFuc2ZlciBvY2N1cnMuAAAAAAAGY2FuY2VsAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAMRSZWZ1bmQgdGhlIGJ1eWVyLgoKUHJlLWRlYWRsaW5lOiBzZWxsZXIgYXV0aG9yaXplcyAodm9sdW50YXJ5IHJlZnVuZCkuIFBvc3QtZGVhZGxpbmU6CmJ1eWVyIGF1dGhvcml6ZXMgKHJlY2xhaW0gb2YgdW5mdWxmaWxsZWQgZnVuZHMpLiBSZWZ1bmRzIG9ubHkgdGhlCioqcmVtYWluaW5nKiogYmFsYW5jZSAoYGFtb3VudCAtIHJlbGVhc2VkYCkuAAAABnJlZnVuZAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAMBGdW5kIHRoZSBlc2Nyb3csIHB1bGxpbmcgdG9rZW5zIGZyb20gdGhlIGJ1eWVyIGludG8gdGhpcyBjb250cmFjdC4KCk9yZGVyaW5nOiB0cmFuc2ZlciAqKmZpcnN0KiosIHN0YXRlIHdyaXRlICoqc2Vjb25kKiog4oCUIHNlZSB0aGUKbW9kdWxlIGRvY3MgZm9yIHdoeSB0aGUgaW52ZXJzZSB3b3VsZCBiZSBhIGZ1bmQtc2FmZXR5IGJ1Zy4AAAAHZGVwb3NpdAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAIVSYWlzZSBhIGRpc3B1dGU6IHRoZSBjbGFpbWFudCAoYnV5ZXIgb3Igc2VsbGVyKSBhdXRob3JpemVzLCB3aGlsZQpgRnVuZGVkYC4gRnJlZXplcyB0aGUgcmVtYWluaW5nIGJhbGFuY2UgdW50aWwgdGhlIGFyYml0ZXIgcmVzb2x2ZXMuAAAAAAAAB2Rpc3B1dGUAAAAAAwAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAAAAAAIY2xhaW1hbnQAAAATAAAAAAAAABNldmlkZW5jZV9jb21taXRtZW50AAAAA+gAAAAOAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAYRSZWxlYXNlIHRoZSBmdWxsIHJlbWFpbmluZyBiYWxhbmNlIHRvIHRoZSBzZWxsZXIuIFNlbGxlci1hdXRob3JpemVkOgpkZWxpdmVyeSBjb25maXJtYXRpb24gYnkgdGhlIHBhaWQgcGFydHksIG5vdCB0aGUgcGF5aW5nIG9uZS4KCkJhY2t3YXJkLWNvbXBhdGlibGU6IGJlaGF2ZXMgaWRlbnRpY2FsbHkgdG8gdGhlIHByZS1wYXJ0aWFsLXJlbGVhc2UKYHJlbGVhc2VgIHdoZW4gYHJlbGVhc2VkID09IDBgLiBXaGVuIHBhcnRpYWwgcmVsZWFzZXMgaGF2ZSBhbHJlYWR5CmJlZW4gbWFkZSwgb25seSB0aGUgKipyZW1haW5pbmcqKiBiYWxhbmNlIChgYW1vdW50IC0gcmVsZWFzZWRgKSBpcwp0cmFuc2ZlcnJlZCDigJQgbWFpbnRhaW5pbmcgdGhlIGNvbnNlcnZhdGlvbiBpbnZhcmlhbnQuAAAAB3JlbGVhc2UAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAINSZXNvbHZlIGEgZGlzcHV0ZTogYXJiaXRlciBvbmx5LCBmaW5hbC4gUGF5cyB0aGUgKipyZW1haW5pbmcqKgpiYWxhbmNlIHRvIHRoZSBzZWxsZXIgKGB0cnVlYCkgb3IgcmVmdW5kcyBpdCB0byB0aGUgYnV5ZXIgKGBmYWxzZWApLgAAAAAHcmVzb2x2ZQAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAAAAABJpbl9mYXZvcl9vZl9zZWxsZXIAAAAAAAEAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAALxSZXR1cm4gdGhlIHJlbWFpbmluZyBsZWRnZXIgY291bnQgYmVmb3JlIHRoZSBlc2Nyb3cgcmVjb3JkIGV4cGlyZXMuCgpgTm90Rm91bmRgIG1lYW5zIHRoZSBpZCBuZXZlciBleGlzdGVkIG9yIGl0cyBwZXJzaXN0ZW50IGVudHJ5IGlzCmFscmVhZHkgYXJjaGl2ZWQuIFRoaXMgdmlldyBwZXJmb3JtcyBubyBUVEwgZXh0ZW5zaW9uLgAAAAh0dGxfaW5mbwAAAAEAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAD6QAAAAQAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAANlQZXJtaXNzaW9ubGVzcyBrZWVwZXI6IGJ1bXAgdGhlIGVzY3JvdyBlbnRyeSdzIFRUTCB3aXRob3V0IGNoYW5naW5nCmFueSBzdGF0ZS4gVGhlIGV4aXN0ZW5jZSBjaGVjayBpcyBkZWxpYmVyYXRlIOKAlCB0b3VjaGluZyBhIG1pc3NpbmcKaWQgbXVzdCBmYWlsIGxvdWRseSBzbyBhIGtlZXBlciBjYW4gZGlzdGluZ3Vpc2ggImV4dGVuZGVkIiBmcm9tCiJubyBzdWNoIGVzY3JvdyIuAAAAAAAACXRvdWNoX3R0bAAAAAAAAAEAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAABVSZWFkIGEgYmFza2V0IHJlY29yZC4AAAAAAAAKZ2V0X2Jhc2tldAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAH0AAAABBCYXNrZXRFc2Nyb3dEYXRhAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAOtSZWFkIHRoZSBmdWxsIGVzY3JvdyByZWNvcmQuIFRoZSByZXR1cm5lZCBgRXNjcm93RGF0YWAgZXhwb3NlcwpgcmVsZWFzZWRgIChjdW11bGF0aXZlIHNlbGxlciBwYXltZW50cykgYW5kIGByZW1haW5pbmcoKWAgKGJhbGFuY2UKaW4gY3VzdG9keSkuIEZvciByZWNvcmRzIGNyZWF0ZWQgYmVmb3JlIHRoZSBwYXJ0aWFsLXJlbGVhc2UgZmVhdHVyZQp3YXMgZGVwbG95ZWQsIGByZWxlYXNlZGAgd2lsbCBiZSBgMGAuAAAAAApnZXRfZXNjcm93AAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAfQAAAACkVzY3Jvd0RhdGEAAAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAADhSZWFkIHRoZSBjdXJyZW50IGxpZmVjeWNsZSBzdGF0dXMgb2YgZWl0aGVyIHJlY29yZCBraW5kLgAAAApnZXRfc3RhdHVzAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAfQAAAADEVzY3Jvd1N0YXR1cwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAFZDYW5jZWwgYSBgUGVuZGluZ2AgYmFza2V0LiBCdXllci1hdXRob3JpemVkOyBub3RoaW5nIGhhcyBtb3ZlZCwgc28gbm8KdHJhbnNmZXIgb2NjdXJzLgAAAAAADWNhbmNlbF9iYXNrZXQAAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAHlDcmVhdGUgYSBiYXNrZXQgZXNjcm93LiBCdXllci1hdXRob3JpemVkIG9ubHksIGV4YWN0bHkgYXMKW2BTZWxmOjpjcmVhdGVfZXNjcm93YF0sIGFuZCBkcmF3aW5nIGZyb20gdGhlIHNhbWUgaWQgc2VxdWVuY2UuAAAAAAAADWNyZWF0ZV9iYXNrZXQAAAAAAAAGAAAAAAAAAAVidXllcgAAAAAAABMAAAAAAAAABnNlbGxlcgAAAAAAEwAAAAAAAAAHYXJiaXRlcgAAAAATAAAAAAAAAAZhc3NldHMAAAAAA+oAAAfQAAAAC0VzY3Jvd0Fzc2V0AAAAAAAAAAAHdGltZW91dAAAAAAGAAAAAAAAABNhY2NlcHRhbmNlX2RlYWRsaW5lAAAAA+gAAAAGAAAAAQAAA+kAAAAGAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAM5DcmVhdGUgYSBuZXcgZXNjcm93IGFuZCByZXR1cm4gaXRzIHN0YWJsZSBpZC4KCk9ubHkgdGhlIGJ1eWVyIGF1dGhvcml6ZXMgYXQgY3JlYXRpb24uIFRoZSBhcmJpdGVyIGRvZXMgbm90CmF1dGhvcml6ZSBlaXRoZXI6IHRoZXkgbXVzdCBiZSBhYmxlIHRvIGByZXNvbHZlYCBsYXRlciBldmVuIGlmCnRoZXkgbmV2ZXIgcGFydGljaXBhdGVkIGluIGNyZWF0aW9uLgAAAAAADWNyZWF0ZV9lc2Nyb3cAAAAAAAAHAAAAAAAAAAVidXllcgAAAAAAABMAAAAAAAAABnNlbGxlcgAAAAAAEwAAAAAAAAAHYXJiaXRlcgAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAAAAAAHdGltZW91dAAAAAAGAAAAAAAAABNhY2NlcHRhbmNlX2RlYWRsaW5lAAAAA+gAAAAGAAAAAQAAA+kAAAAGAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAJFQYXkgZXZlcnkgbGVnJ3MgcmVtYWluaW5nIGJhbGFuY2UgYmFjayB0byB0aGUgYnV5ZXIuIFNlbGxlci1hdXRob3JpemVkCmJlZm9yZSB0aGUgZGVhZGxpbmUsIGJ1eWVyLWF1dGhvcml6ZWQgYWZ0ZXIsIGFzIGZvciBhIHNpbmdsZS10b2tlbgplc2Nyb3cuAAAAAAAADXJlZnVuZF9iYXNrZXQAAAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAYZSZXNvbHZlIGEgZGlzcHV0ZSBieSBzcGxpdHRpbmcgdGhlIGVzY3Jvd2VkIGFtb3VudCBiZXR3ZWVuIHRoZSBzZWxsZXIKYW5kIGJ1eWVyIGFjY29yZGluZyB0byBgc2VsbGVyX2Jwc2Agb3V0IG9mIDEwXzAwMC4KClRoZSBzZWxsZXIgc2hhcmUgaXMgZmxvb3JlZCBhdCBgYW1vdW50ICogc2VsbGVyX2JwcyAvIDEwXzAwMGAgYW5kIHRoZQpyZW1haW5kZXIgc3RheXMgd2l0aCB0aGUgYnV5ZXIuIGBzZWxsZXJfYnBzID09IDBgIHJlc29sdmVzIHRvIGEKYnV5ZXItb25seSByZWZ1bmQgYW5kIGBzZWxsZXJfYnBzID09IDEwXzAwMGAgcmVzb2x2ZXMgdG8gYSBmdWxsIHNlbGxlcgpwYXlvdXQuIEEgZmFpbGVkIHNlY29uZCBwYXlvdXQgcm9sbHMgYmFjayB0aGUgd2hvbGUgaW52b2NhdGlvbi4AAAAAAA1yZXNvbHZlX3NwbGl0AAAAAAAAAgAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAAAAAAKc2VsbGVyX2JwcwAAAAAABAAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAD1GdW5kIGEgYmFza2V0OiBwdWxsIGV2ZXJ5IGxlZyBmcm9tIHRoZSBidXllci4gQWxsLW9yLW5vdGhpbmcuAAAAAAAADmRlcG9zaXRfYmFza2V0AAAAAAABAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAUpSYWlzZSBhIGRpc3B1dGUgb24gYSBiYXNrZXQuIFRoZSBjbGFpbWFudCBtdXN0IGJlIGEgcGFydHkgYW5kIG11c3QKYXV0aG9yaXplIHRoZSBjYWxsOyBmcmVlemVzIGV2ZXJ5IGxlZyB1bnRpbCB0aGUgYXJiaXRlciByZXNvbHZlcy4KYGV2aWRlbmNlX2NvbW1pdG1lbnRgIGZvbGxvd3MgdGhlIHNhbWUgcnVsZXMgYXMgYGRpc3B1dGVgIChpc3N1ZQojNzI3KTogYW4gZW1wdHkgYFNvbWVgIGlzIHJlamVjdGVkOyB0aGUgY29tbWl0bWVudCBpcyBzdG9yZWQgb24gdGhlCmJhc2tldCByZWNvcmQgYW5kIGluIHRoZSBkaXNwdXRlIGV2ZW50LCBhbmQgc3Vydml2ZXMgcmVzb2x1dGlvbi4AAAAAAA5kaXNwdXRlX2Jhc2tldAAAAAAAAwAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAAAAAAIY2xhaW1hbnQAAAATAAAAAAAAABNldmlkZW5jZV9jb21taXRtZW50AAAAA+gAAAAOAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAOlQZXJtaXNzaW9ubGVzc2x5IHJlZnVuZCB0aGUgZXNjcm93J3MgcmVtYWluaW5nIGJhbGFuY2UgdG8gdGhlIGJ1eWVyCm9uY2UgdGhlIGRlYWRsaW5lIGhhcyBwYXNzZWQuIE9ubHkgdmFsaWQgd2hpbGUgYEZ1bmRlZGA7IGEgZGlzcHV0ZWQKZXNjcm93IHN0YXlzIGZyb3plbi4gV29ya3MgZm9yIGJvdGggcmVjb3JkIGtpbmRzIChzZWUgdGhlIHRyYWl0IGRvY3MKZm9yIHRoZSBib3VuZGFyeSBzZW1hbnRpY3MpLgAAAAAAAA5yZWZ1bmRfZXhwaXJlZAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAENQYXkgZXZlcnkgbGVnJ3MgcmVtYWluaW5nIGJhbGFuY2UgdG8gdGhlIHNlbGxlci4gU2VsbGVyLWF1dGhvcml6ZWQuAAAAAA5yZWxlYXNlX2Jhc2tldAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAMFSZXNvbHZlIGEgYmFza2V0IGRpc3B1dGU6IGFyYml0ZXIgb25seSwgZmluYWwsIG9uZSBkaXJlY3Rpb24gZm9yIGV2ZXJ5CmxlZy4gUGF5b3V0IGdvZXMgdGhyb3VnaCB0aGUgc2FtZSBsb29wIGFzIGByZWxlYXNlX2Jhc2tldGAsIHNvIGEKcmVzb2x2ZSBjYW4gbmV2ZXIgc2V0dGxlIGxlZ3MgZGlmZmVyZW50bHkgdGhhbiBhIHJlbGVhc2UuAAAAAAAADnJlc29sdmVfYmFza2V0AAAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAAAAABJpbl9mYXZvcl9vZl9zZWxsZXIAAAAAAAEAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAYBSZWxlYXNlIGEgcGFydGlhbCBgYW1vdW50YCB0byB0aGUgc2VsbGVyLiBTZWxsZXItYXV0aG9yaXplZC4KCk9ubHkgdmFsaWQgd2hpbGUgYEZ1bmRlZGAuIGBhbW91bnRgIG11c3QgYmUgcG9zaXRpdmUgYW5kIG11c3Qgbm90CmV4Y2VlZCB0aGUgcmVtYWluaW5nIGJhbGFuY2UuIFdoZW4gYGFtb3VudCA9PSByZW1haW5pbmdgLCB0aGUgZXNjcm93CnRyYW5zaXRpb25zIHRvIGBDb21wbGV0ZWRgIGluIHRoZSBzYW1lIGNhbGwuCgpUcmFuc2Zlci1iZWZvcmUtc3RhdGUgb3JkZXJpbmcgaXMgcHJlc2VydmVkOiB0aGUgdG9rZW4gcGF5b3V0IGhhcHBlbnMKYmVmb3JlIGFueSBzdG9yYWdlIHdyaXRlLCBzbyBhIGZhaWxlZCB0cmFuc2ZlciBsZWF2ZXMgbm8gcGFydGlhbCBzdGF0ZS4AAAAPcmVsZWFzZV9wYXJ0aWFsAAAAAAIAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAMRPcGVuIGEgcmV2aWV3IHdpbmRvdyBvbiBhIGZ1bmRlZCBlc2Nyb3c6IHRoZSBidXllciByZXF1ZXN0cyB0aGF0IHRoZQplc2Nyb3cgYmUgcmVsZWFzZWQgd2l0aGluIGByZXZpZXdfd2luZG93YCBzZWNvbmRzOyBhcHByb3ZhbCBnb2VzCnRocm91Z2ggdGhlIHN0YW5kYXJkIHJlbGVhc2UgcGF5b3V0LCBleHBpcnkgcmVmdW5kcyB0aGUgYnV5ZXIuAAAAD3JlcXVlc3RfcmVsZWFzZQAAAAACAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAAAAAA1yZXZpZXdfd2luZG93AAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAo1SZWFkIHRoZSByZWFkLW9ubHkgc2V0dGxlbWVudCBzdW1tYXJ5IG9mIGEgYmFza2V0IChpc3N1ZSAjNzI5KToKcGVyLWNvbXBvbmVudCBlbnRyaWVzIHBsdXMgZGlzam9pbnQgdG90YWxzCihgaGVsZCArIHJlbGVhc2VkICsgcmVmdW5kZWQgPT0gdG90YWxfYW1vdW50YCksIGRlcml2ZWQgZGlyZWN0bHkgZnJvbQp0aGUgY29tcG9uZW50IGxlZ3Mgc28gdGhlIHN1bW1hcnkgY2Fubm90IGRpc2FncmVlIHdpdGggdGhlIGJhc2tldApyZWNvcmQuCgpBIGNvcnJ1cHQgb3IgbWlzc2luZyBjb21wb25lbnQg4oCUIGEgbGVnIHdpdGggbmVnYXRpdmUgYW1vdW50LApuZWdhdGl2ZSBgcmVsZWFzZWRgLCBvciBgcmVsZWFzZWQgPiBhbW91bnRgIOKAlCBpcyByZWplY3RlZCB3aXRoCltgRm9yZ2VFcnJvcjo6SW52YWxpZElucHV0YF0gcmF0aGVyIHRoYW4gc2lsZW50bHkgaWdub3JlZC4KClJlYWQtb25seTogbm90aGluZyBpcyB3cml0dGVuIGFuZCBubyBlbnRyeSdzIFRUTCBpcyBleHRlbmRlZC4KCiMgRXJyb3JzCgoqIFtgRm9yZ2VFcnJvcjo6Tm90Rm91bmRgXSDigJQgbm8gYmFza2V0IHdpdGggdGhpcyBpZC4KKiBbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIOKAlCBhIGNvbXBvbmVudCBsZWcgaXMgY29ycnVwdCBvcgptaXNzaW5nIChzZWUgYWJvdmUpLgAAAAAAABJnZXRfYmFza2V0X3N1bW1hcnkAAAAAAAEAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAD6QAAB9AAAAAXQmFza2V0U2V0dGxlbWVudFN1bW1hcnkAAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAQVTZXR0bGUgYW4gZXhwaXJlZCwgc3RpbGwtcGVuZGluZyByZWxlYXNlIHJlcXVlc3Q6IHRoZSBidXllciBpcwpyZWZ1bmRlZCB0aGUgc3RpbGwtaGVsZCBiYWxhbmNlIGFuZCB0aGUgZXNjcm93IHRyYW5zaXRpb25zIHRvCmBSZWZ1bmRlZGAuIFRyYW5zZmVyLWJlZm9yZS1zdGF0ZSwgc28gYSBmYWlsZWQgcGF5b3V0IGxlYXZlcwpldmVyeXRoaW5nIGludGFjdDsgdGhlIHJlcXVlc3QgaXMgbWFya2VkIGBFeHBpcmVkYCBhbmQgY2Fubm90IHNldHRsZQp0d2ljZS4AAAAAAAAWZXhwaXJlX3JlbGVhc2VfcmVxdWVzdAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAItQYXkgYGFtb3VudGAgb2Ygb25lIGxlZyB0byB0aGUgc2VsbGVyLiBTZWxsZXItYXV0aG9yaXplZC4KCkFsbCB2YWxpZGF0aW9uIGJlZm9yZSBhbnkgc3RhdGUgY2hhbmdlIG9yIHRyYW5zZmVyLCBtaXJyb3JpbmcKYHJlbGVhc2VfcGFydGlhbGAuAAAAABZyZWxlYXNlX3BhcnRpYWxfYmFza2V0AAAAAAADAAAAAAAAAAllc2Nyb3dfaWQAAAAAAAAGAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAKdBcHByb3ZlIGEgcGVuZGluZyByZWxlYXNlIHJlcXVlc3QgKGJ1eWVyLW9ubHkpOiBwYXkgdGhlICoqcmVtYWluaW5nKioKYmFsYW5jZSB0byB0aGUgc2VsbGVyIHRocm91Z2ggdGhlIHN0YW5kYXJkIHJlbGVhc2UgcGF0aCBhbmQKdHJhbnNpdGlvbiB0aGUgZXNjcm93IHRvIGBDb21wbGV0ZWRgLgAAAAAXYXBwcm92ZV9yZWxlYXNlX3JlcXVlc3QAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
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
        "AAAABQAAAHlFbWl0dGVkIGJ5IGBiYXNrZXRfcmVmdW5kX2V4cGlyZWRgIHdoZW4gYSBiYXNrZXQncyBhY2NlcHRhbmNlCmRlYWRsaW5lIChpc3N1ZSAjNzI4KSBpcyByZWFjaGVkIGFuZCB0aGUgYnV5ZXIgaXMgcmVmdW5kZWQuAAAAAAAAAAAAABNCYXNrZXRSZWZ1bmRFeHBpcmVkAAAAAAEAAAAVYmFza2V0X3JlZnVuZF9leHBpcmVkAAAAAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAAC",
        "AAAABQAAAAAAAAAAAAAAFVJlbGVhc2VSZXF1ZXN0Q3JlYXRlZAAAAAAAAAEAAAAXcmVsZWFzZV9yZXF1ZXN0X2NyZWF0ZWQAAAAAAgAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAA/QWJzb2x1dGUgVW5peCB0aW1lc3RhbXAgKHNlY29uZHMpIGF0IHdoaWNoIHRoZSByZXF1ZXN0IGV4cGlyZXMuAAAAAAZleHBpcnkAAAAAAAYAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAFVJlbGVhc2VSZXF1ZXN0RXhwaXJlZAAAAAAAAAEAAAAXcmVsZWFzZV9yZXF1ZXN0X2V4cGlyZWQAAAAAAgAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAAkVW5peCB0aW1lc3RhbXAgb2YgZXhwaXJ5IHNldHRsZW1lbnQuAAAACXRpbWVzdGFtcAAAAAAAAAYAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAFlJlbGVhc2VSZXF1ZXN0QXBwcm92ZWQAAAAAAAEAAAAYcmVsZWFzZV9yZXF1ZXN0X2FwcHJvdmVkAAAAAQAAAAAAAAAJZXNjcm93X2lkAAAAAAAABgAAAAEAAAAC",
        "AAAABQAAARNFbWl0dGVkIGJ5IGByZWxlYXNlX3BhcnRpYWxfYmFza2V0YCBmb3IgZXZlcnkgaW5jcmVtZW50YWwgcGF5b3V0IG9mIG9uZQpsZWcuIGB0b2tlbmAgYW5kIGBwYXJ0aWFsX2Ftb3VudGAgZGVzY3JpYmUgdGhlIHRyYW5zZmVycmVkIGxlZzsgYGRhdGFgCmNhcnJpZXMgdGhlIHBvc3QtdXBkYXRlIGBCYXNrZXRFc2Nyb3dEYXRhYCAocGVyLWxlZyBgcmVsZWFzZWRgIGFuZCB0aGUKcG9zc2libGUgYENvbXBsZXRlZGAgc3RhdHVzIG9uY2UgKipldmVyeSoqIGxlZyBpcyBleGhhdXN0ZWQpLgAAAAAAAAAAF0Jhc2tldFBhcnRpYWxseVJlbGVhc2VkAAAAAAEAAAAZYmFza2V0X3BhcnRpYWxseV9yZWxlYXNlZAAAAAAAAAQAAAAAAAAACWVzY3Jvd19pZAAAAAAAAAYAAAABAAAAI1RoZSBsZWcgdGhhdCB3YXMgcGFpZCBpbiB0aGlzIGNhbGwuAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAPVRoZSBhbW91bnQgdHJhbnNmZXJyZWQgdG8gdGhlIHNlbGxlciBpbiB0aGlzIHBhcnRpY3VsYXIgY2FsbC4AAAAAAAAOcGFydGlhbF9hbW91bnQAAAAAAAsAAAAAAAAALkZ1bGwgYmFza2V0IHJlY29yZCBhZnRlciB0aGlzIHBhcnRpYWwgcmVsZWFzZS4AAAAAAARkYXRhAAAH0AAAABBCYXNrZXRFc2Nyb3dEYXRhAAAAAAAAAAI=",
        "AAAAAQAAAD5BIHBhcnRpY2lwYW50IGluIGEgbXVsdGktcGFydHkgZmxvdyAoZXNjcm93LCBnb3Zlcm5hbmNlLCAuLi4pLgAAAAAAAAAAAAVQYXJ0eQAAAAAAAAMAAAAkT24tY2hhaW4gYWRkcmVzcyBvZiB0aGUgcGFydGljaXBhbnQuAAAAB2FkZHJlc3MAAAAAEwAAAD9XaGV0aGVyIHRoaXMgcGFydHkgaGFzIGdyYW50ZWQgYXBwcm92YWwgZm9yIHRoZSBjdXJyZW50IGFjdGlvbi4AAAAACGFwcHJvdmVkAAAAAQAAADlIdW1hbi1yZWFkYWJsZSByb2xlIGxhYmVsLCBlLmcuIGAiYnV5ZXIiYCBvciBgImFyYml0ZXIiYC4AAAAAAAAEcm9sZQAAABA=",
        "AAAAAQAAALtJbmNsdXNpdmUgdGltZSB3aW5kb3cgZXhwcmVzc2VkIGFzIFVuaXggdGltZXN0YW1wcyAoc2Vjb25kcykuCgpTdG9yZWQgYXMgcGxhaW4gYHU2NGAgYmVjYXVzZSBgc29yb2Jhbl9zZGtgIG1vZGVscyB0aW1lIGFzIGB1NjRgOyBhCmRlZGljYXRlZCBuZXd0eXBlIHdvdWxkIGFkZCBjb252ZXJzaW9ucyB3aXRob3V0IGJlbmVmaXQuAAAAAAAAAAAKVGltZUJvdW5kcwAAAAAAAgAAADhMYXRlc3QgbW9tZW50IChpbmNsdXNpdmUpIGF0IHdoaWNoIHRoZSB3aW5kb3cgaXMgYWN0aXZlLgAAAANlbmQAAAAABgAAADpFYXJsaWVzdCBtb21lbnQgKGluY2x1c2l2ZSkgYXQgd2hpY2ggdGhlIHdpbmRvdyBpcyBhY3RpdmUuAAAAAAAFc3RhcnQAAAAAAAAG",
        "AAAAAQAAAUBBIHBhZ2Ugb2YgcmVzdWx0cyBwbHVzIHRoZSBjdXJzb3IgbmVlZGVkIHRvIGZldGNoIHRoZSBuZXh0IHBhZ2UuCgpJdGVtcyBhcmUgc3RvcmVkIGFzIHNlcmlhbGl6ZWQgYEJ5dGVzYCBzbyB0aGUgaGVscGVyIGlzIGFnbm9zdGljIHRvIHRoZQpjb25jcmV0ZSB2YWx1ZSB0eXBlIGEgY29udHJhY3QgcGFnaW5hdGVzLiBDYWxsZXJzIGRlY29kZSBlYWNoIGl0ZW0gaW50bwp0aGVpciBkb21haW4gdHlwZS4gYERlYnVnYCBpcyBvbWl0dGVkIGJlY2F1c2UgdGhlIFNESyBjb2xsZWN0aW9uIGRvZXMgbm90CmltcGxlbWVudCBpdCBmb3IgdGhpcyBjb250cmFjdCB0eXBlLgAAAAAAAAAPUGFnaW5hdGVkUmVzdWx0AAAAAAMAAAA9Q3Vyc29yIGRlc2NyaWJpbmcgdGhlIG5leHQgcGFnZSAob2Zmc2V0IGFkdmFuY2VkIGJ5IGBsaW1pdGApLgAAAAAAAAZjdXJzb3IAAAAAB9AAAAAQUGFnaW5hdGlvbkN1cnNvcgAAABNJdGVtcyBvbiB0aGlzIHBhZ2UuAAAAAAVpdGVtcwAAAAAAA+oAAAAOAAAAJ1RvdGFsIG51bWJlciBvZiBpdGVtcyBhY3Jvc3MgYWxsIHBhZ2VzLgAAAAAFdG90YWwAAAAAAAAE",
        "AAAAAQAAACZPZmZzZXQvbGltaXQgcGFpciBmb3IgcGFnaW5hdGVkIHJlYWRzLgAAAAAAAAAAABBQYWdpbmF0aW9uQ3Vyc29yAAAAAgAAACJNYXhpbXVtIG51bWJlciBvZiBpdGVtcyB0byByZXR1cm4uAAAAAAAFbGltaXQAAAAAAAAEAAAAPk51bWJlciBvZiBpdGVtcyB0byBza2lwIGZyb20gdGhlIHN0YXJ0IG9mIHRoZSBmdWxsIHJlc3VsdCBzZXQuAAAAAAAGb2Zmc2V0AAAAAAAE",
        "AAAABAAAAZxTaGFyZWQgZXJyb3IgdHlwZSB1c2VkIGFjcm9zcyBhbGwgU29yb2JhbiBGb3JnZSBjb250cmFjdHMuCgpEZWZpbmluZyBhIHNpbmdsZSBlcnJvciBlbnVtIGluIGBzaGFyZWQtdXRpbHNgIGtlZXBzIHRoZSBvbi1jaGFpbiBlcnJvcgpzcGFjZSBjb25zaXN0ZW50IGFuZCBpbnRlbGxpZ2libGUgdG8gU0RLIGNvbnN1bWVycywgYW5kIGF2b2lkcyBldmVyeQpjb250cmFjdCByZS1kZWNsYXJpbmcgdGhlIHNhbWUgZmFpbHVyZSBtb2Rlcy4gQ29udHJhY3QgY3JhdGVzIG1heSBleHBvc2UKdGhlaXIgb3duIGRvbWFpbi1zcGVjaWZpYyBlcnJvcnMsIGJ1dCBzaG91bGQgcHJlZmVyIHRoZXNlIHdoZXJlIHRoZXkgZml0LgoKRXJyb3IgY29kZXMgc3RhcnQgYXQgMTsgY29kZSAwIGlzIHJlc2VydmVkIGJ5IHRoZSBTb3JvYmFuIGhvc3QuAAAAAAAAAApGb3JnZUVycm9yAAAAAAAPAAAAM1RoZSBjYWxsZXIgaXMgbm90IHBlcm1pdHRlZCB0byBwZXJmb3JtIHRoaXMgYWN0aW9uLgAAAAAMVW5hdXRob3JpemVkAAAAAQAAAEpUaGUgcmVxdWVzdGVkIGVudGl0eSAoZXNjcm93LCBwcm9wb3NhbCwgc3Vic2NyaXB0aW9uLCAuLi4pIGRvZXMgbm90IGV4aXN0LgAAAAAACE5vdEZvdW5kAAAAAgAAAEhPbmUgb3IgbW9yZSBhcmd1bWVudHMgZmFpbGVkIHZhbGlkYXRpb24gKGUuZy4gemVybyBhbW91bnQsIGJhZCBhZGRyZXNzKS4AAAAMSW52YWxpZElucHV0AAAAAwAAAENUaGUgY29udHJhY3QgZG9lcyBub3QgaG9sZCBlbm91Z2ggYmFsYW5jZSB0byBzYXRpc2Z5IHRoZSBvcGVyYXRpb24uAAAAABFJbnN1ZmZpY2llbnRGdW5kcwAAAAAAAAQAAABCVGhlIGVudGl0eSB3YXMgYWxyZWFkeSBpbml0aWFsaXNlZDsgcmUtaW5pdGlhbGlzYXRpb24gaXMgcmVqZWN0ZWQuAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAAFAAAANlRoZSBlbnRpdHkgd2FzIGV4cGVjdGVkIHRvIGJlIGluaXRpYWxpc2VkIGJ1dCB3YXMgbm90LgAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAGAAAANkFuIG9wZXJhdGlvbiB3YXMgYXR0ZW1wdGVkIGFmdGVyIGl0cyBkZWFkbGluZSBlbGFwc2VkLgAAAAAAD0RlYWRsaW5lUmVhY2hlZAAAAAAHAAAAQUEgcmVxdWlyZWQgdG9rZW4gYWxsb3dhbmNlIHdhcyBsb3dlciB0aGFuIHRoZSBhbW91bnQgYmVpbmcgc3BlbnQuAAAAAAAAFUluc3VmZmljaWVudEFsbG93YW5jZQAAAAAAAAgAAAAjQW4gYXJpdGhtZXRpYyBvcGVyYXRpb24gb3ZlcmZsb3dlZC4AAAAAEkFyaXRobWV0aWNPdmVyZmxvdwAAAAAACQAAAEJDb250cmFjdC1zcGVjaWZpYyBlcnJvciB0aGF0IGRvZXMgbm90IG1hcCB0byB0aGUgY2F0ZWdvcmllcyBhYm92ZS4AAAAAAAZDdXN0b20AAAAAAAoAAAFsQSBTRVAtNDEgdG9rZW4gaW52b2NhdGlvbiBmYWlsZWQgKGluc3VmZmljaWVudCBiYWxhbmNlLCBtaXNzaW5nIG9yCmRlYXV0aG9yaXplZCB0cnVzdGxpbmUsIHVuZGVwbG95ZWQgdG9rZW4gY29udHJhY3QsIG9yIHRva2VuLWxvZ2ljCnJlamVjdGlvbikuIFRoZSByYXcgdG9rZW4gZXJyb3IgZGlzY3JpbWluYW50IGlzIGludGVudGlvbmFsbHkgbm90CmZvcndhcmRlZCDigJQgY2FsbGVycyBjYW5ub3QgdGVsbCB3aGljaCBjb250cmFjdCBwcm9kdWNlZCBhIGZvcndhcmRlZApjb2RlLCBzbyBpdCBpcyBidWNrZXRlZDsgdGhlIHJvb3QgY2F1c2UgcmVtYWlucyB2aXNpYmxlIGluIHRoZQp0cmFuc2FjdGlvbidzIGRpYWdub3N0aWMgZXZlbnRzLgAAABNUb2tlblRyYW5zZmVyRmFpbGVkAAAAAAsAAADbQSBjcm9zcy1jb250cmFjdCBpbnZvY2F0aW9uIGZhaWxlZCAodGFyZ2V0IHJldmVydGVkIG9yIGhvc3QKYWJvcnQpLiBUaGUgdGFyZ2V0IHJldmVydHMgYXJlIHN1cmZhY2VkIGhlcmUgc28gdGhlIGNhbGxlciBjYW4KZGlzdGluZ3Vpc2ggdGhlbSBmcm9tIHRva2VuLXRyYW5zZmVyIGZhaWx1cmVzLCBhbmQgdGhlIGludm9raW5nCnRyYW5zYWN0aW9uIGlzIGxlZnQgdW4tZXhlY3V0ZWQuAAAAABhDb250cmFjdEludm9jYXRpb25GYWlsZWQAAAAMAAABDFRoZSByZXF1ZXN0ZWQgd2l0aGRyYXdhbCB3b3VsZCBwdXNoIGEgdG9rZW4ncyByb2xsaW5nLXdpbmRvdyB0b3RhbApwYXN0IGl0cyBjb25maWd1cmVkIHdpdGhkcmF3YWwgbGltaXQuIEtlcHQgZGlzdGluY3QgZnJvbQpbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIHNvIGEgY2FsbGVyIGNhbiB0ZWxsIGEgcG9saWN5IHJlamVjdGlvbgooYSB2YWxpZCB3aXRoZHJhd2FsIHRoYXQgaXMgdG9vIGxhcmdlIHJpZ2h0IG5vdykgZnJvbSBhIG1hbGZvcm1lZAphcmd1bWVudC4AAAAXV2l0aGRyYXdhbExpbWl0RXhjZWVkZWQAAAAADQAAARtBIHN1YnNjcmlwdGlvbiBpcyBpbiB0aGUgYFBhc3REdWVgIGxhcHNlZCBzdGF0ZSBhbmQgdGhlIHJlcXVlc3RlZApvcGVyYXRpb24gY2Fubm90IGJlIHBlcmZvcm1lZCB1bnRpbCBhIGNhdGNoLXVwIGNoYXJnZSByZXN0b3JlcyBpdCB0bwpgQWN0aXZlYC4gS2VwdCBkaXN0aW5jdCBmcm9tIFtgRm9yZ2VFcnJvcjo6RGVhZGxpbmVSZWFjaGVkYF0gc28gYSBjYWxsZXIKY2FuIHRlbGwgYSBsYXBzZWQgYmlsbGluZyBzdGF0ZSBmcm9tIGFuIGFyZ3VtZW50IHRoYXQgd2FzIG1lcmVseQp0b28gZWFybHkuAAAAABNTdWJzY3JpcHRpb25QYXN0RHVlAAAAAA4AAABxQSBwcm9wb3NlciBoYXMgcmVhY2hlZCB0aGUgbWF4aW11bSBhbGxvd2VkIGNvbmN1cnJlbnQgYWN0aXZlIHByb3Bvc2FscwpvciBpcyB3aXRoaW4gdGhlIHByb3Bvc2VyIGNvb2xkb3duIHdpbmRvdy4AAAAAAAAQUHJvcG9zZXJDb29sZG93bgAAAA8=",
        "AAAAAQAAAWBBdWRpdCBtZXRhZGF0YSBhdHRhY2hlZCB0byBhIHBlcnNpc3RlZCB2YWx1ZS4KCkNvbnRyYWN0cyBzdG9yZSBkb21haW4gZGF0YSBpbiBTb3JvYmFuIGluc3RhbmNlIHN0b3JhZ2U7IHdyYXBwaW5nIGl0IHdpdGgKdGhpcyByZWNvcmQgbGV0cyBjYWxsZXJzIChhbmQgb2ZmLWNoYWluIGluZGV4ZXJzKSBzZWUgd2hlbiBhIHZhbHVlIHdhcyBsYXN0CndyaXR0ZW4uIFRoZSBwYXlsb2FkIGlzIHN0b3JlZCBhcyBvcGFxdWUgc2VyaWFsaXplZCBieXRlcyBzbyB0aGUgcmVjb3JkIGlzCnZhbGlkIGFzIGEgU29yb2JhbiBjb250cmFjdCB0eXBlIHdpdGhvdXQgY291cGxpbmcgaXQgdG8gYSBjb25jcmV0ZSBkb21haW4KdmFsdWUuAAAAAAAAAAxTdG9yYWdlRW50cnkAAAACAAAAPVVuaXggdGltZXN0YW1wIChzZWNvbmRzKSBhdCB3aGljaCB0aGUgZW50cnkgd2FzIGxhc3Qgd3JpdHRlbi4AAAAAAAAKdXBkYXRlZF9hdAAAAAAABgAAADdUaGUgc3RvcmVkIHBheWxvYWQsIHNlcmlhbGl6ZWQgYXMgb3BhcXVlIFNvcm9iYW4gYnl0ZXMuAAAAAAV2YWx1ZQAAAAAAAA4=",
        "AAAAAgAAAF9Db250cmFjdCBleGVjdXRhYmxlIHVzZWQgZm9yIGNyZWF0aW5nIGEgbmV3IGNvbnRyYWN0IGFuZCB1c2VkIGluCmBDcmVhdGVDb250cmFjdEhvc3RGbkNvbnRleHRgLgAAAAAAAAAAEkNvbnRyYWN0RXhlY3V0YWJsZQAAAAAAAgAAAAEAAAB4RXhlY3V0YWJsZSBzcGVjaWZpZWQgYnkgdGhlIGNvbnRyYWN0IGluc3RhbmNlIGFzIGEgc3BlY2lmaWMgV2FzbSBjb250cmFjdCBjb2RlIGVudHJ5IGlkZW50aWZpZWQgYnkgaXRzIFdhc20gc2hhMjU2IGhhc2guAAAABFdhc20AAAABAAAD7gAAACAAAAABAAAAX0V4ZWN1dGFibGUgcmVmZXJlbmNlIHZpYSBhIHBlcnNpc3RlbnQgc3RvcmFnZSBlbnRyeSBvd25lZCBieSB0aGlzIGNvbnRyYWN0IG9yIGFub3RoZXIgY29udHJhY3QuAAAAAAtFeHRlcm5hbFJlZgAAAAABAAAH0AAAABVDb250cmFjdEV4ZWN1dGFibGVSZWYAAAA=",
        "AAAAAQAAAMBFeGVjdXRhYmxlIHJlZmVyZW5jZWQgdmlhIGEgcGVyc2lzdGVudCBzdG9yYWdlIGVudHJ5IG93bmVkIGJ5IGEgY29udHJhY3QsCmVpdGhlciB0aGlzIGNvbnRyYWN0IG9yIGFub3RoZXIgY29udHJhY3QuCgpUaGUgcGVyc2lzdGVudCBzdG9yYWdlIGVudHJ5IG93bmVkIGJ5IHRoZSBgb3duZXJgIGhhcyB0aGUgYHRhZ2AgYXMgaXRzIGtleS4AAAAAAAAAFUNvbnRyYWN0RXhlY3V0YWJsZVJlZgAAAAAAAAIAAAAAAAAABW93bmVyAAAAAAAAEwAAAAAAAAADdGFnAAAAABA=",
        "AAAAAgAAAONDb250ZXh0IG9mIGEgc2luZ2xlIGF1dGhvcml6ZWQgY2FsbCBwZXJmb3JtZWQgYnkgYW4gYWRkcmVzcy4KCkN1c3RvbSBhY2NvdW50IGNvbnRyYWN0cyB0aGF0IGltcGxlbWVudCBgX19jaGVja19hdXRoYCBzcGVjaWFsIGZ1bmN0aW9uCnJlY2VpdmUgYSBsaXN0IG9mIGBDb250ZXh0YCB2YWx1ZXMgY29ycmVzcG9uZGluZyB0byBhbGwgdGhlIGNhbGxzIHRoYXQKbmVlZCB0byBiZSBhdXRob3JpemVkLgAAAAAAAAAAB0NvbnRleHQAAAAAAwAAAAEAAAAUQ29udHJhY3QgaW52b2NhdGlvbi4AAAAIQ29udHJhY3QAAAABAAAH0AAAAA9Db250cmFjdENvbnRleHQAAAAAAQAAAD1Db250cmFjdCB0aGF0IGhhcyBhIGNvbnN0cnVjdG9yIHdpdGggbm8gYXJndW1lbnRzIGlzIGNyZWF0ZWQuAAAAAAAAFENyZWF0ZUNvbnRyYWN0SG9zdEZuAAAAAQAAB9AAAAAbQ3JlYXRlQ29udHJhY3RIb3N0Rm5Db250ZXh0AAAAAAEAAABEQ29udHJhY3QgdGhhdCBoYXMgYSBjb25zdHJ1Y3RvciB3aXRoIDEgb3IgbW9yZSBhcmd1bWVudHMgaXMgY3JlYXRlZC4AAAAcQ3JlYXRlQ29udHJhY3RXaXRoQ3Rvckhvc3RGbgAAAAEAAAfQAAAAKkNyZWF0ZUNvbnRyYWN0V2l0aENvbnN0cnVjdG9ySG9zdEZuQ29udGV4dAAA",
        "AAAAAQAAAL1BdXRob3JpemF0aW9uIGNvbnRleHQgb2YgYSBzaW5nbGUgY29udHJhY3QgY2FsbC4KClRoaXMgc3RydWN0IGNvcnJlc3BvbmRzIHRvIGEgYHJlcXVpcmVfYXV0aF9mb3JfYXJnc2AgY2FsbCBmb3IgYW4gYWRkcmVzcwpmcm9tIGBjb250cmFjdGAgZnVuY3Rpb24gd2l0aCBgZm5fbmFtZWAgbmFtZSBhbmQgYGFyZ3NgIGFyZ3VtZW50cy4AAAAAAAAAAAAAD0NvbnRyYWN0Q29udGV4dAAAAAADAAAAAAAAAARhcmdzAAAD6gAAAAAAAAAAAAAACGNvbnRyYWN0AAAAEwAAAAAAAAAHZm5fbmFtZQAAAAAR",
        "AAAAAQAAADhWYWx1ZSBvZiBjb250cmFjdCBub2RlIGluIEludm9rZXJDb250cmFjdEF1dGhFbnRyeSB0cmVlLgAAAAAAAAAVU3ViQ29udHJhY3RJbnZvY2F0aW9uAAAAAAAAAgAAAAAAAAAHY29udGV4dAAAAAfQAAAAD0NvbnRyYWN0Q29udGV4dAAAAAAAAAAAD3N1Yl9pbnZvY2F0aW9ucwAAAAPqAAAH0AAAABhJbnZva2VyQ29udHJhY3RBdXRoRW50cnk=",
        "AAAAAgAAAS9BIG5vZGUgaW4gdGhlIHRyZWUgb2YgYXV0aG9yaXphdGlvbnMgcGVyZm9ybWVkIG9uIGJlaGFsZiBvZiB0aGUgY3VycmVudApjb250cmFjdCBhcyBpbnZva2VyIG9mIHRoZSBjb250cmFjdHMgZGVlcGVyIGluIHRoZSBjYWxsIHN0YWNrLgoKVGhpcyBpcyB1c2VkIGFzIGFuIGFyZ3VtZW50IG9mIGBhdXRob3JpemVfYXNfY3VycmVudF9jb250cmFjdGAgaG9zdCBmdW5jdGlvbi4KClRoaXMgdHJlZSBjb3JyZXNwb25kcyBgcmVxdWlyZV9hdXRoW19mb3JfYXJnc11gIGNhbGxzIG9uIGJlaGFsZiBvZiB0aGUKY3VycmVudCBjb250cmFjdC4AAAAAAAAAABhJbnZva2VyQ29udHJhY3RBdXRoRW50cnkAAAADAAAAAQAAABJJbnZva2UgYSBjb250cmFjdC4AAAAAAAhDb250cmFjdAAAAAEAAAfQAAAAFVN1YkNvbnRyYWN0SW52b2NhdGlvbgAAAAAAAAEAAAA1Q3JlYXRlIGEgY29udHJhY3QgcGFzc2luZyAwIGFyZ3VtZW50cyB0byBjb25zdHJ1Y3Rvci4AAAAAAAAUQ3JlYXRlQ29udHJhY3RIb3N0Rm4AAAABAAAH0AAAABtDcmVhdGVDb250cmFjdEhvc3RGbkNvbnRleHQAAAAAAQAAAD1DcmVhdGUgYSBjb250cmFjdCBwYXNzaW5nIDAgb3IgbW9yZSBhcmd1bWVudHMgdG8gY29uc3RydWN0b3IuAAAAAAAAHENyZWF0ZUNvbnRyYWN0V2l0aEN0b3JIb3N0Rm4AAAABAAAH0AAAACpDcmVhdGVDb250cmFjdFdpdGhDb25zdHJ1Y3Rvckhvc3RGbkNvbnRleHQAAA==",
        "AAAAAQAAAHZBdXRob3JpemF0aW9uIGNvbnRleHQgZm9yIGBjcmVhdGVfY29udHJhY3RgIGhvc3QgZnVuY3Rpb24gdGhhdCBjcmVhdGVzIGEKbmV3IGNvbnRyYWN0IG9uIGJlaGFsZiBvZiBhdXRob3JpemVyIGFkZHJlc3MuAAAAAAAAAAAAG0NyZWF0ZUNvbnRyYWN0SG9zdEZuQ29udGV4dAAAAAACAAAAAAAAAApleGVjdXRhYmxlAAAAAAfQAAAAEkNvbnRyYWN0RXhlY3V0YWJsZQAAAAAAAAAAAARzYWx0AAAD7gAAACA=",
        "AAAAAQAAANZBdXRob3JpemF0aW9uIGNvbnRleHQgZm9yIGBjcmVhdGVfY29udHJhY3RgIGhvc3QgZnVuY3Rpb24gdGhhdCBjcmVhdGVzIGEKbmV3IGNvbnRyYWN0IG9uIGJlaGFsZiBvZiBhdXRob3JpemVyIGFkZHJlc3MuClRoaXMgaXMgdGhlIHNhbWUgYXMgYENyZWF0ZUNvbnRyYWN0SG9zdEZuQ29udGV4dGAsIGJ1dCBhbHNvIGhhcwpjb250cmFjdCBjb25zdHJ1Y3RvciBhcmd1bWVudHMuAAAAAAAAAAAAKkNyZWF0ZUNvbnRyYWN0V2l0aENvbnN0cnVjdG9ySG9zdEZuQ29udGV4dAAAAAAAAwAAAAAAAAAQY29uc3RydWN0b3JfYXJncwAAA+oAAAAAAAAAAAAAAApleGVjdXRhYmxlAAAAAAfQAAAAEkNvbnRyYWN0RXhlY3V0YWJsZQAAAAAAAAAAAARzYWx0AAAD7gAAACA=",
        "AAAAAgAAAAAAAAAAAAAACkV4ZWN1dGFibGUAAAAAAAMAAAABAAAAAAAAAARXYXNtAAAAAQAAA+4AAAAgAAAAAAAAAAAAAAAMU3RlbGxhckFzc2V0AAAAAAAAAAAAAAAHQWNjb3VudAA=" ]),
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
        request_release: this.txFromJSON<Result<void>>,
        get_basket_summary: this.txFromJSON<Result<BasketSettlementSummary>>,
        expire_release_request: this.txFromJSON<Result<void>>,
        release_partial_basket: this.txFromJSON<Result<void>>,
        approve_release_request: this.txFromJSON<Result<void>>,
        escrows_for_participant: this.txFromJSON<ParticipantEscrowsPage>
  }
}