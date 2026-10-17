export const PENDING_RECEIPT_STORAGE_KEY =
  "soroban-forge:pending-receipt:v1" as const;
export const RECEIPT_SCHEMA_VERSION =
  "soroban-forge/pending-receipt@1" as const;

export const DEFAULT_RECEIPT_TTL_MS = 10 * 60 * 1000; // 10 minutes
export const DEFAULT_MAX_POLL_ATTEMPTS = 6;
export const DEFAULT_POLL_INTERVAL_MS = 2_000;

export type ReceiptOutcomeStatus =
  | "pending"
  | "confirmed"
  | "failed"
  | "expired"
  | "unknown"
  | "mismatch";

export type ReceiptValidationErrorCode =
  | "MALFORMED_RECEIPT"
  | "VERSION_MISMATCH"
  | "SENSITIVE_DATA_REJECTED"
  | "INVALID_RECEIPT_FIELD";

export interface BrowserStorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface PersistedReceiptRecord {
  schemaVersion: typeof RECEIPT_SCHEMA_VERSION;
  txHash: string;
  networkPassphrase: string;
  contractId: string;
  account: string | null;
  action: string;
  escrowId: string | null;
  submittedAtMs: number;
  lastOutcome: ReceiptOutcomeStatus;
  lastDetail: string;
  ledger: number | null;
}

export interface SavePendingReceiptInput {
  txHash: string;
  networkPassphrase: string;
  contractId: string;
  account?: string | null;
  action: string;
  escrowId?: string | null;
  submittedAtMs?: number;
  lastOutcome?: ReceiptOutcomeStatus;
  lastDetail?: string;
  ledger?: number | null;
}

export interface RpcTransactionStatusResponse {
  status: "SUCCESS" | "FAILED" | "NOT_FOUND" | string;
  ledger?: number | null;
  errorDetail?: string;
}

export type RpcGetTransactionFn = (
  txHash: string,
  rpcUrl: string
) => Promise<RpcTransactionStatusResponse>;

export interface ReceiptContextCheckInput {
  record: PersistedReceiptRecord;
  expectedNetworkPassphrase: string;
  expectedContractId?: string;
  connectedAccount?: string | null;
}

export interface ReceiptContextCheckResult {
  matches: boolean;
  mismatchKind?: "network" | "contract" | "account";
  explanation?: string;
}

export interface BoundedReceiptPollOptions {
  record: PersistedReceiptRecord;
  rpcUrl: string;
  expectedNetworkPassphrase: string;
  expectedContractId?: string;
  connectedAccount?: string | null;
  maxAttempts?: number;
  pollIntervalMs?: number;
  ttlMs?: number;
  nowFn?: () => number;
  sleepFn?: (ms: number) => Promise<void>;
  getTransaction?: RpcGetTransactionFn;
  storage?: BrowserStorageLike | null;
  onAttempt?: (attempt: number, maxAttempts: number) => void;
}

export interface BoundedReceiptPollResult {
  outcome: ReceiptOutcomeStatus;
  record: PersistedReceiptRecord;
  attempts: number;
  explanation: string;
}

export class ReceiptValidationError extends Error {
  readonly code: ReceiptValidationErrorCode;

  constructor(code: ReceiptValidationErrorCode, message: string) {
    super(`[${code}] ${message}`);
    this.name = "ReceiptValidationError";
    this.code = code;
  }
}

const TX_HASH_HEX_REGEX = /^[0-9a-fA-F]{64}$/;
const STELLAR_SECRET_SEED_REGEX = /\bS[A-Z2-7]{55}\b/;
const FORBIDDEN_SECRET_KEYS = new Set([
  "secret",
  "secretKey",
  "secret_key",
  "privateKey",
  "private_key",
  "seed",
  "mnemonic",
  "signedTxXdr",
  "signed_tx_xdr",
  "signedEnvelope",
  "envelopeXdr",
]);

const VALID_OUTCOMES = new Set<ReceiptOutcomeStatus>([
  "pending",
  "confirmed",
  "failed",
  "expired",
  "unknown",
  "mismatch",
]);

function assertNoSecretsOrSignedPayloads(value: unknown, path = "root"): void {
  if (typeof value === "string") {
    if (STELLAR_SECRET_SEED_REGEX.test(value)) {
      throw new ReceiptValidationError(
        "SENSITIVE_DATA_REJECTED",
        `Receipt record at '${path}' contains a Stellar secret seed. Secrets cannot be persisted.`
      );
    }
    return;
  }

  if (Array.isArray(value)) {
    value.forEach((item, idx) =>
      assertNoSecretsOrSignedPayloads(item, `${path}[${idx}]`)
    );
    return;
  }

  if (value && typeof value === "object") {
    for (const [key, nested] of Object.entries(
      value as Record<string, unknown>
    )) {
      if (FORBIDDEN_SECRET_KEYS.has(key)) {
        throw new ReceiptValidationError(
          "SENSITIVE_DATA_REJECTED",
          `Field '${path}.${key}' is forbidden; secrets and signed transaction payloads must never be persisted.`
        );
      }
      assertNoSecretsOrSignedPayloads(nested, `${path}.${key}`);
    }
  }
}

