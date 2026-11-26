//! Randomized property-test suite for price-bracket royalty tiers.
//!
//! Uses a fixed [`RngSeed`] so CI and local runs reproduce identical tier
//! tables and bracket-edge sale amounts (`min - 1`, `min`, `min + 1`).
//!
//! Verified invariants:
//! 1. Chosen tier correctness: `quote_sale` reports the exact `bps` of the
//!    highest tier whose minimum is `<= amount`.
//! 2. Quote-to-settlement parity: `quote_sale`, `preview_settlement`,
//!    `settle_sale`, `settle_sales`, and `distribute` agree exactly at every
//!    bracket edge and across multi-recipient splits.
//! 3. Exact value conservation: seller net plus all recipient token balance
//!    deltas equal the gross sale amount, and the contract never retains funds.

use crate::{
    MarketplaceRoyalties, RoyaltyShare, SettlementSummary, SorobanForgeMarketplaceRoyaltiesClient,
    MAX_ROYALTY_RECIPIENTS, MAX_ROYALTY_TIERS, MAX_SETTLE_SALES,
};
use proptest::collection::vec as prop_vec;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::vec::Vec;

const SELLER_POOL: usize = 4;
const TIER_CASES: u32 = 128;
const MAX_TIER_GAP: i128 = 1_000_000_000;
/// Bounded to 4 sales per multi-recipient batch (matching `props.rs`'s
/// `MAX_SPLIT_BATCH`) so up to 5 recipients per sale stay within Soroban's
/// default per-invocation host auth/instruction budget.
const MAX_BATCH_SALES: usize = 4;

/// Independent oracle for the highest tier whose minimum sale amount is `<= amount`.
fn oracle_tier_bps(tiers: &[(i128, u32)], amount: i128) -> u32 {
    tiers
        .iter()
        .rev()
        .find(|(min_amount, _)| *min_amount <= amount)
        .map_or(0, |(_, bps)| *bps)
}

/// Exact integer model for one recipient's share under a tiered rate.
fn oracle_recipient_share(amount: i128, tier_bps: u32, weight: u32, total_weight: u32) -> i128 {
    let numerator = i128::from(tier_bps) * i128::from(weight);
    let denominator = 10_000_i128 * i128::from(total_weight);
    (amount / denominator) * numerator + ((amount % denominator) * numerator) / denominator
}

/// Generate a valid strictly-increasing tier table starting at `min = 1` with
/// 1 to `MAX_ROYALTY_TIERS` brackets and rates in `0..=10_000`.
fn tier_table_strategy() -> impl Strategy<Value = Vec<(i128, u32)>> {
    let tier_bps = prop_oneof![
        1 => Just(0_u32),
        1 => Just(10_000_u32),
        8 => 0_u32..=10_000_u32,
    ];
    let gap = prop_oneof![
        3 => 1_i128..=5_i128,
        3 => 1_i128..=1_000_i128,
        2 => 1_i128..=1_000_000_i128,
        2 => 1_i128..=MAX_TIER_GAP,
    ];
    (1usize..=MAX_ROYALTY_TIERS as usize).prop_flat_map(move |count| {
        (
            prop_vec(tier_bps.clone(), count..=count),
            prop_vec(gap.clone(), (count - 1)..=(count - 1)),
        )
            .prop_map(|(rates, gaps)| {
                let mut table = Vec::with_capacity(rates.len());
                let mut min_amount = 1_i128;
                table.push((min_amount, rates[0]));
                for (rate, step) in rates.into_iter().skip(1).zip(gaps) {
                    min_amount += step;
                    table.push((min_amount, rate));
                }
                table
            })
    })
}

/// 1 to `MAX_ROYALTY_RECIPIENTS` recipient weights summing to at most 10,000.
fn split_weights_strategy() -> impl Strategy<Value = Vec<u32>> {
    let budget = prop_oneof![
        1 => Just(0_u32),
        2 => Just(10_000_u32),
        7 => 1_u32..=10_000_u32,
    ];
    (1usize..=MAX_ROYALTY_RECIPIENTS as usize, budget).prop_flat_map(|(count, budget)| {
        prop_vec(0_u32..=budget, count - 1).prop_map(move |mut cuts| {
            cuts.sort_unstable();
            let mut weights = Vec::with_capacity(count);
            let mut prev = 0_u32;
            for cut in cuts {
                weights.push(cut - prev);
                prev = cut;
            }
            weights.push(budget - prev);
            weights
        })
    })
}

