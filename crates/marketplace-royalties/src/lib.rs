#![no_std]

//! # Soroban Forge — Marketplace Royalties contract
//!
//! Enforces creator royalty splits on secondary sales: when an NFT changes
//! hands, the sale proceeds are split between the seller and one or more
//! royalty recipients according to configured basis-point rates. This
//! iteration stores one royalty configuration per collection, with a single
//! recipient; `settle_sale` moves the computed split in real SEP-41 tokens.
//!
//! Flow:
//!
//! ```text
//! set_royalty(collection, recipient, bps)   -> Active config
//! distribute(collection, seller, amount)    -> computes `amount * bps / 10_000`
//!                                              for the recipient and returns
//!                                              the net owed to the seller
//! settle_sale(collection, token, payer,
//!             seller, amount)               -> transfers the seller net, then
//!                                              the royalty share, then commits
//!                                              the settlement totals
//! ```
//!
//! Authorization model:
//! - `set_royalty` requires the collection (the contract whose config this
//!   is), and `bps` must not exceed 100% (10_000 bps).
//! - `distribute` requires the collection and returns the seller's net after
//!   the configured royalty split; a `Disabled` configuration settles in full
//!   to the seller. It computes only and moves no tokens.
//! - `settle_sale` requires the collection (as `distribute` does) and the
//!   `payer`, whose balance funds both transfers; the payer's authorization
//!   covers the nested token invocations exactly as escrow's does.
//! - `get_royalty` and `get_settlement_summary` are read-only views.
//!
//! Settlement follows escrow's transfer-before-state ordering: both token
//! transfers run before any settlement state is committed, the royalty
//! recipient is paid last so a failed transfer can never leave it partially
//! paid, and token failures are bucketed into `TokenTransferFailed`. Any
//! returned error rolls the whole invocation back. Multiple recipients per
//! collection and per-token royalties remain out of scope for this iteration.

#[cfg(test)]
extern crate std;

use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::{contract, contractclient, contractimpl, contracttype, token, Address, Env};

/// Public interface for the Soroban Forge marketplace royalties contract.
#[contractclient(name = "SorobanForgeMarketplaceRoyaltiesClient")]
pub trait SorobanForgeMarketplaceRoyalties {
    /// Register or update the royalty recipient and basis-point rate for
    /// `collection`.
    fn set_royalty(
        env: Env,
        collection: Address,
        recipient: Address,
        bps: u32,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Distribute `amount` from a sale of `collection`, returning the net to
    /// the seller after royalties. Pure computation: no tokens move.
    fn distribute(
        env: Env,
        collection: Address,
        seller: Address,
        amount: i128,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Settle a sale of `collection` atomically: transfer the seller's net
    /// and the royalty share from `payer` in `token`, then commit the
    /// collection's cumulative settlement totals.
    fn settle_sale(
        env: Env,
        collection: Address,
        token: Address,
        payer: Address,
        seller: Address,
        amount: i128,
    ) -> Result<Settlement, soroban_forge_shared_utils::ForgeError>;

    /// Read the stored royalty configuration for `collection` (read-only view).
    fn get_royalty(
        env: Env,
        collection: Address,
    ) -> Result<Royalty, soroban_forge_shared_utils::ForgeError>;

    /// Read the cumulative settlement totals for `collection` (read-only
    /// view); `NotFound` until the first successful settlement.
    fn get_settlement_summary(
        env: Env,
        collection: Address,
    ) -> Result<SettlementSummary, soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a registered royalty configuration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoyaltyStatus {
    /// Active and applied to sales.
    Active,
    /// Disabled; sales settle to the seller in full.
    Disabled,
}

/// A royalty configuration for a single collection.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Royalty {
    /// Collection (NFT contract) this configuration applies to.
    pub collection: Address,
    /// Address entitled to royalty payments.
    pub recipient: Address,
    /// Royalty rate in basis points (100 bps = 1%).
    pub bps: u32,
    /// Whether the configuration is currently enforced.
    pub status: RoyaltyStatus,
}

/// The two amounts one atomic `settle_sale` invocation transferred.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settlement {
    /// Amount transferred to the configured royalty recipient.
    pub royalty_share: i128,
    /// Amount transferred to the seller.
    pub seller_net: i128,
}

/// Cumulative settlement totals for one collection.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementSummary {
    /// Number of sales settled so far.
    pub sales: u32,
    /// Sum of every settled sale amount.
    pub gross_volume: i128,
    /// Sum of every royalty share transferred to the recipient.
    pub royalties_paid: i128,
}

