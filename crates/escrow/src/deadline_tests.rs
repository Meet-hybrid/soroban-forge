//! Buyer-authorized deadline extension for funded escrows of either kind.
//!
//! Every refusal also checks that the stored `timeout` is unchanged, so "was
//! refused" always means "nothing was written".

extern crate std;

use crate::{events, Escrow, EscrowAsset, EscrowStatus, ReleaseRequest, SorobanForgeEscrowClient};
use soroban_forge_shared_utils::ttl::{BUMP_AMOUNT, BUMP_THRESHOLD};
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::{advance_ledger_to, entry_ttl};
use soroban_sdk::events::Event as _;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Vec};

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 7 * 24 * 60 * 60;
const EXTENDED: u64 = 2 * TIMEOUT;
const AMOUNT: i128 = 1_000;

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
    fn pending(&self) -> u64 {
        self.client.create_escrow(
            &self.buyer,
            &self.seller,
            &self.arbiter,
            &self.token,
            &AMOUNT,
            &TIMEOUT,
        )
    }

    fn funded(&self) -> u64 {
        let id = self.pending();
        self.client.deposit(&id);
        id
    }

    fn funded_basket(&self) -> u64 {
        let mut assets = Vec::new(&self.env);
        assets.push_back(EscrowAsset {
            token: self.token.clone(),
            amount: AMOUNT,
            released: 0,
        });
        let id =
            self.client
                .create_basket(&self.buyer, &self.seller, &self.arbiter, &assets, &TIMEOUT);
        self.client.deposit_basket(&id);
        id
    }

    fn at(&self, t: u64) {
        self.env.ledger().set_timestamp(t);
    }

    fn timeout_of(&self, id: u64) -> u64 {
        self.client.get_escrow(&id).timeout
    }

    fn expect_extended_event(&self, id: u64, old_timeout: u64, new_timeout: u64) {
        let expected = std::vec![events::DeadlineExtended {
            escrow_id: id,
            old_deadline: START + old_timeout,
            new_deadline: START + new_timeout,
        }
        .to_xdr(&self.env, &self.contract)];
        assert_eq!(
            self.env
                .events()
                .all()
                .filter_by_contract(&self.contract)
                .events(),
            expected.as_slice()
        );
    }

    /// Arm exactly one signer for `fn_name(id, new_timeout)`.
    fn sign_as(&self, signer: &Address, fn_name: &'static str, id: u64, new_timeout: u64) {
        self.env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &self.contract,
                fn_name,
                args: (id, new_timeout).into_val(&self.env),
                sub_invokes: &[],
            },
        }]);
    }
}

// ------------------------------------------------------------------ success

#[test]
fn buyer_extends_before_the_deadline() {
    let t = setup();
    let id = t.funded();
    t.at(START + TIMEOUT - 1);

    t.client.extend_deadline(&id, &EXTENDED);
    t.expect_extended_event(id, TIMEOUT, EXTENDED);

    let escrow = t.client.get_escrow(&id);
    assert_eq!(escrow.timeout, EXTENDED);
    assert_eq!(escrow.created_at, START);
    assert_eq!(escrow.status, EscrowStatus::Funded);
    assert_eq!(t.tc.balance(&t.contract), AMOUNT);
}

#[test]
fn buyer_extends_after_the_deadline_has_passed() {
    let t = setup();
    let id = t.funded();
    t.at(START + TIMEOUT + 1);

    t.client.extend_deadline(&id, &EXTENDED);
    t.expect_extended_event(id, TIMEOUT, EXTENDED);
    assert_eq!(t.timeout_of(id), EXTENDED);

    // The late delivery is re-opened: the seller may ask to be paid again.
    let release_at = t.client.request_release(&id, &AMOUNT);
    assert!(release_at > START + TIMEOUT);
}