/// Build sale amounts concentrated around every bracket edge (`min - 1`, `min`,
/// `min + 1`), plus a random offset above the highest bracket.
fn bracket_edge_amounts(tiers: &[(i128, u32)], tail_offset: i128) -> Vec<i128> {
    let mut amounts = Vec::with_capacity(tiers.len() * 3 + 2);
    for &(min_amount, _) in tiers {
        if min_amount > 1 {
            amounts.push(min_amount - 1);
        }
        amounts.push(min_amount);
        amounts.push(min_amount + 1);
    }
    let last_min = tiers.last().map_or(1, |(min_amount, _)| *min_amount);
    amounts.push(last_min + tail_offset);
    amounts.sort_unstable();
    amounts.dedup();
    amounts
}

struct TieredWorld {
    env: Env,
    token: Address,
    contract_id: Address,
    collection: Address,
    payer: Address,
    sellers: Vec<Address>,
    recipients: Vec<Address>,
    weights: Vec<u32>,
}

fn setup_tiered_world(weights: &[u32], tiers: &[(i128, u32)], mint_amount: i128) -> TieredWorld {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin).address();
    let collection = Address::generate(&env);
    let payer = Address::generate(&env);
    let sellers: Vec<Address> = (0..SELLER_POOL).map(|_| Address::generate(&env)).collect();
    let recipients: Vec<Address> = weights.iter().map(|_| Address::generate(&env)).collect();
    if mint_amount > 0 {
        StellarAssetClient::new(&env, &token).mint(&payer, &mint_amount);
    }

    let contract_id = env.register(MarketplaceRoyalties, ());
    let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);

    let mut shares = soroban_sdk::Vec::new(&env);
    for (recipient, &bps) in recipients.iter().zip(weights) {
        shares.push_back(RoyaltyShare {
            recipient: recipient.clone(),
            bps,
        });
    }
    client.set_royalty_splits(&collection, &shares);

    let mut sdk_tiers = soroban_sdk::Vec::new(&env);
    for &(min_amount, bps) in tiers {
        sdk_tiers.push_back((min_amount, bps));
    }
    client.set_royalty_tiers(&collection, &sdk_tiers);

    TieredWorld {
        env,
        token,
        contract_id,
        collection,
        payer,
        sellers,
        recipients,
        weights: weights.to_vec(),
    }
}

