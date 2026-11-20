//! Property-based suite for multi-asset basket settlement (2–8 legs).
//!
//! `props.rs` covers two-leg baskets on the direct release/refund/resolve
//! paths. This module generalizes to the full [`MAX_BASKET_ASSETS`] range and
//! to the review-window flow added in #644, with an independent model checked
//! against the contract after every step:
//!
//! * status, every leg's `released`, and the pending release request;
//! * each token's custody equal to that leg's unreleased amount;
//! * per-token conservation (nothing minted or lost);
//! * leg independence — a partial release on one leg never moves another;
//! * a pending request blocking the buyer's after-deadline refund with
//!   `ReleasePending`, and being cleared by any payout, refund or dispute.
//!
//! Every property runs at least 256 cases against a real Stellar Asset
//! Contract per leg. Seeding is deterministic; `PROPTEST_CASES` can raise the
//! case count for a longer local run.

extern crate std;

use crate::{
    BasketEscrowData, Escrow, EscrowAsset, EscrowStatus, SorobanForgeEscrowClient,
    DEFAULT_REVIEW_PERIOD, MAX_BASKET_ASSETS,
};
use proptest::prelude::*;
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec};

const START: u64 = 1_000_000;
const MAX_AMOUNT: i128 = 1_000_000_000_000;
const DAY: u64 = 24 * 60 * 60;

/// One basket with one SAC per leg, plus the parties and the escrow contract.
struct BasketWorld {
    env: Env,
    escrow: Address,
    buyer: Address,
    seller: Address,
    arbiter: Address,
    tokens: Vec<Address>,
    amounts: Vec<i128>,
}

fn setup_world(amounts: &[i128]) -> BasketWorld {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let mut tokens = Vec::new(&env);
    let mut declared = Vec::new(&env);
    for amount in amounts {
        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        tokens.push_back(token);
        declared.push_back(*amount);
    }

    BasketWorld {
        escrow: env.register(Escrow, ()),
        buyer: Address::generate(&env),
        seller: Address::generate(&env),
        arbiter: Address::generate(&env),
        tokens,
        amounts: declared,
        env,
    }
}

impl BasketWorld {
    fn client(&self) -> SorobanForgeEscrowClient<'_> {
        SorobanForgeEscrowClient::new(&self.env, &self.escrow)
    }

    fn legs(&self) -> u32 {
        self.tokens.len()
    }

    fn leg_assets(&self) -> Vec<EscrowAsset> {
        let mut assets = Vec::new(&self.env);
        for at in 0..self.legs() {
            assets.push_back(EscrowAsset {
                token: self.tokens.get_unchecked(at),
                amount: self.amounts.get_unchecked(at),
                released: 0,
            });
        }
        assets
    }

    /// Mint each leg's exact deposit to the buyer, then create and fund the
    /// basket. Each token's minted supply is therefore exactly its leg amount.
    fn create_funded(&self, timeout: u64) -> u64 {
        for at in 0..self.legs() {
            StellarAssetClient::new(&self.env, &self.tokens.get_unchecked(at))
                .mint(&self.buyer, &self.amounts.get_unchecked(at));
        }
        let id = self.client().create_basket(
            &self.buyer,
            &self.seller,
            &self.arbiter,
            &self.leg_assets(),
            &timeout,
        );
        self.client().deposit_basket(&id);
        id
    }

    fn token(&self, leg: u32) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.tokens.get_unchecked(leg))
    }

    fn custody(&self, leg: u32) -> i128 {
        self.token(leg).balance(&self.escrow)
    }

    /// Every token ever minted: buyer + seller + arbiter + contract.
    fn pool_total(&self, leg: u32) -> i128 {
        let token = self.token(leg);
        token.balance(&self.buyer)
            + token.balance(&self.seller)
            + token.balance(&self.arbiter)
            + token.balance(&self.escrow)
    }

    fn record(&self, id: u64) -> BasketEscrowData {
        self.client().get_basket(&id)
    }
}

/// The model's view of one basket. Independent of contract storage: every
/// field is advanced by the modelled rule, then compared against the record.
struct Model {
    status: EscrowStatus,
    released: std::vec::Vec<i128>,
    pending_release_at: Option<u64>,
}

