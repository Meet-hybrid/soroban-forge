"use client";

import { useState } from "react";
import { DEFAULT_TOKEN_ID } from "../../lib/contracts/registry";
import { validateMultiSigSubmit, validateMultiSigConfirm } from "../../lib/validation";

interface MultiSigPanelProps {
  client: any;
  account: string | null;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
  runWriteFlow: (
    actionName: string,
    validate: () => any,
    buildTx: (c: any, data: any) => Promise<any>
  ) => Promise<void>;
}

export function MultiSigPanel({ client, account, onLog, runWriteFlow }: MultiSigPanelProps) {
  // Read state
  const [txId, setTxId] = useState("1");
  const [txPreview, setTxPreview] = useState<any | null>(null);
  const [threshold, setThreshold] = useState<number | null>(null);
  const [owners, setOwners] = useState<string[]>([]);
  const [readBusy, setReadBusy] = useState(false);

  // Write state (submit_withdrawal)
  const [submitter, setSubmitter] = useState(account ?? "");
  const [token, setToken] = useState(DEFAULT_TOKEN_ID);
  const [destination, setDestination] = useState("");
  const [amount, setAmount] = useState("10000000");

  const handleFetchPreview = async () => {
    if (!client) {
      onLog("Connect wallet or verify client initialization first.", "err");
      return;
    }
    const id = BigInt(txId.trim() || "0");
    if (id <= BigInt(0)) {
      onLog("Please enter a valid Transaction ID (> 0).", "err");
      return;
    }

    setReadBusy(true);
    try {
      onLog(`Querying Multi-Sig Tx Preview #${id.toString()}...`, "info");
      const [previewRes, threshRes, ownersRes] = await Promise.allSettled([
        client.get_tx_preview({ tx_id: id }),
        client.get_threshold(),
        client.get_owners(),
      ]);

      if (previewRes.status === "fulfilled" && previewRes.value.result?.isOk?.()) {
        const preview = previewRes.value.result.unwrap();
        setTxPreview(preview);
        onLog(
          `Tx #${id.toString()} preview: Status=${preview.status?.tag ?? "Unknown"}, Approvals=${preview.approvals}/${preview.threshold}, Remaining=${preview.remaining}`,
          "ok"
        );
      } else {
        setTxPreview(null);
      }

      if (threshRes.status === "fulfilled" && threshRes.value.result?.isOk?.()) {
        setThreshold(threshRes.value.result.unwrap());
      }
      if (ownersRes.status === "fulfilled" && ownersRes.value.result?.isOk?.()) {
        setOwners(ownersRes.value.result.unwrap());
      }
    } catch (e) {
      onLog(`Multi-sig query error: ${(e as Error).message}`, "err");
    } finally {
      setReadBusy(false);
    }
  };

  const handleSubmitWithdrawal = async () => {
    await runWriteFlow(
      "submit_withdrawal",
      () =>
        validateMultiSigSubmit({
          submitter: submitter || account || "",
          token,
          destination,
          amount,
        }),
      async (c, data) => {
        return await c.submit_withdrawal({
          submitter: data.submitter,
          token: data.token,
          destination: data.destination,
          amount: data.amount,
        });
      }
    );
  };

  const handleConfirmTx = async () => {
    await runWriteFlow(
      "confirm",
      () =>
        validateMultiSigConfirm({
          signer: account ?? "",
          txId,
        }),
      async (c, data) => {
        return await c.confirm({
          signer: data.signer,
          tx_id: data.tx_id,
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
            Multi-Sig Read Flows &amp; Preview Query
          </h3>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Query transaction preview (status, threshold, current approvals, remaining needed) and wallet config.
          </p>
        </div>

        <div className="rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-950/50">
          <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
            Transaction ID:
          </label>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={txId}
              onChange={(e) => setTxId(e.target.value)}
              placeholder="e.g. 1"
              className="w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-900"
            />
            <button
              type="button"
              onClick={handleFetchPreview}
              disabled={readBusy}
              className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50 dark:bg-slate-700 dark:hover:bg-slate-600"
            >
              {readBusy ? "Reading..." : "Preview Query"}
            </button>
          </div>

          {txPreview && (
            <div className="mt-3 space-y-2 rounded border border-slate-200 bg-white p-3 font-mono text-xs dark:border-slate-800 dark:bg-slate-900">
              <div className="flex justify-between items-center">
                <span className="text-slate-500">Transaction Status:</span>
                <span className="rounded bg-purple-50 px-2 py-0.5 font-semibold text-purple-700 dark:bg-purple-950 dark:text-purple-300">
                  {txPreview.status?.tag ?? "Unknown"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Configured Threshold:</span>
                <span className="font-semibold">{txPreview.threshold}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Current Approvals:</span>
                <span className="text-emerald-600 dark:text-emerald-400 font-semibold">
                  {txPreview.approvals}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Confirmations Required:</span>
                <span className="text-amber-600 dark:text-amber-400 font-semibold">
                  {txPreview.remaining}
                </span>
              </div>
            </div>
          )}

          {threshold !== null && (
            <div className="mt-2 text-[11px] text-slate-500">
              Wallet Threshold: <strong className="text-slate-700 dark:text-slate-300">{threshold}</strong> · Owners: {owners.length}
            </div>
          )}
        </div>
      </div>

      {/* Write Panel */}
      <div className="space-y-4 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
        <div>
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-base text-slate-900 dark:text-slate-100">
              Write Flow: submit_withdrawal
            </h3>
            <span className="rounded bg-purple-100 px-2 py-0.5 text-[10px] font-semibold text-purple-800 dark:bg-purple-950 dark:text-purple-300">
              Safe &amp; Validated
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Submit a token withdrawal proposal requiring owner confirmations up to the threshold.
          </p>
        </div>

        <div className="space-y-2.5">
          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Submitter (Wallet Owner Address):
            </label>
            <input
              type="text"
              value={submitter}
              onChange={(e) => setSubmitter(e.target.value)}
              placeholder={account ?? "G..."}
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Destination Address:
            </label>
            <input
              type="text"
              value={destination}
              onChange={(e) => setDestination(e.target.value)}
              placeholder="G... or C..."
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
              Amount (stroops):
            </label>
            <input
              type="text"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              placeholder="10000000"
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div className="flex gap-2 pt-2">
            <button
              type="button"
              onClick={handleSubmitWithdrawal}
              className="flex-1 rounded-lg bg-indigo-600 px-3 py-2 text-xs font-semibold text-white shadow hover:bg-indigo-500 transition"
            >
              Propose Withdrawal
            </button>
            <button
              type="button"
              onClick={handleConfirmTx}
              className="rounded-lg border border-indigo-600 px-3 py-2 text-xs font-semibold text-indigo-600 hover:bg-indigo-50 dark:hover:bg-indigo-950/40 transition"
            >
              Confirm Tx #{txId}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
