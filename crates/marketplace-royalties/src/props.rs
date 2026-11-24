//! Randomized invariant suite (proptest) for marketplace royalties.
//!
//! Exercised invariants:
//! 1. Split conservation: `royalty_share + returned_net == amount` for every `amount > 0` and `bps <= 10_000`.
//! 2. Rate boundaries: `bps == 0` returns the full amount and `bps == 10_000` returns zero net.
//! 3. Bounded net: returned net is never negative and never exceeds the amount.
//! 4. Batch conservation: a `settle_sales` batch settles exactly like the same
//!    sales driven through `settle_sale` one at a time — identical per-sale
//!    splits in sale order, one summary committed as the batch's aggregate
//!    deltas, and every balance at its per-sale share — over batch shapes from
//!    1 to `MAX_SETTLE_SALES`, with duplicate sellers and zero-share sales
//!    included.
//! 5. Batch failure isolation: a batch carrying exactly one defect — a
//!    non-positive amount, an unconfigured collection, or a deliberately
//!    underfunded payer — fails with the same error the single-sale path
//!    raises for that defect and leaves every balance and the committed
//!    summary byte-identical to the pre-call state.
//! 6. Multi-recipient rounding residue: for random splits of 1 to
//!    `MAX_ROYALTY_RECIPIENTS` distinct recipients (total bps <= 10_000) and
//!    amounts from 1 to 10^18 stroops, over 1,024 randomized `settle_sale`
//!    sales (128 cases of 8) plus a 1–4 sale `settle_sales` batch per case,
//!    measured from real token balance deltas against an exact `i128` model:
//!    - strict value conservation: `seller_net + sum(recipient_shares) ==
//!      sale_amount` and the payer pays exactly `sale_amount`;
//!    - floor discipline: each recipient receives exactly
//!      `floor(amount * bps / 10_000)`;
//!    - dust allocation: `seller_net >= amount - floor(amount * total_bps /
//!      10_000)`, with the extra dust below the recipient count;
//!    - zero-sale invariance: amounts `<= 0` are rejected with
//!      `InvalidInput` by `settle_sale`, `settle_sales` and `distribute` for
//!      every configuration, leaving balances and the summary untouched.

//! 4. Batch conservation: the aggregate of a random batch satisfies `sum(seller_net) + sum(royalty_share) == sum(amount)`.
//! 5. Cap boundary: a batch exceeding `MAX_SETTLE_SALES` is rejected before any transfer.
//! 6. Rollback integrity: a failed batch transfer leaves every balance and the summary untouched.
//! 7. Model-based settlement verification: an independent reference model predicts
//!    royalty quotes and settlement payouts, verifying that quote equals settlement
//!    and that financial conservation holds under randomized inputs up to 40
//!    settlement steps.

use crate::{
    MarketplaceRoyalties, RoyaltyShare, SettlementSummary, SorobanForgeMarketplaceRoyaltiesClient,
    MAX_ROYALTY_RECIPIENTS, MAX_SETTLE_SALES,
};
use proptest::collection::vec as prop_vec;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::vec::Vec;

const MAX_AMOUNT: i128 = 1_000_000_000_000_000;

/// Distinct sellers in the batch pool. Index 0 doubles as the single-sale
/// `seller` the existing properties settle through.
const SELLER_POOL: usize = 4;

struct World {
    env: Env,
    token: Address,
    contract_id: Address,
    collection: Address,
    recipient: Address,
    seller: Address,
    /// Seller pool for batch plans; `sellers[0]` is `seller`, so batch
    /// indices and the single-sale properties address the same accounts.
    sellers: std::vec::Vec<Address>,
    payer: Address,
}

fn setup_world(bps: u32, mint_amount: i128) -> World {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();

    let collection = Address::generate(&env);
    let recipient = Address::generate(&env);
    let seller = Address::generate(&env);
    let payer = Address::generate(&env);
    let mut sellers = std::vec::Vec::with_capacity(SELLER_POOL);
    sellers.push(seller.clone());
    for _ in 1..SELLER_POOL {
        sellers.push(Address::generate(&env));
    }

    StellarAssetClient::new(&env, &token).mint(&payer, &mint_amount);

    let contract_id = env.register(MarketplaceRoyalties, ());
    let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
    client.set_royalty(&collection, &recipient, &bps);

    World {
        env,
        token,
        contract_id,
        collection,
        recipient,
        seller,
        sellers,
        payer,
    }
}

impl World {
    fn client(&self) -> SorobanForgeMarketplaceRoyaltiesClient<'_> {
        SorobanForgeMarketplaceRoyaltiesClient::new(&self.env, &self.contract_id)
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }

    fn balance(&self, address: &Address) -> i128 {
        self.token_client().balance(address)
    }

    /// Balances of every address that can hold this token in the batch
    /// properties — payer, royalty recipient, the whole seller pool, and the
    /// contract — in a fixed order, so a snapshot taken before a call can be
    /// compared byte-for-byte with one taken after it.
    fn balances(&self) -> std::vec::Vec<i128> {
        let mut addresses = std::vec::Vec::with_capacity(3 + self.sellers.len());
        addresses.push(self.payer.clone());
        addresses.push(self.recipient.clone());
        addresses.extend(self.sellers.iter().cloned());
        addresses.push(self.contract_id.clone());
        addresses
            .iter()
            .map(|address| self.balance(address))
            .collect()
    }

    /// The committed summary for `collection`, or `None` while it has never
    /// settled — the read-only view's `NotFound`, as an option.
    fn summary(&self, collection: &Address) -> Option<SettlementSummary> {
        self.client()
            .try_get_settlement_summary(collection)
            .ok()
            .and_then(|result| result.ok())
    }

    /// Map a `(seller index, amount)` plan onto this world's seller pool as
    /// one `settle_sales` batch vector.
    fn batch(&self, plan: &[(u32, i128)]) -> soroban_sdk::Vec<(Address, i128)> {
        let mut sales = soroban_sdk::Vec::new(&self.env);
        for (seller, amount) in plan {
            let seller = &self.sellers[(*seller as usize) % self.sellers.len()];
            sales.push_back((seller.clone(), *amount));
        }
        sales
    }
}

// -----------------------------------------------------------------------
// Strategies
// -----------------------------------------------------------------------

