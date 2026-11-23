import test from "node:test";
import assert from "node:assert/strict";
import { subscription } from "../../dist/index.js";
import {
  ADDRESS_A, RPC_URL, CONTRACT_ID, TESTNET_PASSPHRASE, MAINNET_PASSPHRASE,
  assertEveryMethodEncodes, assertForgeErrors, assertNetworkConstruction,
  assertDeterministicAddresses, FixtureRpcServer, successfulSimulation,
  failedSimulation, nativeToScVal,
} from "./offline-client-test-utils.js";

const createClient = (options: Record<string, unknown>) => new subscription.Client(options as never);

test("subscription: construction works on testnet, mainnet and custom networks", () => {
  assertDeterministicAddresses();
  assertNetworkConstruction(createClient);
});

test("subscription: every ABI method encodes zero and edge arguments", () => {
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL });
  assertEveryMethodEncodes("subscription", client);
});

test("subscription: ForgeError 1 through 17 map to names and messages", () => {
  assertForgeErrors(subscription.ForgeError);
});

test("subscription: main read methods decode fixture ScVals", async () => {
  const server = new FixtureRpcServer(successfulSimulation(nativeToScVal(42n, { type: "i128" })));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.quote_period({ subscription_id: 7n });
  assert.equal(tx.result.unwrap(), 42n);
  assert.equal(server.simulationCount, 1);
});

test("subscription: failed simulation is surfaced as an error", async () => {
  const server = new FixtureRpcServer(failedSimulation("subscription simulation failed"));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.cancel({ subscription_id: 1n });
  assert.throws(() => tx.result, /subscription simulation failed/);
});
