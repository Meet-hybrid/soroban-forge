//! TTL management and persistent storage migration tests for the vesting contract (issue #332).

use super::*;
use soroban_forge_shared_utils::ttl::{BUMP_AMOUNT, DAY_IN_LEDGERS};
use soroban_forge_test_utils::ttl::{advance_ledger, entry_is_live, entry_ttl};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec};

const START: u64 = 1_000_000;
const CLIFF: u64 = 1_000;
const DURATION: u64 = 4_000;
const TOTAL: i128 = 10_000;

macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let token_admin = StellarAssetClient::new(&env, &token);
        let token_client = TokenClient::new(&env, &token);

        let contract_id = env.register(Vesting, ());
        let client = SorobanForgeVestingClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);

        token_admin.mint(&contract_id, &100_000_i128);

        (env, token, token_client, contract_id, client, accounts)
    }};
}

#[test]
fn linear_schedule_stored_in_persistent_with_ttl_horizon() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    // Verify it is in persistent storage with the 30-day bump amount
    assert!(entry_is_live(&env, &contract_id, &DataKey::Schedule(id)));
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(id)),
        BUMP_AMOUNT
    );

    // Verify it is NOT in instance storage
    let in_instance = env.as_contract(&contract_id, || {
        env.storage().instance().has(&DataKey::Schedule(id))
    });
    assert!(!in_instance);
}

#[test]
fn tranche_schedule_stored_in_persistent_with_ttl_horizon() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let mut tranches = Vec::new(&env);
    tranches.push_back(Tranche {
        unlock_at: 1_000,
        amount: 5_000,
    });
    tranches.push_back(Tranche {
        unlock_at: 2_000,
        amount: 5_000,
    });

    let id = client.create_tranche_schedule(&accounts.user1, &token, &tranches);

    // Verify it is in persistent storage with the 30-day bump amount
    assert!(entry_is_live(
        &env,
        &contract_id,
        &DataKey::TrancheSchedule(id)
    ));
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::TrancheSchedule(id)),
        BUMP_AMOUNT
    );

    // Verify it is NOT in instance storage
    let in_instance = env.as_contract(&contract_id, || {
        env.storage().instance().has(&DataKey::TrancheSchedule(id))
    });
    assert!(!in_instance);
}

#[test]
fn touch_ttl_extends_linear_and_tranche_ttl() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let linear_id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    let mut tranches = Vec::new(&env);
    tranches.push_back(Tranche {
        unlock_at: 1_000,
        amount: TOTAL,
    });
    let tranche_id = client.create_tranche_schedule(&accounts.user1, &token, &tranches);

    // Advance 10 days
    advance_ledger(&env, 10 * DAY_IN_LEDGERS);
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(linear_id)),
        BUMP_AMOUNT - 10 * DAY_IN_LEDGERS
    );
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::TrancheSchedule(tranche_id)),
        BUMP_AMOUNT - 10 * DAY_IN_LEDGERS
    );

    // Touch both keeper entrypoints
    client.touch_ttl(&linear_id);
    client.touch_ttl(&tranche_id);

    // Both should be restored to the 30-day bump amount
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(linear_id)),
        BUMP_AMOUNT
    );
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::TrancheSchedule(tranche_id)),
        BUMP_AMOUNT
    );
}

#[test]
fn touch_ttl_returns_not_found_for_missing_or_expired() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    // Missing id returns NotFound
    let err = client.try_touch_ttl(&999_u64).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);

    // Create schedule then simulate expiration (host eviction)
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    env.as_contract(&contract_id, || {
        env.storage().persistent().remove(&DataKey::Schedule(id));
    });

    let err = client.try_touch_ttl(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
}