#[test]
fn buyer_extends_a_funded_basket() {
    let t = setup();
    let id = t.funded_basket();
    t.at(START + TIMEOUT + 1);

    t.client.extend_basket_deadline(&id, &EXTENDED);
    t.expect_extended_event(id, TIMEOUT, EXTENDED);
    assert_eq!(t.client.get_basket(&id).timeout, EXTENDED);

    // The basket refund path now follows the new deadline: before it, the
    // buyer can no longer reclaim (only the seller may refund).
    t.env.mock_auths(&[MockAuth {
        address: &t.buyer,
        invoke: &MockAuthInvoke {
            contract: &t.contract,
            fn_name: "refund_basket",
            args: (id,).into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    assert!(matches!(
        t.client.try_refund_basket(&id),
        Err(Err(InvokeError::Abort))
    ));
}

#[test]
fn refund_expired_waits_for_the_extended_deadline() {
    let t = setup();
    let id = t.funded();
    t.client.extend_deadline(&id, &EXTENDED);

    t.at(START + TIMEOUT + 1);
    assert_eq!(
        t.client.try_refund_expired(&id),
        Err(Ok(ForgeError::DeadlineReached))
    );
    t.at(START + EXTENDED);
    assert_eq!(
        t.client.try_refund_expired(&id),
        Err(Ok(ForgeError::DeadlineReached))
    );
    assert_eq!(t.tc.balance(&t.buyer), AMOUNT * 9);

    t.at(START + EXTENDED + 1);
    t.client.refund_expired(&id);
    assert_eq!(t.client.get_escrow(&id).status, EscrowStatus::Refunded);
    assert_eq!(t.tc.balance(&t.buyer), AMOUNT * 10);
}

#[test]
fn pending_release_request_is_left_untouched() {
    let t = setup();
    let id = t.funded();
    let release_at = t.client.request_release(&id, &AMOUNT);
    let before = t.client.get_release_request(&id);

    t.client.extend_deadline(&id, &EXTENDED);

    assert_eq!(t.client.get_release_request(&id), before);
    assert_eq!(
        before,
        Some(ReleaseRequest {
            amount: AMOUNT,
            requested_at: START,
            release_at,
        })
    );
    t.at(release_at);
    t.client.execute_release(&id);
    assert_eq!(t.tc.balance(&t.seller), AMOUNT);
}

#[test]
fn extension_refreshes_ttl_and_keeps_participant_indexes() {
    let t = setup();
    let id = t.funded();
    let basket = t.funded_basket();
    let page_before = t.client.escrows_for_participant(&t.buyer, &0, &10);

    let sequence = t.env.ledger().sequence();
    advance_ledger_to(
        &t.env,
        sequence + BUMP_AMOUNT - BUMP_THRESHOLD + 1,
        START + 1,
    );
    let escrow_key = crate::DataKey::Escrow(id);
    let basket_key = crate::DataKey::Basket(basket);
    assert!(entry_ttl(&t.env, &t.contract, &escrow_key) < BUMP_THRESHOLD);
    assert!(entry_ttl(&t.env, &t.contract, &basket_key) < BUMP_THRESHOLD);

    t.client.extend_deadline(&id, &EXTENDED);
    t.client.extend_basket_deadline(&basket, &EXTENDED);

    assert_eq!(entry_ttl(&t.env, &t.contract, &escrow_key), BUMP_AMOUNT);
    assert_eq!(entry_ttl(&t.env, &t.contract, &basket_key), BUMP_AMOUNT);
    assert_eq!(t.client.ttl_info(&id), BUMP_AMOUNT);
    assert_eq!(
        t.client.escrows_for_participant(&t.buyer, &0, &10),
        page_before
    );
    for party in [&t.seller, &t.arbiter] {
        assert_eq!(
            t.client.escrows_for_participant(party, &0, &10).ids,
            page_before.ids
        );
    }
}

// ----------------------------------------------------------------- refusals

#[test]
fn earlier_or_equal_timeouts_are_refused() {
    let t = setup();
    let id = t.funded();
    for new_timeout in [TIMEOUT, TIMEOUT - 1, 0] {
        assert_eq!(
            t.client.try_extend_deadline(&id, &new_timeout),
            Err(Ok(ForgeError::InvalidInput))
        );
    }
    let basket = t.funded_basket();
    assert_eq!(
        t.client.try_extend_basket_deadline(&basket, &TIMEOUT),
        Err(Ok(ForgeError::InvalidInput))
    );
    assert_eq!(t.timeout_of(id), TIMEOUT);
    assert_eq!(t.client.get_basket(&basket).timeout, TIMEOUT);
}

#[test]
fn overflowing_deadline_is_refused() {
    let t = setup();
    let id = t.funded();
    assert_eq!(
        t.client.try_extend_deadline(&id, &u64::MAX),
        Err(Ok(ForgeError::ArithmeticOverflow))
    );
    assert_eq!(t.timeout_of(id), TIMEOUT);
}

#[test]
fn refused_unless_funded() {
    let t = setup();

    let pending = t.pending();

    let disputed = t.funded();
    t.client.dispute(&disputed, &t.buyer);

    let completed = t.funded();
    t.client.release(&completed);

    let refunded = t.funded();
    t.client.refund(&refunded);

    let cancelled = t.pending();
    t.client.cancel(&cancelled);

    for (id, status) in [
        (pending, EscrowStatus::Pending),
        (disputed, EscrowStatus::Disputed),
        (completed, EscrowStatus::Completed),
        (refunded, EscrowStatus::Refunded),
        (cancelled, EscrowStatus::Cancelled),
    ] {
        assert_eq!(t.client.get_escrow(&id).status, status);
        assert_eq!(
            t.client.try_extend_deadline(&id, &EXTENDED),
            Err(Ok(ForgeError::InvalidInput))
        );
        assert_eq!(t.timeout_of(id), TIMEOUT);
    }
}

#[test]
fn basket_refused_unless_funded() {
    let t = setup();
    let mut assets = Vec::new(&t.env);
    assets.push_back(EscrowAsset {
        token: t.token.clone(),
        amount: AMOUNT,
        released: 0,
    });
    let pending = t
        .client
        .create_basket(&t.buyer, &t.seller, &t.arbiter, &assets, &TIMEOUT);
    let disputed = t.funded_basket();
    t.client.dispute_basket(&disputed, &t.seller);

    for id in [pending, disputed] {
        assert_eq!(
            t.client.try_extend_basket_deadline(&id, &EXTENDED),
            Err(Ok(ForgeError::InvalidInput))
        );
        assert_eq!(t.client.get_basket(&id).timeout, TIMEOUT);
    }
}

#[test]
fn record_kinds_are_not_interchangeable() {
    let t = setup();
    let single = t.funded();
    let basket = t.funded_basket();
    assert_eq!(
        t.client.try_extend_basket_deadline(&single, &EXTENDED),
        Err(Ok(ForgeError::NotFound))
    );
    assert_eq!(
        t.client.try_extend_deadline(&basket, &EXTENDED),
        Err(Ok(ForgeError::NotFound))
    );
    assert_eq!(
        t.client.try_extend_deadline(&999, &EXTENDED),
        Err(Ok(ForgeError::NotFound))
    );
}

#[test]
fn only_the_buyer_can_extend() {
    let t = setup();
    let id = t.funded();
    let basket = t.funded_basket();
    let stranger = Address::generate(&t.env);

    for signer in [&t.seller, &t.arbiter, &stranger] {
        t.sign_as(signer, "extend_deadline", id, EXTENDED);
        assert!(matches!(
            t.client.try_extend_deadline(&id, &EXTENDED),
            Err(Err(InvokeError::Abort))
        ));
        t.sign_as(signer, "extend_basket_deadline", basket, EXTENDED);
        assert!(matches!(
            t.client.try_extend_basket_deadline(&basket, &EXTENDED),
            Err(Err(InvokeError::Abort))
        ));
    }
    t.env.mock_all_auths();
    assert_eq!(t.timeout_of(id), TIMEOUT);
    assert_eq!(t.client.get_basket(&basket).timeout, TIMEOUT);

    t.sign_as(&t.buyer, "extend_deadline", id, EXTENDED);
    t.client.extend_deadline(&id, &EXTENDED);
    t.sign_as(&t.buyer, "extend_basket_deadline", basket, EXTENDED);
    t.client.extend_basket_deadline(&basket, &EXTENDED);
    t.env.mock_all_auths();
    assert_eq!(t.timeout_of(id), EXTENDED);
    assert_eq!(t.client.get_basket(&basket).timeout, EXTENDED);
}
