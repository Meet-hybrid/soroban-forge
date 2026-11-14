/**
 * Offline client, argument-encoding, return-decoding, ForgeError, and
 * failed-simulation suite for @soroban-forge multisig client.
 */
import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";
import { xdr } from "@stellar/stellar-sdk";
import { multisig } from "../../dist/index.js";
import type {
  CheckResult,
  TxStatus,
  WalletTx,
  WindowState,
  WithdrawalLimit,
} from "../../dist/clients/multisig.js";
import {
  FIXTURE_ADDR_1,
  FIXTURE_ADDR_2,
  FIXTURE_CONTRACT_ID,
  FIXTURE_TOKEN,
  FixtureRpcServer,
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
  registerCommonClientSuite,
  successfulSimulation,
} from "./client-test-helpers.js";

registerCommonClientSuite({
  suiteName: "multisig",
  fixtureName: "multisig",
  ClientCtor: multisig.Client,
  clientForgeError: multisig.ForgeError,
});

const specClient = new multisig.Client({ ...TESTNET_CONFIG });

// ---------------------------------------------------------------------------
// Argument encoding for all 31 multisig methods (including edge values)
// ---------------------------------------------------------------------------

test("multisig zero-argument read methods encode empty ScVal lists", () => {
  for (const method of ["get_owners", "get_threshold", "get_tx_count"] as const) {
    const args = specClient.spec.funcArgsToScVals(method, {});
    assert.equal(args.length, 0);
  }
});

test("multisig tx_id methods encode zero and MAX_U64 as scvU64", () => {
  for (const method of [
    "execute",
    "get_confirmations",
    "get_rejections",
    "get_tx",
    "is_live",
    "is_tx_live",
    "touch_tx_ttl",
  ] as const) {
    const zeroArgs = specClient.spec.funcArgsToScVals(method, {
      tx_id: ZERO_U64,
    });
    assert.equal(zeroArgs.length, 1);
    assert.equal(zeroArgs[0]!.switch().name, "scvU64");
    assert.equal(zeroArgs[0]!.u64().toString(), "0");

    const maxArgs = specClient.spec.funcArgsToScVals(method, {
      tx_id: MAX_U64,
    });
    assert.equal(maxArgs[0]!.u64().toString(), MAX_U64.toString());
  }
});

test("multisig single-address token and owner methods encode scvAddress", () => {
  for (const method of [
    "balance",
    "get_window_usage",
    "get_withdrawal_limit",
    "get_withdrawal_window",
    "touch_ttl",
  ] as const) {
    const args = specClient.spec.funcArgsToScVals(method, {
      token: FIXTURE_TOKEN,
    });
    assert.equal(args.length, 1);
    assert.equal(args[0]!.switch().name, "scvAddress");
  }

  const ownerArgs = specClient.spec.funcArgsToScVals("is_owner", {
    address: FIXTURE_ADDR_1,
  });
  assert.equal(ownerArgs.length, 1);
  assert.equal(ownerArgs[0]!.switch().name, "scvAddress");
});

test("multisig owner/policy mutation and signer methods encode addresses and u64 ids", () => {
  const addArgs = specClient.spec.funcArgsToScVals("add_owner", {
    submitter: FIXTURE_ADDR_1,
    new_owner: FIXTURE_ADDR_2,
  });
  assert.equal(addArgs.length, 2);
  assert.equal(addArgs[0]!.switch().name, "scvAddress");
  assert.equal(addArgs[1]!.switch().name, "scvAddress");

  const removeArgs = specClient.spec.funcArgsToScVals("remove_owner", {
    submitter: FIXTURE_ADDR_1,
    existing_owner: FIXTURE_ADDR_2,
  });
  assert.equal(removeArgs.length, 2);
  assert.equal(removeArgs[0]!.switch().name, "scvAddress");
  assert.equal(removeArgs[1]!.switch().name, "scvAddress");

  const removeLimitArgs = specClient.spec.funcArgsToScVals(
    "remove_withdrawal_limit",
    {
      submitter: FIXTURE_ADDR_1,
      token: FIXTURE_TOKEN,
    },
  );
  assert.equal(removeLimitArgs.length, 2);
  assert.equal(removeLimitArgs[0]!.switch().name, "scvAddress");
  assert.equal(removeLimitArgs[1]!.switch().name, "scvAddress");

  for (const method of ["confirm", "reject"] as const) {
    const zeroArgs = specClient.spec.funcArgsToScVals(method, {
      tx_id: ZERO_U64,
      signer: FIXTURE_ADDR_1,
    });
    assert.equal(zeroArgs.length, 2);
    assert.equal(zeroArgs[0]!.u64().toString(), "0");
    assert.equal(zeroArgs[1]!.switch().name, "scvAddress");

    const maxArgs = specClient.spec.funcArgsToScVals(method, {
      tx_id: MAX_U64,
      signer: FIXTURE_ADDR_2,
    });
    assert.equal(maxArgs[0]!.u64().toString(), MAX_U64.toString());
  }
});

