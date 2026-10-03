# Marketplace Royalties Contract Reference

- **Crate**: `soroban-forge-marketplace-royalties`
- **Wasm Target**: `wasm32v1-none`
- **Rust Client**: `SorobanForgeMarketplaceRoyaltiesClient`
- **TypeScript Client**: `@soroban-forge/marketplace-royalties-client` (`Client`)

---

## 1. Overview & Architectural Role

The Marketplace Royalties contract enforces creator royalty splits on secondary sales of digital collectibles and tokens:
1. **Flexible Royalty Configurations**: Supports single-recipient royalties or proportional multi-recipient splits (up to 5 recipients) with basis-point accuracy.
2. **Atomic Sale Settlement**: Atomically transfers net proceeds to the seller and royalty cuts to creators in real SEP-41 tokens.
3. **Batch Settlements**: Settles up to 20 sales in a single invocation covered by a single payer authorization.
4. **Pre-Flight Quotations**: Simulates settlement splits using the contract's exact mathematical rounding logic.

---

## 2. API Reference

### Royalty Configuration

#### `set_royalty`
Registers or updates a single-recipient royalty rate for a collection.
