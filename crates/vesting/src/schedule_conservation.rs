//! Model-based schedule conservation suite for the vesting crate.
//!
//! Validates linear and tranche vesting schedules across arbitrary sequences of:
//! - `claim`, `claim_for`, `reassign_beneficiary`, `revoke`, `refund_revoked`, and time jumps.
//!
//! Invariants asserted after every action:
//! - State-machine status, claimable amount, and refunded amount match the independent model.
//! - Conservation holds exactly: `claimed + refunded + remaining_claimable == schedule.total_amount`.
//! - Token pool conservation: `funder_balance + beneficiary_balances + contract_balance == initial_mint`.
//! - At least 128 cases of up to 40 steps, with deterministic fixed seed and readable shrunk failures.

extern crate std;

use crate::{
    SorobanForgeVestingClient, Tranche, TrancheSchedule, Vesting, VestingSchedule, VestingStatus,
};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec as SorobanVec};
use std::collections::HashMap;
use std::string::String;
use std::vec;
use std::vec::Vec;

const START_TIME: u64 = 1_000_000;
const INITIAL_MINT: i128 = 10_000_000;

#[derive(Clone, Debug)]
pub enum VestingAction {
    Claim,
    ClaimFor(usize),
    ReassignBeneficiary(usize),
    Revoke,
    RefundRevoked,
    AdvanceTime(u64),
}

fn vesting_actions() -> impl Strategy<Value = (bool, Vec<VestingAction>)> {
    let action = prop_oneof![
        4 => Just(VestingAction::Claim),
        4 => (0..4usize).prop_map(VestingAction::ClaimFor),
        3 => (0..4usize).prop_map(VestingAction::ReassignBeneficiary),
        2 => Just(VestingAction::Revoke),
        3 => Just(VestingAction::RefundRevoked),
        4 => (1_u64..20_000).prop_map(VestingAction::AdvanceTime),
    ];
    (prop::bool::ANY, prop::collection::vec(action, 1..=40))
}

#[derive(Clone, Debug)]
struct LinearScheduleModel {
    total_amount: i128,
    cliff: u64,
    duration: u64,
    start: u64,
    claimed: i128,
    refunded: i128,
    status: VestingStatus,
    revoked_vested: Option<i128>,
    current_beneficiary: usize,
    beneficiaries: Vec<Address>,
    tenure_start_vested: i128,
    accrued: HashMap<usize, i128>,
    paid: HashMap<usize, i128>,
    now: u64,
}

impl LinearScheduleModel {
    fn new(
        total_amount: i128,
        cliff: u64,
        duration: u64,
        start: u64,
        initial_beneficiaries: Vec<Address>,
    ) -> Self {
        let mut model = Self {
            total_amount,
            cliff,
            duration,
            start,
            claimed: 0,
            refunded: 0,
            status: VestingStatus::Locked,
            revoked_vested: None,
            current_beneficiary: 0,
            beneficiaries: initial_beneficiaries,
            tenure_start_vested: 0,
            accrued: HashMap::new(),
            paid: HashMap::new(),
            now: start,
        };
        for i in 0..model.beneficiaries.len() {
            model.accrued.insert(i, 0);
            model.paid.insert(i, 0);
        }
        model.update_status();
        model
    }

    fn vested_at(&self, timestamp: u64) -> i128 {
        if let Some(frozen) = self.revoked_vested {
            return frozen;
        }
        let cliff_time = self.start + self.cliff;
        if timestamp < cliff_time {
            return 0;
        }
        let end_time = self.start + self.duration;
        if timestamp >= end_time {
            return self.total_amount;
        }
        let period = end_time - cliff_time;
        if period == 0 {
            return self.total_amount;
        }
        let elapsed = timestamp - cliff_time;
        soroban_forge_shared_utils::math::mul_div_floor(
            self.total_amount,
            i128::from(elapsed),
            i128::from(period),
        )
        .unwrap()
    }

    fn update_status(&mut self) {
        if self.revoked_vested.is_some() {
            self.status = VestingStatus::Revoked;
        } else if self.claimed >= self.total_amount {
            self.status = VestingStatus::Completed;
        } else if self.now < self.start + self.cliff {
            self.status = VestingStatus::Locked;
        } else {
            self.status = VestingStatus::Vesting;
        }
    }

    fn claimable_for(&self, idx: usize) -> i128 {
        let accrued_frozen = *self.accrued.get(&idx).unwrap_or(&0);
        let paid = *self.paid.get(&idx).unwrap_or(&0);
        let active_accrual = if idx == self.current_beneficiary {
            let current_vested = self.vested_at(self.now);
            (current_vested - self.tenure_start_vested).max(0)
        } else {
            0
        };
        (accrued_frozen + active_accrual - paid).max(0)
    }