test("multisig amount, threshold, and withdrawal-limit methods encode zero and max boundaries", () => {
  const checkZero = specClient.spec.funcArgsToScVals("check_withdrawal", {
    token: FIXTURE_TOKEN,
    amount: ZERO_I128,
  });
  assert.equal(checkZero.length, 2);
  assert.equal(decodeScValI128(specClient.spec, checkZero[1]!), 0n);

  const checkMax = specClient.spec.funcArgsToScVals("check_withdrawal", {
    token: FIXTURE_TOKEN,
    amount: MAX_I128,
  });
  assert.equal(decodeScValI128(specClient.spec, checkMax[1]!), MAX_I128);

  const depositZero = specClient.spec.funcArgsToScVals("deposit", {
    token: FIXTURE_TOKEN,
    from: FIXTURE_ADDR_1,
    amount: ZERO_I128,
  });
  assert.equal(depositZero.length, 3);
  assert.equal(decodeScValI128(specClient.spec, depositZero[2]!), 0n);

  const depositMax = specClient.spec.funcArgsToScVals("deposit", {
    token: FIXTURE_TOKEN,
    from: FIXTURE_ADDR_1,
    amount: MAX_I128,
  });
  assert.equal(decodeScValI128(specClient.spec, depositMax[2]!), MAX_I128);

  const thresholdZero = specClient.spec.funcArgsToScVals("set_threshold", {
    submitter: FIXTURE_ADDR_1,
    new_threshold: ZERO_U32,
  });
  assert.equal(thresholdZero.length, 2);
  assert.equal(thresholdZero[1]!.u32(), 0);

  const thresholdMax = specClient.spec.funcArgsToScVals("set_threshold", {
    submitter: FIXTURE_ADDR_1,
    new_threshold: MAX_U32,
  });
  assert.equal(thresholdMax[1]!.u32(), MAX_U32);

  const limitZero = specClient.spec.funcArgsToScVals("set_withdrawal_limit", {
    submitter: FIXTURE_ADDR_1,
    token: FIXTURE_TOKEN,
    amount: ZERO_I128,
    window_seconds: ZERO_U64,
  });
  assert.equal(limitZero.length, 4);
  assert.equal(decodeScValI128(specClient.spec, limitZero[2]!), 0n);
  assert.equal(limitZero[3]!.u64().toString(), "0");

  const limitMax = specClient.spec.funcArgsToScVals("set_withdrawal_limit", {
    submitter: FIXTURE_ADDR_1,
    token: FIXTURE_TOKEN,
    amount: MAX_I128,
    window_seconds: MAX_U64,
  });
  assert.equal(decodeScValI128(specClient.spec, limitMax[2]!), MAX_I128);
  assert.equal(limitMax[3]!.u64().toString(), MAX_U64.toString());

  const withdrawZero = specClient.spec.funcArgsToScVals("submit_withdrawal", {
    submitter: FIXTURE_ADDR_1,
    token: FIXTURE_TOKEN,
    destination: FIXTURE_ADDR_2,
    amount: ZERO_I128,
  });
  assert.equal(withdrawZero.length, 4);
  assert.equal(decodeScValI128(specClient.spec, withdrawZero[3]!), 0n);

  const withdrawMax = specClient.spec.funcArgsToScVals("submit_withdrawal", {
    submitter: FIXTURE_ADDR_1,
    token: FIXTURE_TOKEN,
    destination: FIXTURE_ADDR_2,
    amount: MAX_I128,
  });
  assert.equal(decodeScValI128(specClient.spec, withdrawMax[3]!), MAX_I128);
});

