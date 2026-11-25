/** Shared offline helpers for generated-client test suites. */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import {
  nativeToScVal,
  SorobanDataBuilder,
  rpc,
  scValToNative,
  StrKey,
} from "@stellar/stellar-sdk";

export const CONTRACT_ID = "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";
export const TESTNET_PASSPHRASE = "Test SDF Network ; September 2015";
export const MAINNET_PASSPHRASE = "Public Global Stellar Network ; September 2015";
export const RPC_URL = "https://soroban-testnet.stellar.org";
export const CUSTOM_RPC_URL = "https://rpc.fixture.invalid";
export const ADDRESS_A = CONTRACT_ID;
export const ADDRESS_B = "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

export const FORGE_ERRORS: Record<number, string> = {
  1: "Unauthorized", 2: "NotFound", 3: "InvalidInput", 4: "InsufficientFunds",
  5: "AlreadyInitialized", 6: "NotInitialized", 7: "DeadlineReached",
  8: "InsufficientAllowance", 9: "ArithmeticOverflow", 10: "Custom",
  11: "TokenTransferFailed", 12: "ContractInvocationFailed", 13: "WithdrawalLimitExceeded",
  14: "SubscriptionPastDue", 15: "ProposerCooldown", 16: "ReleasePending", 17: "InvalidState",
};

export type AbiInput = { name: string; type: string };
export type AbiFunction = { name: string; inputs: AbiInput[]; outputs: string[] };
export type AbiType = {
  kind: string;
  name: string;
  fields?: Array<{ name: string; type: string }>;
  variants?: Array<{ name: string; types: string[] }>;
  variants_enum?: Array<{ name: string; types: string[] }>;
};

export function fixtureAbi(contract: string): AbiFunction[] {
  const path = resolve("src", "abi-fixtures", `${contract}.json`);
  return (JSON.parse(readFileSync(path, "utf8")) as { functions: AbiFunction[] }).functions;
}

export function fixtureTypes(contract: string): AbiType[] {
  const path = resolve("src", "abi-fixtures", `${contract}.json`);
  return (JSON.parse(readFileSync(path, "utf8")) as { types?: AbiType[] }).types ?? [];
}

export function assertDeterministicAddresses(): void {
  assert.ok(StrKey.isValidContract(CONTRACT_ID));
  assert.ok(StrKey.isValidContract(ADDRESS_B));
}

export function assertNetworkConstruction(create: (options: Record<string, unknown>) => unknown): void {
  assert.ok(create({ contractId: CONTRACT_ID, networkPassphrase: TESTNET_PASSPHRASE, rpcUrl: RPC_URL }));
  assert.ok(create({ contractId: CONTRACT_ID, networkPassphrase: MAINNET_PASSPHRASE, rpcUrl: "https://rpc.mainnet.fixture.invalid" }));
  assert.ok(create({ contractId: CONTRACT_ID, networkPassphrase: "Fixture Custom Network", rpcUrl: CUSTOM_RPC_URL }));
}

export function assertForgeErrors(ForgeError: unknown): void {
  const runtime = ForgeError as Record<number, { message: string }>;
  const runtimeCodes = Object.keys(runtime).map(Number).sort((a, b) => a - b);

  assert.deepEqual(runtimeCodes, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]);
  for (let code = 1; code <= 17; code += 1) {
    assert.equal(runtime[code]?.message, FORGE_ERRORS[code], `ForgeError ${code} name/message`);
  }
}

export function udtValue(contract: string, type: string, edge = false): unknown {
  const name = type.startsWith("udt:") ? type.slice(4) : type;
  const definition = fixtureTypes(contract).find((entry) => entry.name === name);
  if (!definition) return udtValueLegacy(name);
  if (definition.kind === "struct") {
    const result: Record<string, unknown> = {};
    for (const field of definition.fields ?? []) {
      result[field.name] = edgeValue(contract, field.type, edge);
    }
    return result;
  }
  if (definition.kind === "union") {
    const variants = definition.variants ?? definition.variants_enum ?? [];
    const variant = variants[0];
    return variant ? { tag: variant.name, values: undefined } : {};
  }
  if (definition.kind === "enum") {
    const variants = definition.variants ?? definition.variants_enum ?? [];
    const variant = variants[0];
    return variant ? { tag: variant.name, values: undefined } : {};
  }
  return {};
}

