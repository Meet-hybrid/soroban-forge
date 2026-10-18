"use client";

import { type DecodedEvent, type ParsedError } from "../lib/forge-errors";

export type LifecycleStep =
  | "idle"
  | "validating"
  | "simulating"
  | "signing"
  | "submitting"
  | "success"
  | "error";

export interface TransactionLifecycleState {
  step: LifecycleStep;
  actionName?: string;
  txHash?: string;
  ledger?: number;
  simulationDetails?: string;
  decodedEvents?: DecodedEvent[];
  error?: ParsedError;
}

interface TransactionLifecycleProps {
  state: TransactionLifecycleState;
  onDismiss: () => void;
}

const STEPS: Array<{ key: LifecycleStep; label: string }> = [
  { key: "validating", label: "1. Validation" },
  { key: "simulating", label: "2. Simulation" },
  { key: "signing", label: "3. Signing" },
  { key: "submitting", label: "4. Submission" },
  { key: "success", label: "5. Receipt" },
];

export function TransactionLifecycle({ state, onDismiss }: TransactionLifecycleProps) {
  if (state.step === "idle") {
    return null;
  }

  const isFailed = state.step === "error";
  const isDone = state.step === "success";

  const getStepStatus = (stepKey: LifecycleStep) => {
    const order: LifecycleStep[] = ["validating", "simulating", "signing", "submitting", "success"];
    const currentIndex = order.indexOf(state.step);
    const stepIndex = order.indexOf(stepKey);

    if (isFailed) {
      if (stepIndex === currentIndex) return "failed";
      if (stepIndex < currentIndex) return "done";
      return "upcoming";
    }

    if (isDone) return "done";
    if (stepIndex < currentIndex) return "done";
    if (stepIndex === currentIndex) return "active";
    return "upcoming";
  };

  return (
    <div className="mb-6 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
      <div className="flex items-center justify-between border-b border-slate-100 pb-3 dark:border-slate-800">
        <div className="flex items-center gap-2">
          <span className="font-semibold text-sm text-slate-900 dark:text-slate-100">
            Transaction Lifecycle
          </span>
          {state.actionName && (
            <span className="rounded bg-indigo-50 px-2 py-0.5 font-mono text-xs text-indigo-700 dark:bg-indigo-950 dark:text-indigo-300">
              {state.actionName}
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={onDismiss}
          className="rounded p-1 text-xs text-slate-400 hover:text-slate-600 dark:hover:text-slate-200"
        >
          ✕ Dismiss
        </button>
      </div>

      {/* Steps Visualizer */}
      <div className="mt-4 grid grid-cols-5 gap-2 text-center text-xs">
        {STEPS.map((s) => {
          const status = getStepStatus(s.key);
          let bg = "bg-slate-100 text-slate-500 dark:bg-slate-800 dark:text-slate-400";
          if (status === "active") {
            bg = "bg-indigo-600 text-white font-semibold animate-pulse";
          } else if (status === "done") {
            bg = "bg-emerald-100 text-emerald-800 font-medium dark:bg-emerald-950 dark:text-emerald-300";
          } else if (status === "failed") {
            bg = "bg-red-100 text-red-800 font-semibold dark:bg-red-950 dark:text-red-300";
          }

          return (
            <div key={s.key} className={`rounded-lg py-2 px-1 transition ${bg}`}>
              {status === "done" && "✓ "}
              {status === "failed" && "✗ "}
              {s.label}
            </div>
          );
        })}
      </div>

      {/* Active Details */}
      <div className="mt-4 space-y-3">
        {/* Error Details */}
        {state.error && (
          <div className="rounded-lg border border-red-200 bg-red-50/70 p-4 dark:border-red-900/50 dark:bg-red-950/30">
            <div className="flex items-start justify-between">
              <div>
                <h4 className="font-semibold text-sm text-red-800 dark:text-red-300">
                  {state.error.name ? `ForgeError #${state.error.code}: ${state.error.name}` : "Transaction Execution Failed"}
                </h4>
                {state.error.description && (
                  <p className="mt-1 text-xs text-red-700 dark:text-red-400">
                    {state.error.description}
                  </p>
                )}
              </div>
              <button
                type="button"
                onClick={onDismiss}
                className="rounded bg-red-200 px-2 py-1 text-xs font-medium text-red-800 hover:bg-red-300 dark:bg-red-900 dark:text-red-200"
              >
                Reset &amp; Retry
              </button>
            </div>
            <div className="mt-2 rounded bg-red-100/60 p-2 font-mono text-[11px] text-red-900 break-all dark:bg-red-950/50 dark:text-red-300">
              {state.error.rawMessage}
            </div>
          </div>
        )}

        {/* Success / Submission info */}
        {state.txHash && (
          <div className="rounded-lg border border-emerald-200 bg-emerald-50/60 p-3 text-xs dark:border-emerald-900/40 dark:bg-emerald-950/30">
            <span className="font-semibold text-emerald-800 dark:text-emerald-300">
              Submitted to Stellar Testnet:
            </span>{" "}
            <a
              href={`https://stellar.expert/explorer/testnet/tx/${state.txHash}`}
              target="_blank"
              rel="noreferrer"
              className="font-mono text-indigo-600 underline hover:text-indigo-700 dark:text-indigo-400 break-all"
            >
              {state.txHash}
            </a>
            {state.ledger && (
              <span className="ml-2 text-slate-500">
                · Ledger <strong className="text-slate-700 dark:text-slate-300">#{state.ledger}</strong>
              </span>
            )}
          </div>
        )}

        {/* Simulation Preview */}
        {state.simulationDetails && (
          <details className="rounded-lg border border-slate-200 bg-slate-50 p-2 text-xs dark:border-slate-800 dark:bg-slate-950">
            <summary className="cursor-pointer font-medium text-slate-600 dark:text-slate-400">
              Simulation Details
            </summary>
            <pre className="mt-2 overflow-x-auto font-mono text-[11px] text-slate-700 dark:text-slate-300">
              {state.simulationDetails}
            </pre>
          </details>
        )}

        {/* Decoded Events */}
        {state.decodedEvents && state.decodedEvents.length > 0 && (
          <div className="rounded-lg border border-slate-200 p-3 dark:border-slate-800">
            <h5 className="font-semibold text-xs text-slate-700 dark:text-slate-300">
              Decoded Contract Events ({state.decodedEvents.length})
            </h5>
            <div className="mt-2 space-y-1.5 max-h-40 overflow-y-auto">
              {state.decodedEvents.map((ev, i) => (
                <div
                  key={i}
                  className="rounded border border-slate-100 bg-slate-50 p-2 font-mono text-[11px] dark:border-slate-800 dark:bg-slate-950"
                >
                  <div className="text-indigo-600 dark:text-indigo-400 font-medium">
                    Topic: {ev.topic}
                  </div>
                  <div className="text-slate-600 dark:text-slate-300 break-all">
                    Data: {ev.value}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
