"use client";

import { useCallback, useMemo, useState } from "react";
import { Header } from "../components/Header";
import { ContractSelector } from "../components/ContractSelector";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Networks } from "@stellar/stellar-sdk";
import { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit/sdk";
import { defaultModules } from "@creit.tech/stellar-wallets-kit/modules/utils";
import {
  TransactionLifecycle,
  type TransactionLifecycleState,
} from "../components/TransactionLifecycle";
import { LogFeed, type LogEntry } from "../components/LogFeed";
import { EscrowPanel } from "../components/panels/EscrowPanel";
import { VestingPanel } from "../components/panels/VestingPanel";
import { MultiSigPanel } from "../components/panels/MultiSigPanel";
import { DaoPanel } from "../components/panels/DaoPanel";
import { SubscriptionPanel } from "../components/panels/SubscriptionPanel";
import { MarketplacePanel } from "../components/panels/MarketplacePanel";
import {
  CONTRACTS,
  type ContractKey,
  createContractClient,
  TESTNET_PASSPHRASE,
  TESTNET_RPC_URL,
} from "../lib/contracts/registry";
import { parseForgeError, decodeContractEvents } from "../lib/forge-errors";

declare global {
  interface Window {
    freighterApi?: {
      isConnected(): Promise<boolean>;
      getPublicKey(): Promise<string>;
      signTransaction(
        xdr: string,
        opts?: { networkPassphrase?: string; address?: string }
      ): Promise<{ signedTxXdr: string }>;
    };
  }
}

let logCounter = 1;

export default function DeveloperConsolePage() {
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
  const [busy, setBusy] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);

  // Selected contract state
  const [selectedContract, setSelectedContract] = useState<ContractKey>("escrow");

  // Contract IDs mapping
  const [contractIds, setContractIds] = useState<Record<ContractKey, string>>({
    escrow: CONTRACTS.escrow.defaultContractId,
    vesting: CONTRACTS.vesting.defaultContractId,
    "multi-sig-wallet": CONTRACTS["multi-sig-wallet"].defaultContractId,
    "dao-governance": CONTRACTS["dao-governance"].defaultContractId,
    "subscription-payments": CONTRACTS["subscription-payments"].defaultContractId,
    "marketplace-royalties": CONTRACTS["marketplace-royalties"].defaultContractId,
  });

  // Transaction lifecycle state
  const [lifecycle, setLifecycle] = useState<TransactionLifecycleState>({
    step: "idle",
  });

  const pushLog = useCallback((text: string, kind: LogEntry["kind"] = "info") => {
    const time = new Date().toLocaleTimeString();
    setLogs((prev) => [{ id: logCounter++, time, text, kind }, ...prev]);
  }, []);

  const handleConnect = useCallback(async () => {
    setBusy(true);
    try {
      if (typeof window === "undefined" || !window.freighterApi) {
        pushLog("Freighter wallet extension not detected. Please install Freighter and reload.", "err");
        return;
      }
      const isConnected = await window.freighterApi.isConnected();
      if (!isConnected) {
        pushLog("Connecting to Freighter...", "info");
      }
      const pubkey = await window.freighterApi.getPublicKey();
      setAccount(pubkey);
      pushLog(`Connected wallet: ${pubkey.slice(0, 6)}...${pubkey.slice(-4)} (Stellar Testnet)`, "ok");
  const [log, setLog] = useState<LogEntry[]>([]);
  const [friendbotBusy, setFriendbotBusy] = useState(false);

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

  const pushLog = useCallback(
    (text: string, kind: LogEntry["kind"] = "info") => {
      setLog((prev) => [{ id: logId++, time: ts(), text, kind }, ...prev]);
    },
    []
  );

  useEffect(() => {
    // Wallet modules require browser APIs, so initialize them after hydration.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setMounted(true);
    StellarWalletsKit.init({ modules: defaultModules() });
    StellarWalletsKit.setNetwork(Networks.TESTNET);
  }, []);

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
      pushLog(`Wallet connection error: ${(e as Error).message}`, "err");
    } finally {
      setBusy(false);
    }
  }, [pushLog]);

  const handleDisconnect = useCallback(() => {
  }, [listMine, mounted, pushLog]);

  const disconnect = useCallback(() => {
    void StellarWalletsKit.disconnect();
    setAccount(null);
    pushLog("Wallet disconnected. Session cleared.", "info");
  }, [pushLog]);

  const handleUpdateContractId = useCallback((key: ContractKey, id: string) => {
    setContractIds((prev) => ({ ...prev, [key]: id.trim() }));
  }, []);

  const handleResetContractId = useCallback((key: ContractKey) => {
    setContractIds((prev) => ({ ...prev, [key]: CONTRACTS[key].defaultContractId }));
    pushLog(`Reset ${CONTRACTS[key].name} contract ID to default.`, "info");
  }, [pushLog]);

  // Client for selected contract
  const activeClient = useMemo(() => {
    const currentContractId = contractIds[selectedContract];
    try {
      return createContractClient(selectedContract, {
        rpcUrl: TESTNET_RPC_URL,
        networkPassphrase: TESTNET_PASSPHRASE,
        contractId: currentContractId,
        publicKey: account ?? undefined,
        signTransaction: account
          ? async (xdr: string, opts?: { networkPassphrase?: string; address?: string }) => {
              if (typeof window === "undefined" || !window.freighterApi) {
                throw new Error("Freighter wallet not available for signing");
              }
              return await window.freighterApi.signTransaction(xdr, {
                networkPassphrase: opts?.networkPassphrase ?? TESTNET_PASSPHRASE,
                address: opts?.address ?? account,
              });
            }
          : undefined,
      });
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
      console.error(`Failed to initialize client for ${selectedContract}:`, e);
      return null;
    }
  }, [selectedContract, contractIds, account]);
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

  // Robust Transaction Execution Pipeline:
  // Validation -> Simulation -> Signing -> Submission -> Receipt / Event Decoding
  const runWriteFlow = useCallback(
    async (
      actionName: string,
      validate: () => { success: boolean; data?: any; error?: string },
      buildTx: (c: any, data: any) => Promise<any>
    ) => {
      // 1. Validation Stage
      setLifecycle({ step: "validating", actionName });
      pushLog(`${actionName}: validating inputs...`, "info");

      const validation = validate();
      if (!validation.success) {
        const errorMsg = validation.error ?? "Input validation failed";
        pushLog(`${actionName} validation failed: ${errorMsg}`, "err");
        setLifecycle({
          step: "error",
          actionName,
          error: { rawMessage: errorMsg },
        });
        return;
      }

      if (!activeClient) {
        const errorMsg = "Contract client is not initialized.";
        pushLog(`${actionName} aborted: ${errorMsg}`, "err");
        setLifecycle({
          step: "error",
          actionName,
          error: { rawMessage: errorMsg },
        });
        return;
      }

      if (!account) {
        const errorMsg = "Please connect your Freighter wallet before submitting a write transaction.";
        pushLog(`${actionName} aborted: ${errorMsg}`, "err");
        setLifecycle({
          step: "error",
          actionName,
          error: { rawMessage: errorMsg },
        });
        return;
      }

      // 2. Simulation Stage
      setLifecycle({ step: "simulating", actionName });
      pushLog(`${actionName}: simulating invocation on Soroban host...`, "info");

      try {
        const tx = await buildTx(activeClient, validation.data);

        let simDetails = "";
        try {
          if (tx.simulation) {
            simDetails = JSON.stringify(tx.simulation, null, 2);
          }
        } catch {
          // ignore stringify errors
        }

        // 3. Signing Stage
        setLifecycle({
          step: "signing",
          actionName,
          simulationDetails: simDetails || undefined,
        });
        pushLog(`${actionName}: simulation successful — requesting Freighter signature...`, "info");

        // 4. Submission Stage
        const sent = await tx.signAndSend();
        const txHash = sent?.sendTransactionResponse?.hash ?? sent?.hash;

        setLifecycle({
          step: "submitting",
          actionName,
          txHash,
          simulationDetails: simDetails || undefined,
        });
        pushLog(`${actionName} submitted — tx hash: ${txHash ?? "pending"}`, "ok");

        // 5. Receipt & Events Stage
        let decodedEvents: any[] = [];
        if (sent?.events) {
          decodedEvents = decodeContractEvents(sent.events);
        } else if (sent?.getTransactionResponse?.resultXdr) {
          decodedEvents = decodeContractEvents(sent.getTransactionResponse?.diagnosticEventsXdr ?? []);
        }

        const ledgerSeq = sent?.getTransactionResponse?.ledger ?? sent?.ledger;

        setLifecycle({
          step: "success",
          actionName,
          txHash,
          ledger: ledgerSeq,
          decodedEvents,
          simulationDetails: simDetails || undefined,
        });
        pushLog(`${actionName} finalized in ledger ${ledgerSeq ?? "latest"}!`, "ok");
      } catch (err) {
        const parsed = parseForgeError(err);
        const displayMsg = parsed.name
          ? `ForgeError #${parsed.code}: ${parsed.name} (${parsed.description ?? ""})`
          : parsed.rawMessage;

        pushLog(`${actionName} failed: ${displayMsg}`, "err");
        setLifecycle({
          step: "error",
          actionName,
          error: parsed,
        });
      }
    },
    [activeClient, account, pushLog]
  );

  return (
    <main className="mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8">
      {/* Header */}
      <Header
        account={account}
        busy={busy}
        onConnect={handleConnect}
        onDisconnect={handleDisconnect}
        onLog={pushLog}
      />

      {/* Contract Selector */}
      <ContractSelector
        selectedContract={selectedContract}
        contractIds={contractIds}
        onSelect={setSelectedContract}
        onUpdateContractId={handleUpdateContractId}
        onResetContractId={handleResetContractId}
      />

      {/* Transaction Lifecycle Feedback */}
      <TransactionLifecycle
        state={lifecycle}
        onDismiss={() => setLifecycle({ step: "idle" })}
      />

      {/* Active Contract Panel */}
      <div className="mb-6">
        {selectedContract === "escrow" && (
          <EscrowPanel
            client={activeClient}
            account={account}
            onLog={pushLog}
            runWriteFlow={runWriteFlow}
          />
        )}
        {selectedContract === "vesting" && (
          <VestingPanel
            client={activeClient}
            account={account}
            onLog={pushLog}
            runWriteFlow={runWriteFlow}
          />
        )}
        {selectedContract === "multi-sig-wallet" && (
          <MultiSigPanel
            client={activeClient}
            account={account}
            onLog={pushLog}
            runWriteFlow={runWriteFlow}
          />
        )}
        {selectedContract === "dao-governance" && (
          <DaoPanel
            client={activeClient}
            account={account}
            onLog={pushLog}
            runWriteFlow={runWriteFlow}
          />
        )}
        {selectedContract === "subscription-payments" && (
          <SubscriptionPanel
            client={activeClient}
            account={account}
            onLog={pushLog}
            runWriteFlow={runWriteFlow}
          />
        )}
        {selectedContract === "marketplace-royalties" && (
          <MarketplacePanel
            client={activeClient}
            account={account}
            onLog={pushLog}
            runWriteFlow={runWriteFlow}
          />
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
      </div>

      {/* Console Log Feed */}
      <LogFeed logs={logs} onClear={() => setLogs([])} />
    </main>
  );
}
}

function formatAmount(stroops: bigint): string {
  const whole = stroops / SCALE;
  const frac = (stroops % SCALE).toString().padStart(Number(DECIMALS), "0");
  return `${whole.toString()}.${frac}`;
}
