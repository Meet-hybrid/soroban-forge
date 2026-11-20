"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Networks } from "@stellar/stellar-sdk";
import { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit/sdk";
import { defaultModules } from "@creit.tech/stellar-wallets-kit/modules/utils";
import {
  Client,
  networks,
  type EscrowData,
  ForgeError,
} from "@soroban-forge/escrow-client";

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

// Minimum and maximum review window bounds for the review-period escrow flow.
const MIN_REVIEW_PERIOD = 60; // shortest useful window (1 minute)
const MAX_REVIEW_PERIOD = 30 * 24 * 60 * 60; // 30 days
const DEFAULT_REVIEW_PERIOD = 3 * 24 * 60 * 60; // 3 days

export default function Home() {
  const [mounted, setMounted] = useState(false);
  const [account, setAccount] = useState<string | null>(null);
  const [client, setClient] = useState<Client | null>(null);
  const [busy, setBusy] = useState(false);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [friendbotBusy, setFriendbotBusy] = useState(false);

  // Create-escrow form
  const [buyer, setBuyer] = useState("");
  const [seller, setSeller] = useState("");
  const [arbiter, setArbiter] = useState("");
  const [token, setToken] = useState(DEFAULT_TOKEN);
  const [amount, setAmount] = useState("10000000"); // stroops
  const [timeoutSecs, setTimeoutSecs] = useState("3600");
  const [reviewPeriodSecs, setReviewPeriodSecs] = useState(
    DEFAULT_REVIEW_PERIOD.toString()
  );
  const [useReviewWindow, setUseReviewWindow] = useState(false);
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

  // Review-window state (for escrows created with review or when fetching release requests)
  const [releaseRequest, setReleaseRequest] = useState<{
    amount: bigint;
    release_at: bigint;
    requested_at: bigint;
  } | null>(null);
  const [reviewWindowEndsAt, setReviewWindowEndsAt] = useState<bigint | null>(null);
  const [reviewWindowSecondsRemaining, setReviewWindowSecondsRemaining] =
    useState<number | null>(null);

  // List panel
  const [myIds, setMyIds] = useState<Array<bigint>>([]);
  const [listTotal, setListTotal] = useState<number | null>(null);
  const [listBusy, setListBusy] = useState(false);

  const pushLog = useCallback(
    (text: string, kind: LogEntry["kind"] = "info") => {
      setLog((prev) => [{ id: logId++, time: ts(), text, kind }, ...prev]);
    },
    []
  );  // Initialize the wallet kit after hydration.
  useEffect(() => {
    if (!mounted) return;
    StellarWalletsKit.init({ modules: defaultModules() });
    StellarWalletsKit.setNetwork(Networks.TESTNET);
  }, [mounted]);

  // Refresh the review-window countdown every second while one is active.
  useEffect(() => {
    if (reviewWindowEndsAt === null) return;
    const tick = () => {
      const now = BigInt(Math.floor(Date.now() / 1000));
      const remaining = Number(reviewWindowEndsAt - now);
      setReviewWindowSecondsRemaining(remaining > 0 ? remaining : 0);
      if (remaining <= 0) {
        setReviewWindowEndsAt(null);
        setReviewWindowSecondsRemaining(null);
      }
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [reviewWindowEndsAt]);

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

      // Read the wallet's current network for the safe-write guard.
      try {
        const { network } = await StellarWalletsKit.getNetwork();
        setWalletNetwork(network ?? "testnet");
        setWalletPassphrase(networks.testnet.networkPassphrase);
      } catch {
        setWalletNetwork("testnet");
        setWalletPassphrase(networks.testnet.networkPassphrase);
      }

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
    setReleaseRequest(null);
    setReviewWindowEndsAt(null);
    setReviewWindowSecondsRemaining(null);
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

  /**
   * Safe-write guard: refuse to send a write transaction while the wallet's
   * network or account does not match the page's configured network / connected
   * account. Returns a human-readable reason when the guard fails, or null when
   * the write is safe to proceed.
   *
   * Uses the StellarWalletsKit network and address APIs that the page already
   * depends on; does not store keys.
   */
  const safeWriteGuard = useCallback(
    async (actionName: string): Promise<string | null> => {
      if (!account) {
        return "Connect a wallet first.";
      }
      // Check the wallet's current network matches the page's expected network.
      let walletNetwork: string | undefined;
      try {
        const { network } = await StellarWalletsKit.getNetwork();
        walletNetwork = network;
      } catch {
        // If we can't read the network, proceed with a warning.
        pushLog(
          `Safe-write guard: could not read wallet network for ${actionName}; assuming testnet.`,
          "err"
        );
      }
      if (walletNetwork && walletNetwork !== "testnet") {
        return `Wallet is on network "${walletNetwork}" but this demo targets testnet. Switch the wallet to Stellar testnet.`;
      }
      // Check the wallet's current address matches the connected account.
      try {
        const { address: walletAddr } = await StellarWalletsKit.getAddress();
        if (walletAddr && walletAddr !== account) {
          pushLog(
            `Safe-write guard: wallet address ${walletAddr.slice(0, 6)}… does not match the connected account ${account.slice(0, 6)}… for ${actionName}.`,
            "err"
          );
          return `Wallet address does not match the connected account. Disconnect and reconnect.`;
        }
      } catch {
        pushLog(
          `Safe-write guard: could not read wallet address for ${actionName}.`,
          "err"
        );
      }
      return null;
    },
    [account, pushLog]
  );

  /**
   * Which role-related actions the connected account may perform on the given
   * escrow, given the escrow's current status and (optionally) an active
   * review window.
   *
   * Kept in one small function so the action list in the UI stays a pure
   * presentation of this decision.
   */
  const allowedActions = useCallback(
    (
      esc: EscrowData | null,
      hasReviewWindow: boolean,
      isPendingRelease: boolean
    ): Array<{ name: string; role: string }> => {
      if (!esc || !account) return [];

      const s = esc.status.tag;
      const isBuyer = esc.buyer === account;
      const isSeller = esc.seller === account;
      const isArbiter = esc.arbiter === account;
      const actions: Array<{ name: string; role: string }> = [];

      if (s === "Pending") {
        // Only the buyer may deposit into a pending escrow.
        if (isBuyer) actions.push({ name: "Deposit", role: "buyer" });
      } else if (s === "Funded") {
        if (isSeller && !hasReviewWindow && !isPendingRelease) {
          actions.push({ name: "Request release", role: "seller" });
        }
        if (isBuyer && hasReviewWindow && isPendingRelease) {
          actions.push({ name: "Approve release", role: "buyer" });
          actions.push({ name: "Dispute", role: "buyer" });
        }
        if (hasReviewWindow && isPendingRelease) {
          // Anyone may execute once the review window has elapsed.
          actions.push({ name: "Execute release", role: "anyone" });
        }
        if (isBuyer && !hasReviewWindow) {
          actions.push({ name: "Release", role: "buyer" });
          actions.push({ name: "Release partial", role: "buyer" });
          actions.push({ name: "Refund", role: "buyer" });
        }
        if (isArbiter && s === "Funded") {
          // Arbiter resolves only disputes; not added here to keep the set
          // tidy. Resolve is exposed separately below.
        }
      } else if (
        s === "Completed" || s === "Refunded" || s === "Cancelled"
      ) {
        // Terminal states — no write actions.
      } else if (s === "Disputed") {
        if (isArbiter) actions.push({ name: "Resolve", role: "arbiter" });
      }

      return actions;
    },
    [account]
  );

  /**
   * Format a ForgeError code as a human-readable name+message pair.
   */
  const formatForgeError = useCallback((code: number, message?: string): string => {
    const known = ForgeError[code as keyof typeof ForgeError];
    if (known) {
      return known.message + (message ? ` (${message})` : "");
    }
    return message ?? `Error code ${code}`;
  }, []);

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
      const reviewPeriodBig = BigInt(reviewPeriodSecs || "0");
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

      let escrowId: bigint;
      if (useReviewWindow && reviewPeriodBig > BigInt(0)) {
        // Review-window escrow: creates the escrow AND opens a review window
        // for the seller's future release request.
        const tx = await client.create_escrow_with_review(
          {
            buyer: buyerAddress,
            seller: sellerAddress,
            arbiter: arbiterAddress,
            token: tokenAddress,
            amount: amountBig,
            timeout: timeoutBig,
            review_period: reviewPeriodBig,
          },
          { timeoutInSeconds: 60 }
        );
        pushLog("create_escrow_with_review simulated — requesting signature…", "info");
        const sent = await tx.signAndSend();
        if (sent.sendTransactionResponse) {
          pushLog(
            `Submitted — tx hash ${sent.sendTransactionResponse.hash}`,
            "ok"
          );
        }
        escrowId = sent.result.unwrap();
        pushLog(
          `Review-window escrow created — id ${escrowId.toString()} (review window ${reviewPeriodSecs} s)`,
          "ok"
        );
        // Fetch the release request that was created alongside the escrow.
        void fetchReleaseRequest(escrowId);
      } else {
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
        escrowId = sent.result.unwrap();
        setCreatedId(escrowId.toString());
        pushLog(`Escrow created — id ${escrowId.toString()} (Pending)`, "ok");
      }

      setActionEscrowId(escrowId.toString());
      setReadEscrowId(escrowId.toString());
      void fetchEscrow(escrowId.toString());
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
    reviewPeriodSecs,
    useReviewWindow,
    pushLog,
  ]);

  const fetchReleaseRequest = useCallback(
    async (escrowId: bigint) => {
      if (!client) return;
      try {
        const { result } = await client.get_release_request({ escrow_id: escrowId });
        if (result.isOk()) {
          const req = result.unwrap();
          if (req.isSome()) {
            const r = req.unwrap();
            setReleaseRequest({
              amount: r.amount.value ?? BigInt(0),
              release_at: r.release_at.value ?? BigInt(0),
              requested_at: r.requested_at.value ?? BigInt(0),
            });
            setReviewWindowEndsAt(r.release_at.value ?? BigInt(0));
            pushLog(
              `Release request found for escrow ${escrowId.toString()} — review window ends at ${r.release_at.value}`,
              "ok"
            );
          } else {
            setReleaseRequest(null);
            setReviewWindowEndsAt(null);
            pushLog(
              `No release request for escrow ${escrowId.toString()}.`,
              "info"
            );
          }
        } else {
          pushLog(
            `get_release_request returned error: ${result.unwrapErr().message}`,
            "err"
          );
        }
      } catch (e) {
        pushLog(`get_release_request failed: ${(e as Error).message}`, "err");
      }
    },
    [client, pushLog]
  );

  const sendAction = useCallback(
    async (name: string, invoke: () => Promise<void>) => {
      // Safe-write guard: refuse to send while the wallet's network or account
      // doesn't match.
      const guardReason = await safeWriteGuard(name);
      if (guardReason) {
        pushLog(`${name} blocked: ${guardReason}`, "err");
        return;
      }

      setActionInFlight(name);
      try {
        pushLog(`${name} — simulating…`, "info");
        await invoke();
      } catch (e) {
        const err = e as Error;
        // Try to surface a ForgeError name instead of a raw code where possible.
        const codeMatch = err.message?.match(/Error\(Contract, #(\d+)\)/);
        if (codeMatch) {
          const code = parseInt(codeMatch[1], 10);
          pushLog(
            `${name} failed: ${formatForgeError(code, err.message)}`,
            "err"
          );
        } else {
          pushLog(`${name} failed: ${err.message}`, "err");
        }
      } finally {
        setActionInFlight(null);
      }
    },
    [safeWriteGuard, pushLog, formatForgeError]
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

          // If this escrow was created with a review window, also fetch its
          // release request so the countdown and action buttons stay accurate.
          if (useReviewWindow) {
            void fetchReleaseRequest(escrowId);
          }

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
    [client, readEscrowId, pushLog, useReviewWindow, fetchReleaseRequest]
  );

  connectProps;
  // Silence unused-variable lint for the memo that drives the connect button.
  void connectProps;

  // Decide which actions the connected account may take on the currently
  // selected escrow, factoring in any active review window.
  const currentActions = useMemo(
    () =>
      allowedActions(
        escrow,
        releaseRequest !== null,
        releaseRequest !== null
      ),
    [escrow, releaseRequest, allowedActions]
  );

  return (
    <main className="mx-auto max-w-5xl p-6">
      <div className="mb-3 flex items-center justify-between gap-4">
        <h1 className="text-3xl font-bold">
          Soroban Forge — Escrow Testnet Demo
        </h1>
        <a
          href="/issues"
          className="shrink-0 rounded border border-gray-300 px-3 py-2 text-sm text-gray-700 hover:bg-gray-50"
        >
          Issue board
        </a>
      </div>
      <p className="text-sm text-gray-500 mb-6">
        Contract{" "}
        <code className="break-all">{CONTRACT_ID}</code> ·{" "}
        {NETWORK_PASSPHRASE}
      </p>

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
                Default: repo&apos;s testnet demo SAC "credit" token
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

            {/* Review-window toggle */}
            <div className="flex items-center gap-2">
              <label className="flex items-center gap-1 text-sm">
                <input
                  type="checkbox"
                  checked={useReviewWindow}
                  onChange={(e) => setUseReviewWindow(e.target.checked)}
                />
                Use review window (create_escrow_with_review)
              </label>
            </div>
            {useReviewWindow && (
              <label className="block text-sm">
                Review period (seconds)
                <input
                  value={reviewPeriodSecs}
                  onChange={(e) => setReviewPeriodSecs(e.target.value)}
                  placeholder={DEFAULT_REVIEW_PERIOD.toString()}
                  className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
                />
                <span className="text-xs text-gray-500">
                  {MIN_REVIEW_PERIOD}–{MAX_REVIEW_PERIOD} seconds; default{" "}
                  {DEFAULT_REVIEW_PERIOD} s (3 days). After the window ends,
                  anyone may execute the seller&apos;s release.
                </span>
              </label>
            )}

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

          {/* Review-window info */}
          {releaseRequest && (
            <div className="mb-3 rounded border border-gray-200 p-3">
              <p className="text-sm font-medium mb-1">Review window</p>
              <p className="text-sm font-mono">
                Requested at:{" "}
                {new Date(Number(releaseRequest.requested_at) * 1000).toLocaleString()}
              </p>
              <p className="text-sm font-mono">
                Release available at:{" "}
                {new Date(Number(releaseRequest.release_at) * 1000).toLocaleString()}
              </p>
              {reviewWindowSecondsRemaining !== null && (
                <p className="text-sm font-mono">
                  {reviewWindowSecondsRemaining > 0
                    ? `${reviewWindowSecondsRemaining} s remaining in review window`
                    : "Review window ended — release may be executed"}
                </p>
              )}
            </div>
          )}

          {/* Allowed actions for the connected account */}
          {currentActions.length > 0 ? (
            <div className="flex flex-wrap gap-2">
              {currentActions.map((a) => (
                <button
                  key={a.name}
                  disabled={!client || actionInFlight !== null}
                  onClick={() => {
                    void runAction(a.name.toLowerCase(), (c, id) => {
                      switch (a.name) {
                        case "Deposit":
                          return c.deposit({ escrow_id: id });
                        case "Request release":
                          return c.request_release({
                            escrow_id: id,
                            amount: escrow?.amount ?? BigInt(0),
                          });
                        case "Approve release":
                          return c.approve_release({ escrow_id: id });
                        case "Execute release":
                          return c.execute_release({ escrow_id: id });
                        case "Dispute":
                          return c.dispute({
                            escrow_id: id,
                            claimant: account ?? "",
                          });
                        case "Release":
                          return c.release({ escrow_id: id });
                        case "Release partial":
                          return c.release_partial({
                            escrow_id: id,
                            amount: escrow?.amount ?? BigInt(0),
                          });
                        case "Refund":
                          return c.refund({ escrow_id: id });
                        case "Resolve":
                          return c.resolve({
                            escrow_id: id,
                            in_favor_of_seller: resolveFavor === "seller",
                          });
                        default:
                          return c.deposit({ escrow_id: id });
                      }
                    });
                  }}
                  className="rounded border border-blue-600 px-3 py-1.5 text-sm text-blue-600 hover:bg-blue-50 disabled:opacity-50"
                >
                  {actionInFlight === a.name.toLowerCase()
                    ? "Busy…"
                    : a.name}
                </button>
              ))}
            </div>
          ) : (
            <p className="text-sm text-gray-500">
              No write actions available for the connected account in the
              current escrow state.
            </p>
          )}

          {/* Resolve dispute (arbiter only) — always present for visibility */}
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
