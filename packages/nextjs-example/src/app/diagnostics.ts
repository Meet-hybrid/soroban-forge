export type DiagnosticsCheckStatus = "pass" | "fail" | "skip";

export type DiagnosticsFailureKind =
  | "wrong-network"
  | "rpc-unavailable"
  | "missing-contract"
  | "contract-read-error";

export type DiagnosticsHealth = "loading" | "healthy" | "stale" | "error";

export interface DiagnosticsConfig {
  networkName: string;
  networkPassphrase: string;
  rpcUrl: string;
  contractId: string;
}

export interface DiagnosticsCheck {
  status: DiagnosticsCheckStatus;
  detail: string;
  nextStep?: string;
  failureKind?: DiagnosticsFailureKind;
}

export interface DiagnosticsSnapshot {
  network: DiagnosticsCheck;
  rpc: DiagnosticsCheck;
  contract: DiagnosticsCheck;
  latestLedger: number | null;
  checkedAt: number;
  outcome: "healthy" | "error";
  nextSteps: string[];
}

export interface DiagnosticsProbes {
  getLatestLedger?: () => Promise<unknown>;
  probeContract?: () => Promise<unknown>;
  getWalletNetwork?: () => Promise<{
    network: string;
    networkPassphrase: string;
  } | null>;
}

export interface DiagnosticsState {
  phase: "idle" | "loading" | "ready";
  snapshot: DiagnosticsSnapshot | null;
  error: string | null;
}

export type DiagnosticsAction =
  | { type: "check-started" }
  | { type: "check-finished"; snapshot: DiagnosticsSnapshot }
  | { type: "check-failed"; message: string; at: number };

export const DIAGNOSTICS_STALE_MS = 60_000;
export const DIAGNOSTICS_AUTO_REFRESH_MS = 120_000;
export const DIAGNOSTICS_RECHECK_THROTTLE_MS = 10_000;

export const initialDiagnosticsState: DiagnosticsState = {
  phase: "idle",
  snapshot: null,
  error: null,
};

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return "";
}

