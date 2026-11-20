import test from "node:test";
import assert from "node:assert/strict";

import {
  DIAGNOSTICS_STALE_MS,
  classifyDiagnosticsError,
  diagnosticsHealth,
  diagnosticsReducer,
  initialDiagnosticsState,
  runDiagnostics,
  safeEndpoint,
  sanitizeDetail,
  type DiagnosticsConfig,
  type DiagnosticsProbes,
} from "./diagnostics.ts";

const config: DiagnosticsConfig = {
  networkName: "Stellar Testnet",
  networkPassphrase: "Test SDF Network ; September 2015",
  rpcUrl: "https://soroban-testnet.stellar.org",
  contractId: "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
};

const CHECKED_AT = 1_800_000_000_000;

function healthyProbes(
  overrides: Partial<DiagnosticsProbes> = {},
): DiagnosticsProbes {
  return {
    getLatestLedger: async () => ({ sequence: 1_234_567 }),
    probeContract: async () => undefined,
    getWalletNetwork: async () => ({
      network: "TESTNET",
      networkPassphrase: config.networkPassphrase,
    }),
    ...overrides,
  };
}

test("healthy diagnostics report network, RPC ledger, and readable contract", async () => {
  const snapshot = await runDiagnostics(config, healthyProbes(), CHECKED_AT);

  assert.equal(snapshot.outcome, "healthy");
  assert.equal(diagnosticsHealth(snapshot, CHECKED_AT), "healthy");
  assert.equal(snapshot.network.status, "pass");
  assert.equal(snapshot.rpc.status, "pass");
  assert.equal(snapshot.contract.status, "pass");
  assert.equal(snapshot.latestLedger, 1_234_567);
  assert.match(snapshot.rpc.detail, /latest ledger #1234567/);
  assert.match(snapshot.contract.detail, /read-only call/);
  assert.deepEqual(snapshot.nextSteps, []);
});

test("wrong wallet network is reported with a clear next step", async () => {
  const snapshot = await runDiagnostics(
    config,
    healthyProbes({
      getWalletNetwork: async () => ({
        network: "PUBLIC",
        networkPassphrase:
          "Public Global Stellar Network ; September 2015",
      }),
    }),
    CHECKED_AT,
  );

  assert.equal(snapshot.network.status, "fail");
  assert.equal(snapshot.network.failureKind, "wrong-network");
  assert.equal(snapshot.outcome, "error");
  assert.equal(diagnosticsHealth(snapshot, CHECKED_AT), "error");
  assert.match(
    snapshot.network.nextStep ?? "",
    /Switch the connected wallet to Stellar Testnet/,
  );
  assert.match(snapshot.network.detail, /Public Global Stellar Network/);
  assert.equal(snapshot.rpc.status, "pass");
});

test("mismatched passphrase errors are classified as wrong network", () => {
  assert.equal(
    classifyDiagnosticsError(
      new Error("Network passphrase does not match the configured testnet"),
      "wallet",
    ),
    "wrong-network",
  );
});

test("unavailable RPC is reported with an actionable endpoint hint", async () => {
  const snapshot = await runDiagnostics(
    config,
    healthyProbes({
      getLatestLedger: async () => {
        throw new Error("fetch failed: getaddrinfo ENOTFOUND rpc.example.org");
      },
      probeContract: async () => {
        throw new Error("Network error: service unavailable");
      },
    }),
    CHECKED_AT,
  );

  assert.equal(snapshot.rpc.status, "fail");
  assert.equal(snapshot.rpc.failureKind, "rpc-unavailable");
  assert.equal(snapshot.contract.failureKind, "rpc-unavailable");
  assert.equal(snapshot.latestLedger, null);
  assert.equal(diagnosticsHealth(snapshot, CHECKED_AT), "error");
  assert.match(
    snapshot.rpc.nextStep ?? "",
    /https:\/\/soroban-testnet\.stellar\.org/,
  );
});

test("missing contract deployment is distinguished from RPC failure", async () => {
  const snapshot = await runDiagnostics(
    config,
    healthyProbes({
      probeContract: async () => {
        throw new Error(
          "HostError: Error(Storage, ContractNotFound): contract not found",
        );
      },
    }),
    CHECKED_AT,
  );

  assert.equal(snapshot.rpc.status, "pass");
  assert.equal(snapshot.contract.status, "fail");
  assert.equal(snapshot.contract.failureKind, "missing-contract");
  assert.match(
    snapshot.contract.nextStep ?? "",
    /Deploy the escrow contract on Testnet/,
  );
  assert.match(snapshot.contract.nextStep ?? "", /CC227UDF…NBZHSB/);
});

test("diagnostics never echo endpoint credentials, keys, or tokens", async () => {
  const secretConfig: DiagnosticsConfig = {
    ...config,
    rpcUrl:
      "https://user:apikey-SUPERSECRET@rpc.example.com/soroban?api_key=SUPERSECRET",
  };
  const snapshot = await runDiagnostics(
    secretConfig,
    healthyProbes({
      getLatestLedger: async () => {
        throw new Error(
          "request to https://user:apikey-SUPERSECRET@rpc.example.com/soroban?api_key=SUPERSECRET failed: Authorization: Bearer SUPERSECRET",
        );
      },
    }),
    CHECKED_AT,
  );

  const rendered = JSON.stringify(snapshot);
  assert.doesNotMatch(rendered, /SUPERSECRET/);
  assert.doesNotMatch(snapshot.rpc.detail, /apikey-/);
  assert.equal(
    safeEndpoint(secretConfig.rpcUrl),
    "https://rpc.example.com/soroban",
  );
  assert.doesNotMatch(
    sanitizeDetail("secret: value-123 token=abc123"),
    /value-123|abc123/,
  );
});

test("skipped probes are reported without marking the panel unhealthy", async () => {
  const snapshot = await runDiagnostics(
    config,
    {
      getLatestLedger: async () => ({ sequence: 99 }),
    },
    CHECKED_AT,
  );

  assert.equal(snapshot.network.status, "skip");
  assert.equal(snapshot.contract.status, "skip");
  assert.equal(snapshot.outcome, "healthy");
});

test("snapshot ages into stale without another network check", async () => {
  const snapshot = await runDiagnostics(config, healthyProbes(), CHECKED_AT);

  assert.equal(diagnosticsHealth(snapshot, CHECKED_AT), "healthy");
  assert.equal(
    diagnosticsHealth(snapshot, CHECKED_AT + DIAGNOSTICS_STALE_MS + 1),
    "stale",
  );
  assert.equal(diagnosticsHealth(null, CHECKED_AT), "loading");
});

test("state handling covers loading, success, and error phases", async () => {
  const snapshot = await runDiagnostics(config, healthyProbes(), CHECKED_AT);

  let state = initialDiagnosticsState;
  assert.equal(state.phase, "idle");

  state = diagnosticsReducer(state, { type: "check-started" });
  assert.equal(state.phase, "loading");
  assert.equal(state.snapshot, null);

  state = diagnosticsReducer(state, { type: "check-finished", snapshot });
  assert.equal(state.phase, "ready");
  assert.equal(state.snapshot, snapshot);

  state = diagnosticsReducer(state, { type: "check-started" });
  assert.equal(state.phase, "loading");
  assert.equal(state.snapshot, snapshot);

  state = diagnosticsReducer(state, {
    type: "check-failed",
    message: "token=SECRET failed",
    at: CHECKED_AT,
  });
  assert.equal(state.phase, "ready");
  assert.equal(state.snapshot, null);
  assert.match(state.error ?? "", /token=\[redacted\]/);
  assert.doesNotMatch(state.error ?? "", /SECRET/);
});
