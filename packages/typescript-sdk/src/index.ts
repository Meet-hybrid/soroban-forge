/**
 * Soroban Forge — Multi-Contract TypeScript SDK
 *
 * Provides dedicated, typed client wrappers and network configurations for all
 * Soroban Forge smart contracts on Stellar.
 *
 * Subpath imports:
 *   import * as escrow from "@soroban-forge/escrow-client/escrow";
 *   import * as subscriptionPayments from "@soroban-forge/escrow-client/subscription-payments";
 *   import * as multiSigWallet from "@soroban-forge/escrow-client/multi-sig-wallet";
 *   import * as marketplaceRoyalties from "@soroban-forge/escrow-client/marketplace-royalties";
 *   import * as daoGovernance from "@soroban-forge/escrow-client/dao-governance";
 *   import * as vesting from "@soroban-forge/escrow-client/vesting";
 *
 * Namespace imports:
 *   import { escrow, subscriptionPayments, multiSigWallet } from "@soroban-forge/escrow-client";
 *
 * Backward-compatible root exports:
 *   import { Client, networks, type EscrowData } from "@soroban-forge/escrow-client";
 */

// Top-level namespace exports for all contracts
export * as escrow from "./contracts/escrow/index.js";
export * as subscriptionPayments from "./contracts/subscription-payments/index.js";
export * as multiSigWallet from "./contracts/multi-sig-wallet/index.js";
export * as marketplaceRoyalties from "./contracts/marketplace-royalties/index.js";
export * as daoGovernance from "./contracts/dao-governance/index.js";
export * as vesting from "./contracts/vesting/index.js";

// Centralized ForgeError registry and utilities
export * from "./errors.generated.js";

// Backward-compatible root re-exports for escrow client consumers
export * from "./contracts/escrow/index.js";