export function sanitizeDetail(value: string): string {
  const withoutCredentials = value.replace(
    /https?:\/\/[^\s/@]+:[^\s/@]+@[^\s]+/gi,
    "[redacted endpoint]",
  );
  const withoutQueryValues = withoutCredentials.replace(
    /([?&][^=\s"'<>]+=)[^\s"'<>&]+/g,
    "$1[redacted]",
  );
  const withoutAuthSchemes = withoutQueryValues.replace(
    /\b(bearer|basic)\s+[^\s,;]+/gi,
    "$1 [redacted]",
  );
  const withoutSecrets = withoutAuthSchemes.replace(
    /\b(secret|private[_-]?key|api[_-]?key|token|password|authorization|bearer)\b(\s*[:=]\s*)([^\s,;]+)/gi,
    "$1$2[redacted]",
  );
  const compact = withoutSecrets.replace(/\s+/g, " ").trim();
  return compact.length > 240 ? `${compact.slice(0, 237)}…` : compact;
}

export function safeEndpoint(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname.replace(/\/[A-Za-z0-9_-]{20,}/g, "/[redacted]");
    return `${parsed.protocol}//${parsed.host}${path === "/" ? "" : path}`;
  } catch {
    return "[invalid endpoint]";
  }
}

export function shortIdentifier(id: string): string {
  const compact = id.trim();
  return compact.length > 16
    ? `${compact.slice(0, 8)}…${compact.slice(-6)}`
    : compact || "[unset]";
}

export function classifyDiagnosticsError(
  error: unknown,
  source: "wallet" | "rpc" | "contract",
): DiagnosticsFailureKind {
  const message = errorMessage(error).toLowerCase();
  const mentionsNetwork =
    /network|passphrase|chain\s*id|testnet|mainnet|pubnet/.test(message);
  const mentionsMismatch =
    /mismatch|does not match|different|wrong|expected|switch|unsupported|not supported|incorrect/.test(
      message,
    );
  if (mentionsNetwork && mentionsMismatch) {
    return "wrong-network";
  }

  const mentionsMissingContract =
    /(contract|account|host)[^;]{0,80}(not found|does not exist|not deployed|undeployed|not initialized)|(not found|does not exist|not deployed|undeployed)[^;]{0,80}contract|contractnotfound|hosterror:[^;]*not found/.test(
      message,
    );
  if (mentionsMissingContract) {
    return "missing-contract";
  }

  const mentionsRpcFailure =
    /fetch failed|failed to fetch|networkerror|econnrefused|enotfound|ehostunreach|etimedout|timeout|timed out|unavailable|temporarily|bad gateway|service unavailable|socket|connection|aborted|getaddrinfo|ssl|tls|\b50[0234]\b/.test(
      message,
    );
  if (source === "rpc" || mentionsRpcFailure) {
    return "rpc-unavailable";
  }

  return "contract-read-error";
}

export function nextStepForFailure(
  kind: DiagnosticsFailureKind,
  config: DiagnosticsConfig,
): string {
  switch (kind) {
    case "wrong-network":
      return `Switch the connected wallet to ${config.networkName} and reconnect before signing transactions.`;
    case "rpc-unavailable":
      return `Check the RPC endpoint ${safeEndpoint(config.rpcUrl)} and your internet connection, then recheck.`;
    case "missing-contract":
      return `Deploy the escrow contract on Testnet and update the configured contract id in the SDK network config (currently ${shortIdentifier(config.contractId)}).`;
    case "contract-read-error":
      return `Retry the read; if it keeps failing, verify the deployment for contract ${shortIdentifier(config.contractId)}.`;
  }
}

function ledgerSequence(response: unknown): number | null {
  if (typeof response === "number" && Number.isFinite(response)) {
    return response;
  }
  if (typeof response === "string" && /^\d+$/.test(response.trim())) {
    return Number(response);
  }
  if (typeof response === "bigint") {
    return Number(response);
  }
  if (typeof response === "object" && response !== null) {
    const record = response as Record<string, unknown>;
    const candidate = record.sequence ?? record.latestLedger ?? record.ledger;
    if (typeof candidate === "number" && Number.isFinite(candidate)) {
      return candidate;
    }
    if (typeof candidate === "string" && /^\d+$/.test(candidate.trim())) {
      return Number(candidate);
    }
    if (typeof candidate === "bigint") {
      return Number(candidate);
    }
  }
  return null;
}

function failedCheck(
  prefix: string,
  error: unknown,
  source: "wallet" | "rpc" | "contract",
  config: DiagnosticsConfig,
): DiagnosticsCheck {
  const kind = classifyDiagnosticsError(error, source);
  const detail = sanitizeDetail(errorMessage(error)) || "The request failed.";
  return {
    status: "fail",
    detail: `${prefix}: ${detail}`,
    failureKind: kind,
    nextStep: nextStepForFailure(kind, config),
  };
}

export async function runDiagnostics(
  config: DiagnosticsConfig,
  probes: DiagnosticsProbes,
  now: number = Date.now(),
): Promise<DiagnosticsSnapshot> {
  let network: DiagnosticsCheck;
  if (!probes.getWalletNetwork) {
    network = {
      status: "skip",
      detail: `No wallet probe; requests use configured ${config.networkName}.`,
    };
  } else {
    try {
      const walletNetwork = await probes.getWalletNetwork();
      if (walletNetwork === null) {
        network = {
          status: "skip",
          detail: `No wallet network reported; requests use configured ${config.networkName}.`,
        };
      } else if (walletNetwork.networkPassphrase !== config.networkPassphrase) {
        const kind: DiagnosticsFailureKind = "wrong-network";
        network = {
          status: "fail",
          detail: `Wallet is set to ${walletNetwork.network || "another network"} (${walletNetwork.networkPassphrase || "unknown passphrase"}); dashboard expects ${config.networkName}.`,
          failureKind: kind,
          nextStep: nextStepForFailure(kind, config),
        };
      } else {
        network = {
          status: "pass",
          detail: `Wallet and dashboard both use ${config.networkName}.`,
        };
      }
    } catch (error) {
      network = failedCheck("Wallet network check failed", error, "wallet", config);
    }
  }

  let rpc: DiagnosticsCheck;
  let latestLedger: number | null = null;
  if (!probes.getLatestLedger) {
    rpc = { status: "skip", detail: "No RPC ledger probe is configured." };
  } else {
    try {
      const response = await probes.getLatestLedger();
      latestLedger = ledgerSequence(response);
      rpc = {
        status: "pass",
        detail:
          latestLedger === null
            ? `RPC reachable at ${safeEndpoint(config.rpcUrl)}; ledger number unavailable.`
            : `RPC reachable at ${safeEndpoint(config.rpcUrl)} · latest ledger #${latestLedger}`,
      };
    } catch (error) {
      rpc = failedCheck("RPC request failed", error, "rpc", config);
    }
  }

  let contract: DiagnosticsCheck;
  if (!probes.probeContract) {
    contract = {
      status: "skip",
      detail: "Connect a wallet to run the read-only contract probe.",
    };
  } else {
    try {
      await probes.probeContract();
      contract = {
        status: "pass",
        detail: `Contract ${shortIdentifier(config.contractId)} responded to a read-only call.`,
      };
    } catch (error) {
      contract = failedCheck("Contract read failed", error, "contract", config);
    }
  }

  const checks = [network, rpc, contract];
  const nextSteps = checks
    .map((check) => check.nextStep)
    .filter((step): step is string => Boolean(step));

  return {
    network,
    rpc,
    contract,
    latestLedger,
    checkedAt: now,
    outcome: checks.some((check) => check.status === "fail") ? "error" : "healthy",
    nextSteps: [...new Set(nextSteps)],
  };
}

export function diagnosticsHealth(
  snapshot: DiagnosticsSnapshot | null,
  now: number = Date.now(),
): DiagnosticsHealth {
  if (snapshot === null) {
    return "loading";
  }
  if (snapshot.outcome === "error") {
    return "error";
  }
  return now - snapshot.checkedAt > DIAGNOSTICS_STALE_MS ? "stale" : "healthy";
}

export function diagnosticsReducer(
  state: DiagnosticsState,
  action: DiagnosticsAction,
): DiagnosticsState {
  switch (action.type) {
    case "check-started":
      return { phase: "loading", snapshot: state.snapshot, error: null };
    case "check-finished":
      return { phase: "ready", snapshot: action.snapshot, error: null };
    case "check-failed":
      return {
        phase: "ready",
        snapshot: null,
        error: sanitizeDetail(action.message) || "Diagnostics could not run.",
      };
  }
}