#[test]
fn expired_linear_schedule_returns_not_found_and_never_recreates() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    // Simulate entry expiration (persistent entry evicted from host storage)
    env.as_contract(&contract_id, || {
        env.storage().persistent().remove(&DataKey::Schedule(id));
    });
    env.ledger().set_timestamp(START + DURATION);

    assert_eq!(
        client.try_get_schedule(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_claim(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_claimable(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_get_status(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_revoke(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client
            .try_reassign_beneficiary(&id, &accounts.user2)
            .unwrap_err()
            .unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_touch_ttl(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );

    // Verify storage remains empty and defaults are not recreated
    env.as_contract(&contract_id, || {
        assert!(!env.storage().persistent().has(&DataKey::Schedule(id)));
        assert!(!env.storage().instance().has(&DataKey::Schedule(id)));
    });
}

#[test]
fn expired_tranche_schedule_returns_not_found() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let mut tranches = Vec::new(&env);
    tranches.push_back(Tranche {
        unlock_at: 1_000,
        amount: TOTAL,
    });
    let id = client.create_tranche_schedule(&accounts.user1, &token, &tranches);

    // Simulate entry expiration (persistent entry evicted from host storage)
    env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .remove(&DataKey::TrancheSchedule(id));
    });
    env.ledger().set_timestamp(START + 2_000);

    assert_eq!(
        client.try_get_tranche_schedule(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_claim(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_claimable(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_get_status(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_touch_ttl(&id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );

    env.as_contract(&contract_id, || {
        assert!(!env
            .storage()
            .persistent()
            .has(&DataKey::TrancheSchedule(id)));
        assert!(!env.storage().instance().has(&DataKey::TrancheSchedule(id)));
    });
}

#[test]
fn state_changing_operations_renew_persistent_ttl() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    // 1. Advance 5 days and claim partial tokens -> TTL bumps back to BUMP_AMOUNT
    advance_ledger(&env, 5 * DAY_IN_LEDGERS);
    env.ledger().set_timestamp(START + CLIFF + 1_000);
    let claimed = client.claim(&id);
    assert!(claimed > 0);
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(id)),
        BUMP_AMOUNT
    );

    // 2. Advance 5 days and reassign beneficiary -> TTL bumps back to BUMP_AMOUNT
    advance_ledger(&env, 5 * DAY_IN_LEDGERS);
    env.ledger().set_timestamp(START + CLIFF + 2_000);
    client.reassign_beneficiary(&id, &accounts.user2);
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(id)),
        BUMP_AMOUNT
    );

    // Former beneficiary claim key is in persistent storage with BUMP_AMOUNT
    let former_key = DataKey::FormerBeneficiaryClaim(id, accounts.user1.clone());
    assert!(entry_is_live(&env, &contract_id, &former_key));
    assert_eq!(entry_ttl(&env, &contract_id, &former_key), BUMP_AMOUNT);

    // 3. Advance 5 days and revoke schedule -> TTL bumps back to BUMP_AMOUNT
    advance_ledger(&env, 5 * DAY_IN_LEDGERS);
    env.ledger().set_timestamp(START + CLIFF + 2_500);
    client.revoke(&id);
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(id)),
        BUMP_AMOUNT
    );
}

#[test]
fn lazy_migration_linear_schedule_from_instance_storage() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let id = 42_u64;
    let schedule = VestingSchedule {
        funder: accounts.deployer.clone(),
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        start: START,
        cliff: CLIFF,
        duration: DURATION,
        claimed: 1_000,
        reassignment_vested: 0,
        beneficiary_claimed: 1_000,
        reassignment_count: 0,
        revoked_vested: None,
        status: VestingStatus::Vesting,
    };

    // Seed directly in instance storage (simulating pre-migration state)
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&DataKey::Schedule(id), &schedule);
    });

    // Read through get_schedule
    let loaded = client.get_schedule(&id);
    assert_eq!(loaded.total_amount, TOTAL);
    assert_eq!(loaded.claimed, 1_000);

    // Should now be migrated to persistent storage with 30-day TTL
    assert!(entry_is_live(&env, &contract_id, &DataKey::Schedule(id)));
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(id)),
        BUMP_AMOUNT
    );

    // Should be pruned from instance storage
    let in_instance = env.as_contract(&contract_id, || {
        env.storage().instance().has(&DataKey::Schedule(id))
    });
    assert!(!in_instance);
}

#[test]
fn lazy_migration_tranche_schedule_from_instance_storage() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let id = 88_u64;
    let mut tranches = Vec::new(&env);
    tranches.push_back(Tranche {
        unlock_at: 1_000,
        amount: TOTAL,
    });
    let schedule = TrancheSchedule {
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        start: START,
        tranches,
        claimed: 0,
        status: VestingStatus::Locked,
    };

    // Seed directly in instance storage
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&DataKey::TrancheSchedule(id), &schedule);
    });

    let loaded = client.get_tranche_schedule(&id);
    assert_eq!(loaded.total_amount, TOTAL);

    // Should now be migrated to persistent storage with 30-day TTL
    assert!(entry_is_live(
        &env,
        &contract_id,
        &DataKey::TrancheSchedule(id)
    ));
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::TrancheSchedule(id)),
        BUMP_AMOUNT
    );

    let in_instance = env.as_contract(&contract_id, || {
        env.storage().instance().has(&DataKey::TrancheSchedule(id))
    });
    assert!(!in_instance);
}