    fn total_claimable(&self) -> i128 {
        let mut sum = 0i128;
        for i in 0..self.beneficiaries.len() {
            sum += self.claimable_for(i);
        }
        sum
    }

    fn execute_claim_for(&mut self, idx: usize) -> i128 {
        let amount = self.claimable_for(idx);
        if amount > 0 {
            *self.paid.get_mut(&idx).unwrap() += amount;
            self.claimed += amount;
            self.update_status();
        }
        amount
    }
}

#[derive(Clone, Debug)]
struct TrancheScheduleModel {
    total_amount: i128,
    tranches: Vec<(u64, i128)>, // (unlock_at, amount)
    start: u64,
    claimed: i128,
    status: VestingStatus,
    beneficiary: Address,
    now: u64,
}

impl TrancheScheduleModel {
    fn new(tranches: Vec<(u64, i128)>, start: u64, beneficiary: Address) -> Self {
        let total_amount = tranches.iter().map(|(_, a)| *a).sum();
        let mut model = Self {
            total_amount,
            tranches,
            start,
            claimed: 0,
            status: VestingStatus::Locked,
            beneficiary,
            now: start,
        };
        model.update_status();
        model
    }

    fn unlocked_at(&self, timestamp: u64) -> i128 {
        let elapsed = timestamp.saturating_sub(self.start);
        self.tranches
            .iter()
            .filter(|(unlock_at, _)| elapsed >= *unlock_at)
            .map(|(_, amt)| *amt)
            .sum()
    }

    fn update_status(&mut self) {
        if self.claimed >= self.total_amount {
            self.status = VestingStatus::Completed;
        } else {
            let first_unlock = self.tranches.first().map(|(u, _)| *u).unwrap_or(0);
            let elapsed = self.now.saturating_sub(self.start);
            if elapsed < first_unlock {
                self.status = VestingStatus::Locked;
            } else {
                self.status = VestingStatus::Vesting;
            }
        }
    }

    fn claimable(&self) -> i128 {
        (self.unlocked_at(self.now) - self.claimed).max(0)
    }

    fn execute_claim(&mut self) -> i128 {
        let amount = self.claimable();
        if amount > 0 {
            self.claimed += amount;
            self.update_status();
        }
        amount
    }
}

struct TestFixture {
    env: Env,
    client: SorobanForgeVestingClient<'static>,
    token: Address,
    funder: Address,
    beneficiaries: Vec<Address>,
    id: u64,
}

