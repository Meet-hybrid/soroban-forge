/**
 * Offline client, argument-encoding, return-decoding, ForgeError, and
 * failed-simulation suite for @soroban-forge dao client.
 */
import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";
import { dao } from "../../dist/index.js";
import type {
  BondConfig,
  DependencyView,
  Proposal,
} from "../../dist/clients/dao.js";
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
  suiteName: "dao",
  fixtureName: "dao",
  ClientCtor: dao.Client,
  clientForgeError: dao.ForgeError,
});

const specClient = new dao.Client({ ...TESTNET_CONFIG });

// ---------------------------------------------------------------------------
// Argument encoding for all 14 dao methods (including edge values)
// ---------------------------------------------------------------------------

test("dao zero-argument read methods encode empty ScVal lists", () => {
  for (const method of ["get_bond_config", "get_proposal_count"] as const) {
    const args = specClient.spec.funcArgsToScVals(method, {});
    assert.equal(args.length, 0);
  }
});

test("dao single-proposal_id methods encode zero and MAX_U64 as scvU64", () => {
  for (const method of [
    "execute",
    "get_dependencies",
    "get_proposal",
    "touch_ttl",
  ] as const) {
    const zeroArgs = specClient.spec.funcArgsToScVals(method, {
      proposal_id: ZERO_U64,
    });
    assert.equal(zeroArgs.length, 1);
    assert.equal(zeroArgs[0]!.switch().name, "scvU64");
    assert.equal(zeroArgs[0]!.u64().toString(), "0");

    const maxArgs = specClient.spec.funcArgsToScVals(method, {
      proposal_id: MAX_U64,
    });
    assert.equal(maxArgs[0]!.u64().toString(), MAX_U64.toString());
  }
});

test("dao.initialize and get_active_proposal_count: encode address arguments", () => {
  const initArgs = specClient.spec.funcArgsToScVals("initialize", {
    governance_token: FIXTURE_TOKEN,
  });
  assert.equal(initArgs.length, 1);
  assert.equal(initArgs[0]!.switch().name, "scvAddress");

  const activeArgs = specClient.spec.funcArgsToScVals(
    "get_active_proposal_count",
    {
      proposer: FIXTURE_ADDR_1,
    },
  );
  assert.equal(activeArgs.length, 1);
  assert.equal(activeArgs[0]!.switch().name, "scvAddress");
});

test("dao.configure_bond: encodes token, zero/max i128 amount, and treasury", () => {
  const zeroArgs = specClient.spec.funcArgsToScVals("configure_bond", {
    token: FIXTURE_TOKEN,
    amount: ZERO_I128,
    treasury: FIXTURE_ADDR_2,
  });
  assert.equal(zeroArgs.length, 3);
  assert.equal(zeroArgs[0]!.switch().name, "scvAddress");
  assert.equal(decodeScValI128(specClient.spec, zeroArgs[1]!), 0n);
  assert.equal(zeroArgs[2]!.switch().name, "scvAddress");

  const maxArgs = specClient.spec.funcArgsToScVals("configure_bond", {
    token: FIXTURE_TOKEN,
    amount: MAX_I128,
    treasury: FIXTURE_ADDR_2,
  });
  assert.equal(decodeScValI128(specClient.spec, maxArgs[1]!), MAX_I128);
});

test("dao.cancel_proposal, has_voted, vote, and get_proposals: encode edge ids, booleans, and pagination", () => {
  const cancelArgs = specClient.spec.funcArgsToScVals("cancel_proposal", {
    proposal_id: MAX_U64,
    proposer: FIXTURE_ADDR_1,
  });
  assert.equal(cancelArgs.length, 2);
  assert.equal(cancelArgs[0]!.u64().toString(), MAX_U64.toString());
  assert.equal(cancelArgs[1]!.switch().name, "scvAddress");

  const hasVotedArgs = specClient.spec.funcArgsToScVals("has_voted", {
    proposal_id: ZERO_U64,
    voter: FIXTURE_ADDR_2,
  });
  assert.equal(hasVotedArgs.length, 2);
  assert.equal(hasVotedArgs[0]!.u64().toString(), "0");
  assert.equal(hasVotedArgs[1]!.switch().name, "scvAddress");

  for (const support of [false, true] as const) {
    const voteArgs = specClient.spec.funcArgsToScVals("vote", {
      proposal_id: support ? MAX_U64 : ZERO_U64,
      voter: FIXTURE_ADDR_1,
      support,
    });
    assert.equal(voteArgs.length, 3);
    assert.equal(voteArgs[0]!.switch().name, "scvU64");
    assert.equal(voteArgs[1]!.switch().name, "scvAddress");
    assert.equal(voteArgs[2]!.switch().name, "scvBool");
    assert.equal(voteArgs[2]!.b(), support);
  }

  const zeroPage = specClient.spec.funcArgsToScVals("get_proposals", {
    offset: ZERO_U32,
    limit: ZERO_U32,
  });
  assert.equal(zeroPage.length, 2);
  assert.equal(zeroPage[0]!.u32(), 0);
  assert.equal(zeroPage[1]!.u32(), 0);

  const maxPage = specClient.spec.funcArgsToScVals("get_proposals", {
    offset: MAX_U32,
    limit: MAX_U32,
  });
  assert.equal(maxPage[0]!.u32(), MAX_U32);
  assert.equal(maxPage[1]!.u32(), MAX_U32);
});

