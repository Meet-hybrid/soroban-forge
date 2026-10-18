"use client";

import { useCallback, useMemo, useState } from "react";
import { Header } from "../components/Header";
import { ContractSelector } from "../components/ContractSelector";
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
    } catch (e) {
      pushLog(`Wallet connection error: ${(e as Error).message}`, "err");
    } finally {
      setBusy(false);
    }
  }, [pushLog]);

  const handleDisconnect = useCallback(() => {
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
    } catch (e) {
      console.error(`Failed to initialize client for ${selectedContract}:`, e);
      return null;
    }
  }, [selectedContract, contractIds, account]);

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
        )}
      </div>

      {/* Console Log Feed */}
      <LogFeed logs={logs} onClear={() => setLogs([])} />
    </main>
  );
}