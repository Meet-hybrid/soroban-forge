/**
 * Offline tests for the generated @soroban-forge/sdk subscription-payments client.
 *
 * These tests run entirely without network access, private keys, funded
 * accounts, or testnet credentials. They verify:
 *
 *   1. Client construction and configuration surface
 *   2. Generated subscription-payments methods and fromJSON helpers
 *   3. ABI shape — function names, parameter names, and parameter types
 *   4. Argument encoding via ContractSpec.funcArgsToScVals() for subscribe,
 *      authorize_provider, charge, charge_catchup, deposit, withdraw_balance,
 *      set_quotas (including complex Vec<MetricQuota> structures), and views
 *   5. The ForgeError runtime object (all codes 1–15 and subscription-specific
 *      error codes such as SubscriptionPastDue)
 *   6. ContractSpec event definitions for the subscription lifecycle
 *   7. Compile-time usability of type-only exports (Subscription, UsageRecord,
 *      MetricQuota, SubscriptionStatus)
 *
 * Run:
 *   npm test --prefix packages/typescript-sdk
 */

import test from "node:test";
import assert from "node:assert/strict";
import { StrKey } from "@stellar/stellar-sdk";
import {
  subscription,
  networks,
  ForgeError,
  contract,
} from "../../dist/index.js";
import type {
  Subscription,
  SubscriptionStatus,
  MetricQuota,
  UsageRecord,
} from "../../dist/clients/subscription.js";

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

/** Deterministic, valid Stellar contract address used as a "subscriber" fixture. */
const FIXTURE_SUBSCRIBER =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";

/** Deterministic, valid Stellar contract address used as a "provider" fixture. */
const FIXTURE_PROVIDER =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

/** Deterministic, valid Stellar contract address used as a SEP-41 token fixture. */
const FIXTURE_TOKEN =
  "CDBCCKT7RBUAJEGVZDBUNMADCJW4RV2F2CJKIG22B4R4STGN37W52ZXN";

/** The Stellar testnet network passphrase. */
const TESTNET_PASSPHRASE = "Test SDF Network ; September 2015";

/** A dummy RPC URL — construction must succeed without a live endpoint. */
const DUMMY_RPC_URL = "https://soroban-testnet.stellar.org";

function createSubscriptionClient(): subscription.Client {
  return new subscription.Client({
    ...networks.testnet,
    rpcUrl: DUMMY_RPC_URL,
  });
}

// ---------------------------------------------------------------------------
// 1. Client construction
// ---------------------------------------------------------------------------

test("subscription.Client can be constructed with testnet network config", () => {
  const client = createSubscriptionClient();
  assert.ok(
    client instanceof subscription.Client,
    "constructed value should be a subscription.Client instance",
  );
});

test("deterministic mock addresses are valid Stellar contract addresses", () => {
  for (const addr of [FIXTURE_SUBSCRIBER, FIXTURE_PROVIDER, FIXTURE_TOKEN]) {
    assert.ok(
      StrKey.isValidContract(addr),
      `${addr} must be a valid Stellar contract (C…) address`,
    );
  }
  assert.equal(networks.testnet.networkPassphrase, TESTNET_PASSPHRASE);
});

// ---------------------------------------------------------------------------
// 2. Method surface — subscription-payments methods must be present
// ---------------------------------------------------------------------------

const EXPECTED_METHODS = [
  "subscribe",
  "authorize_provider",
  "revoke_provider",
  "is_provider_authorized",
  "subscribe_on_behalf_of",
  "charge",
  "charge_catchup",
  "deposit",
  "withdraw_balance",
  "set_quotas",
  "record_usage",
  "get_usage",
  "quote_period",
  "pause",
  "resume",
  "cancel",
  "get_subscription",
  "get_subscription_count",
  "subscriptions_for_subscriber",
  "subscriptions_for_provider",
] as const;

test("subscription.Client exposes all expected subscription methods", () => {
  const client = createSubscriptionClient();
  for (const method of EXPECTED_METHODS) {
    assert.equal(
      typeof (client as unknown as Record<string, unknown>)[method],
      "function",
      `client.${method} must be a function`,
    );
  }
});

