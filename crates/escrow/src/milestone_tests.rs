//! Milestone-scheduled escrows: creation limits, in-order buyer approval,
//! wrong signers, the closed ad-hoc release paths, and disputes or expiry
//! partway through a schedule.
//!
//! Every test checks real SAC balances, so "paid" means tokens moved.

extern crate std;

use crate::{
    events, ActionIneligibilityReason, Escrow, EscrowAction, EscrowStatus, Milestone,
    MilestoneStatus, SorobanForgeEscrowClient, MAX_MILESTONES,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::events::Event as _;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Vec};

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 7 * 24 * 60 * 60;
const PLAN: [i128; 3] = [100, 250, 650];
const TOTAL: i128 = 1_000;

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
    StellarAssetClient::new(&env, &token).mint(&buyer, &TOTAL);
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
    fn amounts(&self, plan: &[i128]) -> Vec<i128> {
        let mut amounts = Vec::new(&self.env);
        for amount in plan {
            amounts.push_back(*amount);
        }
        amounts
    }

    fn try_create(&self, plan: &[i128]) -> Result<u64, ForgeError> {
        match self.client.try_create_milestone_escrow(
            &self.buyer,
            &self.seller,
            &self.arbiter,
            &self.token,
            &self.amounts(plan),
            &TIMEOUT,
        ) {
            Ok(Ok(id)) => Ok(id),
            Err(Ok(error)) => Err(error),
            other => panic!("unexpected host result: {other:?}"),
        }
    }

    /// A funded escrow over [`PLAN`].
    fn funded(&self) -> u64 {
        let id = self.try_create(&PLAN).unwrap();
        self.client.deposit(&id);
        id
    }

    fn approve(&self, id: u64, index: u32) -> Result<(), ForgeError> {
        match self.client.try_approve_milestone(&id, &index) {
            Ok(Ok(())) => Ok(()),
            Err(Ok(error)) => Err(error),
            other => panic!("unexpected host result: {other:?}"),
        }
    }

    fn statuses(&self, id: u64) -> std::vec::Vec<MilestoneStatus> {
        self.client
            .get_milestones(&id)
            .iter()
            .map(|milestone| milestone.status)
            .collect()
    }

    fn balances(&self) -> (i128, i128, i128, i128) {
        (
            self.tc.balance(&self.buyer),
            self.tc.balance(&self.seller),
            self.tc.balance(&self.arbiter),
            self.tc.balance(&self.contract),
        )
    }

    fn assert_conserved(&self) {
        let (buyer, seller, arbiter, held) = self.balances();
        assert_eq!(buyer + seller + arbiter + held, TOTAL);
    }
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

#[test]
fn create_records_ordered_schedule_and_total() {
    let t = setup();
    let id = t.try_create(&PLAN).unwrap();

    let escrow = t.client.get_escrow(&id);
    assert_eq!(escrow.amount, TOTAL);
    assert_eq!(escrow.status, EscrowStatus::Pending);

    let milestones = t.client.get_milestones(&id);
    assert_eq!(milestones.len(), 3);
    for (index, amount) in PLAN.iter().enumerate() {
        assert_eq!(
            milestones.get_unchecked(index as u32),
            Milestone {
                index: index as u32,
                amount: *amount,
                status: MilestoneStatus::Unpaid,
            }
        );
    }
}

#[test]
fn create_rejects_empty_zero_negative_and_zero_timeout() {
    let t = setup();
    assert_eq!(t.try_create(&[]), Err(ForgeError::InvalidInput));
    assert_eq!(t.try_create(&[100, 0, 5]), Err(ForgeError::InvalidInput));
    assert_eq!(t.try_create(&[100, -1]), Err(ForgeError::InvalidInput));
    assert_eq!(t.try_create(&[i128::MIN]), Err(ForgeError::InvalidInput));
    assert_eq!(
        t.client.try_create_milestone_escrow(
            &t.buyer,
            &t.seller,
            &t.arbiter,
            &t.token,
            &t.amounts(&PLAN),
            &0,
        ),
        Err(Ok(ForgeError::InvalidInput))
    );
}

