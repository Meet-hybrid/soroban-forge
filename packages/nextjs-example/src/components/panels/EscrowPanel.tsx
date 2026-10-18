"use client";

import { useState } from "react";
import { DEFAULT_TOKEN_ID } from "../../lib/contracts/registry";
import { validateCreateEscrow } from "../../lib/validation";

interface EscrowPanelProps {
  client: any;
  account: string | null;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
  runWriteFlow: (
    actionName: string,
    validate: () => any,
    buildTx: (c: any, data: any) => Promise<any>
  ) => Promise<void>;
}

export function EscrowPanel({ client, account, onLog, runWriteFlow }: EscrowPanelProps) {
  // Read state
  const [readId, setReadId] = useState("1");
  const [escrowResult, setEscrowResult] = useState<any | null>(null);
  const [readBusy, setReadBusy] = useState(false);

  // Participant list state
  const [participantAddr, setParticipantAddr] = useState(account ?? "");
  const [participantIds, setParticipantIds] = useState<string[]>([]);
  const [listBusy, setListBusy] = useState(false);

  // Write state (create_escrow)
  const [buyer, setBuyer] = useState(account ?? "");
  const [seller, setSeller] = useState("");
  const [arbiter, setArbiter] = useState("");
  const [token, setToken] = useState(DEFAULT_TOKEN_ID);
  const [amount, setAmount] = useState("10000000"); // 10^7 = 1 XLM/unit
  const [timeoutSecs, setTimeoutSecs] = useState("3600");

  const handleFetchEscrow = async () => {
    if (!client) {
      onLog("Connect wallet or verify client initialization first.", "err");
      return;
    }
    const id = BigInt(readId.trim() || "0");
    if (id <= BigInt(0)) {
      onLog("Please enter a valid positive Escrow ID.", "err");
      return;
    }

    setReadBusy(true);
    try {
      onLog(`Fetching Escrow #${id.toString()}...`, "info");
      const { result } = await client.get_escrow({ escrow_id: id });
      if (result.isOk()) {
        const data = result.unwrap();
        setEscrowResult(data);
        onLog(`Fetched Escrow #${id.toString()} — Status: ${data.status?.tag ?? "Unknown"}`, "ok");
      } else {
        const err = result.unwrapErr();
        setEscrowResult(null);
        onLog(`get_escrow returned: ${err.message ?? JSON.stringify(err)}`, "err");
      }
    } catch (e) {
      onLog(`get_escrow error: ${(e as Error).message}`, "err");
    } finally {
      setReadBusy(false);
    }
  };

  const handleListParticipant = async () => {
    if (!client) return;
    const addr = (participantAddr || account || "").trim();
    if (!addr) {
      onLog("Please enter a valid participant address.", "err");
      return;
    }

    setListBusy(true);
    try {
      onLog(`Fetching escrows for participant ${addr.slice(0, 6)}...`, "info");
      const { result } = await client.escrows_for_participant({
        participant: addr,
        cursor: 0,
        limit: 10,
      });
      const ids = (result.ids ?? []).map((id: bigint) => id.toString());
      setParticipantIds(ids);
      onLog(`Found ${ids.length} escrows (total ${result.total}) for ${addr.slice(0, 6)}...`, "ok");
    } catch (e) {
      onLog(`escrows_for_participant error: ${(e as Error).message}`, "err");
    } finally {
      setListBusy(false);
    }
  };

  const handleCreateEscrow = async () => {
    await runWriteFlow(
      "create_escrow",
      () =>
        validateCreateEscrow({
          buyer: buyer || account || "",
          seller,
          arbiter,
          token,
          amount,
          timeoutSecs,
        }),
      async (c, data) => {
        return await c.create_escrow({
          buyer: data.buyer,
          seller: data.seller,
          arbiter: data.arbiter,
          token: data.token,
          amount: data.amount,
          timeout: data.timeout,
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
            Escrow Read Flows
          </h3>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Query stored 3-party escrow state and participant indices.
          </p>
        </div>

        {/* Query by ID */}
        <div className="rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-950/50">
          <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
            Query by Escrow ID:
          </label>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={readId}
              onChange={(e) => setReadId(e.target.value)}
              placeholder="e.g. 1"
              className="w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-900"
            />
            <button
              type="button"
              onClick={handleFetchEscrow}
              disabled={readBusy}
              className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50 dark:bg-slate-700 dark:hover:bg-slate-600"
            >
              {readBusy ? "Fetching..." : "Fetch"}
            </button>
          </div>

          {escrowResult && (
            <div className="mt-3 space-y-1.5 rounded border border-slate-200 bg-white p-3 font-mono text-xs dark:border-slate-800 dark:bg-slate-900">
              <div className="flex justify-between">
                <span className="text-slate-500">Status:</span>
                <span className="font-semibold text-indigo-600 dark:text-indigo-400">
                  {escrowResult.status?.tag ?? "Unknown"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Amount:</span>
                <span>{escrowResult.amount?.toString()} stroops</span>
              </div>
              <div className="flex justify-between truncate">
                <span className="text-slate-500">Buyer:</span>
                <span className="truncate max-w-[200px]">{escrowResult.buyer}</span>
              </div>
              <div className="flex justify-between truncate">
                <span className="text-slate-500">Seller:</span>
                <span className="truncate max-w-[200px]">{escrowResult.seller}</span>
              </div>
              <div className="flex justify-between truncate">
                <span className="text-slate-500">Arbiter:</span>
                <span className="truncate max-w-[200px]">{escrowResult.arbiter}</span>
              </div>
            </div>
          )}
        </div>

        {/* Participant list */}
        <div className="rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-950/50">
          <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
            Escrows for Participant:
          </label>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={participantAddr}
              onChange={(e) => setParticipantAddr(e.target.value)}
              placeholder="G... or C..."
              className="w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-900"
            />
            <button
              type="button"
              onClick={handleListParticipant}
              disabled={listBusy}
              className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50 dark:bg-slate-700 dark:hover:bg-slate-600"
            >
              {listBusy ? "Loading..." : "List"}
            </button>
          </div>

          {participantIds.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {participantIds.map((id) => (
                <button
                  key={id}
                  type="button"
                  onClick={() => {
                    setReadId(id);
                  }}
                  className="rounded bg-indigo-100 px-2 py-0.5 font-mono text-xs text-indigo-800 hover:bg-indigo-200 dark:bg-indigo-950 dark:text-indigo-300"
                >
                  #{id}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Write Panel */}
      <div className="space-y-4 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
        <div>
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-base text-slate-900 dark:text-slate-100">
              Write Flow: create_escrow
            </h3>
            <span className="rounded bg-blue-100 px-2 py-0.5 text-[10px] font-semibold text-blue-800 dark:bg-blue-950 dark:text-blue-300">
              Safe &amp; Validated
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Creates a new escrow agreement. Pre-flight client validation checks all addresses and positive amounts before signing.
          </p>
        </div>

        <div className="space-y-2.5">
          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Buyer Address:
            </label>
            <input
              type="text"
              value={buyer}
              onChange={(e) => setBuyer(e.target.value)}
              placeholder={account ?? "G..."}
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Seller Address:
            </label>
            <input
              type="text"
              value={seller}
              onChange={(e) => setSeller(e.target.value)}
              placeholder="G... or C..."
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Arbiter Address:
            </label>
            <input
              type="text"
              value={arbiter}
              onChange={(e) => setArbiter(e.target.value)}
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

          <div className="grid grid-cols-2 gap-2">
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
            <div>
              <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
                Timeout (seconds):
              </label>
              <input
                type="text"
                value={timeoutSecs}
                onChange={(e) => setTimeoutSecs(e.target.value)}
                placeholder="3600"
                className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
              />
            </div>
          </div>

          <button
            type="button"
            onClick={handleCreateEscrow}
            className="mt-3 w-full rounded-lg bg-indigo-600 px-4 py-2 text-xs font-semibold text-white shadow hover:bg-indigo-500 transition"
          >
            Simulate, Sign &amp; Create Escrow
          </button>
        </div>
      </div>
    </div>
  );
}