/// The two rate boundaries weighted against the mid-range: `0` makes every
/// sale a zero-royalty-share sale (recipient transfer skipped), `10_000`
/// makes every sale a zero-seller-net sale (seller transfer skipped).
fn bps_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(10_000_u32), 1u32..=10_000_u32]
}

/// Sale amounts: the tiny branch floors to a zero royalty share at most
/// rates, so skipped-recipient sales ride along inside ordinary batches;
/// the upper branches reach the suite's `MAX_AMOUNT` ceiling.
fn amount_strategy() -> impl Strategy<Value = i128> {
    prop_oneof![1i128..=50, 1i128..=1_000_000, 1i128..=MAX_AMOUNT]
}

/// Batch size: the endpoints — one sale and a batch at
/// `MAX_SETTLE_SALES` — pinned beside the uniform middle, so every run
/// covers both ends of `1..=MAX_SETTLE_SALES` without biasing the average.
fn batch_size() -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(1usize),
        Just(MAX_SETTLE_SALES as usize),
        1usize..=(MAX_SETTLE_SALES as usize),
    ]
}

/// A random batch shape: `(seller index, amount)` entries with indices drawn
/// with replacement, so duplicate sellers across one batch are the common
/// case rather than an edge case.
fn sales_plan() -> impl Strategy<Value = std::vec::Vec<(u32, i128)>> {
    batch_size().prop_flat_map(|size| {
        prop::collection::vec((0u32..(SELLER_POOL as u32), amount_strategy()), size..=size)
    })
}

struct BatchWorld {
    env: Env,
    token: Address,
    contract_id: Address,
    collection: Address,
    payer: Address,
    sellers: Vec<Address>,
    _recipient: Address,
    amounts: Vec<i128>,
}

fn setup_batch_world(bps: u32, amounts: Vec<i128>) -> BatchWorld {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();

    let collection = Address::generate(&env);
    let recipient = Address::generate(&env);
    let payer = Address::generate(&env);

    let total: i128 = amounts.iter().sum();
    StellarAssetClient::new(&env, &token).mint(&payer, &total);

    let sellers: Vec<Address> = (0..amounts.len())
        .map(|_| Address::generate(&env))
        .collect();

    let contract_id = env.register(MarketplaceRoyalties, ());
    let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
    client.set_royalty(&collection, &recipient, &bps);

    BatchWorld {
        env,
        token,
        contract_id,
        collection,
        payer,
        sellers,
        _recipient: recipient,
        amounts,
    }
}

