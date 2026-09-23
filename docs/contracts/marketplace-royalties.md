# Marketplace Royalties Contract

NFT / digital asset sales with configurable royalty distribution across
secondary sales — computed by `distribute` and settled atomically in real
SEP-41 tokens by `settle_sale`.

## Interface

```rust
fn set_royalty(collection, recipient, bps) -> Result<(), ForgeError>
fn distribute(collection, seller, amount) -> Result<i128, ForgeError>
fn settle_sale(collection, token, payer, seller, amount) -> Result<Settlement, ForgeError>
fn get_royalty(collection) -> Result<Royalty, ForgeError>
fn get_settlement_summary(collection) -> Result<SettlementSummary, ForgeError>
```

## Concepts

- **Creators** receive a split of every sale, paid to the configured
  `recipient` on settlement.
- **Sellers** receive the net of every sale.
- **Payers** fund both transfers from their `token` balance in a single
  authorized call.

## Settlement

`settle_sale` moves one sale's proceeds with escrow's transfer-before-state
ordering:

1. Load the collection's configuration (`NotFound` if unregistered), validate
   `amount > 0` (`InvalidInput`), and require the collection's and the payer's
   authorization.
2. Compute `royalty_share = amount * bps / 10_000` (floored) and
   `seller_net = amount - royalty_share` with checked arithmetic
   (`ArithmeticOverflow`), and stage the updated settlement totals. The two
   parts always sum exactly to `amount` — rounding dust stays with the
   seller.
3. Transfer `seller_net` from `payer` to `seller`, then `royalty_share` from
   `payer` to the configured `recipient` — the recipient is paid last.
4. Only after both transfers succeed, commit the collection's cumulative
   settlement totals and return the `Settlement`.

A `Disabled` or zero-bps configuration skips the recipient transfer and
settles the full amount to the seller. Token failures (insufficient balance,
missing trustline, undeployed token) surface as
`ForgeError::TokenTransferFailed`, and any returned error rolls back the
whole invocation — a failed settlement can never leave the recipient
partially paid and never commits totals.

`distribute` stays a pure computation for callers that only need the net; it
moves no tokens.

## Compatibility

`set_royalty`, `distribute`, and `get_royalty` are unchanged. `settle_sale`
and `get_settlement_summary` are additive; the generated
`SorobanForgeMarketplaceRoyaltiesClient` gains both automatically.

## Storage

Instance storage: one `Royalty` record and one `SettlementSummary` record
per collection.
