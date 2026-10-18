"use client";

import { useState } from "react";
import { DEFAULT_TOKEN_ID } from "../../lib/contracts/registry";
import { validateSubscribe } from "../../lib/validation";

interface SubscriptionPanelProps {
  client: any;
  account: string | null;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
  runWriteFlow: (
    actionName: string,
    validate: () => any,
    buildTx: (c: any, data: any) => Promise<any>
  ) => Promise<void>;
}

export function SubscriptionPanel({ client, account, onLog, runWriteFlow }: SubscriptionPanelProps) {
  // Read state
  const [subId, setSubId] = useState("1");
  const [subscription, setSubscription] = useState<any | null>(null);
  const [subCount, setSubCount] = useState<string | null>(null);
  const [readBusy, setReadBusy] = useState(false);

  // Write state (subscribe)
  const [subscriber, setSubscriber] = useState(account ?? "");
  const [provider, setProvider] = useState("");
  const [token, setToken] = useState(DEFAULT_TOKEN_ID);
  const [amount, setAmount] = useState("10000000"); // 1 token per period
  const [periodSecs, setPeriodSecs] = useState("2592000"); // 30 days

  const handleFetchSubscription = async () => {
    if (!client) {
      onLog("Connect wallet or verify client initialization first.", "err");
      return;
    }
    const id = BigInt(subId.trim() || "0");
    if (id <= BigInt(0)) {
      onLog("Please enter a valid Subscription ID (> 0).", "err");
      return;
    }

    setReadBusy(true);
    try {
      onLog(`Querying Subscription #${id.toString()}...`, "info");
      const [subRes, countRes] = await Promise.allSettled([
        client.get_subscription({ subscription_id: id }),
        client.get_subscription_count(),
      ]);

      if (subRes.status === "fulfilled" && subRes.value.result?.isOk?.()) {
        const sub = subRes.value.result.unwrap();
        setSubscription(sub);
        onLog(`Fetched Subscription #${id.toString()} — Status: ${sub.status?.tag ?? "Unknown"}`, "ok");
      } else {
        setSubscription(null);
      }

      if (countRes.status === "fulfilled") {
        setSubCount(countRes.value.result?.toString() ?? countRes.value.toString());
      }
    } catch (e) {
      onLog(`Subscription read error: ${(e as Error).message}`, "err");
    } finally {
      setReadBusy(false);
    }
  };

  const handleCreateSubscription = async () => {
    await runWriteFlow(
      "subscribe",
      () =>
        validateSubscribe({
          subscriber: subscriber || account || "",
          provider,
          token,
          amount,
          periodSecs,
        }),
      async (c, data) => {
        return await c.subscribe({
          subscriber: data.subscriber,
          provider: data.provider,
          token: data.token,
          amount: data.amount,
          period: data.period,
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
            Subscription Read Flows
          </h3>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Query recurring payment agreements, provider authorization, and billing status.
          </p>
        </div>

        <div className="rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-950/50">
          <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
            Subscription ID:
          </label>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={subId}
              onChange={(e) => setSubId(e.target.value)}
              placeholder="e.g. 1"
              className="w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-900"
            />
            <button
              type="button"
              onClick={handleFetchSubscription}
              disabled={readBusy}
              className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50 dark:bg-slate-700 dark:hover:bg-slate-600"
            >
              {readBusy ? "Reading..." : "Read Subscription"}
            </button>
          </div>

          {subscription && (
            <div className="mt-3 space-y-2 rounded border border-slate-200 bg-white p-3 font-mono text-xs dark:border-slate-800 dark:bg-slate-900">
              <div className="flex justify-between items-center">
                <span className="text-slate-500">Subscription Status:</span>
                <span className="rounded bg-teal-50 px-2 py-0.5 font-semibold text-teal-700 dark:bg-teal-950 dark:text-teal-300">
                  {subscription.status?.tag ?? "Unknown"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Amount / Period:</span>
                <span>{subscription.amount?.toString()} stroops</span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Billing Period:</span>
                <span>{subscription.period?.toString()} seconds</span>
              </div>
              <div className="flex justify-between truncate">
                <span className="text-slate-500">Subscriber:</span>
                <span className="truncate max-w-[200px]">{subscription.subscriber}</span>
              </div>
              <div className="flex justify-between truncate">
                <span className="text-slate-500">Provider:</span>
                <span className="truncate max-w-[200px]">{subscription.provider}</span>
              </div>
            </div>
          )}

          {subCount !== null && (
            <div className="mt-2 text-[11px] text-slate-500">
              Total Subscriptions Created: <strong className="text-slate-700 dark:text-slate-300">{subCount}</strong>
            </div>
          )}
        </div>
      </div>

      {/* Write Panel */}
      <div className="space-y-4 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
        <div>
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-base text-slate-900 dark:text-slate-100">
              Write Flow: subscribe
            </h3>
            <span className="rounded bg-teal-100 px-2 py-0.5 text-[10px] font-semibold text-teal-800 dark:bg-teal-950 dark:text-teal-300">
              Safe &amp; Validated
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Sets up recurring billing authorization. Validates distinct subscriber/provider addresses and positive terms.
          </p>
        </div>

        <div className="space-y-2.5">
          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Subscriber Address:
            </label>
            <input
              type="text"
              value={subscriber}
              onChange={(e) => setSubscriber(e.target.value)}
              placeholder={account ?? "G..."}
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Provider Address:
            </label>
            <input
              type="text"
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
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
                Amount per Period:
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
                Period (seconds):
              </label>
              <input
                type="text"
                value={periodSecs}
                onChange={(e) => setPeriodSecs(e.target.value)}
                placeholder="2592000"
                className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
              />
            </div>
          </div>

          <button
            type="button"
            onClick={handleCreateSubscription}
            className="mt-3 w-full rounded-lg bg-indigo-600 px-4 py-2 text-xs font-semibold text-white shadow hover:bg-indigo-500 transition"
          >
            Simulate, Sign &amp; Subscribe
          </button>
        </div>
      </div>
    </div>
  );
}
