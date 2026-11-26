//! Fixed-seed proptest state machine model for multi-asset basket escrow.
//!
//! Models basket lifecycle:
//! - Deposits, releases (full and partial per leg), disputes, split resolutions,
//!   refunds, review window requests, and time advancement.
//! - Checks component balances (pool conservation), terminal states, and
//!   participant index alignment after every action.
//! - Shrunk failures print readable basket action sequences.

extern crate std;

use crate::{
    Escrow, EscrowAsset, EscrowStatus, SorobanForgeEscrowClient, DEFAULT_REVIEW_PERIOD,
    MAX_PARTICIPANT_PAGE_SIZE,
};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec as SorobanVec};
use std::string::String;
use std::vec;
use std::vec::Vec;

const START_TIME: u64 = 1_000_000;
const TIMEOUT: u64 = 86_400; // 1 day

#[derive(Clone, Debug)]
pub enum BasketAction {
    Deposit,
    ReleaseFull,
    ReleasePartial { leg: usize, amount: i128 },
    RequestRelease,
    ApproveRelease,
    ExecuteRelease,
    Dispute,
    Resolve { in_favor_of_seller: bool },
    ResolveSplit,
    Refund,
    Cancel,
    AdvanceTime(u64),
}

fn basket_actions() -> impl Strategy<Value = Vec<BasketAction>> {
    let action = prop_oneof![
        3 => Just(BasketAction::Deposit),
        3 => Just(BasketAction::ReleaseFull),
        4 => (0..3usize, 1_i128..500).prop_map(|(leg, amount)| BasketAction::ReleasePartial { leg, amount }),
        2 => Just(BasketAction::RequestRelease),
        2 => Just(BasketAction::ApproveRelease),
        2 => Just(BasketAction::ExecuteRelease),
        2 => Just(BasketAction::Dispute),
        3 => prop::bool::ANY.prop_map(|b| BasketAction::Resolve { in_favor_of_seller: b }),
        3 => Just(BasketAction::ResolveSplit),
        2 => Just(BasketAction::Refund),
        1 => Just(BasketAction::Cancel),
        3 => (1_u64..100_000).prop_map(BasketAction::AdvanceTime),
    ];
    prop::collection::vec(action, 1..=40)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModelLifecycle {
    Pending,
    Funded,
    Disputed,
    Completed,
    Refunded,
    Cancelled,
}

#[derive(Clone, Debug)]
struct BasketModel {
    status: ModelLifecycle,
    amounts: Vec<i128>,
    released: Vec<i128>,
    pending_request_release_at: Option<u64>,
    now: u64,
    timeout: u64,
    created_at: u64,
}

impl BasketModel {
    fn new(amounts: Vec<i128>, now: u64, timeout: u64) -> Self {
        let legs = amounts.len();
        Self {
            status: ModelLifecycle::Pending,
            amounts,
            released: vec![0; legs],
            pending_request_release_at: None,
            now,
            timeout,
            created_at: now,
        }
    }

    fn remaining(&self, leg: usize) -> i128 {
        self.amounts[leg] - self.released[leg]
    }

    fn is_all_fully_released(&self) -> bool {
        self.amounts
            .iter()
            .zip(self.released.iter())
            .all(|(a, r)| a == r)
    }

    fn deadline(&self) -> u64 {
        self.created_at + self.timeout
    }
}

struct TestWorld {
    env: Env,
    client: SorobanForgeEscrowClient<'static>,
    id: u64,
    buyer: Address,
    seller: Address,
    arbiter: Address,
    tokens: Vec<Address>,
}

fn setup_world(env: &Env, initial_amounts: &[i128]) -> TestWorld {
    env.mock_all_auths();
    env.ledger().set_timestamp(START_TIME);

    let admin = Address::generate(env);
    let buyer = Address::generate(env);
    let seller = Address::generate(env);
    let arbiter = Address::generate(env);

    let escrow_contract = env.register(Escrow, ());
    let client = SorobanForgeEscrowClient::new(env, &escrow_contract);

    let mut tokens = Vec::new();
    let mut assets = SorobanVec::new(env);

    for &amt in initial_amounts {
        let token_addr = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        StellarAssetClient::new(env, &token_addr).mint(&buyer, &amt);
        tokens.push(token_addr.clone());
        assets.push_back(EscrowAsset {
            token: token_addr,
            amount: amt,
            released: 0,
        });
    }

    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);

    TestWorld {
        env: env.clone(),
        client,
        id,
        buyer,
        seller,
        arbiter,
        tokens,
    }
}

