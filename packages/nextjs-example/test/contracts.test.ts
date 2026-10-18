import test from "node:test";
import assert from "node:assert/strict";
import {
  CONTRACTS,
  type ContractKey,
  createContractClient,
} from "../src/lib/contracts/registry.ts";

test("CONTRACTS registry defines all six Forge contracts with required fields", () => {
  const expectedKeys: ContractKey[] = [
    "escrow",
    "vesting",
    "multi-sig-wallet",
    "dao-governance",
    "subscription-payments",
    "marketplace-royalties",
  ];

  for (const key of expectedKeys) {
    const config = CONTRACTS[key];
    assert.ok(config, `Missing contract config for ${key}`);
    assert.equal(config.key, key);
    assert.ok(config.name.length > 0);
    assert.ok(config.defaultContractId.startsWith("C"));
    assert.ok(config.spec.length > 0);
    assert.ok(config.writeActionName.length > 0);
    assert.ok(config.writeActionDoc.length > 0);
  }
});

test("createContractClient creates functional contract client instances", () => {
  const expectedKeys: ContractKey[] = [
    "escrow",
    "vesting",
    "multi-sig-wallet",
    "dao-governance",
    "subscription-payments",
    "marketplace-royalties",
  ];

  for (const key of expectedKeys) {
    const client = createContractClient(key, {
      contractId: CONTRACTS[key].defaultContractId,
    });
    assert.ok(client, `Client instantiation failed for ${key}`);
    const writeMethod = CONTRACTS[key].writeActionName;
    assert.equal(
      typeof client[writeMethod],
      "function",
      `Expected method ${writeMethod} to exist on ${key} client`
    );
  }
});