/// Instance-storage keys.
#[contracttype]
enum DataKey {
    /// The royalty configuration for `Address` collection.
    Royalty(Address),
    /// The cumulative settlement totals for `Address` collection.
    Summary(Address),
}

/// The deployable marketplace royalties contract.
#[contract]
pub struct MarketplaceRoyalties;

#[contractimpl]
impl MarketplaceRoyalties {
    /// Register or update a royalty configuration for `collection`.
    ///
    /// Requires the collection's authorization and `bps <= 10_000`
    /// (100%). Re-registration updates the existing configuration in place.
    pub fn set_royalty(
        env: Env,
        collection: Address,
        recipient: Address,
        bps: u32,
    ) -> Result<(), ForgeError> {
        if bps > 10_000 {
            return Err(ForgeError::InvalidInput);
        }
        collection.require_auth();

        let royalty = Royalty {
            collection,
            recipient,
            bps,
            status: RoyaltyStatus::Active,
        };
        env.storage()
            .instance()
            .set(&DataKey::Royalty(royalty.collection.clone()), &royalty);
        Ok(())
    }

    /// Compute the royalty split for a sale.
    ///
    /// Requires the collection's authorization and `amount > 0`. Returns the
    /// net owed to `seller` after reserving `amount * bps / 10_000` for the
    /// configured recipient. A `Disabled` configuration settles in full.
    /// This entrypoint is a pure computation and moves no tokens; use
    /// `settle_sale` to transfer the split in real SEP-41 tokens.
    pub fn distribute(
        env: Env,
        collection: Address,
        seller: Address,
        amount: i128,
    ) -> Result<i128, ForgeError> {
        let royalty: Royalty = env
            .storage()
            .instance()
            .get(&DataKey::Royalty(collection.clone()))
            .ok_or(ForgeError::NotFound)?;
        if amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        collection.require_auth();

        let _ = &seller;
        let (_, seller_net) = split(amount, effective_bps(&royalty))?;
        Ok(seller_net)
    }

    /// Settle a sale of `collection` atomically in `token`.
    ///
    /// Requires the collection's authorization (as `distribute` does)
    /// and the `payer`'s, which covers both nested token transfers. Requires
    /// `amount > 0` and a stored configuration. The split is computed with
    /// checked arithmetic, and both transfers run **before any settlement
    /// state is committed**: the seller's net first, the royalty recipient
    /// last, so a failed transfer can never leave the royalty recipient
    /// partially paid. A `Disabled` or zero-bps configuration settles the
    /// full amount to the seller in a single transfer. Token failures are
    /// bucketed into [`ForgeError::TokenTransferFailed`], and any returned
    /// error rolls the whole invocation back — including an earlier
    /// successful transfer — so retrying after a failure never double-pays.
    pub fn settle_sale(
        env: Env,
        collection: Address,
        token: Address,
        payer: Address,
        seller: Address,
        amount: i128,
    ) -> Result<Settlement, ForgeError> {
        let royalty: Royalty = env
            .storage()
            .instance()
            .get(&DataKey::Royalty(collection.clone()))
            .ok_or(ForgeError::NotFound)?;
        if amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        collection.require_auth();
        payer.require_auth();

        // Every fallible computation runs before the first transfer, so an
        // arithmetic failure can never strand funds mid-settlement.
        let (royalty_share, seller_net) = split(amount, effective_bps(&royalty))?;
        let summary = next_summary(&env, &collection, amount, royalty_share)?;

        // Transfer-before-state (escrow pattern): the seller is paid first
        // and the royalty recipient last, so the protected party is only
        // ever paid when everything before it already succeeded.
        if seller_net > 0 {
            transfer(&env, &token, &payer, &seller, seller_net)?;
        }
        if royalty_share > 0 {
            transfer(&env, &token, &payer, &royalty.recipient, royalty_share)?;
        }

        // Both transfers succeeded; only now commit settlement state.
        env.storage()
            .instance()
            .set(&DataKey::Summary(collection), &summary);

        Ok(Settlement {
            royalty_share,
            seller_net,
        })
    }