test("fromJSON exposes deserialization helpers for all expected subscription methods", () => {
  const client = createSubscriptionClient();
  for (const method of EXPECTED_METHODS) {
    assert.equal(
      typeof client.fromJSON[method],
      "function",
      `client.fromJSON.${method} must be a function`,
    );
  }
});

// ---------------------------------------------------------------------------
// 3. ABI shape — function names, parameter names and types (drift detection)
// ---------------------------------------------------------------------------

test("ContractSpec is accessible via client.spec", () => {
  const client = createSubscriptionClient();
  assert.ok(
    client.spec instanceof contract.Spec,
    "client.spec must be a ContractSpec instance",
  );
});

test("ABI exposes all expected subscription function names", () => {
  const client = createSubscriptionClient();
  const funcNames = client.spec.funcs().map((f) => f.name().toString());
  for (const expected of EXPECTED_METHODS) {
    assert.ok(
      funcNames.includes(expected),
      `ABI must include function "${expected}"; found: ${funcNames.join(", ")}`,
    );
  }
});

const EXPECTED_ABI_SHAPE: Record<string, Array<[string, string]>> = {
  subscribe: [
    ["subscriber", "scSpecTypeAddress"],
    ["provider", "scSpecTypeAddress"],
    ["token", "scSpecTypeAddress"],
    ["amount", "scSpecTypeI128"],
    ["period", "scSpecTypeU64"],
  ],
  authorize_provider: [
    ["subscriber", "scSpecTypeAddress"],
    ["provider", "scSpecTypeAddress"],
  ],
  revoke_provider: [
    ["subscriber", "scSpecTypeAddress"],
    ["provider", "scSpecTypeAddress"],
  ],
  is_provider_authorized: [
    ["subscriber", "scSpecTypeAddress"],
    ["provider", "scSpecTypeAddress"],
  ],
  subscribe_on_behalf_of: [
    ["provider", "scSpecTypeAddress"],
    ["subscriber", "scSpecTypeAddress"],
    ["token", "scSpecTypeAddress"],
    ["amount", "scSpecTypeI128"],
    ["period", "scSpecTypeU64"],
  ],
  charge: [["subscription_id", "scSpecTypeU64"]],
  charge_catchup: [
    ["subscription_id", "scSpecTypeU64"],
    ["max_periods", "scSpecTypeU32"],
  ],
  deposit: [
    ["subscription_id", "scSpecTypeU64"],
    ["amount", "scSpecTypeI128"],
  ],
  withdraw_balance: [
    ["subscription_id", "scSpecTypeU64"],
    ["amount", "scSpecTypeI128"],
  ],
  set_quotas: [
    ["subscription_id", "scSpecTypeU64"],
    ["quotas", "scSpecTypeVec"],
  ],
  record_usage: [
    ["subscription_id", "scSpecTypeU64"],
    ["metric", "scSpecTypeSymbol"],
    ["units", "scSpecTypeU64"],
  ],
  get_usage: [
    ["subscription_id", "scSpecTypeU64"],
    ["metric", "scSpecTypeSymbol"],
  ],
  quote_period: [["subscription_id", "scSpecTypeU64"]],
  pause: [["subscription_id", "scSpecTypeU64"]],
  resume: [["subscription_id", "scSpecTypeU64"]],
  cancel: [["subscription_id", "scSpecTypeU64"]],
  get_subscription: [["subscription_id", "scSpecTypeU64"]],
  get_subscription_count: [],
  subscriptions_for_subscriber: [
    ["subscriber", "scSpecTypeAddress"],
    ["offset", "scSpecTypeU32"],
    ["limit", "scSpecTypeU32"],
  ],
  subscriptions_for_provider: [
    ["provider", "scSpecTypeAddress"],
    ["offset", "scSpecTypeU32"],
    ["limit", "scSpecTypeU32"],
  ],
};

test("ABI parameter names and types match expected shape", () => {
  const client = createSubscriptionClient();
  for (const fn of client.spec.funcs()) {
    const name = fn.name().toString();
    const expected = EXPECTED_ABI_SHAPE[name];
    if (!expected) continue;
    const inputs = fn.inputs();
    assert.equal(
      inputs.length,
      expected.length,
      `${name}: parameter count mismatch`,
    );
    for (let i = 0; i < expected.length; i++) {
      const [expectedParamName, expectedTypeName] = expected[i]!;
      const input = inputs[i]!;
      assert.equal(
        input.name().toString(),
        expectedParamName,
        `${name} param[${i}] name mismatch`,
      );
      assert.equal(
        input.type().switch().name,
        expectedTypeName,
        `${name} param[${i}] type mismatch`,
      );
    }
  }
});

