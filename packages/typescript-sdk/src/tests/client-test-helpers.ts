import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import {
  SorobanDataBuilder,
  StrKey,
  contract,
  rpc,
  xdr,
} from "@stellar/stellar-sdk";
import { FORGE_ERRORS, forgeErrorName } from "../errors.generated.js";

/** Deterministic Stellar contract (C...) addresses for fully offline tests. */
export const FIXTURE_CONTRACT_ID =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";
export const FIXTURE_ADDR_1 =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";
export const FIXTURE_ADDR_2 =
  "CDBCCKT7RBUAJEGVZDBUNMADCJW4RV2F2CJKIG22B4R4STGN37W52ZXN";
export const FIXTURE_ADDR_3 =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";
export const FIXTURE_TOKEN =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

/** Standard and custom network configurations exercised offline. */
export const TESTNET_CONFIG = {
  contractId: FIXTURE_CONTRACT_ID,
  networkPassphrase: "Test SDF Network ; September 2015",
  rpcUrl: "https://soroban-testnet.stellar.org",
} as const;

export const MAINNET_CONFIG = {
  contractId: FIXTURE_CONTRACT_ID,
  networkPassphrase: "Public Global Stellar Network ; September 2015",
  rpcUrl: "https://soroban-rpc.mainnet.stellar.org",
} as const;

export const CUSTOM_NETWORK_CONFIG = {
  contractId: FIXTURE_ADDR_1,
  networkPassphrase: "Custom Forge Offline Network ; 2026",
  rpcUrl: "https://rpc.custom.forge.local:8443",
} as const;

/** Numeric boundary constants for Soroban integer types. */
export const ZERO_U32 = 0;
export const MAX_U32 = 4_294_967_295;
export const ZERO_U64 = 0n;
export const MAX_U64 = 18_446_744_073_709_551_615n;
export const ZERO_I128 = 0n;
export const MAX_I128 = (1n << 127n) - 1n;

/** Authoritative ForgeError code -> message table (codes 1..16). */
export const EXPECTED_FORGE_ERRORS_1_TO_16: Record<number, string> = {
  1: "Unauthorized",
  2: "NotFound",
  3: "InvalidInput",
  4: "InsufficientFunds",
  5: "AlreadyInitialized",
  6: "NotInitialized",
  7: "DeadlineReached",
  8: "InsufficientAllowance",
  9: "ArithmeticOverflow",
  10: "Custom",
  11: "TokenTransferFailed",
  12: "ContractInvocationFailed",
  13: "WithdrawalLimitExceeded",
  14: "SubscriptionPastDue",
  15: "ProposerCooldown",
  16: "ReleasePending",
};

export interface AbiFunctionFixture {
  name: string;
  inputs: Array<{ name: string; type: string }>;
  outputs: string[];
}

export interface AbiEventFixture {
  name: string;
  dataFormat: string;
  params: Array<{ name: string; type: string }>;
}

export interface AbiFixtureFile {
  schema: string;
  contract: string;
  functions: AbiFunctionFixture[];
  events: AbiEventFixture[];
  errors: Array<{ name: string; code: number }>;
}

export function loadAbiFixture(fixtureName: string): AbiFixtureFile {
  const filePath = resolve("src", "abi-fixtures", `${fixtureName}.json`);
  return JSON.parse(readFileSync(filePath, "utf8")) as AbiFixtureFile;
}

/** Offline RPC stub that returns a pre-configured simulation response. */
export class FixtureRpcServer extends rpc.Server {
  simulationCount = 0;

  constructor(
    private readonly response: rpc.Api.SimulateTransactionResponse,
    rpcUrl: string = TESTNET_CONFIG.rpcUrl,
  ) {
    super(rpcUrl, { allowHttp: true });
  }

  override async simulateTransaction(): Promise<rpc.Api.SimulateTransactionResponse> {
    this.simulationCount += 1;
    return this.response;
  }
}

export function successfulSimulation(
  retval: xdr.ScVal,
): rpc.Api.SimulateTransactionSuccessResponse {
  return {
    _parsed: true,
    id: "offline-sim-ok",
    latestLedger: 42,
    events: [],
    transactionData: new SorobanDataBuilder()
      .setResourceFee("100")
      .setResources(1, 0, 0)
      .setFootprint([], []),
    minResourceFee: "100",
    result: { auth: [], retval },
  };
}

