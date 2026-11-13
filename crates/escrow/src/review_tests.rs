//! Release with a review window: the seller requests payment, the buyer can
//! approve or dispute until the window ends, and then anyone can execute.
//!
//! Every test checks real SAC balances, so "paid" means tokens moved.

use crate::{
    Escrow, EscrowAsset, EscrowStatus, ReleaseRequest, SorobanForgeEscrowClient,
    DEFAULT_REVIEW_PERIOD, MAX_REVIEW_PERIOD, MIN_REVIEW_PERIOD,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Vec};

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 7 * 24 * 60 * 60;
const AMOUNT: i128 = 1_000;
const REVIEW: u64 = 3_600;

struct T<'a> {
    env: Env,
    token: Address,
    tc: TokenClient<'a>,
    contract: Address,
    client: SorobanForgeEscrowClient<'a>,
    buyer: Address,
    seller: Address,
    arbiter: Address,
}

fn setup() -> T<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);
    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin).address();
    let contract = env.register(Escrow, ());
    let client = SorobanForgeEscrowClient::new(&env, &contract);
    let (buyer, seller, arbiter) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    StellarAssetClient::new(&env, &token).mint(&buyer, &(AMOUNT * 10));
    let tc = TokenClient::new(&env, &token);
    T {
        env,
        token,
        tc,
        contract,
        client,
        buyer,
        seller,
        arbiter,
    }
}

impl T<'_> {
    /// A funded escrow of `AMOUNT` with a one-hour review window.
    fn funded(&self) -> u64 {
        let id = self.client.create_escrow_with_review(
            &self.buyer,
            &self.seller,
            &self.arbiter,
            &self.token,
            &AMOUNT,
            &TIMEOUT,
            &REVIEW,
        );
        self.client.deposit(&id);
        id
    }

    fn at(&self, t: u64) {
        self.env.ledger().set_timestamp(t);
    }
}

#[test]
fn request_pays_nothing_until_the_window_ends_then_anyone_executes() {
    let t = setup();
    let id = t.funded();
    let release_at = t.client.request_release(&id, &AMOUNT);
    assert_eq!(release_at, START + REVIEW);
    assert_eq!(
        t.client.get_release_request(&id),
        Some(ReleaseRequest {
            amount: AMOUNT,
            requested_at: START,
            release_at
        })
    );
    assert_eq!(t.tc.balance(&t.seller), 0);

    // One second early: refused, nothing moves.
    t.at(START + REVIEW - 1);
    assert_eq!(
        t.client.try_execute_release(&id),
        Err(Ok(ForgeError::DeadlineReached))
    );
    assert_eq!(t.tc.balance(&t.contract), AMOUNT);

    t.at(START + REVIEW);
    t.client.execute_release(&id);
    assert_eq!(t.tc.balance(&t.seller), AMOUNT);
    assert_eq!(t.tc.balance(&t.contract), 0);
    assert_eq!(t.client.get_status(&id), EscrowStatus::Completed);
    assert_eq!(t.client.get_release_request(&id), None);
}

