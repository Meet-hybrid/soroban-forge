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
 * One metered dimension's pricing terms: units included in the base period
 * price plus the per-bucket overage price for what exceeds the inclusion.
 */
export interface MetricQuota {
  /**
 * Units per overage bucket. Must be greater than zero; a started bucket
 * is billed in full (rounding is up).
 */
bucket_units: u64;
  /**
 * Units covered by the base `amount`; usage up to and including this
 * many units bills nothing extra.
 */
included_units: u64;
  /**
 * Cap on billable overage units for one period, or `None` for uncapped.
 *
 * The cap clamps overage units before they are bucketed, so a period can
 * bill at most `ceil(cap / bucket_units)` buckets for this metric.
 */
max_overage_units: Option<u64>;
  /**
 * The metered dimension, e.g. `api_calls` or `storage_bytes`.
 */
metric: string;
  /**
 * Price of one started bucket of overage units, in the subscription's
 * token. Per bucket, never per unit, so no sub-unit price precision is
 * lost to integer division.
 */
overage_price: i128;
}


/**
 * Usage metered for one metric in one billing period.
 */
export interface UsageRecord {
  /**
 * The metered dimension.
 */
metric: string;
  /**
 * The `last_charged` this meter belongs to. A meter whose stamp no
 * longer matches the subscription's open period is not counted, and the
 * next `record_usage` restarts it.
 */
period_start: u64;
  /**
 * Subscription the usage was recorded against.
 */
subscription_id: u64;
  /**
 * Raw units accumulated in the open period. Always a unit count, never a
 * bucket count, so no rounding state can survive into the next period.
 */
units: u64;
}


/**
 * A recurring payment agreement.
 */
export interface Subscription {
  /**
 * Amount charged per period, before usage overage.
 */
amount: i128;
  /**
 * Number of consecutive failed billing attempts.
 */
failed_attempts: u32;
  /**
 * Ledger timestamp of the last successful charge.
 */
last_charged: u64;
  /**
 * Ledger timestamp at which the subscription first became overdue for the
 * current open period, or `None` while the subscription is current.
 * Set when a charge observes an elapsed due time without a successful
 * payment; cleared on a successful charge.
 */
past_due_since: Option<u64>;
  /**
 * Ledger timestamp when paused, if currently paused.
 */
paused_at: Option<u64>;
  /**
 * Length of one billing period, in seconds.
 */
period: u64;
  /**
 * `None` means pull mode; `Some(balance)` means prepaid mode is enabled.
 * A zero balance remains `Some(0)` so topping up never changes modes.
 */
prepaid_balance: Option<i128>;
  /**
 * Account receiving payments.
 */
provider: string;
  /**
 * Per-metric usage quotas priced on top of `amount`.
 *
 * Empty is the flat flow: the period bills exactly `amount`. The list is
 * shaped so it can be embedded verbatim in a plan record (a plan copies
 * its quotas here when a subscriber joins it), keeping the metering core
 * independent of where the terms were published.
 */
quotas: Array<MetricQuota>;
  /**
 * Current state.
 */
status: SubscriptionStatus;
  /**
 * Account being charged.
 */
subscriber: string;
  /**
 * Stable identifier assigned at creation.
 */
subscription_id: u64;
  /**
 * Token contract used for settlement.
 */
token: string;
}

/**
 * Lifecycle state of a subscription.
 */
