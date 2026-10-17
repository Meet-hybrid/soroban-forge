import assert from "node:assert/strict";
import test from "node:test";
import {
  createClientContractProbe,
  createJsonRpcProbe,
  DEFAULT_STALE_THRESHOLD_MS,
  resolveDisplayDiagnosticsState,
  runDiagnosticsCheck,
  sanitizeDiagnosticText,
} from "./diagnostics.ts";

const TESTNET_PASSPHRASE = "Test SDF Network ; September 2015";
const PUBLIC_PASSPHRASE = "Public Global Stellar Network ; September 2015";
const VALID_CONTRACT_ID =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";
const TESTNET_RPC_URL = "https://soroban-testnet.stellar.org";

test("reports healthy state when RPC is reachable, network matches, and contract is readable", async () => {
  const report = await runDiagnosticsCheck({
    rpcUrl: TESTNET_RPC_URL,
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    contractId: VALID_CONTRACT_ID,
    checkedAtMs: 1_700_000_000_000,
    probeRpc: async () => ({
      reachable: true,
      networkPassphrase: TESTNET_PASSPHRASE,
      latestLedger: 512_345,
      latestLedgerCloseTime: 1_700_000_000,
      protocolVersion: 23,
    }),
    probeContract: async () => ({
      readable: true,
      detail: "Read-only simulation succeeded (indexed participant total: 3).",
    }),
  });

  assert.equal(report.status, "healthy");
  assert.equal(report.rpcReachable, true);
  assert.equal(report.networkMatches, true);
  assert.equal(report.contractReadable, true);
  assert.equal(report.latestLedger, 512_345);
  assert.equal(report.protocolVersion, 23);
  assert.equal(report.nextSteps.length, 0);
  assert.match(report.summary, /512345/);
});

test("reports wrong-network state when RPC or wallet passphrase mismatches configured network", async () => {
  const rpcMismatch = await runDiagnosticsCheck({
    rpcUrl: "https://mainnet-rpc.example.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    contractId: VALID_CONTRACT_ID,
    probeRpc: async () => ({
      reachable: true,
      networkPassphrase: PUBLIC_PASSPHRASE,
      latestLedger: 999_999,
      protocolVersion: 23,
    }),
    probeContract: async () => ({
      readable: true,
    }),
  });

  assert.equal(rpcMismatch.status, "wrong-network");
  assert.equal(rpcMismatch.rpcReachable, true);
  assert.equal(rpcMismatch.networkMatches, false);
  assert.equal(rpcMismatch.contractReadable, false);
  assert.ok(rpcMismatch.nextSteps.length >= 2);
  assert.match(rpcMismatch.summary, /Wrong network detected/);

  const walletMismatch = await runDiagnosticsCheck({
    rpcUrl: TESTNET_RPC_URL,
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    walletNetworkPassphrase: PUBLIC_PASSPHRASE,
    contractId: VALID_CONTRACT_ID,
    probeRpc: async () => ({
      reachable: true,
      networkPassphrase: TESTNET_PASSPHRASE,
      latestLedger: 512_346,
    }),
  });

  assert.equal(walletMismatch.status, "wrong-network");
  assert.equal(walletMismatch.networkMatches, false);
  assert.match(walletMismatch.summary, /Connected wallet is on/);
});

test("reports unavailable-rpc state when RPC endpoint cannot be reached", async () => {
  const report = await runDiagnosticsCheck({
    rpcUrl: "https://unreachable-soroban-rpc.example.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    contractId: VALID_CONTRACT_ID,
    probeRpc: async () => ({
      reachable: false,
      errorMessage: "fetch failed: ECONNREFUSED",
    }),
  });

  assert.equal(report.status, "unavailable-rpc");
  assert.equal(report.rpcReachable, false);
  assert.equal(report.contractReadable, false);
  assert.equal(report.latestLedger, null);
  assert.ok(report.nextSteps.length >= 2);
  assert.match(report.summary, /unavailable/i);
});

test("reports missing-contract state when contract ID is invalid or contract cannot be read", async () => {
  const invalidIdReport = await runDiagnosticsCheck({
    rpcUrl: TESTNET_RPC_URL,
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    contractId: "NOT_A_VALID_CONTRACT_ID",
    probeRpc: async () => ({
      reachable: true,
      networkPassphrase: TESTNET_PASSPHRASE,
      latestLedger: 512_400,
    }),
  });

  assert.equal(invalidIdReport.status, "missing-contract");
  assert.equal(invalidIdReport.rpcReachable, true);
  assert.equal(invalidIdReport.contractReadable, false);
  assert.ok(invalidIdReport.nextSteps.length >= 2);

  const undeployedContractProbe = createClientContractProbe(() => ({
    escrows_for_participant: async () => {
      throw new Error(
        "HostError: Error(Storage, MissingValue) — contract instance not found"
      );
    },
  }));

  const missingOnChainReport = await runDiagnosticsCheck({
    rpcUrl: TESTNET_RPC_URL,
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    contractId: VALID_CONTRACT_ID,
    probeRpc: async () => ({
      reachable: true,
      networkPassphrase: TESTNET_PASSPHRASE,
      latestLedger: 512_401,
    }),
    probeContract: undeployedContractProbe,
  });

  assert.equal(missingOnChainReport.status, "missing-contract");
  assert.equal(missingOnChainReport.rpcReachable, true);
  assert.equal(missingOnChainReport.contractReadable, false);
  assert.match(missingOnChainReport.contractDetail, /MissingValue/);
  assert.ok(
    missingOnChainReport.nextSteps.some((step) =>
      step.includes("stellar contract restore")
    )
  );
});

