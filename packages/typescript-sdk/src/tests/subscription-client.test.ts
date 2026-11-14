/**
 * Offline client, argument-encoding, return-decoding, ForgeError, and
 * failed-simulation suite for @soroban-forge subscription client.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { subscription } from "../../dist/index.js";
import type {
  MetricQuota,
  Subscription,
  SubscriptionStatus,
  UsageRecord,
} from "../../dist/clients/subscription.js";
import {
  FIXTURE_ADDR_1,
  FIXTURE_ADDR_2,
  FIXTURE_TOKEN,
  MAX_I128,
  MAX_U32,
  MAX_U64,
  TESTNET_CONFIG,
  ZERO_I128,
  ZERO_U32,
  ZERO_U64,
  contractErrorScVal,
  createClientWithFailure,
  createClientWithReturn,
  decodeScValI128,
  FixtureRpcServer,
  registerCommonClientSuite,
  successfulSimulation,
} from "./client-test-helpers.js";

registerCommonClientSuite({
  suiteName: "subscription",
  fixtureName: "subscription",
  ClientCtor: subscription.Client,
  clientForgeError: subscription.ForgeError,
});

const specClient = new subscription.Client({ ...TESTNET_CONFIG });

// Compile-time check for SubscriptionStatus union variants.
const _statuses: SubscriptionStatus[] = [
  { tag: "Active", values: undefined },
  { tag: "Cancelled", values: undefined },
  { tag: "PastDue", values: undefined },
  { tag: "Paused", values: undefined },
];
void _statuses;

// ---------------------------------------------------------------------------
// Argument encoding for all 21 subscription methods (including edge values)
// ---------------------------------------------------------------------------

test("subscription single-id methods encode subscription_id (0 and MAX_U64) as scvU64", () => {
  const singleIdMethods = [
    "cancel",
    "charge",
    "get_subscription",
    "pause",
    "quote_period",
    "resume",
    "touch_ttl",
  ] as const;

  for (const method of singleIdMethods) {
    const zeroArgs = specClient.spec.funcArgsToScVals(method, {
      subscription_id: ZERO_U64,
    });
    assert.equal(zeroArgs.length, 1);
    assert.equal(zeroArgs[0]!.switch().name, "scvU64");
    assert.equal(zeroArgs[0]!.u64().toString(), "0");

    const maxArgs = specClient.spec.funcArgsToScVals(method, {
      subscription_id: MAX_U64,
    });
    assert.equal(maxArgs[0]!.u64().toString(), MAX_U64.toString());
  }
});

test("subscription.get_subscription_count: encodes zero arguments", () => {
  const args = specClient.spec.funcArgsToScVals("get_subscription_count", {});
  assert.equal(args.length, 0);
});

test("subscription provider opt-in methods encode subscriber and provider as scvAddress", () => {
  for (const method of [
    "authorize_provider",
    "revoke_provider",
    "is_provider_authorized",
  ] as const) {
    const args = specClient.spec.funcArgsToScVals(method, {
      subscriber: FIXTURE_ADDR_1,
      provider: FIXTURE_ADDR_2,
    });
    assert.equal(args.length, 2);
    assert.equal(args[0]!.switch().name, "scvAddress");
    assert.equal(args[1]!.switch().name, "scvAddress");
  }
});

test("subscription.charge_catchup: encodes subscription_id (scvU64) and max_periods (0 and MAX_U32)", () => {
  const zeroArgs = specClient.spec.funcArgsToScVals("charge_catchup", {
    subscription_id: 1n,
    max_periods: ZERO_U32,
  });
  assert.equal(zeroArgs.length, 2);
  assert.equal(zeroArgs[0]!.switch().name, "scvU64");
  assert.equal(zeroArgs[1]!.switch().name, "scvU32");
  assert.equal(zeroArgs[1]!.u32(), 0);

  const maxArgs = specClient.spec.funcArgsToScVals("charge_catchup", {
    subscription_id: MAX_U64,
    max_periods: MAX_U32,
  });
  assert.equal(maxArgs[1]!.u32(), MAX_U32);
});

test("subscription.deposit and withdraw_balance: encode zero and MAX_I128 amounts", () => {
  for (const method of ["deposit", "withdraw_balance"] as const) {
    const zeroArgs = specClient.spec.funcArgsToScVals(method, {
      subscription_id: ZERO_U64,
      amount: ZERO_I128,
    });
    assert.equal(zeroArgs.length, 2);
    assert.equal(zeroArgs[0]!.switch().name, "scvU64");
    assert.equal(decodeScValI128(specClient.spec, zeroArgs[1]!), 0n);

    const maxArgs = specClient.spec.funcArgsToScVals(method, {
      subscription_id: MAX_U64,
      amount: MAX_I128,
    });
    assert.equal(decodeScValI128(specClient.spec, maxArgs[1]!), MAX_I128);
  }
});

test("subscription.get_usage and record_usage: encode symbol metrics and u64 units", () => {
  const usageArgs = specClient.spec.funcArgsToScVals("get_usage", {
    subscription_id: 5n,
    metric: "api_calls",
  });
  assert.equal(usageArgs.length, 2);
  assert.equal(usageArgs[0]!.switch().name, "scvU64");
  assert.equal(usageArgs[1]!.switch().name, "scvSymbol");
  assert.equal(usageArgs[1]!.sym().toString(), "api_calls");

  const recordZero = specClient.spec.funcArgsToScVals("record_usage", {
    subscription_id: ZERO_U64,
    metric: "storage_bytes",
    units: ZERO_U64,
  });
  assert.equal(recordZero.length, 3);
  assert.equal(recordZero[1]!.switch().name, "scvSymbol");
  assert.equal(recordZero[2]!.switch().name, "scvU64");
  assert.equal(recordZero[2]!.u64().toString(), "0");

  const recordMax = specClient.spec.funcArgsToScVals("record_usage", {
    subscription_id: MAX_U64,
    metric: "storage_bytes",
    units: MAX_U64,
  });
  assert.equal(recordMax[2]!.u64().toString(), MAX_U64.toString());
});

test("subscription.set_quotas: encodes empty vector, max-length vector, and Option None/Some", () => {
  const emptyArgs = specClient.spec.funcArgsToScVals("set_quotas", {
    subscription_id: 1n,
    quotas: [],
  });
  assert.equal(emptyArgs.length, 2);
  assert.equal(emptyArgs[0]!.switch().name, "scvU64");
  assert.equal(emptyArgs[1]!.switch().name, "scvVec");
  assert.equal(emptyArgs[1]!.vec()!.length, 0);

  const maxQuotas: MetricQuota[] = Array.from({ length: 16 }, (_, idx) => ({
    bucket_units: idx === 0 ? 1n : MAX_U64,
    included_units: idx === 0 ? ZERO_U64 : 1_000n,
    max_overage_units: idx % 2 === 0 ? undefined : MAX_U64,
    metric: `metric_${idx}`,
    overage_price: idx === 0 ? ZERO_I128 : MAX_I128,
  }));
  const fullArgs = specClient.spec.funcArgsToScVals("set_quotas", {
    subscription_id: 2n,
    quotas: maxQuotas,
  });
  const vecEntries = fullArgs[1]!.vec()!;
  assert.equal(vecEntries.length, 16);

  // Verify Option::None (scvVoid) vs Option::Some (scvU64) inside MetricQuota maps
  const findMapField = (entryIdx: number, fieldName: string) =>
    vecEntries[entryIdx]!
      .map()!
      .find((kv) => kv.key().sym().toString() === fieldName)!
      .val();

  assert.equal(findMapField(0, "max_overage_units").switch().name, "scvVoid");
  assert.equal(findMapField(1, "max_overage_units").switch().name, "scvU64");
  assert.equal(
    findMapField(1, "max_overage_units").u64().toString(),
    MAX_U64.toString(),
  );
});

test("subscription.subscribe and subscribe_on_behalf_of: encode addresses, i128 amount, and u64 period", () => {
  const subArgs = specClient.spec.funcArgsToScVals("subscribe", {
    subscriber: FIXTURE_ADDR_1,
    provider: FIXTURE_ADDR_2,
    token: FIXTURE_TOKEN,
    amount: MAX_I128,
    period: MAX_U64,
  });
  assert.equal(subArgs.length, 5);
  assert.equal(subArgs[0]!.switch().name, "scvAddress");
  assert.equal(subArgs[1]!.switch().name, "scvAddress");
  assert.equal(subArgs[2]!.switch().name, "scvAddress");
  assert.equal(decodeScValI128(specClient.spec, subArgs[3]!), MAX_I128);
  assert.equal(subArgs[4]!.u64().toString(), MAX_U64.toString());

  const behalfArgs = specClient.spec.funcArgsToScVals(
    "subscribe_on_behalf_of",
    {
      provider: FIXTURE_ADDR_2,
      subscriber: FIXTURE_ADDR_1,
      token: FIXTURE_TOKEN,
      amount: ZERO_I128,
      period: ZERO_U64,
    },
  );
  assert.equal(behalfArgs.length, 5);
  assert.equal(decodeScValI128(specClient.spec, behalfArgs[3]!), 0n);
  assert.equal(behalfArgs[4]!.u64().toString(), "0");
});

test("subscription pagination methods encode address and u32 offset/limit boundaries", () => {
  const provArgs = specClient.spec.funcArgsToScVals(
    "subscriptions_for_provider",
    {
      provider: FIXTURE_ADDR_2,
      offset: ZERO_U32,
      limit: MAX_U32,
    },
  );
  assert.equal(provArgs.length, 3);
  assert.equal(provArgs[0]!.switch().name, "scvAddress");
  assert.equal(provArgs[1]!.u32(), 0);
  assert.equal(provArgs[2]!.u32(), MAX_U32);

  const subArgs = specClient.spec.funcArgsToScVals(
    "subscriptions_for_subscriber",
    {
      subscriber: FIXTURE_ADDR_1,
      offset: MAX_U32,
      limit: ZERO_U32,
    },
  );
  assert.equal(subArgs.length, 3);
  assert.equal(subArgs[0]!.switch().name, "scvAddress");
  assert.equal(subArgs[1]!.u32(), MAX_U32);
  assert.equal(subArgs[2]!.u32(), 0);
});

// ---------------------------------------------------------------------------
// Return value decoding from fixture ScVals
// ---------------------------------------------------------------------------

const baseSubscriptionFixture: Subscription = {
  amount: 2_500n,
  failed_attempts: 0,
  last_charged: 1_700_000_000n,
  past_due_since: undefined,
  paused_at: undefined,
  period: 86_400n,
  prepaid_balance: undefined,
  provider: FIXTURE_ADDR_2,
  quotas: [],
  status: { tag: "Active", values: undefined },
  subscriber: FIXTURE_ADDR_1,
  subscription_id: 11n,
  token: FIXTURE_TOKEN,
};

test("subscription client decodes get_subscription with Option fields as None and Some", async () => {
  const noneCtx = createClientWithReturn(
    subscription.Client,
    specClient.spec,
    "get_subscription",
    baseSubscriptionFixture,
  );
  const noneSub = (
    await noneCtx.client.get_subscription({ subscription_id: 11n })
  ).result.unwrap();
  assert.equal(noneSub.subscription_id, 11n);
  assert.equal(noneSub.amount, 2_500n);
  assert.equal(noneSub.past_due_since, null);
  assert.equal(noneSub.paused_at, null);
  assert.equal(noneSub.prepaid_balance, null);
  assert.equal(noneSub.status.tag, "Active");

  const someFixture: Subscription = {
    ...baseSubscriptionFixture,
    past_due_since: 1_700_086_400n,
    paused_at: 1_700_090_000n,
    prepaid_balance: 10_000n,
    quotas: [
      {
        bucket_units: 100n,
        included_units: 1_000n,
        max_overage_units: 5_000n,
        metric: "api_calls",
        overage_price: 50n,
      },
    ],
    status: { tag: "PastDue", values: undefined },
  };
  const someCtx = createClientWithReturn(
    subscription.Client,
    specClient.spec,
    "get_subscription",
    someFixture,
  );
  const someSub = (
    await someCtx.client.get_subscription({ subscription_id: 11n })
  ).result.unwrap();
  assert.equal(someSub.past_due_since, 1_700_086_400n);
  assert.equal(someSub.paused_at, 1_700_090_000n);
  assert.equal(someSub.prepaid_balance, 10_000n);
  assert.equal(someSub.quotas.length, 1);
  assert.equal(someSub.quotas[0]!.max_overage_units, 5_000n);
  assert.equal(someSub.status.tag, "PastDue");
});

test("subscription client decodes get_subscription_count, is_provider_authorized, quote_period, get_usage, and page views", async () => {
  const countCtx = createClientWithReturn(
    subscription.Client,
    specClient.spec,
    "get_subscription_count",
    42n,
  );
  assert.equal((await countCtx.client.get_subscription_count()).result, 42n);

  const authCtx = createClientWithReturn(
    subscription.Client,
    specClient.spec,
    "is_provider_authorized",
    true,
  );
  assert.equal(
    (
      await authCtx.client.is_provider_authorized({
        subscriber: FIXTURE_ADDR_1,
        provider: FIXTURE_ADDR_2,
      })
    ).result,
    true,
  );

  const quoteCtx = createClientWithReturn(
    subscription.Client,
    specClient.spec,
    "quote_period",
    3_750n,
  );
  assert.equal(
    (await quoteCtx.client.quote_period({ subscription_id: 11n })).result.unwrap(),
    3_750n,
  );

  const usageFixture: UsageRecord = {
    metric: "api_calls",
    period_start: 1_700_000_000n,
    subscription_id: 11n,
    units: 1_250n,
  };
  const usageCtx = createClientWithReturn(
    subscription.Client,
    specClient.spec,
    "get_usage",
    usageFixture,
  );
  const decodedUsage = (
    await usageCtx.client.get_usage({
      subscription_id: 11n,
      metric: "api_calls",
    })
  ).result.unwrap();
  assert.equal(decodedUsage.metric, "api_calls");
  assert.equal(decodedUsage.units, 1_250n);

  const pageCtx = createClientWithReturn(
    subscription.Client,
    specClient.spec,
    "subscriptions_for_provider",
    [baseSubscriptionFixture],
  );
  const page = (
    await pageCtx.client.subscriptions_for_provider({
      provider: FIXTURE_ADDR_2,
      offset: 0,
      limit: 10,
    })
  ).result.unwrap();
  assert.equal(page.length, 1);
  assert.equal(page[0]!.subscription_id, 11n);
});

// ---------------------------------------------------------------------------
// Failed simulations and contract-level errors
// ---------------------------------------------------------------------------

test("subscription client reports failed simulation as an error rather than empty success", async () => {
  const { client, server } = createClientWithFailure(
    subscription.Client,
    "subscription simulation reverted",
  );
  const tx = await client.subscribe({
    subscriber: FIXTURE_ADDR_1,
    provider: FIXTURE_ADDR_2,
    token: FIXTURE_TOKEN,
    amount: 100n,
    period: 3_600n,
  });

  assert.equal(server.simulationCount, 1);
  assert.throws(() => tx.result, /subscription simulation reverted/);
});

test("subscription client surfaces on-chain ForgeError Result::Err", async () => {
  const server = new FixtureRpcServer(
    successfulSimulation(contractErrorScVal(14)),
  );
  const client = new subscription.Client({ ...TESTNET_CONFIG, server });
  const tx = await client.charge({ subscription_id: 11n });
  assert.equal(tx.result.isErr(), true);
  assert.throws(() => tx.result.unwrap());
});
