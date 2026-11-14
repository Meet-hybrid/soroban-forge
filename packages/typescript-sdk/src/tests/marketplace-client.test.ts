/**
 * Offline client, argument-encoding, return-decoding, ForgeError, and
 * failed-simulation suite for @soroban-forge marketplace client.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { marketplace } from "../../dist/index.js";
import type {
  Royalty,
  RoyaltyShare,
  SaleQuote,
  Settlement,
  SettlementSummary,
} from "../../dist/clients/marketplace.js";
import {
  FIXTURE_ADDR_1,
  FIXTURE_ADDR_2,
  FIXTURE_CONTRACT_ID,
  FIXTURE_TOKEN,
  FixtureRpcServer,
  MAX_I128,
  MAX_U32,
  TESTNET_CONFIG,
  ZERO_I128,
  ZERO_U32,
  contractErrorScVal,
  createClientWithFailure,
  createClientWithReturn,
  decodeScValI128,
  registerCommonClientSuite,
  successfulSimulation,
} from "./client-test-helpers.js";

registerCommonClientSuite({
  suiteName: "marketplace",
  fixtureName: "marketplace",
  ClientCtor: marketplace.Client,
  clientForgeError: marketplace.ForgeError,
});

const specClient = new marketplace.Client({ ...TESTNET_CONFIG });

// ---------------------------------------------------------------------------
// Argument encoding for all 9 marketplace methods (including edge values)
// ---------------------------------------------------------------------------

test("marketplace.distribute and settle_sale: encode addresses and zero/max i128 amounts", () => {
  for (const method of ["distribute", "settle_sale"] as const) {
    const zeroArgs = specClient.spec.funcArgsToScVals(method, {
      collection: FIXTURE_CONTRACT_ID,
      token: FIXTURE_TOKEN,
      payer: FIXTURE_ADDR_1,
      seller: FIXTURE_ADDR_2,
      amount: ZERO_I128,
    });
    assert.equal(zeroArgs.length, 5);
    assert.equal(zeroArgs[0]!.switch().name, "scvAddress");
    assert.equal(zeroArgs[1]!.switch().name, "scvAddress");
    assert.equal(zeroArgs[2]!.switch().name, "scvAddress");
    assert.equal(zeroArgs[3]!.switch().name, "scvAddress");
    assert.equal(decodeScValI128(specClient.spec, zeroArgs[4]!), 0n);

    const maxArgs = specClient.spec.funcArgsToScVals(method, {
      collection: FIXTURE_CONTRACT_ID,
      token: FIXTURE_TOKEN,
      payer: FIXTURE_ADDR_1,
      seller: FIXTURE_ADDR_2,
      amount: MAX_I128,
    });
    assert.equal(decodeScValI128(specClient.spec, maxArgs[4]!), MAX_I128);
  }
});

test("marketplace.get_royalty, get_settlement_summary, touch_ttl: encode collection as scvAddress", () => {
  for (const method of [
    "get_royalty",
    "get_settlement_summary",
    "touch_ttl",
  ] as const) {
    const args = specClient.spec.funcArgsToScVals(method, {
      collection: FIXTURE_CONTRACT_ID,
    });
    assert.equal(args.length, 1);
    assert.equal(args[0]!.switch().name, "scvAddress");
  }
});

test("marketplace.quote_sale: encodes collection and zero/max i128 amount", () => {
  const zeroArgs = specClient.spec.funcArgsToScVals("quote_sale", {
    collection: FIXTURE_CONTRACT_ID,
    amount: ZERO_I128,
  });
  assert.equal(zeroArgs.length, 2);
  assert.equal(zeroArgs[0]!.switch().name, "scvAddress");
  assert.equal(decodeScValI128(specClient.spec, zeroArgs[1]!), 0n);

  const maxArgs = specClient.spec.funcArgsToScVals("quote_sale", {
    collection: FIXTURE_CONTRACT_ID,
    amount: MAX_I128,
  });
  assert.equal(decodeScValI128(specClient.spec, maxArgs[1]!), MAX_I128);
});

test("marketplace.set_royalty: encodes collection, recipient, and zero/10_000/max u32 bps", () => {
  const zeroArgs = specClient.spec.funcArgsToScVals("set_royalty", {
    collection: FIXTURE_CONTRACT_ID,
    recipient: FIXTURE_ADDR_1,
    bps: ZERO_U32,
  });
  assert.equal(zeroArgs.length, 3);
  assert.equal(zeroArgs[0]!.switch().name, "scvAddress");
  assert.equal(zeroArgs[1]!.switch().name, "scvAddress");
  assert.equal(zeroArgs[2]!.switch().name, "scvU32");
  assert.equal(zeroArgs[2]!.u32(), 0);

  const fullArgs = specClient.spec.funcArgsToScVals("set_royalty", {
    collection: FIXTURE_CONTRACT_ID,
    recipient: FIXTURE_ADDR_1,
    bps: 10_000,
  });
  assert.equal(fullArgs[2]!.u32(), 10_000);

  const maxArgs = specClient.spec.funcArgsToScVals("set_royalty", {
    collection: FIXTURE_CONTRACT_ID,
    recipient: FIXTURE_ADDR_1,
    bps: MAX_U32,
  });
  assert.equal(maxArgs[2]!.u32(), MAX_U32);
});

test("marketplace.set_royalty_splits: encodes empty and multi-recipient RoyaltyShare vectors", () => {
  const emptyArgs = specClient.spec.funcArgsToScVals("set_royalty_splits", {
    collection: FIXTURE_CONTRACT_ID,
    recipients: [],
  });
  assert.equal(emptyArgs.length, 2);
  assert.equal(emptyArgs[0]!.switch().name, "scvAddress");
  assert.equal(emptyArgs[1]!.switch().name, "scvVec");
  assert.equal(emptyArgs[1]!.vec()!.length, 0);

  const maxShares: RoyaltyShare[] = Array.from({ length: 16 }, (_, idx) => ({
    bps: idx === 0 ? ZERO_U32 : idx === 15 ? MAX_U32 : 250,
    recipient: idx % 2 === 0 ? FIXTURE_ADDR_1 : FIXTURE_ADDR_2,
  }));
  const batchArgs = specClient.spec.funcArgsToScVals("set_royalty_splits", {
    collection: FIXTURE_CONTRACT_ID,
    recipients: maxShares,
  });
  assert.equal(batchArgs[1]!.switch().name, "scvVec");
  assert.equal(batchArgs[1]!.vec()!.length, 16);
  assert.equal(batchArgs[1]!.vec()![0]!.switch().name, "scvMap");
});

test("marketplace.settle_sales: encodes empty and batch tuple vectors of (seller, amount)", () => {
  const emptyArgs = specClient.spec.funcArgsToScVals("settle_sales", {
    collection: FIXTURE_CONTRACT_ID,
    token: FIXTURE_TOKEN,
    payer: FIXTURE_ADDR_1,
    sales: [],
  });
  assert.equal(emptyArgs.length, 4);
  assert.equal(emptyArgs[0]!.switch().name, "scvAddress");
  assert.equal(emptyArgs[1]!.switch().name, "scvAddress");
  assert.equal(emptyArgs[2]!.switch().name, "scvAddress");
  assert.equal(emptyArgs[3]!.switch().name, "scvVec");
  assert.equal(emptyArgs[3]!.vec()!.length, 0);

  const batchSales: Array<readonly [string, bigint]> = Array.from(
    { length: 16 },
    (_, idx) =>
      [
        idx % 2 === 0 ? FIXTURE_ADDR_1 : FIXTURE_ADDR_2,
        idx === 0 ? ZERO_I128 : idx === 15 ? MAX_I128 : BigInt(idx * 1_000),
      ] as const,
  );
  const fullArgs = specClient.spec.funcArgsToScVals("settle_sales", {
    collection: FIXTURE_CONTRACT_ID,
    token: FIXTURE_TOKEN,
    payer: FIXTURE_ADDR_1,
    sales: batchSales,
  });
  assert.equal(fullArgs[3]!.switch().name, "scvVec");
  assert.equal(fullArgs[3]!.vec()!.length, 16);
  const firstTuple = fullArgs[3]!.vec()![0]!;
  assert.equal(firstTuple.switch().name, "scvVec");
  assert.equal(firstTuple.vec()!.length, 2);
  assert.equal(firstTuple.vec()![0]!.switch().name, "scvAddress");
  assert.equal(decodeScValI128(specClient.spec, firstTuple.vec()![1]!), 0n);

  const lastTuple = fullArgs[3]!.vec()![15]!;
  assert.equal(
    decodeScValI128(specClient.spec, lastTuple.vec()![1]!),
    MAX_I128,
  );
});

// ---------------------------------------------------------------------------
// Return value decoding from fixture ScVals
// ---------------------------------------------------------------------------

test("marketplace client decodes get_royalty for Active and Disabled statuses", async () => {
  for (const tag of ["Active", "Disabled"] as const) {
    const royaltyFixture: Royalty = {
      bps: tag === "Active" ? 500 : 0,
      collection: FIXTURE_CONTRACT_ID,
      recipient: FIXTURE_ADDR_1,
      status: { tag, values: undefined },
    };
    const { client, server } = createClientWithReturn(
      marketplace.Client,
      specClient.spec,
      "get_royalty",
      royaltyFixture,
    );
    const decoded = (
      await client.get_royalty({ collection: FIXTURE_CONTRACT_ID })
    ).result.unwrap();
    assert.equal(server.simulationCount, 1);
    assert.equal(decoded.bps, royaltyFixture.bps);
    assert.equal(decoded.collection, FIXTURE_CONTRACT_ID);
    assert.equal(decoded.recipient, FIXTURE_ADDR_1);
    assert.equal(decoded.status.tag, tag);
  }
});

test("marketplace client decodes quote_sale and get_settlement_summary structs", async () => {
  const quoteFixture: SaleQuote = {
    gross: 10_000n,
    royalty_amount: 750n,
    royalty_bps: 750,
    seller_net: 9_250n,
  };
  const quoteCtx = createClientWithReturn(
    marketplace.Client,
    specClient.spec,
    "quote_sale",
    quoteFixture,
  );
  const decodedQuote = (
    await quoteCtx.client.quote_sale({
      collection: FIXTURE_CONTRACT_ID,
      amount: 10_000n,
    })
  ).result.unwrap();
  assert.equal(decodedQuote.gross, 10_000n);
  assert.equal(decodedQuote.royalty_amount, 750n);
  assert.equal(decodedQuote.royalty_bps, 750);
  assert.equal(decodedQuote.seller_net, 9_250n);

  const summaryFixture: SettlementSummary = {
    gross_volume: MAX_I128,
    royalties_paid: 50_000n,
    sales: 12,
  };
  const summaryCtx = createClientWithReturn(
    marketplace.Client,
    specClient.spec,
    "get_settlement_summary",
    summaryFixture,
  );
  const decodedSummary = (
    await summaryCtx.client.get_settlement_summary({
      collection: FIXTURE_CONTRACT_ID,
    })
  ).result.unwrap();
  assert.equal(decodedSummary.gross_volume, MAX_I128);
  assert.equal(decodedSummary.royalties_paid, 50_000n);
  assert.equal(decodedSummary.sales, 12);
});

test("marketplace client decodes distribute, settle_sale, and settle_sales results", async () => {
  const distCtx = createClientWithReturn(
    marketplace.Client,
    specClient.spec,
    "distribute",
    9_500n,
  );
  const sellerNet = (
    await distCtx.client.distribute({
      collection: FIXTURE_CONTRACT_ID,
      token: FIXTURE_TOKEN,
      payer: FIXTURE_ADDR_1,
      seller: FIXTURE_ADDR_2,
      amount: 10_000n,
    })
  ).result.unwrap();
  assert.equal(sellerNet, 9_500n);

  const singleSettlement: Settlement = {
    royalty_share: 500n,
    seller_net: 9_500n,
  };
  const settleCtx = createClientWithReturn(
    marketplace.Client,
    specClient.spec,
    "settle_sale",
    singleSettlement,
  );
  const decodedSingle = (
    await settleCtx.client.settle_sale({
      collection: FIXTURE_CONTRACT_ID,
      token: FIXTURE_TOKEN,
      payer: FIXTURE_ADDR_1,
      seller: FIXTURE_ADDR_2,
      amount: 10_000n,
    })
  ).result.unwrap();
  assert.equal(decodedSingle.royalty_share, 500n);
  assert.equal(decodedSingle.seller_net, 9_500n);

  const batchSettlements: Settlement[] = [
    { royalty_share: 100n, seller_net: 1_900n },
    { royalty_share: 250n, seller_net: 4_750n },
  ];
  const batchCtx = createClientWithReturn(
    marketplace.Client,
    specClient.spec,
    "settle_sales",
    batchSettlements,
  );
  const decodedBatch = (
    await batchCtx.client.settle_sales({
      collection: FIXTURE_CONTRACT_ID,
      token: FIXTURE_TOKEN,
      payer: FIXTURE_ADDR_1,
      sales: [
        [FIXTURE_ADDR_2, 2_000n],
        [FIXTURE_ADDR_2, 5_000n],
      ],
    })
  ).result.unwrap();
  assert.equal(decodedBatch.length, 2);
  assert.equal(decodedBatch[0]!.royalty_share, 100n);
  assert.equal(decodedBatch[1]!.seller_net, 4_750n);
});

// ---------------------------------------------------------------------------
// Failed simulations and contract-level errors
// ---------------------------------------------------------------------------

test("marketplace client reports a failed simulation as an error rather than empty success", async () => {
  const { client, server } = createClientWithFailure(
    marketplace.Client,
    "marketplace simulation reverted",
  );
  const tx = await client.quote_sale({
    collection: FIXTURE_CONTRACT_ID,
    amount: 1_000n,
  });
  assert.equal(server.simulationCount, 1);
  assert.throws(() => tx.result, /marketplace simulation reverted/);
});

test("marketplace client decodes on-chain ForgeError Result::Err without silent success", async () => {
  const server = new FixtureRpcServer(
    successfulSimulation(contractErrorScVal(2)),
  );
  const client = new marketplace.Client({ ...TESTNET_CONFIG, server });
  const tx = await client.get_royalty({ collection: FIXTURE_CONTRACT_ID });
  assert.equal(tx.result.isErr(), true);
  assert.throws(() => tx.result.unwrap());
});