test("dao.propose: encodes empty/non-empty action bytes, empty/max dependency vectors, and Option<u64> None/Some", () => {
  const minArgs = specClient.spec.funcArgsToScVals("propose", {
    proposer: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    action: Buffer.alloc(0),
    duration: ZERO_U64,
    requires: [],
    conflicts_with: undefined,
  });
  assert.equal(minArgs.length, 6);
  assert.equal(minArgs[0]!.switch().name, "scvAddress");
  assert.equal(minArgs[1]!.switch().name, "scvAddress");
  assert.equal(minArgs[2]!.switch().name, "scvBytes");
  assert.equal(minArgs[2]!.bytes().length, 0);
  assert.equal(minArgs[3]!.u64().toString(), "0");
  assert.equal(minArgs[4]!.switch().name, "scvVec");
  assert.equal(minArgs[4]!.vec()!.length, 0);
  assert.equal(minArgs[5]!.switch().name, "scvVoid");

  const maxRequires = Array.from({ length: 16 }, (_, idx) =>
    idx === 15 ? MAX_U64 : BigInt(idx),
  );
  const actionPayload = Buffer.from("upgrade-protocol-v2", "utf8");
  const maxArgs = specClient.spec.funcArgsToScVals("propose", {
    proposer: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    action: actionPayload,
    duration: MAX_U64,
    requires: maxRequires,
    conflicts_with: MAX_U64,
  });
  assert.equal(maxArgs[2]!.bytes().length, actionPayload.length);
  assert.equal(maxArgs[3]!.u64().toString(), MAX_U64.toString());
  assert.equal(maxArgs[4]!.vec()!.length, 16);
  assert.equal(maxArgs[4]!.vec()![0]!.u64().toString(), "0");
  assert.equal(maxArgs[4]!.vec()![15]!.u64().toString(), MAX_U64.toString());
  assert.equal(maxArgs[5]!.switch().name, "scvU64");
  assert.equal(maxArgs[5]!.u64().toString(), MAX_U64.toString());
});

// ---------------------------------------------------------------------------
// Return value decoding from fixture ScVals (including Option None and Some)
// ---------------------------------------------------------------------------

test("dao client decodes direct get_proposal_count and get_active_proposal_count views", async () => {
  const countCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "get_proposal_count",
    42n,
  );
  assert.equal((await countCtx.client.get_proposal_count()).result, 42n);

  const activeCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "get_active_proposal_count",
    3,
  );
  assert.equal(
    (
      await activeCtx.client.get_active_proposal_count({
        proposer: FIXTURE_ADDR_1,
      })
    ).result,
    3,
  );
});

