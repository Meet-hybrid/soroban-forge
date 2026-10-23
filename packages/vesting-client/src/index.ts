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
 * One entry of a tranche schedule's unlock table: an amount that becomes
 * claimable at a fixed offset from the schedule start.
 *
 * A grant agreement's "25% at TGE, 25% at +6 months, 50% at +12 months"
 * is three of these, not a ramp.
 */
export interface Tranche {
  /**
 * Amount that unlocks at `unlock_at`; must be positive.
 */
amount: i128;
  /**
 * Seconds after `start` at which this tranche unlocks. Compared as an
 * offset, so `u64::MAX` is a valid (never-reached) boundary rather than
 * an overflow.
 */
unlock_at: u64;
}

/**
 * Lifecycle state of a vesting schedule.
 */
export type VestingStatus = {tag: "Locked", values: void} | {tag: "Vesting", values: void} | {tag: "Completed", values: void} | {tag: "Revoked", values: void};


/**
 * A vesting schedule whose unlock table is an explicit list of tranches
 * instead of a cliff-plus-ramp.
 *
 * A separate record from [`VestingSchedule`] on purpose: the linear record's
 * wire shape stays untouched, and the two kinds sit under distinct
 * `DataKey` variants while sharing one monotonic id counter.
 */
export interface TrancheSchedule {
  /**
 * Recipient of the unlocked tokens.
 */
beneficiary: string;
  /**
 * Amount already claimed by the beneficiary.
 */
claimed: i128;
  /**
 * Ledger timestamp the tranche offsets are measured from (creation
 * time).
 */
start: u64;
  /**
 * Current lifecycle state.
 */
status: VestingStatus;
  /**
 * Token contract whose balance is drawn down.
 */
token: string;
  /**
 * Sum of every tranche amount; the amount unlocked at the last unlock.
 */
total_amount: i128;
  /**
 * The immutable unlock table, ordered by strictly increasing
 * `unlock_at`. Non-empty and at most [`MAX_TRANCHES`] long.
 */
tranches: Array<Tranche>;
}


/**
 * A single token-vesting schedule.
 */