fn run_basket_trace(trace: &[BasketAction]) -> Result<(), TestCaseError> {
    let env = Env::default();
    let initial_amounts = [1_000_i128, 2_000_i128, 3_000_i128];
    let world = setup_world(&env, &initial_amounts);
    let mut model = BasketModel::new(initial_amounts.to_vec(), START_TIME, TIMEOUT);

    for (step_idx, action) in trace.iter().enumerate() {
        let context = |m: &BasketModel| -> String {
            std::format!("step {step_idx}: {action:?}\nmodel: {m:#?}")
        };

        match action {
            BasketAction::Deposit => {
                let valid = model.status == ModelLifecycle::Pending;
                let res = world.client.try_deposit_basket(&world.id);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    model.status = ModelLifecycle::Funded;
                }
            }
            BasketAction::ReleaseFull => {
                let valid = model.status == ModelLifecycle::Funded;
                let res = world.client.try_release_basket(&world.id);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    for i in 0..model.amounts.len() {
                        model.released[i] = model.amounts[i];
                    }
                    model.status = ModelLifecycle::Completed;
                    model.pending_request_release_at = None;
                }
            }
            BasketAction::ReleasePartial { leg, amount } => {
                let valid_leg = *leg < model.amounts.len();
                let valid = valid_leg
                    && model.status == ModelLifecycle::Funded
                    && *amount > 0
                    && *amount <= model.remaining(*leg);

                let res = if valid_leg {
                    world
                        .client
                        .try_release_partial_basket(&world.id, &world.tokens[*leg], amount)
                } else {
                    let dummy_token = Address::generate(&env);
                    world
                        .client
                        .try_release_partial_basket(&world.id, &dummy_token, amount)
                };

                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    model.released[*leg] += amount;
                    model.pending_request_release_at = None;
                    if model.is_all_fully_released() {
                        model.status = ModelLifecycle::Completed;
                    }
                }
            }
            BasketAction::RequestRelease => {
                let valid = model.status == ModelLifecycle::Funded
                    && model.pending_request_release_at.is_none()
                    && model.now < model.deadline();
                let res = world.client.try_request_release_basket(&world.id);
                prop_assert_eq!(matches!(res, Ok(Ok(_))), valid, "{}", context(&model));
                if valid {
                    model.pending_request_release_at = Some(model.now + DEFAULT_REVIEW_PERIOD);
                }
            }
            BasketAction::ApproveRelease => {
                let valid = model.status == ModelLifecycle::Funded
                    && model.pending_request_release_at.is_some();
                let res = world.client.try_approve_release(&world.id);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    for i in 0..model.amounts.len() {
                        model.released[i] = model.amounts[i];
                    }
                    model.status = ModelLifecycle::Completed;
                    model.pending_request_release_at = None;
                }
            }
            BasketAction::ExecuteRelease => {
                let valid = model.status == ModelLifecycle::Funded
                    && model
                        .pending_request_release_at
                        .is_some_and(|at| model.now >= at);
                let res = world.client.try_execute_release(&world.id);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    for i in 0..model.amounts.len() {
                        model.released[i] = model.amounts[i];
                    }
                    model.status = ModelLifecycle::Completed;
                    model.pending_request_release_at = None;
                }
            }
            BasketAction::Dispute => {
                let valid = model.status == ModelLifecycle::Funded;
                let res = world.client.try_dispute_basket(&world.id, &world.buyer);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    model.status = ModelLifecycle::Disputed;
                    model.pending_request_release_at = None;
                }
            }
            BasketAction::Resolve { in_favor_of_seller } => {
                let valid = model.status == ModelLifecycle::Disputed;
                let res = world
                    .client
                    .try_resolve_basket(&world.id, in_favor_of_seller);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    for i in 0..model.amounts.len() {
                        model.released[i] = model.amounts[i];
                    }
                    model.status = if *in_favor_of_seller {
                        ModelLifecycle::Completed
                    } else {
                        ModelLifecycle::Refunded
                    };
                }
            }
            BasketAction::ResolveSplit => {
                let res = world.client.try_resolve_split(&world.id, &5000);
                prop_assert!(res.is_err() || res.unwrap().is_err(), "{}", context(&model));
            }
            BasketAction::Refund => {
                let release_pending = model.pending_request_release_at.is_some();
                let is_after_deadline = model.now >= model.deadline();
                // refund_authorizer: seller authorizes before deadline, buyer after deadline.
                // If release is pending and authorizer is buyer (after deadline), returns ReleasePending error.
                let blocked_by_pending_release = is_after_deadline && release_pending;
                let valid = model.status == ModelLifecycle::Funded && !blocked_by_pending_release;
                let res = world.client.try_refund_basket(&world.id);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    model.status = ModelLifecycle::Refunded;
                    model.pending_request_release_at = None;
                }
            }
            BasketAction::Cancel => {
                let valid = model.status == ModelLifecycle::Pending;
                let res = world.client.try_cancel_basket(&world.id);
                prop_assert_eq!(matches!(res, Ok(Ok(()))), valid, "{}", context(&model));
                if valid {
                    model.status = ModelLifecycle::Cancelled;
                }
            }
            BasketAction::AdvanceTime(secs) => {
                model.now += secs;
                world.env.ledger().set_timestamp(model.now);
            }
        }

        // Verify state after every step:
        // 1. Status matches model
        let actual_status = world.client.get_status(&world.id);
        let expected_status = match model.status {
            ModelLifecycle::Pending => EscrowStatus::Pending,
            ModelLifecycle::Funded => EscrowStatus::Funded,
            ModelLifecycle::Disputed => EscrowStatus::Disputed,
            ModelLifecycle::Completed => EscrowStatus::Completed,
            ModelLifecycle::Refunded => EscrowStatus::Refunded,
            ModelLifecycle::Cancelled => EscrowStatus::Cancelled,
        };
        prop_assert_eq!(
            actual_status,
            expected_status.clone(),
            "{}",
            context(&model)
        );

        // 2. Component balances and pool conservation
        for (i, token_addr) in world.tokens.iter().enumerate() {
            let tc = TokenClient::new(&world.env, token_addr);
            let contract_bal = tc.balance(&world.client.address);
            let buyer_bal = tc.balance(&world.buyer);
            let seller_bal = tc.balance(&world.seller);
            let arbiter_bal = tc.balance(&world.arbiter);

            // Pool conservation
            prop_assert_eq!(
                contract_bal + buyer_bal + seller_bal + arbiter_bal,
                model.amounts[i],
                "{}",
                context(&model)
            );

            // Custody checks
            match model.status {
                ModelLifecycle::Pending | ModelLifecycle::Cancelled => {
                    prop_assert_eq!(contract_bal, 0, "{}", context(&model));
                }
                ModelLifecycle::Funded | ModelLifecycle::Disputed => {
                    let expected_custody = model.remaining(i);
                    prop_assert_eq!(contract_bal, expected_custody, "{}", context(&model));
                }
                ModelLifecycle::Completed | ModelLifecycle::Refunded => {
                    prop_assert_eq!(contract_bal, 0, "{}", context(&model));
                }
            }
        }

        // 3. Basket record inspection
        if model.status != ModelLifecycle::Cancelled {
            let basket = world.client.get_basket(&world.id);
            prop_assert_eq!(basket.status, expected_status, "{}", context(&model));
            for (i, asset) in basket.assets.iter().enumerate() {
                prop_assert_eq!(asset.amount, model.amounts[i], "{}", context(&model));
                if model.status == ModelLifecycle::Completed
                    || model.status == ModelLifecycle::Refunded
                {
                    prop_assert_eq!(asset.released, model.amounts[i], "{}", context(&model));
                } else {
                    prop_assert_eq!(asset.released, model.released[i], "{}", context(&model));
                }
            }
        }

        // 4. Participant index checks
        let buyer_page =
            world
                .client
                .escrows_for_participant(&world.buyer, &0, &MAX_PARTICIPANT_PAGE_SIZE);
        let seller_page =
            world
                .client
                .escrows_for_participant(&world.seller, &0, &MAX_PARTICIPANT_PAGE_SIZE);
        let arbiter_page =
            world
                .client
                .escrows_for_participant(&world.arbiter, &0, &MAX_PARTICIPANT_PAGE_SIZE);

        prop_assert_eq!(
            buyer_page.ids.contains(world.id),
            true,
            "Active/Terminal id must be in buyer index: {}",
            context(&model)
        );
        prop_assert_eq!(
            seller_page.ids.contains(world.id),
            true,
            "Active/Terminal id must be in seller index: {}",
            context(&model)
        );
        prop_assert_eq!(
            arbiter_page.ids.contains(world.id),
            true,
            "Active/Terminal id must be in arbiter index: {}",
            context(&model)
        );
    }

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x827_2026),
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    })]

    #[test]
    fn fixed_seed_basket_state_machine(trace in basket_actions()) {
        run_basket_trace(&trace)?;
    }
}