test("handles loading, healthy, stale, and error display states", async () => {
  const checkedAtMs = 1_700_000_000_000;
  const healthyReport = await runDiagnosticsCheck({
    rpcUrl: TESTNET_RPC_URL,
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    contractId: VALID_CONTRACT_ID,
    checkedAtMs,
    probeRpc: async () => ({
      reachable: true,
      networkPassphrase: TESTNET_PASSPHRASE,
      latestLedger: 100,
    }),
    probeContract: async () => ({ readable: true }),
  });

  assert.equal(
    resolveDisplayDiagnosticsState(null, false, checkedAtMs).displayState,
    "idle"
  );
  assert.equal(
    resolveDisplayDiagnosticsState(healthyReport, true, checkedAtMs + 5_000)
      .displayState,
    "loading"
  );
  assert.equal(
    resolveDisplayDiagnosticsState(healthyReport, false, checkedAtMs + 10_000)
      .displayState,
    "healthy"
  );

  const staleState = resolveDisplayDiagnosticsState(
    healthyReport,
    false,
    checkedAtMs + DEFAULT_STALE_THRESHOLD_MS + 1_000
  );
  assert.equal(staleState.displayState, "stale");
  assert.equal(staleState.isStale, true);
});

test("redacts Stellar secret seeds, URL credentials, and API tokens from failure messages", async () => {
  const secretSeed = "SAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
  const raw = `Failed connecting to https://admin:supersecret@rpc.example.org?api_key=my_secret_token123 with signer ${secretSeed}`;
  const sanitized = sanitizeDiagnosticText(raw);

  assert.equal(sanitized.includes(secretSeed), false);
  assert.equal(sanitized.includes("supersecret"), false);
  assert.equal(sanitized.includes("my_secret_token123"), false);
  assert.match(sanitized, /\[REDACTED_SECRET\]/);
  assert.match(sanitized, /\[REDACTED\]/);

  const report = await runDiagnosticsCheck({
    rpcUrl: "https://user:pass123@soroban-testnet.stellar.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    contractId: VALID_CONTRACT_ID,
    probeRpc: async () => ({
      reachable: false,
      errorMessage: `Auth error with key=${secretSeed} and Bearer secret_jwt_token`,
    }),
  });

  assert.equal(report.summary.includes(secretSeed), false);
  assert.equal(report.summary.includes("secret_jwt_token"), false);
  assert.equal(report.rpcUrl.includes("pass123"), false);
});

test("createJsonRpcProbe queries getHealth, getNetwork, and getLatestLedger over JSON-RPC", async () => {
  const calledMethods: string[] = [];
  const fakeFetch: typeof fetch = async (_input, init) => {
    const body = JSON.parse(String(init?.body ?? "{}")) as { method: string };
    calledMethods.push(body.method);

    if (body.method === "getHealth") {
      return new Response(
        JSON.stringify({ jsonrpc: "2.0", id: "getHealth", result: { status: "healthy" } }),
        { status: 200 }
      );
    }
    if (body.method === "getNetwork") {
      return new Response(
        JSON.stringify({
          jsonrpc: "2.0",
          id: "getNetwork",
          result: { passphrase: TESTNET_PASSPHRASE, protocolVersion: 23 },
        }),
        { status: 200 }
      );
    }
    return new Response(
      JSON.stringify({
        jsonrpc: "2.0",
        id: "getLatestLedger",
        result: { sequence: 777_888, closeTime: "1700001234", protocolVersion: 23 },
      }),
      { status: 200 }
    );
  };

  const probe = createJsonRpcProbe(fakeFetch);
  const res = await probe(TESTNET_RPC_URL);
  assert.equal(res.reachable, true);
  assert.equal(res.networkPassphrase, TESTNET_PASSPHRASE);
  assert.equal(res.latestLedger, 777_888);
  assert.equal(res.latestLedgerCloseTime, 1_700_001_234);
  assert.equal(res.protocolVersion, 23);
  assert.deepEqual(calledMethods.sort(), ["getHealth", "getLatestLedger", "getNetwork"]);
});