fn run_conservation_trace(is_linear: bool, trace: &[VestingAction]) -> Result<(), TestCaseError> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START_TIME);

    let admin = Address::generate(&env);
    let funder = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin).address();
    StellarAssetClient::new(&env, &token).mint(&funder, &INITIAL_MINT);

    let contract = env.register(Vesting, ());
    let client = SorobanForgeVestingClient::new(&env, &contract);

    let beneficiaries = vec![
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];

    if is_linear {
        let total_amount = 100_000_i128;
        let cliff = 1_000_u64;
        let duration = 10_000_u64;

        StellarAssetClient::new(&env, &token).mint(&contract, &total_amount);

        let id = client.create_schedule(
            &funder,
            &beneficiaries[0],
            &token,
            &total_amount,
            &cliff,
            &duration,
        );

        let fixture = TestFixture {
            env: env.clone(),
            client,
            token,
            funder,
            beneficiaries: beneficiaries.clone(),
            id,
        };

        let mut model =
            LinearScheduleModel::new(total_amount, cliff, duration, START_TIME, beneficiaries);

        for (step_idx, action) in trace.iter().enumerate() {
            let context = |m: &LinearScheduleModel| -> String {
                std::format!("linear step {step_idx}: {action:?}\nmodel: {m:#?}")
            };

            match action {
                VestingAction::Claim => {
                    let cur_idx = model.current_beneficiary;
                    let expected_amt = model.claimable_for(cur_idx);
                    let res = fixture.client.try_claim(&fixture.id);
                    prop_assert_eq!(matches!(res, Ok(Ok(_))), true, "{}", context(&model));
                    let amt = res.unwrap().unwrap();
                    prop_assert_eq!(amt, expected_amt, "{}", context(&model));
                    model.execute_claim_for(cur_idx);
                }
                VestingAction::ClaimFor(idx) => {
                    let target_idx = *idx % model.beneficiaries.len();
                    let expected_amt = model.claimable_for(target_idx);
                    let res = fixture
                        .client
                        .try_claim_for(&fixture.id, &model.beneficiaries[target_idx]);
                    prop_assert_eq!(matches!(res, Ok(Ok(_))), true, "{}", context(&model));
                    let amt = res.unwrap().unwrap();
                    prop_assert_eq!(amt, expected_amt, "{}", context(&model));
                    model.execute_claim_for(target_idx);
                }
                VestingAction::ReassignBeneficiary(new_idx) => {
                    let next_idx = *new_idx % model.beneficiaries.len();
                    let next_addr = model.beneficiaries[next_idx].clone();
                    let can_reassign = model.status != VestingStatus::Revoked
                        && model.claimed < model.total_amount
                        && next_idx != model.current_beneficiary
                        && next_addr != fixture.funder;

                    let res = fixture
                        .client
                        .try_reassign_beneficiary(&fixture.id, &next_addr);
                    prop_assert_eq!(
                        matches!(res, Ok(Ok(()))),
                        can_reassign,
                        "{}",
                        context(&model)
                    );
                    if can_reassign {
                        let now_vested = model.vested_at(model.now);
                        let old_accrual = (now_vested - model.tenure_start_vested).max(0);
                        *model.accrued.get_mut(&model.current_beneficiary).unwrap() += old_accrual;
                        model.tenure_start_vested = now_vested;
                        model.current_beneficiary = next_idx;
                        model.update_status();
                    }
                }
                VestingAction::Revoke => {
                    let can_revoke = model.status == VestingStatus::Locked
                        || model.status == VestingStatus::Vesting;
                    let res = fixture.client.try_revoke(&fixture.id);
                    prop_assert_eq!(matches!(res, Ok(Ok(()))), can_revoke, "{}", context(&model));
                    if can_revoke {
                        model.revoked_vested = Some(model.vested_at(model.now));
                        model.update_status();
                    }
                }
                VestingAction::RefundRevoked => {
                    let can_refund = model.status == VestingStatus::Revoked;
                    let res = fixture.client.try_refund_revoked(&fixture.id);
                    prop_assert_eq!(matches!(res, Ok(Ok(_))), can_refund, "{}", context(&model));
                    if can_refund {
                        let amt = res.unwrap().unwrap();
                        let unvested =
                            (model.total_amount - model.revoked_vested.unwrap() - model.refunded)
                                .max(0);
                        prop_assert_eq!(amt, unvested, "{}", context(&model));
                        model.refunded += amt;
                    }
                }
                VestingAction::AdvanceTime(secs) => {
                    model.now += secs;
                    fixture.env.ledger().set_timestamp(model.now);
                    model.update_status();
                }
            }

            // Invariant assertions after every step:
            // 1. Status matches model
            let actual_status = fixture.client.get_status(&fixture.id);
            prop_assert_eq!(actual_status, model.status.clone(), "{}", context(&model));

            let actual_sched: VestingSchedule = fixture.client.get_schedule(&fixture.id);

            // 2. Claimable values match model
            let actual_claimable = fixture.client.claimable(&fixture.id);
            let expected_claimable = model.claimable_for(model.current_beneficiary);
            prop_assert_eq!(actual_claimable, expected_claimable, "{}", context(&model));

            for (i, b) in model.beneficiaries.iter().enumerate() {
                let actual_for = fixture.client.claimable_for(&fixture.id, b);
                prop_assert_eq!(actual_for, model.claimable_for(i), "{}", context(&model));
            }

            // 3. Exact schedule conservation: claimed + refunded + remaining_claimable == schedule.total_amount
            let total_claimable = model.total_claimable();
            if model.status == VestingStatus::Revoked {
                // When revoked, unvested amount is refundable: claimed + refunded + total_claimable + (unvested - refunded) == total
                prop_assert_eq!(
                    actual_sched.claimed + model.refunded + total_claimable,
                    model.revoked_vested.unwrap() + model.refunded,
                    "{}",
                    context(&model)
                );
            } else {
                let unvested_remaining = (model.total_amount - model.vested_at(model.now)).max(0);
                prop_assert_eq!(
                    actual_sched.claimed + model.refunded + total_claimable + unvested_remaining,
                    model.total_amount,
                    "Exact conservation violated: {}",
                    context(&model)
                );
            }

            // 4. Token pool conservation
            let tc = TokenClient::new(&fixture.env, &fixture.token);
            let contract_bal = tc.balance(&fixture.client.address);
            let funder_bal = tc.balance(&fixture.funder);
            let benef_bal: i128 = model.beneficiaries.iter().map(|b| tc.balance(b)).sum();
            prop_assert_eq!(
                contract_bal + funder_bal + benef_bal,
                INITIAL_MINT + total_amount,
                "Token pool conservation violated: {}",
                context(&model)
            );
        }
    } else {
        // Tranche schedule
        let tranches_data = vec![
            (0u64, 20_000i128),
            (2_000u64, 30_000i128),
            (5_000u64, 50_000i128),
        ];
        let mut tranches_soroban = SorobanVec::new(&env);
        for &(unlock_at, amount) in &tranches_data {
            tranches_soroban.push_back(Tranche { unlock_at, amount });
        }

        let total_amount: i128 = tranches_data.iter().map(|(_, a)| *a).sum();
        StellarAssetClient::new(&env, &token).mint(&contract, &total_amount);

        let fixture = TestFixture {
            env: env.clone(),
            client,
            token,
            funder,
            beneficiaries: beneficiaries.clone(),
            id: 0,
        };

        let id = fixture.client.create_tranche_schedule(
            &beneficiaries[0],
            &fixture.token,
            &tranches_soroban,
        );
        let fixture = TestFixture { id, ..fixture };

        let mut model =
            TrancheScheduleModel::new(tranches_data, START_TIME, beneficiaries[0].clone());

        for (step_idx, action) in trace.iter().enumerate() {
            let context = |m: &TrancheScheduleModel| -> String {
                std::format!("tranche step {step_idx}: {action:?}\nmodel: {m:#?}")
            };

            match action {
                VestingAction::Claim => {
                    let expected_amt = model.claimable();
                    let res = fixture.client.try_claim(&fixture.id);
                    prop_assert_eq!(matches!(res, Ok(Ok(_))), true, "{}", context(&model));
                    let amt = res.unwrap().unwrap();
                    prop_assert_eq!(amt, expected_amt, "{}", context(&model));
                    model.execute_claim();
                }
                VestingAction::ClaimFor(idx) => {
                    let is_primary = (*idx % fixture.beneficiaries.len()) == 0;
                    let res = fixture.client.try_claim_for(
                        &fixture.id,
                        &fixture.beneficiaries[*idx % fixture.beneficiaries.len()],
                    );
                    if is_primary {
                        prop_assert_eq!(matches!(res, Ok(Ok(_))), true, "{}", context(&model));
                        let amt = res.unwrap().unwrap();
                        prop_assert_eq!(amt, model.claimable(), "{}", context(&model));
                        model.execute_claim();
                    } else {
                        // Former/other beneficiary has no balance on an unreassigned tranche schedule
                        prop_assert_eq!(matches!(res, Ok(Ok(0))), true, "{}", context(&model));
                    }
                }
                VestingAction::ReassignBeneficiary(_) => {
                    // Tranche schedules cannot be reassigned (entrypoint rejects non-linear)
                    let dummy_addr = Address::generate(&fixture.env);
                    let res = fixture
                        .client
                        .try_reassign_beneficiary(&fixture.id, &dummy_addr);
                    prop_assert!(res.is_err() || res.unwrap().is_err(), "{}", context(&model));
                }
                VestingAction::Revoke => {
                    // Tranche schedules cannot be revoked
                    let res = fixture.client.try_revoke(&fixture.id);
                    prop_assert!(res.is_err() || res.unwrap().is_err(), "{}", context(&model));
                }
                VestingAction::RefundRevoked => {
                    // Tranche schedules cannot be refunded via refund_revoked
                    let res = fixture.client.try_refund_revoked(&fixture.id);
                    prop_assert!(res.is_err() || res.unwrap().is_err(), "{}", context(&model));
                }
                VestingAction::AdvanceTime(secs) => {
                    model.now += secs;
                    fixture.env.ledger().set_timestamp(model.now);
                    model.update_status();
                }
            }

            // Invariant assertions:
            let actual_status = fixture.client.get_status(&fixture.id);
            prop_assert_eq!(actual_status, model.status.clone(), "{}", context(&model));

            let actual_sched: TrancheSchedule = fixture.client.get_tranche_schedule(&fixture.id);

            let actual_claimable = fixture.client.claimable(&fixture.id);
            prop_assert_eq!(actual_claimable, model.claimable(), "{}", context(&model));

            // Conservation: claimed + remaining_claimable + locked == total_amount
            let locked = (model.total_amount - model.unlocked_at(model.now)).max(0);
            prop_assert_eq!(
                actual_sched.claimed + actual_claimable + locked,
                model.total_amount,
                "Tranche schedule conservation violated: {}",
                context(&model)
            );

            // Token pool conservation
            let tc = TokenClient::new(&fixture.env, &fixture.token);
            let contract_bal = tc.balance(&fixture.client.address);
            let funder_bal = tc.balance(&fixture.funder);
            let benef_bal = tc.balance(&model.beneficiary);
            prop_assert_eq!(
                contract_bal + funder_bal + benef_bal,
                INITIAL_MINT + total_amount,
                "Token pool conservation violated: {}",
                context(&model)
            );
        }
    }

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x932_2026),
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    })]

    #[test]
    fn fixed_seed_schedule_conservation_suite(
        (is_linear, trace) in vesting_actions(),
    ) {
        run_conservation_trace(is_linear, &trace)?;
    }
}
