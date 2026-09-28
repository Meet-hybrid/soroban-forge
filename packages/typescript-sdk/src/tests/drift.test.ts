/**
 * Fixture-driven drift tests across all Soroban Forge contracts.
 *
 * Verifies that the TypeScript SDK client specs (function names, parameter names,
 * parameter types, and events) match the authoritative checked-in JSON fixtures
 * in `src/fixtures/*.fixture.json`.
 *
 * If any contract ABI or event signature is changed without regenerating the
 * corresponding fixture, these tests will fail immediately with clear diffs.
 *
 * Run:
 *   npm test
 */

import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { StrKey } from "@stellar/stellar-sdk";

import {
  escrow,
  subscriptionPayments,
  multiSigWallet,
  marketplaceRoyalties,
  daoGovernance,
  vesting,
} from "../../dist/index.js";

const DUMMY_RPC_URL = "https://soroban-testnet.stellar.org";

interface FixtureParam {
  name: string;
  type: string;
}

interface FixtureFunc {
  name: string;
  inputs: Array<FixtureParam>;
}

interface ContractFixture {
  contract: string;
  functions: Array<FixtureFunc>;
  events: Array<string>;
}

function loadFixture(contractName: string): ContractFixture {
  const possiblePaths = [
    new URL(`../fixtures/${contractName}.fixture.json`, import.meta.url),
    new URL(`../../src/fixtures/${contractName}.fixture.json`, import.meta.url),
  ];

  for (const p of possiblePaths) {
    const filePath = fileURLToPath(p);
    if (fs.existsSync(filePath)) {
      return JSON.parse(fs.readFileSync(filePath, "utf8")) as ContractFixture;
    }
  }

  throw new Error(`Fixture not found for contract "${contractName}"`);
}

interface ContractModuleSuite {
  name: string;
  module: {
    Client: new (...args: any[]) => any;
    networks: {
      testnet: {
        networkPassphrase: string;
        contractId: string;
      };
    };
    config?: {
      contractId: string;
      networkPassphrase: string;
    };
  };
}

const CONTRACT_MODULES: Array<ContractModuleSuite> = [
  { name: "escrow", module: escrow },
  { name: "subscription-payments", module: subscriptionPayments },
  { name: "multi-sig-wallet", module: multiSigWallet },
  { name: "marketplace-royalties", module: marketplaceRoyalties },
  { name: "dao-governance", module: daoGovernance },
  { name: "vesting", module: vesting },
];

for (const { name, module } of CONTRACT_MODULES) {
  test(`${name}: Client can be constructed and carries valid contract ID`, () => {
    const client = new module.Client({
      ...module.networks.testnet,
      rpcUrl: DUMMY_RPC_URL,
    });
    assert.ok(client instanceof module.Client, "client must be a Client instance");
    assert.ok(client.spec, "client must expose spec");

    const contractId = module.networks.testnet.contractId;
    assert.ok(
      StrKey.isValidContract(contractId),
      `${contractId} must be a valid Stellar contract address for ${name}`,
    );
    assert.equal(
      module.networks.testnet.networkPassphrase,
      "Test SDF Network ; September 2015",
    );
  });

  test(`${name}: ABI function list matches checked-in fixture`, () => {
    const fixture = loadFixture(name);
    const client = new module.Client({
      ...module.networks.testnet,
      rpcUrl: DUMMY_RPC_URL,
    });

    const actualFuncNames = client.spec.funcs().map((f: any) => f.name().toString());
    const expectedFuncNames = fixture.functions.map((f) => f.name);

    assert.deepEqual(
      [...actualFuncNames].sort(),
      [...expectedFuncNames].sort(),
      `Function surface drift detected in contract "${name}"`,
    );
  });

  test(`${name}: ABI function parameter names and types match checked-in fixture`, () => {
    const fixture = loadFixture(name);
    const client = new module.Client({
      ...module.networks.testnet,
      rpcUrl: DUMMY_RPC_URL,
    });

    const expectedMap = new Map<string, Array<FixtureParam>>();
    for (const fn of fixture.functions) {
      expectedMap.set(fn.name, fn.inputs);
    }

    for (const fn of client.spec.funcs()) {
      const fnName = fn.name().toString();
      const expectedInputs = expectedMap.get(fnName);
      assert.ok(expectedInputs !== undefined, `Unexpected function in spec: ${fnName}`);

      const actualInputs = fn.inputs();
      assert.equal(
        actualInputs.length,
        expectedInputs.length,
        `${name}.${fnName} parameter count mismatch`,
      );

      for (let i = 0; i < expectedInputs.length; i++) {
        const expected = expectedInputs[i]!;
        const actual = actualInputs[i]!;

        assert.equal(
          actual.name().toString(),
          expected.name,
          `${name}.${fnName} param[${i}] name mismatch`,
        );
        assert.equal(
          actual.type().switch().name,
          expected.type,
          `${name}.${fnName} param[${i}] (${expected.name}) type mismatch`,
        );
      }
    }
  });

  test(`${name}: Contract events match checked-in fixture`, () => {
    const fixture = loadFixture(name);
    const client = new module.Client({
      ...module.networks.testnet,
      rpcUrl: DUMMY_RPC_URL,
    });

    const actualEvents = client.spec.events().map((e: any) => e.name().toString());
    assert.deepEqual(
      [...actualEvents].sort(),
      [...fixture.events].sort(),
      `Event definition drift detected in contract "${name}"`,
    );
  });
}