test("multisig.initialize: encodes empty and max-owner vectors with zero and max u32 threshold", () => {
  const emptyArgs = specClient.spec.funcArgsToScVals("initialize", {
    owners: [],
    threshold: ZERO_U32,
  });
  assert.equal(emptyArgs.length, 2);
  assert.equal(emptyArgs[0]!.switch().name, "scvVec");
  assert.equal(emptyArgs[0]!.vec()!.length, 0);
  assert.equal(emptyArgs[1]!.u32(), 0);

  const maxOwners = Array.from({ length: 20 }, (_, idx) =>
    idx % 2 === 0 ? FIXTURE_ADDR_1 : FIXTURE_ADDR_2,
  );
  const fullArgs = specClient.spec.funcArgsToScVals("initialize", {
    owners: maxOwners,
    threshold: MAX_U32,
  });
  assert.equal(fullArgs[0]!.vec()!.length, 20);
  assert.equal(fullArgs[0]!.vec()![0]!.switch().name, "scvAddress");
  assert.equal(fullArgs[1]!.u32(), MAX_U32);
});

test("multisig.submit and submit_call: encode empty/max payloads, argument vectors, and Option<u64> None/Some", () => {
  const submitNone = specClient.spec.funcArgsToScVals("submit", {
    submitter: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    tx: Buffer.alloc(0),
    expiry: undefined,
  });
  assert.equal(submitNone.length, 4);
  assert.equal(submitNone[2]!.switch().name, "scvBytes");
  assert.equal(submitNone[2]!.bytes().length, 0);
  assert.equal(submitNone[3]!.switch().name, "scvVoid");

  const payloadBytes = Buffer.alloc(256, 0x5a);
  const submitSome = specClient.spec.funcArgsToScVals("submit", {
    submitter: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    tx: payloadBytes,
    expiry: MAX_U64,
  });
  assert.equal(submitSome[2]!.bytes().length, 256);
  assert.equal(submitSome[3]!.switch().name, "scvU64");
  assert.equal(submitSome[3]!.u64().toString(), MAX_U64.toString());

  const callEmpty = specClient.spec.funcArgsToScVals("submit_call", {
    submitter: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    fn_name: "ping",
    args: [],
    expiry: undefined,
  });
  assert.equal(callEmpty.length, 5);
  assert.equal(callEmpty[2]!.switch().name, "scvSymbol");
  assert.equal(callEmpty[2]!.sym().toString(), "ping");
  assert.equal(callEmpty[3]!.switch().name, "scvVec");
  assert.equal(callEmpty[3]!.vec()!.length, 0);
  assert.equal(callEmpty[4]!.switch().name, "scvVoid");

  const maxCallArgs = Array.from({ length: 16 }, (_, idx) =>
    xdr.ScVal.scvU32(idx),
  );
  const callFull = specClient.spec.funcArgsToScVals("submit_call", {
    submitter: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    fn_name: "batch_exec",
    args: maxCallArgs,
    expiry: MAX_U64,
  });
  assert.equal(callFull[3]!.vec()!.length, 16);
  assert.equal(callFull[3]!.vec()![15]!.u32(), 15);
  assert.equal(callFull[4]!.switch().name, "scvU64");
  assert.equal(callFull[4]!.u64().toString(), MAX_U64.toString());
});