impl TieredWorld {
    fn client(&self) -> SorobanForgeMarketplaceRoyaltiesClient<'_> {
        SorobanForgeMarketplaceRoyaltiesClient::new(&self.env, &self.contract_id)
    }

    fn balance(&self, address: &Address) -> i128 {
        TokenClient::new(&self.env, &self.token).balance(address)
    }

    fn recipient_balances(&self) -> Vec<i128> {
        self.recipients.iter().map(|r| self.balance(r)).collect()
    }

    fn expected_shares(&self, amount: i128, tier_bps: u32) -> Vec<i128> {
        let total_weight: u32 = self.weights.iter().sum();
        if total_weight > 0 {
            self.weights
                .iter()
                .map(|&w| oracle_recipient_share(amount, tier_bps, w, total_weight))
                .collect()
        } else {
            let count = self.weights.len() as u32;
            (0..count)
                .map(|_| oracle_recipient_share(amount, tier_bps, 1, count))
                .collect()
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: TIER_CASES,
        // Fixed seed for deterministic bracket-edge coverage locally and in CI.
        rng_seed: RngSeed::Fixed(0x5449_4552_5052_4f50),
        ..ProptestConfig::default()
    })]

    /// At every bracket edge (`min - 1`, `min`, `min + 1`) and above the top
    /// tier, `quote_sale`, `preview_settlement`, `settle_sale`, and
    /// `distribute` select the exact expected tier, agree on every payout, and
    /// conserve gross sale value exactly.
    #[test]
    fn prop_tiered_bracket_edges_quote_equals_settlement(
        tiers in tier_table_strategy(),
        weights in split_weights_strategy(),
        tail_offset in 2_i128..=1_000_000_i128,
    ) {
        let amounts = bracket_edge_amounts(&tiers, tail_offset);
        // Fund both a `settle_sale` pass and a `distribute` pass for every edge.
        let total_per_pass: i128 = amounts.iter().sum();
        let w = setup_tiered_world(&weights, &tiers, total_per_pass * 2);
        let client = w.client();

        let stored = client.get_royalty_tiers(&w.collection);
        prop_assert_eq!(stored.len() as usize, tiers.len());

        let mut expected_gross = 0_i128;
        let mut expected_royalties = 0_i128;
        let mut expected_sales = 0_u32;

        for (idx, &amount) in amounts.iter().enumerate() {
            let expected_bps = oracle_tier_bps(&tiers, amount);
            let expected_shares = w.expected_shares(amount, expected_bps);
            let expected_royalty: i128 = expected_shares.iter().sum();
            let expected_seller_net = amount - expected_royalty;

            // 1. Quote check
            let quote = client.quote_sale(&w.collection, &amount);
            prop_assert_eq!(quote.gross, amount);
            prop_assert_eq!(
                quote.royalty_bps,
                expected_bps,
                "chosen tier bps mismatch for amount={} in tiers={:?}",
                amount,
                tiers
            );
            prop_assert_eq!(quote.royalty_amount, expected_royalty);
            prop_assert_eq!(quote.seller_net, expected_seller_net);
            prop_assert_eq!(quote.royalty_amount + quote.seller_net, amount);

            // 2. Preview check
            let preview = client.preview_settlement(&w.collection, &amount);
            prop_assert_eq!(preview.gross, amount);
            prop_assert_eq!(preview.royalty_amount, quote.royalty_amount);
            prop_assert_eq!(preview.seller_net, quote.seller_net);
            prop_assert_eq!(preview.royalty_amount + preview.seller_net, amount);
            prop_assert!(preview.rounding_residue >= 0);
            prop_assert!(preview.rounding_residue < weights.len() as i128);
            for (r_idx, &exp_share) in expected_shares.iter().enumerate() {
                let (addr, share) = preview.recipients.get(r_idx as u32).unwrap();
                prop_assert_eq!(addr, w.recipients[r_idx].clone());
                prop_assert_eq!(share, exp_share);
            }

            // 3. `settle_sale` check against real token balance deltas
            let seller = &w.sellers[idx % SELLER_POOL];
            let payer_before = w.balance(&w.payer);
            let seller_before = w.balance(seller);
            let recipients_before = w.recipient_balances();

            let settled = client.settle_sale(&w.collection, &w.token, &w.payer, seller, &amount);
            prop_assert_eq!(settled.royalty_share, quote.royalty_amount);
            prop_assert_eq!(settled.seller_net, quote.seller_net);
            prop_assert_eq!(settled.royalty_share + settled.seller_net, amount);

            let seller_delta = w.balance(seller) - seller_before;
            let recipient_deltas: Vec<i128> = w
                .recipient_balances()
                .iter()
                .zip(&recipients_before)
                .map(|(after, before)| after - before)
                .collect();
            let total_paid_out = seller_delta + recipient_deltas.iter().sum::<i128>();
            prop_assert_eq!(seller_delta, quote.seller_net);
            prop_assert_eq!(recipient_deltas, expected_shares);
            prop_assert_eq!(total_paid_out, amount);
            prop_assert_eq!(payer_before - w.balance(&w.payer), amount);
            prop_assert_eq!(w.balance(&w.contract_id), 0);

            expected_gross += amount;
            expected_royalties += settled.royalty_share;
            expected_sales += 1;
        }

        // 4. `distribute` check at the first, middle, and last edge amounts
        for &probe_idx in &[0usize, amounts.len() / 2, amounts.len() - 1] {
            let amount = amounts[probe_idx];
            let quote = client.quote_sale(&w.collection, &amount);
            let seller = &w.sellers[probe_idx % SELLER_POOL];
            let payer_before = w.balance(&w.payer);
            let seller_before = w.balance(seller);
            let recipients_before = w.recipient_balances();

            let net = client.distribute(&w.collection, &w.token, &w.payer, seller, &amount);
            prop_assert_eq!(net, quote.seller_net);
            prop_assert_eq!(w.balance(seller), seller_before, "distribute does not transfer seller net");
            let recipient_sum: i128 = w
                .recipient_balances()
                .iter()
                .zip(&recipients_before)
                .map(|(after, before)| after - before)
                .sum();
            prop_assert_eq!(recipient_sum, quote.royalty_amount);
            prop_assert_eq!(payer_before - w.balance(&w.payer), quote.royalty_amount);
            prop_assert_eq!(net + recipient_sum, amount);

            expected_gross += amount;
            expected_royalties += quote.royalty_amount;
            expected_sales += 1;
        }

        let summary = client.get_settlement_summary(&w.collection);
        prop_assert_eq!(
            summary,
            SettlementSummary {
                sales: expected_sales,
                gross_volume: expected_gross,
                royalties_paid: expected_royalties,
            }
        );
    }

    /// A `settle_sales` batch mixing bracket-edge amounts across multiple tiers
    /// settles each sale identically to its `quote_sale`, conserves value
    /// across all sellers and recipients, and commits exact aggregate totals.
    #[test]
    fn prop_tiered_mixed_bracket_batch_matches_quotes_and_conserves_value(
        tiers in tier_table_strategy(),
        weights in split_weights_strategy(),
        tail_offset in 2_i128..=1_000_000_i128,
        picks in prop_vec((0usize..SELLER_POOL, any::<usize>()), 1..=MAX_BATCH_SALES),
    ) {
        prop_assert!(picks.len() <= MAX_SETTLE_SALES as usize);
        let pool = bracket_edge_amounts(&tiers, tail_offset);
        let plan: Vec<(usize, i128)> = picks
            .into_iter()
            .map(|(seller_idx, pick)| (seller_idx, pool[pick % pool.len()]))
            .collect();
        let total: i128 = plan.iter().map(|(_, amount)| *amount).sum();

        let w = setup_tiered_world(&weights, &tiers, total);
        let client = w.client();

        let mut quotes = Vec::with_capacity(plan.len());
        let mut sales = soroban_sdk::Vec::new(&w.env);
        for &(seller_idx, amount) in &plan {
            let quote = client.quote_sale(&w.collection, &amount);
            prop_assert_eq!(quote.royalty_bps, oracle_tier_bps(&tiers, amount));
            quotes.push(quote);
            sales.push_back((w.sellers[seller_idx].clone(), amount));
        }

        let settlements = client.settle_sales(&w.collection, &w.token, &w.payer, &sales);
        prop_assert_eq!(settlements.len() as usize, plan.len());

        let mut expected_seller_gain = std::vec![0_i128; SELLER_POOL];
        let mut expected_recipient_gain = std::vec![0_i128; w.recipients.len()];
        let mut expected_royalties = 0_i128;

        for (i, (&(seller_idx, amount), quote)) in plan.iter().zip(&quotes).enumerate() {
            let settled = settlements.get(i as u32).unwrap();
            prop_assert_eq!(settled.royalty_share, quote.royalty_amount);
            prop_assert_eq!(settled.seller_net, quote.seller_net);
            prop_assert_eq!(settled.royalty_share + settled.seller_net, amount);

            expected_seller_gain[seller_idx] += settled.seller_net;
            expected_royalties += settled.royalty_share;
            let shares = w.expected_shares(amount, quote.royalty_bps);
            for (gain, share) in expected_recipient_gain.iter_mut().zip(shares) {
                *gain += share;
            }
        }

        for (idx, seller) in w.sellers.iter().enumerate() {
            prop_assert_eq!(w.balance(seller), expected_seller_gain[idx]);
        }
        for (idx, recipient) in w.recipients.iter().enumerate() {
            prop_assert_eq!(w.balance(recipient), expected_recipient_gain[idx]);
        }
        let sum_sellers: i128 = expected_seller_gain.iter().sum();
        let sum_recipients: i128 = expected_recipient_gain.iter().sum();
        prop_assert_eq!(sum_sellers + sum_recipients, total);
        prop_assert_eq!(w.balance(&w.payer), 0);
        prop_assert_eq!(w.balance(&w.contract_id), 0);

        let summary = client.get_settlement_summary(&w.collection);
        prop_assert_eq!(
            summary,
            SettlementSummary {
                sales: plan.len() as u32,
                gross_volume: total,
                royalties_paid: expected_royalties,
            }
        );
    }
}