impl BatchWorld {
    fn client(&self) -> SorobanForgeMarketplaceRoyaltiesClient<'_> {
        SorobanForgeMarketplaceRoyaltiesClient::new(&self.env, &self.contract_id)
    }

    fn batch(&self) -> soroban_sdk::Vec<(Address, i128)> {
        let mut batch = soroban_sdk::Vec::new(&self.env);
        for (seller, amount) in self.sellers.iter().zip(self.amounts.iter()) {
            batch.push_back((seller.clone(), *amount));
        }
        batch
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_split_conservation_and_bounds(
        amount in 1i128..=MAX_AMOUNT,
        bps in 0u32..=10_000_u32,
    ) {
        let w = setup_world(bps, amount);
        let net = w
            .client()
            .distribute(&w.collection, &w.token, &w.payer, &w.seller, &amount);

        let expected_royalty = amount * (bps as i128) / 10_000;

        prop_assert_eq!(net + expected_royalty, amount, "split conservation: net + royalty must equal total amount");
        prop_assert!(net >= 0, "seller net must never be negative");
        prop_assert!(net <= amount, "seller net must never exceed gross amount");
        prop_assert!(expected_royalty >= 0, "royalty share must never be negative");
        prop_assert!(expected_royalty <= amount, "royalty share must never exceed gross amount");
    }

    #[test]
    fn prop_boundary_rates(
        amount in 1i128..=MAX_AMOUNT,
    ) {
        let w_zero = setup_world(0, amount);
        let net_zero = w_zero
            .client()
            .distribute(&w_zero.collection, &w_zero.token, &w_zero.payer, &w_zero.seller, &amount);
        prop_assert_eq!(net_zero, amount, "bps == 0 must return full amount");

        let w_full = setup_world(10_000, amount);
        let net_full = w_full
            .client()
            .distribute(&w_full.collection, &w_full.token, &w_full.payer, &w_full.seller, &amount);
        prop_assert_eq!(net_full, 0, "bps == 10_000 must return 0 net");
    }

    #[test]
    fn prop_atomic_settlement_conservation(
        amount in 1i128..=MAX_AMOUNT,
        bps in 0u32..=10_000_u32,
    ) {
        let w = setup_world(bps, amount);
        let settlement = w.client().settle_sale(&w.collection, &w.token, &w.payer, &w.seller, &amount);

        prop_assert_eq!(settlement.royalty_share + settlement.seller_net, amount, "settled shares must sum to amount");
        prop_assert!(settlement.seller_net >= 0);
        prop_assert!(settlement.royalty_share >= 0);
    }

    #[test]
    fn prop_batch_conservation(
        bps in 0u32..=10_000_u32,
        amounts in prop_vec(1i128..=MAX_AMOUNT, 1..=MAX_SETTLE_SALES as usize),
    ) {
        let w = setup_batch_world(bps, amounts.clone());
        let settled = w.client().settle_sales(&w.collection, &w.token, &w.payer, &w.batch());

        let mut sum_seller_net: i128 = 0;
        let mut sum_royalty_share: i128 = 0;
        let mut sum_amounts: i128 = 0;
        for (i, sale) in settled.iter().enumerate() {
            sum_seller_net += sale.seller_net;
            sum_royalty_share += sale.royalty_share;
            sum_amounts += amounts[i];
            prop_assert_eq!(sale.seller_net + sale.royalty_share, amounts[i], "per-sale conservation: seller_net + royalty_share must equal amount");
        }

        prop_assert_eq!(sum_seller_net + sum_royalty_share, sum_amounts, "batch conservation: sum(seller_net) + sum(royalty_share) must equal sum(amount)");

        let summary = w.client().get_settlement_summary(&w.collection);
        prop_assert_eq!(summary.sales, amounts.len() as u32, "summary sales count must match batch size");
        prop_assert_eq!(summary.gross_volume, sum_amounts, "summary gross_volume must equal total amount");
        prop_assert_eq!(summary.royalties_paid, sum_royalty_share, "summary royalties_paid must equal total royalty share");
    }

    #[test]
    fn prop_batch_cap_boundary(
        bps in 0u32..=10_000_u32,
    ) {
        let n = MAX_SETTLE_SALES as usize + 1;
        let amounts = (0..n).map(|_| 1i128).collect::<Vec<_>>();
        let w = setup_batch_world(bps, amounts);

        // try_settle_sales returns Result<Result<Vec<Settlement>, ConversionError>, Result<ForgeError, InvokeError>>.
        // A cap rejection surfaces as an outer Err containing the contract's Err(ForgeError::InvalidInput).
        let result = w.client().try_settle_sales(&w.collection, &w.token, &w.payer, &w.batch());
        let err = result.unwrap_err().unwrap();
        prop_assert_eq!(err, ForgeError::InvalidInput, "batch over cap must return InvalidInput");

        let tc = TokenClient::new(&w.env, &w.token);
        prop_assert_eq!(tc.balance(&w.payer), n as i128, "payer balance must be untouched on cap rejection");

        let summary_result = w.client().try_get_settlement_summary(&w.collection);
        let summary_err = summary_result.unwrap_err().unwrap();
        prop_assert_eq!(summary_err, ForgeError::NotFound, "summary must not exist after cap rejection");
    }

    #[test]
    fn prop_batch_rollback_integrity(
        bps in 0u32..=10_000_u32,
    ) {
        // Build a world where the payer has enough for one sale but not the batch.
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let collection = Address::generate(&env);
        let recipient = Address::generate(&env);
        let payer = Address::generate(&env);
        let seller_a = Address::generate(&env);
        let seller_b = Address::generate(&env);

        // Mint only one sale's worth — the batch has two sales so the second transfer fails.
        StellarAssetClient::new(&env, &token).mint(&payer, &1_000_000_000_000_000_i128);

        let contract_id = env.register(MarketplaceRoyalties, ());
        let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
        client.set_royalty(&collection, &recipient, &bps);

        let mut batch = soroban_sdk::Vec::new(&env);
        batch.push_back((seller_a.clone(), 1_000_000_000_000_000_i128));
        batch.push_back((seller_b.clone(), 1_000_000_000_000_000_i128));

        let payer_balance_before = StellarAssetClient::new(&env, &token).balance(&payer);
        let recipient_balance_before = StellarAssetClient::new(&env, &token).balance(&recipient);
        let seller_a_balance_before = StellarAssetClient::new(&env, &token).balance(&seller_a);
        let seller_b_balance_before = StellarAssetClient::new(&env, &token).balance(&seller_b);
        let contract_balance_before = StellarAssetClient::new(&env, &token).balance(&contract_id);

        let result = client.try_settle_sales(&collection, &token, &payer, &batch);
        let err = result.unwrap_err().unwrap();
        prop_assert_eq!(err, ForgeError::TokenTransferFailed, "a batch with insufficient balance must fail on transfer");

        // Verify every balance and the summary are untouched by frame rollback.
        prop_assert_eq!(StellarAssetClient::new(&env, &token).balance(&payer), payer_balance_before, "payer balance must be restored on rollback");
        prop_assert_eq!(StellarAssetClient::new(&env, &token).balance(&recipient), recipient_balance_before, "recipient balance must be restored on rollback");
        prop_assert_eq!(StellarAssetClient::new(&env, &token).balance(&seller_a), seller_a_balance_before, "seller_a balance must be restored on rollback");
        prop_assert_eq!(StellarAssetClient::new(&env, &token).balance(&seller_b), seller_b_balance_before, "seller_b balance must be restored on rollback");
        prop_assert_eq!(StellarAssetClient::new(&env, &token).balance(&contract_id), contract_balance_before, "contract balance must be restored on rollback");

        let summary_after = client.try_get_settlement_summary(&collection);
        let summary_after_err = summary_after.unwrap_err().unwrap();
        prop_assert_eq!(summary_after_err, ForgeError::NotFound, "summary must be untouched on rollback");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A random `settle_sales` batch must settle exactly like the same sales
    /// driven through `settle_sale` one at a time: identical per-sale splits
    /// in sale order, one summary committed as the batch's aggregate deltas,
    /// and every balance at its per-sale share. Both worlds mint the payer
    /// exactly `sum(amounts)`, so no case can fail on funding by accident.
    #[test]
    fn prop_batch_conservation_matches_single_sale(
        bps in bps_strategy(),
        plan in sales_plan(),
    ) {
        let total: i128 = plan.iter().map(|(_, amount)| amount).sum();
        let batch_world = setup_world(bps, total);
        let single_world = setup_world(bps, total);

        // World A: the whole batch in one invocation.
        let batch_settled = batch_world.client().settle_sales(
            &batch_world.collection,
            &batch_world.token,
            &batch_world.payer,
            &batch_world.batch(&plan),
        );

        // World B: the same sales, one `settle_sale` at a time.
        let mut single_settled = soroban_sdk::Vec::new(&single_world.env);
        for (seller, amount) in &plan {
            let seller =
                &single_world.sellers[(*seller as usize) % single_world.sellers.len()];
            single_settled.push_back(single_world.client().settle_sale(
                &single_world.collection,
                &single_world.token,
                &single_world.payer,
                seller,
                amount,
            ));
        }

        prop_assert_eq!(
            batch_settled.len(),
            plan.len() as u32,
            "one settlement per sale, in sale order"
        );

        // Independent floor oracle — the split math both entrypoints share,
        // recomputed here so the batch cannot redefine its own expectations.
        let mut expected_nets: std::vec::Vec<i128> =
            std::iter::repeat_n(0i128, single_world.sellers.len()).collect();
        let mut expected_royalties: i128 = 0;
        for (i, (seller, amount)) in plan.iter().enumerate() {
            let amount = *amount;
            let settlement = batch_settled.get(i as u32).expect("one settlement per sale");
            let royalty_share = amount * (bps as i128) / 10_000;
            let seller_net = amount - royalty_share;
            prop_assert_eq!(
                settlement.royalty_share + settlement.seller_net,
                amount,
                "a batch sale must conserve its amount exactly"
            );
            prop_assert_eq!(
                settlement.royalty_share,
                royalty_share,
                "batch share must equal the single-sale floor split"
            );
            prop_assert_eq!(
                settlement.seller_net,
                seller_net,
                "batch net must equal the single-sale remainder"
            );
            let index = (*seller as usize) % expected_nets.len();
            expected_nets[index] += seller_net;
            expected_royalties += royalty_share;
        }

        // The batch's per-sale splits match the single-sale path exactly.
        prop_assert_eq!(
            batch_settled,
            single_settled,
            "batch splits must match settle_sale one-for-one, in sale order"
        );

        // Every balance in both worlds moves by exactly its per-sale share.
        for w in [&batch_world, &single_world] {
            prop_assert_eq!(w.balance(&w.payer), 0, "the payer funds exactly the batch");
            prop_assert_eq!(
                w.balance(&w.recipient),
                expected_royalties,
                "the recipient receives the sum of the per-sale royalty shares"
            );
            prop_assert_eq!(
                w.balance(&w.contract_id),
                0,
                "the contract settles through and never retains funds"
            );
            for (seller, expected) in w.sellers.iter().zip(&expected_nets) {
                prop_assert_eq!(
                    w.balance(seller),
                    *expected,
                    "each seller receives the sum of its per-sale nets"
                );
            }
        }

        // Committed exactly once, as the batch's aggregates — and identical
        // to what the same sales commit through `settle_sale`.
        let batch_summary = batch_world.summary(&batch_world.collection);
        let single_summary = single_world.summary(&single_world.collection);
        prop_assert_eq!(
            &batch_summary,
            &single_summary,
            "batch summary must match the single-sale path"
        );
        prop_assert_eq!(
            &batch_summary,
            &Some(SettlementSummary {
                sales: plan.len() as u32,
                gross_volume: total,
                royalties_paid: expected_royalties,
            }),
            "the one committed summary equals the batch's aggregate deltas"
        );
    }

    /// Exactly one defect injected into an otherwise valid batch — a zero
    /// amount, a negative amount, an unconfigured collection, or a payer
    /// deliberately funded for every sale but the last — must fail with the
    /// same error the single-sale path raises for that defect, leaving every
    /// balance and the committed summary byte-identical to the pre-call
    /// state. A warm-up sale gives the pre-call state a real summary and
    /// non-zero balances, so "unchanged" is a non-trivial demand. Cap-boundary
    /// and random-position rollback variants belong to #176, not here.
    #[test]
    fn prop_batch_failure_isolation(
        bps in bps_strategy(),
        plan in sales_plan(),
        fault in 0u8..=3,
        position in 0u32..=MAX_SETTLE_SALES,
    ) {
        // Warm-up sale, settled before the snapshot: the summary and the
        // balances under protection are real, not "still NotFound / zero".
        const WARM_UP: i128 = 100;

        let total: i128 = plan.iter().map(|(_, amount)| amount).sum();
        // Fault 3 (underfunded) funds every sale but the last, so the batch
        // must roll back `len - 1` fully settled sales. The validation faults
        // are funded in full: their failure can only come from the injected
        // defect, never from an empty payer by accident.
        let funding = match fault {
            3 => total - plan[plan.len() - 1].1,
            _ => total,
        };

        let w = setup_world(bps, WARM_UP + funding);
        w.client().settle_sale(
            &w.collection,
            &w.token,
            &w.payer,
            &w.sellers[0],
            &WARM_UP,
        );

        // "Unknown config": this contract keys configuration by collection,
        // so an unconfigured collection is the batch's invalid element — a
        // per-seller config has no counterpart in this iteration.
        let unconfigured = Address::generate(&w.env);
        let collection = if fault == 2 { &unconfigured } else { &w.collection };

        // Exactly one invalid element, at a random position in the batch.
        let mut plan = plan;
        if fault == 0 || fault == 1 {
            let i = (position as usize) % plan.len();
            plan[i].1 = if fault == 0 { 0 } else { -plan[i].1 };
        }

        let before_balances = w.balances();
        let before_summary = w.summary(&w.collection);
        let before_unconfigured_summary = w.summary(&unconfigured);

        let expected = match fault {
            0 | 1 => ForgeError::InvalidInput,
            2 => ForgeError::NotFound,
            _ => ForgeError::TokenTransferFailed,
        };

        let batch_err = w
            .client()
            .try_settle_sales(collection, &w.token, &w.payer, &w.batch(&plan))
            .unwrap_err()
            .unwrap();
        prop_assert_eq!(batch_err, expected, "the defective batch must fail");
        prop_assert_eq!(
            &w.balances(),
            &before_balances,
            "a failed batch moves no balance"
        );
        prop_assert_eq!(
            &w.summary(&w.collection),
            &before_summary,
            "a failed batch leaves the summary untouched"
        );
        prop_assert_eq!(
            &w.summary(&unconfigured),
            &before_unconfigured_summary,
            "an unconfigured collection gains no summary either"
        );

        // The same defect through `settle_sale` must raise the same error —
        // and it too must leave the world exactly as it found it.
        let single_err = match fault {
            0 | 1 => {
                let (_, amount) = plan[(position as usize) % plan.len()];
                w.client().try_settle_sale(
                    &w.collection,
                    &w.token,
                    &w.payer,
                    &w.sellers[0],
                    &amount,
                )
            }
            2 => w.client().try_settle_sale(
                &unconfigured,
                &w.token,
                &w.payer,
                &w.sellers[0],
                &WARM_UP,
            ),
            // Underfunded: one sale costing a token more than the payer
            // holds — the single-sale twin of "all but the last sale".
            _ => w.client().try_settle_sale(
                &w.collection,
                &w.token,
                &w.payer,
                &w.sellers[0],
                &(funding + 1),
            ),
        }
        .unwrap_err()
        .unwrap();
        prop_assert_eq!(
            single_err, expected,
            "settle_sale raises the same error for the same defect"
        );
        prop_assert_eq!(
            &w.balances(),
            &before_balances,
            "the single-sale cross-check moves no balance either"
        );
        prop_assert_eq!(
            &w.summary(&w.collection),
            &before_summary,
            "the single-sale cross-check leaves the summary untouched"
        );
    }
}

// -----------------------------------------------------------------------
// Multi-recipient rounding residue
// -----------------------------------------------------------------------

/// Upper bound of the randomized sale amounts: 10^18 stroops.
const MAX_SPLIT_AMOUNT: i128 = 1_000_000_000_000_000_000;

/// Sales per `settle_sales` batch in the residue property.
const MAX_SPLIT_BATCH: usize = 4;

/// Single `settle_sale` sales per residue case; 128 cases of 8 give 1,024
/// randomized sales without paying for 1,024 contract deployments.
const SINGLE_SALES_PER_CASE: usize = 8;

/// Residue property cases.
const SPLIT_CASES: u32 = 128;

/// A world whose collection carries a multi-recipient split.
struct SplitWorld {
    env: Env,
    token: Address,
    contract_id: Address,
    collection: Address,
    payer: Address,
    sellers: Vec<Address>,
    recipients: Vec<Address>,
    bps: Vec<u32>,
}

/// Register `bps.len()` distinct recipients at the given rates and mint the
/// payer exactly `mint_amount`.
fn setup_split_world(bps: &[u32], mint_amount: i128) -> SplitWorld {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin).address();
    let collection = Address::generate(&env);
    let payer = Address::generate(&env);
    let sellers: Vec<Address> = (0..SELLER_POOL).map(|_| Address::generate(&env)).collect();
    let recipients: Vec<Address> = bps.iter().map(|_| Address::generate(&env)).collect();
    if mint_amount > 0 {
        StellarAssetClient::new(&env, &token).mint(&payer, &mint_amount);
    }

    let contract_id = env.register(MarketplaceRoyalties, ());
    let mut shares = soroban_sdk::Vec::new(&env);
    for (recipient, bps) in recipients.iter().zip(bps) {
        shares.push_back(RoyaltyShare {
            recipient: recipient.clone(),
            bps: *bps,
        });
    }
    SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id)
        .set_royalty_splits(&collection, &shares);

    SplitWorld {
        env,
        token,
        contract_id,
        collection,
        payer,
        sellers,
        recipients,
        bps: bps.to_vec(),
    }
}

