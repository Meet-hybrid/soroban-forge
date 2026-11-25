use crate::{
    CreateScheduleArgs, DataKey, SorobanForgeVestingClient, Tranche, Vesting,
    MAX_SCHEDULE_PAGE_SIZE,
};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::{ForgeError, BUMP_AMOUNT};
use soroban_sdk::testutils::{storage::Persistent as _, Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Vec};

struct World {
    env: Env,
    contract: Address,
    accounts: [Address; 5],
    token: Address,
}

impl World {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let accounts: [Address; 5] = core::array::from_fn(|_| Address::generate(&env));
        let contract = env.register(Vesting, ());
        let token = env
            .register_stellar_asset_contract_v2(accounts[0].clone())
            .address();
        let mint = StellarAssetClient::new(&env, &token);
        for account in &accounts[..2] {
            mint.mint(account, &100_000);
        }
        mint.mint(&contract, &100_000);
        Self {
            env,
            contract,
            accounts,
            token,
        }
    }

    fn client(&self) -> SorobanForgeVestingClient<'_> {
        SorobanForgeVestingClient::new(&self.env, &self.contract)
    }

    fn args(&self, beneficiary: usize, amount: i128) -> CreateScheduleArgs {
        CreateScheduleArgs {
            beneficiary: self.accounts[beneficiary].clone(),
            token: self.token.clone(),
            total_amount: amount,
            cliff: 0,
            duration: 100,
        }
    }

    fn create(&self, funder: usize, beneficiary: usize) -> u64 {
        self.client().create_schedule(
            &self.accounts[funder],
            &self.accounts[beneficiary],
            &self.token,
            &100,
            &0,
            &100,
        )
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12,
        rng_seed: RngSeed::Fixed(702),
        ..ProptestConfig::default()
    })]

    #[test]
    fn indexes_match_the_schedule_table(
        steps in prop::collection::vec((0u8..7, 0usize..32, 0usize..2, 2usize..5), 8..=23),
        page_size in prop_oneof![1u32..=5, 1u32..=MAX_SCHEDULE_PAGE_SIZE],
    ) {
        let world = World::new();
        let client = world.client();
        let mut count = world.create(0, 2);
        for (step, (op, picked, funder, beneficiary)) in steps.into_iter().enumerate() {
            world.env.ledger().set_timestamp(step as u64 * 6);
            let id = (picked as u64 % count) + 1;
            match op {
                0 => { count = world.create(funder, beneficiary); }
                1 => {
                    let batch = Vec::from_array(&world.env, [
                        world.args(beneficiary, 100),
                        world.args(2 + (beneficiary - 1) % 3, 100),
                    ]);
                    let ids = client.create_schedules_batch(&world.accounts[funder], &batch);
                    prop_assert_eq!(ids, Vec::from_array(&world.env, [count + 1, count + 2]));
                    count += 2;
                }
                2 => {
                    let batch = Vec::from_array(&world.env, [
                        world.args(beneficiary, 100),
                        world.args(beneficiary, 0),
                    ]);
                    prop_assert_eq!(
                        client.try_create_schedules_batch(&world.accounts[funder], &batch),
                        Err(Ok(ForgeError::InvalidInput))
                    );
                }
                3 => {
                    let _ = client.try_reassign_beneficiary(&id, &world.accounts[beneficiary]);
                }
                4 => { let _ = client.try_revoke(&id); }
                5 => { let _ = client.try_claim(&id); }
                _ => {
                    count = client.create_tranche_schedule(
                        &world.accounts[beneficiary], &world.token,
                        &Vec::from_array(&world.env, [Tranche { amount: 100, unlock_at: 10 }]),
                    );
                }
            }

            let stored_count: u64 = world.env.as_contract(&world.contract, || {
                world.env.storage().instance().get(&DataKey::Count).unwrap()
            });
            prop_assert_eq!(stored_count, count);

            let mut beneficiaries = soroban_sdk::Map::new(&world.env);
            let mut funders = soroban_sdk::Map::new(&world.env);
            for id in 1..=count {
                let (beneficiary, funder) = match client.try_get_schedule(&id) {
                    Ok(Ok(schedule)) => (schedule.beneficiary, schedule.funder),
                    _ => {
                        let schedule = client.get_tranche_schedule(&id);
                        (schedule.beneficiary.clone(), schedule.beneficiary)
                    }
                };
                for (index, owner) in [(&mut beneficiaries, beneficiary), (&mut funders, funder)] {
                    let mut ids = index.get(owner.clone()).unwrap_or_else(|| Vec::new(&world.env));
                    ids.push_back(id);
                    index.set(owner, ids);
                }
            }

            for address in &world.accounts {
                for (is_funder, index) in [(false, &beneficiaries), (true, &funders)] {
                    let expected = index.get(address.clone()).unwrap_or_else(|| Vec::new(&world.env));
                    let mut ids = Vec::new(&world.env);
                    loop {
                        let offset = ids.len();
                        let page = if is_funder {
                            client.schedules_for_funder(address, &offset, &page_size)
                        } else {
                            client.schedules_for_beneficiary(address, &offset, &page_size)
                        };
                        let last = page.len() < page_size;
                        ids.append(&page);
                        prop_assert!(ids.len() <= expected.len());
                        if last { break; }
                    }
                    prop_assert_eq!(ids, expected);
                }
            }
        }
    }
}