export function validatePersistedReceiptRecord(
  raw: unknown
): PersistedReceiptRecord {
  let parsed: unknown = raw;
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (!trimmed) {
      throw new ReceiptValidationError(
        "MALFORMED_RECEIPT",
        "Persisted receipt JSON cannot be empty."
      );
    }
    try {
      parsed = JSON.parse(trimmed);
    } catch (err) {
      throw new ReceiptValidationError(
        "MALFORMED_RECEIPT",
        `Persisted receipt is not valid JSON (${err instanceof Error ? err.message : String(err)}).`
      );
    }
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new ReceiptValidationError(
      "MALFORMED_RECEIPT",
      "Persisted receipt must be a JSON object."
    );
  }

  assertNoSecretsOrSignedPayloads(parsed);

  const obj = parsed as Record<string, unknown>;
  if (obj.schemaVersion !== RECEIPT_SCHEMA_VERSION) {
    throw new ReceiptValidationError(
      "VERSION_MISMATCH",
      `Unsupported receipt schemaVersion '${String(obj.schemaVersion)}'. Expected '${RECEIPT_SCHEMA_VERSION}'.`
    );
  }

  const txHash = typeof obj.txHash === "string" ? obj.txHash.trim() : "";
  if (!TX_HASH_HEX_REGEX.test(txHash)) {
    throw new ReceiptValidationError(
      "INVALID_RECEIPT_FIELD",
      "Receipt `txHash` must be a 64-character hexadecimal transaction hash."
    );
  }

  const networkPassphrase =
    typeof obj.networkPassphrase === "string"
      ? obj.networkPassphrase.trim()
      : "";
  if (!networkPassphrase) {
    throw new ReceiptValidationError(
      "INVALID_RECEIPT_FIELD",
      "Receipt `networkPassphrase` must be a non-empty string."
    );
  }

  const contractId =
    typeof obj.contractId === "string" ? obj.contractId.trim() : "";
  if (!contractId) {
    throw new ReceiptValidationError(
      "INVALID_RECEIPT_FIELD",
      "Receipt `contractId` must be a non-empty string."
    );
  }

  const action = typeof obj.action === "string" ? obj.action.trim() : "";
  if (!action) {
    throw new ReceiptValidationError(
      "INVALID_RECEIPT_FIELD",
      "Receipt `action` must be a non-empty string."
    );
  }

  if (
    typeof obj.submittedAtMs !== "number" ||
    !Number.isFinite(obj.submittedAtMs) ||
    obj.submittedAtMs <= 0
  ) {
    throw new ReceiptValidationError(
      "INVALID_RECEIPT_FIELD",
      "Receipt `submittedAtMs` must be a positive finite timestamp."
    );
  }

  const account =
    obj.account === null || obj.account === undefined
      ? null
      : typeof obj.account === "string" && obj.account.trim()
        ? obj.account.trim()
        : null;

  const escrowId =
    obj.escrowId === null || obj.escrowId === undefined
      ? null
      : typeof obj.escrowId === "string" && obj.escrowId.trim()
        ? obj.escrowId.trim()
        : null;

  const rawOutcome = (obj.lastOutcome ?? "pending") as ReceiptOutcomeStatus;
  if (!VALID_OUTCOMES.has(rawOutcome)) {
    throw new ReceiptValidationError(
      "INVALID_RECEIPT_FIELD",
      `Unsupported receipt outcome '${String(rawOutcome)}'.`
    );
  }

  const lastDetail =
    typeof obj.lastDetail === "string"
      ? obj.lastDetail
      : "Submitted transaction awaiting confirmation.";

  const ledger =
    typeof obj.ledger === "number" && Number.isFinite(obj.ledger)
      ? obj.ledger
      : null;

  return {
    schemaVersion: RECEIPT_SCHEMA_VERSION,
    txHash: txHash.toLowerCase(),
    networkPassphrase,
    contractId,
    account,
    action,
    escrowId,
    submittedAtMs: Math.trunc(obj.submittedAtMs),
    lastOutcome: rawOutcome,
    lastDetail,
    ledger,
  };
}

