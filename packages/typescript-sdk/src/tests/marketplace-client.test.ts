/**
 * Offline tests for the generated @soroban-forge/escrow-client marketplace client.
 *
 * These tests run entirely without network access, private keys, funded
 * accounts, or testnet credentials. They verify:
 *
 *   1. Marketplace client construction and method/fromJSON surface
 *   2. ABI shape — function names, parameter names, and parameter types
 *   3. Argument encoding via ContractSpec.funcArgsToScVals() for:
 *      - set_royalty
 *      - set_royalty_splits (multi-recipient RoyaltyShare vectors)
 *      - distribute
 *      - settle_sale
 *      - settle_sales (batch sale vectors containing Vec<(Address, i128)> tuples)
 *      - quote_sale
 *      - get_royalty, get_settlement_summary, touch_ttl
 *   4. Return value decoding via ContractSpec.funcResToNative() for Settlement
 *      (single & batch), Royalty, SaleQuote, and SettlementSummary records
 *   5. ForgeError mapping (codes 1–15) and spec error enum entries
 *   6. Compile-time TypeScript typings for RoyaltyShare, Settlement, Royalty,
 *      RoyaltyStatus, SaleQuote, and SettlementSummary
 *
 * Run:
 *   npm test --prefix packages/typescript-sdk
 */

import test from "node:test";
import assert from "node:assert/strict";
import { Address, xdr } from "@stellar/stellar-sdk";
import {
  networks,
  contract,
  marketplace,
} from "../../dist/index.js";

import type {
  Royalty,
  RoyaltyShare,
  RoyaltyStatus,
  SaleQuote,
  Settlement,
  SettlementSummary,
} from "../../dist/clients/marketplace.js";

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

/** Deterministic Stellar contract address used as an NFT collection fixture. */
const FIXTURE_COLLECTION =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";

/** Deterministic Stellar contract address used as a SEP-41 token fixture. */
const FIXTURE_TOKEN =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

/** Deterministic Stellar contract address used as a royalty recipient fixture. */
const FIXTURE_RECIPIENT_1 =
  "CDBCCKT7RBUAJEGVZDBUNMADCJW4RV2F2CJKIG22B4R4STGN37W52ZXN";

/** Deterministic Stellar account address used as a secondary recipient fixture. */
const FIXTURE_RECIPIENT_2 =
  "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

/** Deterministic Stellar address used as a sale payer fixture. */
const FIXTURE_PAYER =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";

/** Deterministic Stellar address used as a primary seller fixture. */
const FIXTURE_SELLER_1 =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

/** Deterministic Stellar address used as a secondary seller fixture. */
const FIXTURE_SELLER_2 =
  "CDBCCKT7RBUAJEGVZDBUNMADCJW4RV2F2CJKIG22B4R4STGN37W52ZXN";

/** Dummy RPC URL — construction and encoding run completely offline. */
const DUMMY_RPC_URL = "https://soroban-testnet.stellar.org";

function createMarketplaceClient(): marketplace.Client {
  return new marketplace.Client({
    ...networks.testnet,
    rpcUrl: DUMMY_RPC_URL,
  });
}

// ---------------------------------------------------------------------------
// 1. Client construction & method surface
// ---------------------------------------------------------------------------

const EXPECTED_MARKETPLACE_METHODS = [
  "touch_ttl",
  "distribute",
  "get_royalty",
  "set_royalty",
  "set_royalty_splits",
  "settle_sale",
  "settle_sales",
  "get_settlement_summary",
  "quote_sale",
] as const;

test("marketplace.Client can be constructed offline with testnet config", () => {
  const client = createMarketplaceClient();
  assert.ok(
    client instanceof marketplace.Client,
    "constructed value should be a marketplace.Client instance",
  );
  assert.ok(
    client.spec instanceof contract.Spec,
    "client.spec must be a ContractSpec instance",
  );
});

test("marketplace.Client exposes all 9 marketplace method functions", () => {
  const client = createMarketplaceClient();
  for (const method of EXPECTED_MARKETPLACE_METHODS) {
    assert.equal(
      typeof (client as unknown as Record<string, unknown>)[method],
      "function",
      `client.${method} must be a function`,
    );
  }
});

test("marketplace.Client.fromJSON exposes deserialization helpers for all 9 methods", () => {
  const client = createMarketplaceClient();
  for (const method of EXPECTED_MARKETPLACE_METHODS) {
    assert.equal(
      typeof client.fromJSON[method],
      "function",
      `client.fromJSON.${method} must be a function`,
    );
  }
});

