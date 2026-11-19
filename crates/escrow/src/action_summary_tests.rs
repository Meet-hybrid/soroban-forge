use crate::{
    ActionIneligibilityReason as Reason, Escrow, EscrowAction as Action, EscrowActionEligibility,
    EscrowActionSummary, EscrowAsset, EscrowStatus, SorobanForgeEscrowClient,
    DEFAULT_REVIEW_PERIOD,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Vec};

const START: u64 = 10_000;
const TIMEOUT: u64 = 100;
const AMOUNT: i128 = 1_000;

struct TestContext<'a> {
    env: Env,
    client: SorobanForgeEscrowClient<'a>,
    token: Address,
    buyer: Address,
    seller: Address,
    arbiter: Address,
    outsider: Address,
}

fn setup() -> TestContext<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);
    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin).address();
    let contract = env.register(Escrow, ());
    let client = SorobanForgeEscrowClient::new(&env, &contract);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let arbiter = Address::generate(&env);
    let outsider = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&buyer, &(AMOUNT * 10));
    TestContext {
        env,
        client,
        token,
        buyer,
        seller,
        arbiter,
        outsider,
    }
}

impl TestContext<'_> {
    fn create(&self) -> u64 {
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
        let id = self.create();
        self.client.deposit(&id);
        id
    }

    fn summary(&self, id: u64, caller: &Address) -> EscrowActionSummary {
        self.client.get_action_summary(&id, caller)
    }
}

fn action(summary: &EscrowActionSummary, action: Action) -> EscrowActionEligibility {
    summary
        .actions
        .iter()
        .find(|entry| entry.action == action)
        .expect("summary includes expected action")
}

fn assert_action(
    summary: &EscrowActionSummary,
    action_id: Action,
    eligible: bool,
    reason: Option<Reason>,
) {
    let entry = action(summary, action_id);
    assert_eq!(entry.eligible, eligible, "{:?}", entry.action);
    assert_eq!(
        entry.reason,
        reason.unwrap_or(Reason::Eligible),
        "{:?}",
        entry.action
    );
}

#[test]
fn pending_summary_is_role_specific_and_cancelled_is_terminal() {
    let t = setup();
    let id = t.create();

    let buyer = t.summary(id, &t.buyer);
    assert_eq!(buyer.status, EscrowStatus::Pending);
    assert_eq!(buyer.deadline, START + TIMEOUT);
    assert_eq!(buyer.actions.len(), 12);
    assert_action(&buyer, Action::Deposit, true, None);
    assert_action(&buyer, Action::Cancel, true, None);
    assert_action(
        &buyer,
        Action::RequestRelease,
        false,
        Some(Reason::WrongStatus),
    );

    for outsider in [&t.seller, &t.arbiter, &t.outsider] {
        let view = t.summary(id, outsider);
        assert_action(&view, Action::Deposit, false, Some(Reason::WrongRole));
        assert_action(&view, Action::Cancel, false, Some(Reason::WrongRole));
    }

    t.client.cancel(&id);
    let terminal = t.summary(id, &t.buyer);
    assert_eq!(terminal.status, EscrowStatus::Cancelled);
    for entry in terminal.actions.iter() {
        assert!(!entry.eligible);
        assert_eq!(entry.reason, Reason::WrongStatus);
    }
}