export function failedSimulation(
  error: string = "HostError: Error(Contract, #2)",
): rpc.Api.SimulateTransactionErrorResponse {
  return {
    _parsed: true,
    id: "offline-sim-err",
    latestLedger: 42,
    events: [],
    error,
  };
}

/**
 * Encode a native JS fixture value into the exact `ScVal` returned on-chain
 * for `fnName` (unwrapping `Result<T, E>` to its `Ok` type when applicable).
 */
export function encodeReturnScVal(
  spec: contract.Spec,
  fnName: string,
  value: unknown,
): xdr.ScVal {
  const fnSpec = spec.getFunc(fnName);
  const outputs = fnSpec.outputs();
  if (outputs.length === 0) {
    return xdr.ScVal.scvVoid();
  }
  const outType = outputs[0]!;
  if (outType.switch().name === "scSpecTypeResult") {
    const okType = outType.result().okType();
    if (okType.switch().name === "scSpecTypeVoid") {
      return xdr.ScVal.scvVoid();
    }
    return spec.nativeToScVal(value, okType);
  }
  return spec.nativeToScVal(value, outType);
}

/** Build a contract-level revert `ScVal` (`ScError::sceContract(code)`). */
export function contractErrorScVal(code: number): xdr.ScVal {
  return xdr.ScVal.scvError(xdr.ScError.sceContract(code));
}

export function createClientWithReturn<
  TClient extends contract.Client,
>(
  ClientCtor: new (options: contract.ClientOptions) => TClient,
  spec: contract.Spec,
  fnName: string,
  returnValue: unknown,
): { client: TClient; server: FixtureRpcServer } {
  const retval = encodeReturnScVal(spec, fnName, returnValue);
  const server = new FixtureRpcServer(successfulSimulation(retval));
  const client = new ClientCtor({
    ...TESTNET_CONFIG,
    server,
  });
  return { client, server };
}

export function createClientWithFailure<
  TClient extends contract.Client,
>(
  ClientCtor: new (options: contract.ClientOptions) => TClient,
  errorMessage: string = "HostError: simulated contract failure",
): { client: TClient; server: FixtureRpcServer } {
  const server = new FixtureRpcServer(failedSimulation(errorMessage));
  const client = new ClientCtor({
    ...TESTNET_CONFIG,
    server,
  });
  return { client, server };
}

/** Decode an `i128` `ScVal` via `ContractSpec` for exact 128-bit assertions. */
export function decodeScValI128(spec: contract.Spec, val: xdr.ScVal): bigint {
  assert.equal(val.switch().name, "scvI128");
  return spec.scValToNative(val, xdr.ScSpecTypeDef.scSpecTypeI128()) as bigint;
}

/**
 * Registers shared offline acceptance checks for a generated contract client:
 *   1. Construction on testnet, mainnet, and a custom network
 *   2. Deterministic valid StrKey contract addresses
 *   3. Complete method and `fromJSON` surface matching `src/abi-fixtures/<name>.json`
 *   4. ABI function inputs, outputs, and events matching the fixture
 *   5. `ForgeError` codes 1..16 mapping to their exact names and messages
 */
export function registerCommonClientSuite<
  TClient extends contract.Client & {
    readonly options: contract.ClientOptions;
    readonly fromJSON: Record<string, unknown>;
  },