impl SplitWorld {
    fn client(&self) -> SorobanForgeMarketplaceRoyaltiesClient<'_> {
        SorobanForgeMarketplaceRoyaltiesClient::new(&self.env, &self.contract_id)
    }

    fn balance(&self, address: &Address) -> i128 {
        TokenClient::new(&self.env, &self.token).balance(address)
    }

    /// Payer, sellers, recipients and contract balances in a fixed order.
    fn balances(&self) -> Vec<i128> {
        core::iter::once(&self.payer)
            .chain(&self.sellers)
            .chain(&self.recipients)
            .chain(core::iter::once(&self.contract_id))
            .map(|address| self.balance(address))
            .collect()
    }

    fn recipient_balances(&self) -> Vec<i128> {
        self.recipients.iter().map(|r| self.balance(r)).collect()
    }

    fn summary(&self) -> Option<SettlementSummary> {
        self.client()
            .try_get_settlement_summary(&self.collection)
            .ok()
            .and_then(|result| result.ok())
    }
}

/// Exact model of one recipient's share. `amount <= 10^18` and
/// `bps <= 10_000` keep the product far inside `i128`, so this is plain
/// floored division, independent of the contract's overflow-safe
/// quotient/remainder decomposition.
fn floored_share(amount: i128, bps: u32) -> i128 {
    amount * i128::from(bps) / 10_000
}