impl Model {
    fn new(legs: u32) -> Self {
        Self {
            status: EscrowStatus::Funded,
            released: std::vec![0; usize::try_from(legs).expect("legs fit in usize")],
            pending_release_at: None,
        }
    }
}

/// checked-in model assertion: status, per-leg `released`, pending request and
/// per-token custody all agree with the model.
fn assert_world(
    world: &BasketWorld,
    id: u64,
    model: &Model,
    amounts: &[i128],
) -> Result<(), TestCaseError> {
    let record = world.record(id);
    prop_assert_eq!(
        &record.status,
        &model.status,
        "status diverged from the model"
    );
    prop_assert_eq!(
        record.assets.len(),
        u32::try_from(model.released.len()).expect("model legs fit in u32")
    );

    for leg in 0..record.assets.len() {
        let asset = record.assets.get_unchecked(leg);
        prop_assert_eq!(
            asset.released,
            model.released[leg as usize],
            "leg {} released diverged from the model",
            leg
        );
        prop_assert_eq!(
            asset.remaining(),
            amounts[leg as usize] - model.released[leg as usize],
            "leg {} remaining must be the unreleased amount",
            leg
        );
        prop_assert_eq!(
            world.custody(leg),
            asset.remaining(),
            "leg {} custody must equal its unreleased amount",
            leg
        );
        prop_assert_eq!(
            world.pool_total(leg),
            amounts[leg as usize],
            "leg {} must conserve its minted supply",
            leg
        );
    }

    let on_chain = world
        .client()
        .get_release_request(&id)
        .map(|r| r.release_at);
    prop_assert_eq!(
        on_chain,
        model.pending_release_at,
        "pending release request diverged from the model"
    );
    Ok(())
}

