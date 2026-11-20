export const RECEIPT_STORAGE_KEY = "soroban-forge.pending-receipts";

/** Maximum number of status polls before a still-unknown receipt expires. */
export const MAX_RECEIPT_ATTEMPTS = 6;

/** Delay between bounded status polls. */
export const RECEIPT_POLL_INTERVAL_MS = 2_000;

/** A receipt older than this is considered stale even if polls remain. */
export const RECEIPT_TTL_MS = 10 * 60 * 1000;

/** Maximum receipts kept in local storage. */
export const MAX_STORED_RECEIPTS = 10;

export type ReceiptStatus = "pending" | "confirmed" | "failed" | "expired" | "unknown";

export type ReceiptNetwork = "testnet";

export interface PendingReceipt {
  hash: string;
  network: ReceiptNetwork;
  account: string;
  action: string;
  timestamp: number;
  status: ReceiptStatus;
  attempts: number;
}

export interface ReceiptStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const HASH_PATTERN = /^([a-fA-F0-9]{64}|demo-[a-zA-Z0-9-]+|demo-tx-[a-zA-Z0-9-]+)$/;
const STATUSES: readonly ReceiptStatus[] = [
  "pending",
  "confirmed",
  "failed",
  "expired",
  "unknown",
];

export function validateReceipt(value: unknown): PendingReceipt | null {
  if (!value || typeof value !== "object") return null;
  const receipt = value as Record<string, unknown>;
  if (typeof receipt.hash !== "string" || !HASH_PATTERN.test(receipt.hash)) return null;
  if (receipt.network !== "testnet") return null;
  if (typeof receipt.account !== "string" || typeof receipt.action !== "string") return null;
  if (typeof receipt.timestamp !== "number" || !Number.isFinite(receipt.timestamp)) return null;
  if (!STATUSES.includes(receipt.status as ReceiptStatus)) return null;
  if (
    typeof receipt.attempts !== "number" ||
    !Number.isInteger(receipt.attempts) ||
    receipt.attempts < 0
  ) {
    return null;
  }
  return {
    hash: receipt.hash,
    network: "testnet",
    account: receipt.account,
    action: receipt.action,
    timestamp: receipt.timestamp,
    status: receipt.status as ReceiptStatus,
    attempts: receipt.attempts,
  };
}

export function createPendingReceipt(
  input: { hash: string; account: string; action: string },
  now: number = Date.now(),
): PendingReceipt {
  return {
    hash: input.hash,
    network: "testnet",
    account: input.account,
    action: input.action,
    timestamp: now,
    status: "pending",
    attempts: 0,
  };
}

/**
 * Map a Stellar RPC `getTransaction` status onto a receipt outcome.
 * `NOT_FOUND` (or a failed lookup) means "not observed yet": keep polling.
 */
export function classifyTransactionStatus(status: string | null | undefined): ReceiptStatus {
  switch (status) {
    case "SUCCESS":
      return "confirmed";
    case "FAILED":
      return "failed";
    case "NOT_FOUND":
    case null:
    case undefined:
      return "pending";
    default:
      return "unknown";
  }
}

export function isExpired(receipt: PendingReceipt, now: number = Date.now()): boolean {
  return receipt.status === "pending" && now - receipt.timestamp > RECEIPT_TTL_MS;
}

/**
 * Apply one bounded poll result. Unknown RPC statuses settle immediately;
 * still-unobserved transactions keep polling until the attempt or age bound
 * is reached, then expire.
 */
export function recordPollAttempt(
  receipt: PendingReceipt,
  status: string | null | undefined,
  now: number = Date.now(),
): PendingReceipt {
  const attempts = receipt.attempts + 1;
  const outcome = classifyTransactionStatus(status);
  if (outcome !== "pending") {
    return { ...receipt, attempts, status: outcome };
  }
  if (attempts >= MAX_RECEIPT_ATTEMPTS || now - receipt.timestamp > RECEIPT_TTL_MS) {
    return { ...receipt, attempts, status: "expired" };
  }
  return { ...receipt, attempts, status: "pending" };
}

export function loadReceipts(storage: ReceiptStorage): PendingReceipt[] {
  try {
    const raw = storage.getItem(RECEIPT_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map(validateReceipt)
      .filter((receipt): receipt is PendingReceipt => receipt !== null)
      .slice(0, MAX_STORED_RECEIPTS);
  } catch {
    return [];
  }
}

/** Reload view: validate stored receipts and expire ones that aged out. */
export function restoreReceipts(
  storage: ReceiptStorage,
  now: number = Date.now(),
): PendingReceipt[] {
  const receipts = loadReceipts(storage);
  const restored = receipts.map((receipt) =>
    isExpired(receipt, now) ? { ...receipt, status: "expired" as const } : receipt,
  );
  saveReceipts(storage, restored);
  return restored;
}

export function saveReceipts(storage: ReceiptStorage, receipts: PendingReceipt[]): void {
  storage.setItem(RECEIPT_STORAGE_KEY, JSON.stringify(receipts.slice(0, MAX_STORED_RECEIPTS)));
}

export function upsertReceipt(storage: ReceiptStorage, receipt: PendingReceipt): PendingReceipt[] {
  const receipts = loadReceipts(storage).filter((item) => item.hash !== receipt.hash);
  const next = [receipt, ...receipts].slice(0, MAX_STORED_RECEIPTS);
  saveReceipts(storage, next);
  return next;
}

export function removeReceipt(storage: ReceiptStorage, hash: string): PendingReceipt[] {
  const next = loadReceipts(storage).filter((item) => item.hash !== hash);
  saveReceipts(storage, next);
  return next;
}

/**
 * Guard rails for resuming a stored receipt: never continue when the saved
 * network or account does not match the active one.
 */
export function isSafeToResume(
  receipt: PendingReceipt,
  network: string,
  account: string | null,
): string | null {
  if (network !== receipt.network) {
    return `Network mismatch: saved receipt is for ${receipt.network}, but the current network is ${network}.`;
  }
  if (!account) {
    return "Connect the account that submitted this transaction before retrying receipt polling.";
  }
  if (account !== receipt.account) {
    return `Account mismatch: saved receipt belongs to ${receipt.account}, but the active account is ${account}.`;
  }
  return null;
}

export function receiptStatusLabel(status: ReceiptStatus): string {
  switch (status) {
    case "pending":
      return "Pending — polling";
    case "confirmed":
      return "Confirmed on-chain";
    case "failed":
      return "Failed on-chain";
    case "expired":
      return "Expired — outcome not observed";
    case "unknown":
      return "Unknown outcome";
  }
}