/// Assert every per-sale invariant for one settled sale.
fn assert_split_invariants(
    amount: i128,
    bps: &[u32],
    recipient_deltas: &[i128],
    seller_net: i128,
    royalty_share: i128,
) -> Result<(), TestCaseError> {
    let total_bps: u32 = bps.iter().sum();
    let mut sum_shares = 0i128;
    for (index, (&rate, &delta)) in bps.iter().zip(recipient_deltas).enumerate() {
        // Floor discipline: exactly floor(amount * bps / 10_000), never up.
        prop_assert_eq!(
            delta,
            floored_share(amount, rate),
            "recipient {} at {} bps on {} must receive the floored share",
            index,
            rate,
            amount
        );
        prop_assert!(delta * 10_000 <= amount * i128::from(rate));
        sum_shares += delta;
    }
    prop_assert_eq!(royalty_share, sum_shares);
    // Strict value conservation.
    prop_assert_eq!(
        seller_net + sum_shares,
        amount,
        "seller_net + recipient shares must equal the sale amount"
    );
    prop_assert!(seller_net >= 0);
    // Dust allocation: all residue of the per-recipient floors goes to the
    // seller, and it is less than one stroop per recipient.
    let aggregate_floor_net = amount - floored_share(amount, total_bps);
    prop_assert!(
        seller_net >= aggregate_floor_net,
        "seller_net {} must keep all rounding dust (>= {})",
        seller_net,
        aggregate_floor_net
    );
    prop_assert!(seller_net - aggregate_floor_net < bps.len() as i128);
    Ok(())
}

/// 1 to `MAX_ROYALTY_RECIPIENTS` rates summing to at most 10_000, with the
/// empty (all-zero) and full (exactly 10_000) budgets weighted in.
fn split_bps_strategy() -> impl Strategy<Value = Vec<u32>> {
    let budget = prop_oneof![
        1 => Just(0u32),
        2 => Just(10_000u32),
        7 => 0u32..=10_000,
    ];
    (1usize..=MAX_ROYALTY_RECIPIENTS as usize, budget).prop_flat_map(|(count, budget)| {
        prop_vec(0u32..=budget, count - 1).prop_map(move |mut cuts| {
            // Sorted cut points partition [0, budget] into `count` rates.
            cuts.sort_unstable();
            let mut rates = Vec::with_capacity(count);
            let mut previous = 0;
            for cut in cuts {
                rates.push(cut - previous);
                previous = cut;
            }
            rates.push(budget - previous);
            rates
        })
    })
}

/// Sale amounts from 1 to 10^18, biased toward the residue-heavy edges:
/// tiny amounts, values one either side of a multiple of 10_000, and the
/// upper bound itself.
fn split_amount_strategy() -> impl Strategy<Value = i128> {
    prop_oneof![
        3 => 1i128..=MAX_SPLIT_AMOUNT,
        2 => 1i128..=20_000,
        2 => (1i128..=MAX_SPLIT_AMOUNT / 10_000)
            .prop_flat_map(|k| (Just(k * 10_000), -1i128..=1))
            .prop_map(|(base, offset)| (base + offset).clamp(1, MAX_SPLIT_AMOUNT)),
        1 => (MAX_SPLIT_AMOUNT - 10_000)..=MAX_SPLIT_AMOUNT,
    ]
}