#[test]
fn backward_compatible_decoding_v0_linear_schedule() {
    let (env, token, tc, contract_id, client, accounts) = setup!();

    let id = 101_u64;
    let v0 = VestingScheduleV0 {
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        start: START,
        cliff: CLIFF,
        duration: DURATION,
        claimed: 2_000,
        status: VestingStatus::Vesting,
    };

    // Seed v0 into instance storage
    env.as_contract(&contract_id, || {
        env.storage().instance().set(&DataKey::Schedule(id), &v0);
    });

    // Read through get_schedule
    let loaded = client.get_schedule(&id);
    assert_eq!(loaded.beneficiary, accounts.user1);
    assert_eq!(loaded.funder, accounts.user1); // funder defaulted to beneficiary
    assert_eq!(loaded.claimed, 2_000);
    assert_eq!(loaded.reassignment_count, 0);
    assert_eq!(loaded.revoked_vested, None);

    // Verify it was migrated to persistent storage
    assert!(entry_is_live(&env, &contract_id, &DataKey::Schedule(id)));
    assert_eq!(
        entry_ttl(&env, &contract_id, &DataKey::Schedule(id)),
        BUMP_AMOUNT
    );

    // Beneficiary can claim remaining tokens seamlessly
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claim(&id), TOTAL - 2_000);
    assert_eq!(tc.balance(&accounts.user1), TOTAL - 2_000);
}

#[test]
fn backward_compatible_decoding_v1_linear_schedule() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let id = 102_u64;
    let v1 = VestingScheduleV1 {
        funder: accounts.deployer.clone(),
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        start: START,
        cliff: CLIFF,
        duration: DURATION,
        claimed: 1_500,
        revoked_vested: Some(6_000),
        status: VestingStatus::Revoked,
    };

    // Seed v1 into instance storage
    env.as_contract(&contract_id, || {
        env.storage().instance().set(&DataKey::Schedule(id), &v1);
    });

    let loaded = client.get_schedule(&id);
    assert_eq!(loaded.funder, accounts.deployer);
    assert_eq!(loaded.beneficiary, accounts.user1);
    assert_eq!(loaded.claimed, 1_500);
    assert_eq!(loaded.revoked_vested, Some(6_000));
    assert_eq!(loaded.status, VestingStatus::Revoked);

    // Verify it was migrated to persistent storage
    assert!(entry_is_live(&env, &contract_id, &DataKey::Schedule(id)));

    // Claim on revoked schedule works up to frozen limit
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claimable(&id), 6_000 - 1_500);
    assert_eq!(client.claim(&id), 6_000 - 1_500);
    assert_eq!(client.claimable(&id), 0);
}

#[test]
fn former_beneficiary_claim_lazy_migration_and_claim_for() {
    let (env, token, tc, contract_id, client, accounts) = setup!();

    let id = 200_u64;
    let schedule = VestingSchedule {
        funder: accounts.deployer.clone(),
        beneficiary: accounts.user2.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        start: START,
        cliff: CLIFF,
        duration: DURATION,
        claimed: 3_000,
        reassignment_vested: 5_000,
        beneficiary_claimed: 0,
        reassignment_count: 1,
        revoked_vested: None,
        status: VestingStatus::Vesting,
    };

    // Put schedule in persistent and former claim in legacy instance storage
    env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::Schedule(id), &schedule);
        let former_key = DataKey::FormerBeneficiaryClaim(id, accounts.user1.clone());
        env.storage().instance().set(&former_key, &2_000_i128);
    });

    // Check claimable_for migrates and sees former balance
    env.ledger().set_timestamp(START + CLIFF + 2_000);
    assert_eq!(client.claimable_for(&id, &accounts.user1), 2_000);

    // Claim for former beneficiary
    assert_eq!(client.claim_for(&id, &accounts.user1), 2_000);
    assert_eq!(tc.balance(&accounts.user1), 2_000);
    assert_eq!(client.claimable_for(&id, &accounts.user1), 0);

    // Both instance and persistent entries for former balance should be cleared
    let former_key = DataKey::FormerBeneficiaryClaim(id, accounts.user1.clone());
    let (in_pers, in_inst) = env.as_contract(&contract_id, || {
        (
            env.storage().persistent().has(&former_key),
            env.storage().instance().has(&former_key),
        )
    });
    assert!(!in_pers);
    assert!(!in_inst);
}

#[test]
fn monotonic_counter_stays_in_instance_storage() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();

    let id1 = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    let id2 = client.create_schedule(
        &accounts.deployer,
        &accounts.user2,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);

    let count: u64 = env.as_contract(&contract_id, || {
        env.storage().instance().get(&DataKey::Count).unwrap()
    });
    assert_eq!(count, 2);

    // Count is not in persistent storage
    let pers_count = env.as_contract(&contract_id, || {
        env.storage().persistent().has(&DataKey::Count)
    });
    assert!(!pers_count);
}
