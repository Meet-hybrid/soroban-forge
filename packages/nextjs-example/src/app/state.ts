/**
 * Transaction-state and environment machine for the Soroban Forge Next.js example.
 *
 * Provides deterministic state transitions for:
 * 1. Execution environment mode (Local Demo vs Live Testnet).
 * 2. Transaction lifecycle stages (idle -> simulating -> signing -> submitting -> confirmed / failed).
 * 3. Prerequisites evaluation for live transactions.
 * 4. In-memory demo state experimentation without network access.
 * 5. Support for all six Soroban Forge contracts.
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
  contract?: string;
}

export type TransactionAction =
  | { type: "START_SIMULATION"; action: string; mode: ExecutionMode; contract?: string }
  | { type: "START_SIGNING" }
  | { type: "START_SUBMISSION"; hash?: string }
  | { type: "CONFIRM"; hash?: string; details?: string; contract?: string }
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
        contract: action.contract,
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

export type ContractType =
  | "escrow"
  | "vesting"
  | "multisig"
  | "dao"
  | "subscription"
  | "marketplace";

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

// --- Contract-specific read result types ---

export interface EscrowReadResult {
  escrow_id: bigint;
  buyer: string;
  seller: string;
  arbiter: string;
  token: string;
  amount: bigint;
  timeout: bigint;
  created_at: bigint;
  status: {
    tag: "Pending" | "Funded" | "Completed" | "Refunded" | "Disputed" | "Cancelled";
    values: void;
  };
}

export interface VestingReadResult {
  schedule_id: bigint;
  start: bigint;
  total_amount: bigint;
  claimed: bigint;
  cliff: bigint;
  duration: bigint;
  beneficiary: string;
  token: string;
  status: "Locked" | "Vesting" | "Completed" | "Revoked";
}

export interface MultisigReadResult {
  threshold: bigint;
  owners: string[];
  status: "Pending" | "Executed" | "Rejected" | "Expired";
  tx_count: bigint;
}

export interface DaoReadResult {
  proposal_count: bigint;
  active_proposal_count: bigint;
  bond_state: "Posted" | "Refunded" | "Forfeited";
}

export interface SubscriptionReadResult {
  subscription_id: bigint;
  status: "Active" | "Cancelled" | "PastDue" | "Paused";
  provider: string;
  subscriber: string;
  amount: bigint;
  period: bigint;
  token: string;
}

export interface MarketplaceReadResult {
  collection: string;
  royalty_bps: bigint;
  recipient: string;
  status: "Active" | "Disabled";
}

// --- Demo record types for local simulation ---

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
    tag: "Pending" | "Funded" | "Completed" | "Refunded" | "Disputed" | "Cancelled";
    values: undefined;
  };
}

export interface DemoVestingRecord {
  schedule_id: bigint;
  start: bigint;
  total_amount: bigint;
  claimed: bigint;
  cliff: bigint;
  duration: bigint;
  beneficiary: string;
  token: string;
  status: "Locked" | "Vesting" | "Completed" | "Revoked";
}

export type DemoRecord = DemoEscrowRecord | DemoVestingRecord;

// --- DEMO ACCOUNTS for all supported roles ---

export interface DemoAccount {
  role:
    | "buyer"
    | "seller"
    | "arbiter"
    | "funder"
    | "beneficiary"
    | "proposer"
    | "voter"
    | "subscriber"
    | "provider";
  name: string;
  address: string;
  balanceXlm: number;
}

export const DEMO_ACCOUNTS: Record<
  "buyer" | "seller" | "arbiter" | "funder" | "beneficiary" | "proposer" | "voter" | "subscriber" | "provider",
  DemoAccount
> = {
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
  funder: {
    role: "funder",
    name: "Demo Funder",
    address: "GDEMO_FUNDER_2A3B4C5D6E7F8G9H0I1J2K3L4M5N6O7P",
    balanceXlm: 500,
  },
  beneficiary: {
    role: "beneficiary",
    name: "Demo Beneficiary",
    address: "GDEMO_BENEFICIARY_8P7Q6R5S4T3U2V1W0X9Y8Z6A5B4C3D2",
    balanceXlm: 100,
  },
  proposer: {
    role: "proposer",
    name: "Demo Proposer",
    address: "GDEMO_PROPOSER_1F2E3D4C5B6A7Z8Y9X6W5V4U3T2S1",
    balanceXlm: 50,
  },
  voter: {
    role: "voter",
    name: "Demo Voter",
    address: "GDEMO_VOTER_9Z8Y7X6W5V4U3T2S1R0Q9P8O7N6M5L4K",
    balanceXlm: 30,
  },
  subscriber: {
    role: "subscriber",
    name: "Demo Subscriber",
    address: "GDEMO_SUBSCRIBER_5A4B3C2D1E0F9A8B7C6D5E4F3A2B1C0",
    balanceXlm: 200,
  },
  provider: {
    role: "provider",
    name: "Demo Provider",
    address: "GDEMO_PROVIDER_1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6",
    balanceXlm: 300,
  },
};

// --- Demo escrow state (in-memory) ---

export const demoEscrows: Record<string, DemoEscrowRecord> = {
  "1": {
    escrow_id: BigInt(1),
    buyer: DEMO_ACCOUNTS.buyer.address,
    seller: DEMO_ACCOUNTS.seller.address,
    arbiter: DEMO_ACCOUNTS.arbiter.address,
    token: "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O",
    amount: BigInt(10000000),
    timeout: BigInt(3600),
    created_at: BigInt(1700000000),
    status: { tag: "Pending", values: undefined },
  },
};

// --- Contract transition functions for demo simulation ---

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
      const targetTag: "Completed" | "Refunded" = opts?.in_favor_of_seller ? "Completed" : "Refunded";
      return {
        success: true,
        updated: { ...escrow, status: { tag: targetTag, values: undefined } },
      };

    default:
      return { success: false, error: `Unknown action '${action}'` };
  }
}

// --- Vesting demo transitions ---

export function transitionDemoVesting(
  schedule: DemoVestingRecord,
  action: "claim",
  opts?: { beneficiary?: string }
): { success: true; updated: DemoVestingRecord } | { success: false; error: string } {
  const now = BigInt(Math.floor(Date.now() / 1000));
  const status = schedule.status;

  if (status === "Completed") {
    return {
      success: false,
      error: `Cannot claim: schedule #${schedule.schedule_id} is already completed.`,
    };
  }

  if (status === "Revoked") {
    return {
      success: false,
      error: `Cannot claim: schedule #${schedule.schedule_id} has been revoked.`,
    };
  }

  const cliffTime = schedule.start + schedule.cliff;
  if (now < cliffTime) {
    return {
      success: false,
      error: `Cannot claim: current time is before cliff (${cliffTime.toString()}).`,
    };
  }

  const endTime = schedule.start + schedule.duration;
  if (now >= endTime) {
    return {
      success: true,
      updated: {
        ...schedule,
        claimed: schedule.total_amount,
        status: "Completed",
      },
    };
  }

  const vestingPeriod = endTime - cliffTime;
  if (vestingPeriod <= BigInt(0)) {
    return {
      success: true,
      updated: {
        ...schedule,
        claimed: schedule.total_amount,
        status: "Completed",
      },
    };
  }

  const amount = (schedule.total_amount * (now - cliffTime)) / vestingPeriod;
  if (amount <= BigInt(0)) {
    return {
      success: false,
      error: "No tokens claimable at this time.",
    };
  }

return {
    success: true,
    updated: {
      ...schedule,
      claimed: schedule.claimed + amount,
      status: "Vesting",
    }
  };
}

// --- Demo vesting state ---

export const demoVestingSchedules: Record<string, DemoVestingRecord> = {};

// --- Generic forge error utility ---

export function formatForgeError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "object" && error !== null && "message" in error) {
    return String(error.message);
  }
  return String(error);
}

// --- Testnet-only warning helper ---

export function testnetWarning(mode: ExecutionMode): string {
  if (mode === "testnet") {
    return (
      "⚠️ **Testnet Only**: This action interacts with Stellar Testnet. " +
      "No real funds are involved in this demonstration. " +
      "Do not use mainnet keys or real SEP-41 tokens."
    );
  }
  return "";
}