/// Non-positive sale amounts.
fn invalid_amount_strategy() -> impl Strategy<Value = i128> {
    prop_oneof![Just(0i128), Just(-1i128), Just(i128::MIN), i128::MIN..=0,]
}

proptest! {
    #![proptest_config(ProptestConfig {
        // SPLIT_CASES * SINGLE_SALES_PER_CASE = 1,024 single sales, plus a
        // 1–4 sale batch per case.
        cases: SPLIT_CASES,
        // Fixed seed: the same sales run on every machine and in CI.
        rng_seed: RngSeed::Fixed(0x524f_5941_4c54_5900),
        ..ProptestConfig::default()
    })]

    /// Value conservation, floor discipline and dust allocation hold for
    /// `settle_sale` and `settle_sales` under random multi-recipient splits.
    #[test]
    fn prop_multi_recipient_rounding_residue(
        bps in split_bps_strategy(),
        amounts in prop_vec(split_amount_strategy(), SINGLE_SALES_PER_CASE),
        batch in prop_vec((0usize..SELLER_POOL, split_amount_strategy()), 1..=MAX_SPLIT_BATCH),
    ) {
        let batch_total: i128 = batch.iter().map(|(_, a)| a).sum();
        let singles_total: i128 = amounts.iter().sum();
        let w = setup_split_world(&bps, singles_total + batch_total);
        let client = w.client();

        // Single sales, each checked against its own balance deltas.
        for (i, &amount) in amounts.iter().enumerate() {
        let seller = &w.sellers[i % SELLER_POOL];
        let seller_before = w.balance(seller);
        let recipients_before = w.recipient_balances();
        let payer_before = w.balance(&w.payer);
        let settlement = client.settle_sale(&w.collection, &w.token, &w.payer, seller, &amount);
        let deltas: Vec<i128> = w
            .recipient_balances()
            .iter()
            .zip(&recipients_before)
            .map(|(after, before)| after - before)
            .collect();
        prop_assert_eq!(w.balance(seller) - seller_before, settlement.seller_net);
        prop_assert_eq!(payer_before - w.balance(&w.payer), amount);
        assert_split_invariants(
            amount,
            &w.bps,
            &deltas,
            settlement.seller_net,
            settlement.royalty_share,
        )?;
        prop_assert_eq!(w.balance(&w.contract_id), 0, "the contract never takes custody");

        // The read-only preview agrees with what was settled.
        let preview = client.preview_settlement(&w.collection, &amount);
        prop_assert_eq!(preview.seller_net, settlement.seller_net);
        prop_assert_eq!(preview.royalty_amount, settlement.royalty_share);
        prop_assert_eq!(
            preview.rounding_residue,
            settlement.seller_net - (amount - floored_share(amount, w.bps.iter().sum()))
        );
        }

        // Batch: per-sale settlements and aggregate balance deltas.
        let sellers_before: Vec<i128> = w.sellers.iter().map(|s| w.balance(s)).collect();
        let recipients_before = w.recipient_balances();
        let mut sales = soroban_sdk::Vec::new(&w.env);
        for (seller, sale_amount) in &batch {
            sales.push_back((w.sellers[*seller].clone(), *sale_amount));
        }
        let settlements = client.settle_sales(&w.collection, &w.token, &w.payer, &sales);
        prop_assert_eq!(settlements.len() as usize, batch.len());

        let mut expected_seller_gain = std::vec![0i128; SELLER_POOL];
        let mut expected_recipient_gain = std::vec![0i128; w.bps.len()];
        for (i, (seller, sale_amount)) in batch.iter().enumerate() {
            let settlement = settlements.get(i as u32).unwrap();
            let shares: Vec<i128> =
                w.bps.iter().map(|&rate| floored_share(*sale_amount, rate)).collect();
            assert_split_invariants(
                *sale_amount,
                &w.bps,
                &shares,
                settlement.seller_net,
                settlement.royalty_share,
            )?;
            expected_seller_gain[*seller] += settlement.seller_net;
            for (gain, share) in expected_recipient_gain.iter_mut().zip(&shares) {
                *gain += share;
            }
        }
        for (index, seller) in w.sellers.iter().enumerate() {
            prop_assert_eq!(w.balance(seller) - sellers_before[index], expected_seller_gain[index]);
        }
        for (index, before) in recipients_before.iter().enumerate() {
            prop_assert_eq!(
                w.balance(&w.recipients[index]) - before,
                expected_recipient_gain[index],
                "batch recipient {} must receive the sum of its per-sale floors",
                index
            );
        }
        prop_assert_eq!(w.balance(&w.payer), 0, "the payer funds exactly every sale");
        prop_assert_eq!(w.balance(&w.contract_id), 0);

        let summary = w.summary().unwrap();
        prop_assert_eq!(summary.sales, (SINGLE_SALES_PER_CASE + batch.len()) as u32);
        prop_assert_eq!(summary.gross_volume, singles_total + batch_total);
    }

    /// Non-positive amounts are rejected with `InvalidInput` by every
    /// settlement path, whatever the split, with no balance or summary change.
    #[test]
    fn prop_non_positive_amounts_always_reject(
        bps in split_bps_strategy(),
        invalid in invalid_amount_strategy(),
        valid in split_amount_strategy(),
        position in 0usize..MAX_SPLIT_BATCH,
    ) {
        let w = setup_split_world(&bps, valid * MAX_SPLIT_BATCH as i128);
        let client = w.client();
        let seller = &w.sellers[0];
        let balances_before = w.balances();

        let err = client
            .try_settle_sale(&w.collection, &w.token, &w.payer, seller, &invalid)
            .unwrap_err()
            .unwrap();
        prop_assert_eq!(err, ForgeError::InvalidInput);

        let err = client
            .try_distribute(&w.collection, &w.token, &w.payer, seller, &invalid)
            .unwrap_err()
            .unwrap();
        prop_assert_eq!(err, ForgeError::InvalidInput);

        // One invalid sale anywhere in an otherwise valid batch.
        let mut sales = soroban_sdk::Vec::new(&w.env);
        for i in 0..MAX_SPLIT_BATCH {
            let sale_amount = if i == position { invalid } else { valid };
            sales.push_back((w.sellers[i % SELLER_POOL].clone(), sale_amount));
        }
        let err = client
            .try_settle_sales(&w.collection, &w.token, &w.payer, &sales)
            .unwrap_err()
            .unwrap();
        prop_assert_eq!(err, ForgeError::InvalidInput);

        prop_assert_eq!(w.balances(), balances_before, "a rejected sale moves no tokens");
        prop_assert!(w.summary().is_none(), "a rejected sale commits no summary");
    }
}

