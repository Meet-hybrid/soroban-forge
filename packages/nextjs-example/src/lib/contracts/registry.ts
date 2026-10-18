import { contract } from "@soroban-forge/escrow-client";
import {
  ESCROW_SPEC,
  VESTING_SPEC,
  MULTI_SIG_SPEC,
  DAO_SPEC,
  SUBSCRIPTION_SPEC,
  MARKETPLACE_SPEC,
} from "./specs.ts";

export type ContractKey =
  | "escrow"
  | "vesting"
  | "multi-sig-wallet"
  | "dao-governance"
  | "subscription-payments"
  | "marketplace-royalties";

export interface ContractConfig {
  key: ContractKey;
  name: string;
  badge: string;
  tagline: string;
  defaultContractId: string;
  spec: string[];
  writeActionName: string;
  writeActionDoc: string;
}

export const TESTNET_RPC_URL = "https://soroban-testnet.stellar.org";
export const TESTNET_PASSPHRASE = "Test SDF Network ; September 2015";
export const DEFAULT_TOKEN_ID = "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

export const CONTRACTS: Record<ContractKey, ContractConfig> = {
  escrow: {
    key: "escrow",
    name: "Escrow",
    badge: "3-Party Custody",
    tagline: "Trustless three-party escrow with buyer funding, seller fulfilment, and arbiter dispute resolution.",
    defaultContractId: "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
    spec: ESCROW_SPEC,
    writeActionName: "create_escrow",
    writeActionDoc: "Initialize a new 3-party escrow record with buyer, seller, arbiter, token, amount, and expiry timeout.",
  },
  vesting: {
    key: "vesting",
    name: "Vesting",
    badge: "Linear + Cliff",
    tagline: "Token vesting schedule with configurable cliff and linear unlock over a designated duration.",
    defaultContractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM",
    spec: VESTING_SPEC,
    writeActionName: "create_schedule",
    writeActionDoc: "Create a new linear vesting schedule for a beneficiary with a defined cliff and total duration.",
  },
  "multi-sig-wallet": {
    key: "multi-sig-wallet",
    name: "Multi-Sig Wallet",
    badge: "M-of-N Threshold",
    tagline: "Multi-signature treasury wallet with threshold-gated execution, rejections, and preview queries.",
    defaultContractId: "CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBAD2KM",
    spec: MULTI_SIG_SPEC,
    writeActionName: "submit_withdrawal",
    writeActionDoc: "Propose a new token withdrawal transaction for multi-sig owner confirmation and threshold approval.",
  },
  "dao-governance": {
    key: "dao-governance",
    name: "DAO Governance",
    badge: "Proposal Voting",
    tagline: "Decentralized proposal creation, bond custody, token-weighted voting, and automated execution.",
    defaultContractId: "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCAD2KM",
    spec: DAO_SPEC,
    writeActionName: "vote",
    writeActionDoc: "Cast a definitive 'for' or 'against' vote on an active governance proposal before the voting deadline.",
  },
  "subscription-payments": {
    key: "subscription-payments",
    name: "Subscription Payments",
    badge: "Recurring Billing",
    tagline: "Periodic subscription billing engine with subscriber authorization, pause, resume, and cancellation.",
    defaultContractId: "CDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDAD2KM",
    spec: SUBSCRIPTION_SPEC,
    writeActionName: "subscribe",
    writeActionDoc: "Create a periodic subscription agreement authorizing regular billing from subscriber to provider.",
  },
  "marketplace-royalties": {
    key: "marketplace-royalties",
    name: "Marketplace Royalties",
    badge: "EIP-2981 Splits",
    tagline: "On-chain royalty management and atomic split settlement for secondary NFT marketplace sales.",
    defaultContractId: "CEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEAD2KM",
    spec: MARKETPLACE_SPEC,
    writeActionName: "set_royalty",
    writeActionDoc: "Register or update the basis-point royalty rate and payout recipient for an NFT collection contract.",
  },
};

export interface SignerCallback {
  (
    xdr: string,
    opts?: { networkPassphrase?: string; address?: string }
  ): Promise<{ signedTxXdr: string }>;
}

export interface ClientFactoryOptions {
  rpcUrl?: string;
  networkPassphrase?: string;
  contractId: string;
  publicKey?: string;
  signTransaction?: SignerCallback;
}

/**
 * Creates an instantiated Soroban contract client with full dynamic method bindings.
 */
export function createContractClient(
  contractKey: ContractKey,
  options: ClientFactoryOptions
): any {
  const config = CONTRACTS[contractKey];
  const specInstance = new contract.Spec(config.spec);

  return new contract.Client(specInstance, {
    rpcUrl: options.rpcUrl ?? TESTNET_RPC_URL,
    networkPassphrase: options.networkPassphrase ?? TESTNET_PASSPHRASE,
    contractId: options.contractId,
    publicKey: options.publicKey,
    signTransaction: options.signTransaction,
  });
}