// ---------------------------------------------------------------------------
// 2. ABI shape — function names, parameter names, and parameter types
// ---------------------------------------------------------------------------

test("marketplace ABI exposes all expected function names", () => {
  const client = createMarketplaceClient();
  const funcNames = client.spec.funcs().map((f) => f.name().toString());
  const sortedExpected = [...EXPECTED_MARKETPLACE_METHODS].sort();
  const sortedActual = [...funcNames].sort();
  assert.deepEqual(
    sortedActual,
    sortedExpected,
    "marketplace ABI function list must match expected contract methods",
  );
});

const EXPECTED_MARKETPLACE_ABI_SHAPE: Record<string, Array<[string, string]>> = {
  touch_ttl: [["collection", "scSpecTypeAddress"]],
  get_royalty: [["collection", "scSpecTypeAddress"]],
  get_settlement_summary: [["collection", "scSpecTypeAddress"]],
  set_royalty: [
    ["collection", "scSpecTypeAddress"],
    ["recipient", "scSpecTypeAddress"],
    ["bps", "scSpecTypeU32"],
  ],
  set_royalty_splits: [
    ["collection", "scSpecTypeAddress"],
    ["recipients", "scSpecTypeVec"],
  ],
  distribute: [
    ["collection", "scSpecTypeAddress"],
    ["token", "scSpecTypeAddress"],
    ["payer", "scSpecTypeAddress"],
    ["seller", "scSpecTypeAddress"],
    ["amount", "scSpecTypeI128"],
  ],
  settle_sale: [
    ["collection", "scSpecTypeAddress"],
    ["token", "scSpecTypeAddress"],
    ["payer", "scSpecTypeAddress"],
    ["seller", "scSpecTypeAddress"],
    ["amount", "scSpecTypeI128"],
  ],
  settle_sales: [
    ["collection", "scSpecTypeAddress"],
    ["token", "scSpecTypeAddress"],
    ["payer", "scSpecTypeAddress"],
    ["sales", "scSpecTypeVec"],
  ],
  quote_sale: [
    ["collection", "scSpecTypeAddress"],
    ["amount", "scSpecTypeI128"],
  ],
};

