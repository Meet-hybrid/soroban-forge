"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Networks } from "@stellar/stellar-sdk";
import { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit/sdk";
import { defaultModules } from "@creit.tech/stellar-wallets-kit/modules/utils";
import {
  Client,
  networks,
  type EscrowData,
} from "@soroban-forge/escrow-client";
import {
  createClientContractProbe,
  resolveDisplayDiagnosticsState,
  runDiagnosticsCheck,
  type DiagnosticsReport,
} from "../lib/diagnostics";

const RPC_URL = "https://soroban-testnet.stellar.org";
const NETWORK_PASSPHRASE = networks.testnet.networkPassphrase;
const CONTRACT_ID = networks.testnet.contractId;

// 7-decimal demo token (SAC "credit" asset) used by the repo's own testnet demo
// (see the README proof table at the repository root).
const DEFAULT_TOKEN =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

// 1 unit = 10^7 stroops (SEP-41 default for 7-decimal tokens such as XLM)
const DECIMALS = BigInt(7);
const SCALE = BigInt(10) ** DECIMALS;

type LogEntry = { id: number; time: string; text: string; kind: "info" | "ok" | "err" };

let logId = 0;

function ts(): string {
  return new Date().toLocaleTimeString();
}

export default function Home() {
  const [mounted, setMounted] = useState(false);
  const [account, setAccount] = useState<string | null>(null);
  const [client, setClient] = useState<Client | null>(null);
  const [busy, setBusy] = useState(false);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [friendbotBusy, setFriendbotBusy] = useState(false);

  // Contract health & network diagnostics state
  const [diagnosticsReport, setDiagnosticsReport] =
    useState<DiagnosticsReport | null>(null);
  const [diagnosticsBusy, setDiagnosticsBusy] = useState(false);
  const [diagnosticsNowMs, setDiagnosticsNowMs] = useState<number>(() =>
    Date.now()
  );

  // Create-escrow form
  const [buyer, setBuyer] = useState("");
  const [seller, setSeller] = useState("");
  const [arbiter, setArbiter] = useState("");
  const [token, setToken] = useState(DEFAULT_TOKEN);
  const [amount, setAmount] = useState("10000000"); // stroops
  const [timeoutSecs, setTimeoutSecs] = useState("3600");
  const [creating, setCreating] = useState(false);
  const [createdId, setCreatedId] = useState<string | null>(null);

  // Actions panel
  const [actionEscrowId, setActionEscrowId] = useState("");
  const [actionInFlight, setActionInFlight] = useState<string | null>(null);
  const [resolveFavor, setResolveFavor] = useState<"seller" | "buyer">(
    "seller"
  );

  // Read panel
  const [readEscrowId, setReadEscrowId] = useState("");
  const [escrow, setEscrow] = useState<EscrowData | null>(null);
  const [readBusy, setReadBusy] = useState(false);

  // List panel
  const [myIds, setMyIds] = useState<Array<bigint>>([]);
  const [listTotal, setListTotal] = useState<number | null>(null);
  const [listBusy, setListBusy] = useState(false);

  const readOnlyClient = useMemo(
    () =>
      new Client({
        rpcUrl: RPC_URL,
        networkPassphrase: NETWORK_PASSPHRASE,
        contractId: CONTRACT_ID,
      }),
    []
  );

  const pushLog = useCallback(
    (text: string, kind: LogEntry["kind"] = "info") => {
      setLog((prev) => [{ id: logId++, time: ts(), text, kind }, ...prev]);
    },
    []
  );

  const refreshDiagnostics = useCallback(
    async (activeClient?: Client | null) => {
      setDiagnosticsBusy(true);
      try {
        const targetClient = activeClient ?? client ?? readOnlyClient;
        const probeContract = createClientContractProbe(() => targetClient);
        const report = await runDiagnosticsCheck({
          rpcUrl: RPC_URL,
          expectedNetworkPassphrase: NETWORK_PASSPHRASE,
          contractId: CONTRACT_ID,
          probeContract,
        });
        setDiagnosticsReport(report);
        setDiagnosticsNowMs(Date.now());
      } finally {
        setDiagnosticsBusy(false);
      }
    },
    [client, readOnlyClient]
  );

  useEffect(() => {
    // Wallet modules require browser APIs, so initialize them after hydration.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setMounted(true);
    StellarWalletsKit.init({ modules: defaultModules() });
    StellarWalletsKit.setNetwork(Networks.TESTNET);
    void refreshDiagnostics(readOnlyClient);
  }, [readOnlyClient, refreshDiagnostics]);

  useEffect(() => {
    if (!diagnosticsReport) return;
    const timer = window.setInterval(() => {
      setDiagnosticsNowMs(Date.now());
    }, 15_000);
    return () => window.clearInterval(timer);
  }, [diagnosticsReport]);

  const diagnosticsView = useMemo(
    () =>
      resolveDisplayDiagnosticsState(
        diagnosticsReport,
        diagnosticsBusy,
        diagnosticsNowMs
      ),
    [diagnosticsReport, diagnosticsBusy, diagnosticsNowMs]
  );

  const listMine = useCallback(
    async (c: Client, addr: string) => {
      setListBusy(true);
      try {
        const { result } = await c.escrows_for_participant({
          participant: addr,
          cursor: 0,
          limit: 10,
        });
        setMyIds(result.ids);
        setListTotal(result.total);
        pushLog(
          `escrows_for_participant(${addr.slice(0, 6)}…): ${result.ids.length} ids (total ${result.total})`,
          "ok"
        );
      } catch (e) {
        pushLog(`List escrows failed: ${(e as Error).message}`, "err");
      } finally {
        setListBusy(false);
      }
    },
    [pushLog]
  );

  const connect = useCallback(async () => {
    setBusy(true);
    try {
      if (!mounted) {
        throw new Error("Wallet connectors are still loading; try again shortly.");
      }

      const { address: pubkey } = await StellarWalletsKit.authModal();
      setAccount(pubkey);
      // Keep the visible buyer value and the transaction payload in sync.
      setBuyer((current) => current || pubkey);
      const c = new Client({
        rpcUrl: RPC_URL,
        networkPassphrase: NETWORK_PASSPHRASE,
        contractId: CONTRACT_ID,
        publicKey: pubkey,
        signTransaction: async (
          xdr: string,
          opts?: { networkPassphrase?: string; address?: string }
        ) => {
          const result = await StellarWalletsKit.signTransaction(xdr, {
            networkPassphrase: opts?.networkPassphrase ?? NETWORK_PASSPHRASE,
            address: opts?.address ?? pubkey,
          });
          if (!result.signedTxXdr) {
            throw new Error("The selected wallet did not return a signed transaction.");
          }
          return { signedTxXdr: result.signedTxXdr };
        },
      });
      setClient(c);
      pushLog(
        `Connected to ${pubkey.slice(0, 6)}…${pubkey.slice(-4)} (testnet)`,
        "ok"
      );
      void listMine(c, pubkey);
    } catch (e) {
      pushLog(`Wallet connect failed: ${(e as Error).message}`, "err");
    } finally {
      setBusy(false);
    }
  }, [listMine, mounted, pushLog]);

  const disconnect = useCallback(() => {
    void StellarWalletsKit.disconnect();
    setAccount(null);
    setClient(null);
    setEscrow(null);
    setMyIds([]);
    setListTotal(null);
    setCreatedId(null);
    pushLog("Disconnected. Wallet state cleared.", "info");
  }, [pushLog]);

  const fundFriendbot = useCallback(async () => {
    if (!account) return;
    setFriendbotBusy(true);
    pushLog(`Requesting testnet funds from friendbot for ${account}…`, "info");
    try {
      const res = await fetch(`https://friendbot.stellar.org?addr=${account}`);
      const json = (await res.json()) as {
        hash?: string;
        detail?: string;
      };
      if (res.ok && json.hash) {
        pushLog(
          `Friendbot funded the account — tx hash ${json.hash}`,
          "ok"
        );
      } else {
        pushLog(`Friendbot: ${json.detail ?? "unexpected response"}`, "err");
      }
    } catch (e) {
      pushLog(`Friendbot request failed: ${(e as Error).message}`, "err");
    } finally {
      setFriendbotBusy(false);
    }
  }, [account, pushLog]);

  const createEscrow = useCallback(async () => {
    if (!client) return;
    setCreating(true);
    try {
      const amountBig = BigInt(amount || "0");
      if (amountBig <= BigInt(0)) {
        pushLog("Amount must be a positive integer number of stroops.", "err");
        return;
      }
      const timeoutBig = BigInt(timeoutSecs || "0");
      const buyerAddress = buyer.trim() || account?.trim() || "";
      const sellerAddress = seller.trim();
      const arbiterAddress = arbiter.trim();
      const tokenAddress = token.trim();
      if (!buyerAddress || !sellerAddress || !arbiterAddress || !tokenAddress) {
        pushLog(
          "Enter complete buyer, seller, arbiter, and token addresses before creating the escrow.",
          "err"
        );
        return;
      }
      const tx = await client.create_escrow(
        {
          buyer: buyerAddress,
          seller: sellerAddress,
          arbiter: arbiterAddress,
          token: tokenAddress,
          amount: amountBig,
          timeout: timeoutBig,
        },
        { timeoutInSeconds: 60 }
      );
      pushLog("create_escrow simulated — requesting signature…", "info");
      const sent = await tx.signAndSend();
      if (sent.sendTransactionResponse) {
        pushLog(
          `Submitted — tx hash ${sent.sendTransactionResponse.hash}`,
          "ok"
        );
      }
      const escrowId = sent.result.unwrap();
      setCreatedId(escrowId.toString());
      setActionEscrowId(escrowId.toString());
      setReadEscrowId(escrowId.toString());
      pushLog(`Escrow created — id ${escrowId.toString()} (Pending)`, "ok");
    } catch (e) {
      pushLog(`Create escrow failed: ${(e as Error).message}`, "err");
    } finally {
      setCreating(false);
    }
  }, [
    account,
    client,
    buyer,
    seller,
    arbiter,
    token,
    amount,
    timeoutSecs,
    pushLog,
  ]);

  const sendAction = useCallback(
    async (name: string, invoke: () => Promise<void>) => {
      setActionInFlight(name);
      try {
        pushLog(`${name} — simulating…`, "info");
        await invoke();
      } catch (e) {
        pushLog(`${name} failed: ${(e as Error).message}`, "err");
      } finally {
        setActionInFlight(null);
      }
    },
    [pushLog]
  );

  const runAction = useCallback(
    async (
      name: string,
      build: (c: Client, id: bigint) => Promise<{
        signAndSend: () => Promise<{ sendTransactionResponse?: { hash: string } }>;
      }>
    ) => {
      if (!client) return;
      const id = BigInt(actionEscrowId || "0");
      if (id <= BigInt(0)) {
        pushLog(`Enter a valid escrow id for ${name}.`, "err");
        return;
      }
      await sendAction(name, async () => {
        const tx = await build(client, id);
        pushLog(`${name} — requesting signature…`, "info");
        const sent = await tx.signAndSend();
        pushLog(
          sent.sendTransactionResponse?.hash
            ? `${name} submitted — tx hash ${sent.sendTransactionResponse.hash}`
            : `${name} succeeded.`,
          "ok"
        );
      });
    },
    [client, actionEscrowId, pushLog, sendAction]
  );

  const fetchEscrow = useCallback(
    async (id?: string) => {
      if (!client) return;
      const raw = id ?? readEscrowId;
      const escrowId = BigInt(raw || "0");
      if (escrowId <= BigInt(0)) {
        pushLog("Enter a valid escrow id to fetch.", "err");
        return;
      }
      setReadBusy(true);
      try {
        const { result } = await client.get_escrow({ escrow_id: escrowId });
        if (result.isOk()) {
          const data = result.unwrap();
          setEscrow(data);
          pushLog(
            `Fetched escrow ${escrowId.toString()} — ${data.status.tag}`,
            "ok"
          );
        } else {
          pushLog(
            `get_escrow returned error: ${result.unwrapErr().message}`,
            "err"
          );
        }
      } catch (e) {
        pushLog(`get_escrow failed: ${(e as Error).message}`, "err");
      } finally {
        setReadBusy(false);
      }
    },
    [client, readEscrowId, pushLog]
  );

  const connectProps = useMemo(
    () => ({
      disabled: busy,
      onClick: connect,
    }),
    [busy, connect]
  );

  return (
    <main className="mx-auto max-w-5xl p-6">
      <div className="mb-3 flex items-center justify-between gap-4">
        <h1 className="text-3xl font-bold">Soroban Forge — Escrow Testnet Demo</h1>
        <a href="/issues" className="shrink-0 rounded border border-gray-300 px-3 py-2 text-sm text-gray-700 hover:bg-gray-50">
          Issue board
        </a>
      </div>
      <p className="text-sm text-gray-500 mb-6">
        Contract{" "}
        <code className="break-all">{CONTRACT_ID}</code> ·{" "}
        {NETWORK_PASSPHRASE}
      </p>

      {/* Contract Health & Network Diagnostics */}
      <section className="rounded-xl border border-gray-300 bg-gray-50/60 p-4 mb-6">
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-sm font-semibold uppercase tracking-wide text-gray-700">
              Contract Health &amp; Network Diagnostics
            </h2>
            <span
              className={
                "rounded-full px-2.5 py-0.5 text-xs font-medium " +
                (diagnosticsView.displayState === "healthy"
                  ? "bg-green-100 text-green-800"
                  : diagnosticsView.displayState === "loading" ||
                      diagnosticsView.displayState === "idle"
                    ? "bg-blue-100 text-blue-800"
                    : diagnosticsView.displayState === "stale"
                      ? "bg-amber-100 text-amber-800"
                      : "bg-red-100 text-red-800")
              }
            >
              {diagnosticsView.displayState === "loading"
                ? "Checking…"
                : diagnosticsView.displayState}
            </span>
          </div>
          <button
            type="button"
            onClick={() => void refreshDiagnostics()}
            disabled={diagnosticsBusy}
            className="rounded border border-gray-300 bg-white px-3 py-1 text-xs font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50"
          >
            {diagnosticsBusy ? "Checking…" : "Run diagnostics"}
          </button>
        </div>

        <p className="mb-3 text-xs text-gray-700">
          {diagnosticsReport
            ? diagnosticsReport.summary
            : "Running initial read-only connectivity and contract health check…"}
        </p>

        <dl className="grid grid-cols-1 gap-2 text-xs sm:grid-cols-2 lg:grid-cols-4">
          <div className="rounded border border-gray-200 bg-white p-2">
            <dt className="font-medium text-gray-500">Network</dt>
            <dd className="mt-0.5 font-mono break-all text-gray-900">
              {NETWORK_PASSPHRASE}
            </dd>
            <dd className="mt-0.5 text-gray-500">
              {diagnosticsReport
                ? diagnosticsReport.networkMatches
                  ? "Passphrase matched"
                  : `Mismatch (${diagnosticsReport.detectedNetworkPassphrase ?? "unknown"})`
                : "Pending check"}
            </dd>
          </div>
          <div className="rounded border border-gray-200 bg-white p-2">
            <dt className="font-medium text-gray-500">RPC Endpoint</dt>
            <dd className="mt-0.5 font-mono break-all text-gray-900">
              {diagnosticsReport?.rpcUrl ?? RPC_URL}
            </dd>
            <dd className="mt-0.5 text-gray-500">
              {diagnosticsReport
                ? diagnosticsReport.rpcReachable
                  ? "Reachable"
                  : "Unavailable"
                : "Pending check"}
            </dd>
          </div>
          <div className="rounded border border-gray-200 bg-white p-2">
            <dt className="font-medium text-gray-500">Contract</dt>
            <dd className="mt-0.5 font-mono break-all text-gray-900">
              {CONTRACT_ID.slice(0, 8)}…{CONTRACT_ID.slice(-6)}
            </dd>
            <dd className="mt-0.5 text-gray-500">
              {diagnosticsReport
                ? diagnosticsReport.contractReadable
                  ? "Readable on-chain"
                  : "Unreadable / missing"
                : "Pending check"}
            </dd>
          </div>
          <div className="rounded border border-gray-200 bg-white p-2">
            <dt className="font-medium text-gray-500">Latest Ledger</dt>
            <dd className="mt-0.5 font-mono text-gray-900">
              {diagnosticsReport?.latestLedger !== null &&
              diagnosticsReport?.latestLedger !== undefined
                ? `#${diagnosticsReport.latestLedger}`
                : "—"}
            </dd>
            <dd className="mt-0.5 text-gray-500">
              {diagnosticsReport?.protocolVersion
                ? `Protocol v${diagnosticsReport.protocolVersion}`
                : diagnosticsView.isStale
                  ? "Stale — click Run diagnostics"
                  : "Read-only probe"}
            </dd>
          </div>
        </dl>

        {diagnosticsReport && diagnosticsReport.nextSteps.length > 0 && (
          <div className="mt-3 rounded border border-red-200 bg-red-50 p-2.5 text-xs text-red-900">
            <p className="font-semibold mb-1">Actionable next steps:</p>
            <ul className="list-disc pl-4 space-y-0.5">
              {diagnosticsReport.nextSteps.map((step, idx) => (
                <li key={idx}>{step}</li>
              ))}
            </ul>
          </div>
        )}
      </section>

      {/* Wallet */}
      <section className="rounded-xl border border-gray-300 p-4 mb-6">
        <h2 className="text-lg font-semibold mb-2">1 · Wallet</h2>
        {account ? (
          <div className="flex flex-wrap items-center gap-3">
            <span className="font-mono text-sm break-all">{account}</span>
            <span className="rounded-full bg-green-100 px-2 py-0.5 text-xs text-green-800">
              connected
            </span>
            <button
              onClick={fundFriendbot}
              disabled={friendbotBusy}
              className="rounded bg-yellow-500 px-3 py-1.5 text-sm text-white hover:bg-yellow-600 disabled:opacity-50"
            >
              {friendbotBusy ? "Funding…" : "Fund with Friendbot (testnet XLM)"}
            </button>
            <button
              onClick={disconnect}
              className="rounded bg-gray-200 px-3 py-1.5 text-sm hover:bg-gray-300"
            >
              Disconnect
            </button>
          </div>
        ) : (
          <div>
            <button
              {...(mounted ? connectProps : { disabled: true })}
              className="rounded bg-blue-600 px-4 py-2 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
            >
              {busy ? "Connecting…" : "Connect wallet"}
            </button>
            {!mounted && (
              <p className="mt-2 text-sm text-amber-700">
                Loading Stellar wallet connectors…
              </p>
            )}
          </div>
        )}
      </section>

      <div className="grid gap-6 lg:grid-cols-2">
        {/* Create */}
        <section className="rounded-xl border border-gray-300 p-4">
          <h2 className="text-lg font-semibold mb-3">2 · Create escrow</h2>
          <div className="space-y-2">
            <label className="block text-sm">
              Buyer (G…)
              <input
                value={buyer}
                onChange={(e) => setBuyer(e.target.value)}
                placeholder={account ?? "connected account address"}
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
              />
            </label>
            <label className="block text-sm">
              Seller (G…)
              <input
                value={seller}
                onChange={(e) => setSeller(e.target.value)}
                placeholder="seller public key"
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
              />
            </label>
            <label className="block text-sm">
              Arbiter (G…)
              <input
                value={arbiter}
                onChange={(e) => setArbiter(e.target.value)}
                placeholder="arbiter public key"
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
              />
            </label>
            <label className="block text-sm">
              Token (C…)
              <input
                value={token}
                onChange={(e) => setToken(e.target.value)}
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
              />
              <span className="text-xs text-gray-500">
                Default: repo&apos;s testnet demo SAC &quot;credit&quot; token
                (7 decimals). Replace with your own testnet token contract id.
              </span>
            </label>
            <div className="grid grid-cols-2 gap-2">
              <label className="block text-sm">
                Amount (stroops)
                <input
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  placeholder="e.g. 10000000 = 1 unit"
                  className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
                />
              </label>
              <label className="block text-sm">
                Timeout (seconds)
                <input
                  value={timeoutSecs}
                  onChange={(e) => setTimeoutSecs(e.target.value)}
                  placeholder="3600"
                  className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
                />
              </label>
            </div>
            <button
              onClick={createEscrow}
              disabled={!client || creating}
              className="mt-1 w-full rounded bg-blue-600 px-4 py-2 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
            >
              {creating
                ? "Creating… (simulate + sign + send)"
                : client
                  ? "Create escrow"
                  : "Connect wallet first"}
            </button>
            {createdId && (
              <p className="text-sm text-green-700">
                Last created escrow:{" "}
                <button
                  onClick={() => {
                    setReadEscrowId(createdId);
                    void fetchEscrow(createdId);
                  }}
                  className="font-mono underline"
                >
                  {createdId}
                </button>{" "}
                (Pending)
              </p>
            )}
          </div>
        </section>

        {/* Actions */}
        <section className="rounded-xl border border-gray-300 p-4">
          <h2 className="text-lg font-semibold mb-3">3 · Actions</h2>
          <label className="block text-sm mb-3">
            Escrow id
            <input
              value={actionEscrowId}
              onChange={(e) => setActionEscrowId(e.target.value)}
              placeholder="escrow id from create step"
              className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
            />
          </label>
          <div className="flex flex-wrap gap-2">
            {(["Deposit", "Release", "Refund", "Cancel", "Dispute"] as const).map(
              (name) => (
                <button
                  key={name}
                  disabled={!client || actionInFlight !== null}
                  onClick={() =>
                    void runAction(name.toLowerCase(), (c, id) =>
                      name === "Dispute"
                        ? c.dispute({ escrow_id: id, claimant: account ?? "" })
                        : c[
                            name.toLowerCase() as
                              | "deposit"
                              | "release"
                              | "refund"
                              | "cancel"
                          ]({ escrow_id: id })
                    )
                  }
                  className="rounded border border-blue-600 px-3 py-1.5 text-sm text-blue-600 hover:bg-blue-50 disabled:opacity-50"
                >
                  {actionInFlight === name.toLowerCase()
                    ? "Busy…"
                    : name}
                </button>
              )
            )}
          </div>
          <div className="mt-4 rounded border border-gray-200 p-3">
            <p className="text-sm font-medium mb-2">Resolve dispute (arbiter only)</p>
            <div className="flex items-center gap-4">
              <label className="flex items-center gap-1 text-sm">
                <input
                  type="radio"
                  name="resolve-favor"
                  checked={resolveFavor === "seller"}
                  onChange={() => setResolveFavor("seller")}
                />
                In favor of seller
              </label>
              <label className="flex items-center gap-1 text-sm">
                <input
                  type="radio"
                  name="resolve-favor"
                  checked={resolveFavor === "buyer"}
                  onChange={() => setResolveFavor("buyer")}
                />
                In favor of buyer
              </label>
              <button
                disabled={!client || actionInFlight !== null}
                onClick={() =>
                  void runAction("resolve", (c, id) =>
                    c.resolve({
                      escrow_id: id,
                      in_favor_of_seller: resolveFavor === "seller",
                    })
                  )
                }
                className="rounded bg-purple-600 px-3 py-1.5 text-sm text-white hover:bg-purple-700 disabled:opacity-50"
              >
                {actionInFlight === "resolve" ? "Busy…" : "Resolve"}
              </button>
            </div>
          </div>
        </section>
      </div>

      <div className="grid gap-6 lg:grid-cols-2 mt-6">
        {/* Read */}
        <section className="rounded-xl border border-gray-300 p-4">
          <h2 className="text-lg font-semibold mb-3">4 · Read escrow</h2>
          <div className="flex gap-2 mb-3">
            <input
              value={readEscrowId}
              onChange={(e) => setReadEscrowId(e.target.value)}
              placeholder="escrow id"
              className="flex-1 rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
            />
            <button
              onClick={() => void fetchEscrow()}
              disabled={!client || readBusy}
              className="rounded bg-gray-800 px-4 py-1.5 text-sm text-white hover:bg-gray-900 disabled:opacity-50"
            >
              {readBusy ? "Fetching…" : "Fetch"}
            </button>
          </div>
          {escrow && (
            <dl className="grid grid-cols-1 gap-x-4 gap-y-1 text-sm sm:grid-cols-2">
              <dt className="font-medium text-gray-500">Escrow id</dt>
              <dd className="font-mono">{escrow.escrow_id.toString()}</dd>
              <dt className="font-medium text-gray-500">Status</dt>
              <dd>
                <span
                  className={
                    "rounded-full px-2 py-0.5 text-xs " +
                    (escrow.status.tag === "Completed" ||
                    escrow.status.tag === "Refunded" ||
                    escrow.status.tag === "Cancelled"
                      ? "bg-gray-200 text-gray-700"
                      : escrow.status.tag === "Disputed"
                        ? "bg-red-100 text-red-700"
                        : escrow.status.tag === "Funded"
                          ? "bg-blue-100 text-blue-700"
                          : "bg-yellow-100 text-yellow-700")
                  }
                >
                  {escrow.status.tag}
                </span>
              </dd>
              <dt className="font-medium text-gray-500">Buyer</dt>
              <dd className="font-mono break-all">{escrow.buyer}</dd>
              <dt className="font-medium text-gray-500">Seller</dt>
              <dd className="font-mono break-all">{escrow.seller}</dd>
              <dt className="font-medium text-gray-500">Arbiter</dt>
              <dd className="font-mono break-all">{escrow.arbiter}</dd>
              <dt className="font-medium text-gray-500">Token</dt>
              <dd className="font-mono break-all">{escrow.token}</dd>
              <dt className="font-medium text-gray-500">Amount</dt>
              <dd className="font-mono">
                {formatAmount(escrow.amount)} ({escrow.amount.toString()} stroops)
              </dd>
              <dt className="font-medium text-gray-500">Created at</dt>
              <dd className="font-mono">
                {escrow.created_at.toString()} (
                {new Date(Number(escrow.created_at) * 1000).toLocaleString()})
              </dd>
              <dt className="font-medium text-gray-500">Timeout</dt>
              <dd className="font-mono">{escrow.timeout.toString()} s</dd>
            </dl>
          )}
        </section>

        {/* List */}
        <section className="rounded-xl border border-gray-300 p-4">
          <h2 className="text-lg font-semibold mb-3">5 · My escrows</h2>
          <div className="flex items-center gap-2">
            <button
              onClick={() => client && account && void listMine(client, account)}
              disabled={!client || listBusy}
              className="rounded bg-gray-800 px-4 py-1.5 text-sm text-white hover:bg-gray-900 disabled:opacity-50"
            >
              {listBusy ? "Loading…" : "Refresh"}
            </button>
            {listTotal !== null && (
              <span className="text-sm text-gray-500">
                {listTotal} escrow{listTotal === 1 ? "" : "s"} involving your
                account
              </span>
            )}
          </div>
          {myIds.length === 0 ? (
            <p className="mt-2 text-sm text-gray-500">
              No escrows found for the connected account yet.
            </p>
          ) : (
            <div className="mt-2 flex flex-wrap gap-2">
              {myIds.map((id) => (
                <button
                  key={id.toString()}
                  onClick={() => {
                    const s = id.toString();
                    setReadEscrowId(s);
                    void fetchEscrow(s);
                  }}
                  className="rounded-full border border-blue-600 px-3 py-1 font-mono text-sm text-blue-600 hover:bg-blue-50"
                >
                  {id.toString()}
                </button>
              ))}
            </div>
          )}
        </section>
      </div>

      {/* Log feed */}
      <section className="mt-6">
        <h2 className="text-lg font-semibold mb-2">Transaction log</h2>
        <div className="max-h-72 overflow-y-auto rounded-lg bg-gray-950 p-3 font-mono text-xs">
          {log.length === 0 ? (
            <p className="text-gray-500">No activity yet…</p>
          ) : (
            log.map((entry) => (
              <p
                key={entry.id}
                className={
                  entry.kind === "err"
                    ? "mb-1 break-all text-red-400"
                    : entry.kind === "ok"
                      ? "mb-1 break-all text-green-400"
                      : "mb-1 break-all text-gray-300"
                }
              >
                <span className="text-gray-500">[{entry.time}] </span>
                {entry.text}
              </p>
            ))
          )}
        </div>
      </section>
    </main>
  );
}

function formatAmount(stroops: bigint): string {
  const whole = stroops / SCALE;
  const frac = (stroops % SCALE).toString().padStart(Number(DECIMALS), "0");
  return `${whole.toString()}.${frac}`;
}