test("multisig.get_transactions and get_transactions_by_status: encode status union and pagination bounds", () => {
  const zeroPage = specClient.spec.funcArgsToScVals("get_transactions", {
    offset: ZERO_U32,
    limit: ZERO_U32,
  });
  assert.equal(zeroPage.length, 2);
  assert.equal(zeroPage[0]!.u32(), 0);
  assert.equal(zeroPage[1]!.u32(), 0);

  const maxPage = specClient.spec.funcArgsToScVals("get_transactions", {
    offset: MAX_U32,
    limit: MAX_U32,
  });
  assert.equal(maxPage[0]!.u32(), MAX_U32);
  assert.equal(maxPage[1]!.u32(), MAX_U32);

  for (const tag of ["Pending", "Executed", "Rejected", "Expired"] as const) {
    const status: TxStatus = { tag, values: undefined };
    const statusArgs = specClient.spec.funcArgsToScVals(
      "get_transactions_by_status",
      {
        status,
        offset: ZERO_U32,
        limit: MAX_U32,
      },
    );
    assert.equal(statusArgs.length, 3);
    assert.equal(statusArgs[0]!.switch().name, "scvVec");
    assert.equal(statusArgs[0]!.vec()![0]!.sym().toString(), tag);
    assert.equal(statusArgs[1]!.u32(), 0);
    assert.equal(statusArgs[2]!.u32(), MAX_U32);
  }
});

// ---------------------------------------------------------------------------
// Return value decoding from fixture ScVals (including Option None and Some)
// ---------------------------------------------------------------------------

test("multisig client decodes direct scalar and union read views", async () => {
  const balCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "balance",
    MAX_I128,
  );
  assert.equal((await balCtx.client.balance({ token: FIXTURE_TOKEN })).result, MAX_I128);

  const liveCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "is_live",
    true,
  );
  assert.equal((await liveCtx.client.is_live({ tx_id: 1n })).result, true);

  const txLiveCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "is_tx_live",
    false,
  );
  assert.equal((await txLiveCtx.client.is_tx_live({ tx_id: 1n })).result, false);

  const ownerCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "is_owner",
    true,
  );
  assert.equal(
    (await ownerCtx.client.is_owner({ address: FIXTURE_ADDR_1 })).result,
    true,
  );

  const countCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_tx_count",
    19n,
  );
  assert.equal((await countCtx.client.get_tx_count()).result, 19n);

  const usageCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_window_usage",
    750n,
  );
  assert.equal(
    (await usageCtx.client.get_window_usage({ token: FIXTURE_TOKEN })).result,
    750n,
  );

  for (const tag of ["Allowed", "LimitExceeded", "InsufficientFunds"] as const) {
    const checkFixture: CheckResult = { tag, values: undefined };
    const checkCtx = createClientWithReturn(
      multisig.Client,
      specClient.spec,
      "check_withdrawal",
      checkFixture,
    );
    const res = (
      await checkCtx.client.check_withdrawal({
        token: FIXTURE_TOKEN,
        amount: 500n,
      })
    ).result;
    assert.equal(res.tag, tag);
  }
});

test("multisig client decodes Option None and Some for get_withdrawal_limit and get_withdrawal_window", async () => {
  const noneLimitCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_withdrawal_limit",
    undefined,
  );
  assert.equal(
    (await noneLimitCtx.client.get_withdrawal_limit({ token: FIXTURE_TOKEN }))
      .result,
    null,
  );

  const someLimitFixture: WithdrawalLimit = {
    amount: 5_000n,
    token: FIXTURE_TOKEN,
    window_seconds: 86_400n,
  };
  const someLimitCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_withdrawal_limit",
    someLimitFixture,
  );
  const decodedLimit = (
    await someLimitCtx.client.get_withdrawal_limit({ token: FIXTURE_TOKEN })
  ).result;
  assert.ok(decodedLimit);
  assert.equal(decodedLimit.amount, 5_000n);
  assert.equal(decodedLimit.token, FIXTURE_TOKEN);
  assert.equal(decodedLimit.window_seconds, 86_400n);

  const emptyWindow: WindowState = {
    reset_at: undefined,
    total: 0n,
    window_start: undefined,
  };
  const emptyWinCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_withdrawal_window",
    emptyWindow,
  );
  const decodedEmptyWin = (
    await emptyWinCtx.client.get_withdrawal_window({ token: FIXTURE_TOKEN })
  ).result;
  assert.equal(decodedEmptyWin.total, 0n);
  assert.equal(decodedEmptyWin.reset_at, null);
  assert.equal(decodedEmptyWin.window_start, null);

  const activeWindow: WindowState = {
    reset_at: 2_600n,
    total: 1_250n,
    window_start: 1_000n,
  };
  const activeWinCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_withdrawal_window",
    activeWindow,
  );
  const decodedActiveWin = (
    await activeWinCtx.client.get_withdrawal_window({ token: FIXTURE_TOKEN })
  ).result;
  assert.equal(decodedActiveWin.total, 1_250n);
  assert.equal(decodedActiveWin.reset_at, 2_600n);
  assert.equal(decodedActiveWin.window_start, 1_000n);
});

