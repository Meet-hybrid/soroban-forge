export const DEFAULT_STALE_THRESHOLD_MS = 60_000;

export type DiagnosticsHealthStatus =
  | "healthy"
  | "wrong-network"
  | "unavailable-rpc"
  | "missing-contract";

export type DiagnosticsDisplayState =
  | "idle"
  | "loading"
  | "stale"
  | DiagnosticsHealthStatus;

export interface RpcProbeResult {
  reachable: boolean;
  networkPassphrase?: string | null;
  latestLedger?: number | null;
  latestLedgerCloseTime?: number | null;
  protocolVersion?: number | null;
  errorMessage?: string;
}

export interface ContractProbeResult {
  readable: boolean;
  detail?: string;
  errorCode?: DiagnosticsHealthStatus;
}

export type RpcProbeFn = (rpcUrl: string) => Promise<RpcProbeResult>;
export type ContractProbeFn = (
  contractId: string,
  rpcUrl: string,
  networkPassphrase: string
) => Promise<ContractProbeResult>;

export interface DiagnosticsCheckOptions {
  rpcUrl: string;
  expectedNetworkPassphrase: string;
  walletNetworkPassphrase?: string | null;
  contractId: string;
  checkedAtMs?: number;
  probeRpc?: RpcProbeFn;
  probeContract?: ContractProbeFn;
}

export interface DiagnosticsReport {
  status: DiagnosticsHealthStatus;
  rpcUrl: string;
  rpcReachable: boolean;
  configuredNetworkPassphrase: string;
  detectedNetworkPassphrase: string | null;
  walletNetworkPassphrase: string | null;
  networkMatches: boolean;
  contractId: string;
  contractReadable: boolean;
  contractDetail: string;
  latestLedger: number | null;
  latestLedgerCloseTime: number | null;
  protocolVersion: number | null;
  checkedAtMs: number;
  summary: string;
  nextSteps: string[];
}

export interface DisplayDiagnosticsState {
  displayState: DiagnosticsDisplayState;
  isStale: boolean;
  ageMs: number | null;
}

const STELLAR_SECRET_SEED_REGEX = /\bS[A-Z2-7]{55}\b/g;
const STELLAR_CONTRACT_ID_REGEX = /^C[A-Z2-7]{55}$/;
const URL_CREDENTIALS_REGEX = /(https?:\/\/)([^/\s:@]+(?::[^/\s@]*)?)@/gi;
const SECRET_QUERY_OR_HEADER_REGEX =
  /((?:bearer\s+|api[_-]?key\s*[=:]\s*|token\s*[=:]\s*|secret\s*[=:]\s*|password\s*[=:]\s*))([^\s&"',;]+)/gi;

export function sanitizeDiagnosticText(input: string): string {
  if (!input) return "";
  return input
    .replace(STELLAR_SECRET_SEED_REGEX, "[REDACTED_SECRET]")
    .replace(URL_CREDENTIALS_REGEX, "$1[REDACTED]@")
    .replace(SECRET_QUERY_OR_HEADER_REGEX, "$1[REDACTED]");
}

export function isValidContractId(contractId: string): boolean {
  return STELLAR_CONTRACT_ID_REGEX.test(contractId.trim());
}

export function classifyProbeError(
  error: unknown
): { code: DiagnosticsHealthStatus; message: string } {
  const rawMessage =
    error instanceof Error ? error.message : String(error ?? "Unknown error");
  const message = sanitizeDiagnosticText(rawMessage);
  const lower = message.toLowerCase();

  if (
    lower.includes("wrong network") ||
    lower.includes("network passphrase") ||
    lower.includes("passphrase mismatch")
  ) {
    return { code: "wrong-network", message };
  }

  if (
    lower.includes("not found") ||
    lower.includes("missingvalue") ||
    lower.includes("not deployed") ||
    lower.includes("wasm") ||
    lower.includes("contract instance") ||
    lower.includes("archived") ||
    lower.includes("invalid contract")
  ) {
    return { code: "missing-contract", message };
  }

  return { code: "unavailable-rpc", message };
}

export function createJsonRpcProbe(
  fetchImpl: typeof fetch = fetch
): RpcProbeFn {
  return async (rpcUrl: string): Promise<RpcProbeResult> => {
    const safeUrl = rpcUrl.trim();
    if (!safeUrl) {
      return {
        reachable: false,
        errorMessage: "RPC URL is not configured.",
      };
    }

    try {
      const callRpc = async (method: string) => {
        const res = await fetchImpl(safeUrl, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            jsonrpc: "2.0",
            id: method,
            method,
          }),
        });
        if (!res.ok) {
          throw new Error(`HTTP ${res.status} from RPC (${method})`);
        }
        const payload = (await res.json()) as {
          result?: Record<string, unknown>;
          error?: { message?: string };
        };
        if (payload.error) {
          throw new Error(payload.error.message ?? `RPC error in ${method}`);
        }
        return payload.result ?? {};
      };

      const [healthResult, networkResult, ledgerResult] = await Promise.all([
        callRpc("getHealth"),
        callRpc("getNetwork"),
        callRpc("getLatestLedger"),
      ]);

      const healthStatus =
        typeof healthResult.status === "string"
          ? healthResult.status.toLowerCase()
          : "healthy";
      if (healthStatus !== "healthy") {
        return {
          reachable: false,
          errorMessage: sanitizeDiagnosticText(
            `RPC reported non-healthy status: ${String(healthResult.status)}`
          ),
        };
      }

      const networkPassphrase =
        typeof networkResult.passphrase === "string"
          ? networkResult.passphrase
          : null;
      const protocolVersion =
        typeof networkResult.protocolVersion === "number"
          ? networkResult.protocolVersion
          : typeof ledgerResult.protocolVersion === "number"
            ? ledgerResult.protocolVersion
            : null;
      const latestLedger =
        typeof ledgerResult.sequence === "number"
          ? ledgerResult.sequence
          : typeof healthResult.latestLedger === "number"
            ? healthResult.latestLedger
            : null;
      const latestLedgerCloseTime =
        typeof ledgerResult.closeTime === "number"
          ? ledgerResult.closeTime
          : typeof ledgerResult.closeTime === "string" &&
              Number.isFinite(Number(ledgerResult.closeTime))
            ? Number(ledgerResult.closeTime)
            : null;

      return {
        reachable: true,
        networkPassphrase,
        latestLedger,
        latestLedgerCloseTime,
        protocolVersion,
      };
    } catch (error) {
      return {
        reachable: false,
        errorMessage: sanitizeDiagnosticText(
          error instanceof Error ? error.message : String(error)
        ),
      };
    }
  };
}