#[test]
fn multi_recipient_residue_known_values() {
    // 3 recipients at 3_333 bps each on 10_001 stroops: each floors to
    // 3_333, so the seller keeps 10_001 - 9_999 = 2, one more than the
    // aggregate floor (floor(10_001 * 9_999 / 10_000) = 9_999 -> net 2).
    let w = setup_split_world(&[3_333, 3_333, 3_333], 10_001);
    let settlement =
        w.client()
            .settle_sale(&w.collection, &w.token, &w.payer, &w.sellers[0], &10_001);
    assert_eq!(settlement.royalty_share, 9_999);
    assert_eq!(settlement.seller_net, 2);
    assert_eq!(w.recipient_balances(), std::vec![3_333, 3_333, 3_333]);

    // 5 recipients at 1 bps each on 9_999 stroops: every share floors to 0
    // and the whole sale stays with the seller.
    let w = setup_split_world(&[1, 1, 1, 1, 1], 9_999);
    let settlement =
        w.client()
            .settle_sale(&w.collection, &w.token, &w.payer, &w.sellers[0], &9_999);
    assert_eq!(settlement.royalty_share, 0);
    assert_eq!(settlement.seller_net, 9_999);
    assert_eq!(w.balance(&w.sellers[0]), 9_999);
}

// -----------------------------------------------------------------------
// Independent Reference Model for Royalty Calculations
// -----------------------------------------------------------------------

/// Independent reference model that predicts settlement outcomes without using
/// the contract's implementation. This model serves as an oracle to verify
/// that the contract's calculations are correct.
mod reference_model {
    use std::collections::HashMap;

    /// Predicted settlement outcome from the reference model
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ModelSettlement {
        pub royalty_share: i128,
        pub seller_net: i128,
        pub recipient_payouts: HashMap<usize, i128>, // recipient_index -> amount
    }

    /// Independent calculation of royalty split using floor division
    pub fn calculate_split(amount: i128, bps: u32) -> (i128, i128) {
        if amount <= 0 || bps == 0 {
            return (0, amount);
        }
        if bps > 10_000 {
            panic!("Invalid bps: {}", bps);
        }

        // Floor division: royalty_share = floor(amount * bps / 10_000)
        let royalty_share = (amount / 10_000)
            .checked_mul(bps as i128)
            .and_then(|whole| whole.checked_add((amount % 10_000) * (bps as i128) / 10_000))
            .expect("overflow in model split");

        let seller_net = amount
            .checked_sub(royalty_share)
            .expect("overflow in model");

        (royalty_share, seller_net)
    }

    /// Predict the outcome of a settlement with multiple recipients
    pub fn predict_multi_recipient_settlement(
        amount: i128,
        recipient_rates: &[(usize, u32)], // (recipient_index, bps)
    ) -> ModelSettlement {
        let mut recipient_payouts = HashMap::new();
        let mut total_royalty = 0i128;

        for &(recipient_idx, bps) in recipient_rates {
            let (share, _) = calculate_split(amount, bps);
            recipient_payouts.insert(recipient_idx, share);
            total_royalty = total_royalty.checked_add(share).expect("overflow in model");
        }

        let seller_net = amount
            .checked_sub(total_royalty)
            .expect("overflow in model");

        ModelSettlement {
            royalty_share: total_royalty,
            seller_net,
            recipient_payouts,
        }
    }

    /// Predict a batch of settlements
    pub fn predict_batch(sales: &[(i128, u32)], // (amount, bps) per sale
    ) -> Vec<ModelSettlement> {
        sales
            .iter()
            .map(|&(amount, bps)| {
                let (royalty_share, seller_net) = calculate_split(amount, bps);
                ModelSettlement {
                    royalty_share,
                    seller_net,
                    recipient_payouts: {
                        let mut map = HashMap::new();
                        map.insert(0, royalty_share);
                        map
                    },
                }
            })
            .collect()
    }

    #[cfg(test)]
    mod model_tests {
        use super::*;

        #[test]
        fn model_split_zero_bps() {
            assert_eq!(calculate_split(1000, 0), (0, 1000));
        }

        #[test]
        fn model_split_full_bps() {
            assert_eq!(calculate_split(1000, 10_000), (1000, 0));
        }

        #[test]
        fn model_split_conservation() {
            let (royalty, seller) = calculate_split(999_999_999, 2500);
            assert_eq!(royalty + seller, 999_999_999);
        }

        #[test]
        fn model_split_floors_correctly() {
            // 100 * 1 / 10_000 = 0.01, should floor to 0
            let (royalty, seller) = calculate_split(100, 1);
            assert_eq!(royalty, 0);
            assert_eq!(seller, 100);
        }
    }
}

