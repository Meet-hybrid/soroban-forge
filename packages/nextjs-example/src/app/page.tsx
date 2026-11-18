"use client";

import { useCallback, useEffect, useMemo, useReducer, useState } from "react";
import { Networks } from "@stellar/stellar-sdk";
import { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit/sdk";
import { defaultModules } from "@creit.tech/stellar-wallets-kit/modules/utils";
import {
  Client,
  networks,
  type EscrowData,
} from "@soroban-forge/escrow-client";
import {
  DEMO_ACCOUNTS,
  evaluatePrerequisites,
  initialTransactionState,
  transactionReducer,
  transitionDemoEscrow,
  type DemoAccount,
  type DemoEscrowRecord,
  type ExecutionMode,
} from "./state";

const RPC_URL = "https://soroban-testnet.stellar.org";
const NETWORK_PASSPHRASE = networks.testnet.networkPassphrase;
const CONTRACT_ID = networks.testnet.contractId;

// 7-decimal demo token (SAC "credit" asset) used by the repo's testnet demo
const DEFAULT_TOKEN =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

const DECIMALS = BigInt(7);
const SCALE = BigInt(10) ** DECIMALS;

type LogEntry = {
  id: number;
  time: string;
  mode: ExecutionMode;
  text: string;
  kind: "info" | "ok" | "err";
};

let logId = 0;

function ts(): string {
  return new Date().toLocaleTimeString();
}

function delay(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export default function Home() {
  const [mounted, setMounted] = useState(false);
  const [mode, setMode] = useState<ExecutionMode>("demo");

  // Live Testnet state
  const [account, setAccount] = useState<string | null>(null);
  const [client, setClient] = useState<Client | null>(null);
  const [walletBusy, setWalletBusy] = useState(false);
  const [friendbotBusy, setFriendbotBusy] = useState(false);

  // Demo / Local state
  const [selectedDemoRole, setSelectedDemoRole] =
    useState<keyof typeof DEMO_ACCOUNTS>("buyer");
  const [demoAccountsState, setDemoAccountsState] = useState(DEMO_ACCOUNTS);
  const [demoEscrows, setDemoEscrows] = useState<Record<string, DemoEscrowRecord>>({
    "1": {
      escrow_id: BigInt(1),
      buyer: DEMO_ACCOUNTS.buyer.address,
      seller: DEMO_ACCOUNTS.seller.address,
      arbiter: DEMO_ACCOUNTS.arbiter.address,
      token: DEFAULT_TOKEN,
      amount: BigInt(10000000),
      timeout: BigInt(3600),
      created_at: BigInt(1700000000),
      status: { tag: "Pending", values: undefined },
    },
  });
  const [nextDemoId, setNextDemoId] = useState(2);

  // Transaction State Machine
  const [txState, dispatchTx] = useReducer(
    transactionReducer,
    initialTransactionState("demo")
  );

  // Log Feed
  const [log, setLog] = useState<LogEntry[]>([]);

  // Create-escrow form
  const [buyer, setBuyer] = useState(DEMO_ACCOUNTS.buyer.address);
  const [seller, setSeller] = useState(DEMO_ACCOUNTS.seller.address);
  const [arbiter, setArbiter] = useState(DEMO_ACCOUNTS.arbiter.address);
  const [token, setToken] = useState(DEFAULT_TOKEN);
  const [amount, setAmount] = useState("10000000"); // stroops
  const [timeoutSecs, setTimeoutSecs] = useState("3600");
  const [createdId, setCreatedId] = useState<string | null>(null);

  // Actions panel
  const [actionEscrowId, setActionEscrowId] = useState("1");
  const [resolveFavor, setResolveFavor] = useState<"seller" | "buyer">("seller");

  // Read panel
  const [readEscrowId, setReadEscrowId] = useState("1");
  const [escrow, setEscrow] = useState<EscrowData | DemoEscrowRecord | null>(null);
  const [readBusy, setReadBusy] = useState(false);

  // List panel
  const [myIds, setMyIds] = useState<Array<bigint>>([BigInt(1)]);
  const [listTotal, setListTotal] = useState<number | null>(1);
  const [listBusy, setListBusy] = useState(false);

  const pushLog = useCallback(
    (text: string, kind: LogEntry["kind"] = "info", entryMode: ExecutionMode = mode) => {
      setLog((prev) => [
        { id: logId++, time: ts(), mode: entryMode, text, kind },
        ...prev,
      ]);
    },
    [mode]
  );

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setMounted(true);
    StellarWalletsKit.init({ modules: defaultModules() });
    StellarWalletsKit.setNetwork(Networks.TESTNET);
    pushLog(
      "Initialized demo environment. Mode is DEMO / LOCAL (simulated in-memory transactions).",
      "info",
      "demo"
    );
  }, [pushLog]);

  // Evaluate prerequisites for live testnet
  const prerequisites = useMemo(
    () =>
      evaluatePrerequisites({
        walletMounted: mounted,
        connectedAccount: account,
        networkPassphrase: NETWORK_PASSPHRASE,
        expectedPassphrase: "Test SDF Network ; September 2015",
      }),
    [mounted, account]
  );

  const allPrerequisitesMet = useMemo(
    () => prerequisites.every((p) => p.satisfied),
    [prerequisites]
  );

  // Active address depending on mode
  const activeAddress = useMemo(() => {
    if (mode === "demo") {
      return demoAccountsState[selectedDemoRole].address;
    }
    return account;
  }, [mode, selectedDemoRole, demoAccountsState, account]);

  // Switch execution mode
  const handleModeSwitch = useCallback(
    (newMode: ExecutionMode) => {
      setMode(newMode);
      dispatchTx({ type: "SET_MODE", mode: newMode });
      if (newMode === "demo") {
        setBuyer(demoAccountsState[selectedDemoRole].address);
        setSeller(demoAccountsState.seller.address);
        setArbiter(demoAccountsState.arbiter.address);
        setActionEscrowId("1");
        setReadEscrowId("1");
        pushLog("Switched execution mode to DEMO / LOCAL.", "info", "demo");
      } else {
        setBuyer(account ?? "");
        setSeller("");
        setArbiter("");
        pushLog(
          "Switched execution mode to LIVE / TESTNET. Requires Freighter or compatible wallet.",
          "info",
          "testnet"
        );
      }
    },
    [demoAccountsState, selectedDemoRole, account, pushLog]
  );

  // Live Testnet: list escrows for participant
  const listMineLive = useCallback(
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
          "ok",
          "testnet"
        );
      } catch (e) {
        pushLog(`List escrows failed: ${(e as Error).message}`, "err", "testnet");
      } finally {
        setListBusy(false);
      }
    },
    [pushLog]
  );

  // Demo: list escrows for participant
  const listMineDemo = useCallback(
    (addr: string) => {
      setListBusy(true);
      const matchingIds: bigint[] = [];
      for (const item of Object.values(demoEscrows)) {
        if (
          item.buyer === addr ||
          item.seller === addr ||
          item.arbiter === addr
        ) {
          matchingIds.push(item.escrow_id);
        }
      }
      setMyIds(matchingIds);
      setListTotal(matchingIds.length);
      pushLog(
        `Local escrows for ${addr.slice(0, 10)}…: found ${matchingIds.length} records`,
        "ok",
        "demo"
      );
      setListBusy(false);
    },
    [demoEscrows, pushLog]
  );

  const listMine = useCallback(() => {
    if (mode === "demo") {
      listMineDemo(activeAddress ?? DEMO_ACCOUNTS.buyer.address);
    } else if (client && account) {
      void listMineLive(client, account);
    }
  }, [mode, activeAddress, client, account, listMineDemo, listMineLive]);

  // Live Wallet connect
  const connectLive = useCallback(async () => {
    setWalletBusy(true);
    try {
      if (!mounted) {
        throw new Error("Wallet connectors are still loading; try again shortly.");
      }
      const { address: pubkey } = await StellarWalletsKit.authModal();
      setAccount(pubkey);
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
        `Connected to ${pubkey.slice(0, 6)}…${pubkey.slice(-4)} on Stellar Testnet`,
        "ok",
        "testnet"
      );
      void listMineLive(c, pubkey);
    } catch (e) {
      pushLog(`Wallet connect failed: ${(e as Error).message}`, "err", "testnet");
    } finally {
      setWalletBusy(false);
    }
  }, [mounted, listMineLive, pushLog]);

  const disconnectLive = useCallback(() => {
    void StellarWalletsKit.disconnect();
    setAccount(null);
    setClient(null);
    setEscrow(null);
    setMyIds([]);
    setListTotal(null);
    setCreatedId(null);
    pushLog("Disconnected live wallet. Cleared live state.", "info", "testnet");
  }, [pushLog]);

  const fundFriendbot = useCallback(async () => {
    if (!account) return;
    setFriendbotBusy(true);
    pushLog(`Requesting testnet funds from Friendbot for ${account}…`, "info", "testnet");
    try {
      const res = await fetch(`https://friendbot.stellar.org?addr=${account}`);
      const json = (await res.json()) as { hash?: string; detail?: string };
      if (res.ok && json.hash) {
        pushLog(`Friendbot funded account — tx hash ${json.hash}`, "ok", "testnet");
      } else {
        pushLog(`Friendbot error: ${json.detail ?? "unexpected response"}`, "err", "testnet");
      }
    } catch (e) {
      pushLog(`Friendbot request failed: ${(e as Error).message}`, "err", "testnet");
    } finally {
      setFriendbotBusy(false);
    }
  }, [account, pushLog]);

  // Demo: switch active role or add simulated funds
  const handleSelectDemoRole = (role: keyof typeof DEMO_ACCOUNTS) => {
    setSelectedDemoRole(role);
    setBuyer(demoAccountsState[role].address);
    pushLog(
      `Selected demo role '${demoAccountsState[role].name}' (${demoAccountsState[role].address.slice(0, 12)}…).`,
      "info",
      "demo"
    );
  };

  const handleAddDemoFunds = () => {
    setDemoAccountsState((prev) => ({
      ...prev,
      [selectedDemoRole]: {
        ...prev[selectedDemoRole],
        balanceXlm: prev[selectedDemoRole].balanceXlm + 50,
      },
    }));
    pushLog(
      `Added 50 simulated XLM to ${demoAccountsState[selectedDemoRole].name}.`,
      "ok",
      "demo"
    );
  };

  // Fetch escrow
  const fetchEscrow = useCallback(
    async (id?: string) => {
      const raw = id ?? readEscrowId;
      const parsedId = BigInt(raw || "0");
      if (parsedId <= BigInt(0)) {
        pushLog("Enter a valid escrow id to fetch.", "err");
        return;
      }
      setReadBusy(true);

      if (mode === "demo") {
        await delay(200);
        const record = demoEscrows[parsedId.toString()];
        if (record) {
          setEscrow(record);
          pushLog(
            `Fetched demo escrow #${parsedId.toString()} — Status: ${record.status.tag}`,
            "ok",
            "demo"
          );
        } else {
          setEscrow(null);
          pushLog(
            `Demo escrow #${parsedId.toString()} not found in local memory store.`,
            "err",
            "demo"
          );
        }
        setReadBusy(false);
        return;
      }

      // Live Testnet
      if (!client) {
        pushLog("Connect wallet first to read testnet escrow data.", "err", "testnet");
        setReadBusy(false);
        return;
      }

      try {
        const { result } = await client.get_escrow({ escrow_id: parsedId });
        if (result.isOk()) {
          const data = result.unwrap();
          setEscrow(data);
          pushLog(
            `Fetched on-chain escrow #${parsedId.toString()} — Status: ${data.status.tag}`,
            "ok",
            "testnet"
          );
        } else {
          pushLog(
            `get_escrow returned contract error: ${result.unwrapErr().message}`,
            "err",
            "testnet"
          );
        }
      } catch (e) {
        pushLog(`get_escrow failed: ${(e as Error).message}`, "err", "testnet");
      } finally {
        setReadBusy(false);
      }
    },
    [client, mode, readEscrowId, demoEscrows, pushLog]
  );

  // Create Escrow
  const createEscrow = useCallback(async () => {
    const amountBig = BigInt(amount || "0");
    if (amountBig <= BigInt(0)) {
      pushLog("Amount must be a positive integer number of stroops.", "err");
      return;
    }
    const timeoutBig = BigInt(timeoutSecs || "0");
    const buyerAddress = buyer.trim() || activeAddress || "";
    const sellerAddress = seller.trim();
    const arbiterAddress = arbiter.trim();
    const tokenAddress = token.trim();

    if (!buyerAddress || !sellerAddress || !arbiterAddress || !tokenAddress) {
      pushLog("Enter complete buyer, seller, arbiter, and token addresses.", "err");
      return;
    }

    if (mode === "demo") {
      dispatchTx({
        type: "START_SIMULATION",
        action: "create_escrow",
        mode: "demo",
      });
      pushLog("[DEMO] Simulating create_escrow preconditions…", "info", "demo");
      await delay(350);

      dispatchTx({ type: "START_SIGNING" });
      pushLog("[DEMO] Authorizing with local demo keypair…", "info", "demo");
      await delay(300);

      dispatchTx({ type: "START_SUBMISSION", hash: `demo-tx-${Date.now()}` });
      pushLog("[DEMO] Committing new escrow record to local state…", "info", "demo");
      await delay(250);

      const newId = nextDemoId.toString();
      const newEscrow: DemoEscrowRecord = {
        escrow_id: BigInt(newId),
        buyer: buyerAddress,
        seller: sellerAddress,
        arbiter: arbiterAddress,
        token: tokenAddress,
        amount: amountBig,
        timeout: timeoutBig,
        created_at: BigInt(Math.floor(Date.now() / 1000)),
        status: { tag: "Pending", values: undefined },
      };

      setDemoEscrows((prev) => ({ ...prev, [newId]: newEscrow }));
      setNextDemoId((prev) => prev + 1);
      setCreatedId(newId);
      setActionEscrowId(newId);
      setReadEscrowId(newId);

      dispatchTx({
        type: "CONFIRM",
        hash: `demo-local-ref-${newId}`,
        details: `Demo escrow #${newId} created successfully (Status: Pending).`,
      });
      pushLog(
        `[DEMO] Escrow created — id #${newId} (Pending). Ready for deposit/actions.`,
        "ok",
        "demo"
      );
      listMineDemo(buyerAddress);
      return;
    }

    // Live Testnet Flow
    if (!client) {
      pushLog("Wallet not connected. Connect your wallet for live testnet transactions.", "err", "testnet");
      return;
    }

    try {
      dispatchTx({
        type: "START_SIMULATION",
        action: "create_escrow",
        mode: "testnet",
      });
      pushLog("Simulating create_escrow on Stellar Testnet…", "info", "testnet");

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

      dispatchTx({ type: "START_SIGNING" });
      pushLog("Simulation passed. Requesting wallet signature in Freighter…", "info", "testnet");

      const sent = await tx.signAndSend();
      const txHash = sent.sendTransactionResponse?.hash;

      dispatchTx({
        type: "START_SUBMISSION",
        hash: txHash,
      });
      pushLog(`Submitted to Testnet — tx hash ${txHash ?? "unknown"}`, "ok", "testnet");

      const escrowId = sent.result.unwrap();
      const idStr = escrowId.toString();
      setCreatedId(idStr);
      setActionEscrowId(idStr);
      setReadEscrowId(idStr);

      dispatchTx({
        type: "CONFIRM",
        hash: txHash,
        details: `On-chain Escrow #${idStr} confirmed on Stellar Testnet.`,
      });
      pushLog(`On-chain Escrow #${idStr} created (Pending).`, "ok", "testnet");
      if (account) {
        void listMineLive(client, account);
      }
    } catch (e) {
      const errMsg = (e as Error).message;
      dispatchTx({ type: "FAIL", error: errMsg });
      pushLog(`create_escrow failed: ${errMsg}`, "err", "testnet");
    }
  }, [
    amount,
    timeoutSecs,
    buyer,
    activeAddress,
    seller,
    arbiter,
    token,
    mode,
    nextDemoId,
    client,
    pushLog,
    listMineDemo,
    listMineLive,
    account,
  ]);

  // Execute Action (Deposit, Release, Refund, Cancel, Dispute, Resolve)
  const runAction = useCallback(
    async (
      actionName: "deposit" | "release" | "refund" | "cancel" | "dispute" | "resolve",
      resolveToSeller?: boolean
    ) => {
      const idRaw = actionEscrowId.trim();
      const idBig = BigInt(idRaw || "0");
      if (idBig <= BigInt(0)) {
        pushLog(`Enter a valid escrow id for ${actionName}.`, "err");
        return;
      }

      if (mode === "demo") {
        dispatchTx({
          type: "START_SIMULATION",
          action: actionName,
          mode: "demo",
        });
        pushLog(`[DEMO] Simulating ${actionName} preconditions…`, "info", "demo");
        await delay(350);

        const targetEscrow = demoEscrows[idRaw];
        if (!targetEscrow) {
          const err = `Demo escrow #${idRaw} does not exist in local store.`;
          dispatchTx({ type: "FAIL", error: err });
          pushLog(`[DEMO] ${actionName} failed: ${err}`, "err", "demo");
          return;
        }

        const transitionResult = transitionDemoEscrow(targetEscrow, actionName, {
          claimant: activeAddress ?? undefined,
          in_favor_of_seller: resolveToSeller ?? (resolveFavor === "seller"),
        });

        if (!transitionResult.success) {
          dispatchTx({ type: "FAIL", error: transitionResult.error });
          pushLog(`[DEMO] ${actionName} failed: ${transitionResult.error}`, "err", "demo");
          return;
        }

        dispatchTx({ type: "START_SIGNING" });
        pushLog(`[DEMO] Authorizing ${actionName} with local demo signer…`, "info", "demo");
        await delay(300);

        dispatchTx({ type: "START_SUBMISSION", hash: `demo-tx-${Date.now()}` });
        pushLog(`[DEMO] Applying state transition to escrow #${idRaw}…`, "info", "demo");
        await delay(250);

        setDemoEscrows((prev) => ({
          ...prev,
          [idRaw]: transitionResult.updated,
        }));
        setEscrow(transitionResult.updated);

        dispatchTx({
          type: "CONFIRM",
          hash: `demo-ref-${idRaw}`,
          details: `Action '${actionName}' executed successfully. Escrow status is now '${transitionResult.updated.status.tag}'.`,
        });
        pushLog(
          `[DEMO] ${actionName} succeeded on escrow #${idRaw} -> ${transitionResult.updated.status.tag}`,
          "ok",
          "demo"
        );
        return;
      }

      // Live Testnet Action
      if (!client) {
        pushLog("Wallet not connected. Connect your wallet to send testnet transactions.", "err", "testnet");
        return;
      }

      try {
        dispatchTx({
          type: "START_SIMULATION",
          action: actionName,
          mode: "testnet",
        });
        pushLog(`Simulating ${actionName} on Stellar Testnet…`, "info", "testnet");

        let tx;
        if (actionName === "dispute") {
          tx = await client.dispute({
            escrow_id: idBig,
            claimant: account ?? "",
          });
        } else if (actionName === "resolve") {
          tx = await client.resolve({
            escrow_id: idBig,
            in_favor_of_seller: resolveToSeller ?? (resolveFavor === "seller"),
          });
        } else {
          tx = await client[actionName]({ escrow_id: idBig });
        }

        dispatchTx({ type: "START_SIGNING" });
        pushLog(`Simulation succeeded. Awaiting wallet signature for ${actionName}…`, "info", "testnet");

        const sent = await tx.signAndSend();
        const txHash = sent.sendTransactionResponse?.hash;

        dispatchTx({
          type: "START_SUBMISSION",
          hash: txHash,
        });
        pushLog(`${actionName} submitted to Testnet — tx hash ${txHash ?? "unknown"}`, "ok", "testnet");

        dispatchTx({
          type: "CONFIRM",
          hash: txHash,
          details: `Action '${actionName}' confirmed on Stellar Testnet.`,
        });
        pushLog(`${actionName} confirmed on testnet contract!`, "ok", "testnet");
        void fetchEscrow(idRaw);
      } catch (e) {
        const errMsg = (e as Error).message;
        dispatchTx({ type: "FAIL", error: errMsg });
        pushLog(`${actionName} failed: ${errMsg}`, "err", "testnet");
      }
    },
    [
      actionEscrowId,
      mode,
      client,
      demoEscrows,
      activeAddress,
      resolveFavor,
      account,
      fetchEscrow,
      pushLog,
    ]
  );

  const isInFlight =
    txState.stage === "simulating" ||
    txState.stage === "signing" ||
    txState.stage === "submitting";

  return (
    <main className="mx-auto max-w-5xl p-6">
      <div className="mb-1 flex items-center justify-between gap-4">
        <h1 className="text-3xl font-bold">Soroban Forge — Escrow Testnet Demo</h1>
        <a href="/ui/index.html" className="shrink-0 rounded border border-gray-300 px-3 py-2 text-sm text-gray-700 hover:bg-gray-50">
          Dashboard screens
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
    <main className="mx-auto max-w-5xl p-6 font-sans text-gray-900">
      {/* Testnet-Only & Unaudited Banner */}
      <div className="mb-6 rounded-xl border border-amber-300 bg-amber-50 p-4 shadow-sm">
        <div className="flex items-start gap-3">
          <span className="text-xl">⚠️</span>
          <div className="text-sm text-amber-900">
            <strong className="font-semibold block mb-0.5">
              Testnet-Only &amp; Unaudited Demonstration
            </strong>
            <p>
              This example application is strictly for developer experimentation on the{" "}
              <strong>Stellar Testnet</strong>. The underlying smart contracts, TypeScript SDK,
              and this interface are <strong>unaudited</strong> and must <strong>never</strong> be
              used with Stellar Mainnet keys, accounts, or real funds.
            </p>
          </div>
        </div>
      </div>

      {/* Header & Mode Switcher */}
      <header className="mb-6 flex flex-col gap-4 border-b border-gray-200 pb-6 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-bold tracking-tight text-gray-900 sm:text-3xl">
              Soroban Forge — Escrow Explorer
            </h1>
            <span
              className={`rounded-full px-2.5 py-0.5 text-xs font-semibold uppercase tracking-wider ${
                mode === "demo"
                  ? "bg-purple-100 text-purple-800 border border-purple-300"
                  : "bg-emerald-100 text-emerald-800 border border-emerald-300"
              }`}
            >
              {mode === "demo" ? "Demo / Local Mode" : "Live Testnet Mode"}
            </span>
          </div>
          <p className="mt-1 text-sm text-gray-600">
            {mode === "demo"
              ? "Local simulated in-memory store. Test escrow creation and transitions without a browser wallet."
              : `Connected to Stellar Testnet contract (${CONTRACT_ID.slice(0, 8)}…${CONTRACT_ID.slice(-6)})`}
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div className="flex rounded-lg border border-gray-300 bg-gray-100 p-1">
            <button
              onClick={() => handleModeSwitch("demo")}
              className={`rounded-md px-3 py-1.5 text-xs font-semibold transition ${
                mode === "demo"
                  ? "bg-white text-purple-900 shadow-sm"
                  : "text-gray-600 hover:text-gray-900"
              }`}
            >
              Demo / Local
            </button>
            <button
              onClick={() => handleModeSwitch("testnet")}
              className={`rounded-md px-3 py-1.5 text-xs font-semibold transition ${
                mode === "testnet"
                  ? "bg-white text-emerald-900 shadow-sm"
                  : "text-gray-600 hover:text-gray-900"
              }`}
            >
              Live Testnet
            </button>
          </div>

          <a
            href="/issues"
            className="rounded-lg border border-gray-300 bg-white px-3 py-2 text-xs font-medium text-gray-700 hover:bg-gray-50 shadow-sm"
          >
            Issue board
          </a>
        </div>
      </header>

      {/* Transaction State Banner */}
      {txState.stage !== "idle" && (
        <section
          aria-live="polite"
          className={`mb-6 rounded-xl border p-4 shadow-sm transition ${
            txState.stage === "simulating"
              ? "border-amber-300 bg-amber-50"
              : txState.stage === "signing"
                ? "border-blue-300 bg-blue-50"
                : txState.stage === "submitting"
                  ? "border-indigo-300 bg-indigo-50"
                  : txState.stage === "confirmed"
                    ? "border-emerald-300 bg-emerald-50"
                    : "border-red-300 bg-red-50"
          }`}
        >
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-start gap-3">
              <div className="mt-0.5">
                {txState.stage === "confirmed" && (
                  <span className="flex h-6 w-6 items-center justify-center rounded-full bg-emerald-600 text-xs text-white">
                    ✓
                  </span>
                )}
                {txState.stage === "failed" && (
                  <span className="flex h-6 w-6 items-center justify-center rounded-full bg-red-600 text-xs text-white">
                    ✕
                  </span>
                )}
                {isInFlight && (
                  <span className="inline-block h-5 w-5 animate-spin rounded-full border-2 border-current border-t-transparent text-gray-700" />
                )}
              </div>

              <div>
                <div className="flex items-center gap-2">
                  <strong className="font-semibold text-gray-900 text-sm">
                    {txState.stage === "simulating" && `Simulating '${txState.action}'`}
                    {txState.stage === "signing" && `Awaiting Signature for '${txState.action}'`}
                    {txState.stage === "submitting" && `Submitting '${txState.action}'`}
                    {txState.stage === "confirmed" && `Confirmed: '${txState.action}' succeeded`}
                    {txState.stage === "failed" && `Failed: '${txState.action}' rejected`}
                  </strong>
                  <span className="rounded bg-black/10 px-1.5 py-0.5 text-[11px] font-mono uppercase text-gray-800">
                    {txState.mode === "demo" ? "DEMO / LOCAL" : "LIVE / TESTNET"}
                  </span>
                </div>

                <p className="mt-1 text-xs text-gray-700">
                  {txState.stage === "simulating" &&
                    (txState.mode === "demo"
                      ? "Verifying contract rules in local simulation…"
                      : "Simulating invocation against Soroban Testnet RPC…")}
                  {txState.stage === "signing" &&
                    (txState.mode === "demo"
                      ? "Simulating cryptographic signature authorization with local demo key…"
                      : "Prompting browser wallet for transaction authorization. Check your extension window.")}
                  {txState.stage === "submitting" &&
                    (txState.mode === "demo"
                      ? "Committing state transition to memory store…"
                      : "Broadcasting transaction envelope to Stellar Testnet RPC…")}
                  {txState.stage === "confirmed" && (txState.details || "Transaction successfully completed.")}
                  {txState.stage === "failed" && (txState.error || "Transaction encountered an error.")}
                </p>

                {txState.hash && (
                  <p className="mt-1 text-xs font-mono text-gray-600">
                    Ref / Tx Hash:{" "}
                    {txState.mode === "testnet" && txState.hash.length === 64 ? (
                      <a
                        href={`https://stellar.expert/explorer/testnet/tx/${txState.hash}`}
                        target="_blank"
                        rel="noreferrer"
                        className="underline text-blue-600 hover:text-blue-800"
                      >
                        {txState.hash} ↗
                      </a>
                    ) : (
                      <span>{txState.hash}</span>
                    )}
                  </p>
                )}
              </div>
            </div>

            <div className="flex items-center gap-2 sm:self-center">
              {/* Progress step dots */}
              <div className="flex items-center gap-1 text-[11px] font-medium text-gray-500">
                <span className="text-blue-700 font-bold">1. Simulate</span>
                <span>→</span>
                <span className={txState.stage === "signing" || txState.stage === "submitting" || txState.stage === "confirmed" ? "text-blue-700 font-bold" : ""}>2. Sign</span>
                <span>→</span>
                <span className={txState.stage === "submitting" || txState.stage === "confirmed" ? "text-blue-700 font-bold" : ""}>3. Submit</span>
                <span>→</span>
                <span className={txState.stage === "confirmed" ? "text-emerald-700 font-bold" : txState.stage === "failed" ? "text-red-700 font-bold" : ""}>4. Status</span>
              </div>

              <button
                onClick={() => dispatchTx({ type: "RESET" })}
                className="rounded border border-gray-300 bg-white px-2 py-1 text-xs text-gray-700 hover:bg-gray-100"
              >
                Dismiss
              </button>
            </div>
          </div>
        </section>
      )}

      {/* Section 1: Wallet & Prerequisites */}
      <section className="mb-6 rounded-xl border border-gray-300 bg-white p-5 shadow-sm">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between mb-3">
          <h2 className="text-lg font-semibold flex items-center gap-2">
            <span>1 · Wallet &amp; Identity</span>
            <span
              className={`rounded px-2 py-0.5 text-xs font-semibold ${
                mode === "demo"
                  ? "bg-purple-100 text-purple-800"
                  : "bg-emerald-100 text-emerald-800"
              }`}
            >
              {mode === "demo" ? "Simulated Demo Keys" : "Freighter / Wallets Kit"}
            </span>
          </h2>
          <span className="text-xs text-gray-500">
            {mode === "demo"
              ? "Testing locally without wallet plugins"
              : "Live testnet signature provider"}
          </span>
        </div>

        {mode === "demo" ? (
          <div className="space-y-3">
            <p className="text-xs text-gray-600">
              In <strong>Demo Mode</strong>, actions are signed by simulated local accounts. Select an active role to simulate being the buyer, seller, or neutral arbiter:
            </p>
            <div className="flex flex-wrap gap-2">
              {(Object.keys(demoAccountsState) as Array<keyof typeof DEMO_ACCOUNTS>).map((role) => {
                const acc: DemoAccount = demoAccountsState[role];
                const isSelected = selectedDemoRole === role;
                return (
                  <button
                    key={role}
                    onClick={() => handleSelectDemoRole(role)}
                    className={`rounded-lg border px-3 py-2 text-left transition ${
                      isSelected
                        ? "border-purple-600 bg-purple-50 ring-2 ring-purple-500/20"
                        : "border-gray-200 bg-gray-50 hover:bg-gray-100"
                    }`}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-bold text-gray-900">{acc.name}</span>
                      <span className="rounded bg-white px-1.5 py-0.5 text-[10px] font-mono text-gray-600 border border-gray-200">
                        {acc.balanceXlm} XLM
                      </span>
                    </div>
                    <code className="text-[11px] text-gray-600 font-mono block mt-0.5">
                      {acc.address.slice(0, 16)}…
                    </code>
                  </button>
                );
              })}
              <button
                onClick={handleAddDemoFunds}
                className="self-center rounded-lg border border-purple-300 bg-purple-50 px-3 py-2 text-xs font-semibold text-purple-700 hover:bg-purple-100"
              >
                + Add 50 Demo XLM
              </button>
            </div>
          </div>
        ) : (
          <div>
            {/* Live Testnet Prerequisites Checklist */}
            <div className="mb-4 rounded-lg border border-gray-200 bg-gray-50 p-3">
              <h3 className="text-xs font-bold uppercase tracking-wider text-gray-600 mb-2">
                Live Testnet Prerequisites Checklist
              </h3>
              <ul className="grid gap-2 sm:grid-cols-2 text-xs">
                {prerequisites.map((p) => (
                  <li
                    key={p.id}
                    className="flex items-start gap-2 rounded border border-gray-200 bg-white p-2"
                  >
                    <span className="mt-0.5">
                      {p.satisfied ? (
                        <span className="text-emerald-600 font-bold">✓</span>
                      ) : (
                        <span className="text-amber-600 font-bold">○</span>
                      )}
                    </span>
                    <div>
                      <strong className="block font-medium text-gray-900">{p.label}</strong>
                      <span className="text-gray-500 text-[11px] block">{p.description}</span>
                      {p.actionHint && (
                        <span className="text-amber-700 text-[11px] font-medium block mt-0.5">
                          ↳ {p.actionHint}
                        </span>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            </div>

            {account ? (
              <div className="flex flex-wrap items-center gap-3">
                <span className="font-mono text-xs bg-gray-100 rounded px-2 py-1 text-gray-800 break-all">
                  {account}
                </span>
                <span className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs text-emerald-800 font-medium">
                  Connected (Testnet)
                </span>
                <button
                  onClick={fundFriendbot}
                  disabled={friendbotBusy}
                  className="rounded bg-amber-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-amber-600 disabled:opacity-50"
                >
                  {friendbotBusy ? "Funding…" : "Fund with Friendbot (10,000 testnet XLM)"}
                </button>
                <button
                  onClick={disconnectLive}
                  className="rounded bg-gray-200 px-3 py-1.5 text-xs font-semibold hover:bg-gray-300"
                >
                  Disconnect
                </button>
              </div>
            ) : (
              <div>
                <button
                  onClick={connectLive}
                  disabled={walletBusy || !mounted}
                  className="rounded bg-blue-600 px-4 py-2 text-sm font-semibold text-white hover:bg-blue-700 disabled:opacity-50"
                >
                  {walletBusy ? "Connecting…" : "Connect wallet (Freighter / Wallets Kit)"}
                </button>
                {!mounted && (
                  <p className="mt-2 text-xs text-amber-700">
                    Loading Stellar wallet connectors…
                  </p>
                )}
                {!allPrerequisitesMet && (
                  <p className="mt-2 text-xs text-gray-500">
                    Make sure your wallet extension network is switched to <strong>Testnet</strong> before signing transactions.
                  </p>
                )}
              </div>
            )}
          </div>
        )}
      </section>

      <div className="grid gap-6 lg:grid-cols-2">
        {/* Section 2: Create Escrow */}
        <section className="rounded-xl border border-gray-300 bg-white p-5 shadow-sm">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-lg font-semibold">2 · Create Escrow</h2>
            <span
              className={`rounded px-2 py-0.5 text-[11px] font-semibold uppercase ${
                mode === "demo"
                  ? "bg-purple-100 text-purple-800"
                  : "bg-emerald-100 text-emerald-800"
              }`}
            >
              {mode === "demo" ? "Local Simulated Action" : "Live On-Chain Tx"}
            </span>
          </div>

          <div className="space-y-3">
            <label className="block text-xs font-semibold text-gray-700">
              Buyer Address (G…)
              <input
                value={buyer}
                onChange={(e) => setBuyer(e.target.value)}
                placeholder={activeAddress ?? "buyer public key"}
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
              />
            </label>

            <label className="block text-xs font-semibold text-gray-700">
              Seller Address (G…)
              <input
                value={seller}
                onChange={(e) => setSeller(e.target.value)}
                placeholder="seller public key"
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
              />
            </label>

            <label className="block text-xs font-semibold text-gray-700">
              Arbiter Address (G…)
              <input
                value={arbiter}
                onChange={(e) => setArbiter(e.target.value)}
                placeholder="arbiter public key"
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
              />
            </label>

            <label className="block text-xs font-semibold text-gray-700">
              SEP-41 Token Address (C…)
              <input
                value={token}
                onChange={(e) => setToken(e.target.value)}
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
              />
              <span className="text-[11px] text-gray-500 block mt-0.5">
                Default: repo testnet demo SAC &quot;credit&quot; token (7 decimals).
              </span>
            </label>

            <div className="grid grid-cols-2 gap-3">
              <label className="block text-xs font-semibold text-gray-700">
                Amount (stroops)
                <input
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  placeholder="10000000 = 1 unit"
                  className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
                />
              </label>

              <label className="block text-xs font-semibold text-gray-700">
                Timeout (seconds)
                <input
                  value={timeoutSecs}
                  onChange={(e) => setTimeoutSecs(e.target.value)}
                  placeholder="3600"
                  className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
                />
              </label>
            </div>

            <button
              onClick={createEscrow}
              disabled={isInFlight || (mode === "testnet" && !client)}
              className={`mt-2 w-full rounded px-4 py-2.5 text-xs font-bold text-white transition disabled:opacity-50 ${
                mode === "demo"
                  ? "bg-purple-600 hover:bg-purple-700"
                  : "bg-blue-600 hover:bg-blue-700"
              }`}
            >
              {isInFlight && txState.action === "create_escrow"
                ? `Creating… (${txState.stage})`
                : mode === "demo"
                  ? "Create Escrow (Simulate Local Demo)"
                  : client
                    ? "Create Escrow (Broadcast to Testnet)"
                    : "Connect Wallet First"}
            </button>

            {createdId && (
              <p className="text-xs text-emerald-800 bg-emerald-50 border border-emerald-200 p-2 rounded">
                Last created escrow:{" "}
                <button
                  onClick={() => {
                    setReadEscrowId(createdId);
                    void fetchEscrow(createdId);
                  }}
                  className="font-mono font-bold underline"
                >
                  #{createdId}
                </button>{" "}
                (Status: Pending)
              </p>
            )}
          </div>
        </section>

        {/* Section 3: Actions */}
        <section className="rounded-xl border border-gray-300 bg-white p-5 shadow-sm">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-lg font-semibold">3 · Escrow Actions</h2>
            <span
              className={`rounded px-2 py-0.5 text-[11px] font-semibold uppercase ${
                mode === "demo"
                  ? "bg-purple-100 text-purple-800"
                  : "bg-emerald-100 text-emerald-800"
              }`}
            >
              {mode === "demo" ? "Local Simulated Flow" : "Live Signed On-Chain"}
            </span>
          </div>

          <label className="block text-xs font-semibold text-gray-700 mb-3">
            Target Escrow ID
            <input
              value={actionEscrowId}
              onChange={(e) => setActionEscrowId(e.target.value)}
              placeholder="e.g. 1"
              className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
            />
          </label>

          <div className="flex flex-wrap gap-2 mb-4">
            {(["Deposit", "Release", "Refund", "Cancel", "Dispute"] as const).map(
              (name) => {
                const actionKey = name.toLowerCase() as
                  | "deposit"
                  | "release"
                  | "refund"
                  | "cancel"
                  | "dispute";
                const isThisActionInFlight =
                  isInFlight && txState.action === actionKey;

                return (
                  <button
                    key={name}
                    disabled={isInFlight || (mode === "testnet" && !client)}
                    onClick={() => void runAction(actionKey)}
                    className="rounded border border-blue-600 bg-white px-3 py-1.5 text-xs font-semibold text-blue-600 hover:bg-blue-50 disabled:opacity-50 transition"
                  >
                    {isThisActionInFlight ? `${txState.stage}…` : name}
                  </button>
                );
              }
            )}
          </div>

          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3">
            <p className="text-xs font-semibold text-gray-900 mb-2">
              Resolve Dispute (Arbiter Only)
            </p>
            <div className="flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-1.5 text-xs text-gray-700">
                <input
                  type="radio"
                  name="resolve-favor"
                  checked={resolveFavor === "seller"}
                  onChange={() => setResolveFavor("seller")}
                />
                In favor of seller
              </label>
              <label className="flex items-center gap-1.5 text-xs text-gray-700">
                <input
                  type="radio"
                  name="resolve-favor"
                  checked={resolveFavor === "buyer"}
                  onChange={() => setResolveFavor("buyer")}
                />
                In favor of buyer
              </label>
              <button
                disabled={isInFlight || (mode === "testnet" && !client)}
                onClick={() => void runAction("resolve", resolveFavor === "seller")}
                className="rounded bg-purple-600 px-3 py-1.5 text-xs font-bold text-white hover:bg-purple-700 disabled:opacity-50 transition"
              >
                {isInFlight && txState.action === "resolve"
                  ? `${txState.stage}…`
                  : "Resolve"}
              </button>
            </div>
          </div>
        </section>
      </div>

      <div className="mt-6 grid gap-6 lg:grid-cols-2">
        {/* Section 4: Read Escrow */}
        <section className="rounded-xl border border-gray-300 bg-white p-5 shadow-sm">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-lg font-semibold">4 · Inspect Escrow</h2>
            <span className="rounded bg-gray-100 px-2 py-0.5 text-[11px] font-mono text-gray-600 border border-gray-200">
              Source: {mode === "demo" ? "Local Memory Store" : "Testnet Contract RPC"}
            </span>
          </div>

          <div className="flex gap-2 mb-4">
            <input
              value={readEscrowId}
              onChange={(e) => setReadEscrowId(e.target.value)}
              placeholder="escrow id"
              className="flex-1 rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
            />
            <button
              onClick={() => void fetchEscrow()}
              disabled={readBusy || (mode === "testnet" && !client)}
              className="rounded bg-gray-800 px-4 py-1.5 text-xs font-bold text-white hover:bg-gray-900 disabled:opacity-50"
            >
              {readBusy ? "Fetching…" : "Fetch"}
            </button>
          </div>

          {escrow ? (
            <dl className="grid grid-cols-1 gap-x-4 gap-y-1.5 text-xs sm:grid-cols-2">
              <dt className="font-semibold text-gray-500">Escrow ID</dt>
              <dd className="font-mono font-bold">#{escrow.escrow_id.toString()}</dd>

              <dt className="font-semibold text-gray-500">Status</dt>
              <dd>
                <span
                  className={`rounded-full px-2 py-0.5 text-[11px] font-bold ${
                    escrow.status.tag === "Completed" ||
                    escrow.status.tag === "Refunded" ||
                    escrow.status.tag === "Cancelled"
                      ? "bg-gray-200 text-gray-700"
                      : escrow.status.tag === "Disputed"
                        ? "bg-red-100 text-red-700"
                        : escrow.status.tag === "Funded"
                          ? "bg-blue-100 text-blue-700"
                          : "bg-amber-100 text-amber-700"
                  }`}
                >
                  {escrow.status.tag}
                </span>
              </dd>

              <dt className="font-semibold text-gray-500">Buyer</dt>
              <dd className="font-mono break-all text-gray-700">{escrow.buyer}</dd>

              <dt className="font-semibold text-gray-500">Seller</dt>
              <dd className="font-mono break-all text-gray-700">{escrow.seller}</dd>

              <dt className="font-semibold text-gray-500">Arbiter</dt>
              <dd className="font-mono break-all text-gray-700">{escrow.arbiter}</dd>

              <dt className="font-semibold text-gray-500">Token</dt>
              <dd className="font-mono break-all text-gray-700">{escrow.token}</dd>

              <dt className="font-semibold text-gray-500">Amount</dt>
              <dd className="font-mono">
                {formatAmount(escrow.amount)} ({escrow.amount.toString()} stroops)
              </dd>

              <dt className="font-semibold text-gray-500">Created At</dt>
              <dd className="font-mono">
                {new Date(Number(escrow.created_at) * 1000).toLocaleString()}
              </dd>

              <dt className="font-semibold text-gray-500">Timeout</dt>
              <dd className="font-mono">{escrow.timeout.toString()} seconds</dd>
            </dl>
          ) : (
            <p className="text-xs text-gray-500 italic">No escrow data loaded yet.</p>
          )}
        </section>

        {/* Section 5: My Escrows */}
        <section className="rounded-xl border border-gray-300 bg-white p-5 shadow-sm">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-lg font-semibold">5 · Participant Escrows</h2>
            <span className="rounded bg-gray-100 px-2 py-0.5 text-[11px] font-mono text-gray-600 border border-gray-200">
              {mode === "demo" ? "Local Index" : "On-Chain Participant Index"}
            </span>
          </div>

          <div className="flex items-center gap-3 mb-3">
            <button
              onClick={listMine}
              disabled={listBusy || (mode === "testnet" && !client)}
              className="rounded bg-gray-800 px-3 py-1.5 text-xs font-bold text-white hover:bg-gray-900 disabled:opacity-50"
            >
              {listBusy ? "Loading…" : "Refresh List"}
            </button>
            {listTotal !== null && (
              <span className="text-xs text-gray-600">
                {listTotal} escrow{listTotal === 1 ? "" : "s"} involving active address
              </span>
            )}
          </div>

          {myIds.length === 0 ? (
            <p className="text-xs text-gray-500">No escrows found for active address.</p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {myIds.map((id) => (
                <button
                  key={id.toString()}
                  onClick={() => {
                    const s = id.toString();
                    setReadEscrowId(s);
                    setActionEscrowId(s);
                    void fetchEscrow(s);
                  }}
                  className="rounded-lg border border-blue-600 bg-white px-2.5 py-1 font-mono text-xs font-semibold text-blue-600 hover:bg-blue-50"
                >
                  #{id.toString()}
                </button>
              ))}
            </div>
          )}
        </section>
      </div>

      {/* Transaction Log */}
      <section className="mt-6 rounded-xl border border-gray-300 bg-white p-5 shadow-sm">
        <div className="flex items-center justify-between mb-2">
          <h2 className="text-lg font-semibold">Activity &amp; Transaction Log</h2>
          <button
            onClick={() => setLog([])}
            className="text-xs text-gray-500 hover:text-gray-800 underline"
          >
            Clear log
          </button>
        </div>
        <div className="max-h-64 overflow-y-auto rounded-lg bg-gray-950 p-3 font-mono text-xs">
          {log.length === 0 ? (
            <p className="text-gray-500">No events logged yet…</p>
          ) : (
            log.map((entry) => (
              <p
                key={entry.id}
                className={`mb-1 break-all ${
                  entry.kind === "err"
                    ? "text-red-400"
                    : entry.kind === "ok"
                      ? "text-emerald-400"
                      : "text-gray-300"
                }`}
              >
                <span className="text-gray-500">[{entry.time}]</span>{" "}
                <span
                  className={`rounded px-1 text-[10px] uppercase font-bold mr-1 ${
                    entry.mode === "demo"
                      ? "bg-purple-900 text-purple-200"
                      : "bg-emerald-900 text-emerald-200"
                  }`}
                >
                  {entry.mode}
                </span>{" "}
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