// ---------------------------------------------------------------------------
// 4. Argument encoding tests — ContractSpec.funcArgsToScVals()
// ---------------------------------------------------------------------------

test("subscribe: encodes subscriber, provider, token (scvAddress), amount (scvI128), and period (scvU64)", () => {
  const client = createSubscriptionClient();
  const args = client.spec.funcArgsToScVals("subscribe", {
    subscriber: FIXTURE_SUBSCRIBER,
    provider: FIXTURE_PROVIDER,
    token: FIXTURE_TOKEN,
    amount: 2_500n,
    period: 2_592_000n,
  });

  assert.equal(args.length, 5, "subscribe must encode 5 arguments");
  assert.equal(args[0]!.switch().name, "scvAddress", "subscriber must be scvAddress");
  assert.equal(args[1]!.switch().name, "scvAddress", "provider must be scvAddress");
  assert.equal(args[2]!.switch().name, "scvAddress", "token must be scvAddress");
  assert.equal(args[3]!.switch().name, "scvI128", "amount must be scvI128");
  assert.equal(args[3]!.i128().lo().toString(), "2500", "amount lo must equal 2500");
  assert.equal(args[4]!.switch().name, "scvU64", "period must be scvU64");
  assert.equal(args[4]!.u64().toString(), "2592000", "period must equal 2592000");
});

test("authorize_provider: encodes subscriber (scvAddress) and provider (scvAddress)", () => {
  const client = createSubscriptionClient();
  const args = client.spec.funcArgsToScVals("authorize_provider", {
    subscriber: FIXTURE_SUBSCRIBER,
    provider: FIXTURE_PROVIDER,
  });

  assert.equal(args.length, 2, "authorize_provider must encode 2 arguments");
  assert.equal(args[0]!.switch().name, "scvAddress");
  assert.equal(args[1]!.switch().name, "scvAddress");
});

test("charge: encodes subscription_id as scvU64", () => {
  const client = createSubscriptionClient();
  const args = client.spec.funcArgsToScVals("charge", {
    subscription_id: 17n,
  });

  assert.equal(args.length, 1);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "17");
});

test("charge_catchup: encodes subscription_id (scvU64) and max_periods (scvU32)", () => {
  const client = createSubscriptionClient();
  const args = client.spec.funcArgsToScVals("charge_catchup", {
    subscription_id: 9n,
    max_periods: 6,
  });

  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "9");
  assert.equal(args[1]!.switch().name, "scvU32");
  assert.equal(args[1]!.u32(), 6);
});

test("deposit: encodes subscription_id (scvU64) and amount (scvI128)", () => {
  const client = createSubscriptionClient();
  const args = client.spec.funcArgsToScVals("deposit", {
    subscription_id: 4n,
    amount: 10_000n,
  });

  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "4");
  assert.equal(args[1]!.switch().name, "scvI128");
  assert.equal(args[1]!.i128().lo().toString(), "10000");
});

test("withdraw_balance: encodes subscription_id (scvU64) and amount (scvI128)", () => {
  const client = createSubscriptionClient();
  const args = client.spec.funcArgsToScVals("withdraw_balance", {
    subscription_id: 4n,
    amount: 3_500n,
  });

  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "4");
  assert.equal(args[1]!.switch().name, "scvI128");
  assert.equal(args[1]!.i128().lo().toString(), "3500");
});