function udtValueLegacy(type: string): unknown {
  if (type === "MetricQuota") return { metric: "api", limit: 100n, period: 60n, max_overage_units: undefined };
  if (type === "RoyaltyShare") return { recipient: ADDRESS_A, bps: 1000 };
  if (type === "Tranche") return { amount: 500n, unlock_at: 100n };
  if (type === "TxStatus") return { tag: "Pending", values: undefined };
  return {};
}

export function sampleValue(contract: string, type: string): unknown {
  if (type === "address") return ADDRESS_A;
  if (type === "u64") return 0n;
  if (type === "u32") return 0;
  if (type === "i128") return 0n;
  if (type === "bool") return false;
  if (type === "bytes") return Buffer.alloc(0);
  if (type === "symbol") return "fixture";
  if (type === "val") return 0n;
  if (type.startsWith("option<")) return undefined;
  if (type.startsWith("vec<")) return [];
  if (type.startsWith("tuple<")) {
    const inner = type.slice(6, -1).split(",");
    return inner.map((entry) => sampleValue(contract, entry.trim()));
  }
  if (type.startsWith("udt:")) return udtValue(contract, type);
  return 0n;
}

export function edgeValue(contract: string, type: string, edge = true): unknown {
  if (type === "u64") return 18_446_744_073_709_551_615n;
  if (type === "u32") return 4_294_967_295;
  if (type === "i128") return 170_141_183_460_469_231_731_687_303_715_884_105_727n;
  if (type === "address") return ADDRESS_B;
  if (type === "bytes") return Buffer.alloc(64, 0xff);
  if (type === "symbol") return "x".repeat(32);
  if (type.startsWith("option<")) return edge ? edgeValue(contract, type.slice(7, -1), edge) : undefined;
  if (type.startsWith("vec<")) {
    const inner = type.slice(4, -1);
    return edge ? Array.from({ length: 64 }, () => edgeValue(contract, inner, true)) : [];
  }
  if (type.startsWith("tuple<")) {
    const inner = type.slice(6, -1).split(",");
    return inner.map((entry) => edgeValue(contract, entry.trim(), edge));
  }
  if (type.startsWith("udt:")) return udtValue(contract, type, edge);
  return sampleValue(contract, type);
}

export class FixtureRpcServer extends rpc.Server {
  simulationCount = 0;
  constructor(private readonly response: rpc.Api.SimulateTransactionResponse) {
    super(RPC_URL);
  }
  override async simulateTransaction(): Promise<rpc.Api.SimulateTransactionResponse> {
    this.simulationCount += 1;
    return this.response;
  }
}

export function successfulSimulation(retval: ReturnType<typeof nativeToScVal>): rpc.Api.SimulateTransactionSuccessResponse {
  return {
    _parsed: true,
    id: "forge-fixture",
    latestLedger: 1,
    events: [],
    transactionData: new SorobanDataBuilder().setResourceFee("100").setResources(1, 0, 0).setFootprint([], []),
    minResourceFee: "100",
    result: { auth: [], retval },
  };
}

export function failedSimulation(message = "fixture contract simulation failure"): rpc.Api.SimulateTransactionErrorResponse {
  return { _parsed: true, id: "forge-fixture", latestLedger: 1, events: [], error: message };
}

export { nativeToScVal, scValToNative };

export function assertEveryMethodEncodes(
  contract: string,
  client: { spec: { funcs(): Array<{ name(): { toString(): string }; inputs(): unknown[] }>; funcArgsToScVals(name: string, args: Record<string, unknown>): unknown[] } },
): void {
  const fixtureNames = fixtureAbi(contract).map((fn) => fn.name).sort();
  const specNames = client.spec.funcs().map((fn) => fn.name().toString()).sort();
  assert.deepEqual(specNames, fixtureNames, `${contract}: fixture/spec method coverage`);

  for (const fn of fixtureAbi(contract)) {
    const args: Record<string, unknown> = {};
    const edgeArgs: Record<string, unknown> = {};
    const emptyArgs: Record<string, unknown> = {};
    for (const input of fn.inputs) {
      args[input.name] = sampleValue(contract, input.type);
      edgeArgs[input.name] = edgeValue(contract, input.type, true);
      emptyArgs[input.name] = edgeValue(contract, input.type, false);
    }
    for (const [mode, values] of [["sample", args], ["edge", edgeArgs], ["empty/none", emptyArgs]] as const) {
      const encoded = client.spec.funcArgsToScVals(fn.name, values);
      assert.equal(encoded.length, fn.inputs.length, `${contract}.${fn.name}: ${mode} encoded argument count`);
    }
  }
}
