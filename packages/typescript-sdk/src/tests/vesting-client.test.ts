import assert from "node:assert/strict";
import test from "node:test";
import { nativeToScVal, SorobanDataBuilder, rpc } from "@stellar/stellar-sdk";
import { vesting } from "../../dist/index.js";
import type { VestingSchedule } from "../../dist/clients/vesting.js";

const CONTRACT_ID =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";
const NETWORK_PASSPHRASE = "Test SDF Network ; September 2015";
const RPC_URL = "https://soroban-testnet.stellar.org";

const scheduleFixture: VestingSchedule = {
  beneficiary: CONTRACT_ID,
  beneficiary_claimed: 0n,
  claimed: 0n,
  cliff: 10n,
  duration: 100n,
  funder: CONTRACT_ID,
  reassignment_count: 0,
  reassignment_vested: 0n,
  revoked_vested: undefined,
  start: 1n,
  status: { tag: "Locked", values: undefined },
  token: CONTRACT_ID,
  total_amount: 500n,
};

class FixtureRpcServer extends rpc.Server {
  simulationCount = 0;

  constructor(
    private readonly response: rpc.Api.SimulateTransactionResponse,
  ) {
    super(RPC_URL);
  }

  override async simulateTransaction(): Promise<rpc.Api.SimulateTransactionResponse> {
    this.simulationCount += 1;
    return this.response;
  }
}

function successfulSimulation(
  retval: ReturnType<typeof nativeToScVal>,
): rpc.Api.SimulateTransactionSuccessResponse {
  return {
    _parsed: true,
    id: "vesting-fixture",
    latestLedger: 1,
    events: [],
    transactionData: new SorobanDataBuilder()
      .setResourceFee("100")
      .setResources(1, 0, 0)
      .setFootprint([], []),
    minResourceFee: "100",
    result: { auth: [], retval },
  };
}

function createClient(server: FixtureRpcServer): vesting.Client {
  return new vesting.Client({
    contractId: CONTRACT_ID,
    networkPassphrase: NETWORK_PASSPHRASE,
    rpcUrl: RPC_URL,
    server,
  });
}

test("vesting client simulates claimable and decodes the fixture result", async () => {
  assert.equal(scheduleFixture.status.tag, "Locked");

  const server = new FixtureRpcServer(
    successfulSimulation(nativeToScVal(125n, { type: "i128" })),
  );
  const client = createClient(server);
  const transaction = await client.claimable({ schedule_id: 7n });

  assert.equal(server.simulationCount, 1);
  assert.equal(transaction.result.unwrap(), 125n);
});

test("vesting client reports a failed create_schedule simulation", async () => {
  const server = new FixtureRpcServer({
    _parsed: true,
    id: "vesting-fixture",
    latestLedger: 1,
    events: [],
    error: "fixture contract simulation failure",
  });
  const client = createClient(server);

  const transaction = await client.create_schedule({
    funder: CONTRACT_ID,
    beneficiary: CONTRACT_ID,
    token: CONTRACT_ID,
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