export type SubscriptionStatus = {tag: "Active", values: void} | {tag: "Cancelled", values: void} | {tag: "PastDue", values: void} | {tag: "Paused", values: void};








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
   * Construct and simulate a pause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pause an active subscription, preventing charges while paused.
   *
   * Requires the subscriber. Only valid when `Active`.
   */
  pause: ({subscription_id}: {subscription_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a cancel transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cancel a subscription, preventing further charges.
   *
   * Requires the subscriber. Valid when `Active`, `Paused`, or `PastDue`. Cancelling
   * an already-cancelled subscription is rejected.
   */
  cancel: ({subscription_id}: {subscription_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a charge transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Bill one due period.
   *
   * Requires the provider. If a full period has not elapsed since the last
   * charge, returns `0` and leaves the subscription untouched. Otherwise
   * attempts to transfer the period's derived amount of `token` from
   * `subscriber` to `provider`.
   *
   * The derived amount is `amount` plus the overage for every declared
   * metric (see the module docs on metered usage) — exactly `amount` when no
   * quotas are declared. It is computed before the transfer, so an
   * arithmetic failure never moves funds.
   *
   * - On successful payment: advances `last_charged` by one period, drops
   * the period's usage meters, resets `failed_attempts` to 0, transitions
   * status to `Active`, and returns the derived amount.
   * - On failed payment: `last_charged` is NOT advanced and the usage
   * meters are NOT dropped, so the retry re-derives the same amount.
   * Increments `failed_attempts`. If `failed_attempts >= MAX_RETRIES`
   * (3), status becomes `Cancelled`. Otherwise status becomes `PastDue`.
   * Returns `0`.
   */
  charge: ({subscription_id}: {subscription_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a resume transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Resume a paused subscription, advancing the next due date by the
   * elapsed paused duration so that paused periods are not billed.
   *
   * Requires the subscriber. Only valid when `Paused`.
   */
  resume: ({subscription_id}: {subscription_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a deposit transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pull tokens into custody and enable prepaid mode only after success.
   */
  deposit: ({subscription_id, amount}: {subscription_id: u64, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a get_usage transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read the usage recorded for `metric` in the open period (read-only
   * view).
   */
  get_usage: ({subscription_id, metric}: {subscription_id: u64, metric: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<UsageRecord>>>

  /**
   * Construct and simulate a subscribe transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Create a new subscription and return its stable id.
   *
   * Requires `amount > 0` and `period > 0`. The subscriber is authorized at
   * creation time; billing starts from the moment of subscription.
   */
  subscribe: ({subscriber, provider, token, amount, period}: {subscriber: string, provider: string, token: string, amount: i128, period: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a touch_ttl transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Extend the persistent TTL of a subscription record without changing
   * its business state. This is intended for permissionless keeper calls.
   */
  touch_ttl: ({subscription_id}: {subscription_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a set_quotas transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Declare (or replace) the metered-usage quotas of `subscription_id`.
   *
   * Requires the subscriber — see the module docs on metered usage for why
   * the account being charged consents to the overage price. Rejected unless
   * the subscription is `Active` and the open period has no recorded usage,
   * so already-metered units can never be repriced mid-period.
   */
  set_quotas: ({subscription_id, quotas}: {subscription_id: u64, quotas: Array<MetricQuota>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a quote_period transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * The amount the open period would bill right now (read-only view).
   */
  quote_period: ({subscription_id}: {subscription_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a record_usage transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Record `units` of `metric` consumed in the subscription's open period.
   *
   * Requires the provider. See the module docs on metered usage for the
   * period-boundary, cap, and retry semantics.
   */
  record_usage: ({subscription_id, metric, units}: {subscription_id: u64, metric: string, units: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a charge_catchup transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Atomically bill multiple elapsed periods.
   */
  charge_catchup: ({subscription_id, max_periods}: {subscription_id: u64, max_periods: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a revoke_provider transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Withdraw `subscriber`'s explicit authorization of `provider` (see the
   * module docs on provider opt-in).
   *
   * Requires the subscriber. Idempotent: revoking a provider that is not
   * authorized succeeds and leaves the opt-in absent.
   */
  revoke_provider: ({subscriber, provider}: {subscriber: string, provider: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_subscription transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read a stored subscription by id (read-only view).
   */
  get_subscription: ({subscription_id}: {subscription_id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Subscription>>>

  /**
   * Construct and simulate a withdraw_balance transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return prepaid funds to the subscriber while retaining prepaid mode.
   */
  withdraw_balance: ({subscription_id, amount}: {subscription_id: u64, amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<i128>>>

  /**
   * Construct and simulate a authorize_provider transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Explicitly authorize `provider` to create subscriptions on
   * `subscriber`'s behalf (see the module docs on provider opt-in).
   *
   * Requires the subscriber. Idempotent: authorizing a provider that is
   * already authorized succeeds.
   */
  authorize_provider: ({subscriber, provider}: {subscriber: string, provider: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_subscription_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Total number of subscriptions created so far (read-only view).
   *
   * This is the monotonic id counter, which only `subscribe` advances, so
   * it never decreases and equals the number of live ids returned by the
   * enumeration views.
   */
  get_subscription_count: (options?: MethodOptions) => Promise<AssembledTransaction<u64>>

  /**
   * Construct and simulate a is_provider_authorized transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Read whether `subscriber` has explicitly authorized `provider`
   * (read-only view; requires no authorization).
   */
  is_provider_authorized: ({subscriber, provider}: {subscriber: string, provider: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a subscribe_on_behalf_of transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Subscribe `subscriber` to `provider`'s service on the provider's
   * initiative (see the module docs on provider opt-in).
   *
   * Requires `amount > 0`, `period > 0`, the provider's authorization, and
   * an explicit subscriber opt-in for the provider (checked before the
   * provider is authorized, so a missing opt-in surfaces without spending
   * the provider's signature). The subscriber is **not** authorized at
   * creation. The record is created through the same internal path as
   * [`subscribe`], sharing the same sequential id counter.
   */
  subscribe_on_behalf_of: ({provider, subscriber, token, amount, period}: {provider: string, subscriber: string, token: string, amount: i128, period: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a subscriptions_for_provider transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * List `provider`'s subscriptions in creation order, one page at a time
   * (read-only view).
   *
   * Requires no authorization and never mutates storage. An address with
   * no subscriptions — or an offset at or past the end of its list —
   * returns an empty `Vec`, not an error, so clients can back "my
   * subscribers" views without an off-chain indexer.
   *
   * # Errors
   *
   * * [`ForgeError::InvalidInput`] — `limit` is zero.
   */
  subscriptions_for_provider: ({provider, offset, limit}: {provider: string, offset: u32, limit: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Array<Subscription>>>>

  /**
   * Construct and simulate a subscriptions_for_subscriber transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * List `subscriber`'s subscriptions in creation order, one page at a
   * time (read-only view).
   *
   * Requires no authorization and never mutates storage. An address with
   * no subscriptions — or an offset at or past the end of its list —
   * returns an empty `Vec`, not an error, so clients can back "my
   * subscriptions" views without an off-chain indexer.
   *
   * # Errors
   *
   * * [`ForgeError::InvalidInput`] — `limit` is zero.
   */
  subscriptions_for_subscriber: ({subscriber, offset, limit}: {subscriber: string, offset: u32, limit: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Array<Subscription>>>>

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
      new ContractSpec([ "AAAAAQAAAJBPbmUgbWV0ZXJlZCBkaW1lbnNpb24ncyBwcmljaW5nIHRlcm1zOiB1bml0cyBpbmNsdWRlZCBpbiB0aGUgYmFzZSBwZXJpb2QKcHJpY2UgcGx1cyB0aGUgcGVyLWJ1Y2tldCBvdmVyYWdlIHByaWNlIGZvciB3aGF0IGV4Y2VlZHMgdGhlIGluY2x1c2lvbi4AAAAAAAAAC01ldHJpY1F1b3RhAAAAAAUAAABpVW5pdHMgcGVyIG92ZXJhZ2UgYnVja2V0LiBNdXN0IGJlIGdyZWF0ZXIgdGhhbiB6ZXJvOyBhIHN0YXJ0ZWQgYnVja2V0CmlzIGJpbGxlZCBpbiBmdWxsIChyb3VuZGluZyBpcyB1cCkuAAAAAAAADGJ1Y2tldF91bml0cwAAAAYAAABiVW5pdHMgY292ZXJlZCBieSB0aGUgYmFzZSBgYW1vdW50YDsgdXNhZ2UgdXAgdG8gYW5kIGluY2x1ZGluZyB0aGlzCm1hbnkgdW5pdHMgYmlsbHMgbm90aGluZyBleHRyYS4AAAAAAA5pbmNsdWRlZF91bml0cwAAAAAABgAAAM5DYXAgb24gYmlsbGFibGUgb3ZlcmFnZSB1bml0cyBmb3Igb25lIHBlcmlvZCwgb3IgYE5vbmVgIGZvciB1bmNhcHBlZC4KClRoZSBjYXAgY2xhbXBzIG92ZXJhZ2UgdW5pdHMgYmVmb3JlIHRoZXkgYXJlIGJ1Y2tldGVkLCBzbyBhIHBlcmlvZCBjYW4KYmlsbCBhdCBtb3N0IGBjZWlsKGNhcCAvIGJ1Y2tldF91bml0cylgIGJ1Y2tldHMgZm9yIHRoaXMgbWV0cmljLgAAAAAAEW1heF9vdmVyYWdlX3VuaXRzAAAAAAAD6AAAAAYAAAA7VGhlIG1ldGVyZWQgZGltZW5zaW9uLCBlLmcuIGBhcGlfY2FsbHNgIG9yIGBzdG9yYWdlX2J5dGVzYC4AAAAABm1ldHJpYwAAAAAAEQAAAKJQcmljZSBvZiBvbmUgc3RhcnRlZCBidWNrZXQgb2Ygb3ZlcmFnZSB1bml0cywgaW4gdGhlIHN1YnNjcmlwdGlvbidzCnRva2VuLiBQZXIgYnVja2V0LCBuZXZlciBwZXIgdW5pdCwgc28gbm8gc3ViLXVuaXQgcHJpY2UgcHJlY2lzaW9uIGlzCmxvc3QgdG8gaW50ZWdlciBkaXZpc2lvbi4AAAAAAA1vdmVyYWdlX3ByaWNlAAAAAAAACw==",
        "AAAAAQAAADNVc2FnZSBtZXRlcmVkIGZvciBvbmUgbWV0cmljIGluIG9uZSBiaWxsaW5nIHBlcmlvZC4AAAAAAAAAAAtVc2FnZVJlY29yZAAAAAAEAAAAFlRoZSBtZXRlcmVkIGRpbWVuc2lvbi4AAAAAAAZtZXRyaWMAAAAAABEAAACnVGhlIGBsYXN0X2NoYXJnZWRgIHRoaXMgbWV0ZXIgYmVsb25ncyB0by4gQSBtZXRlciB3aG9zZSBzdGFtcCBubwpsb25nZXIgbWF0Y2hlcyB0aGUgc3Vic2NyaXB0aW9uJ3Mgb3BlbiBwZXJpb2QgaXMgbm90IGNvdW50ZWQsIGFuZCB0aGUKbmV4dCBgcmVjb3JkX3VzYWdlYCByZXN0YXJ0cyBpdC4AAAAADHBlcmlvZF9zdGFydAAAAAYAAAAsU3Vic2NyaXB0aW9uIHRoZSB1c2FnZSB3YXMgcmVjb3JkZWQgYWdhaW5zdC4AAAAPc3Vic2NyaXB0aW9uX2lkAAAAAAYAAACLUmF3IHVuaXRzIGFjY3VtdWxhdGVkIGluIHRoZSBvcGVuIHBlcmlvZC4gQWx3YXlzIGEgdW5pdCBjb3VudCwgbmV2ZXIgYQpidWNrZXQgY291bnQsIHNvIG5vIHJvdW5kaW5nIHN0YXRlIGNhbiBzdXJ2aXZlIGludG8gdGhlIG5leHQgcGVyaW9kLgAAAAAFdW5pdHMAAAAAAAAG",
        "AAAAAQAAAB5BIHJlY3VycmluZyBwYXltZW50IGFncmVlbWVudC4AAAAAAAAAAAAMU3Vic2NyaXB0aW9uAAAADQAAADBBbW91bnQgY2hhcmdlZCBwZXIgcGVyaW9kLCBiZWZvcmUgdXNhZ2Ugb3ZlcmFnZS4AAAAGYW1vdW50AAAAAAALAAAALk51bWJlciBvZiBjb25zZWN1dGl2ZSBmYWlsZWQgYmlsbGluZyBhdHRlbXB0cy4AAAAAAA9mYWlsZWRfYXR0ZW1wdHMAAAAABAAAAC9MZWRnZXIgdGltZXN0YW1wIG9mIHRoZSBsYXN0IHN1Y2Nlc3NmdWwgY2hhcmdlLgAAAAAMbGFzdF9jaGFyZ2VkAAAABgAAAPZMZWRnZXIgdGltZXN0YW1wIGF0IHdoaWNoIHRoZSBzdWJzY3JpcHRpb24gZmlyc3QgYmVjYW1lIG92ZXJkdWUgZm9yIHRoZQpjdXJyZW50IG9wZW4gcGVyaW9kLCBvciBgTm9uZWAgd2hpbGUgdGhlIHN1YnNjcmlwdGlvbiBpcyBjdXJyZW50LgpTZXQgd2hlbiBhIGNoYXJnZSBvYnNlcnZlcyBhbiBlbGFwc2VkIGR1ZSB0aW1lIHdpdGhvdXQgYSBzdWNjZXNzZnVsCnBheW1lbnQ7IGNsZWFyZWQgb24gYSBzdWNjZXNzZnVsIGNoYXJnZS4AAAAAAA5wYXN0X2R1ZV9zaW5jZQAAAAAD6AAAAAYAAAAyTGVkZ2VyIHRpbWVzdGFtcCB3aGVuIHBhdXNlZCwgaWYgY3VycmVudGx5IHBhdXNlZC4AAAAAAAlwYXVzZWRfYXQAAAAAAAPoAAAABgAAAClMZW5ndGggb2Ygb25lIGJpbGxpbmcgcGVyaW9kLCBpbiBzZWNvbmRzLgAAAAAAAAZwZXJpb2QAAAAAAAYAAACKYE5vbmVgIG1lYW5zIHB1bGwgbW9kZTsgYFNvbWUoYmFsYW5jZSlgIG1lYW5zIHByZXBhaWQgbW9kZSBpcyBlbmFibGVkLgpBIHplcm8gYmFsYW5jZSByZW1haW5zIGBTb21lKDApYCBzbyB0b3BwaW5nIHVwIG5ldmVyIGNoYW5nZXMgbW9kZXMuAAAAAAAPcHJlcGFpZF9iYWxhbmNlAAAAA+gAAAALAAAAG0FjY291bnQgcmVjZWl2aW5nIHBheW1lbnRzLgAAAAAIcHJvdmlkZXIAAAATAAABNlBlci1tZXRyaWMgdXNhZ2UgcXVvdGFzIHByaWNlZCBvbiB0b3Agb2YgYGFtb3VudGAuCgpFbXB0eSBpcyB0aGUgZmxhdCBmbG93OiB0aGUgcGVyaW9kIGJpbGxzIGV4YWN0bHkgYGFtb3VudGAuIFRoZSBsaXN0IGlzCnNoYXBlZCBzbyBpdCBjYW4gYmUgZW1iZWRkZWQgdmVyYmF0aW0gaW4gYSBwbGFuIHJlY29yZCAoYSBwbGFuIGNvcGllcwppdHMgcXVvdGFzIGhlcmUgd2hlbiBhIHN1YnNjcmliZXIgam9pbnMgaXQpLCBrZWVwaW5nIHRoZSBtZXRlcmluZyBjb3JlCmluZGVwZW5kZW50IG9mIHdoZXJlIHRoZSB0ZXJtcyB3ZXJlIHB1Ymxpc2hlZC4AAAAAAAZxdW90YXMAAAAAA+oAAAfQAAAAC01ldHJpY1F1b3RhAAAAAA5DdXJyZW50IHN0YXRlLgAAAAAABnN0YXR1cwAAAAAH0AAAABJTdWJzY3JpcHRpb25TdGF0dXMAAAAAABZBY2NvdW50IGJlaW5nIGNoYXJnZWQuAAAAAAAKc3Vic2NyaWJlcgAAAAAAEwAAACdTdGFibGUgaWRlbnRpZmllciBhc3NpZ25lZCBhdCBjcmVhdGlvbi4AAAAAD3N1YnNjcmlwdGlvbl9pZAAAAAAGAAAAI1Rva2VuIGNvbnRyYWN0IHVzZWQgZm9yIHNldHRsZW1lbnQuAAAAAAV0b2tlbgAAAAAAABM=",
        "AAAAAgAAACJMaWZlY3ljbGUgc3RhdGUgb2YgYSBzdWJzY3JpcHRpb24uAAAAAAAAAAAAElN1YnNjcmlwdGlvblN0YXR1cwAAAAAABAAAAAAAAAAWQWN0aXZlIGFuZCBjaGFyZ2VhYmxlLgAAAAAABkFjdGl2ZQAAAAAAAAAAAB5DYW5jZWxsZWQ7IG5vIGZ1cnRoZXIgY2hhcmdlcy4AAAAAAAlDYW5jZWxsZWQAAAAAAAAAAAAAMlBheW1lbnQgZmFpbGVkIGFuZCB0aGUgc3Vic2NyaXB0aW9uIGlzIGluIGFycmVhcnMuAAAAAAAHUGFzdER1ZQAAAAAAAAAAOVRlbXBvcmFyaWx5IHBhdXNlZDsgbm8gY2hhcmdlcyBjYW4gYmUgbWFkZSB1bnRpbCByZXN1bWVkLgAAAAAAAAZQYXVzZWQAAA==",
        "AAAAAAAAAHJQYXVzZSBhbiBhY3RpdmUgc3Vic2NyaXB0aW9uLCBwcmV2ZW50aW5nIGNoYXJnZXMgd2hpbGUgcGF1c2VkLgoKUmVxdWlyZXMgdGhlIHN1YnNjcmliZXIuIE9ubHkgdmFsaWQgd2hlbiBgQWN0aXZlYC4AAAAAAAVwYXVzZQAAAAAAAAEAAAAAAAAAD3N1YnNjcmlwdGlvbl9pZAAAAAAGAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAALNDYW5jZWwgYSBzdWJzY3JpcHRpb24sIHByZXZlbnRpbmcgZnVydGhlciBjaGFyZ2VzLgoKUmVxdWlyZXMgdGhlIHN1YnNjcmliZXIuIFZhbGlkIHdoZW4gYEFjdGl2ZWAsIGBQYXVzZWRgLCBvciBgUGFzdER1ZWAuIENhbmNlbGxpbmcKYW4gYWxyZWFkeS1jYW5jZWxsZWQgc3Vic2NyaXB0aW9uIGlzIHJlamVjdGVkLgAAAAAGY2FuY2VsAAAAAAABAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAA8pCaWxsIG9uZSBkdWUgcGVyaW9kLgoKUmVxdWlyZXMgdGhlIHByb3ZpZGVyLiBJZiBhIGZ1bGwgcGVyaW9kIGhhcyBub3QgZWxhcHNlZCBzaW5jZSB0aGUgbGFzdApjaGFyZ2UsIHJldHVybnMgYDBgIGFuZCBsZWF2ZXMgdGhlIHN1YnNjcmlwdGlvbiB1bnRvdWNoZWQuIE90aGVyd2lzZQphdHRlbXB0cyB0byB0cmFuc2ZlciB0aGUgcGVyaW9kJ3MgZGVyaXZlZCBhbW91bnQgb2YgYHRva2VuYCBmcm9tCmBzdWJzY3JpYmVyYCB0byBgcHJvdmlkZXJgLgoKVGhlIGRlcml2ZWQgYW1vdW50IGlzIGBhbW91bnRgIHBsdXMgdGhlIG92ZXJhZ2UgZm9yIGV2ZXJ5IGRlY2xhcmVkCm1ldHJpYyAoc2VlIHRoZSBtb2R1bGUgZG9jcyBvbiBtZXRlcmVkIHVzYWdlKSDigJQgZXhhY3RseSBgYW1vdW50YCB3aGVuIG5vCnF1b3RhcyBhcmUgZGVjbGFyZWQuIEl0IGlzIGNvbXB1dGVkIGJlZm9yZSB0aGUgdHJhbnNmZXIsIHNvIGFuCmFyaXRobWV0aWMgZmFpbHVyZSBuZXZlciBtb3ZlcyBmdW5kcy4KCi0gT24gc3VjY2Vzc2Z1bCBwYXltZW50OiBhZHZhbmNlcyBgbGFzdF9jaGFyZ2VkYCBieSBvbmUgcGVyaW9kLCBkcm9wcwp0aGUgcGVyaW9kJ3MgdXNhZ2UgbWV0ZXJzLCByZXNldHMgYGZhaWxlZF9hdHRlbXB0c2AgdG8gMCwgdHJhbnNpdGlvbnMKc3RhdHVzIHRvIGBBY3RpdmVgLCBhbmQgcmV0dXJucyB0aGUgZGVyaXZlZCBhbW91bnQuCi0gT24gZmFpbGVkIHBheW1lbnQ6IGBsYXN0X2NoYXJnZWRgIGlzIE5PVCBhZHZhbmNlZCBhbmQgdGhlIHVzYWdlCm1ldGVycyBhcmUgTk9UIGRyb3BwZWQsIHNvIHRoZSByZXRyeSByZS1kZXJpdmVzIHRoZSBzYW1lIGFtb3VudC4KSW5jcmVtZW50cyBgZmFpbGVkX2F0dGVtcHRzYC4gSWYgYGZhaWxlZF9hdHRlbXB0cyA+PSBNQVhfUkVUUklFU2AKKDMpLCBzdGF0dXMgYmVjb21lcyBgQ2FuY2VsbGVkYC4gT3RoZXJ3aXNlIHN0YXR1cyBiZWNvbWVzIGBQYXN0RHVlYC4KUmV0dXJucyBgMGAuAAAAAAAGY2hhcmdlAAAAAAABAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAALNSZXN1bWUgYSBwYXVzZWQgc3Vic2NyaXB0aW9uLCBhZHZhbmNpbmcgdGhlIG5leHQgZHVlIGRhdGUgYnkgdGhlCmVsYXBzZWQgcGF1c2VkIGR1cmF0aW9uIHNvIHRoYXQgcGF1c2VkIHBlcmlvZHMgYXJlIG5vdCBiaWxsZWQuCgpSZXF1aXJlcyB0aGUgc3Vic2NyaWJlci4gT25seSB2YWxpZCB3aGVuIGBQYXVzZWRgLgAAAAAGcmVzdW1lAAAAAAABAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAERQdWxsIHRva2VucyBpbnRvIGN1c3RvZHkgYW5kIGVuYWJsZSBwcmVwYWlkIG1vZGUgb25seSBhZnRlciBzdWNjZXNzLgAAAAdkZXBvc2l0AAAAAAIAAAAAAAAAD3N1YnNjcmlwdGlvbl9pZAAAAAAGAAAAAAAAAAZhbW91bnQAAAAAAAsAAAABAAAD6QAAAAsAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAElSZWFkIHRoZSB1c2FnZSByZWNvcmRlZCBmb3IgYG1ldHJpY2AgaW4gdGhlIG9wZW4gcGVyaW9kIChyZWFkLW9ubHkKdmlldykuAAAAAAAACWdldF91c2FnZQAAAAAAAAIAAAAAAAAAD3N1YnNjcmlwdGlvbl9pZAAAAAAGAAAAAAAAAAZtZXRyaWMAAAAAABEAAAABAAAD6QAAB9AAAAALVXNhZ2VSZWNvcmQAAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAALtDcmVhdGUgYSBuZXcgc3Vic2NyaXB0aW9uIGFuZCByZXR1cm4gaXRzIHN0YWJsZSBpZC4KClJlcXVpcmVzIGBhbW91bnQgPiAwYCBhbmQgYHBlcmlvZCA+IDBgLiBUaGUgc3Vic2NyaWJlciBpcyBhdXRob3JpemVkIGF0CmNyZWF0aW9uIHRpbWU7IGJpbGxpbmcgc3RhcnRzIGZyb20gdGhlIG1vbWVudCBvZiBzdWJzY3JpcHRpb24uAAAAAAlzdWJzY3JpYmUAAAAAAAAFAAAAAAAAAApzdWJzY3JpYmVyAAAAAAATAAAAAAAAAAhwcm92aWRlcgAAABMAAAAAAAAABXRva2VuAAAAAAAAEwAAAAAAAAAGYW1vdW50AAAAAAALAAAAAAAAAAZwZXJpb2QAAAAAAAYAAAABAAAD6QAAAAYAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAIlFeHRlbmQgdGhlIHBlcnNpc3RlbnQgVFRMIG9mIGEgc3Vic2NyaXB0aW9uIHJlY29yZCB3aXRob3V0IGNoYW5naW5nCml0cyBidXNpbmVzcyBzdGF0ZS4gVGhpcyBpcyBpbnRlbmRlZCBmb3IgcGVybWlzc2lvbmxlc3Mga2VlcGVyIGNhbGxzLgAAAAAAAAl0b3VjaF90dGwAAAAAAAABAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAVlEZWNsYXJlIChvciByZXBsYWNlKSB0aGUgbWV0ZXJlZC11c2FnZSBxdW90YXMgb2YgYHN1YnNjcmlwdGlvbl9pZGAuCgpSZXF1aXJlcyB0aGUgc3Vic2NyaWJlciDigJQgc2VlIHRoZSBtb2R1bGUgZG9jcyBvbiBtZXRlcmVkIHVzYWdlIGZvciB3aHkKdGhlIGFjY291bnQgYmVpbmcgY2hhcmdlZCBjb25zZW50cyB0byB0aGUgb3ZlcmFnZSBwcmljZS4gUmVqZWN0ZWQgdW5sZXNzCnRoZSBzdWJzY3JpcHRpb24gaXMgYEFjdGl2ZWAgYW5kIHRoZSBvcGVuIHBlcmlvZCBoYXMgbm8gcmVjb3JkZWQgdXNhZ2UsCnNvIGFscmVhZHktbWV0ZXJlZCB1bml0cyBjYW4gbmV2ZXIgYmUgcmVwcmljZWQgbWlkLXBlcmlvZC4AAAAAAAAKc2V0X3F1b3RhcwAAAAAAAgAAAAAAAAAPc3Vic2NyaXB0aW9uX2lkAAAAAAYAAAAAAAAABnF1b3RhcwAAAAAD6gAAB9AAAAALTWV0cmljUXVvdGEAAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAEFUaGUgYW1vdW50IHRoZSBvcGVuIHBlcmlvZCB3b3VsZCBiaWxsIHJpZ2h0IG5vdyAocmVhZC1vbmx5IHZpZXcpLgAAAAAAAAxxdW90ZV9wZXJpb2QAAAABAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAALZSZWNvcmQgYHVuaXRzYCBvZiBgbWV0cmljYCBjb25zdW1lZCBpbiB0aGUgc3Vic2NyaXB0aW9uJ3Mgb3BlbiBwZXJpb2QuCgpSZXF1aXJlcyB0aGUgcHJvdmlkZXIuIFNlZSB0aGUgbW9kdWxlIGRvY3Mgb24gbWV0ZXJlZCB1c2FnZSBmb3IgdGhlCnBlcmlvZC1ib3VuZGFyeSwgY2FwLCBhbmQgcmV0cnkgc2VtYW50aWNzLgAAAAAADHJlY29yZF91c2FnZQAAAAMAAAAAAAAAD3N1YnNjcmlwdGlvbl9pZAAAAAAGAAAAAAAAAAZtZXRyaWMAAAAAABEAAAAAAAAABXVuaXRzAAAAAAAABgAAAAEAAAPpAAAAAgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAAClBdG9taWNhbGx5IGJpbGwgbXVsdGlwbGUgZWxhcHNlZCBwZXJpb2RzLgAAAAAAAA5jaGFyZ2VfY2F0Y2h1cAAAAAAAAgAAAAAAAAAPc3Vic2NyaXB0aW9uX2lkAAAAAAYAAAAAAAAAC21heF9wZXJpb2RzAAAAAAQAAAABAAAD6QAAAAsAAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAN5XaXRoZHJhdyBgc3Vic2NyaWJlcmAncyBleHBsaWNpdCBhdXRob3JpemF0aW9uIG9mIGBwcm92aWRlcmAgKHNlZSB0aGUKbW9kdWxlIGRvY3Mgb24gcHJvdmlkZXIgb3B0LWluKS4KClJlcXVpcmVzIHRoZSBzdWJzY3JpYmVyLiBJZGVtcG90ZW50OiByZXZva2luZyBhIHByb3ZpZGVyIHRoYXQgaXMgbm90CmF1dGhvcml6ZWQgc3VjY2VlZHMgYW5kIGxlYXZlcyB0aGUgb3B0LWluIGFic2VudC4AAAAAAA9yZXZva2VfcHJvdmlkZXIAAAAAAgAAAAAAAAAKc3Vic2NyaWJlcgAAAAAAEwAAAAAAAAAIcHJvdmlkZXIAAAATAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAADJSZWFkIGEgc3RvcmVkIHN1YnNjcmlwdGlvbiBieSBpZCAocmVhZC1vbmx5IHZpZXcpLgAAAAAAEGdldF9zdWJzY3JpcHRpb24AAAABAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAPpAAAH0AAAAAxTdWJzY3JpcHRpb24AAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAERSZXR1cm4gcHJlcGFpZCBmdW5kcyB0byB0aGUgc3Vic2NyaWJlciB3aGlsZSByZXRhaW5pbmcgcHJlcGFpZCBtb2RlLgAAABB3aXRoZHJhd19iYWxhbmNlAAAAAgAAAAAAAAAPc3Vic2NyaXB0aW9uX2lkAAAAAAYAAAAAAAAABmFtb3VudAAAAAAACwAAAAEAAAPpAAAACwAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAAAAAAANxFeHBsaWNpdGx5IGF1dGhvcml6ZSBgcHJvdmlkZXJgIHRvIGNyZWF0ZSBzdWJzY3JpcHRpb25zIG9uCmBzdWJzY3JpYmVyYCdzIGJlaGFsZiAoc2VlIHRoZSBtb2R1bGUgZG9jcyBvbiBwcm92aWRlciBvcHQtaW4pLgoKUmVxdWlyZXMgdGhlIHN1YnNjcmliZXIuIElkZW1wb3RlbnQ6IGF1dGhvcml6aW5nIGEgcHJvdmlkZXIgdGhhdCBpcwphbHJlYWR5IGF1dGhvcml6ZWQgc3VjY2VlZHMuAAAAEmF1dGhvcml6ZV9wcm92aWRlcgAAAAAAAgAAAAAAAAAKc3Vic2NyaWJlcgAAAAAAEwAAAAAAAAAIcHJvdmlkZXIAAAATAAAAAQAAA+kAAAACAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAN1Ub3RhbCBudW1iZXIgb2Ygc3Vic2NyaXB0aW9ucyBjcmVhdGVkIHNvIGZhciAocmVhZC1vbmx5IHZpZXcpLgoKVGhpcyBpcyB0aGUgbW9ub3RvbmljIGlkIGNvdW50ZXIsIHdoaWNoIG9ubHkgYHN1YnNjcmliZWAgYWR2YW5jZXMsIHNvCml0IG5ldmVyIGRlY3JlYXNlcyBhbmQgZXF1YWxzIHRoZSBudW1iZXIgb2YgbGl2ZSBpZHMgcmV0dXJuZWQgYnkgdGhlCmVudW1lcmF0aW9uIHZpZXdzLgAAAAAAABZnZXRfc3Vic2NyaXB0aW9uX2NvdW50AAAAAAAAAAAAAQAAAAY=",
        "AAAAAAAAAGtSZWFkIHdoZXRoZXIgYHN1YnNjcmliZXJgIGhhcyBleHBsaWNpdGx5IGF1dGhvcml6ZWQgYHByb3ZpZGVyYAoocmVhZC1vbmx5IHZpZXc7IHJlcXVpcmVzIG5vIGF1dGhvcml6YXRpb24pLgAAAAAWaXNfcHJvdmlkZXJfYXV0aG9yaXplZAAAAAAAAgAAAAAAAAAKc3Vic2NyaWJlcgAAAAAAEwAAAAAAAAAIcHJvdmlkZXIAAAATAAAAAQAAAAE=",
        "AAAAAAAAAgJTdWJzY3JpYmUgYHN1YnNjcmliZXJgIHRvIGBwcm92aWRlcmAncyBzZXJ2aWNlIG9uIHRoZSBwcm92aWRlcidzCmluaXRpYXRpdmUgKHNlZSB0aGUgbW9kdWxlIGRvY3Mgb24gcHJvdmlkZXIgb3B0LWluKS4KClJlcXVpcmVzIGBhbW91bnQgPiAwYCwgYHBlcmlvZCA+IDBgLCB0aGUgcHJvdmlkZXIncyBhdXRob3JpemF0aW9uLCBhbmQKYW4gZXhwbGljaXQgc3Vic2NyaWJlciBvcHQtaW4gZm9yIHRoZSBwcm92aWRlciAoY2hlY2tlZCBiZWZvcmUgdGhlCnByb3ZpZGVyIGlzIGF1dGhvcml6ZWQsIHNvIGEgbWlzc2luZyBvcHQtaW4gc3VyZmFjZXMgd2l0aG91dCBzcGVuZGluZwp0aGUgcHJvdmlkZXIncyBzaWduYXR1cmUpLiBUaGUgc3Vic2NyaWJlciBpcyAqKm5vdCoqIGF1dGhvcml6ZWQgYXQKY3JlYXRpb24uIFRoZSByZWNvcmQgaXMgY3JlYXRlZCB0aHJvdWdoIHRoZSBzYW1lIGludGVybmFsIHBhdGggYXMKW2BzdWJzY3JpYmVgXSwgc2hhcmluZyB0aGUgc2FtZSBzZXF1ZW50aWFsIGlkIGNvdW50ZXIuAAAAAAAWc3Vic2NyaWJlX29uX2JlaGFsZl9vZgAAAAAABQAAAAAAAAAIcHJvdmlkZXIAAAATAAAAAAAAAApzdWJzY3JpYmVyAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAABmFtb3VudAAAAAAACwAAAAAAAAAGcGVyaW9kAAAAAAAGAAAAAQAAA+kAAAAGAAAH0AAAAApGb3JnZUVycm9yAAA=",
        "AAAAAAAAAZBMaXN0IGBwcm92aWRlcmAncyBzdWJzY3JpcHRpb25zIGluIGNyZWF0aW9uIG9yZGVyLCBvbmUgcGFnZSBhdCBhIHRpbWUKKHJlYWQtb25seSB2aWV3KS4KClJlcXVpcmVzIG5vIGF1dGhvcml6YXRpb24gYW5kIG5ldmVyIG11dGF0ZXMgc3RvcmFnZS4gQW4gYWRkcmVzcyB3aXRoCm5vIHN1YnNjcmlwdGlvbnMg4oCUIG9yIGFuIG9mZnNldCBhdCBvciBwYXN0IHRoZSBlbmQgb2YgaXRzIGxpc3Qg4oCUCnJldHVybnMgYW4gZW1wdHkgYFZlY2AsIG5vdCBhbiBlcnJvciwgc28gY2xpZW50cyBjYW4gYmFjayAibXkKc3Vic2NyaWJlcnMiIHZpZXdzIHdpdGhvdXQgYW4gb2ZmLWNoYWluIGluZGV4ZXIuCgojIEVycm9ycwoKKiBbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIOKAlCBgbGltaXRgIGlzIHplcm8uAAAAGnN1YnNjcmlwdGlvbnNfZm9yX3Byb3ZpZGVyAAAAAAADAAAAAAAAAAhwcm92aWRlcgAAABMAAAAAAAAABm9mZnNldAAAAAAABAAAAAAAAAAFbGltaXQAAAAAAAAEAAAAAQAAA+kAAAPqAAAH0AAAAAxTdWJzY3JpcHRpb24AAAfQAAAACkZvcmdlRXJyb3IAAA==",
        "AAAAAAAAAZRMaXN0IGBzdWJzY3JpYmVyYCdzIHN1YnNjcmlwdGlvbnMgaW4gY3JlYXRpb24gb3JkZXIsIG9uZSBwYWdlIGF0IGEKdGltZSAocmVhZC1vbmx5IHZpZXcpLgoKUmVxdWlyZXMgbm8gYXV0aG9yaXphdGlvbiBhbmQgbmV2ZXIgbXV0YXRlcyBzdG9yYWdlLiBBbiBhZGRyZXNzIHdpdGgKbm8gc3Vic2NyaXB0aW9ucyDigJQgb3IgYW4gb2Zmc2V0IGF0IG9yIHBhc3QgdGhlIGVuZCBvZiBpdHMgbGlzdCDigJQKcmV0dXJucyBhbiBlbXB0eSBgVmVjYCwgbm90IGFuIGVycm9yLCBzbyBjbGllbnRzIGNhbiBiYWNrICJteQpzdWJzY3JpcHRpb25zIiB2aWV3cyB3aXRob3V0IGFuIG9mZi1jaGFpbiBpbmRleGVyLgoKIyBFcnJvcnMKCiogW2BGb3JnZUVycm9yOjpJbnZhbGlkSW5wdXRgXSDigJQgYGxpbWl0YCBpcyB6ZXJvLgAAABxzdWJzY3JpcHRpb25zX2Zvcl9zdWJzY3JpYmVyAAAAAwAAAAAAAAAKc3Vic2NyaWJlcgAAAAAAEwAAAAAAAAAGb2Zmc2V0AAAAAAAEAAAAAAAAAAVsaW1pdAAAAAAAAAQAAAABAAAD6QAAA+oAAAfQAAAADFN1YnNjcmlwdGlvbgAAB9AAAAAKRm9yZ2VFcnJvcgAA",
        "AAAABQAAAAAAAAAAAAAAB0NoYXJnZWQAAAAAAQAAAAdjaGFyZ2VkAAAAAAQAAAAAAAAAD3N1YnNjcmlwdGlvbl9pZAAAAAAGAAAAAQAAAGxBbW91bnQgYWN0dWFsbHkgdHJhbnNmZXJyZWQgZm9yIHRoZSBzZXR0bGVkIHBlcmlvZDogdGhlIGRlcml2ZWQKYGJhc2UgKyBvdmVyYWdlYCBhbW91bnQsIG5vdCB0aGUgYmFzZSBhbG9uZS4AAAAGYW1vdW50AAAAAAALAAAAAAAAAAAAAAAMbGFzdF9jaGFyZ2VkAAAABgAAAAAAAAAAAAAADm5leHRfY2hhcmdlX2F0AAAAAAAGAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAACUNhbmNlbGxlZAAAAAAAAAEAAAAJY2FuY2VsbGVkAAAAAAAAAgAAAAAAAAAPc3Vic2NyaXB0aW9uX2lkAAAAAAYAAAABAAAAAAAAAApzdWJzY3JpYmVyAAAAAAATAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAACURlcG9zaXRlZAAAAAAAAAEAAAAJZGVwb3NpdGVkAAAAAAAAAwAAAAAAAAAPc3Vic2NyaXB0aW9uX2lkAAAAAAYAAAABAAAAAAAAAAZhbW91bnQAAAAAAAsAAAAAAAAAAAAAAA1iYWxhbmNlX2FmdGVyAAAAAAAACwAAAAAAAAAC",
        "AAAABQAAADRUaGUgc3Vic2NyaXB0aW9uJ3MgZGVjbGFyZWQgcXVvdGEgbGlzdCB3YXMgcmVwbGFjZWQuAAAAAAAAAAlRdW90YXNTZXQAAAAAAAABAAAACnF1b3Rhc19zZXQAAAAAAAIAAAAAAAAAD3N1YnNjcmlwdGlvbl9pZAAAAAAGAAAAAQAAAAAAAAAGcXVvdGFzAAAAAAPqAAAH0AAAAAtNZXRyaWNRdW90YQAAAAAAAAAAAg==",
        "AAAABQAAAIdVc2FnZSBtZXRlcmVkIGZvciBvbmUgbWV0cmljIGluIHRoZSBvcGVuIHBlcmlvZC4gSW5kZXhlcnMgY2FuIHJlYnVpbGQKZXZlcnkgc2V0dGxlZCBwZXJpb2QncyBvdmVyYWdlIGZyb20gdGhlc2UgcGx1cyB0aGUgcGxhbidzIHF1b3Rhcy4AAAAAAAAAAA1Vc2FnZVJlY29yZGVkAAAAAAAAAQAAAA51c2FnZV9yZWNvcmRlZAAAAAAABQAAAAAAAAAPc3Vic2NyaXB0aW9uX2lkAAAAAAYAAAABAAAAAAAAAAZtZXRyaWMAAAAAABEAAAABAAAAGVVuaXRzIGFkZGVkIGJ5IHRoaXMgY2FsbC4AAAAAAAAFdW5pdHMAAAAAAAAGAAAAAAAAADdSdW5uaW5nIHRvdGFsIGZvciB0aGUgb3BlbiBwZXJpb2QsIGluY2x1ZGluZyB0aGlzIGNhbGwuAAAAAAxwZXJpb2RfdW5pdHMAAAAGAAAAAAAAAClUaGUgYmlsbGluZyB3aW5kb3cgdGhlc2UgdW5pdHMgYmVsb25nIHRvLgAAAAAAAAxwZXJpb2Rfc3RhcnQAAAAGAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAADkJhbGFuY2VEZWJpdGVkAAAAAAABAAAAD2JhbGFuY2VfZGViaXRlZAAAAAADAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAAAAAAABmFtb3VudAAAAAAACwAAAAAAAAAAAAAADWJhbGFuY2VfYWZ0ZXIAAAAAAAALAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAAD0JhbGFuY2VSZWZ1bmRlZAAAAAABAAAAEGJhbGFuY2VfcmVmdW5kZWQAAAADAAAAAAAAAA9zdWJzY3JpcHRpb25faWQAAAAABgAAAAEAAAAAAAAABmFtb3VudAAAAAAACwAAAAAAAAAAAAAADWJhbGFuY2VfYWZ0ZXIAAAAAAAALAAAAAAAAAAI=",
        "AAAABAAAAZxTaGFyZWQgZXJyb3IgdHlwZSB1c2VkIGFjcm9zcyBhbGwgU29yb2JhbiBGb3JnZSBjb250cmFjdHMuCgpEZWZpbmluZyBhIHNpbmdsZSBlcnJvciBlbnVtIGluIGBzaGFyZWQtdXRpbHNgIGtlZXBzIHRoZSBvbi1jaGFpbiBlcnJvcgpzcGFjZSBjb25zaXN0ZW50IGFuZCBpbnRlbGxpZ2libGUgdG8gU0RLIGNvbnN1bWVycywgYW5kIGF2b2lkcyBldmVyeQpjb250cmFjdCByZS1kZWNsYXJpbmcgdGhlIHNhbWUgZmFpbHVyZSBtb2Rlcy4gQ29udHJhY3QgY3JhdGVzIG1heSBleHBvc2UKdGhlaXIgb3duIGRvbWFpbi1zcGVjaWZpYyBlcnJvcnMsIGJ1dCBzaG91bGQgcHJlZmVyIHRoZXNlIHdoZXJlIHRoZXkgZml0LgoKRXJyb3IgY29kZXMgc3RhcnQgYXQgMTsgY29kZSAwIGlzIHJlc2VydmVkIGJ5IHRoZSBTb3JvYmFuIGhvc3QuAAAAAAAAAApGb3JnZUVycm9yAAAAAAAPAAAAM1RoZSBjYWxsZXIgaXMgbm90IHBlcm1pdHRlZCB0byBwZXJmb3JtIHRoaXMgYWN0aW9uLgAAAAAMVW5hdXRob3JpemVkAAAAAQAAAEpUaGUgcmVxdWVzdGVkIGVudGl0eSAoZXNjcm93LCBwcm9wb3NhbCwgc3Vic2NyaXB0aW9uLCAuLi4pIGRvZXMgbm90IGV4aXN0LgAAAAAACE5vdEZvdW5kAAAAAgAAAEhPbmUgb3IgbW9yZSBhcmd1bWVudHMgZmFpbGVkIHZhbGlkYXRpb24gKGUuZy4gemVybyBhbW91bnQsIGJhZCBhZGRyZXNzKS4AAAAMSW52YWxpZElucHV0AAAAAwAAAENUaGUgY29udHJhY3QgZG9lcyBub3QgaG9sZCBlbm91Z2ggYmFsYW5jZSB0byBzYXRpc2Z5IHRoZSBvcGVyYXRpb24uAAAAABFJbnN1ZmZpY2llbnRGdW5kcwAAAAAAAAQAAABCVGhlIGVudGl0eSB3YXMgYWxyZWFkeSBpbml0aWFsaXNlZDsgcmUtaW5pdGlhbGlzYXRpb24gaXMgcmVqZWN0ZWQuAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAAFAAAANlRoZSBlbnRpdHkgd2FzIGV4cGVjdGVkIHRvIGJlIGluaXRpYWxpc2VkIGJ1dCB3YXMgbm90LgAAAAAADk5vdEluaXRpYWxpemVkAAAAAAAGAAAANkFuIG9wZXJhdGlvbiB3YXMgYXR0ZW1wdGVkIGFmdGVyIGl0cyBkZWFkbGluZSBlbGFwc2VkLgAAAAAAD0RlYWRsaW5lUmVhY2hlZAAAAAAHAAAAQUEgcmVxdWlyZWQgdG9rZW4gYWxsb3dhbmNlIHdhcyBsb3dlciB0aGFuIHRoZSBhbW91bnQgYmVpbmcgc3BlbnQuAAAAAAAAFUluc3VmZmljaWVudEFsbG93YW5jZQAAAAAAAAgAAAAjQW4gYXJpdGhtZXRpYyBvcGVyYXRpb24gb3ZlcmZsb3dlZC4AAAAAEkFyaXRobWV0aWNPdmVyZmxvdwAAAAAACQAAAEJDb250cmFjdC1zcGVjaWZpYyBlcnJvciB0aGF0IGRvZXMgbm90IG1hcCB0byB0aGUgY2F0ZWdvcmllcyBhYm92ZS4AAAAAAAZDdXN0b20AAAAAAAoAAAFsQSBTRVAtNDEgdG9rZW4gaW52b2NhdGlvbiBmYWlsZWQgKGluc3VmZmljaWVudCBiYWxhbmNlLCBtaXNzaW5nIG9yCmRlYXV0aG9yaXplZCB0cnVzdGxpbmUsIHVuZGVwbG95ZWQgdG9rZW4gY29udHJhY3QsIG9yIHRva2VuLWxvZ2ljCnJlamVjdGlvbikuIFRoZSByYXcgdG9rZW4gZXJyb3IgZGlzY3JpbWluYW50IGlzIGludGVudGlvbmFsbHkgbm90CmZvcndhcmRlZCDigJQgY2FsbGVycyBjYW5ub3QgdGVsbCB3aGljaCBjb250cmFjdCBwcm9kdWNlZCBhIGZvcndhcmRlZApjb2RlLCBzbyBpdCBpcyBidWNrZXRlZDsgdGhlIHJvb3QgY2F1c2UgcmVtYWlucyB2aXNpYmxlIGluIHRoZQp0cmFuc2FjdGlvbidzIGRpYWdub3N0aWMgZXZlbnRzLgAAABNUb2tlblRyYW5zZmVyRmFpbGVkAAAAAAsAAADbQSBjcm9zcy1jb250cmFjdCBpbnZvY2F0aW9uIGZhaWxlZCAodGFyZ2V0IHJldmVydGVkIG9yIGhvc3QKYWJvcnQpLiBUaGUgdGFyZ2V0IHJldmVydHMgYXJlIHN1cmZhY2VkIGhlcmUgc28gdGhlIGNhbGxlciBjYW4KZGlzdGluZ3Vpc2ggdGhlbSBmcm9tIHRva2VuLXRyYW5zZmVyIGZhaWx1cmVzLCBhbmQgdGhlIGludm9raW5nCnRyYW5zYWN0aW9uIGlzIGxlZnQgdW4tZXhlY3V0ZWQuAAAAABhDb250cmFjdEludm9jYXRpb25GYWlsZWQAAAAMAAABDFRoZSByZXF1ZXN0ZWQgd2l0aGRyYXdhbCB3b3VsZCBwdXNoIGEgdG9rZW4ncyByb2xsaW5nLXdpbmRvdyB0b3RhbApwYXN0IGl0cyBjb25maWd1cmVkIHdpdGhkcmF3YWwgbGltaXQuIEtlcHQgZGlzdGluY3QgZnJvbQpbYEZvcmdlRXJyb3I6OkludmFsaWRJbnB1dGBdIHNvIGEgY2FsbGVyIGNhbiB0ZWxsIGEgcG9saWN5IHJlamVjdGlvbgooYSB2YWxpZCB3aXRoZHJhd2FsIHRoYXQgaXMgdG9vIGxhcmdlIHJpZ2h0IG5vdykgZnJvbSBhIG1hbGZvcm1lZAphcmd1bWVudC4AAAAXV2l0aGRyYXdhbExpbWl0RXhjZWVkZWQAAAAADQAAARtBIHN1YnNjcmlwdGlvbiBpcyBpbiB0aGUgYFBhc3REdWVgIGxhcHNlZCBzdGF0ZSBhbmQgdGhlIHJlcXVlc3RlZApvcGVyYXRpb24gY2Fubm90IGJlIHBlcmZvcm1lZCB1bnRpbCBhIGNhdGNoLXVwIGNoYXJnZSByZXN0b3JlcyBpdCB0bwpgQWN0aXZlYC4gS2VwdCBkaXN0aW5jdCBmcm9tIFtgRm9yZ2VFcnJvcjo6RGVhZGxpbmVSZWFjaGVkYF0gc28gYSBjYWxsZXIKY2FuIHRlbGwgYSBsYXBzZWQgYmlsbGluZyBzdGF0ZSBmcm9tIGFuIGFyZ3VtZW50IHRoYXQgd2FzIG1lcmVseQp0b28gZWFybHkuAAAAABNTdWJzY3JpcHRpb25QYXN0RHVlAAAAAA4AAABxQSBwcm9wb3NlciBoYXMgcmVhY2hlZCB0aGUgbWF4aW11bSBhbGxvd2VkIGNvbmN1cnJlbnQgYWN0aXZlIHByb3Bvc2FscwpvciBpcyB3aXRoaW4gdGhlIHByb3Bvc2VyIGNvb2xkb3duIHdpbmRvdy4AAAAAAAAQUHJvcG9zZXJDb29sZG93bgAAAA8=" ]),
      options
    )
  }
  public readonly fromJSON = {
    pause: this.txFromJSON<Result<void>>,
        cancel: this.txFromJSON<Result<void>>,
        charge: this.txFromJSON<Result<i128>>,
        resume: this.txFromJSON<Result<void>>,
        deposit: this.txFromJSON<Result<i128>>,
        get_usage: this.txFromJSON<Result<UsageRecord>>,
        subscribe: this.txFromJSON<Result<u64>>,
        touch_ttl: this.txFromJSON<Result<void>>,
        set_quotas: this.txFromJSON<Result<void>>,
        quote_period: this.txFromJSON<Result<i128>>,
        record_usage: this.txFromJSON<Result<void>>,
        charge_catchup: this.txFromJSON<Result<i128>>,
        revoke_provider: this.txFromJSON<Result<void>>,
        get_subscription: this.txFromJSON<Result<Subscription>>,
        withdraw_balance: this.txFromJSON<Result<i128>>,
        authorize_provider: this.txFromJSON<Result<void>>,
        get_subscription_count: this.txFromJSON<u64>,
        is_provider_authorized: this.txFromJSON<boolean>,
        subscribe_on_behalf_of: this.txFromJSON<Result<u64>>,
        subscriptions_for_provider: this.txFromJSON<Result<Array<Subscription>>>,
        subscriptions_for_subscriber: this.txFromJSON<Result<Array<Subscription>>>
  }
}