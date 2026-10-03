# Escrow Contract Reference

- **Crate**: `soroban-forge-escrow`
- **Wasm Target**: `wasm32v1-none`
- **Rust Client**: `SorobanForgeEscrowClient`
- **TypeScript Client**: `@soroban-forge/escrow-client` (`Client`)

---

## 1. Overview & Architectural Role

The Escrow contract provides trust-minimized, three-party token custody (`buyer`, `seller`, `arbiter`) for secondary transactions, milestone disbursements, and cross-contract trade settlement. It supports:
1. **Single-Token Escrows**: Custodies a single SEP-41 token with support for full releases, partial milestones, voluntary refunds, timeout reclaims, and dispute splits.
2. **Multi-Asset Baskets**: Holds up to 8 distinct tokens in a single escrow ID with atomic all-or-nothing deposits and independent leg tracking.
3. **Participant Indexing**: Maintains an on-chain, creation-ordered index of non-cancelled escrows for buyers, sellers, and arbiters.

---

## 2. API Reference

### Single-Token Entrypoints

#### `create_escrow`
Initializes a new single-token escrow. Only the `buyer` authorizes creation.
