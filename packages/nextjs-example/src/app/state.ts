/**
 * Transaction-state and environment machine for the Soroban Forge Next.js example.
 *
 * Provides deterministic state transitions for:
 * 1. Execution environment mode (Local Demo vs Live Testnet).
 * 2. Transaction lifecycle stages (idle -> simulating -> signing -> submitting -> confirmed / failed).
 * 3. Prerequisites evaluation for live transactions.
 * 4. In-memory demo escrow state transitions for local experimentation without network access.
 */

export type ExecutionMode = "demo" | "testnet";

export type TransactionStage =
  | "idle"
  | "simulating"
  | "signing"
  | "submitting"
  | "confirmed"
  | "failed";

export interface TransactionState {
  stage: TransactionStage;
  action: string | null;
  mode: ExecutionMode;
  hash: string | null;
  error: string | null;
  details: string | null;
  timestamp: number | null;
}

export type TransactionAction =
  | { type: "START_SIMULATION"; action: string; mode: ExecutionMode }
  | { type: "START_SIGNING" }
  | { type: "START_SUBMISSION"; hash?: string }
  | { type: "CONFIRM"; hash?: string; details?: string }
  | { type: "FAIL"; error: string }
  | { type: "RESET" }
  | { type: "SET_MODE"; mode: ExecutionMode };

export function initialTransactionState(mode: ExecutionMode = "demo"): TransactionState {
  return {
    stage: "idle",
    action: null,
    mode,
    hash: null,
    error: null,
    details: null,
    timestamp: null,
  };
}

export function transactionReducer(
  state: TransactionState,
  action: TransactionAction
): TransactionState {
  switch (action.type) {
    case "SET_MODE":
      return {
        ...initialTransactionState(action.mode),
        mode: action.mode,
      };

    case "START_SIMULATION":
      return {
        stage: "simulating",
        action: action.action,
        mode: action.mode,
        hash: null,
        error: null,
        details: null,
        timestamp: Date.now(),
      };

    case "START_SIGNING":
      if (state.stage !== "simulating" && state.stage !== "idle") {
        return state;
      }
      return {
        ...state,
        stage: "signing",
      };

    case "START_SUBMISSION":
      if (state.stage !== "signing" && state.stage !== "simulating") {
        return state;
      }
      return {
        ...state,
        stage: "submitting",
        hash: action.hash ?? state.hash,
      };

    case "CONFIRM":
      return {
        ...state,
        stage: "confirmed",
        hash: action.hash ?? state.hash,
        details: action.details ?? null,
        error: null,
        timestamp: Date.now(),
      };

    case "FAIL":
      return {
        ...state,
        stage: "failed",
        error: action.error,
        details: null,
        timestamp: Date.now(),
      };

    case "RESET":
      return initialTransactionState(state.mode);

    default:
      return state;
  }
}

export interface NetworkPrerequisite {
  id: string;
  label: string;
  description: string;
  satisfied: boolean;
  category: "wallet" | "network" | "balance";
  actionHint?: string;
}

export function evaluatePrerequisites(opts: {
  walletMounted: boolean;
  connectedAccount: string | null;
  networkPassphrase: string;
  expectedPassphrase: string;
}): NetworkPrerequisite[] {
  const isWalletMounted = opts.walletMounted;
  const isConnected = Boolean(opts.connectedAccount);
  const isTestnetPassphrase = opts.networkPassphrase === opts.expectedPassphrase;

  return [
    {
      id: "wallet-connectors",
      label: "Browser Wallet Connector",
      description: "Stellar Wallets Kit connector initialized in browser context.",
      satisfied: isWalletMounted,
      category: "wallet",
      actionHint: isWalletMounted ? undefined : "Awaiting client hydration...",
    },
    {
      id: "installed-wallet",
      label: "Connected Testnet Account",
      description: "Freighter, xBull, Albedo, or Bitget wallet connected.",
      satisfied: isConnected,
      category: "wallet",
      actionHint: isConnected ? undefined : "Click 'Connect wallet' to authenticate.",
    },
    {
      id: "network-target",
      label: "Stellar Testnet Network",
      description: `Must be configured for Test SDF Network (passphrase: ${opts.expectedPassphrase}).`,
      satisfied: isTestnetPassphrase,
      category: "network",
      actionHint: isTestnetPassphrase
        ? undefined
        : "Switch wallet network to Stellar Testnet (NOT Mainnet).",
    },
    {
      id: "account-funding",
      label: "Testnet XLM Balance",
      description: "Account must hold testnet XLM for transaction fees and reserves.",
      satisfied: isConnected,
      category: "balance",
      actionHint: isConnected
        ? undefined
        : "Use the 'Fund with Friendbot' button after connecting.",
    },
  ];
}