test("dao client decodes get_bond_config, has_voted, and get_dependencies (None and Some)", async () => {
  const bondFixture: BondConfig = {
    amount: 10_000n,
    token: FIXTURE_TOKEN,
    treasury: FIXTURE_ADDR_2,
  };
  const bondCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "get_bond_config",
    bondFixture,
  );
  const decodedBond = (await bondCtx.client.get_bond_config()).result.unwrap();
  assert.equal(decodedBond.amount, 10_000n);
  assert.equal(decodedBond.token, FIXTURE_TOKEN);
  assert.equal(decodedBond.treasury, FIXTURE_ADDR_2);

  const votedCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "has_voted",
    true,
  );
  assert.equal(
    (
      await votedCtx.client.has_voted({
        proposal_id: 1n,
        voter: FIXTURE_ADDR_1,
      })
    ).result.unwrap(),
    true,
  );

  const depsNone: DependencyView = {
    conflicts_with: undefined,
    requires: [],
  };
  const depsNoneCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "get_dependencies",
    depsNone,
  );
  const decodedDepsNone = (
    await depsNoneCtx.client.get_dependencies({ proposal_id: 1n })
  ).result.unwrap();
  assert.equal(decodedDepsNone.conflicts_with, null);
  assert.deepEqual(decodedDepsNone.requires, []);

  const depsSome: DependencyView = {
    conflicts_with: 9n,
    requires: [1n, 4n],
  };
  const depsSomeCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "get_dependencies",
    depsSome,
  );
  const decodedDepsSome = (
    await depsSomeCtx.client.get_dependencies({ proposal_id: 5n })
  ).result.unwrap();
  assert.equal(decodedDepsSome.conflicts_with, 9n);
  assert.deepEqual(decodedDepsSome.requires, [1n, 4n]);
});

test("dao client decodes get_proposal and get_proposals with Option None and Some", async () => {
  const baseProposal: Proposal = {
    action: Buffer.from([0xde, 0xad, 0xbe, 0xef]),
    against_votes: 100n,
    bond_amount: 1_000n,
    bond_state: { tag: "Posted", values: undefined },
    bond_token: FIXTURE_TOKEN,
    conflicts_with: undefined,
    for_votes: 500n,
    proposal_id: 1n,
    proposer: FIXTURE_ADDR_1,
    requires: [],
    state: { tag: "Active", values: undefined },
    target: FIXTURE_CONTRACT_ID,
    voting_ends: 86_400n,
  };
  const propNoneCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "get_proposal",
    baseProposal,
  );
  const decodedNone = (
    await propNoneCtx.client.get_proposal({ proposal_id: 1n })
  ).result.unwrap();
  assert.equal(decodedNone.proposal_id, 1n);
  assert.equal(decodedNone.conflicts_with, null);
  assert.equal(decodedNone.state.tag, "Active");
  assert.equal(decodedNone.bond_state.tag, "Posted");
  assert.deepEqual(
    Buffer.from(decodedNone.action),
    Buffer.from([0xde, 0xad, 0xbe, 0xef]),
  );

  const executedProposal: Proposal = {
    ...baseProposal,
    bond_state: { tag: "Refunded", values: undefined },
    conflicts_with: 3n,
    proposal_id: 2n,
    requires: [1n],
    state: { tag: "Executed", values: undefined },
  };
  const listCtx = createClientWithReturn(
    dao.Client,
    specClient.spec,
    "get_proposals",
    [baseProposal, executedProposal],
  );
  const decodedList = (
    await listCtx.client.get_proposals({ offset: 0, limit: 10 })
  ).result.unwrap();
  assert.equal(decodedList.length, 2);
  assert.equal(decodedList[1]!.conflicts_with, 3n);
  assert.deepEqual(decodedList[1]!.requires, [1n]);
  assert.equal(decodedList[1]!.state.tag, "Executed");
  assert.equal(decodedList[1]!.bond_state.tag, "Refunded");
});

// ---------------------------------------------------------------------------
// Failed simulations and contract-level errors
// ---------------------------------------------------------------------------

test("dao client reports a failed simulation as an error rather than empty success", async () => {
  const { client, server } = createClientWithFailure(
    dao.Client,
    "dao governance simulation reverted",
  );
  const tx = await client.get_proposal({ proposal_id: 77n });
  assert.equal(server.simulationCount, 1);
  assert.throws(() => tx.result, /dao governance simulation reverted/);
});

test("dao client decodes on-chain ForgeError Result::Err without silent success", async () => {
  const server = new FixtureRpcServer(
    successfulSimulation(contractErrorScVal(15)),
  );
  const client = new dao.Client({ ...TESTNET_CONFIG, server });
  const tx = await client.propose({
    proposer: FIXTURE_ADDR_1,
    target: FIXTURE_CONTRACT_ID,
    action: Buffer.from("exec"),
    duration: 3_600n,
    requires: [],
    conflicts_with: undefined,
  });
  assert.equal(tx.result.isErr(), true);
  assert.throws(() => tx.result.unwrap());
});