#[test]
fn paging_bounds_reassignment_and_index_ttl() {
    let world = World::new();
    let client = world.client();
    let a = &world.accounts[2];
    let b = &world.accounts[3];
    let funder = &world.accounts[0];
    let first = world.create(0, 2);
    let second = world.create(0, 2);
    client.reassign_beneficiary(&first, b);
    assert_eq!(
        client.schedules_for_beneficiary(a, &0, &100),
        Vec::from_array(&world.env, [second])
    );
    let key = DataKey::BeneficiaryIndex(a.clone());
    let funder_key = DataKey::FunderIndex(funder.clone());
    let ttl = |key| {
        world.env.as_contract(&world.contract, || {
            world.env.storage().persistent().get_ttl(key)
        })
    };
    assert_eq!(ttl(&key), BUMP_AMOUNT);
    assert_eq!(ttl(&funder_key), BUMP_AMOUNT);
    world.env.as_contract(&world.contract, || {
        world
            .env
            .storage()
            .instance()
            .extend_ttl(BUMP_AMOUNT, BUMP_AMOUNT);
    });
    world
        .env
        .ledger()
        .set_sequence_number(world.env.ledger().sequence() + 18_000);
    let aged = ttl(&key);
    for limit in [0, MAX_SCHEDULE_PAGE_SIZE + 1] {
        assert_eq!(
            client.try_schedules_for_beneficiary(a, &0, &limit),
            Err(Ok(ForgeError::InvalidInput))
        );
        assert_eq!(
            client.try_schedules_for_funder(funder, &0, &limit),
            Err(Ok(ForgeError::InvalidInput))
        );
    }
    assert!(client
        .schedules_for_beneficiary(a, &u32::MAX, &1)
        .is_empty());
    assert!(client
        .schedules_for_funder(&world.accounts[4], &0, &1)
        .is_empty());
    assert_eq!(ttl(&key), aged);
    client.reassign_beneficiary(&first, a);
    assert_eq!(ttl(&key), BUMP_AMOUNT);
    assert_eq!(ttl(&funder_key), aged);
    assert_eq!(
        client.schedules_for_beneficiary(a, &0, &1),
        Vec::from_array(&world.env, [first])
    );
    assert_eq!(
        client.schedules_for_beneficiary(a, &1, &1),
        Vec::from_array(&world.env, [second])
    );
    assert!(client.schedules_for_beneficiary(b, &0, &1).is_empty());
    assert_eq!(
        client.schedules_for_funder(funder, &0, &100),
        Vec::from_array(&world.env, [first, second])
    );
}

#[test]
fn discovery_includes_pre_index_schedules_after_upgrade() {
    let world = World::new();
    let client = world.client();
    let legacy = world.create(0, 2);
    let a = &world.accounts[2];
    let b = &world.accounts[3];
    let funder = &world.accounts[0];
    world.env.as_contract(&world.contract, || {
        world
            .env
            .storage()
            .instance()
            .remove(&DataKey::LegacyIndexEnd);
        world
            .env
            .storage()
            .persistent()
            .remove(&DataKey::BeneficiaryIndex(a.clone()));
        world
            .env
            .storage()
            .persistent()
            .remove(&DataKey::FunderIndex(funder.clone()));
    });
    assert_eq!(
        client.schedules_for_beneficiary(a, &0, &1),
        Vec::from_array(&world.env, [legacy])
    );

    let indexed = world.create(0, 3);
    client.reassign_beneficiary(&legacy, b);
    assert!(client.schedules_for_beneficiary(a, &0, &1).is_empty());
    assert_eq!(
        client.schedules_for_beneficiary(b, &0, &2),
        Vec::from_array(&world.env, [legacy, indexed])
    );
    assert_eq!(
        client.schedules_for_funder(funder, &1, &2),
        Vec::from_array(&world.env, [indexed])
    );
}