    /// Read the stored royalty configuration for `collection` (read-only view).
    pub fn get_royalty(env: Env, collection: Address) -> Result<Royalty, ForgeError> {
        env.storage()
            .instance()
            .get(&DataKey::Royalty(collection))
            .ok_or(ForgeError::NotFound)
    }

    /// Read the cumulative settlement totals for `collection` (read-only
    /// view). `NotFound` until the collection settles its first sale.
    pub fn get_settlement_summary(
        env: Env,
        collection: Address,
    ) -> Result<SettlementSummary, ForgeError> {
        env.storage()
            .instance()
            .get(&DataKey::Summary(collection))
            .ok_or(ForgeError::NotFound)
    }
}

/// The rate actually applied to sales under `royalty`: zero once the
/// configuration is disabled, so disabled collections settle in full.
fn effective_bps(royalty: &Royalty) -> u32 {
    match royalty.status {
        RoyaltyStatus::Active => royalty.bps,
        RoyaltyStatus::Disabled => 0,
    }
}

/// Split `amount` into `(royalty_share, seller_net)` at `bps` using checked
/// arithmetic. The share floors (`amount * bps / 10_000`) and the remainder
/// stays with the seller, so `royalty_share + seller_net == amount` exactly;
/// `bps <= 10_000` guarantees the share never exceeds the amount. Overflow
/// surfaces as [`ForgeError::ArithmeticOverflow`] before any transfer runs.
fn split(amount: i128, bps: u32) -> Result<(i128, i128), ForgeError> {
    let royalty_share = amount
        .checked_mul(bps as i128)
        .ok_or(ForgeError::ArithmeticOverflow)?
        / 10_000;
    let seller_net = amount
        .checked_sub(royalty_share)
        .ok_or(ForgeError::ArithmeticOverflow)?;
    Ok((royalty_share, seller_net))
}

/// Read the collection's current totals with `amount` / `royalty_share`
/// added checked, returning the record **without** writing it. Runs before
/// any transfer so an overflow aborts the settlement while no funds moved.
fn next_summary(
    env: &Env,
    collection: &Address,
    amount: i128,
    royalty_share: i128,
) -> Result<SettlementSummary, ForgeError> {
    let current: Option<SettlementSummary> = env
        .storage()
        .instance()
        .get(&DataKey::Summary(collection.clone()));
    let summary = match current {
        None => SettlementSummary {
            sales: 1,
            gross_volume: amount,
            royalties_paid: royalty_share,
        },
        Some(totals) => SettlementSummary {
            sales: totals
                .sales
                .checked_add(1)
                .ok_or(ForgeError::ArithmeticOverflow)?,
            gross_volume: totals
                .gross_volume
                .checked_add(amount)
                .ok_or(ForgeError::ArithmeticOverflow)?,
            royalties_paid: totals
                .royalties_paid
                .checked_add(royalty_share)
                .ok_or(ForgeError::ArithmeticOverflow)?,
        },
    };
    Ok(summary)
}

/// Move `amount` of `token` from `from` to `to`.
///
/// Same typed-error bucketing as escrow: a client receiving
/// `Error(Contract, #N)` cannot know whether `N` came from the token or this
/// contract, so every token-side failure collapses into
/// [`ForgeError::TokenTransferFailed`] and the raw discriminant is
/// discarded; the root cause remains visible in the transaction's
/// diagnostic events. The payer's authorization on the calling entrypoint
/// covers the nested token invocation — no allowance is needed for a
/// `transfer` pull when the holder authorizes the call.
fn transfer(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) -> Result<(), ForgeError> {
    match token::TokenClient::new(env, token).try_transfer(from, to, &amount) {
        Ok(Ok(())) => Ok(()),
        // Token returned a typed error (insufficient balance, missing
        // trustline, custom token logic) or the host aborted (most commonly
        // an undeployed token address).
        _ => Err(ForgeError::TokenTransferFailed),
    }
}

