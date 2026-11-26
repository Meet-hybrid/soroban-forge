//! Property-based tests for the two-step royalty recipient rotation feature.
//!
//! Invariants verified:
//! 1. Settlement conservation: `royalty_share + seller_net == gross` holds for every
//!    settlement, regardless of whether a rotation is pending or accepted.
//! 2. Pending rotations do not change payout destinations — settlement goes to the
//!    current recipient, not the proposed one.
//! 3. Accepted rotations change all subsequent settlements to the new recipient
//!    without changing bps or recipient order.
//! 4. No rotation creates or destroys royalty value — the total settled across
//!    multiple operations remains conservative.
//! 5. Interleaved sequences of propose/accept/cancel/settle are always consistent.

use crate::{MarketplaceRoyalties, RoyaltyShare, SorobanForgeMarketplaceRoyaltiesClient};
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::vec::Vec;

const MAX_AMOUNT: i128 = 10_000_000;

// ── World setup ───────────────────────────────────────────────────────────────

struct RotationWorld {
    env: Env,
    token: Address,
    contract_id: Address,
    collection: Address,
    /// Current configured primary recipient.
    recipient: Address,
    /// A second pre-generated address for use as `new_addr` in rotations.
    new_addr: Address,
    /// A third address, used as a second split recipient.
    third: Address,
    seller: Address,
    payer: Address,
}

impl RotationWorld {
    fn client(&self) -> SorobanForgeMarketplaceRoyaltiesClient<'_> {
        SorobanForgeMarketplaceRoyaltiesClient::new(&self.env, &self.contract_id)
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }

    fn balance(&self, addr: &Address) -> i128 {
        self.token_client().balance(addr)
    }
}

fn setup_world(bps: u32, mint: i128) -> RotationWorld {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();

    let collection = Address::generate(&env);
    let recipient = Address::generate(&env);
    let new_addr = Address::generate(&env);
    let third = Address::generate(&env);
    let seller = Address::generate(&env);
    let payer = Address::generate(&env);

    StellarAssetClient::new(&env, &token).mint(&payer, &mint);

    let contract_id = env.register(MarketplaceRoyalties, ());
    let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
    client.set_royalty(&collection, &recipient, &bps);

    RotationWorld {
        env,
        token,
        contract_id,
        collection,
        recipient,
        new_addr,
        third,
        seller,
        payer,
    }
}

// ── Strategies ────────────────────────────────────────────────────────────────

fn bps_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(10_000u32), 1u32..=9_999u32,]
}

fn amount_strategy() -> impl Strategy<Value = i128> {
    prop_oneof![1i128..=100, 1i128..=100_000, 1i128..=MAX_AMOUNT,]
}

/// A sequence of sale amounts (1–8 items).
fn amounts_strategy() -> impl Strategy<Value = Vec<i128>> {
    prop::collection::vec(amount_strategy(), 1..=8)
}

// ── Action enum for interleaved sequences ────────────────────────────────────

#[derive(Clone, Debug)]
enum RotationAction {
    Propose,
    Accept,
    Cancel,
    Settle(i128),
    SettleBatch(Vec<i128>),
}

fn action_strategy() -> impl Strategy<Value = RotationAction> {
    prop_oneof![
        Just(RotationAction::Propose),
        Just(RotationAction::Accept),
        Just(RotationAction::Cancel),
        amount_strategy().prop_map(RotationAction::Settle),
        amounts_strategy().prop_map(RotationAction::SettleBatch),
    ]
}

fn sequence_strategy() -> impl Strategy<Value = Vec<RotationAction>> {
    prop::collection::vec(action_strategy(), 1..=12)
}

