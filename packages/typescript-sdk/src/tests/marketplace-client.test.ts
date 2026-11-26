import test from "node:test";
import assert from "node:assert/strict";
import { marketplace } from "../../dist/index.js";
import { CONTRACT_ID, TESTNET_PASSPHRASE, RPC_URL, assertEveryMethodEncodes, assertForgeErrors, assertNetworkConstruction, assertDeterministicAddresses, FixtureRpcServer, successfulSimulation, failedSimulation, nativeToScVal } from "./offline-client-test-utils.js";

const createClient = (options: Record<string, unknown>) => new marketplace.Client(options as never);

test("marketplace: construction works on testnet, mainnet and custom networks", () => {
  assertDeterministicAddresses();
  assertNetworkConstruction(createClient);
});

test("marketplace: every ABI method encodes zero and edge arguments", () => {
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL });
  assertEveryMethodEncodes("marketplace", client);
});

test("marketplace: ForgeError 1 through 17 map to names and messages", () => assertForgeErrors(marketplace.ForgeError));

test("marketplace: main read methods decode fixture ScVals", async () => {
  const fixture = {
    gross: 250n,
    royalty_amount: 25n,
    royalty_bps: 1000n,
    seller_net: 225n,
  };
  const server = new FixtureRpcServer(successfulSimulation(nativeToScVal(fixture)));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.quote_sale({ collection: CONTRACT_ID, amount: 250n });
  assert.deepEqual(tx.result.unwrap(), fixture);
  assert.equal(server.simulationCount, 1);
});

test("marketplace: failed simulation is surfaced as an error", async () => {
  const server = new FixtureRpcServer(failedSimulation("marketplace simulation failed"));
  const client = createClient({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL, server });
  const tx = await client.touch_ttl({ collection: CONTRACT_ID });
  assert.throws(() => tx.result, /marketplace simulation failed/);
});
