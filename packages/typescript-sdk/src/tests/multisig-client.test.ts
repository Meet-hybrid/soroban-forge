import test from "node:test";
import assert from "node:assert/strict";
import { multisig } from "../../dist/index.js";
import { CONTRACT_ID, TESTNET_PASSPHRASE, RPC_URL, assertEveryMethodEncodes, assertForgeErrors, assertNetworkConstruction, assertDeterministicAddresses, FixtureRpcServer, successfulSimulation, failedSimulation, nativeToScVal } from "./offline-client-test-utils.js";

const createClient = (options: Record<string, unknown>) => new multisig.Client(options as never);

test("multisig: construction works on testnet, mainnet and custom networks", () => {
  assertDeterministicAddresses();
  assertNetworkConstruction(createClient);
});

test("multisig: every ABI method encodes zero and edge arguments", () => {
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL });
  assertEveryMethodEncodes("multisig", client);
});

test("multisig: ForgeError 1 through 17 map to names and messages", () => assertForgeErrors(multisig.ForgeError));

test("multisig: main read methods decode fixture ScVals", async () => {
  const server = new FixtureRpcServer(successfulSimulation(nativeToScVal(3n, { type: "u32" })));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.get_threshold();
  assert.equal(tx.result.unwrap(), 3);
  assert.equal(server.simulationCount, 1);
});

test("multisig: failed simulation is surfaced as an error", async () => {
  const server = new FixtureRpcServer(failedSimulation("multisig simulation failed"));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.execute({ tx_id: 1n });
  assert.throws(() => tx.result, /multisig simulation failed/);
});