#[test]
fn create_enforces_the_milestone_cap() {
    let t = setup();
    let at_cap = std::vec![1_i128; MAX_MILESTONES as usize];
    let over_cap = std::vec![1_i128; MAX_MILESTONES as usize + 1];
    let id = t.try_create(&at_cap).unwrap();
    assert_eq!(t.client.get_milestones(&id).len(), MAX_MILESTONES);
    assert_eq!(t.try_create(&over_cap), Err(ForgeError::InvalidInput));
}

#[test]
fn create_rejects_an_overflowing_total() {
    let t = setup();
    assert_eq!(
        t.try_create(&[i128::MAX, 1]),
        Err(ForgeError::ArithmeticOverflow)
    );
    assert_eq!(
        t.try_create(&[i128::MAX / 2 + 1, i128::MAX / 2 + 1]),
        Err(ForgeError::ArithmeticOverflow)
    );
    // The largest representable total is still accepted.
    t.try_create(&[i128::MAX - 1, 1]).unwrap();
}

#[test]
fn create_requires_the_buyer_signature() {
    let t = setup();
    t.env.mock_auths(&[MockAuth {
        address: &t.seller,
        invoke: &MockAuthInvoke {
            contract: &t.contract,
            fn_name: "create_milestone_escrow",
            args: (
                t.buyer.clone(),
                t.seller.clone(),
                t.arbiter.clone(),
                t.token.clone(),
                t.amounts(&PLAN),
                TIMEOUT,
            )
                .into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    assert!(matches!(
        t.client.try_create_milestone_escrow(
            &t.buyer,
            &t.seller,
            &t.arbiter,
            &t.token,
            &t.amounts(&PLAN),
            &TIMEOUT,
        ),
        Err(Err(InvokeError::Abort))
    ));
}

// ---------------------------------------------------------------------------
// In-order approval
// ---------------------------------------------------------------------------

#[test]
fn approving_every_milestone_in_order_pays_exactly_each_amount() {
    let t = setup();
    let id = t.funded();
    assert_eq!(t.balances(), (0, 0, 0, TOTAL));

    let mut paid = 0;
    for (index, amount) in PLAN.iter().enumerate() {
        t.approve(id, index as u32).unwrap();
        paid += amount;
        assert_eq!(t.tc.balance(&t.seller), paid);
        assert_eq!(t.tc.balance(&t.contract), TOTAL - paid);
        assert_eq!(t.client.get_escrow(&id).released, paid);
        t.assert_conserved();
    }

    assert_eq!(t.client.get_escrow(&id).status, EscrowStatus::Completed);
    assert_eq!(t.statuses(id), std::vec![MilestoneStatus::Paid; 3]);
    assert_eq!(t.approve(id, 3), Err(ForgeError::InvalidInput));
}

#[test]
fn approval_emits_milestone_released_and_final_released() {
    let t = setup();
    let id = t.funded();
    t.approve(id, 0).unwrap();
    assert_eq!(
        t.env
            .events()
            .all()
            .filter_by_contract(&t.contract)
            .events(),
        std::vec![events::MilestoneReleased {
            escrow_id: id,
            index: 0,
            amount: PLAN[0],
        }
        .to_xdr(&t.env, &t.contract)]
        .as_slice()
    );

    t.approve(id, 1).unwrap();
    t.approve(id, 2).unwrap();
    // `events().all()` only reports the most recent invocation, so read the
    // final approval's events before the `get_escrow` view call below.
    let final_events = t
        .env
        .events()
        .all()
        .filter_by_contract(&t.contract)
        .events()
        .to_vec();
    let completed = t.client.get_escrow(&id);
    assert_eq!(
        final_events.as_slice(),
        std::vec![
            events::MilestoneReleased {
                escrow_id: id,
                index: 2,
                amount: PLAN[2],
            }
            .to_xdr(&t.env, &t.contract),
            events::Released {
                escrow_id: id,
                data: completed,
            }
            .to_xdr(&t.env, &t.contract),
        ]
        .as_slice()
    );
}

#[test]
fn out_of_order_approval_is_rejected_without_moving_funds() {
    let t = setup();
    let id = t.funded();

    assert_eq!(t.approve(id, 1), Err(ForgeError::InvalidInput));
    assert_eq!(t.approve(id, 2), Err(ForgeError::InvalidInput));
    assert_eq!(t.approve(id, 99), Err(ForgeError::InvalidInput));
    assert_eq!(t.balances(), (0, 0, 0, TOTAL));

    t.approve(id, 0).unwrap();
    // Paying a milestone twice is just another out-of-order approval.
    assert_eq!(t.approve(id, 0), Err(ForgeError::InvalidInput));
    assert_eq!(t.approve(id, 2), Err(ForgeError::InvalidInput));
    assert_eq!(t.tc.balance(&t.seller), PLAN[0]);
    assert_eq!(
        t.statuses(id),
        std::vec![
            MilestoneStatus::Paid,
            MilestoneStatus::Unpaid,
            MilestoneStatus::Unpaid
        ]
    );
}

#[test]
fn approval_before_funding_is_rejected() {
    let t = setup();
    let id = t.try_create(&PLAN).unwrap();
    assert_eq!(t.approve(id, 0), Err(ForgeError::InvalidInput));
}

#[test]
fn wrong_signer_cannot_approve() {
    for signer in ["seller", "arbiter"] {
        let t = setup();
        let id = t.funded();
        let address = if signer == "seller" {
            &t.seller
        } else {
            &t.arbiter
        };
        t.env.mock_auths(&[MockAuth {
            address,
            invoke: &MockAuthInvoke {
                contract: &t.contract,
                fn_name: "approve_milestone",
                args: (id, 0_u32).into_val(&t.env),
                sub_invokes: &[],
            },
        }]);
        assert!(
            matches!(
                t.client.try_approve_milestone(&id, &0),
                Err(Err(InvokeError::Abort))
            ),
            "{signer} must not be able to approve"
        );
        assert_eq!(t.balances(), (0, 0, 0, TOTAL));
    }
}

#[test]
fn buyer_signature_alone_approves() {
    let t = setup();
    let id = t.funded();
    t.env.mock_auths(&[MockAuth {
        address: &t.buyer,
        invoke: &MockAuthInvoke {
            contract: &t.contract,
            fn_name: "approve_milestone",
            args: (id, 0_u32).into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    t.client.approve_milestone(&id, &0);
    assert_eq!(t.tc.balance(&t.seller), PLAN[0]);
}

// ---------------------------------------------------------------------------
// Ad-hoc release paths are closed
// ---------------------------------------------------------------------------

#[test]
fn ad_hoc_release_paths_are_rejected_for_milestone_escrows() {
    let t = setup();
    let id = t.funded();
    assert_eq!(t.client.try_release(&id), Err(Ok(ForgeError::InvalidInput)));
    assert_eq!(
        t.client.try_release_partial(&id, &PLAN[0]),
        Err(Ok(ForgeError::InvalidInput))
    );
    assert_eq!(
        t.client.try_request_release(&id, &PLAN[0]),
        Err(Ok(ForgeError::InvalidInput))
    );
    assert_eq!(t.balances(), (0, 0, 0, TOTAL));
}

#[test]
fn action_summary_reports_the_milestone_schedule_reason() {
    let t = setup();
    let id = t.funded();
    for (caller, action) in [
        (&t.seller, EscrowAction::RequestRelease),
        (&t.buyer, EscrowAction::Release),
        (&t.buyer, EscrowAction::ReleasePartial),
    ] {
        let summary = t.client.get_action_summary(&id, caller);
        let entry = summary
            .actions
            .iter()
            .find(|entry| entry.action == action)
            .unwrap();
        assert!(!entry.eligible);
        assert_eq!(entry.reason, ActionIneligibilityReason::MilestoneSchedule);
    }
}

#[test]
fn milestone_views_reject_other_escrow_kinds() {
    let t = setup();
    let plain = t
        .client
        .create_escrow(&t.buyer, &t.seller, &t.arbiter, &t.token, &TOTAL, &TIMEOUT);
    assert_eq!(
        t.client.try_get_milestones(&plain),
        Err(Ok(ForgeError::NotFound))
    );
    assert_eq!(
        t.client.try_approve_milestone(&plain, &0),
        Err(Ok(ForgeError::NotFound))
    );
    assert_eq!(
        t.client.try_get_milestones(&404),
        Err(Ok(ForgeError::NotFound))
    );
}

// ---------------------------------------------------------------------------
// Disputes, resolution, expiry and cancellation
// ---------------------------------------------------------------------------

#[test]
fn dispute_partway_freezes_the_remaining_milestones() {
    let t = setup();
    let id = t.funded();
    t.approve(id, 0).unwrap();
    t.client.dispute(&id, &t.seller);

    assert_eq!(t.approve(id, 1), Err(ForgeError::InvalidInput));
    assert_eq!(
        t.statuses(id),
        std::vec![
            MilestoneStatus::Paid,
            MilestoneStatus::Frozen,
            MilestoneStatus::Frozen
        ]
    );
    assert_eq!(t.tc.balance(&t.contract), TOTAL - PLAN[0]);
}

#[test]
fn arbiter_refund_after_dispute_returns_only_the_unpaid_remainder() {
    let t = setup();
    let id = t.funded();
    t.approve(id, 0).unwrap();
    t.approve(id, 1).unwrap();
    t.client.dispute(&id, &t.buyer);
    t.client.resolve(&id, &false);

    assert_eq!(t.balances(), (PLAN[2], PLAN[0] + PLAN[1], 0, 0));
    assert_eq!(t.client.get_escrow(&id).status, EscrowStatus::Refunded);
    assert_eq!(
        t.statuses(id),
        std::vec![
            MilestoneStatus::Paid,
            MilestoneStatus::Paid,
            MilestoneStatus::Settled
        ]
    );
    assert_eq!(t.approve(id, 2), Err(ForgeError::InvalidInput));
}

#[test]
fn arbiter_award_to_seller_pays_only_the_unpaid_remainder() {
    let t = setup();
    let id = t.funded();
    t.approve(id, 0).unwrap();
    t.client.dispute(&id, &t.seller);
    t.client.resolve(&id, &true);
    assert_eq!(t.balances(), (0, TOTAL, 0, 0));
    assert_eq!(t.client.get_escrow(&id).status, EscrowStatus::Completed);
}

#[test]
fn split_resolution_after_partial_schedule_conserves_the_remainder() {
    let t = setup();
    let id = t.funded();
    t.approve(id, 0).unwrap();
    t.client.dispute(&id, &t.buyer);
    t.client.resolve_split(&id, &5_000);
    let remainder = TOTAL - PLAN[0];
    assert_eq!(t.balances(), (remainder / 2, PLAN[0] + remainder / 2, 0, 0));
    t.assert_conserved();
}

#[test]
fn refund_expired_returns_only_the_unpaid_remainder() {
    let t = setup();
    let id = t.funded();
    t.approve(id, 0).unwrap();

    assert_eq!(
        t.client.try_refund_expired(&id),
        Err(Ok(ForgeError::DeadlineReached))
    );
    t.env.ledger().set_timestamp(START + TIMEOUT + 1);
    t.client.refund_expired(&id);

    assert_eq!(t.balances(), (TOTAL - PLAN[0], PLAN[0], 0, 0));
    assert_eq!(
        t.statuses(id),
        std::vec![
            MilestoneStatus::Paid,
            MilestoneStatus::Settled,
            MilestoneStatus::Settled
        ]
    );
}

#[test]
fn milestones_can_still_be_approved_after_the_deadline_until_refunded() {
    let t = setup();
    let id = t.funded();
    t.env.ledger().set_timestamp(START + TIMEOUT + 1);
    t.approve(id, 0).unwrap();
    t.client.refund_expired(&id);
    assert_eq!(t.balances(), (TOTAL - PLAN[0], PLAN[0], 0, 0));
}

#[test]
fn cancelled_schedule_reports_cancelled_milestones() {
    let t = setup();
    let id = t.try_create(&PLAN).unwrap();
    t.client.cancel(&id);
    assert_eq!(t.statuses(id), std::vec![MilestoneStatus::Cancelled; 3]);
    assert_eq!(t.approve(id, 0), Err(ForgeError::InvalidInput));
}

#[test]
fn touch_ttl_keeps_the_schedule_alive() {
    let t = setup();
    let id = t.funded();
    t.client.touch_ttl(&id);
    assert_eq!(t.client.get_milestones(&id).len(), 3);
}
