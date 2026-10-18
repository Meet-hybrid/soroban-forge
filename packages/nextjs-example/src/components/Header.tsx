"use client";

import { useState } from "react";
import { TESTNET_RPC_URL, TESTNET_PASSPHRASE } from "../lib/contracts/registry";

interface HeaderProps {
  account: string | null;
  busy: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
}

export function Header({ account, busy, onConnect, onDisconnect, onLog }: HeaderProps) {
  const [friendbotBusy, setFriendbotBusy] = useState(false);

  const fundFriendbot = async () => {
    if (!account) return;
    setFriendbotBusy(true);
    onLog(`Requesting testnet funds from Friendbot for ${account}...`, "info");
    try {
      const res = await fetch(`https://friendbot.stellar.org?addr=${account}`);
      const json = (await res.json()) as { hash?: string; detail?: string };
      if (res.ok && json.hash) {
        onLog(`Friendbot funded 10,000 testnet XLM — tx: ${json.hash}`, "ok");
      } else {
        onLog(`Friendbot: ${json.detail ?? "unexpected response"}`, "err");
      }
    } catch (e) {
      onLog(`Friendbot request failed: ${(e as Error).message}`, "err");
    } finally {
      setFriendbotBusy(false);
    }
  };

  const hasFreighter = typeof window !== "undefined" && Boolean(window.freighterApi);

  return (
    <header className="mb-6 space-y-4">
      {/* Top Banner */}
      <div className="flex flex-col gap-2 rounded-xl bg-gradient-to-r from-slate-900 to-indigo-950 p-6 text-white shadow-lg">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-2">
              <span className="rounded bg-indigo-500/30 px-2.5 py-0.5 text-xs font-semibold tracking-wider text-indigo-300 border border-indigo-400/30 uppercase">
                Soroban Forge
              </span>
              <span className="rounded bg-emerald-500/20 px-2 py-0.5 text-xs font-medium text-emerald-400 border border-emerald-500/30">
                Developer Console
              </span>
            </div>
            <h1 className="mt-2 text-2xl font-bold tracking-tight sm:text-3xl">
              Six-Contract Soroban Console
            </h1>
            <p className="mt-1 text-sm text-slate-300">
              Interactive safe read &amp; write operations across Escrow, Vesting, Multi-Sig, DAO, Subscriptions, and Marketplace contracts.
            </p>
          </div>

          {/* Wallet Action */}
          <div className="flex flex-col items-end gap-2">
            {account ? (
              <div className="flex flex-wrap items-center gap-2">
                <div className="rounded-lg bg-slate-800/80 px-3 py-1.5 border border-slate-700 text-xs font-mono text-emerald-400 flex items-center gap-2">
                  <span className="h-2 w-2 rounded-full bg-emerald-400 animate-pulse" />
                  {account.slice(0, 6)}...{account.slice(-4)}
                </div>
                <button
                  type="button"
                  onClick={fundFriendbot}
                  disabled={friendbotBusy}
                  className="rounded-lg bg-amber-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-amber-500 disabled:opacity-50 transition shadow"
                >
                  {friendbotBusy ? "Funding..." : "Friendbot (+XLM)"}
                </button>
                <button
                  type="button"
                  onClick={onDisconnect}
                  className="rounded-lg bg-slate-700 px-3 py-1.5 text-xs font-medium text-slate-200 hover:bg-slate-600 transition"
                >
                  Disconnect
                </button>
              </div>
            ) : (
              <div>
                <button
                  type="button"
                  onClick={onConnect}
                  disabled={busy}
                  className="rounded-lg bg-indigo-600 px-4 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition shadow"
                >
                  {busy ? "Connecting..." : "Connect Freighter"}
                </button>
              </div>
            )}
          </div>
        </div>

        {/* Testnet-only warning & Non-custodial banner */}
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-slate-800 pt-3 text-xs text-slate-400">
          <div className="flex items-center gap-2 text-amber-300 font-medium">
            <span>⚠️ Testnet Only:</span>
            <span className="text-slate-300">
              Non-custodial console for testing. No private keys stored. Testnet tokens have no monetary value.
            </span>
          </div>
          <div className="flex items-center gap-3 font-mono text-[11px] text-slate-400">
            <span>RPC: <code className="text-slate-200">{TESTNET_RPC_URL}</code></span>
            <span>Passphrase: <code className="text-slate-200">{TESTNET_PASSPHRASE}</code></span>
          </div>
        </div>
      </div>

      {!hasFreighter && typeof window !== "undefined" && !account && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200">
          <strong>Freighter Wallet Extension Not Detected:</strong> Please install Freighter for Stellar from{" "}
          <a
            href="https://www.freighter.app"
            target="_blank"
            rel="noreferrer"
            className="underline font-semibold"
          >
            freighter.app
          </a>
          , enable Testnet in settings, and reload the page.
        </div>
      )}
    </header>
  );
}