export interface ReadOnlyEscrowProbeClient {
  escrows_for_participant: (args: {
    participant: string;
    cursor: number;
    limit: number;
  }) => Promise<{
    result: {
      ids: Array<bigint>;
      total: number;
    };
  }>;
}

export function createClientContractProbe(
  getClient: (
    contractId: string,
    rpcUrl: string,
    networkPassphrase: string
  ) => ReadOnlyEscrowProbeClient
): ContractProbeFn {
  return async (
    contractId: string,
    rpcUrl: string,
    networkPassphrase: string
  ): Promise<ContractProbeResult> => {
    const trimmed = contractId.trim();
    if (!isValidContractId(trimmed)) {
      return {
        readable: false,
        errorCode: "missing-contract",
        detail:
          "Configured contract identifier is missing or not a valid Stellar contract StrKey (C...).",
      };
    }

    try {
      const client = getClient(trimmed, rpcUrl, networkPassphrase);
      const response = await client.escrows_for_participant({
        participant: trimmed,
        cursor: 0,
        limit: 1,
      });
      const total = response?.result?.total ?? 0;
      return {
        readable: true,
        detail: `Read-only simulation succeeded (indexed participant total: ${total}).`,
      };
    } catch (error) {
      const classified = classifyProbeError(error);
      return {
        readable: false,
        errorCode: classified.code,
        detail: classified.message,
      };
    }
  };
}

