# Marketplace Royalties Contract

NFT / digital asset sales with configurable royalty distribution across secondary sales.

## Interface

```rust
fn register_asset(creator, asset_id, url) -> Result<(), ForgeError>
fn create_sale(asset_id, seller, price) -> Result<(), ForgeError>
fn purchase(asset_id, buyer, price) -> Result<(), ForgeError>
fn update_royalties(asset_id, recipients) -> Result<(), ForgeError>
fn get_royalty_info(asset_id) -> Result<Vec<(Address, i128)>, ForgeError>
```

## Concepts

- **Creators** receive a split of every sale.
- **Sellers** list assets.
- **Buyers** purchase assets, splitting payment between seller and royalties.
