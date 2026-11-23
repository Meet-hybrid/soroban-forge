import test from "node:test";
import assert from "node:assert/strict";
import { dao } from "../../dist/index.js";
import { CONTRACT_ID, TESTNET_PASSPHRASE, RPC_URL, assertEveryMethodEncodes, assertForgeErrors, assertNetworkConstruction, assertDeterministicAddresses, FixtureRpcServer, successfulSimulation, failedSimulation, nativeToScVal } from "./offline-client-test-utils.js";

const createClient = (options: Record<string, unknown>) => new dao.Client(options as never);

test("dao: construction works on testnet, mainnet and custom networks", () => {
  assertDeterministicAddresses();
  assertNetworkConstruction(createClient);
});

test("dao: every ABI method encodes zero and edge arguments", () => {
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL });
  assertEveryMethodEncodes("dao", client);
});

test("dao: ForgeError 1 through 17 map to names and messages", () => assertForgeErrors(dao.ForgeError));

test("dao: main read methods decode fixture ScVals", async () => {
  const server = new FixtureRpcServer(successfulSimulation(nativeToScVal(9n, { type: "u64" })));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.get_proposal_count();
  assert.equal(tx.result, 9n);
  assert.equal(server.simulationCount, 1);
});

test("dao: failed simulation is surfaced as an error", async () => {
  const server = new FixtureRpcServer(failedSimulation("dao simulation failed"));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.execute({ proposal_id: 1n });
  assert.throws(() => tx.result, /dao simulation failed/);
});