export type EscrowStatusTag =
  | "Pending"
  | "Funded"
  | "Completed"
  | "Refunded"
  | "Disputed"
  | "Cancelled";

export interface DemoEscrowRecord {
  escrow_id: bigint;
  buyer: string;
  seller: string;
  arbiter: string;
  token: string;
  amount: bigint;
  timeout: bigint;
  created_at: bigint;
  status: {
    tag: EscrowStatusTag;
    values: void;
  };
}

export interface DemoAccount {
  role: "buyer" | "seller" | "arbiter";
  name: string;
  address: string;
  balanceXlm: number;
}

export const DEMO_ACCOUNTS: Record<"buyer" | "seller" | "arbiter", DemoAccount> = {
  buyer: {
    role: "buyer",
    name: "Demo Buyer",
    address: "GDEMO_BUYER_7K4J3X9Q2W8M1N6P0R5T7V9X2Z4B6D8F",
    balanceXlm: 100,
  },
  seller: {
    role: "seller",
    name: "Demo Seller",
    address: "GDEMO_SELLER_8L5K4Y0R3X9N2P7Q1S6U8W0Y3A5C7E9G",
    balanceXlm: 20,
  },
  arbiter: {
    role: "arbiter",
    name: "Demo Arbiter",
    address: "GDEMO_ARBITER_9M6L5Z1S4Y0P3Q8R2T7V9X1Z4B6D8F0H",
    balanceXlm: 10,
  },
};

export function transitionDemoEscrow(
  escrow: DemoEscrowRecord,
  action: "deposit" | "release" | "refund" | "cancel" | "dispute" | "resolve",
  opts?: { claimant?: string; in_favor_of_seller?: boolean }
): { success: true; updated: DemoEscrowRecord } | { success: false; error: string } {
  const currentStatus = escrow.status.tag;

  switch (action) {
    case "deposit":
      if (currentStatus !== "Pending") {
        return {
          success: false,
          error: `Cannot deposit: escrow ${escrow.escrow_id} is in status '${currentStatus}', expected 'Pending'.`,
        };
      }
      return {
        success: true,
        updated: { ...escrow, status: { tag: "Funded", values: undefined } },
      };

    case "release":
      if (currentStatus !== "Funded") {
        return {
          success: false,
          error: `Cannot release: escrow ${escrow.escrow_id} is in status '${currentStatus}', expected 'Funded'.`,
        };
      }
      return {
        success: true,
        updated: { ...escrow, status: { tag: "Completed", values: undefined } },
      };

    case "refund":
      if (currentStatus !== "Funded") {
        return {
          success: false,
          error: `Cannot refund: escrow ${escrow.escrow_id} is in status '${currentStatus}', expected 'Funded'.`,
        };
      }
      return {
        success: true,
        updated: { ...escrow, status: { tag: "Refunded", values: undefined } },
      };

    case "cancel":
      if (currentStatus !== "Pending") {
        return {
          success: false,
          error: `Cannot cancel: escrow ${escrow.escrow_id} is in status '${currentStatus}', expected 'Pending'.`,
        };
      }
      return {
        success: true,
        updated: { ...escrow, status: { tag: "Cancelled", values: undefined } },
      };

    case "dispute":
      if (currentStatus !== "Funded") {
        return {
          success: false,
          error: `Cannot dispute: escrow ${escrow.escrow_id} is in status '${currentStatus}', expected 'Funded'.`,
        };
      }
      if (opts?.claimant) {
        const claimant = opts.claimant.trim();
        if (claimant !== escrow.buyer && claimant !== escrow.seller) {
          return {
            success: false,
            error: `Dispute claimant must be either buyer (${escrow.buyer}) or seller (${escrow.seller}).`,
          };
        }
      }
      return {
        success: true,
        updated: { ...escrow, status: { tag: "Disputed", values: undefined } },
      };

    case "resolve":
      if (currentStatus !== "Disputed") {
        return {
          success: false,
          error: `Cannot resolve: escrow ${escrow.escrow_id} is in status '${currentStatus}', expected 'Disputed'.`,
        };
      }
      const targetTag: EscrowStatusTag = opts?.in_favor_of_seller ? "Completed" : "Refunded";
      return {
        success: true,
        updated: { ...escrow, status: { tag: targetTag, values: undefined } },
      };

    default:
      return { success: false, error: `Unknown action '${action}'` };
  }
}
