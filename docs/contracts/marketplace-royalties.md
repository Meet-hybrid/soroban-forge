# Marketplace Royalties Contract Reference

- **Crate**: `crates/marketplace-royalties`
- **Package**: `soroban-forge-marketplace-royalties`
- **Client**: `SorobanForgeMarketplaceRoyaltiesClient`
- **Contract Type**: `MarketplaceRoyalties`
- **Trait**: `SorobanForgeMarketplaceRoyalties`

---

## Overview

The Marketplace Royalties contract enforces creator royalty splits on secondary NFT sales. When an NFT is sold, sales proceeds are atomically split between the seller and up to 5 configured royalty recipients according to configured basis-point rates.

### Key Capabilities
- **Multi-Recipient Splits**: Supports up to 5 distinct royalty recipients (`MAX_ROYALTY_RECIPIENTS`) with individual basis-point rates.
- **Dust Retention with Seller**: Each recipient's royalty share is floored independently; fractional rounding dust remains with the seller.
- **Atomic Batch Settlements**: Settles up to 20 sales in a single transaction (`MAX_SETTLE_SALES`) under a single payer authorization with all-or-nothing rollback semantics.
- **Zero Custody Architecture**: The contract never holds token custody. Payments flow directly from the buyer/payer to the seller and royalty recipients.
- **Offline Quote Derivation**: The `quote_sale` entrypoint allows marketplaces to derive exact splits without executing on-chain transactions.

---

## Royalty Splitting & Settlement Mathematics

### Basis-Point Rate Specification
Royalty rates are denominated in basis points ($\text{bps}$), where $100\text{ bps} = 1.00\%$ and $10,000\text{ bps} = 100.00\%$. The sum of all recipient rates for a collection cannot exceed $10,000\text{ bps}$.

### Single-Recipient Math
For a gross sale amount $G$ and royalty rate $B$:

$$
\text{royalty\_share} = \left\lfloor \frac{G \cdot B}{10,000} \right\rfloor
$$

$$
\text{seller\_net} = G - \text{royalty\_share}
$$

Overflow protection: Amount $G$ is decomposed into quotient and remainder (`(G / 10_000) * B + ((G % 10_000) * B) / 10_000`), ensuring exact calculations up to `i128::MAX`.

### Multi-Recipient Math
For a list of recipients $[R_1, \dots, R_k]$ with rates $[B_1, \dots, B_k]$:

$$
S_i = \left\lfloor \frac{G \cdot B_i}{10,000} \right\rfloor \quad \text{for } i \in \{1, \dots, k\}
$$

$$
\text{royalty\_total} = \sum_{i=1}^k S_i
$$

$$
\text{seller\_net} = G - \text{royalty\_total}
$$

**Dust Retention Property**: Because each recipient share $S_i$ is floored independently:

$$
\sum_{i=1}^k \left\lfloor \frac{G \cdot B_i}{10,000} \right\rfloor \le \left\lfloor \frac{G \cdot \sum B_i}{10,000} \right\rfloor
$$

Any fractional unallocated dust from integer division remains with the seller, guaranteeing:

$$
\text{seller\_net} + \sum_{i=1}^k S_i = G
$$

---

## Settlement Lifecycles & Modes