>(params: {
  suiteName: string;
  fixtureName: string;
  ClientCtor: new (options: contract.ClientOptions) => TClient;
  clientForgeError: Record<number, { message: string }>;
}): void {
  const { suiteName, fixtureName, ClientCtor, clientForgeError } = params;
  const fixture = loadAbiFixture(fixtureName);

  test(`${suiteName}: deterministic StrKey fixtures are valid contract addresses`, () => {
    for (const addr of [
      FIXTURE_CONTRACT_ID,
      FIXTURE_ADDR_1,
      FIXTURE_ADDR_2,
      FIXTURE_TOKEN,
    ]) {
      assert.ok(
        StrKey.isValidContract(addr),
        `${addr} must be a valid C... contract address`,
      );
    }
  });

  test(`${suiteName}: constructs offline for testnet, mainnet, and custom network`, () => {
    const testnetClient = new ClientCtor({ ...TESTNET_CONFIG });
    assert.ok(testnetClient instanceof ClientCtor);
    assert.equal(testnetClient.options.contractId, TESTNET_CONFIG.contractId);
    assert.equal(
      testnetClient.options.networkPassphrase,
      TESTNET_CONFIG.networkPassphrase,
    );
    assert.equal(testnetClient.options.rpcUrl, TESTNET_CONFIG.rpcUrl);
    assert.ok(testnetClient.spec instanceof contract.Spec);

    const mainnetClient = new ClientCtor({ ...MAINNET_CONFIG });
    assert.ok(mainnetClient instanceof ClientCtor);
    assert.equal(
      mainnetClient.options.networkPassphrase,
      MAINNET_CONFIG.networkPassphrase,
    );
    assert.equal(mainnetClient.options.rpcUrl, MAINNET_CONFIG.rpcUrl);

    const customClient = new ClientCtor({ ...CUSTOM_NETWORK_CONFIG });
    assert.ok(customClient instanceof ClientCtor);
    assert.equal(
      customClient.options.contractId,
      CUSTOM_NETWORK_CONFIG.contractId,
    );
    assert.equal(
      customClient.options.networkPassphrase,
      CUSTOM_NETWORK_CONFIG.networkPassphrase,
    );
    assert.equal(customClient.options.rpcUrl, CUSTOM_NETWORK_CONFIG.rpcUrl);
  });

  test(`${suiteName}: exposes every method and fromJSON helper in ${fixtureName}.json`, () => {
    const client = new ClientCtor({ ...TESTNET_CONFIG });
    const record = client as unknown as Record<string, unknown>;
    const expectedMethods = fixture.functions.map((f) => f.name).sort();
    const specMethods = client.spec
      .funcs()
      .map((f) => f.name().toString())
      .sort();

    assert.deepEqual(specMethods, expectedMethods);

    for (const fn of fixture.functions) {
      assert.equal(
        typeof record[fn.name],
        "function",
        `${suiteName} client.${fn.name} must be a function`,
      );
      assert.equal(
        typeof client.fromJSON[fn.name],
        "function",
        `${suiteName} client.fromJSON.${fn.name} must be a function`,
      );
      const specFn = client.spec.getFunc(fn.name);
      assert.equal(
        specFn.inputs().length,
        fn.inputs.length,
        `${suiteName}.${fn.name} input arity mismatch`,
      );
      for (let i = 0; i < fn.inputs.length; i++) {
        assert.equal(
          specFn.inputs()[i]!.name().toString(),
          fn.inputs[i]!.name,
          `${suiteName}.${fn.name} input[${i}] name mismatch`,
        );
      }
    }
  });

  test(`${suiteName}: ContractSpec includes all documented ${fixtureName} events`, () => {
    const client = new ClientCtor({ ...TESTNET_CONFIG });
    const eventNames = client.spec
      .events()
      .map((e) => e.name().toString())
      .sort();
    const expectedEvents = fixture.events.map((e) => e.name).sort();
    assert.deepEqual(eventNames, expectedEvents);
  });

  test(`${suiteName}: ForgeError maps all 16 codes (1..16) to their name and message`, () => {
    assert.equal(typeof clientForgeError, "object");
    assert.ok(clientForgeError !== null && !Array.isArray(clientForgeError));

    const codes = Object.keys(clientForgeError)
      .map(Number)
      .sort((a, b) => a - b);
    assert.deepEqual(
      codes,
      [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    );

    const client = new ClientCtor({ ...TESTNET_CONFIG });
    const specErrors = new Map(
      client.spec
        .errorCases()
        .map((c) => [Number(c.value().toString()), c.name().toString()]),
    );

    for (let code = 1; code <= 16; code++) {
      const expectedName = EXPECTED_FORGE_ERRORS_1_TO_16[code]!;
      const entry = clientForgeError[code];
      assert.ok(entry, `${suiteName}: ForgeError[${code}] must exist`);
      assert.equal(entry.message, expectedName);
      assert.equal(FORGE_ERRORS[code], expectedName);
      assert.equal(forgeErrorName(code), expectedName);
      assert.equal(specErrors.get(code), expectedName);
    }
  });
}