test("multisig client decodes Result-wrapped owner, threshold, and WalletTx views", async () => {
  const threshCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_threshold",
    2,
  );
  assert.equal((await threshCtx.client.get_threshold()).result.unwrap(), 2);

  const ownersCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_owners",
    [FIXTURE_ADDR_1, FIXTURE_ADDR_2],
  );
  assert.deepEqual((await ownersCtx.client.get_owners()).result.unwrap(), [
    FIXTURE_ADDR_1,
    FIXTURE_ADDR_2,
  ]);

  const confCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_confirmations",
    [FIXTURE_ADDR_1],
  );
  assert.deepEqual(
    (await confCtx.client.get_confirmations({ tx_id: 0n })).result.unwrap(),
    [FIXTURE_ADDR_1],
  );

  const rejCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_rejections",
    [FIXTURE_ADDR_2],
  );
  assert.deepEqual(
    (await rejCtx.client.get_rejections({ tx_id: 0n })).result.unwrap(),
    [FIXTURE_ADDR_2],
  );

  const opaqueTx: WalletTx = {
    confirmations: [FIXTURE_ADDR_1],
    expiry: undefined,
    kind: { tag: "Opaque", values: undefined },
    payload: Buffer.from([1, 2, 3, 4]),
    rejections: [],
    status: { tag: "Pending", values: undefined },
    submitter: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    tx_id: 0n,
  };
  const txNoneCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_tx",
    opaqueTx,
  );
  const decodedOpaque = (
    await txNoneCtx.client.get_tx({ tx_id: 0n })
  ).result.unwrap();
  assert.equal(decodedOpaque.expiry, null);
  assert.equal(decodedOpaque.kind.tag, "Opaque");
  assert.equal(decodedOpaque.status.tag, "Pending");
  assert.deepEqual(Buffer.from(decodedOpaque.payload), Buffer.from([1, 2, 3, 4]));

  const withdrawalTx: WalletTx = {
    ...opaqueTx,
    expiry: 9_999n,
    kind: {
      tag: "Withdrawal",
      values: [
        {
          amount: 350n,
          destination: FIXTURE_ADDR_2,
          token: FIXTURE_TOKEN,
        },
      ],
    },
    status: { tag: "Executed", values: undefined },
    tx_id: 1n,
  };
  const listCtx = createClientWithReturn(
    multisig.Client,
    specClient.spec,
    "get_transactions",
    [opaqueTx, withdrawalTx],
  );
  const decodedList = (
    await listCtx.client.get_transactions({ offset: 0, limit: 10 })
  ).result.unwrap();
  assert.equal(decodedList.length, 2);
  assert.equal(decodedList[1]!.expiry, 9_999n);
  assert.equal(decodedList[1]!.kind.tag, "Withdrawal");
  assert.equal(decodedList[1]!.status.tag, "Executed");
});

// ---------------------------------------------------------------------------
// Failed simulations and contract-level errors
// ---------------------------------------------------------------------------

test("multisig client reports a failed simulation as an error rather than empty success", async () => {
  const { client, server } = createClientWithFailure(
    multisig.Client,
    "multisig simulation reverted",
  );
  const tx = await client.get_tx({ tx_id: 42n });
  assert.equal(server.simulationCount, 1);
  assert.throws(() => tx.result, /multisig simulation reverted/);
});

test("multisig client decodes on-chain ForgeError Result::Err without silent success", async () => {
  const server = new FixtureRpcServer(
    successfulSimulation(contractErrorScVal(13)),
  );
  const client = new multisig.Client({ ...TESTNET_CONFIG, server });
  const tx = await client.submit_withdrawal({
    submitter: FIXTURE_ADDR_1,
    token: FIXTURE_TOKEN,
    destination: FIXTURE_ADDR_2,
    amount: 10_000n,
  });
  assert.equal(tx.result.isErr(), true);
  assert.throws(() => tx.result.unwrap());
});