export interface VestingSchedule {
  /**
 * Recipient of the vested tokens.
 */
beneficiary: string;
  /**
 * Amount claimed by the current beneficiary during this assignment.
 */
beneficiary_claimed: i128;
  /**
 * Total amount already paid across all beneficiaries.
 */
claimed: i128;
  /**
 * Seconds after `start` at which claims become possible.
 */
cliff: u64;
  /**
 * Seconds after `start` at which the schedule is fully vested.
 */
duration: u64;
  /**
 * Party that funded the grant; the only address that may `revoke`.
 */
funder: string;
  /**
 * Number of successful beneficiary reassignments.
 */
reassignment_count: u32;
  /**
 * Amount vested when the current beneficiary assignment began.
 */
reassignment_vested: i128;
  /**
 * Vested amount frozen at revocation (`Some` exactly when `status` is
 * `Revoked`); `claim` pays only up to it.
 */
revoked_vested: Option<i128>;
  /**
 * Ledger timestamp at which vesting begins (creation time).
 */
start: u64;
  /**
 * Current lifecycle state.
 */
status: VestingStatus;
  /**
 * Token contract whose balance is drawn down.
 */
token: string;
  /**
 * Total amount to vest linearly between `cliff` and `duration`.
 */
total_amount: i128;
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
   * Construct and simulate a claim transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Claim the vested-but-unclaimed amount.
   *
   * Requires the beneficiary. Works for both schedule kinds — the id
   * resolves to a linear or a tranche record and the matching math runs.
   * Returns exactly what vested since the last claim (or `0` when nothing
   * is claimable), so repeated claims can never overpay or underpay. On a
   * `Revoked` linear schedule only the amount frozen at revocation is
   * payable; a drained revoked schedule claims `0` with no transfer.
   *
   * Ordering: the SEP-41 transfer runs **before** the schedule write —
   * see the module docs. A zero-claim call returns before either.
   */
  claim: ({schedule_id}: {schedule_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a revoke transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Revoke a linear schedule before completion and permanently stop
   * further vesting.
   *
   * Transitions `Locked | Vesting -> Revoked` and freezes the vested
   * amount at this ledger timestamp into `revoked_vested`: `claim`
   * afterwards succeeds only up to that amount, and `claimable` returns
   * `0` once it is claimed. Pure state-machine logic — no tokens move at
   * revocation. Requires the `funder` recorded at creation; the
   * beneficiary cannot revoke.
   *
   * # Errors
   *
   * * [`ForgeError::NotFound`] — no linear schedule with this id (a
   * tranche id is `NotFound` here).
   * * [`ForgeError::InvalidInput`] — the schedule is `Completed` or
   * already `Revoked`; double-revoke is impossible.
   * * [`ForgeError::Unauthorized`] — the funder authorization is missing
   * or invalid (a non-funder caller is rejected at the host).
   */
  revoke: ({schedule_id}: {schedule_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a claim_for transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Claim for an explicit beneficiary, including a frozen former balance.
   */
  claim_for: ({schedule_id, beneficiary}: {schedule_id: u64, beneficiary: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a claimable transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Amount currently claimable (read-only view; no state change).
   *
   * On a `Revoked` linear schedule this is the frozen vested amount minus
   * what has been claimed — `0` once that amount is claimed.
   */
  claimable: ({schedule_id}: {schedule_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a get_status transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the current lifecycle status (read-only view).
   *
   * The status is derived from the ledger time and claimed amount rather
   * than the stored field, so it is always current between claims. A
   * revoked schedule reports `Revoked` regardless of ledger time.
   */
  get_status: ({schedule_id}: {schedule_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<VestingStatus>>>

  /**
   * Construct and simulate a get_schedule transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the full linear schedule record (read-only view; no state
   * change).
   *
   * Mirrors `get_tranche_schedule`: the stored record, unchanged. A
   * tranche id is `NotFound` here; use `get_tranche_schedule` for those.
   */
  get_schedule: ({schedule_id}: {schedule_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<VestingSchedule>>>

  /**
   * Construct and simulate a claimable_for transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Amount currently claimable by the explicit beneficiary.
   */
  claimable_for: ({schedule_id, beneficiary}: {schedule_id: u64, beneficiary: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a create_schedule transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new vesting schedule and return its stable id.
   *
   * Requires `total_amount > 0`, `duration > 0`, `cliff <= duration`, and
   * `funder != beneficiary`. The beneficiary is authorized at creation
   * time; `funder` is recorded without authorizing (mirroring escrow's
   * non-consenting parties) and is the only party that may later revoke or
   * reassign the schedule.
   */
  create_schedule: ({funder, beneficiary, token, total_amount, cliff, duration}: {funder: string, beneficiary: string, token: string, total_amount: i128, cliff: u64, duration: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a get_tranche_schedule transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the full tranche schedule record, unlock table included
   * (read-only view; no state change).
   */
  get_tranche_schedule: ({schedule_id}: {schedule_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<TrancheSchedule>>>

  /**
   * Construct and simulate a reassign_beneficiary transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Reassign only a linear schedule's unvested balance.
   *
   * The old beneficiary's vested-but-unclaimed amount is frozen under a
   * schedule-scoped claim key; cliff and duration remain unchanged.
   *
   * # Errors
   *
   * * [`ForgeError::NotFound`] — no linear schedule with this id.
   * * [`ForgeError::InvalidInput`] — the schedule is completed or revoked,
   * or the new beneficiary is already the current beneficiary or funder.
   * * [`ForgeError::Unauthorized`] — the funder's authorization is missing
   * or invalid.
   */
  reassign_beneficiary: ({schedule_id, new_beneficiary}: {schedule_id: u64, new_beneficiary: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a create_tranche_schedule transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new tranche vesting schedule and return its stable id.
   *
   * `tranches` is the unlock table, ordered by strictly increasing
   * `unlock_at` offsets in seconds from the creation timestamp. Requires a
   * non-empty table of at most [`MAX_TRANCHES`] entries, every
   * `amount > 0`, and a cumulative sum that fits in `i128`; the table is
   * then stored once and treated as immutable. The beneficiary is
   * authorized at creation time, mirroring `create_schedule`.
   *
   * A first tranche at `unlock_at == 0` is allowed: it unlocks at the
   * creation timestamp, the "TGE tranche" of a typical grant agreement.
   */
  create_tranche_schedule: ({beneficiary, token, tranches}: {beneficiary: string, token: string, tranches: Array<Tranche>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

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
      new ContractSpec([ "AAAAAAAAAkJDbGFpbSB0aGUgdmVzdGVkLWJ1dC11bmNsYWltZWQgYW1vdW50LgoKUmVxdWlyZXMgdGhlIGJlbmVmaWNpYXJ5LiBXb3JrcyBmb3IgYm90aCBzY2hlZHVsZSBraW5kcyDigJQgdGhlIGlkCnJlc29sdmVzIHRvIGEgbGluZWFyIG9yIGEgdHJhbmNoZSByZWNvcmQgYW5kIHRoZSBtYXRjaGluZyBtYXRoIHJ1bnMuClJldHVybnMgZXhhY3RseSB3aGF0IHZlc3RlZCBzaW5jZSB0aGUgbGFzdCBjbGFpbSAob3IgYDBgIHdoZW4gbm90aGluZwppcyBjbGFpbWFibGUpLCBzbyByZXBlYXRlZCBjbGFpbXMgY2FuIG5ldmVyIG92ZXJwYXkgb3IgdW5kZXJwYXkuIE9uIGEKYFJldm9rZWRgIGxpbmVhciBzY2hlZHVsZSBvbmx5IHRoZSBhbW91bnQgZnJvemVuIGF0IHJldm9jYXRpb24gaXMKcGF5YWJsZTsgYSBkcmFpbmVkIHJldm9rZWQgc2NoZWR1bGUgY2xhaW1zIGAwYCB3aXRoIG5vIHRyYW5zZmVyLgoKT3JkZXJpbmc6IHRoZSBTRVAtNDEgdHJhbnNmZXIgcnVucyAqKmJlZm9yZSoqIHRoZSBzY2hlZHVsZSB3cml0ZSDigJQKc2VlIHRoZSBtb2R1bGUgZG9jcy4gQSB6ZXJvLWNsYWltIGNhbGwgcmV0dXJucyBiZWZvcmUgZWl0aGVyLgAAAAAABWNsYWltAAAAAAAAAQAAAAAAAAALc2NoZWR1bGVfaWQAAAAABgAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAQAAAOFPbmUgZW50cnkgb2YgYSB0cmFuY2hlIHNjaGVkdWxlJ3MgdW5sb2NrIHRhYmxlOiBhbiBhbW91bnQgdGhhdCBiZWNvbWVzCmNsYWltYWJsZSBhdCBhIGZpeGVkIG9mZnNldCBmcm9tIHRoZSBzY2hlZHVsZSBzdGFydC4KCkEgZ3JhbnQgYWdyZWVtZW50J3MgIjI1JSBhdCBUR0UsIDI1JSBhdCArNiBtb250aHMsIDUwJSBhdCArMTIgbW9udGhzIgppcyB0aHJlZSBvZiB0aGVzZSwgbm90IGEgcmFtcC4AAAAAAAAAAAAAB1RyYW5jaGUAAAAAAgAAADVBbW91bnQgdGhhdCB1bmxvY2tzIGF0IGB1bmxvY2tfYXRgOyBtdXN0IGJlIHBvc2l0aXZlLgAAAAAAAAZhbW91bnQAAAAAAAsAAACWU2Vjb25kcyBhZnRlciBgc3RhcnRgIGF0IHdoaWNoIHRoaXMgdHJhbmNoZSB1bmxvY2tzLiBDb21wYXJlZCBhcyBhbgpvZmZzZXQsIHNvIGB1NjQ6Ok1BWGAgaXMgYSB2YWxpZCAobmV2ZXItcmVhY2hlZCkgYm91bmRhcnkgcmF0aGVyIHRoYW4KYW4gb3ZlcmZsb3cuAAAAAAAJdW5sb2NrX2F0AAAAAAAABg==",
        "AAAAAAAAAxNSZXZva2UgYSBsaW5lYXIgc2NoZWR1bGUgYmVmb3JlIGNvbXBsZXRpb24gYW5kIHBlcm1hbmVudGx5IHN0b3AKZnVydGhlciB2ZXN0aW5nLgoKVHJhbnNpdGlvbnMgYExvY2tlZCB8IFZlc3RpbmcgLT4gUmV2b2tlZGAgYW5kIGZyZWV6ZXMgdGhlIHZlc3RlZAphbW91bnQgYXQgdGhpcyBsZWRnZXIgdGltZXN0YW1wIGludG8gYHJldm9rZWRfdmVzdGVkYDogYGNsYWltYAphZnRlcndhcmRzIHN1Y2NlZWRzIG9ubHkgdXAgdG8gdGhhdCBhbW91bnQsIGFuZCBgY2xhaW1hYmxlYCByZXR1cm5zCmAwYCBvbmNlIGl0IGlzIGNsYWltZWQuIFB1cmUgc3RhdGUtbWFjaGluZSBsb2dpYyDigJQgbm8gdG9rZW5zIG1vdmUgYXQKcmV2b2NhdGlvbi4gUmVxdWlyZXMgdGhlIGBmdW5kZXJgIHJlY29yZGVkIGF0IGNyZWF0aW9uOyB0aGUKYmVuZWZpY2lhcnkgY2Fubm90IHJldm9rZS4KCiMgRXJyb3JzCgoqIFtgRm9yZ2VFcnJvcjo6Tm90Rm91bmRgXSDigJQgbm8gbGluZWFyIHNjaGVkdWxlIHdpdGggdGhpcyBpZCAoYQp0cmFuY2hlIGlkIGlzIGBOb3RGb3VuZGAgaGVyZSkuCiogW2BGb3JnZUVycm9yOjpJbnZhbGlkSW5wdXRgXSDigJQgdGhlIHNjaGVkdWxlIGlzIGBDb21wbGV0ZWRgIG9yCmFscmVhZHkgYFJldm9rZWRgOyBkb3VibGUtcmV2b2tlIGlzIGltcG9zc2libGUuCiogW2BGb3JnZUVycm9yOjpVbmF1dGhvcml6ZWRgXSDigJQgdGhlIGZ1bmRlciBhdXRob3JpemF0aW9uIGlzIG1pc3NpbmcKb3IgaW52YWxpZCAoYSBub24tZnVuZGVyIGNhbGxlciBpcyByZWplY3RlZCBhdCB0aGUgaG9zdCkuAAAAAAZyZXZva2UAAAAAAAEAAAAAAAAAC3NjaGVkdWxlX2lkAAAAAAYAAAABAAAD6QAAAAIAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAEVDbGFpbSBmb3IgYW4gZXhwbGljaXQgYmVuZWZpY2lhcnksIGluY2x1ZGluZyBhIGZyb3plbiBmb3JtZXIgYmFsYW5jZS4AAAAAAAAJY2xhaW1fZm9yAAAAAAAAAgAAAAAAAAALc2NoZWR1bGVfaWQAAAAABgAAAAAAAAALYmVuZWZpY2lhcnkAAAAAEwAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAL9BbW91bnQgY3VycmVudGx5IGNsYWltYWJsZSAocmVhZC1vbmx5IHZpZXc7IG5vIHN0YXRlIGNoYW5nZSkuCgpPbiBhIGBSZXZva2VkYCBsaW5lYXIgc2NoZWR1bGUgdGhpcyBpcyB0aGUgZnJvemVuIHZlc3RlZCBhbW91bnQgbWludXMKd2hhdCBoYXMgYmVlbiBjbGFpbWVkIOKAlCBgMGAgb25jZSB0aGF0IGFtb3VudCBpcyBjbGFpbWVkLgAAAAAJY2xhaW1hYmxlAAAAAAAAAQAAAAAAAAALc2NoZWR1bGVfaWQAAAAABgAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAPhSZWFkIHRoZSBjdXJyZW50IGxpZmVjeWNsZSBzdGF0dXMgKHJlYWQtb25seSB2aWV3KS4KClRoZSBzdGF0dXMgaXMgZGVyaXZlZCBmcm9tIHRoZSBsZWRnZXIgdGltZSBhbmQgY2xhaW1lZCBhbW91bnQgcmF0aGVyCnRoYW4gdGhlIHN0b3JlZCBmaWVsZCwgc28gaXQgaXMgYWx3YXlzIGN1cnJlbnQgYmV0d2VlbiBjbGFpbXMuIEEKcmV2b2tlZCBzY2hlZHVsZSByZXBvcnRzIGBSZXZva2VkYCByZWdhcmRsZXNzIG9mIGxlZGdlciB0aW1lLgAAAApnZXRfc3RhdHVzAAAAAAABAAAAAAAAAAtzY2hlZHVsZV9pZAAAAAAGAAAAAQAAA+kAAAfQAAAADVZlc3RpbmdTdGF0dXMAAAAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAgAAACZMaWZlY3ljbGUgc3RhdGUgb2YgYSB2ZXN0aW5nIHNjaGVkdWxlLgAAAAAAAAAAAA1WZXN0aW5nU3RhdHVzAAAAAAAABAAAAAAAAAAiQmVmb3JlIHRoZSBjbGlmZiBoYXMgYmVlbiByZWFjaGVkLgAAAAAABkxvY2tlZAAAAAAAAAAAACxQYXN0IHRoZSBjbGlmZjsgdG9rZW5zIGFyZSB2ZXN0aW5nIGxpbmVhcmx5LgAAAAdWZXN0aW5nAAAAAAAAAAAZRnVsbHkgdmVzdGVkIGFuZCBjbGFpbWVkLgAAAAAAAAlDb21wbGV0ZWQAAAAAAAAAAAAAcVNjaGVkdWxlIHdhcyB0ZXJtaW5hdGVkIGJ5IHRoZSBmdW5kZXIgYmVmb3JlIGNvbXBsZXRpb247IHRoZSB2ZXN0ZWQKYW1vdW50IGF0IHRoZSByZXZvY2F0aW9uIHRpbWVzdGFtcCBpcyBmcm96ZW4uAAAAAAAAB1Jldm9rZWQA",
        "AAAAAAAAAM1SZWFkIHRoZSBmdWxsIGxpbmVhciBzY2hlZHVsZSByZWNvcmQgKHJlYWQtb25seSB2aWV3OyBubyBzdGF0ZQpjaGFuZ2UpLgoKTWlycm9ycyBgZ2V0X3RyYW5jaGVfc2NoZWR1bGVgOiB0aGUgc3RvcmVkIHJlY29yZCwgdW5jaGFuZ2VkLiBBCnRyYW5jaGUgaWQgaXMgYE5vdEZvdW5kYCBoZXJlOyB1c2UgYGdldF90cmFuY2hlX3NjaGVkdWxlYCBmb3IgdGhvc2UuAAAAAAAADGdldF9zY2hlZHVsZQAAAAEAAAAAAAAAC3NjaGVkdWxlX2lkAAAAAAYAAAABAAAD6QAAB9AAAAAPVmVzdGluZ1NjaGVkdWxlAAAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAADdBbW91bnQgY3VycmVudGx5IGNsYWltYWJsZSBieSB0aGUgZXhwbGljaXQgYmVuZWZpY2lhcnkuAAAAAA1jbGFpbWFibGVfZm9yAAAAAAAAAgAAAAAAAAALc2NoZWR1bGVfaWQAAAAABgAAAAAAAAALYmVuZWZpY2lhcnkAAAAAEwAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAQAAAStBIHZlc3Rpbmcgc2NoZWR1bGUgd2hvc2UgdW5sb2NrIHRhYmxlIGlzIGFuIGV4cGxpY2l0IGxpc3Qgb2YgdHJhbmNoZXMKaW5zdGVhZCBvZiBhIGNsaWZmLXBsdXMtcmFtcC4KCkEgc2VwYXJhdGUgcmVjb3JkIGZyb20gW2BWZXN0aW5nU2NoZWR1bGVgXSBvbiBwdXJwb3NlOiB0aGUgbGluZWFyIHJlY29yZCdzCndpcmUgc2hhcGUgc3RheXMgdW50b3VjaGVkLCBhbmQgdGhlIHR3byBraW5kcyBzaXQgdW5kZXIgZGlzdGluY3QKYERhdGFLZXlgIHZhcmlhbnRzIHdoaWxlIHNoYXJpbmcgb25lIG1vbm90b25pYyBpZCBjb3VudGVyLgAAAAAAAAAAD1RyYW5jaGVTY2hlZHVsZQAAAAAHAAAAIVJlY2lwaWVudCBvZiB0aGUgdW5sb2NrZWQgdG9rZW5zLgAAAAAAAAtiZW5lZmljaWFyeQAAAAATAAAAKkFtb3VudCBhbHJlYWR5IGNsYWltZWQgYnkgdGhlIGJlbmVmaWNpYXJ5LgAAAAAAB2NsYWltZWQAAAAACwAAAEdMZWRnZXIgdGltZXN0YW1wIHRoZSB0cmFuY2hlIG9mZnNldHMgYXJlIG1lYXN1cmVkIGZyb20gKGNyZWF0aW9uCnRpbWUpLgAAAAAFc3RhcnQAAAAAAAAGAAAAGEN1cnJlbnQgbGlmZWN5Y2xlIHN0YXRlLgAAAAZzdGF0dXMAAAAAB9AAAAANVmVzdGluZ1N0YXR1cwAAAAAAACtUb2tlbiBjb250cmFjdCB3aG9zZSBiYWxhbmNlIGlzIGRyYXduIGRvd24uAAAAAAV0b2tlbgAAAAAAABMAAABEU3VtIG9mIGV2ZXJ5IHRyYW5jaGUgYW1vdW50OyB0aGUgYW1vdW50IHVubG9ja2VkIGF0IHRoZSBsYXN0IHVubG9jay4AAAAMdG90YWxfYW1vdW50AAAACwAAAHRUaGUgaW1tdXRhYmxlIHVubG9jayB0YWJsZSwgb3JkZXJlZCBieSBzdHJpY3RseSBpbmNyZWFzaW5nCmB1bmxvY2tfYXRgLiBOb24tZW1wdHkgYW5kIGF0IG1vc3QgW2BNQVhfVFJBTkNIRVNgXSBsb25nLgAAAAh0cmFuY2hlcwAAA+oAAAfQAAAAB1RyYW5jaGUA",
        "AAAAAQAAACBBIHNpbmdsZSB0b2tlbi12ZXN0aW5nIHNjaGVkdWxlLgAAAAAAAAAPVmVzdGluZ1NjaGVkdWxlAAAAAA0AAAAfUmVjaXBpZW50IG9mIHRoZSB2ZXN0ZWQgdG9rZW5zLgAAAAALYmVuZWZpY2lhcnkAAAAAEwAAAEFBbW91bnQgY2xhaW1lZCBieSB0aGUgY3VycmVudCBiZW5lZmljaWFyeSBkdXJpbmcgdGhpcyBhc3NpZ25tZW50LgAAAAAAABNiZW5lZmljaWFyeV9jbGFpbWVkAAAAAAsAAAAzVG90YWwgYW1vdW50IGFscmVhZHkgcGFpZCBhY3Jvc3MgYWxsIGJlbmVmaWNpYXJpZXMuAAAAAAdjbGFpbWVkAAAAAAsAAAA2U2Vjb25kcyBhZnRlciBgc3RhcnRgIGF0IHdoaWNoIGNsYWltcyBiZWNvbWUgcG9zc2libGUuAAAAAAAFY2xpZmYAAAAAAAAGAAAAPFNlY29uZHMgYWZ0ZXIgYHN0YXJ0YCBhdCB3aGljaCB0aGUgc2NoZWR1bGUgaXMgZnVsbHkgdmVzdGVkLgAAAAhkdXJhdGlvbgAAAAYAAABAUGFydHkgdGhhdCBmdW5kZWQgdGhlIGdyYW50OyB0aGUgb25seSBhZGRyZXNzIHRoYXQgbWF5IGByZXZva2VgLgAAAAZmdW5kZXIAAAAAABMAAAAvTnVtYmVyIG9mIHN1Y2Nlc3NmdWwgYmVuZWZpY2lhcnkgcmVhc3NpZ25tZW50cy4AAAAAEnJlYXNzaWdubWVudF9jb3VudAAAAAAABAAAADxBbW91bnQgdmVzdGVkIHdoZW4gdGhlIGN1cnJlbnQgYmVuZWZpY2lhcnkgYXNzaWdubWVudCBiZWdhbi4AAAATcmVhc3NpZ25tZW50X3Zlc3RlZAAAAAALAAAAa1Zlc3RlZCBhbW91bnQgZnJvemVuIGF0IHJldm9jYXRpb24gKGBTb21lYCBleGFjdGx5IHdoZW4gYHN0YXR1c2AgaXMKYFJldm9rZWRgKTsgYGNsYWltYCBwYXlzIG9ubHkgdXAgdG8gaXQuAAAAAA5yZXZva2VkX3Zlc3RlZAAAAAAD6AAAAAsAAAA5TGVkZ2VyIHRpbWVzdGFtcCBhdCB3aGljaCB2ZXN0aW5nIGJlZ2lucyAoY3JlYXRpb24gdGltZSkuAAAAAAAABXN0YXJ0AAAAAAAABgAAABhDdXJyZW50IGxpZmVjeWNsZSBzdGF0ZS4AAAAGc3RhdHVzAAAAAAfQAAAADVZlc3RpbmdTdGF0dXMAAAAAAAArVG9rZW4gY29udHJhY3Qgd2hvc2UgYmFsYW5jZSBpcyBkcmF3biBkb3duLgAAAAAFdG9rZW4AAAAAAAATAAAAPVRvdGFsIGFtb3VudCB0byB2ZXN0IGxpbmVhcmx5IGJldHdlZW4gYGNsaWZmYCBhbmQgYGR1cmF0aW9uYC4AAAAAAAAMdG90YWxfYW1vdW50AAAACw==",
        "AAAAAAAAAWJDcmVhdGUgYSBuZXcgdmVzdGluZyBzY2hlZHVsZSBhbmQgcmV0dXJuIGl0cyBzdGFibGUgaWQuCgpSZXF1aXJlcyBgdG90YWxfYW1vdW50ID4gMGAsIGBkdXJhdGlvbiA+IDBgLCBgY2xpZmYgPD0gZHVyYXRpb25gLCBhbmQKYGZ1bmRlciAhPSBiZW5lZmljaWFyeWAuIFRoZSBiZW5lZmljaWFyeSBpcyBhdXRob3JpemVkIGF0IGNyZWF0aW9uCnRpbWU7IGBmdW5kZXJgIGlzIHJlY29yZGVkIHdpdGhvdXQgYXV0aG9yaXppbmcgKG1pcnJvcmluZyBlc2Nyb3cncwpub24tY29uc2VudGluZyBwYXJ0aWVzKSBhbmQgaXMgdGhlIG9ubHkgcGFydHkgdGhhdCBtYXkgbGF0ZXIgcmV2b2tlIG9yCnJlYXNzaWduIHRoZSBzY2hlZHVsZS4AAAAAAA9jcmVhdGVfc2NoZWR1bGUAAAAABgAAAAAAAAAGZnVuZGVyAAAAAAATAAAAAAAAAAtiZW5lZmljaWFyeQAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAADHRvdGFsX2Ftb3VudAAAAAsAAAAAAAAABWNsaWZmAAAAAAAABgAAAAAAAAAIZHVyYXRpb24AAAAGAAAAAQAAA+kAAAAGAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAF9SZWFkIHRoZSBmdWxsIHRyYW5jaGUgc2NoZWR1bGUgcmVjb3JkLCB1bmxvY2sgdGFibGUgaW5jbHVkZWQKKHJlYWQtb25seSB2aWV3OyBubyBzdGF0ZSBjaGFuZ2UpLgAAAAAUZ2V0X3RyYW5jaGVfc2NoZWR1bGUAAAABAAAAAAAAAAtzY2hlZHVsZV9pZAAAAAAGAAAAAQAAA+kAAAfQAAAAD1RyYW5jaGVTY2hlZHVsZQAAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAeZSZWFzc2lnbiBvbmx5IGEgbGluZWFyIHNjaGVkdWxlJ3MgdW52ZXN0ZWQgYmFsYW5jZS4KClRoZSBvbGQgYmVuZWZpY2lhcnkncyB2ZXN0ZWQtYnV0LXVuY2xhaW1lZCBhbW91bnQgaXMgZnJvemVuIHVuZGVyIGEKc2NoZWR1bGUtc2NvcGVkIGNsYWltIGtleTsgY2xpZmYgYW5kIGR1cmF0aW9uIHJlbWFpbiB1bmNoYW5nZWQuCgojIEVycm9ycwoKKiBbYEZvcmdlRXJyb3I6Ok5vdEZvdW5kYF0g4oCUIG5vIGxpbmVhciBzY2hlZHVsZSB3aXRoIHRoaXMgaWQuCiogW2BGb3JnZUVycm9yOjpJbnZhbGlkSW5wdXRgXSDigJQgdGhlIHNjaGVkdWxlIGlzIGNvbXBsZXRlZCBvciByZXZva2VkLApvciB0aGUgbmV3IGJlbmVmaWNpYXJ5IGlzIGFscmVhZHkgdGhlIGN1cnJlbnQgYmVuZWZpY2lhcnkgb3IgZnVuZGVyLgoqIFtgRm9yZ2VFcnJvcjo6VW5hdXRob3JpemVkYF0g4oCUIHRoZSBmdW5kZXIncyBhdXRob3JpemF0aW9uIGlzIG1pc3NpbmcKb3IgaW52YWxpZC4AAAAAABRyZWFzc2lnbl9iZW5lZmljaWFyeQAAAAIAAAAAAAAAC3NjaGVkdWxlX2lkAAAAAAYAAAAAAAAAD25ld19iZW5lZmljaWFyeQAAAAATAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAkVDcmVhdGUgYSBuZXcgdHJhbmNoZSB2ZXN0aW5nIHNjaGVkdWxlIGFuZCByZXR1cm4gaXRzIHN0YWJsZSBpZC4KCmB0cmFuY2hlc2AgaXMgdGhlIHVubG9jayB0YWJsZSwgb3JkZXJlZCBieSBzdHJpY3RseSBpbmNyZWFzaW5nCmB1bmxvY2tfYXRgIG9mZnNldHMgaW4gc2Vjb25kcyBmcm9tIHRoZSBjcmVhdGlvbiB0aW1lc3RhbXAuIFJlcXVpcmVzIGEKbm9uLWVtcHR5IHRhYmxlIG9mIGF0IG1vc3QgW2BNQVhfVFJBTkNIRVNgXSBlbnRyaWVzLCBldmVyeQpgYW1vdW50ID4gMGAsIGFuZCBhIGN1bXVsYXRpdmUgc3VtIHRoYXQgZml0cyBpbiBgaTEyOGA7IHRoZSB0YWJsZSBpcwp0aGVuIHN0b3JlZCBvbmNlIGFuZCB0cmVhdGVkIGFzIGltbXV0YWJsZS4gVGhlIGJlbmVmaWNpYXJ5IGlzCmF1dGhvcml6ZWQgYXQgY3JlYXRpb24gdGltZSwgbWlycm9yaW5nIGBjcmVhdGVfc2NoZWR1bGVgLgoKQSBmaXJzdCB0cmFuY2hlIGF0IGB1bmxvY2tfYXQgPT0gMGAgaXMgYWxsb3dlZDogaXQgdW5sb2NrcyBhdCB0aGUKY3JlYXRpb24gdGltZXN0YW1wLCB0aGUgIlRHRSB0cmFuY2hlIiBvZiBhIHR5cGljYWwgZ3JhbnQgYWdyZWVtZW50LgAAAAAAABdjcmVhdGVfdHJhbmNoZV9zY2hlZHVsZQAAAAADAAAAAAAAAAtiZW5lZmljaWFyeQAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAACHRyYW5jaGVzAAAD6gAAB9AAAAAHVHJhbmNoZQAAAAABAAAD6QAAAAYAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAABQAAAAAAAAAAAAAAF0JlbmVmaWNpYXJ5UmVhc3NpZ25lZFRvAAAAAAEAAAAZYmVuZWZpY2lhcnlfcmVhc3NpZ25lZF90bwAAAAAAAAUAAAAAAAAAC3NjaGVkdWxlX2lkAAAAAAYAAAABAAAAAAAAAAtiZW5lZmljaWFyeQAAAAATAAAAAQAAAAAAAAAPb2xkX2JlbmVmaWNpYXJ5AAAAABMAAAAAAAAAAAAAABB2ZXN0ZWRfdW5jbGFpbWVkAAAACwAAAAAAAAAAAAAAEnJlYXNzaWdubWVudF9jb3VudAAAAAAABAAAAAAAAAAC",
        "AAAABQAAAAAAAAAAAAAAGUJlbmVmaWNpYXJ5UmVhc3NpZ25lZEZyb20AAAAAAAABAAAAG2JlbmVmaWNpYXJ5X3JlYXNzaWduZWRfZnJvbQAAAAAFAAAAAAAAAAtzY2hlZHVsZV9pZAAAAAAGAAAAAQAAAAAAAAALYmVuZWZpY2lhcnkAAAAAEwAAAAEAAAAAAAAAD25ld19iZW5lZmljaWFyeQAAAAATAAAAAAAAAAAAAAAQdmVzdGVkX3VuY2xhaW1lZAAAAAsAAAAAAAAAAAAAABJyZWFzc2lnbm1lbnRfY291bnQAAAAAAAQAAAAAAAAAAg==",
        "AAAABAAAAZxTaGFyZWQgZXJyb3IgdHlwZSB1c2VkIGFjcm9zcyBhbGwgU29yb2JhbiBGb3JnZSBjb250cmFjdHMuCgpEZWZpbmluZyBhIHNpbmdsZSBlcnJvciBlbnVtIGluIGBzaGFyZWQtdXRpbHNgIGtlZXBzIHRoZSBvbi1jaGFpbiBlcnJvcgpzcGFjZSBjb25zaXN0ZW50IGFuZCBpbnRlbGxpZ2libGUgdG8gU0RLIGNvbnN1bWVycywgYW5kIGF2b2lkcyBldmVyeQpjb250cmFjdCByZS1kZWNsYXJpbmcgdGhlIHNhbWUgZmFpbHVyZSBtb2Rlcy4gQ29udHJhY3QgY3JhdGVzIG1heSBleHBvc2UKdGhlaXIgb3duIGRvbWFpbi1zcGVjaWZpYyBlcnJvcnMsIGJ1dCBzaG91bGQgcHJlZmVyIHRoZXNlIHdoZXJlIHRoZXkgZml0LgoKRXJyb3IgY29kZXMgc3RhcnQgYXQgMTsgY29kZSAwIGlzIHJlc2VydmVkIGJ5IHRoZSBTb3JvYmFuIGhvc3QuAAAAAAAAAApGb3JnZUVycm9yAAAAAAAPAAAAM1RoZSBjYWxsZXIgaXMgbm90IHBlcm1pdHRlZCB0byBwZXJmb3JtIHRoaXMgYWN0aW9uLgAAAAAMVW5hdXRob3JpemVkAAAAAQAAAEpUaGUgcmVxdWVzdGVkIGVudGl0eSAoZXNjcm93LCBwcm9wb3NhbCwgc3Vic2NyaXB0aW9uLCAuLi4pIGRvZXMgbm90IGV4aXN0LgAAAAAACE5vdEZvdW5kAAAAAgAAAEhPbmUgb3IgbW9yZSBhcmd1bWVudHMgZmFpbGVkIHZhbGlkYXRpb24gKGUuZy4gemVybyBhbW91bnQsIGJhZCBhZGRyZXNzKS4AAAAMSW52YWxpZElucHV0AAAAAwAAAENUaGUgY29udHJhY3QgZG9lcyBub3QgaG9sZCBlbm91Z2ggYmFsYW5jZSB0byBzYXRpc2Z5IHRoZSBvcGVyYXRpb24uAAAAABFJbnN1ZmZpY2llbnRGdW5kcwAAAAAAAAQAAABCVGhlIGVudGl0eSB3YXMgYWxyZWFkeSBpbml0aWFsaXNlZDsgcmUtaW5pdGlhbGlzYXRpb24gaXMgcmVqZWN0ZWQuAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAAFAAAANlRoZSBlbnRpdHkgd2FzIGV4cGVjdGVkIHRvIGJlIGluaXRpYWxpc2VkIGJ1dCB3YXMgbm90LgAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAGAAAANkFuIG9wZXJhdGlvbiB3YXMgYXR0ZW1wdGVkIGFmdGVyIGl0cyBkZWFkbGluZSBlbGFwc2VkLgAAAAAAD0RlYWRsaW5lUmVhY2hlZAAAAAAHAAAAQUEgcmVxdWlyZWQgdG9rZW4gYWxsb3dhbmNlIHdhcyBsb3dlciB0aGFuIHRoZSBhbW91bnQgYmVpbmcgc3BlbnQuAAAAAAAAFUluc3VmZmljaWVudEFsbG93YW5jZQAAAAAAAAgAAAAjQW4gYXJpdGhtZXRpYyBvcGVyYXRpb24gb3ZlcmZsb3dlZC4AAAAAEkFyaXRobWV0aWNPdmVyZmxvdwAAAAAACQAAAEJDb250cmFjdC1zcGVjaWZpYyBlcnJvciB0aGF0IGRvZXMgbm90IG1hcCB0byB0aGUgY2F0ZWdvcmllcyBhYm92ZS4AAAAAAAZDdXN0b20AAAAAAAoAAAFsQSBTRVAtNDEgdG9rZW4gaW52b2NhdGlvbiBmYWlsZWQgKGluc3VmZmljaWVudCBiYWxhbmNlLCBtaXNzaW5nIG9yCmRlYXV0aG9yaXplZCB0cnVzdGxpbmUsIHVuZGVwbG95ZWQgdG9rZW4gY29udHJhY3QsIG9yIHRva2VuLWxvZ2ljCnJlamVjdGlvbikuIFRoZSByYXcgdG9rZW4gZXJyb3IgZGlzY3JpbWluYW50IGlzIGludGVudGlvbmFsbHkgbm90CmZvcndhcmRlZCDigJQgY2FsbGVycyBjYW5ub3QgdGVsbCB3aGljaCBjb250cmFjdCBwcm9kdWNlZCBhIGZvcndhcmRlZApjb2RlLCBzbyBpdCBpcyBidWNrZXRlZDsgdGhlIHJvb3QgY2F1c2UgcmVtYWlucyB2aXNpYmxlIGluIHRoZQp0cmFuc2FjdGlvbidzIGRpYWdub3N0aWMgZXZlbnRzLgAAABNUb2tlblRyYW5zZmVyRmFpbGVkAAAAAAsAAADbQSBjcm9zcy1jb250cmFjdCBpbnZvY2F0aW9uIGZhaWxlZCAodGFyZ2V0IHJldmVydGVkIG9yIGhvc3QKYWJvcnQpLiBUaGUgdGFyZ2V0IHJldmVydHMgYXJlIHN1cmZhY2VkIGhlcmUgc28gdGhlIGNhbGxlciBjYW4KZGlzdGluZ3Vpc2ggdGhlbSBmcm9tIHRva2VuLXRyYW5zZmVyIGZhaWx1cmVzLCBhbmQgdGhlIGludm9raW5nCnRyYW5zYWN0aW9uIGlzIGxlZnQgdW4tZXhlY3V0ZWQuAAAAABhDb250cmFjdEludm9jYXRpb25GYWlsZWQAAAAMAAABDFRoZSByZXF1ZXN0ZWQgd2l0aGRyYXdhbCB3b3VsZCBwdXNoIGEgdG9rZW4ncyByb2xsaW5nLXdpbmRvdyB0b3RhbApwYXN0IGl0cyBjb25maWd1cmVkIHdpdGhkcmF3YWwgbGltaXQuIEtlcHQgZGlzdGluY3QgZnJvbQpbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIHNvIGEgY2FsbGVyIGNhbiB0ZWxsIGEgcG9saWN5IHJlamVjdGlvbgooYSB2YWxpZCB3aXRoZHJhd2FsIHRoYXQgaXMgdG9vIGxhcmdlIHJpZ2h0IG5vdykgZnJvbSBhIG1hbGZvcm1lZAphcmd1bWVudC4AAAAXV2l0aGRyYXdhbExpbWl0RXhjZWVkZWQAAAAADQAAARtBIHN1YnNjcmlwdGlvbiBpcyBpbiB0aGUgYFBhc3REdWVgIGxhcHNlZCBzdGF0ZSBhbmQgdGhlIHJlcXVlc3RlZApvcGVyYXRpb24gY2Fubm90IGJlIHBlcmZvcm1lZCB1bnRpbCBhIGNhdGNoLXVwIGNoYXJnZSByZXN0b3JlcyBpdCB0bwpgQWN0aXZlYC4gS2VwdCBkaXN0aW5jdCBmcm9tIFtgRm9yZ2VFcnJvcjo6RGVhZGxpbmVSZWFjaGVkYF0gc28gYSBjYWxsZXIKY2FuIHRlbGwgYSBsYXBzZWQgYmlsbGluZyBzdGF0ZSBmcm9tIGFuIGFyZ3VtZW50IHRoYXQgd2FzIG1lcmVseQp0b28gZWFybHkuAAAAABNTdWJzY3JpcHRpb25QYXN0RHVlAAAAAA4AAABxQSBwcm9wb3NlciBoYXMgcmVhY2hlZCB0aGUgbWF4aW11bSBhbGxvd2VkIGNvbmN1cnJlbnQgYWN0aXZlIHByb3Bvc2FscwpvciBpcyB3aXRoaW4gdGhlIHByb3Bvc2VyIGNvb2xkb3duIHdpbmRvdy4AAAAAAAAQUHJvcG9zZXJDb29sZG93bgAAAA8=" ]),
      options
    )
  }
  public readonly fromJSON = {
    claim: this.txFromJSON<Result<i128>>,
        revoke: this.txFromJSON<Result<void>>,
        claim_for: this.txFromJSON<Result<i128>>,
        claimable: this.txFromJSON<Result<i128>>,
        get_status: this.txFromJSON<Result<VestingStatus>>,
        get_schedule: this.txFromJSON<Result<VestingSchedule>>,
        claimable_for: this.txFromJSON<Result<i128>>,
        create_schedule: this.txFromJSON<Result<u64>>,
        get_tranche_schedule: this.txFromJSON<Result<TrancheSchedule>>,
        reassign_beneficiary: this.txFromJSON<Result<void>>,
        create_tranche_schedule: this.txFromJSON<Result<u64>>
  }
}