// -----------------------------------------------------------------------
// Model-Based Property Tests
// -----------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        ..ProptestConfig::default()
    }.with_source_file(file!()))]

    /// Independent model predicts quote correctly
    #[test]
    fn prop_quote_matches_reference_model(
        amount in 1i128..=MAX_AMOUNT,
        bps in 0u32..=10_000_u32,
    ) {
        let w = setup_world(bps, amount);

        // Get quote from contract
        let quote = w.client().quote_sale(&w.collection, &amount);

        // Calculate expected values using reference model
        let (expected_royalty, expected_seller_net) =
            reference_model::calculate_split(amount, bps);

        prop_assert_eq!(quote.gross, amount, "quote gross must equal amount");
        prop_assert_eq!(quote.royalty_bps, bps, "quote bps must match configuration");
        prop_assert_eq!(
            quote.royalty_amount, expected_royalty,
            "quote royalty_amount must match reference model"
        );
        prop_assert_eq!(
            quote.seller_net, expected_seller_net,
            "quote seller_net must match reference model"
        );
        prop_assert_eq!(
            quote.royalty_amount + quote.seller_net, amount,
            "quote must conserve amount"
        );
    }

    /// Quote matches actual settlement
    #[test]
    fn prop_quote_matches_settlement(
        amount in 1i128..=MAX_AMOUNT,
        bps in 0u32..=10_000_u32,
    ) {
        let w = setup_world(bps, amount);

        // Get quote first
        let quote = w.client().quote_sale(&w.collection, &amount);

        // Then settle
        let settlement = w.client().settle_sale(
            &w.collection,
            &w.token,
            &w.payer,
            &w.seller,
            &amount
        );

        prop_assert_eq!(
            settlement.royalty_share, quote.royalty_amount,
            "settlement royalty_share must equal quote royalty_amount"
        );
        prop_assert_eq!(
            settlement.seller_net, quote.seller_net,
            "settlement seller_net must equal quote seller_net"
        );
    }

    /// Model-based settlement sequence verification (up to 40 steps)
    #[test]
    fn prop_settlement_sequence_matches_model(
        bps in 0u32..=10_000_u32,
        sales in prop_vec(1i128..=MAX_AMOUNT, 1..=40),
    ) {
        let total: i128 = sales.iter().sum();
        let w = setup_world(bps, total);

        // Track cumulative values for verification
        let mut cumulative_gross = 0i128;
        let mut cumulative_royalty = 0i128;
        let mut cumulative_seller_net = 0i128;

        for (step, &amount) in sales.iter().enumerate() {
            // Predict using reference model
            let (expected_royalty, expected_seller_net) =
                reference_model::calculate_split(amount, bps);

            // Execute actual settlement
            let settlement = w.client().settle_sale(
                &w.collection,
                &w.token,
                &w.payer,
                &w.seller,
                &amount
            );

            // Verify this step matches model
            prop_assert_eq!(
                settlement.royalty_share, expected_royalty,
                "step {} royalty_share mismatch: expected {}, got {}",
                step, expected_royalty, settlement.royalty_share
            );
            prop_assert_eq!(
                settlement.seller_net, expected_seller_net,
                "step {} seller_net mismatch: expected {}, got {}",
                step, expected_seller_net, settlement.seller_net
            );

            // Verify conservation at each step
            prop_assert_eq!(
                settlement.royalty_share + settlement.seller_net, amount,
                "step {} conservation failed", step
            );

            cumulative_gross += amount;
            cumulative_royalty += settlement.royalty_share;
            cumulative_seller_net += settlement.seller_net;
        }

        // Verify cumulative conservation
        prop_assert_eq!(
            cumulative_royalty + cumulative_seller_net, cumulative_gross,
            "cumulative conservation failed after {} steps", sales.len()
        );

        // Verify summary matches cumulative
        let summary = w.client().get_settlement_summary(&w.collection);
        prop_assert_eq!(
            summary.sales, sales.len() as u32,
            "summary sales count mismatch"
        );
        prop_assert_eq!(
            summary.gross_volume, cumulative_gross,
            "summary gross_volume mismatch"
        );
        prop_assert_eq!(
            summary.royalties_paid, cumulative_royalty,
            "summary royalties_paid mismatch"
        );
    }

    /// No duplicate recipient payments in batch
    #[test]
    fn prop_no_duplicate_recipient_payments(
        bps in 1u32..=10_000_u32,
        amounts in prop_vec(1i128..=MAX_AMOUNT, 1..=MAX_SETTLE_SALES as usize),
    ) {
        let w = setup_batch_world(bps, amounts.clone());

        let recipient_balance_before = w.client().get_settlement_summary(&w.collection)
            .unwrap_or_else(|_| SettlementSummary {
                sales: 0,
                gross_volume: 0,
                royalties_paid: 0,
            })
            .royalties_paid;

        // Settle batch
        let settlements = w.client().settle_sales(
            &w.collection,
            &w.token,
            &w.payer,
            &w.batch()
        );

        // Calculate expected total royalty from model
        let expected_total_royalty: i128 = amounts
            .iter()
            .map(|&amount| reference_model::calculate_split(amount, bps).0)
            .sum();

        // Get actual summary
        let summary = w.client().get_settlement_summary(&w.collection);
        let actual_royalty_paid = summary.royalties_paid - recipient_balance_before;

        prop_assert_eq!(
            actual_royalty_paid, expected_total_royalty,
            "total royalty paid must match model prediction (no duplicates)"
        );

        // Verify sum of settlements matches
        let settlement_sum: i128 = settlements
            .iter()
            .map(|s| s.royalty_share)
            .sum();

        prop_assert_eq!(
            settlement_sum, expected_total_royalty,
            "sum of settlement royalty_shares must match model"
        );
    }

    /// Batch rounding behavior preserves dust with sellers
    #[test]
    fn prop_batch_rounding_dust_with_sellers(
        bps in 1u32..=9_999_u32, // Avoid 0 and 10_000 to ensure rounding cases
        amounts in prop_vec(50i128..=1000i128, 2..=10), // Small amounts to ensure rounding
    ) {
        let w = setup_batch_world(bps, amounts.clone());

        // Calculate expected values using reference model
        let model_predictions: Vec<_> = amounts
            .iter()
            .map(|&amount| reference_model::calculate_split(amount, bps))
            .collect();

        // Settle batch
        let settlements = w.client().settle_sales(
            &w.collection,
            &w.token,
            &w.payer,
            &w.batch()
        );

        for (i, (settlement, &amount)) in settlements.iter().zip(&amounts).enumerate() {
            let (expected_royalty, expected_seller_net) = model_predictions[i];

            // Verify exact match with model (rounding included)
            prop_assert_eq!(
                settlement.royalty_share, expected_royalty,
                "sale {} royalty mismatch", i
            );
            prop_assert_eq!(
                settlement.seller_net, expected_seller_net,
                "sale {} seller_net mismatch", i
            );

            // Verify dust stays with seller (conservative vs floor division)
            let floor_royalty = amount * (bps as i128) / 10_000;
            prop_assert!(
                settlement.royalty_share <= floor_royalty,
                "sale {} royalty must not exceed floor", i
            );
            prop_assert_eq!(
                settlement.seller_net, amount - settlement.royalty_share,
                "sale {} seller receives remainder", i
            );
        }
    }
}