test("set_quotas: encodes subscription_id (scvU64) and complex Vec<MetricQuota> with capped and uncapped entries", () => {
  const client = createSubscriptionClient();
  const quotas: Array<MetricQuota> = [
    {
      metric: "api_calls",
      included_units: 10_000n,
      overage_price: 5n,
      bucket_units: 1_000n,
      max_overage_units: 50_000n,
    },
    {
      metric: "storage_gb",
      included_units: 100n,
      overage_price: 25n,
      bucket_units: 10n,
      max_overage_units: undefined,
    },
  ];

  const args = client.spec.funcArgsToScVals("set_quotas", {
    subscription_id: 12n,
    quotas,
  });

  assert.equal(args.length, 2, "set_quotas must encode 2 arguments");
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "12");
  assert.equal(args[1]!.switch().name, "scvVec");

  const vec = args[1]!.vec();
  assert.ok(vec, "encoded quotas vector must be present");
  assert.equal(vec.length, 2, "quotas vector must contain 2 MetricQuota maps");

  // First quota: capped max_overage_units
  const firstQuota = vec[0]!;
  assert.equal(firstQuota.switch().name, "scvMap");
  const firstEntries = firstQuota.map()!;
  const firstKeys = firstEntries.map((e) => e.key().sym().toString());
  assert.deepEqual(firstKeys, [
    "bucket_units",
    "included_units",
    "max_overage_units",
    "metric",
    "overage_price",
  ]);
  assert.equal(firstEntries[0]!.val().switch().name, "scvU64");
  assert.equal(firstEntries[0]!.val().u64().toString(), "1000");
  assert.equal(firstEntries[1]!.val().switch().name, "scvU64");
  assert.equal(firstEntries[1]!.val().u64().toString(), "10000");
  assert.equal(firstEntries[2]!.val().switch().name, "scvU64");
  assert.equal(firstEntries[2]!.val().u64().toString(), "50000");
  assert.equal(firstEntries[3]!.val().switch().name, "scvSymbol");
  assert.equal(firstEntries[3]!.val().sym().toString(), "api_calls");
  assert.equal(firstEntries[4]!.val().switch().name, "scvI128");
  assert.equal(firstEntries[4]!.val().i128().lo().toString(), "5");

  // Second quota: uncapped (max_overage_units -> scvVoid)
  const secondQuota = vec[1]!;
  assert.equal(secondQuota.switch().name, "scvMap");
  const secondEntries = secondQuota.map()!;
  assert.equal(secondEntries[2]!.key().sym().toString(), "max_overage_units");
  assert.equal(secondEntries[2]!.val().switch().name, "scvVoid");
  assert.equal(secondEntries[3]!.val().sym().toString(), "storage_gb");
});

test("set_quotas: encodes empty Vec<MetricQuota> for flat pricing reset", () => {
  const client = createSubscriptionClient();
  const args = client.spec.funcArgsToScVals("set_quotas", {
    subscription_id: 12n,
    quotas: [],
  });

  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[1]!.switch().name, "scvVec");
  assert.equal(args[1]!.vec()!.length, 0);
});

test("record_usage and get_usage: encode subscription_id (scvU64), metric (scvSymbol), and units (scvU64)", () => {
  const client = createSubscriptionClient();
  const recordArgs = client.spec.funcArgsToScVals("record_usage", {
    subscription_id: 8n,
    metric: "api_calls",
    units: 1_500n,
  });

  assert.equal(recordArgs.length, 3);
  assert.equal(recordArgs[0]!.switch().name, "scvU64");
  assert.equal(recordArgs[0]!.u64().toString(), "8");
  assert.equal(recordArgs[1]!.switch().name, "scvSymbol");
  assert.equal(recordArgs[1]!.sym().toString(), "api_calls");
  assert.equal(recordArgs[2]!.switch().name, "scvU64");
  assert.equal(recordArgs[2]!.u64().toString(), "1500");

  const getUsageArgs = client.spec.funcArgsToScVals("get_usage", {
    subscription_id: 8n,
    metric: "api_calls",
  });
  assert.equal(getUsageArgs.length, 2);
  assert.equal(getUsageArgs[0]!.switch().name, "scvU64");
  assert.equal(getUsageArgs[1]!.switch().name, "scvSymbol");
});

// ---------------------------------------------------------------------------
// 5. ForgeError surface (including subscription-specific error codes)
// ---------------------------------------------------------------------------

const EXPECTED_FORGE_ERRORS: Record<number, string> = {
  1: "Unauthorized",
  2: "NotFound",
  3: "InvalidInput",
  4: "InsufficientFunds",
  5: "AlreadyInitialized",
  6: "NotInitialized",
  7: "DeadlineReached",
  8: "InsufficientAllowance",
  9: "ArithmeticOverflow",
  10: "Custom",
  11: "TokenTransferFailed",
  12: "ContractInvocationFailed",
  13: "WithdrawalLimitExceeded",
  14: "SubscriptionPastDue",
  15: "ProposerCooldown",
};

