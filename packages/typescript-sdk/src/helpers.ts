import type { AssembledTransaction, SentTransaction } from "@stellar/stellar-sdk/contract";
import type { SignTransaction } from "@stellar/stellar-sdk/contract";
import { rpc } from "@stellar/stellar-sdk";
import { FORGE_ERRORS } from "./errors.generated.js";

export type ForgeResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: ForgeErrorDetail };

export interface ForgeErrorDetail {
  code?: number;
  name?: string;
  message: string;
  raw?: unknown;
}

export interface ForgeNetworkConfig {
  rpcUrl: string;
  networkPassphrase: string;
  contractId: string;
  allowHttp?: boolean;
  headers?: Record<string, string>;
}

export interface SignerCallbacks {
  signTransaction: SignTransaction;
  signAuthEntry?: Parameters<SignTransaction>[0] extends never ? never : never;
}

export type ForgeOutcome<T> =
  | { stage: "success"; value: T; events: unknown[] }
  | { stage: "contract_error"; error: ForgeErrorDetail; events: unknown[] }
  | { stage: "simulation_failed" | "submission_failed" | "timeout" | "malformed_receipt"; error: ForgeErrorDetail };

export class ForgeSdkError extends Error {
  constructor(public readonly stage: Exclude<ForgeOutcome<never>["stage"], "success">, message: string, public readonly raw?: unknown) {
    super(message);
    this.name = "ForgeSdkError";
  }
}

export function decodeContractResult<T>(raw: unknown): ForgeResult<T> {
  if (!raw || typeof raw !== "object" || !("ok" in raw)) {
    return { ok: false, error: { message: "Malformed contract result payload", raw } };
  }
  const record = raw as Record<string, unknown>;
  if (record.ok === true) return { ok: true, value: record.value as T };

  const errorRaw = record.error;
  if (errorRaw && typeof errorRaw === "object") {
    const error = errorRaw as Record<string, unknown>;
    const code = typeof error.code === "number"
      ? error.code
      : typeof error.code === "bigint"
        ? Number(error.code)
        : undefined;
    const name = typeof error.name === "string" ? error.name : code !== undefined ? FORGE_ERRORS[code] : undefined;
    const message = typeof error.message === "string" ? error.message : name ?? "Unknown contract error";
    return { ok: false, error: { code, name, message, raw: errorRaw } };
  }
  if (typeof errorRaw === "number") {
    const code = errorRaw;
    const name = FORGE_ERRORS[code];
    return { ok: false, error: { code, name, message: name ?? "Unknown contract error", raw: errorRaw } };
  }
  return { ok: false, error: { message: "Unknown contract error", raw: errorRaw ?? raw } };
}

export function decodeEscrowResultCompat<T>(raw: unknown): ForgeResult<T> {
  return decodeContractResult<T>(raw);
}

export interface PollOptions {
  maxAttempts?: number;
  intervalMs?: number;
  timeoutMs?: number;
  sleep?: (ms: number) => Promise<void>;
}

export type TransactionPoller = {
  getTransaction(hash: string): Promise<rpc.Api.GetTransactionResponse>;
};

export async function pollTransactionStatus(
  poller: TransactionPoller | rpc.Server,
  hash: string,
  options: PollOptions = {},
): Promise<rpc.Api.GetTransactionResponse> {
  const maxAttempts = options.maxAttempts ?? 30;
  const intervalMs = options.intervalMs ?? 1000;
  const timeoutAt = options.timeoutMs === undefined ? undefined : Date.now() + options.timeoutMs;
  const sleep = options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)));

  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    if (timeoutAt !== undefined && Date.now() >= timeoutAt) {
      throw new ForgeSdkError("timeout", `Transaction polling timed out for ${hash}`, { hash });
    }
    const response = await poller.getTransaction(hash);
    if (response.status !== rpc.Api.GetTransactionStatus.NOT_FOUND) return response;
    await sleep(intervalMs);
  }
  throw new ForgeSdkError("timeout", `Transaction polling timed out for ${hash}`, { hash });
}

export const pollTransaction = pollTransactionStatus;
export const poll = pollTransactionStatus;

export async function simulate<T>(
  invoke: () => Promise<AssembledTransaction<T>>,
): Promise<AssembledTransaction<T> | ForgeOutcome<T>> {
  try {
    return await invoke();
  } catch (error) {
    return {
      stage: "simulation_failed",
      error: { message: error instanceof Error ? error.message : "Simulation failed", raw: error },
    };
  }
}

export async function signTransaction<T>(
  assembled: AssembledTransaction<T>,
  signTransaction: SignTransaction,
): Promise<void> {
  await assembled.sign({ signTransaction });
}

export async function signAndSend<T>(
  assembled: AssembledTransaction<T>,
  signTransaction: SignTransaction,
): Promise<SentTransaction<T>> {
  try {
    return await assembled.signAndSend({ signTransaction });
  } catch (error) {
    throw new ForgeSdkError("submission_failed", error instanceof Error ? error.message : "Submission failed", error);
  }
}

export function decodeForgeReceipt<T>(receipt: unknown): ForgeOutcome<T> {
  if (!receipt || typeof receipt !== "object") {
    return { stage: "malformed_receipt", error: { message: "Malformed transaction receipt", raw: receipt } };
  }
  const record = receipt as Record<string, unknown>;
  const status = typeof record.status === "string" ? record.status : undefined;
  const events = Array.isArray(record.events)
    ? record.events
    : Array.isArray(record.diagnosticEventsXdr)
      ? record.diagnosticEventsXdr
      : [];

  if (status === "FAILED" || status === "FAILURE") {
    const decoded = decodeContractResult<T>(record.result);
    return {
      stage: "contract_error",
      error: decoded.ok ? { message: "Transaction failed", raw: receipt } : decoded.error,
      events,
    };
  }
  if (status === "NOT_FOUND") {
    return { stage: "timeout", error: { message: "Transaction receipt not found", raw: receipt } };
  }
  if (status !== undefined && status !== "SUCCESS") {
    return { stage: "malformed_receipt", error: { message: `Unexpected receipt status: ${status}`, raw: receipt } };
  }

  if ("returnValue" in record) {
    return { stage: "success", value: record.returnValue as T, events };
  }
  if ("result" in record) {
    const decoded = decodeContractResult<T>(record.result);
    if (!decoded.ok) return { stage: "contract_error", error: decoded.error, events };
    return { stage: "success", value: decoded.value, events };
  }
  if (status === "SUCCESS") {
    return { stage: "malformed_receipt", error: { message: "Successful receipt has no typed return value", raw: receipt } };
  }
  return { stage: "malformed_receipt", error: { message: "Malformed transaction receipt", raw: receipt } };
}