#[test]
fn buyer_can_approve_early() {
    let t = setup();
    let id = t.funded();
    t.client.request_release(&id, &AMOUNT);
    t.client.approve_release(&id);
    assert_eq!(t.tc.balance(&t.seller), AMOUNT);
    assert_eq!(t.client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn a_dispute_during_the_window_blocks_the_payout() {
    let t = setup();
    let id = t.funded();
    t.client.request_release(&id, &AMOUNT);
    t.client.dispute(&id, &t.buyer);
    assert_eq!(t.client.get_release_request(&id), None);

    t.at(START + REVIEW + 1);
    assert_eq!(
        t.client.try_execute_release(&id),
        Err(Ok(ForgeError::NotFound))
    );
    assert_eq!(t.tc.balance(&t.seller), 0);
    assert_eq!(t.tc.balance(&t.contract), AMOUNT);

    // The arbiter decides; the old request never comes back.
    t.client.resolve(&id, &false);
    assert_eq!(t.tc.balance(&t.buyer), AMOUNT * 10);
}

#[test]
fn milestone_requests_pay_part_and_leave_the_rest() {
    let t = setup();
    let id = t.funded();
    t.client.request_release(&id, &300);
    t.at(START + REVIEW);
    t.client.execute_release(&id);
    assert_eq!(t.tc.balance(&t.seller), 300);
    assert_eq!(t.client.get_status(&id), EscrowStatus::Funded);
    assert_eq!(t.client.get_escrow(&id).released, 300);

    // A second milestone for everything left completes the escrow.
    t.client.request_release(&id, &700);
    t.client.approve_release(&id);
    assert_eq!(t.tc.balance(&t.seller), AMOUNT);
    assert_eq!(t.client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn request_amounts_and_states_are_validated() {
    let t = setup();
    let id = t.funded();
    assert_eq!(
        t.client.try_request_release(&id, &0),
        Err(Ok(ForgeError::InvalidInput))
    );
    assert_eq!(
        t.client.try_request_release(&id, &(AMOUNT + 1)),
        Err(Ok(ForgeError::InvalidInput))
    );
    t.client.request_release(&id, &100);
    assert_eq!(
        t.client.try_request_release(&id, &100),
        Err(Ok(ForgeError::ReleasePending))
    );

    let pending = t
        .client
        .create_escrow(&t.buyer, &t.seller, &t.arbiter, &t.token, &AMOUNT, &TIMEOUT);
    assert_eq!(
        t.client.try_request_release(&pending, &AMOUNT),
        Err(Ok(ForgeError::InvalidInput))
    );
    assert_eq!(
        t.client.try_approve_release(&pending),
        Err(Ok(ForgeError::NotFound))
    );
}

#[test]
fn a_buyer_cannot_wait_out_the_deadline_to_dodge_a_request() {
    let t = setup();
    let id = t.funded();
    // The seller delivers and asks just before the deadline.
    t.at(START + TIMEOUT - 10);
    t.client.request_release(&id, &AMOUNT);

    // After the deadline the buyer may normally reclaim, but not while the
    // request is waiting: both refund paths are refused.
    t.at(START + TIMEOUT + 1);
    assert_eq!(
        t.client.try_refund(&id),
        Err(Ok(ForgeError::ReleasePending))
    );
    assert_eq!(
        t.client.try_refund_expired(&id),
        Err(Ok(ForgeError::ReleasePending))
    );

    // The window runs on past the deadline and pays the seller.
    t.at(START + TIMEOUT - 10 + REVIEW);
    t.client.execute_release(&id);
    assert_eq!(t.tc.balance(&t.seller), AMOUNT);
}

#[test]
fn a_seller_cannot_block_the_buyers_refund_after_the_deadline() {
    let t = setup();
    let id = t.funded();
    t.at(START + TIMEOUT + 1);
    assert_eq!(
        t.client.try_request_release(&id, &AMOUNT),
        Err(Ok(ForgeError::DeadlineReached))
    );
    t.client.refund(&id);
    assert_eq!(t.tc.balance(&t.buyer), AMOUNT * 10);
}

#[test]
fn a_seller_refund_or_a_direct_release_clears_the_request() {
    let t = setup();
    let id = t.funded();
    t.client.request_release(&id, &AMOUNT);
    t.client.refund(&id); // before the deadline the seller may refund
    assert_eq!(t.client.get_release_request(&id), None);
    assert_eq!(t.tc.balance(&t.buyer), AMOUNT * 10);

    let id = t.funded();
    t.client.request_release(&id, &AMOUNT);
    t.client.release_partial(&id, &400); // buyer pays a part directly
    assert_eq!(t.client.get_release_request(&id), None);
    t.at(START + REVIEW + 1);
    assert_eq!(
        t.client.try_execute_release(&id),
        Err(Ok(ForgeError::NotFound))
    );
    assert_eq!(t.tc.balance(&t.seller), 400);
}

#[test]
fn refund_expired_returns_only_what_is_left_after_partial_releases() {
    // Two escrows in the same token share the contract's custody. Before this
    // fix, refund_expired paid the full original amount, taking the
    // difference from the other escrow.
    let t = setup();
    let a = t.funded();
    let b = t.funded();
    t.client.release_partial(&a, &400);
    t.at(START + TIMEOUT + 1);
    t.client.refund_expired(&a);
    assert_eq!(t.tc.balance(&t.seller), 400);
    assert_eq!(
        t.tc.balance(&t.contract),
        AMOUNT,
        "escrow b's funds must be untouched"
    );
    assert_eq!(t.client.get_escrow(&a).released, AMOUNT);
    t.client.refund_expired(&b);
    assert_eq!(t.tc.balance(&t.buyer), AMOUNT * 10 - 400);
    assert_eq!(t.tc.balance(&t.contract), 0);
}

#[test]
fn review_periods_are_bounded_and_default_to_three_days() {
    let t = setup();
    for bad in [0, MIN_REVIEW_PERIOD - 1, MAX_REVIEW_PERIOD + 1] {
        assert_eq!(
            t.client.try_create_escrow_with_review(
                &t.buyer, &t.seller, &t.arbiter, &t.token, &AMOUNT, &TIMEOUT, &bad
            ),
            Err(Ok(ForgeError::InvalidInput))
        );
    }
    let plain = t
        .client
        .create_escrow(&t.buyer, &t.seller, &t.arbiter, &t.token, &AMOUNT, &TIMEOUT);
    assert_eq!(t.client.review_period(&plain), DEFAULT_REVIEW_PERIOD);
    // Escrows created before review windows existed read as the default.
    assert_eq!(t.client.review_period(&9_999), DEFAULT_REVIEW_PERIOD);
}

#[test]
fn basket_requests_follow_the_same_rules() {
    let t = setup();
    let admin = Address::generate(&t.env);
    let token_b = t.env.register_stellar_asset_contract_v2(admin).address();
    StellarAssetClient::new(&t.env, &token_b).mint(&t.buyer, &AMOUNT);
    let mut assets = Vec::new(&t.env);
    assets.push_back(EscrowAsset {
        token: t.token.clone(),
        amount: AMOUNT,
        released: 0,
    });
    assets.push_back(EscrowAsset {
        token: token_b.clone(),
        amount: AMOUNT,
        released: 0,
    });
    let id = t
        .client
        .create_basket(&t.buyer, &t.seller, &t.arbiter, &assets, &TIMEOUT);
    t.client.deposit_basket(&id);

    let release_at = t.client.request_release_basket(&id);
    assert_eq!(release_at, START + DEFAULT_REVIEW_PERIOD);
    t.at(START + TIMEOUT + 1);
    assert_eq!(
        t.client.try_refund_basket(&id),
        Err(Ok(ForgeError::ReleasePending))
    );
    t.at(release_at);
    t.client.execute_release(&id);
    assert_eq!(t.tc.balance(&t.seller), AMOUNT);
    assert_eq!(
        TokenClient::new(&t.env, &token_b).balance(&t.seller),
        AMOUNT
    );
    assert_eq!(t.client.get_status(&id), EscrowStatus::Completed);
}

// ------------------------------------------------------------- who must sign

#[test]
fn only_the_seller_can_request_and_only_the_buyer_can_approve() {
    let t = setup();
    let id = t.funded();

    t.env.mock_auths(&[MockAuth {
        address: &t.buyer,
        invoke: &MockAuthInvoke {
            contract: &t.contract,
            fn_name: "request_release",
            args: (id, AMOUNT).into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    assert!(matches!(
        t.client.try_request_release(&id, &AMOUNT),
        Err(Err(InvokeError::Abort))
    ));

    t.env.mock_all_auths();
    t.client.request_release(&id, &AMOUNT);

    t.env.mock_auths(&[MockAuth {
        address: &t.seller,
        invoke: &MockAuthInvoke {
            contract: &t.contract,
            fn_name: "approve_release",
            args: (id,).into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    assert!(matches!(
        t.client.try_approve_release(&id),
        Err(Err(InvokeError::Abort))
    ));
    assert_eq!(t.tc.balance(&t.seller), 0);
}

#[test]
fn execute_needs_no_signature() {
    let t = setup();
    let id = t.funded();
    t.client.request_release(&id, &AMOUNT);
    t.at(START + REVIEW);
    // No signatures armed at all: a keeper (or the seller) can finish it.
    t.env.mock_auths(&[]);
    t.client.execute_release(&id);
    assert_eq!(t.tc.balance(&t.seller), AMOUNT);
}
