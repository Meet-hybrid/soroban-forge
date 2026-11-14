/**
 * Offline client, argument-encoding, return-decoding, ForgeError, and
 * failed-simulation suite for @soroban-forge vesting client.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { vesting } from "../../dist/index.js";
import type {
  Tranche,
  TrancheSchedule,
  VestingSchedule,
  VestingStatus,
} from "../../dist/clients/vesting.js";
import {
  FIXTURE_ADDR_1,
  FIXTURE_ADDR_2,
  FIXTURE_CONTRACT_ID,
  FIXTURE_TOKEN,
  MAX_I128,
  MAX_U64,
  TESTNET_CONFIG,
  ZERO_I128,
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
  suiteName: "vesting",
  fixtureName: "vesting",
  ClientCtor: vesting.Client,
  clientForgeError: vesting.ForgeError,
});

const specClient = new vesting.Client({ ...TESTNET_CONFIG });

// ---------------------------------------------------------------------------
// Argument encoding for all 11 vesting methods (including edge values)
// ---------------------------------------------------------------------------

test("vesting.claim: encodes schedule_id (0 and MAX_U64) as scvU64", () => {
  const zeroArgs = specClient.spec.funcArgsToScVals("claim", {
    schedule_id: ZERO_U64,
  });
  assert.equal(zeroArgs.length, 1);
  assert.equal(zeroArgs[0]!.switch().name, "scvU64");
  assert.equal(zeroArgs[0]!.u64().toString(), "0");

  const maxArgs = specClient.spec.funcArgsToScVals("claim", {
    schedule_id: MAX_U64,
  });
  assert.equal(maxArgs[0]!.u64().toString(), MAX_U64.toString());
});

test("vesting.claim_for: encodes schedule_id (scvU64) and beneficiary (scvAddress)", () => {
  const args = specClient.spec.funcArgsToScVals("claim_for", {
    schedule_id: 9n,
    beneficiary: FIXTURE_ADDR_1,
  });
  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "9");
  assert.equal(args[1]!.switch().name, "scvAddress");
});

test("vesting.claimable: encodes schedule_id as scvU64", () => {
  const args = specClient.spec.funcArgsToScVals("claimable", {
    schedule_id: 7n,
  });
  assert.equal(args.length, 1);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "7");
});

test("vesting.claimable_for: encodes schedule_id and beneficiary", () => {
  const args = specClient.spec.funcArgsToScVals("claimable_for", {
    schedule_id: MAX_U64,
    beneficiary: FIXTURE_ADDR_2,
  });
  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), MAX_U64.toString());
  assert.equal(args[1]!.switch().name, "scvAddress");
});

test("vesting.create_schedule: encodes addresses, zero/max i128 amounts, and zero/max u64 durations", () => {
  const minArgs = specClient.spec.funcArgsToScVals("create_schedule", {
    funder: FIXTURE_ADDR_1,
    beneficiary: FIXTURE_ADDR_2,
    token: FIXTURE_TOKEN,
    total_amount: ZERO_I128,
    cliff: ZERO_U64,
    duration: ZERO_U64,
  });
  assert.equal(minArgs.length, 6);
  assert.equal(minArgs[0]!.switch().name, "scvAddress");
  assert.equal(minArgs[1]!.switch().name, "scvAddress");
  assert.equal(minArgs[2]!.switch().name, "scvAddress");
  assert.equal(decodeScValI128(specClient.spec, minArgs[3]!), 0n);
  assert.equal(minArgs[4]!.u64().toString(), "0");
  assert.equal(minArgs[5]!.u64().toString(), "0");

  const maxArgs = specClient.spec.funcArgsToScVals("create_schedule", {
    funder: FIXTURE_ADDR_1,
    beneficiary: FIXTURE_ADDR_2,
    token: FIXTURE_TOKEN,
    total_amount: MAX_I128,
    cliff: MAX_U64,
    duration: MAX_U64,
  });
  assert.equal(decodeScValI128(specClient.spec, maxArgs[3]!), MAX_I128);
  assert.equal(maxArgs[4]!.u64().toString(), MAX_U64.toString());
  assert.equal(maxArgs[5]!.u64().toString(), MAX_U64.toString());
});

test("vesting.create_tranche_schedule: encodes empty and maximum-length tranche vectors", () => {
  const emptyArgs = specClient.spec.funcArgsToScVals(
    "create_tranche_schedule",
    {
      beneficiary: FIXTURE_ADDR_1,
      token: FIXTURE_TOKEN,
      tranches: [],
    },
  );
  assert.equal(emptyArgs.length, 3);
  assert.equal(emptyArgs[0]!.switch().name, "scvAddress");
  assert.equal(emptyArgs[1]!.switch().name, "scvAddress");
  assert.equal(emptyArgs[2]!.switch().name, "scvVec");
  assert.equal(emptyArgs[2]!.vec()!.length, 0);

  const maxTranches: Tranche[] = Array.from({ length: 32 }, (_, idx) => ({
    amount: idx === 31 ? MAX_I128 : 1_000n,
    unlock_at: idx === 0 ? ZERO_U64 : idx === 31 ? MAX_U64 : BigInt(idx * 60),
  }));
  const fullArgs = specClient.spec.funcArgsToScVals("create_tranche_schedule", {
    beneficiary: FIXTURE_ADDR_1,
    token: FIXTURE_TOKEN,
    tranches: maxTranches,
  });
  assert.equal(fullArgs[2]!.switch().name, "scvVec");
  assert.equal(fullArgs[2]!.vec()!.length, 32);
  assert.equal(fullArgs[2]!.vec()![0]!.switch().name, "scvMap");
});

test("vesting.get_schedule, get_status, get_tranche_schedule, revoke: encode schedule_id as scvU64", () => {
  for (const method of [
    "get_schedule",
    "get_status",
    "get_tranche_schedule",
    "revoke",
  ] as const) {
    const args = specClient.spec.funcArgsToScVals(method, {
      schedule_id: 42n,
    });
    assert.equal(args.length, 1);
    assert.equal(args[0]!.switch().name, "scvU64");
    assert.equal(args[0]!.u64().toString(), "42");
  }
});

test("vesting.reassign_beneficiary: encodes schedule_id (scvU64) and new_beneficiary (scvAddress)", () => {
  const args = specClient.spec.funcArgsToScVals("reassign_beneficiary", {
    schedule_id: 15n,
    new_beneficiary: FIXTURE_ADDR_2,
  });
  assert.equal(args.length, 2);
  assert.equal(args[0]!.switch().name, "scvU64");
  assert.equal(args[0]!.u64().toString(), "15");
  assert.equal(args[1]!.switch().name, "scvAddress");
});

// ---------------------------------------------------------------------------
// Return value decoding from fixture ScVals (including Option None and Some)
// ---------------------------------------------------------------------------

test("vesting client decodes claimable and claimable_for i128 results", async () => {
  const { client, server } = createClientWithReturn(
    vesting.Client,
    specClient.spec,
    "claimable",
    125n,
  );
  const tx = await client.claimable({ schedule_id: 7n });
  assert.equal(server.simulationCount, 1);
  assert.equal(tx.result.unwrap(), 125n);

  const forCtx = createClientWithReturn(
    vesting.Client,
    specClient.spec,
    "claimable_for",
    MAX_I128,
  );
  const forTx = await forCtx.client.claimable_for({
    schedule_id: 7n,
    beneficiary: FIXTURE_ADDR_1,
  });
  assert.equal(forTx.result.unwrap(), MAX_I128);
});

test("vesting client decodes get_status union variants", async () => {
  for (const tag of ["Locked", "Vesting", "Completed", "Revoked"] as const) {
    const statusFixture: VestingStatus = { tag, values: undefined };
    const { client } = createClientWithReturn(
      vesting.Client,
      specClient.spec,
      "get_status",
      statusFixture,
    );
    const tx = await client.get_status({ schedule_id: 1n });
    assert.equal(tx.result.unwrap().tag, tag);
  }
});

test("vesting client decodes get_schedule with revoked_vested as None and Some", async () => {
  const activeFixture: VestingSchedule = {
    beneficiary: FIXTURE_ADDR_1,
    beneficiary_claimed: 0n,
    claimed: 0n,
    cliff: 10n,
    duration: 100n,
    funder: FIXTURE_CONTRACT_ID,
    reassignment_count: 0,
    reassignment_vested: 0n,
    revoked_vested: undefined,
    start: 1_000n,
    status: { tag: "Locked", values: undefined },
    token: FIXTURE_TOKEN,
    total_amount: 500n,
  };
  const noneCtx = createClientWithReturn(
    vesting.Client,
    specClient.spec,
    "get_schedule",
    activeFixture,
  );
  const noneDecoded = (
    await noneCtx.client.get_schedule({ schedule_id: 1n })
  ).result.unwrap();
  assert.equal(noneDecoded.beneficiary, FIXTURE_ADDR_1);
  assert.equal(noneDecoded.revoked_vested, null);
  assert.equal(noneDecoded.status.tag, "Locked");
  assert.equal(noneDecoded.total_amount, 500n);

  const revokedFixture: VestingSchedule = {
    ...activeFixture,
    revoked_vested: 250n,
    status: { tag: "Revoked", values: undefined },
  };
  const someCtx = createClientWithReturn(
    vesting.Client,
    specClient.spec,
    "get_schedule",
    revokedFixture,
  );
  const someDecoded = (
    await someCtx.client.get_schedule({ schedule_id: 2n })
  ).result.unwrap();
  assert.equal(someDecoded.revoked_vested, 250n);
  assert.equal(someDecoded.status.tag, "Revoked");
});

test("vesting client decodes get_tranche_schedule struct and tranche vector", async () => {
  const trancheFixture: TrancheSchedule = {
    beneficiary: FIXTURE_ADDR_2,
    claimed: 100n,
    start: 5_000n,
    status: { tag: "Vesting", values: undefined },
    token: FIXTURE_TOKEN,
    total_amount: 300n,
    tranches: [
      { amount: 100n, unlock_at: 0n },
      { amount: 200n, unlock_at: 3_600n },
    ],
  };
  const { client } = createClientWithReturn(
    vesting.Client,
    specClient.spec,
    "get_tranche_schedule",
    trancheFixture,
  );
  const decoded = (
    await client.get_tranche_schedule({ schedule_id: 3n })
  ).result.unwrap();
  assert.equal(decoded.beneficiary, FIXTURE_ADDR_2);
  assert.equal(decoded.total_amount, 300n);
  assert.equal(decoded.tranches.length, 2);
  assert.equal(decoded.tranches[0]!.amount, 100n);
  assert.equal(decoded.tranches[1]!.unlock_at, 3_600n);
});

// ---------------------------------------------------------------------------
// Failed simulations and contract-level errors
// ---------------------------------------------------------------------------

test("vesting client reports a failed simulation as an error rather than empty success", async () => {
  const { client, server } = createClientWithFailure(
    vesting.Client,
    "fixture contract simulation failure",
  );
  const transaction = await client.create_schedule({
    funder: FIXTURE_CONTRACT_ID,
    beneficiary: FIXTURE_ADDR_1,
    token: FIXTURE_TOKEN,
    total_amount: 500n,
    cliff: 10n,
    duration: 100n,
  });

  assert.equal(server.simulationCount, 1);
  assert.throws(
    () => transaction.result,
    /fixture contract simulation failure/,
  );
});

test("vesting client decodes on-chain ForgeError Result::Err without silent success", async () => {
  const server = new FixtureRpcServer(
    successfulSimulation(contractErrorScVal(2)),
  );
  const client = new vesting.Client({ ...TESTNET_CONFIG, server });
  const tx = await client.get_schedule({ schedule_id: 999n });
  assert.equal(tx.result.isErr(), true);
  assert.throws(() => tx.result.unwrap());
});