#[test]
fn funded_summary_reflects_roles_deadline_request_and_review_window() {
    let t = setup();
    let id = t.funded();

    let buyer = t.summary(id, &t.buyer);
    assert_action(&buyer, Action::Release, true, None);
    assert_action(&buyer, Action::ReleasePartial, true, None);
    assert_action(&buyer, Action::Refund, false, Some(Reason::WrongRole));
    assert_action(
        &buyer,
        Action::ApproveRelease,
        false,
        Some(Reason::NoPendingRequest),
    );
    assert_action(
        &buyer,
        Action::ExecuteRelease,
        false,
        Some(Reason::NoPendingRequest),
    );

    let seller = t.summary(id, &t.seller);
    assert_action(&seller, Action::RequestRelease, true, None);
    assert_action(&seller, Action::Refund, true, None);
    assert_action(&seller, Action::Dispute, true, None);
    assert_action(&seller, Action::Release, false, Some(Reason::WrongRole));

    let arbiter = t.summary(id, &t.arbiter);
    assert_action(&arbiter, Action::Dispute, false, Some(Reason::WrongRole));
    assert_action(&arbiter, Action::Resolve, false, Some(Reason::WrongStatus));
    let outsider = t.summary(id, &t.outsider);
    assert_action(
        &outsider,
        Action::RefundExpired,
        false,
        Some(Reason::BeforeDeadline),
    );
    assert_action(
        &outsider,
        Action::ExecuteRelease,
        false,
        Some(Reason::NoPendingRequest),
    );
    assert_action(
        &outsider,
        Action::ApproveRelease,
        false,
        Some(Reason::NoPendingRequest),
    );

    let release_at = t.client.request_release(&id, &400);
    let pending = t.summary(id, &t.buyer);
    assert_eq!(pending.pending_request_amount, Some(400));
    assert!(pending.has_pending_request);
    assert_eq!(
        pending.pending_release_at,
        Some(START + DEFAULT_REVIEW_PERIOD)
    );
    assert_action(&pending, Action::ApproveRelease, true, None);
    assert_action(
        &pending,
        Action::ExecuteRelease,
        false,
        Some(Reason::ReviewWindowNotEnded),
    );
    assert_action(&pending, Action::Release, true, None);
    assert_action(&pending, Action::ReleasePartial, true, None);
    assert_action(&pending, Action::Dispute, true, None);
    assert_action(
        &t.summary(id, &t.seller),
        Action::RequestRelease,
        false,
        Some(Reason::ReleasePending),
    );

    t.env.ledger().set_timestamp(release_at);
    assert_action(
        &t.summary(id, &t.outsider),
        Action::ExecuteRelease,
        true,
        None,
    );

    t.env.ledger().set_timestamp(START + TIMEOUT + 1);
    let after_deadline = t.summary(id, &t.buyer);
    assert_action(
        &after_deadline,
        Action::Refund,
        false,
        Some(Reason::ReleasePending),
    );
    assert_action(
        &after_deadline,
        Action::RefundExpired,
        false,
        Some(Reason::ReleasePending),
    );

    let id_without_request = t.funded();
    let deadline = t.client.get_escrow(&id_without_request).created_at + TIMEOUT;
    t.env.ledger().set_timestamp(deadline);
    let at_deadline = t.summary(id_without_request, &t.buyer);
    assert_action(&at_deadline, Action::Refund, true, None);
    assert_action(
        &at_deadline,
        Action::RefundExpired,
        false,
        Some(Reason::BeforeDeadline),
    );
    t.env.ledger().set_timestamp(deadline + 1);
    let after_deadline = t.summary(id_without_request, &t.outsider);
    assert_action(&after_deadline, Action::RefundExpired, true, None);
    assert_action(
        &t.summary(id_without_request, &t.seller),
        Action::RequestRelease,
        false,
        Some(Reason::AfterDeadline),
    );
}