// ── Invariant 1: Conservation on a single settle after pending proposal ───────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_rotation_pending_settlement_conservation(
        bps in bps_strategy(),
        amount in amount_strategy(),
    ) {
        let total_mint = amount * 2;
        let w = setup_world(bps, total_mint);
        let client = w.client();

        // Propose a rotation — do NOT accept.
        client.propose_recipient_rotation(&w.collection, &w.recipient, &w.new_addr);

        let before_payer = w.balance(&w.payer);
        let result = client.settle_sale(
            &w.collection, &w.token, &w.payer, &w.seller, &amount,
        );

        // Conservation.
        prop_assert_eq!(result.royalty_share + result.seller_net, amount);
        // Payer balance decreased by exact gross.
        prop_assert_eq!(w.balance(&w.payer), before_payer - amount);
        // Pending rotation did not alter payouts — recipient still received royalty.
        prop_assert!(w.balance(&w.new_addr) == 0,
            "new_addr should not receive anything while rotation is pending");
    }

    #[test]
    fn prop_rotation_accepted_settlement_conservation(
        bps in bps_strategy(),
        amount in amount_strategy(),
    ) {
        let total_mint = amount * 2;
        let w = setup_world(bps, total_mint);
        let client = w.client();

        // Complete the rotation.
        client.propose_recipient_rotation(&w.collection, &w.recipient, &w.new_addr);
        client.accept_recipient_rotation(&w.collection, &w.recipient);

        let before_payer = w.balance(&w.payer);
        let result = client.settle_sale(
            &w.collection, &w.token, &w.payer, &w.seller, &amount,
        );

        // Conservation holds.
        prop_assert_eq!(result.royalty_share + result.seller_net, amount);
        prop_assert_eq!(w.balance(&w.payer), before_payer - amount);

        // After acceptance old recipient receives nothing, new_addr receives royalty
        // when the share actually floors to something positive.
        if result.royalty_share > 0 {
            prop_assert!(w.balance(&w.new_addr) > 0,
                "new_addr should receive royalty after accepted rotation (bps={}, share={})",
                bps, result.royalty_share);
            prop_assert_eq!(w.balance(&w.recipient), 0i128,
                "old recipient should receive nothing after rotation");
        }
    }

    #[test]
    fn prop_rotation_cancel_does_not_change_recipient(
        bps in bps_strategy(),
        amount in amount_strategy(),
    ) {
        let total_mint = amount * 2;
        let w = setup_world(bps, total_mint);
        let client = w.client();

        client.propose_recipient_rotation(&w.collection, &w.recipient, &w.new_addr);
        client.cancel_recipient_rotation(&w.collection, &w.recipient);

        let before_recipient = w.balance(&w.recipient);
        let result = client.settle_sale(
            &w.collection, &w.token, &w.payer, &w.seller, &amount,
        );

        prop_assert_eq!(result.royalty_share + result.seller_net, amount);
        // Old recipient still gets royalty when the share is positive.
        if result.royalty_share > 0 {
            prop_assert!(w.balance(&w.recipient) > before_recipient,
                "old recipient should still receive royalty after cancel (bps={}, share={})",
                bps, result.royalty_share);
        }
        prop_assert_eq!(w.balance(&w.new_addr), 0i128);
    }

    #[test]
    fn prop_multi_recipient_rotation_conservation(
        bps_a in 100u32..=4_000u32,
        bps_b in 100u32..=4_000u32,
        amounts in amounts_strategy(),
    ) {
        let total: i128 = amounts.iter().sum::<i128>() + 1;
        let w = setup_world(0, total * 2);
        let client = w.client();

        // Set up a split with two recipients.
        let bps_a = bps_a.min(5_000);
        let bps_b = bps_b.min(10_000 - bps_a);
        let mut splits = soroban_sdk::Vec::new(&w.env);
        splits.push_back(RoyaltyShare { recipient: w.recipient.clone(), bps: bps_a });
        splits.push_back(RoyaltyShare { recipient: w.third.clone(), bps: bps_b });
        client.set_royalty_splits(&w.collection, &splits);

        // Propose rotation of w.recipient → w.new_addr and accept.
        client.propose_recipient_rotation(&w.collection, &w.recipient, &w.new_addr);
        client.accept_recipient_rotation(&w.collection, &w.recipient);

        let before_payer = w.balance(&w.payer);
        let mut total_gross = 0i128;

        for amount in &amounts {
            let result = client.settle_sale(
                &w.collection, &w.token, &w.payer, &w.seller, amount,
            );
            prop_assert_eq!(result.royalty_share + result.seller_net, *amount,
                "conservation per sale: amount={}", amount);
            total_gross += amount;
        }

        prop_assert_eq!(w.balance(&w.payer), before_payer - total_gross);
        // old recipient receives nothing after rotation
        prop_assert_eq!(w.balance(&w.recipient), 0i128);
    }

    #[test]
    fn prop_interleaved_rotation_sequence_maintains_conservation(
        bps in 100u32..=2_000u32,
        actions in sequence_strategy(),
    ) {
        // We need enough mint to cover any settle actions.
        let max_total = 1_000_000_000i128;
        let w = setup_world(bps, max_total);
        let client = w.client();

        // Track whether a rotation is pending.
        let mut pending = false;
        // Track whether the rotation has been accepted.
        let mut accepted = false;

        for action in &actions {
            match action {
                RotationAction::Propose => {
                    // Only propose if not already pending.
                    if !pending {
                        let current = if accepted { &w.new_addr } else { &w.recipient };
                        let next = if accepted { &w.third } else { &w.new_addr };
                        // Check next is not already a recipient — skip if so.
                        if current != next {
                            if let Ok(Ok(())) = client.try_propose_recipient_rotation(
                                &w.collection, current, next,
                            ) {
                                pending = true;
                            }
                        }
                    }
                }
                RotationAction::Accept => {
                    if pending {
                        let current = if accepted { &w.new_addr } else { &w.recipient };
                        if let Ok(Ok(())) = client.try_accept_recipient_rotation(
                            &w.collection, current,
                        ) {
                            pending = false;
                            accepted = !accepted; // toggle so the third becomes next
                        }
                    }
                }
                RotationAction::Cancel => {
                    if pending {
                        let current = if accepted { &w.new_addr } else { &w.recipient };
                        if let Ok(Ok(())) = client.try_cancel_recipient_rotation(
                            &w.collection, current,
                        ) {
                            pending = false;
                        }
                    }
                }
                RotationAction::Settle(amount) => {
                    let before = w.balance(&w.payer);
                    if let Ok(Ok(result)) = client.try_settle_sale(
                        &w.collection, &w.token, &w.payer, &w.seller, amount,
                    ) {
                        // Conservation invariant.
                        prop_assert_eq!(result.royalty_share + result.seller_net, *amount,
                            "conservation: amount={}", amount);
                        prop_assert_eq!(w.balance(&w.payer), before - amount);
                    }
                }
                RotationAction::SettleBatch(amounts) => {
                    if amounts.is_empty() {
                        continue;
                    }
                    let mut sales = soroban_sdk::Vec::new(&w.env);
                    let mut total_a = 0i128;
                    for a in amounts {
                        sales.push_back((w.seller.clone(), *a));
                        total_a += a;
                    }
                    let before = w.balance(&w.payer);
                    if let Ok(Ok(settlements)) = client.try_settle_sales(
                        &w.collection, &w.token, &w.payer, &sales,
                    ) {
                        for (i, s) in settlements.iter().enumerate() {
                            let a = amounts[i];
                            prop_assert_eq!(s.royalty_share + s.seller_net, a,
                                "batch conservation: sale={} amount={}", i, a);
                        }
                        prop_assert_eq!(w.balance(&w.payer), before - total_a);
                    }
                }
            }
        }
    }

    #[test]
    fn prop_rotation_does_not_change_total_bps(
        bps in 100u32..=5_000u32,
    ) {
        let w = setup_world(bps, 0);
        let client = w.client();

        let before = client.get_royalty(&w.collection);
        prop_assert_eq!(before.bps, bps);

        client.propose_recipient_rotation(&w.collection, &w.recipient, &w.new_addr);
        client.accept_recipient_rotation(&w.collection, &w.recipient);

        let after = client.get_royalty(&w.collection);
        prop_assert_eq!(after.bps, bps, "bps must not change after rotation");
    }
}