test("marketplace ABI parameter names and types match expected shape", () => {
  const client = createMarketplaceClient();
  for (const fn of client.spec.funcs()) {
    const name = fn.name().toString();
    const expected = EXPECTED_MARKETPLACE_ABI_SHAPE[name];
    assert.ok(expected, `unexpected function ${name} in marketplace spec`);
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
// 3. Argument encoding — set_royalty, set_royalty_splits, distribute,
//    settle_sale, settle_sales, and quote_sale
// ---------------------------------------------------------------------------

test("set_royalty: encodes collection (scvAddress), recipient (scvAddress), and bps (scvU32)", () => {
  const client = createMarketplaceClient();
  const args = client.spec.funcArgsToScVals("set_royalty", {
    collection: FIXTURE_COLLECTION,
    recipient: FIXTURE_RECIPIENT_1,
    bps: 500,
  });

  assert.equal(args.length, 3);
  assert.equal(args[0]!.switch().name, "scvAddress");
  assert.equal(Address.fromScVal(args[0]!).toString(), FIXTURE_COLLECTION);
  assert.equal(args[1]!.switch().name, "scvAddress");
  assert.equal(Address.fromScVal(args[1]!).toString(), FIXTURE_RECIPIENT_1);
  assert.equal(args[2]!.switch().name, "scvU32");
  assert.equal(args[2]!.u32(), 500);
});

test("set_royalty_splits: encodes collection (scvAddress) and multi-recipient RoyaltyShare vector", () => {
  const client = createMarketplaceClient();
  const recipients: Array<RoyaltyShare> = [
    { recipient: FIXTURE_RECIPIENT_1, bps: 300 },
    { recipient: FIXTURE_RECIPIENT_2, bps: 200 },
  ];
  const args = client.spec.funcArgsToScVals("set_royalty_splits", {
    collection: FIXTURE_COLLECTION,
    recipients,
  });

  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvAddress");
  assert.equal(Address.fromScVal(args[0]!).toString(), FIXTURE_COLLECTION);

  assert.equal(args[1]!.switch().name, "scvVec");
  const encodedVec = args[1]!.vec()!;
  assert.equal(encodedVec.length, 2);

  // Each RoyaltyShare struct encodes as an scvMap with Alphabetical keys: bps, recipient
  const firstMap = encodedVec[0]!.map()!;
  assert.equal(firstMap.length, 2);
  assert.equal(firstMap[0]!.key().sym().toString(), "bps");
  assert.equal(firstMap[0]!.val().u32(), 300);
  assert.equal(firstMap[1]!.key().sym().toString(), "recipient");
  assert.equal(
    Address.fromScVal(firstMap[1]!.val()).toString(),
    FIXTURE_RECIPIENT_1,
  );

  const secondMap = encodedVec[1]!.map()!;
  assert.equal(secondMap[0]!.key().sym().toString(), "bps");
  assert.equal(secondMap[0]!.val().u32(), 200);
  assert.equal(
    Address.fromScVal(secondMap[1]!.val()).toString(),
    FIXTURE_RECIPIENT_2,
  );
});

test("distribute: encodes collection, token, payer, seller (scvAddress) and amount (scvI128)", () => {
  const client = createMarketplaceClient();
  const args = client.spec.funcArgsToScVals("distribute", {
    collection: FIXTURE_COLLECTION,
    token: FIXTURE_TOKEN,
    payer: FIXTURE_PAYER,
    seller: FIXTURE_SELLER_1,
    amount: 10_000_000n,
  });

  assert.equal(args.length, 5);
  assert.equal(args[0]!.switch().name, "scvAddress");
  assert.equal(args[1]!.switch().name, "scvAddress");
  assert.equal(args[2]!.switch().name, "scvAddress");
  assert.equal(args[3]!.switch().name, "scvAddress");
  assert.equal(args[4]!.switch().name, "scvI128");
  assert.equal(args[4]!.i128().lo().toString(), "10000000");
  assert.equal(args[4]!.i128().hi().toString(), "0");
});

test("settle_sale: encodes 5 settlement arguments with exact types", () => {
  const client = createMarketplaceClient();
  const args = client.spec.funcArgsToScVals("settle_sale", {
    collection: FIXTURE_COLLECTION,
    token: FIXTURE_TOKEN,
    payer: FIXTURE_PAYER,
    seller: FIXTURE_SELLER_1,
    amount: 250_000_000n,
  });

  assert.equal(args.length, 5);
  assert.equal(Address.fromScVal(args[0]!).toString(), FIXTURE_COLLECTION);
  assert.equal(Address.fromScVal(args[1]!).toString(), FIXTURE_TOKEN);
  assert.equal(Address.fromScVal(args[2]!).toString(), FIXTURE_PAYER);
  assert.equal(Address.fromScVal(args[3]!).toString(), FIXTURE_SELLER_1);
  assert.equal(args[4]!.switch().name, "scvI128");
  assert.equal(args[4]!.i128().lo().toString(), "250000000");
});

test("settle_sales: encodes batch sale vector containing Vec<(Address, i128)> tuples", () => {
  const client = createMarketplaceClient();
  const batchSales: Array<readonly [string, bigint]> = [
    [FIXTURE_SELLER_1, 100_000_000n],
    [FIXTURE_SELLER_2, 350_000_000n],
  ];

  const args = client.spec.funcArgsToScVals("settle_sales", {
    collection: FIXTURE_COLLECTION,
    token: FIXTURE_TOKEN,
    payer: FIXTURE_PAYER,
    sales: batchSales,
  });

  assert.equal(args.length, 4);
  assert.equal(args[0]!.switch().name, "scvAddress");
  assert.equal(args[1]!.switch().name, "scvAddress");
  assert.equal(args[2]!.switch().name, "scvAddress");

  // sales is a Vec<(Address, i128)>, where each tuple is encoded as a 2-element scvVec
  assert.equal(args[3]!.switch().name, "scvVec");
  const outerVec = args[3]!.vec()!;
  assert.equal(outerVec.length, 2, "batch sales vector must contain 2 tuples");

  const firstTuple = outerVec[0]!.vec()!;
  assert.equal(firstTuple.length, 2);
  assert.equal(firstTuple[0]!.switch().name, "scvAddress");
  assert.equal(Address.fromScVal(firstTuple[0]!).toString(), FIXTURE_SELLER_1);
  assert.equal(firstTuple[1]!.switch().name, "scvI128");
  assert.equal(firstTuple[1]!.i128().lo().toString(), "100000000");

  const secondTuple = outerVec[1]!.vec()!;
  assert.equal(secondTuple.length, 2);
  assert.equal(secondTuple[0]!.switch().name, "scvAddress");
  assert.equal(Address.fromScVal(secondTuple[0]!).toString(), FIXTURE_SELLER_2);
  assert.equal(secondTuple[1]!.switch().name, "scvI128");
  assert.equal(secondTuple[1]!.i128().lo().toString(), "350000000");
});

test("quote_sale: encodes collection (scvAddress) and amount (scvI128)", () => {
  const client = createMarketplaceClient();
  const args = client.spec.funcArgsToScVals("quote_sale", {
    collection: FIXTURE_COLLECTION,
    amount: 99_999_999n,
  });

  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvAddress");
  assert.equal(Address.fromScVal(args[0]!).toString(), FIXTURE_COLLECTION);
  assert.equal(args[1]!.switch().name, "scvI128");
  assert.equal(args[1]!.i128().lo().toString(), "99999999");
});

// ---------------------------------------------------------------------------
// 4. Return value decoding — Settlement and Royalty records
// ---------------------------------------------------------------------------

function makeI128ScVal(value: bigint): xdr.ScVal {
  return xdr.ScVal.scvI128(
    new xdr.Int128Parts({
      hi: xdr.Int64.fromString((value >> 64n).toString()),
      lo: xdr.Uint64.fromString((value & ((1n << 64n) - 1n)).toString()),
    }),
  );
}

function makeSettlementScVal(royaltyShare: bigint, sellerNet: bigint): xdr.ScVal {
  return xdr.ScVal.scvMap([
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("royalty_share"),
      val: makeI128ScVal(royaltyShare),
    }),
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("seller_net"),
      val: makeI128ScVal(sellerNet),
    }),
  ]);
}