#[test]
fn disputed_and_terminal_summaries_expose_only_status_valid_actions() {
    let t = setup();
    let disputed_id = t.funded();
    t.client.dispute(&disputed_id, &t.buyer);
    let buyer = t.summary(disputed_id, &t.buyer);
    assert_eq!(buyer.status, EscrowStatus::Disputed);
    assert_action(&buyer, Action::Resolve, false, Some(Reason::WrongRole));
    let arbiter = t.summary(disputed_id, &t.arbiter);
    assert_action(&arbiter, Action::Resolve, true, None);
    assert_action(&arbiter, Action::ResolveSplit, true, None);
    t.client.resolve(&disputed_id, &false);
    for caller in [&t.buyer, &t.seller, &t.arbiter, &t.outsider] {
        let terminal = t.summary(disputed_id, caller);
        assert_eq!(terminal.status, EscrowStatus::Refunded);
        for entry in terminal.actions.iter() {
            assert!(!entry.eligible);
            assert_eq!(entry.reason, Reason::WrongStatus);
        }
    }

    let completed_id = t.funded();
    t.client.release(&completed_id);
    let completed = t.summary(completed_id, &t.buyer);
    assert_eq!(completed.status, EscrowStatus::Completed);
    for entry in completed.actions.iter() {
        assert!(!entry.eligible);
        assert_eq!(entry.reason, Reason::WrongStatus);
    }
}

#[test]
fn basket_summary_uses_basket_action_names_and_keeps_shared_review_actions() {
    let t = setup();
    let assets = Vec::from_array(
        &t.env,
        [EscrowAsset {
            token: t.token.clone(),
            amount: AMOUNT,
            released: 0,
        }],
    );
    let id = t
        .client
        .create_basket(&t.buyer, &t.seller, &t.arbiter, &assets, &TIMEOUT);
    let pending = t.client.get_basket_action_summary(&id, &t.buyer);
    assert_eq!(pending.actions.len(), 10);
    assert_action(&pending, Action::DepositBasket, true, None);
    assert_action(&pending, Action::CancelBasket, true, None);
    assert!(pending.actions.iter().all(|entry| {
        !matches!(
            entry.action,
            Action::Deposit | Action::Cancel | Action::ResolveSplit
        )
    }));

    t.client.deposit_basket(&id);
    t.client.request_release_basket(&id);
    let funded = t.summary(id, &t.seller);
    assert_eq!(funded.pending_request_amount, Some(0));
    assert!(funded.has_pending_request);
    assert_action(
        &funded,
        Action::RequestReleaseBasket,
        false,
        Some(Reason::ReleasePending),
    );
    assert_action(&funded, Action::DisputeBasket, true, None);
    assert_action(
        &funded,
        Action::ApproveRelease,
        false,
        Some(Reason::WrongRole),
    );

    t.client.dispute_basket(&id, &t.buyer);
    let arbiter = t.client.get_basket_action_summary(&id, &t.arbiter);
    assert_action(&arbiter, Action::ResolveBasket, true, None);
    assert!(!arbiter
        .actions
        .iter()
        .any(|entry| entry.action == Action::ResolveSplit));
}

#[test]
fn summary_queries_return_not_found_and_preserve_ttl() {
    let t = setup();
    assert_eq!(
        t.client.try_get_action_summary(&999, &t.buyer),
        Err(Ok(ForgeError::NotFound))
    );
    assert_eq!(
        t.client.try_get_basket_action_summary(&999, &t.buyer),
        Err(Ok(ForgeError::NotFound))
    );

    let id = t.funded();
    let ttl_before = t.client.ttl_info(&id);
    let request_release_at = t.client.request_release(&id, &500);
    t.env.ledger().set_timestamp(START + 1);
    let ttl_before_query = t.client.ttl_info(&id);
    let request_before = t.client.get_release_request(&id);
    let balance_before = StellarAssetClient::new(&t.env, &t.token).balance(&t.client.address);

    let _ = t.summary(id, &t.buyer);
    let _ = t.summary(id, &t.seller);

    assert_eq!(t.client.ttl_info(&id), ttl_before_query);
    assert_eq!(t.client.get_release_request(&id), request_before);
    assert_eq!(
        StellarAssetClient::new(&t.env, &t.token).balance(&t.client.address),
        balance_before
    );
    assert!(ttl_before >= ttl_before_query);
    assert_eq!(
        request_before.map(|request| request.release_at),
        Some(request_release_at)
    );
}
