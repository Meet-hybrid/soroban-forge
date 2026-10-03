# Escrow Contract Reference

- **Crate**: `crates/escrow`
- **Package**: `soroban-forge-escrow`
- **Client**: `SorobanForgeEscrowClient`
- **Contract Type**: `Escrow`
- **Trait**: `SorobanForgeEscrow`

---

## Overview

The Escrow contract implements trust-minimized, three-party token custody on Stellar. A buyer, seller, and neutral arbiter agree on terms for a single SEP-41 token or an atomic multi-asset basket. Funds are held in contract custody until released to the seller, refunded to the buyer, or arbitrated by the arbiter.

### Key Capabilities
- **Single-Token & Multi-Asset Baskets**: Supports single-token agreements as well as atomic baskets of up to 8 distinct tokens (`MAX_BASKET_ASSETS`).
- **Milestone Partial Releases**: Allows incremental payouts to the seller while preserving the remaining balance in custody.
- **Dispute Resolution & Basis-Point Splits**: The arbiter can award the remaining balance entirely to one party or settle an exact basis-point proportional split.
- **Permissionless Expiry Refund**: Anyone may trigger a refund to the buyer once the timeout deadline has strictly passed.
- **Participant Indexing**: Per-participant persistent index for discovery and paginated querying of active and historical escrows.

---

## State Machine & Lifecycle

### Single-Token Escrow Lifecycle