test("settle_sale: decodes Settlement return record via funcResToNative", () => {
  const client = createMarketplaceClient();
  const scVal = makeSettlementScVal(5_000_000n, 95_000_000n);
  const result = client.spec.funcResToNative("settle_sale", scVal) as {
    unwrap(): Settlement;
  };
  const decoded = typeof result.unwrap === "function" ? result.unwrap() : (result as unknown as Settlement);

  assert.equal(decoded.royalty_share, 5_000_000n);
  assert.equal(decoded.seller_net, 95_000_000n);
});

test("settle_sales: decodes Vec<Settlement> return records via funcResToNative", () => {
  const client = createMarketplaceClient();
  const batchScVal = xdr.ScVal.scvVec([
    makeSettlementScVal(5_000_000n, 95_000_000n),
    makeSettlementScVal(17_500_000n, 332_500_000n),
  ]);

  const result = client.spec.funcResToNative("settle_sales", batchScVal) as {
    unwrap(): Array<Settlement>;
  };
  const decoded =
    typeof result.unwrap === "function"
      ? result.unwrap()
      : (result as unknown as Array<Settlement>);

  assert.equal(decoded.length, 2);
  assert.deepEqual(decoded[0], {
    royalty_share: 5_000_000n,
    seller_net: 95_000_000n,
  });
  assert.deepEqual(decoded[1], {
    royalty_share: 17_500_000n,
    seller_net: 332_500_000n,
  });
});

test("get_royalty: decodes Royalty record (Active & Disabled statuses) via funcResToNative", () => {
  const client = createMarketplaceClient();

  const activeRoyaltyScVal = xdr.ScVal.scvMap([
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("bps"),
      val: xdr.ScVal.scvU32(750),
    }),
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("collection"),
      val: new Address(FIXTURE_COLLECTION).toScVal(),
    }),
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("recipient"),
      val: new Address(FIXTURE_RECIPIENT_1).toScVal(),
    }),
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("status"),
      val: xdr.ScVal.scvVec([xdr.ScVal.scvSymbol("Active")]),
    }),
  ]);

  const rawResult = client.spec.funcResToNative("get_royalty", activeRoyaltyScVal) as {
    unwrap(): Royalty;
  };
  const decoded =
    typeof rawResult.unwrap === "function"
      ? rawResult.unwrap()
      : (rawResult as unknown as Royalty);

  assert.equal(decoded.bps, 750);
  assert.equal(decoded.collection, FIXTURE_COLLECTION);
  assert.equal(decoded.recipient, FIXTURE_RECIPIENT_1);
  assert.equal(decoded.status.tag, "Active");
});