export function savePendingReceipt(
  input: SavePendingReceiptInput,
  storage?: BrowserStorageLike | null
): PersistedReceiptRecord {
  assertNoSecretsOrSignedPayloads(input);

  const record = validatePersistedReceiptRecord({
    schemaVersion: RECEIPT_SCHEMA_VERSION,
    txHash: input.txHash,
    networkPassphrase: input.networkPassphrase,
    contractId: input.contractId,
    account: input.account ?? null,
    action: input.action,
    escrowId: input.escrowId ?? null,
    submittedAtMs: input.submittedAtMs ?? Date.now(),
    lastOutcome: input.lastOutcome ?? "pending",
    lastDetail:
      input.lastDetail ??
      `Submitted '${input.action}' transaction; awaiting ledger confirmation.`,
    ledger: input.ledger ?? null,
  });

  if (storage) {
    storage.setItem(PENDING_RECEIPT_STORAGE_KEY, JSON.stringify(record));
  }
  return record;
}

export function loadPendingReceipt(storage?: BrowserStorageLike | null): {
  record: PersistedReceiptRecord | null;
  error?: ReceiptValidationError;
} {
  if (!storage) {
    return { record: null };
  }

  const raw = storage.getItem(PENDING_RECEIPT_STORAGE_KEY);
  if (raw === null || !raw.trim()) {
    return { record: null };
  }

  try {
    const record = validatePersistedReceiptRecord(raw);
    return { record };
  } catch (err) {
    const validationErr =
      err instanceof ReceiptValidationError
        ? err
        : new ReceiptValidationError(
            "MALFORMED_RECEIPT",
            err instanceof Error ? err.message : String(err)
          );
    return { record: null, error: validationErr };
  }
}

export function dismissPendingReceipt(
  storage?: BrowserStorageLike | null
): void {
  if (storage) {
    storage.removeItem(PENDING_RECEIPT_STORAGE_KEY);
  }
}

export function checkReceiptContextMatch(
  input: ReceiptContextCheckInput
): ReceiptContextCheckResult {
  const { record, expectedNetworkPassphrase, expectedContractId, connectedAccount } =
    input;

  if (record.networkPassphrase !== expectedNetworkPassphrase.trim()) {
    return {
      matches: false,
      mismatchKind: "network",
      explanation: `Network mismatch: persisted receipt was submitted on '${record.networkPassphrase}', but the dashboard is configured for '${expectedNetworkPassphrase}'. Recovery polling is paused to prevent unsafe cross-network state assumptions.`,
    };
  }

  if (
    expectedContractId &&
    record.contractId !== expectedContractId.trim()
  ) {
    return {
      matches: false,
      mismatchKind: "contract",
      explanation: `Contract mismatch: persisted receipt targets contract '${record.contractId}', but the dashboard is configured for '${expectedContractId}'.`,
    };
  }

  if (
    record.account &&
    connectedAccount &&
    record.account !== connectedAccount.trim()
  ) {
    return {
      matches: false,
      mismatchKind: "account",
      explanation: `Account mismatch: persisted '${record.action}' receipt was submitted by ${record.account.slice(0, 6)}…${record.account.slice(-4)}, but wallet ${connectedAccount.slice(0, 6)}…${connectedAccount.slice(-4)} is currently connected. Switch accounts or dismiss this receipt before continuing.`,
    };
  }

  return { matches: true };
}

export function createRpcGetTransactionPoller(
  fetchImpl: typeof fetch = fetch
): RpcGetTransactionFn {
  return async (
    txHash: string,
    rpcUrl: string
  ): Promise<RpcTransactionStatusResponse> => {
    const res = await fetchImpl(rpcUrl.trim(), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: `getTransaction-${txHash.slice(0, 8)}`,
        method: "getTransaction",
        params: { hash: txHash },
      }),
    });

    if (!res.ok) {
      throw new Error(`HTTP ${res.status} from Soroban RPC getTransaction`);
    }

    const payload = (await res.json()) as {
      result?: {
        status?: string;
        ledger?: number;
        resultXdr?: string;
      };
      error?: { message?: string };
    };

    if (payload.error) {
      throw new Error(
        payload.error.message ?? "RPC error during getTransaction"
      );
    }

    const status = payload.result?.status ?? "NOT_FOUND";
    const ledger =
      typeof payload.result?.ledger === "number" ? payload.result.ledger : null;

    return {
      status,
      ledger,
    };
  };
}

function defaultSleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function pollReceiptOutcome(
  options: BoundedReceiptPollOptions
): Promise<BoundedReceiptPollResult> {
  const maxAttempts = Math.max(
    1,
    options.maxAttempts ?? DEFAULT_MAX_POLL_ATTEMPTS
  );
  const pollIntervalMs = Math.max(
    0,
    options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS
  );
  const ttlMs = options.ttlMs ?? DEFAULT_RECEIPT_TTL_MS;
  const nowFn = options.nowFn ?? (() => Date.now());
  const sleepFn = options.sleepFn ?? defaultSleep;
  const getTx = options.getTransaction ?? createRpcGetTransactionPoller();

  const persistUpdate = (
    outcome: ReceiptOutcomeStatus,
    detail: string,
    ledger: number | null = options.record.ledger
  ): PersistedReceiptRecord => {
    return savePendingReceipt(
      {
        ...options.record,
        lastOutcome: outcome,
        lastDetail: detail,
        ledger,
      },
      options.storage
    );
  };

  // 1. Check context match (network, contract, account)
  const contextCheck = checkReceiptContextMatch({
    record: options.record,
    expectedNetworkPassphrase: options.expectedNetworkPassphrase,
    expectedContractId: options.expectedContractId,
    connectedAccount: options.connectedAccount,
  });

  if (!contextCheck.matches) {
    const explanation =
      contextCheck.explanation ??
      "Persisted receipt does not match the current network or account context.";
    const updated = persistUpdate("mismatch", explanation);
    return {
      outcome: "mismatch",
      record: updated,
      attempts: 0,
      explanation,
    };
  }

  let attempts = 0;
  let lastError: string | null = null;

  while (attempts < maxAttempts) {
    const elapsedMs = nowFn() - options.record.submittedAtMs;
    if (elapsedMs > ttlMs) {
      const explanation = `Transaction ${options.record.txHash.slice(0, 10)}… (${options.record.action}) expired after ${Math.round(elapsedMs / 1000)}s without ledger confirmation. You can retry polling or dismiss this receipt.`;
      const updated = persistUpdate("expired", explanation);
      return {
        outcome: "expired",
        record: updated,
        attempts,
        explanation,
      };
    }

    attempts += 1;
    options.onAttempt?.(attempts, maxAttempts);

    try {
      const res = await getTx(options.record.txHash, options.rpcUrl);
      const status = res.status.toUpperCase();

      if (status === "SUCCESS") {
        const ledgerSuffix = res.ledger ? ` in ledger #${res.ledger}` : "";
        const explanation = `Transaction ${options.record.txHash.slice(0, 10)}… (${options.record.action}) confirmed on-chain${ledgerSuffix}.`;
        const updated = persistUpdate(
          "confirmed",
          explanation,
          res.ledger ?? null
        );
        return {
          outcome: "confirmed",
          record: updated,
          attempts,
          explanation,
        };
      }

      if (status === "FAILED") {
        const detailSuffix = res.errorDetail ? ` (${res.errorDetail})` : "";
        const explanation = `Transaction ${options.record.txHash.slice(0, 10)}… (${options.record.action}) failed on-chain${detailSuffix}.`;
        const updated = persistUpdate(
          "failed",
          explanation,
          res.ledger ?? null
        );
        return {
          outcome: "failed",
          record: updated,
          attempts,
          explanation,
        };
      }

      // NOT_FOUND or other transient state: continue bounded polling
      lastError = null;
    } catch (err) {
      lastError = err instanceof Error ? err.message : String(err);
    }

    if (attempts < maxAttempts && pollIntervalMs > 0) {
      await sleepFn(pollIntervalMs);
    }
  }

  // Re-check expiration after final attempt
  const finalElapsedMs = nowFn() - options.record.submittedAtMs;
  if (finalElapsedMs > ttlMs) {
    const explanation = `Transaction ${options.record.txHash.slice(0, 10)}… (${options.record.action}) expired without confirmation.`;
    const updated = persistUpdate("expired", explanation);
    return {
      outcome: "expired",
      record: updated,
      attempts,
      explanation,
    };
  }

  const unknownReason = lastError
    ? `RPC error after ${attempts} attempts: ${lastError}`
    : `Status remained NOT_FOUND after ${attempts} bounded polling attempts`;
  const explanation = `Transaction ${options.record.txHash.slice(0, 10)}… (${options.record.action}) outcome is currently unknown (${unknownReason}). Click 'Retry receipt check' or dismiss.`;
  const updated = persistUpdate("unknown", explanation);

  return {
    outcome: "unknown",
    record: updated,
    attempts,
    explanation,
  };
}

export function createMemoryReceiptStorage(
  initial: Record<string, string> = {}
): BrowserStorageLike & { snapshot(): Record<string, string> } {
  const map = new Map<string, string>(Object.entries(initial));
  return {
    getItem(key: string): string | null {
      return map.has(key) ? (map.get(key) ?? null) : null;
    },
    setItem(key: string, value: string): void {
      map.set(key, value);
    },
    removeItem(key: string): void {
      map.delete(key);
    },
    snapshot(): Record<string, string> {
      return Object.fromEntries(map.entries());
    },
  };
}