export async function runDiagnosticsCheck(
  options: DiagnosticsCheckOptions
): Promise<DiagnosticsReport> {
  const checkedAtMs = options.checkedAtMs ?? Date.now();
  const safeRpcUrl = sanitizeDiagnosticText(options.rpcUrl.trim());
  const expectedNetwork = options.expectedNetworkPassphrase.trim();
  const walletNetwork = options.walletNetworkPassphrase?.trim() || null;
  const contractId = sanitizeDiagnosticText(options.contractId.trim());

  const probeRpc = options.probeRpc ?? createJsonRpcProbe();
  const rpcResult = await probeRpc(options.rpcUrl);

  if (!rpcResult.reachable) {
    const reason = rpcResult.errorMessage
      ? sanitizeDiagnosticText(rpcResult.errorMessage)
      : "RPC endpoint did not respond to health probe.";
    return {
      status: "unavailable-rpc",
      rpcUrl: safeRpcUrl,
      rpcReachable: false,
      configuredNetworkPassphrase: expectedNetwork,
      detectedNetworkPassphrase: null,
      walletNetworkPassphrase: walletNetwork,
      networkMatches: false,
      contractId,
      contractReadable: false,
      contractDetail: "Skipped contract read probe because RPC is unreachable.",
      latestLedger: null,
      latestLedgerCloseTime: null,
      protocolVersion: null,
      checkedAtMs,
      summary: `RPC endpoint is unavailable (${reason}).`,
      nextSteps: [
        `Verify the Soroban RPC endpoint (${safeRpcUrl || "unconfigured"}) is online and reachable from your browser.`,
        "Check network connectivity, firewall/CORS settings, or switch to an active Soroban Testnet RPC provider.",
        "Click 'Run diagnostics' after RPC connectivity is restored.",
      ],
    };
  }

  const detectedNetwork = rpcResult.networkPassphrase?.trim() || null;
  const rpcNetworkMatches =
    detectedNetwork === null || detectedNetwork === expectedNetwork;
  const walletNetworkMatches =
    walletNetwork === null || walletNetwork === expectedNetwork;
  const networkMatches = rpcNetworkMatches && walletNetworkMatches;

  if (!networkMatches) {
    const mismatchDetail = !rpcNetworkMatches
      ? `RPC reported '${detectedNetwork}', expected '${expectedNetwork}'.`
      : `Connected wallet is on '${walletNetwork}', expected '${expectedNetwork}'.`;

    return {
      status: "wrong-network",
      rpcUrl: safeRpcUrl,
      rpcReachable: true,
      configuredNetworkPassphrase: expectedNetwork,
      detectedNetworkPassphrase: detectedNetwork,
      walletNetworkPassphrase: walletNetwork,
      networkMatches: false,
      contractId,
      contractReadable: false,
      contractDetail:
        "Contract probe blocked because the active network does not match the configured Soroban Testnet passphrase.",
      latestLedger: rpcResult.latestLedger ?? null,
      latestLedgerCloseTime: rpcResult.latestLedgerCloseTime ?? null,
      protocolVersion: rpcResult.protocolVersion ?? null,
      checkedAtMs,
      summary: `Wrong network detected: ${sanitizeDiagnosticText(mismatchDetail)}`,
      nextSteps: [
        `Switch your wallet and RPC configuration to '${expectedNetwork}'.`,
        "Ensure the dashboard RPC URL points to Soroban Testnet (https://soroban-testnet.stellar.org).",
        "Reconnect your wallet and re-run diagnostics to verify network alignment.",
      ],
    };
  }

  if (!isValidContractId(contractId)) {
    return {
      status: "missing-contract",
      rpcUrl: safeRpcUrl,
      rpcReachable: true,
      configuredNetworkPassphrase: expectedNetwork,
      detectedNetworkPassphrase: detectedNetwork ?? expectedNetwork,
      walletNetworkPassphrase: walletNetwork,
      networkMatches: true,
      contractId,
      contractReadable: false,
      contractDetail:
        "Configured contract ID is empty or not a valid Stellar contract StrKey (C...).",
      latestLedger: rpcResult.latestLedger ?? null,
      latestLedgerCloseTime: rpcResult.latestLedgerCloseTime ?? null,
      protocolVersion: rpcResult.protocolVersion ?? null,
      checkedAtMs,
      summary: "Contract deployment is missing or has an invalid contract ID.",
      nextSteps: [
        "Set a valid 56-character Stellar contract address starting with 'C' in the client configuration.",
        "Deploy the escrow WASM to Soroban Testnet (`stellar contract deploy`) and regenerate or update the client contract ID.",
      ],
    };
  }

  const probeContract = options.probeContract;
  const contractResult: ContractProbeResult = probeContract
    ? await probeContract(contractId, options.rpcUrl, expectedNetwork)
    : {
        readable: true,
        detail: "Contract ID format verified against reachable Testnet RPC.",
      };

  if (!contractResult.readable) {
    const safeDetail = sanitizeDiagnosticText(
      contractResult.detail ??
        "Contract instance could not be read on the configured network."
    );
    const status: DiagnosticsHealthStatus =
      contractResult.errorCode ?? "missing-contract";

    if (status === "unavailable-rpc") {
      return {
        status: "unavailable-rpc",
        rpcUrl: safeRpcUrl,
        rpcReachable: false,
        configuredNetworkPassphrase: expectedNetwork,
        detectedNetworkPassphrase: detectedNetwork ?? expectedNetwork,
        walletNetworkPassphrase: walletNetwork,
        networkMatches: true,
        contractId,
        contractReadable: false,
        contractDetail: safeDetail,
        latestLedger: rpcResult.latestLedger ?? null,
        latestLedgerCloseTime: rpcResult.latestLedgerCloseTime ?? null,
        protocolVersion: rpcResult.protocolVersion ?? null,
        checkedAtMs,
        summary: `RPC failed during contract read probe (${safeDetail}).`,
        nextSteps: [
          `Verify ${safeRpcUrl} supports Soroban simulateTransaction requests.`,
          "Retry diagnostics once the RPC node finishes syncing or rate-limiting clears.",
        ],
      };
    }

    if (status === "wrong-network") {
      return {
        status: "wrong-network",
        rpcUrl: safeRpcUrl,
        rpcReachable: true,
        configuredNetworkPassphrase: expectedNetwork,
        detectedNetworkPassphrase: detectedNetwork ?? expectedNetwork,
        walletNetworkPassphrase: walletNetwork,
        networkMatches: false,
        contractId,
        contractReadable: false,
        contractDetail: safeDetail,
        latestLedger: rpcResult.latestLedger ?? null,
        latestLedgerCloseTime: rpcResult.latestLedgerCloseTime ?? null,
        protocolVersion: rpcResult.protocolVersion ?? null,
        checkedAtMs,
        summary: `Network mismatch during contract read (${safeDetail}).`,
        nextSteps: [
          `Confirm both RPC and wallet use '${expectedNetwork}'.`,
          "Reconnect your wallet and re-run diagnostics.",
        ],
      };
    }

    return {
      status: "missing-contract",
      rpcUrl: safeRpcUrl,
      rpcReachable: true,
      configuredNetworkPassphrase: expectedNetwork,
      detectedNetworkPassphrase: detectedNetwork ?? expectedNetwork,
      walletNetworkPassphrase: walletNetwork,
      networkMatches: true,
      contractId,
      contractReadable: false,
      contractDetail: safeDetail,
      latestLedger: rpcResult.latestLedger ?? null,
      latestLedgerCloseTime: rpcResult.latestLedgerCloseTime ?? null,
      protocolVersion: rpcResult.protocolVersion ?? null,
      checkedAtMs,
      summary: `Contract '${contractId.slice(0, 6)}…${contractId.slice(-4)}' could not be read on Testnet.`,
      nextSteps: [
        `Verify contract ${contractId} is deployed on '${expectedNetwork}'.`,
        "If the contract instance or WASM entry archived on Testnet, restore it with `stellar contract restore` or redeploy the contract.",
        "Re-run diagnostics after restoring or updating the contract address.",
      ],
    };
  }

  return {
    status: "healthy",
    rpcUrl: safeRpcUrl,
    rpcReachable: true,
    configuredNetworkPassphrase: expectedNetwork,
    detectedNetworkPassphrase: detectedNetwork ?? expectedNetwork,
    walletNetworkPassphrase: walletNetwork,
    networkMatches: true,
    contractId,
    contractReadable: true,
    contractDetail: sanitizeDiagnosticText(
      contractResult.detail ?? "Contract is deployed and readable."
    ),
    latestLedger: rpcResult.latestLedger ?? null,
    latestLedgerCloseTime: rpcResult.latestLedgerCloseTime ?? null,
    protocolVersion: rpcResult.protocolVersion ?? null,
    checkedAtMs,
    summary:
      rpcResult.latestLedger !== null && rpcResult.latestLedger !== undefined
        ? `Healthy — Testnet RPC online (latest ledger #${rpcResult.latestLedger}) and escrow contract readable.`
        : "Healthy — Testnet RPC online and escrow contract readable.",
    nextSteps: [],
  };
}

export function resolveDisplayDiagnosticsState(
  report: DiagnosticsReport | null,
  isLoading: boolean,
  nowMs: number = Date.now(),
  staleThresholdMs: number = DEFAULT_STALE_THRESHOLD_MS
): DisplayDiagnosticsState {
  if (isLoading) {
    return {
      displayState: "loading",
      isStale: false,
      ageMs: report ? Math.max(0, nowMs - report.checkedAtMs) : null,
    };
  }

  if (!report) {
    return {
      displayState: "idle",
      isStale: false,
      ageMs: null,
    };
  }

  const ageMs = Math.max(0, nowMs - report.checkedAtMs);
  const isStale = ageMs > staleThresholdMs;

  if (report.status === "healthy" && isStale) {
    return {
      displayState: "stale",
      isStale: true,
      ageMs,
    };
  }

  return {
    displayState: report.status,
    isStale,
    ageMs,
  };
}
