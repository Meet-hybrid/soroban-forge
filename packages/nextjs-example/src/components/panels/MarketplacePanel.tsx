"use client";

import { useState } from "react";
import { validateSetRoyalty } from "../../lib/validation";

interface MarketplacePanelProps {
  client: any;
  account: string | null;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
  runWriteFlow: (
    actionName: string,
    validate: () => any,
    buildTx: (c: any, data: any) => Promise<any>
  ) => Promise<void>;
}

export function MarketplacePanel({ client, account, onLog, runWriteFlow }: MarketplacePanelProps) {
  // Read state
  const [collection, setCollection] = useState("CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM");
  const [royaltyResult, setRoyaltyResult] = useState<any | null>(null);
  const [summaryResult, setSummaryResult] = useState<any | null>(null);
  const [readBusy, setReadBusy] = useState(false);

  // Write state (set_royalty)
  const [writeCollection, setWriteCollection] = useState(
    "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM"
  );
  const [recipient, setRecipient] = useState(account ?? "");
  const [bps, setBps] = useState("500"); // 500 bps = 5%

  const handleFetchRoyalty = async () => {
    if (!client) {
      onLog("Connect wallet or verify client initialization first.", "err");
      return;
    }
    const coll = collection.trim();
    if (!coll) {
      onLog("Please enter a collection contract ID.", "err");
      return;
    }

    setReadBusy(true);
    try {
      onLog(`Querying Royalty for collection ${coll.slice(0, 8)}...`, "info");
      const [royRes, sumRes] = await Promise.allSettled([
        client.get_royalty({ collection: coll }),
        client.get_settlement_summary({ collection: coll }),
      ]);

      if (royRes.status === "fulfilled" && royRes.value.result?.isOk?.()) {
        const roy = royRes.value.result.unwrap();
        setRoyaltyResult(roy);
        onLog(`Collection Royalty: ${roy.bps} bps (${(roy.bps / 100).toFixed(2)}%)`, "ok");
      } else {
        setRoyaltyResult(null);
      }

      if (sumRes.status === "fulfilled" && sumRes.value.result?.isOk?.()) {
        setSummaryResult(sumRes.value.result.unwrap());
      } else {
        setSummaryResult(null);
      }
    } catch (e) {
      onLog(`Royalty query error: ${(e as Error).message}`, "err");
    } finally {
      setReadBusy(false);
    }
  };

  const handleSetRoyalty = async () => {
    await runWriteFlow(
      "set_royalty",
      () =>
        validateSetRoyalty({
          collection: writeCollection,
          recipient: recipient || account || "",
          bps,
        }),
      async (c, data) => {
        return await c.set_royalty({
          collection: data.collection,
          recipient: data.recipient,
          bps: data.bps,
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
            Marketplace Royalties Read Flows
          </h3>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Query collection royalty rates, payout addresses, and cumulative sales volume summaries.
          </p>
        </div>

        <div className="rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-950/50">
          <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
            NFT Collection Contract ID:
          </label>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={collection}
              onChange={(e) => setCollection(e.target.value)}
              placeholder="C..."
              className="w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-900"
            />
            <button
              type="button"
              onClick={handleFetchRoyalty}
              disabled={readBusy}
              className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50 dark:bg-slate-700 dark:hover:bg-slate-600"
            >
              {readBusy ? "Reading..." : "Read Royalty"}
            </button>
          </div>

          {royaltyResult && (
            <div className="mt-3 space-y-2 rounded border border-slate-200 bg-white p-3 font-mono text-xs dark:border-slate-800 dark:bg-slate-900">
              <div className="flex justify-between items-center">
                <span className="text-slate-500">Royalty Status:</span>
                <span className="rounded bg-rose-50 px-2 py-0.5 font-semibold text-rose-700 dark:bg-rose-950 dark:text-rose-300">
                  {royaltyResult.status?.tag ?? "Active"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Basis Points (bps):</span>
                <span className="font-semibold">
                  {royaltyResult.bps} bps ({(royaltyResult.bps / 100).toFixed(2)}%)
                </span>
              </div>
              <div className="flex justify-between truncate">
                <span className="text-slate-500">Recipient:</span>
                <span className="truncate max-w-[200px]">{royaltyResult.recipient}</span>
              </div>
            </div>
          )}

          {summaryResult && (
            <div className="mt-3 space-y-1 rounded border border-slate-200 bg-slate-50 p-2.5 font-mono text-xs dark:border-slate-800 dark:bg-slate-950">
              <span className="font-semibold text-slate-700 dark:text-slate-300">Settlement Summary:</span>
              <div className="flex justify-between text-[11px] text-slate-600 dark:text-slate-400">
                <span>Gross Volume:</span>
                <span>{summaryResult.gross_volume?.toString()} stroops</span>
              </div>
              <div className="flex justify-between text-[11px] text-slate-600 dark:text-slate-400">
                <span>Royalties Paid:</span>
                <span>{summaryResult.royalties_paid?.toString()} stroops</span>
              </div>
              <div className="flex justify-between text-[11px] text-slate-600 dark:text-slate-400">
                <span>Total Sales:</span>
                <span>{summaryResult.sales?.toString()}</span>
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
              Write Flow: set_royalty
            </h3>
            <span className="rounded bg-rose-100 px-2 py-0.5 text-[10px] font-semibold text-rose-800 dark:bg-rose-950 dark:text-rose-300">
              Safe &amp; Validated
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Set or update royalty configuration for an NFT collection. Validates 0 ≤ bps ≤ 10,000 before signing.
          </p>
        </div>

        <div className="space-y-2.5">
          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Collection Contract ID:
            </label>
            <input
              type="text"
              value={writeCollection}
              onChange={(e) => setWriteCollection(e.target.value)}
              placeholder="C..."
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Royalty Recipient Address:
            </label>
            <input
              type="text"
              value={recipient}
              onChange={(e) => setRecipient(e.target.value)}
              placeholder={account ?? "G... or C..."}
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Royalty (Basis Points, 100 bps = 1%):
            </label>
            <input
              type="number"
              min="0"
              max="10000"
              value={bps}
              onChange={(e) => setBps(e.target.value)}
              placeholder="500 (5%)"
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
            <span className="text-[11px] text-slate-500">
              Current rate: {(Number(bps || "0") / 100).toFixed(2)}%
            </span>
          </div>

          <button
            type="button"
            onClick={handleSetRoyalty}
            className="mt-3 w-full rounded-lg bg-indigo-600 px-4 py-2 text-xs font-semibold text-white shadow hover:bg-indigo-500 transition"
          >
            Simulate, Sign &amp; Set Royalty
          </button>
        </div>
      </div>
    </div>
  );
}