/// op, leg pick, resolve direction, time step.
fn action() -> impl Strategy<Value = (u8, u8, bool, u64)> {
    (0u8..=7, 0u8..=u8::MAX, prop::bool::ANY, 0u64..=4 * DAY)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Random 2–8-leg baskets stepped through the review-window flow. After
    /// every call the contract must agree with the model, including custody
    /// per token, and no call may create or destroy value.
    #[test]
    fn p8_basket_review_sequences_match_model(
        amounts in prop::collection::vec(1i128..=MAX_AMOUNT, 2..=8),
        timeout in 1u64..=30 * DAY,
        actions in prop::collection::vec(action(), 0..=16),
    ) {
        let world = setup_world(&amounts);
        let id = world.create_funded(timeout);
        let client = world.client();
        let review = DEFAULT_REVIEW_PERIOD;
        let deadline = START + timeout;

        let mut now = START;
        let mut model = Model::new(world.legs());
        let minted = amounts.clone();

        assert_world(&world, id, &model, &amounts)?;

        for (op, leg_pick, pay_seller, step) in actions {
            let leg = u32::from(leg_pick) % world.legs();
            let before = world.record(id);
            let remaining = amounts[leg as usize] - model.released[leg as usize];

            match op {
                // Seller asks to be paid every leg.
                0 => {
                    let res = client.try_request_release_basket(&id);
                    let allowed = model.status == EscrowStatus::Funded
                        && model.pending_release_at.is_none()
                        && now <= deadline;
                    prop_assert_eq!(
                        res.is_ok(),
                        allowed,
                        "request at now={} deadline={}",
                        now,
                        deadline
                    );
                    if allowed {
                        model.pending_release_at = Some(now + review);
                    }
                }
                // Buyer approves: pays every leg immediately.
                1 => {
                    let res = client.try_approve_release(&id);
                    let allowed = model.status == EscrowStatus::Funded
                        && model.pending_release_at.is_some();
                    prop_assert_eq!(res.is_ok(), allowed, "approve");
                    if allowed {
                        model.released = amounts.to_vec();
                        model.status = EscrowStatus::Completed;
                        model.pending_release_at = None;
                    }
                }
                // Anyone executes once the window has ended.
                2 => {
                    let res = client.try_execute_release(&id);
                    let allowed = model.status == EscrowStatus::Funded
                        && model.pending_release_at.is_some_and(|at| now >= at);
                    prop_assert_eq!(res.is_ok(), allowed, "execute at now={}", now);
                    if allowed {
                        model.released = amounts.to_vec();
                        model.status = EscrowStatus::Completed;
                        model.pending_release_at = None;
                    }
                }
                // Buyer or seller freezes every leg.
                3 => {
                    let claimant = if pay_seller { &world.buyer } else { &world.seller };
                    let res = client.try_dispute_basket(&id, claimant);
                    let allowed = model.status == EscrowStatus::Funded;
                    prop_assert_eq!(res.is_ok(), allowed, "dispute");
                    if allowed {
                        model.status = EscrowStatus::Disputed;
                        model.pending_release_at = None;
                    }
                }
                // Arbiter decides for one side, every leg.
                4 => {
                    let res = client.try_resolve_basket(&id, &pay_seller);
                    let allowed = model.status == EscrowStatus::Disputed;
                    prop_assert_eq!(res.is_ok(), allowed, "resolve");
                    if allowed {
                        model.released = amounts.to_vec();
                        model.status = if pay_seller {
                            EscrowStatus::Completed
                        } else {
                            EscrowStatus::Refunded
                        };
                        model.pending_release_at = None;
                    }
                }
                // Refund: seller before the deadline, buyer after it, unless a
                // request is pending (then the buyer must approve or dispute).
                5 => {
                    let before_deadline = now < deadline;
                    let blocked = !before_deadline && model.pending_release_at.is_some();
                    let allowed = model.status == EscrowStatus::Funded && !blocked;
                    let res = client.try_refund_basket(&id);
                    prop_assert_eq!(
                        res.is_ok(),
                        allowed,
                        "refund at now={} deadline={} pending={:?}",
                        now,
                        deadline,
                        model.pending_release_at
                    );
                    if blocked {
                        prop_assert_eq!(res, Err(Ok(ForgeError::ReleasePending)));
                    }
                    if allowed {
                        model.released = amounts.to_vec();
                        model.status = EscrowStatus::Refunded;
                        model.pending_release_at = None;
                    }
                }
                // Buyer pays part of one randomly chosen leg.
                6 => {
                    let token = world.tokens.get_unchecked(leg);
                    let ask = ((amounts[leg as usize] / 10).max(1)).min(remaining.max(1));
                    let allowed = model.status == EscrowStatus::Funded && remaining > 0;
                    let res = client.try_release_partial_basket(&id, &token, &ask);
                    prop_assert_eq!(
                        res.is_ok(),
                        allowed,
                        "partial leg={} ask={} remaining={}",
                        leg,
                        ask,
                        remaining
                    );
                    if allowed {
                        model.released[leg as usize] += ask;
                        if model
                            .released
                            .iter()
                            .zip(amounts.iter())
                            .all(|(released, amount)| released == amount)
                        {
                            model.status = EscrowStatus::Completed;
                        }
                        // Any payout clears a pending request.
                        model.pending_release_at = None;
                    }

                    // Leg independence: no other leg's accounting moved.
                    let after = world.record(id);
                    for other in 0..world.legs() {
                        if other == leg {
                            continue;
                        }
                        prop_assert_eq!(
                            after.assets.get_unchecked(other).released,
                            before.assets.get_unchecked(other).released,
                            "partial on leg {} changed leg {}",
                            leg,
                            other
                        );
                        prop_assert_eq!(
                            world.custody(other),
                            amounts[other as usize] - model.released[other as usize],
                            "partial on leg {} moved custody of leg {}",
                            leg,
                            other
                        );
                    }
                }
                // Time passes (possibly past the deadline or `release_at`).
                _ => {
                    now += step;
                    world.env.ledger().set_timestamp(now);
                }
            }

            assert_world(&world, id, &model, &amounts)?;
            for leg in 0..world.legs() {
                prop_assert_eq!(
                    world.pool_total(leg),
                    minted[leg as usize],
                    "leg {} supply must be conserved after every step",
                    leg
                );
            }
        }

        // What the model calls terminal is terminal: no further payout works.
        if matches!(
            model.status,
            EscrowStatus::Completed | EscrowStatus::Refunded
        ) {
            prop_assert!(client.try_release_basket(&id).is_err());
            prop_assert!(client.try_refund_basket(&id).is_err());
            prop_assert!(client.try_dispute_basket(&id, &world.buyer).is_err());
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A partial release moves value only on the named leg: every other leg's
    /// `released`, remaining balance and custody stay byte-for-byte the same.
    #[test]
    fn p9_partial_release_leaves_other_legs_untouched(
        amounts in prop::collection::vec(10i128..=MAX_AMOUNT, 2..=8),
        leg_pick in 0u8..=u8::MAX,
        frac in 1u8..=9,
    ) {
        let world = setup_world(&amounts);
        let id = world.create_funded(30 * DAY);
        let client = world.client();
        let leg = u32::from(leg_pick) % world.legs();

        let before = world.record(id);
        let ask = (amounts[leg as usize] * i128::from(frac) / 10).max(1);
        client.release_partial_basket(&id, &world.tokens.get_unchecked(leg), &ask);

        let after = world.record(id);
        prop_assert_eq!(
            after.assets.get_unchecked(leg).released,
            ask,
            "the named leg must record the partial payment"
        );
        for other in 0..world.legs() {
            if other == leg {
                continue;
            }
            prop_assert_eq!(
                after.assets.get_unchecked(other).released,
                before.assets.get_unchecked(other).released,
                "leg {} changed by a partial on leg {}",
                other,
                leg
            );
            prop_assert_eq!(
                world.custody(other),
                amounts[other as usize],
                "leg {} custody changed by a partial on leg {}",
                other,
                leg
            );
        }
        prop_assert_eq!(
            world.custody(leg),
            amounts[leg as usize] - ask,
            "the named leg's custody must equal its unreleased balance"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A seller request made before the deadline blocks the buyer's
    /// after-deadline refund with `ReleasePending`; approving or disputing
    /// clears the request, and approving then pays every leg.
    #[test]
    fn p10_pending_request_blocks_after_deadline_refund(
        amounts in prop::collection::vec(1i128..=MAX_AMOUNT, 2..=8),
        timeout in 1u64..=10 * DAY,
        approve in prop::bool::ANY,
    ) {
        let world = setup_world(&amounts);
        let id = world.create_funded(timeout);
        let client = world.client();

        client.request_release_basket(&id);
        let request = client.get_release_request(&id).expect("request is pending");
        prop_assert_eq!(request.amount, 0, "a basket request covers every leg");
        prop_assert_eq!(request.release_at, START + DEFAULT_REVIEW_PERIOD);

        // Move strictly past the escrow deadline; the review window is three
        // days, so also past `release_at` for any timeout <= 10 days.
        let past_deadline = START + timeout + 1;
        world.env.ledger().set_timestamp(past_deadline);

        let err = client.try_refund_basket(&id).unwrap_err().unwrap();
        prop_assert_eq!(err, ForgeError::ReleasePending);
        prop_assert_eq!(client.get_status(&id), EscrowStatus::Funded);

        if approve {
            client.approve_release(&id);
            prop_assert_eq!(client.get_status(&id), EscrowStatus::Completed);
        } else {
            client.dispute_basket(&id, &world.buyer);
            prop_assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
            // A dispute freezes the basket; the arbiter then settles it.
            client.resolve_basket(&id, &true);
            prop_assert_eq!(client.get_status(&id), EscrowStatus::Completed);
        }
        prop_assert_eq!(client.get_release_request(&id), None);

        // Every token's custody is empty: the whole basket settled.
        for leg in 0..world.legs() {
            prop_assert_eq!(world.custody(leg), 0, "leg {} custody must be empty", leg);
            prop_assert_eq!(
                world.pool_total(leg),
                amounts[leg as usize],
                "leg {} supply conserved",
                leg
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The 8-leg maximum is enforceable and a 9-leg basket is rejected, so the
    /// property range above really is the whole supported space.
    #[test]
    fn p11_basket_leg_limit_is_enforced(
        legs in (MAX_BASKET_ASSETS + 1)..=(MAX_BASKET_ASSETS + 4),
    ) {
        prop_assert_eq!(MAX_BASKET_ASSETS, 8);

        let amounts = std::vec![1_i128; usize::try_from(legs).expect("legs fit in usize")];
        let world = setup_world(&amounts);
        let client = world.client();

        let err = client
            .try_create_basket(
                &world.buyer,
                &world.seller,
                &world.arbiter,
                &world.leg_assets(),
                &30_u64,
            )
            .unwrap_err()
            .unwrap();
        prop_assert_eq!(err, ForgeError::InvalidInput);
    }
}
