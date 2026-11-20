"use client";

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import {
  DIAGNOSTICS_AUTO_REFRESH_MS,
  DIAGNOSTICS_RECHECK_THROTTLE_MS,
  diagnosticsHealth,
  diagnosticsReducer,
  initialDiagnosticsState,
  runDiagnostics,
  safeEndpoint,
  type DiagnosticsCheck,
  type DiagnosticsCheckStatus,
  type DiagnosticsConfig,
  type DiagnosticsHealth,
  type DiagnosticsProbes,
} from "./diagnostics";

interface DiagnosticsPanelProps {
  config: DiagnosticsConfig;
  probes: DiagnosticsProbes;
  enabled: boolean;
  refreshKey?: string;
}

const CHECK_LABELS = [
  { key: "network", label: "Network" },
  { key: "rpc", label: "RPC" },
  { key: "contract", label: "Contract" },
] as const;

const HEALTH_LABELS: Record<DiagnosticsHealth, string> = {
  loading: "Checking…",
  healthy: "Healthy",
  stale: "Stale",
  error: "Action needed",
};

const HEALTH_STYLES: Record<DiagnosticsHealth, string> = {
  loading: "bg-blue-100 text-blue-800 border-blue-300",
  healthy: "bg-emerald-100 text-emerald-800 border-emerald-300",
  stale: "bg-amber-100 text-amber-800 border-amber-300",
  error: "bg-red-100 text-red-800 border-red-300",
};

const CHECK_STYLES: Record<DiagnosticsCheckStatus, string> = {
  pass: "bg-emerald-100 text-emerald-800",
  fail: "bg-red-100 text-red-800",
  skip: "bg-gray-100 text-gray-600",
};

function CheckItem({
  label,
  check,
}: {
  label: string;
  check: DiagnosticsCheck | undefined;
}) {
  return (
    <div className="rounded-lg border border-gray-200 bg-gray-50 p-2.5">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-semibold text-gray-700">{label}</span>
        <span
          className={`rounded px-1.5 py-0.5 text-[10px] font-bold uppercase ${CHECK_STYLES[check?.status ?? "skip"]}`}
        >
          {check?.status ?? "skip"}
        </span>
      </div>
      <p className="mt-1 break-words text-[11px] text-gray-600">
        {check?.detail ?? "No result yet."}
      </p>
    </div>
  );
}

export default function DiagnosticsPanel({
  config,
  probes,
  enabled,
  refreshKey,
}: DiagnosticsPanelProps) {
  const [state, dispatch] = useReducer(diagnosticsReducer, initialDiagnosticsState);
  const [now, setNow] = useState(() => Date.now());
  const probesRef = useRef(probes);
  const lastRunRef = useRef(0);
  const inFlightRef = useRef(false);
  const mountedRef = useRef(true);

  const runCheck = useCallback(async () => {
    if (inFlightRef.current) {
      return;
    }
    const startedAt = Date.now();
    if (startedAt - lastRunRef.current < DIAGNOSTICS_RECHECK_THROTTLE_MS) {
      return;
    }
    lastRunRef.current = startedAt;
    inFlightRef.current = true;
    dispatch({ type: "check-started" });
    try {
      const snapshot = await runDiagnostics(config, probesRef.current, startedAt);
      if (mountedRef.current) {
        dispatch({ type: "check-finished", snapshot });
      }
    } catch (error) {
      if (mountedRef.current) {
        dispatch({
          type: "check-failed",
          message: error instanceof Error ? error.message : String(error),
          at: Date.now(),
        });
      }
    } finally {
      inFlightRef.current = false;
    }
  }, [config]);

  useEffect(() => {
    probesRef.current = probes;
  }, [probes]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void runCheck();
    const timer = window.setInterval(() => {
      void runCheck();
    }, DIAGNOSTICS_AUTO_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [enabled, refreshKey, runCheck]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 15_000);
    return () => window.clearInterval(timer);
  }, []);

  const health: DiagnosticsHealth =
    state.phase === "loading"
      ? "loading"
      : state.error !== null
        ? "error"
        : diagnosticsHealth(state.snapshot, now);

  const snapshot = state.snapshot;
  const lastChecked = snapshot === null ? null : new Date(snapshot.checkedAt).toLocaleTimeString();

  return (
    <section
      aria-label="Live testnet diagnostics"
      className="mb-6 rounded-xl border border-gray-300 bg-white p-4 shadow-sm"
    >
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold text-gray-900">
            Live Testnet Diagnostics
          </h2>
          <p className="text-[11px] text-gray-500">
            Read-only checks · no wallet signatures · rechecks are throttled.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <span
            role="status"
            aria-live="polite"
            className={`rounded-full border px-2 py-0.5 text-[11px] font-semibold ${HEALTH_STYLES[health]}`}
          >
            {HEALTH_LABELS[health]}
          </span>
          <button
            type="button"
            onClick={() => void runCheck()}
            disabled={state.phase === "loading"}
            className="rounded border border-gray-300 bg-white px-2.5 py-1 text-[11px] font-semibold text-gray-700 hover:bg-gray-50 disabled:opacity-50"
          >
            {state.phase === "loading" ? "Checking…" : "Recheck"}
          </button>
        </div>
      </div>

      <dl className="grid gap-2 sm:grid-cols-3">
        {CHECK_LABELS.map(({ key, label }) => (
          <div key={key}>
            <dt className="sr-only">{label} diagnostics</dt>
            <dd>
              <CheckItem label={label} check={snapshot?.[key]} />
            </dd>
          </div>
        ))}
      </dl>

      <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-gray-500">
        <span>
          Network: <strong>{config.networkName}</strong> ·{" "}
          <span className="font-mono">{config.networkPassphrase}</span>
        </span>
        <span className="break-all font-mono">{safeEndpoint(config.rpcUrl)}</span>
        <span className="break-all font-mono">{config.contractId}</span>
        {typeof snapshot?.latestLedger === "number" && (
          <span>Latest ledger: #{snapshot.latestLedger}</span>
        )}
        {lastChecked !== null && <span>Last checked: {lastChecked}</span>}
      </div>

      {state.phase === "loading" && (
        <p className="mt-2 text-[11px] text-blue-700">
          Checking network, RPC reachability, and contract readability…
        </p>
      )}

      {state.error !== null && (
        <p role="alert" className="mt-2 rounded border border-red-200 bg-red-50 p-2 text-[11px] text-red-800">
          {state.error} Use Recheck to try again.
        </p>
      )}

      {snapshot !== null &&
        (snapshot.nextSteps.length > 0 || health === "stale") &&
        health !== "healthy" && (
          <ul role="alert" className="mt-2 space-y-1 rounded border border-amber-200 bg-amber-50 p-2 text-[11px] text-amber-900">
            {snapshot.nextSteps.map((step) => (
              <li key={step}>• {step}</li>
            ))}
            {health === "stale" && (
              <li>• Recheck to refresh the latest ledger and contract status.</li>
            )}
          </ul>
        )}

      {!enabled && (
        <p className="mt-2 text-[11px] text-gray-500">
          Diagnostics run in Live Testnet mode.
        </p>
      )}
    </section>
  );
}
