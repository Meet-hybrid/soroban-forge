# Contracts Overview

| Contract | Path | Status |
|----------|------|--------|
| Escrow | crates/escrow | Alpha (skeleton) |
| Vesting | crates/vesting | Alpha (skeleton) |
| Multi-Sig Wallet | crates/multi-sig-wallet | Alpha (skeleton) |
| DAO Governance | crates/dao-governance | Alpha (skeleton) |
| Subscription Payments | crates/subscription-payments | Alpha (skeleton) |
| Marketplace Royalties | crates/marketplace-royalties | Alpha (skeleton) |

## Adding a New Contract

1. Add a new crate under `crates/`.
2. Register it in workspace `Cargo.toml`.
3. Add a document in `docs/contracts/`.
4. Add CI checks.
5. Tag a release.