#[cfg(test)]
mod authz;
#[cfg(test)]
mod props;

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_forge_test_utils::TestAccounts;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
    use soroban_sdk::Env;

    /// Build a fresh env with mocked auths, a registered contract, a registered
    /// royalty config (500 bps = 5%), and named accounts.
    macro_rules! setup {
        () => {{
            let env = Env::default();
            env.mock_all_auths();
            let contract_id = env.register(MarketplaceRoyalties, ());
            let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);
            client.set_royalty(&accounts.arbiter, &accounts.user2, &500_u32);
            (env, client, accounts)
        }};
    }

    /// Settlement variant of `setup!`: additionally registers a real SEP-41
    /// (Stellar Asset Contract) token and mints 1_000 units to the payer, so
    /// every settlement test asserts actual balance movement. Layout:
    /// `arbiter` is the collection, `user2` the royalty recipient (500 bps),
    /// `user3` the seller, and `user1` the payer.
    macro_rules! setup_settlement {
        () => {{
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let sac = env.register_stellar_asset_contract_v2(admin);
            let token = sac.address();
            let token_admin = StellarAssetClient::new(&env, &token);
            let token_client = TokenClient::new(&env, &token);

            let contract_id = env.register(MarketplaceRoyalties, ());
            let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);
            client.set_royalty(&accounts.arbiter, &accounts.user2, &500_u32);
            token_admin.mint(&accounts.user1, &1_000_i128);

            (env, token, token_client, contract_id, client, accounts)
        }};
    }

    #[test]
    fn set_royalty_stores_config() {
        let (_env, client, accounts) = setup!();
        let royalty = client.get_royalty(&accounts.arbiter);
        assert_eq!(royalty.recipient, accounts.user2);
        assert_eq!(royalty.bps, 500);
        assert_eq!(royalty.status, RoyaltyStatus::Active);
    }

    #[test]
    fn set_royalty_update_in_place() {
        let (_env, client, accounts) = setup!();
        client.set_royalty(&accounts.arbiter, &accounts.user3, &1_000_u32);
        let royalty = client.get_royalty(&accounts.arbiter);
        assert_eq!(royalty.recipient, accounts.user3);
        assert_eq!(royalty.bps, 1_000);
    }

    #[test]
    fn set_royalty_rejects_bps_over_100_percent() {
        let (_env, client, accounts) = setup!();
        let err = client
            .try_set_royalty(&accounts.arbiter, &accounts.user2, &10_001_u32)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn distribute_pays_royalty_and_returns_net() {
        let (_env, client, accounts) = setup!();
        // 1000 units sold with a 5% royalty -> 50 to the recipient, 950 net.
        let net = client.distribute(&accounts.arbiter, &accounts.user1, &1_000_i128);
        assert_eq!(net, 950);
    }

    #[test]
    fn distribute_zero_bps_returns_full_amount() {
        let (_env, client, accounts) = setup!();
        client.set_royalty(&accounts.arbiter, &accounts.user2, &0_u32);
        let net = client.distribute(&accounts.arbiter, &accounts.user1, &1_000_i128);
        assert_eq!(net, 1_000);
    }

    #[test]
    fn distribute_100_percent_returns_zero_net() {
        let (_env, client, accounts) = setup!();
        client.set_royalty(&accounts.arbiter, &accounts.user2, &10_000_u32);
        let net = client.distribute(&accounts.arbiter, &accounts.user1, &1_000_i128);
        assert_eq!(net, 0);
    }

    #[test]
    fn distribute_rejects_non_positive_amount() {
        let (_env, client, accounts) = setup!();
        let err = client
            .try_distribute(&accounts.arbiter, &accounts.user1, &0_i128)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn distribute_missing_config_is_not_found() {
        let (_env, client, accounts) = setup!();
        // An unregistered collection has no config.
        let err = client
            .try_distribute(&accounts.validator, &accounts.user1, &1_000_i128)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_royalty_missing_is_not_found() {
        let (_env, client, accounts) = setup!();
        let err = client
            .try_get_royalty(&accounts.validator)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn distribute_disabled_collection_settles_in_full() {
        let (_env, client, accounts) = setup!();
        // A re-registration with bps 0 keeps the config `Active`; emulate a
        // disabled state by checking that a zero-bps config settles in full.
        client.set_royalty(&accounts.arbiter, &accounts.user2, &0_u32);
        assert_eq!(
            client.get_royalty(&accounts.arbiter).status,
            RoyaltyStatus::Active
        );
        let net = client.distribute(&accounts.arbiter, &accounts.user1, &1_000_i128);
        assert_eq!(net, 1_000);
    }

    // -------------------------------------------------------------------
    // settle_sale — atomic SEP-41 settlement against a real token
    // -------------------------------------------------------------------

    #[test]
    fn settle_sale_pays_seller_and_recipient_and_records_totals() {
        let (_env, token, tc, contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;

        let settled = client.settle_sale(collection, &token, payer, seller, &1_000_i128);

        // 5% of 1000 = 50 for the recipient, 950 for the seller — exact.
        assert_eq!(settled.royalty_share, 50);
        assert_eq!(settled.seller_net, 950);
        assert_eq!(settled.royalty_share + settled.seller_net, 1_000);
        assert_eq!(tc.balance(payer), 0);
        assert_eq!(tc.balance(seller), 950);
        assert_eq!(tc.balance(recipient), 50);
        // The contract settles through and never retains funds.
        assert_eq!(tc.balance(&contract_id), 0);

        let summary = client.get_settlement_summary(collection);
        assert_eq!(summary.sales, 1);
        assert_eq!(summary.gross_volume, 1_000);
        assert_eq!(summary.royalties_paid, 50);
    }

    #[test]
    fn settle_accumulates_totals_across_sales() {
        let (env, token, _tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;

        client.settle_sale(collection, &token, payer, seller, &1_000_i128);
        StellarAssetClient::new(&env, &token).mint(payer, &1_000_i128);
        client.settle_sale(collection, &token, payer, seller, &1_000_i128);

        let summary = client.get_settlement_summary(collection);
        assert_eq!(summary.sales, 2);
        assert_eq!(summary.gross_volume, 2_000);
        assert_eq!(summary.royalties_paid, 100);
    }

    #[test]
    fn settle_zero_bps_settles_in_full_to_seller() {
        let (_env, token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;
        client.set_royalty(collection, recipient, &0_u32);

        let settled = client.settle_sale(collection, &token, payer, seller, &1_000_i128);

        assert_eq!(settled.royalty_share, 0);
        assert_eq!(settled.seller_net, 1_000);
        assert_eq!(tc.balance(seller), 1_000);
        assert_eq!(tc.balance(recipient), 0);
        assert_eq!(client.get_settlement_summary(collection).royalties_paid, 0);
    }

    #[test]
    fn settle_disabled_config_settles_in_full_to_seller() {
        let (env, token, tc, contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;

        // `set_royalty` has no public "disable" switch (it always stores
        // `Active`), so write the `Disabled` record directly — the same gap
        // the compute-only `distribute` suite documents.
        let disabled = Royalty {
            collection: collection.clone(),
            recipient: recipient.clone(),
            bps: 500,
            status: RoyaltyStatus::Disabled,
        };
        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&DataKey::Royalty(collection.clone()), &disabled);
        });

        let settled = client.settle_sale(collection, &token, payer, seller, &1_000_i128);

        assert_eq!(settled.royalty_share, 0);
        assert_eq!(settled.seller_net, 1_000);
        assert_eq!(tc.balance(seller), 1_000);
        assert_eq!(tc.balance(recipient), 0);
    }

    #[test]
    fn settle_100_percent_settles_in_full_to_recipient() {
        let (_env, token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;
        client.set_royalty(collection, recipient, &10_000_u32);

        let settled = client.settle_sale(collection, &token, payer, seller, &1_000_i128);

        assert_eq!(settled.royalty_share, 1_000);
        assert_eq!(settled.seller_net, 0);
        assert_eq!(tc.balance(recipient), 1_000);
        assert_eq!(tc.balance(seller), 0);
    }

    #[test]
    fn settle_rejects_non_positive_amount() {
        let (_env, token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;

        for amount in [0_i128, -100] {
            let err = client
                .try_settle_sale(collection, &token, payer, seller, &amount)
                .unwrap_err()
                .unwrap();
            assert_eq!(err, ForgeError::InvalidInput);
        }
        assert_eq!(tc.balance(payer), 1_000);
    }

    #[test]
    fn settle_missing_config_is_not_found() {
        let (_env, token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let seller = &accounts.user3;
        let unregistered = &accounts.validator; // never configured

        let err = client
            .try_settle_sale(unregistered, &token, payer, seller, &1_000_i128)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::NotFound);
        assert_eq!(tc.balance(payer), 1_000);
        assert_eq!(tc.balance(seller), 0);
    }

    #[test]
    fn settle_overflow_fails_before_any_transfer() {
        let (_env, token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;

        // i128::MAX * 500 bps overflows the checked multiply.
        let err = client
            .try_settle_sale(collection, &token, payer, seller, &i128::MAX)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::ArithmeticOverflow);
        assert_eq!(tc.balance(payer), 1_000);
        assert_eq!(tc.balance(seller), 0);
        assert_eq!(tc.balance(recipient), 0);
        assert_eq!(
            client
                .try_get_settlement_summary(collection)
                .unwrap_err()
                .unwrap(),
            ForgeError::NotFound
        );
    }

    #[test]
    fn settle_with_insufficient_balance_pays_nobody() {
        let (_env, token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;

        // The payer holds 1_000; a 10_000 sale needs 9_500 for the seller
        // first, so the very first transfer already fails at the token.
        let err = client
            .try_settle_sale(collection, &token, payer, seller, &10_000_i128)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::TokenTransferFailed);
        assert_eq!(tc.balance(payer), 1_000);
        assert_eq!(tc.balance(seller), 0);
        assert_eq!(tc.balance(recipient), 0);
        assert_eq!(
            client
                .try_get_settlement_summary(collection)
                .unwrap_err()
                .unwrap(),
            ForgeError::NotFound
        );
    }

    #[test]
    fn settle_with_undeployed_token_fails_and_moves_nothing() {
        let (env, _token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;
        let not_a_token = Address::generate(&env);

        // The host aborts inside `try_transfer`; the failure is bucketed
        // exactly like escrow's undeployed-token path.
        let err = client
            .try_settle_sale(collection, &not_a_token, payer, seller, &1_000_i128)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::TokenTransferFailed);
        // The real token's balances are untouched.
        assert_eq!(tc.balance(payer), 1_000);
        assert_eq!(tc.balance(seller), 0);
        assert_eq!(tc.balance(recipient), 0);
        assert_eq!(
            client
                .try_get_settlement_summary(collection)
                .unwrap_err()
                .unwrap(),
            ForgeError::NotFound
        );
    }

    #[test]
    fn settle_never_leaves_the_recipient_partially_paid() {
        let (_env, token, tc, _contract_id, client, accounts) = setup_settlement!();
        let payer = &accounts.user1;
        let recipient = &accounts.user2;
        let seller = &accounts.user3;
        let collection = &accounts.arbiter;

        // amount 1_030 -> share 51, net 979. The payer's 1_000 covers the
        // seller's transfer but leaves only 21 — not enough for the
        // recipient's 51 — so the SECOND transfer fails after the first
        // succeeded. The recipient must end up with nothing: no partial
        // payment, no settlement state, and the invocation rollback
        // restores the payer's and seller's balances.
        let err = client
            .try_settle_sale(collection, &token, payer, seller, &1_030_i128)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::TokenTransferFailed);
        assert_eq!(
            tc.balance(recipient),
            0,
            "recipient must never be partially paid"
        );
        assert_eq!(tc.balance(payer), 1_000, "rollback restores the payer");
        assert_eq!(tc.balance(seller), 0, "rollback restores the seller");
        assert_eq!(
            client
                .try_get_settlement_summary(collection)
                .unwrap_err()
                .unwrap(),
            ForgeError::NotFound,
            "no settlement state is committed on failure"
        );
    }

    #[test]
    fn get_settlement_summary_missing_is_not_found() {
        let (_env, client, accounts) = setup!();
        let err = client
            .try_get_settlement_summary(&accounts.arbiter)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }
}
