"use client";

import { useState } from "react";
import { DEFAULT_TOKEN_ID } from "../../lib/contracts/registry";
import { validateCreateVesting } from "../../lib/validation";

interface VestingPanelProps {
  client: any;
  account: string | null;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
  runWriteFlow: (
    actionName: string,
    validate: () => any,
    buildTx: (c: any, data: any) => Promise<any>
  ) => Promise<void>;
}

export function VestingPanel({ client, account, onLog, runWriteFlow }: VestingPanelProps) {
  // Read state
  const [scheduleId, setScheduleId] = useState("1");
  const [statusResult, setStatusResult] = useState<string | null>(null);
  const [claimableResult, setClaimableResult] = useState<string | null>(null);
  const [readBusy, setReadBusy] = useState(false);

  // Write state (create_schedule)
  const [beneficiary, setBeneficiary] = useState(account ?? "");
  const [token, setToken] = useState(DEFAULT_TOKEN_ID);
  const [totalAmount, setTotalAmount] = useState("50000000"); // 5 tokens
  const [cliffSecs, setCliffSecs] = useState("86400"); // 1 day
  const [durationSecs, setDurationSecs] = useState("604800"); // 7 days

  const handleFetchStatus = async () => {
    if (!client) {
      onLog("Connect wallet or verify client initialization first.", "err");
      return;
    }
    const id = BigInt(scheduleId.trim() || "0");
    if (id <= BigInt(0)) {
      onLog("Please enter a valid schedule ID (> 0).", "err");
      return;
    }

    setReadBusy(true);
    try {
      onLog(`Querying Vesting Schedule #${id.toString()}...`, "info");
      const [statusTx, claimableTx] = await Promise.allSettled([
        client.get_status({ schedule_id: id }),
        client.claimable({ schedule_id: id }),
      ]);

      if (statusTx.status === "fulfilled" && statusTx.value.result?.isOk?.()) {
        const status = statusTx.value.result.unwrap();
        setStatusResult(status.tag ?? JSON.stringify(status));
      } else {
        setStatusResult(null);
      }

      if (claimableTx.status === "fulfilled" && claimableTx.value.result?.isOk?.()) {
        const claimable = claimableTx.value.result.unwrap();
        setClaimableResult(claimable.toString());
      } else {
        setClaimableResult(null);
      }

      onLog(`Schedule #${id.toString()} queried successfully.`, "ok");
    } catch (e) {
      onLog(`Vesting read error: ${(e as Error).message}`, "err");
    } finally {
      setReadBusy(false);
    }
  };

  const handleCreateSchedule = async () => {
    await runWriteFlow(
      "create_schedule",
      () =>
        validateCreateVesting({
          beneficiary: beneficiary || account || "",
          token,
          totalAmount,
          cliffSecs,
          durationSecs,
        }),
      async (c, data) => {
        return await c.create_schedule({
          beneficiary: data.beneficiary,
          token: data.token,
          total_amount: data.total_amount,
          cliff: data.cliff,
          duration: data.duration,
        });
      }
    );
  };

  return (
    <div className="grid gap-6 lg:grid-cols-2">
      {/* Read Panel */}
      <div className="space-y-4 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
        <div>
          <h3 className="font-semibold text-base text-slate-900 dark:text-slate-100">
            Vesting Read Flows
          </h3>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Inspect real-time vesting progress, lifecycle status, and claimable tokens.
          </p>
        </div>

        <div className="rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-950/50">
          <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
            Schedule ID:
          </label>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={scheduleId}
              onChange={(e) => setScheduleId(e.target.value)}
              placeholder="e.g. 1"
              className="w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-900"
            />
            <button
              type="button"
              onClick={handleFetchStatus}
              disabled={readBusy}
              className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50 dark:bg-slate-700 dark:hover:bg-slate-600"
            >
              {readBusy ? "Reading..." : "Read"}
            </button>
          </div>

          {(statusResult !== null || claimableResult !== null) && (
            <div className="mt-3 space-y-2 rounded border border-slate-200 bg-white p-3 font-mono text-xs dark:border-slate-800 dark:bg-slate-900">
              <div className="flex justify-between items-center">
                <span className="text-slate-500">Lifecycle Status:</span>
                <span className="rounded bg-indigo-50 px-2 py-0.5 font-semibold text-indigo-700 dark:bg-indigo-950 dark:text-indigo-300">
                  {statusResult ?? "Not Found / Error"}
                </span>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-slate-500">Claimable Amount:</span>
                <span className="font-semibold text-emerald-600 dark:text-emerald-400">
                  {claimableResult !== null ? `${claimableResult} stroops` : "N/A"}
                </span>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Write Panel */}
      <div className="space-y-4 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
        <div>
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-base text-slate-900 dark:text-slate-100">
              Write Flow: create_schedule
            </h3>
            <span className="rounded bg-emerald-100 px-2 py-0.5 text-[10px] font-semibold text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300">
              Safe &amp; Validated
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Creates a linear token-vesting schedule. Enforces cliff ≤ duration and amount &gt; 0 before signing.
          </p>
        </div>

        <div className="space-y-2.5">
          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Beneficiary Address:
            </label>
            <input
              type="text"
              value={beneficiary}
              onChange={(e) => setBeneficiary(e.target.value)}
              placeholder={account ?? "G..."}
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Token Contract ID:
            </label>
            <input
              type="text"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="C..."
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Total Amount (stroops):
            </label>
            <input
              type="text"
              value={totalAmount}
              onChange={(e) => setTotalAmount(e.target.value)}
              placeholder="50000000"
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
                Cliff (seconds):
              </label>
              <input
                type="text"
                value={cliffSecs}
                onChange={(e) => setCliffSecs(e.target.value)}
                placeholder="86400"
                className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
                Duration (seconds):
              </label>
              <input
                type="text"
                value={durationSecs}
                onChange={(e) => setDurationSecs(e.target.value)}
                placeholder="604800"
                className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
              />
            </div>
          </div>

          <button
            type="button"
            onClick={handleCreateSchedule}
            className="mt-3 w-full rounded-lg bg-indigo-600 px-4 py-2 text-xs font-semibold text-white shadow hover:bg-indigo-500 transition"
          >
            Simulate, Sign &amp; Create Schedule
          </button>
        </div>
      </div>
    </div>
  );
}