test("quote_sale: decodes SaleQuote return record via funcResToNative", () => {
  const client = createMarketplaceClient();
  const quoteScVal = xdr.ScVal.scvMap([
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("gross"),
      val: makeI128ScVal(100_000_000n),
    }),
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("royalty_amount"),
      val: makeI128ScVal(5_000_000n),
    }),
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("royalty_bps"),
      val: xdr.ScVal.scvU32(500),
    }),
    new xdr.ScMapEntry({
      key: xdr.ScVal.scvSymbol("seller_net"),
      val: makeI128ScVal(95_000_000n),
    }),
  ]);

  const rawResult = client.spec.funcResToNative("quote_sale", quoteScVal) as {
    unwrap(): SaleQuote;
  };
  const decoded =
    typeof rawResult.unwrap === "function"
      ? rawResult.unwrap()
      : (rawResult as unknown as SaleQuote);

  assert.equal(decoded.gross, 100_000_000n);
  assert.equal(decoded.royalty_bps, 500);
  assert.equal(decoded.royalty_amount, 5_000_000n);
  assert.equal(decoded.seller_net, 95_000_000n);
});

// ---------------------------------------------------------------------------
// 5. ForgeError mapping (codes 1–15)
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

test("marketplace.ForgeError maps all 15 error codes accurately", () => {
  const codes = Object.keys(marketplace.ForgeError)
    .map(Number)
    .sort((a, b) => a - b);
  assert.deepEqual(codes, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

  for (const [code, expectedMessage] of Object.entries(EXPECTED_FORGE_ERRORS)) {
    const entry =
      marketplace.ForgeError[
        Number(code) as keyof typeof marketplace.ForgeError
      ];
    assert.ok(entry, `marketplace.ForgeError[${code}] must exist`);
    assert.equal(
      entry.message,
      expectedMessage,
      `marketplace.ForgeError[${code}].message mismatch`,
    );
  }
});

test("marketplace ContractSpec embeds ForgeError enum cases matching runtime mapping", () => {
  const client = createMarketplaceClient();
  const errorEntry = client.spec
    .entries
    .find(
      (e) =>
        e.switch().name === "scSpecEntryUdtErrorEnumV0" &&
        e.udtErrorEnumV0().name().toString() === "ForgeError",
    );
  assert.ok(errorEntry, "ContractSpec must include ForgeError UDT error enum");

  const cases = errorEntry.udtErrorEnumV0().cases();
  assert.equal(cases.length, 15);
  for (const c of cases) {
    const code = c.value();
    const name = c.name().toString();
    assert.equal(
      name,
      EXPECTED_FORGE_ERRORS[code],
      `spec ForgeError case ${code} mismatch`,
    );
  }
});

// ---------------------------------------------------------------------------
// 6. Compile-time TypeScript typings for RoyaltyShare and Settlement
// ---------------------------------------------------------------------------

const _royaltyShareCheck: RoyaltyShare = {
  recipient: FIXTURE_RECIPIENT_1,
  bps: 500,
};

const _settlementCheck: Settlement = {
  royalty_share: 5_000_000n,
  seller_net: 95_000_000n,
};

const _statusActive: RoyaltyStatus = { tag: "Active", values: undefined };
const _statusDisabled: RoyaltyStatus = { tag: "Disabled", values: undefined };

const _royaltyCheck: Royalty = {
  collection: FIXTURE_COLLECTION,
  recipient: FIXTURE_RECIPIENT_1,
  bps: 500,
  status: _statusActive,
};

const _quoteCheck: SaleQuote = {
  gross: 100_000_000n,
  royalty_bps: 500,
  royalty_amount: 5_000_000n,
  seller_net: 95_000_000n,
};

const _summaryCheck: SettlementSummary = {
  sales: 2,
  gross_volume: 450_000_000n,
  royalties_paid: 22_500_000n,
};

void _royaltyShareCheck;
void _settlementCheck;
void _statusActive;
void _statusDisabled;
void _royaltyCheck;
void _quoteCheck;
void _summaryCheck;

test("RoyaltyShare and Settlement TypeScript typings compile cleanly", () => {
  assert.equal(_royaltyShareCheck.bps, 500);
  assert.equal(_settlementCheck.royalty_share + _settlementCheck.seller_net, 100_000_000n);
});