test("subscription.ForgeError and root ForgeError expose all 15 error codes (1–15)", () => {
  assert.equal(typeof subscription.ForgeError, "object");
  assert.ok(subscription.ForgeError !== null);

  const subCodes = Object.keys(subscription.ForgeError)
    .map(Number)
    .sort((a, b) => a - b);
  assert.deepEqual(
    subCodes,
    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
  );

  for (const [code, expectedMessage] of Object.entries(EXPECTED_FORGE_ERRORS)) {
    const numericCode = Number(code) as keyof typeof subscription.ForgeError;
    assert.equal(
      subscription.ForgeError[numericCode].message,
      expectedMessage,
      `subscription.ForgeError[${code}].message mismatch`,
    );
    assert.equal(
      ForgeError[numericCode].message,
      expectedMessage,
      `ForgeError[${code}].message mismatch`,
    );
  }
});

test("subscription-specific ForgeError codes map accurately to descriptive strings", () => {
  assert.equal(subscription.ForgeError[1].message, "Unauthorized");
  assert.equal(subscription.ForgeError[2].message, "NotFound");
  assert.equal(subscription.ForgeError[3].message, "InvalidInput");
  assert.equal(subscription.ForgeError[4].message, "InsufficientFunds");
  assert.equal(subscription.ForgeError[9].message, "ArithmeticOverflow");
  assert.equal(subscription.ForgeError[11].message, "TokenTransferFailed");
  assert.equal(subscription.ForgeError[14].message, "SubscriptionPastDue");
});

// ---------------------------------------------------------------------------
// 6. Event types
// ---------------------------------------------------------------------------

const EXPECTED_SUBSCRIPTION_EVENTS = [
  "Charged",
  "Cancelled",
  "Deposited",
  "QuotasSet",
  "UsageRecorded",
  "BalanceDebited",
  "BalanceRefunded",
] as const;

test("ContractSpec includes all documented subscription event types", () => {
  const client = createSubscriptionClient();
  const events = client.spec.events();
  const eventNames = events.map((e) => e.name().toString());
  for (const expected of EXPECTED_SUBSCRIPTION_EVENTS) {
    assert.ok(
      eventNames.includes(expected),
      `spec must include event "${expected}"; found: ${eventNames.join(", ")}`,
    );
  }
});

// ---------------------------------------------------------------------------
// 7. Type-level compile-time checks for Subscription and UsageRecord
// ---------------------------------------------------------------------------

const _statusActive: SubscriptionStatus = { tag: "Active", values: undefined };
const _statusCancelled: SubscriptionStatus = {
  tag: "Cancelled",
  values: undefined,
};
const _statusPastDue: SubscriptionStatus = {
  tag: "PastDue",
  values: undefined,
};
const _statusPaused: SubscriptionStatus = { tag: "Paused", values: undefined };

const _metricQuotaShape: MetricQuota = {
  metric: "api_calls",
  included_units: 10_000n,
  overage_price: 5n,
  bucket_units: 1_000n,
  max_overage_units: 50_000n,
};

const _subscriptionShape: Subscription = {
  subscription_id: 1n,
  subscriber: FIXTURE_SUBSCRIBER,
  provider: FIXTURE_PROVIDER,
  token: FIXTURE_TOKEN,
  amount: 2_500n,
  period: 2_592_000n,
  last_charged: 1_700_000_000n,
  status: _statusActive,
  paused_at: undefined,
  failed_attempts: 0,
  past_due_since: undefined,
  quotas: [_metricQuotaShape],
  prepaid_balance: 10_000n,
};

const _usageRecordShape: UsageRecord = {
  subscription_id: 1n,
  metric: "api_calls",
  units: 12_500n,
  period_start: 1_700_000_000n,
};

void _statusActive;
void _statusCancelled;
void _statusPastDue;
void _statusPaused;
void _metricQuotaShape;
void _subscriptionShape;
void _usageRecordShape;

test("Subscription and UsageRecord compile-time type definitions match contract structures", () => {
  assert.equal(_subscriptionShape.subscription_id, 1n);
  assert.equal(_subscriptionShape.quotas.length, 1);
  assert.equal(_subscriptionShape.quotas[0]!.metric, "api_calls");
  assert.equal(_usageRecordShape.units, 12_500n);
});
