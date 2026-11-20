use super::*;
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::events::Event as _;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Env, Vec as SorobanVec};

const START: u64 = 1_000_000;
const CLIFF: u64 = 1_000;
const DURATION: u64 = 4_000;
const TOTAL: i128 = 10_000;

/// Build a fresh env with mocked auths, a registered contract, and named
/// accounts. The generated client borrows the env, so it cannot be
/// returned from a helper.
macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);

        // Real Stellar Asset Contract — the same fixture escrow uses.
        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let token_admin = StellarAssetClient::new(&env, &token);
        let token_client = TokenClient::new(&env, &token);
        let contract_id = env.register(Vesting, ());
        let client = SorobanForgeVestingClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        // Fund the schedule contract up front with the full allocation,
        // the way a deployment is topped up before schedules run.
        token_admin.mint(&contract_id, &TOTAL);
        (env, token, token_client, contract_id, client, accounts)
    }};
}

// NOTE: In soroban-sdk 27.0.6, entrypoint `require_auth` checks are verified
// via `env.mock_auths` (enforce mode). An unauthorized caller or a signature
// with mismatched arguments aborts the invocation with `Err(Err(InvokeError::Abort))`.
// Contract self-authorization for token settlement is implicit in the host:
// the executing contract's own transfer is auto-approved and requires no
// separate user signature frame.

fn create(client: &SorobanForgeVestingClient<'_>, token: &Address, accounts: &TestAccounts) -> u64 {
    client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    )
}

#[test]
fn create_schedule_succeeds_and_is_locked() {
    let (_env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    assert_eq!(client.get_status(&id), VestingStatus::Locked);
    assert_eq!(client.claimable(&id), 0);
}

#[test]
fn create_schedule_assigns_distinct_ids() {
    let (_env, token, _tc, _cid, client, accounts) = setup!();
    let id1 = create(&client, &token, &accounts);
    let id2 = create(&client, &token, &accounts);
    assert_ne!(id1, id2);
}

#[test]
fn create_schedule_without_cliff_starts_vesting() {
    let (_env, _token, _tc, _cid, client, accounts) = setup!();
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &accounts.validator,
        &TOTAL,
        &0_u64,
        &DURATION,
    );
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
}

#[test]
fn create_schedule_rejects_zero_total() {
    let (_env, _token, _tc, _cid, client, accounts) = setup!();
    let err = client
        .try_create_schedule(
            &accounts.deployer,
            &accounts.user1,
            &accounts.validator,
            &0_i128,
            &CLIFF,
            &DURATION,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn create_schedule_rejects_zero_duration() {
    let (_env, _token, _tc, _cid, client, accounts) = setup!();
    let err = client
        .try_create_schedule(
            &accounts.deployer,
            &accounts.user1,
            &accounts.validator,
            &TOTAL,
            &CLIFF,
            &0_u64,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn create_schedule_rejects_cliff_after_duration() {
    let (_env, _token, _tc, _cid, client, accounts) = setup!();
    let err = client
        .try_create_schedule(
            &accounts.deployer,
            &accounts.user1,
            &accounts.validator,
            &TOTAL,
            &5_000_u64,
            &4_000_u64,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn claimable_before_cliff_is_zero() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    // Halfway between start and the cliff.
    env.ledger().set_timestamp(START + CLIFF / 2);
    assert_eq!(client.claimable(&id), 0);
}

#[test]
fn claimable_at_cliff_is_zero() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + CLIFF);
    assert_eq!(client.claimable(&id), 0);
}

#[test]
fn claim_before_cliff_returns_zero() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + CLIFF / 2);
    assert_eq!(client.claim(&id), 0);
}

#[test]
fn claimable_midway_is_half() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    // Halfway through the vesting window (cliff .. duration).
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    assert_eq!(client.claimable(&id), TOTAL / 2);
}

#[test]
fn claimable_at_duration_is_full() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claimable(&id), TOTAL);
}

#[test]
fn claimable_after_duration_is_full() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + DURATION + 1);
    assert_eq!(client.claimable(&id), TOTAL);
}

#[test]
fn claim_pays_exact_amount() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    assert_eq!(client.claim(&id), TOTAL / 2);
}

#[test]
fn repeated_claims_never_overpay_or_underpay() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Claim half at the midway point.
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    assert_eq!(client.claim(&id), TOTAL / 2);
    assert_eq!(client.claimable(&id), 0);

    // Advance past the end; the remaining half becomes claimable.
    env.ledger().set_timestamp(START + DURATION + 100);
    assert_eq!(client.claim(&id), TOTAL - TOTAL / 2);
    assert_eq!(client.claimable(&id), 0);

    // A further claim is a no-op.
    assert_eq!(client.claim(&id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
}

#[test]
fn claim_after_end_completes_status() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + DURATION + 1);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.claim(&id), TOTAL);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
}

#[test]
fn claim_without_cliff_vests_from_start() {
    let (env, _token, _tc, _cid, client, accounts) = setup!();
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &accounts.validator,
        &TOTAL,
        &0_u64,
        &DURATION,
    );
    env.ledger().set_timestamp(START + DURATION / 2);
    assert_eq!(client.claimable(&id), TOTAL / 2);
}

#[test]
fn cliff_equals_duration_vests_at_once() {
    let (env, _token, _tc, _cid, client, accounts) = setup!();
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &accounts.validator,
        &TOTAL,
        &DURATION,
        &DURATION,
    );
    env.ledger().set_timestamp(START + DURATION - 1);
    assert_eq!(client.claimable(&id), 0);
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claimable(&id), TOTAL);
}

#[test]
fn claim_missing_schedule_is_not_found() {
    let (_env, _token, _tc, _cid, client, _accounts) = setup!();
    let err = client.try_claim(&999).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
}

#[test]
fn claimable_missing_schedule_is_not_found() {
    let (_env, _token, _tc, _cid, client, _accounts) = setup!();
    let err = client.try_claimable(&999).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
}

#[test]
fn get_status_missing_schedule_is_not_found() {
    let (_env, _token, _tc, _cid, client, _accounts) = setup!();
    let err = client.try_get_status(&999).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
}

#[test]
fn claimable_overflow_is_reported() {
    let (env, _token, _tc, _cid, client, accounts) = setup!();
    // A huge total with a non-trivial elapsed time overflows the
    // intermediate `total * elapsed` product.
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &accounts.validator,
        &i128::MAX,
        &0_u64,
        &1_000_u64,
    );
    env.ledger().set_timestamp(START + 500);
    let err = client.try_claimable(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::ArithmeticOverflow);
}

// -------------------------------------------------------------------
// Settlement: claim pays real SEP-41 tokens
// -------------------------------------------------------------------

#[test]
fn claim_transfers_vested_tokens_to_beneficiary() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Halfway through the vesting window: 5_000 claimable.
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    assert_eq!(client.claim(&id), TOTAL / 2);

    // Tokens actually moved, and the schedule recorded the claim.
    assert_eq!(tc.balance(&accounts.user1), TOTAL / 2);
    assert_eq!(tc.balance(&contract_id), TOTAL - TOTAL / 2);
    assert_eq!(client.claimable(&id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
}

#[test]
fn final_claim_settles_remainder_and_completes() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    assert_eq!(client.claim(&id), TOTAL / 2);

    env.ledger().set_timestamp(START + DURATION + 100);
    assert_eq!(client.claim(&id), TOTAL - TOTAL / 2);

    // Full allocation paid out; contract drained; schedule completed.
    assert_eq!(tc.balance(&accounts.user1), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
    assert_eq!(client.claimable(&id), 0);

    // A repeated claim after completion is a silent no-op: no transfer.
    assert_eq!(client.claim(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
}

#[test]
fn repeated_claims_settle_token_balances_each_time() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Three separate claims across the window; each moves exactly the
    // newly claimable amount and nothing more.
    env.ledger().set_timestamp(START + CLIFF + 1_000);
    assert_eq!(client.claim(&id), 3_333);
    assert_eq!(tc.balance(&accounts.user1), 3_333);

    env.ledger().set_timestamp(START + CLIFF + 2_000);
    assert_eq!(client.claim(&id), 3_333);
    assert_eq!(tc.balance(&accounts.user1), 6_666);

    env.ledger().set_timestamp(START + DURATION + 1);
    assert_eq!(client.claim(&id), 3_334);
    assert_eq!(tc.balance(&accounts.user1), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
}

#[test]
fn failed_transfer_leaves_claim_unchanged() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    // Schedule promises double what the contract actually holds.
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &(TOTAL * 2),
        &0_u64,
        &DURATION,
    );
    env.ledger().set_timestamp(START + DURATION + 1);
    assert_eq!(client.claimable(&id), TOTAL * 2);

    let err = client.try_claim(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::TokenTransferFailed);

    // Nothing moved, nothing recorded: balances and schedule unchanged.
    assert_eq!(tc.balance(&accounts.user1), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);
    assert_eq!(client.claimable(&id), TOTAL * 2);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
}

#[test]
fn zero_claim_issues_no_token_transfer() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Before the cliff: returns 0, moves nothing.
    env.ledger().set_timestamp(START + CLIFF / 2);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);

    // A second immediate claim right after a payout is also a no-op.
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    assert_eq!(client.claim(&id), TOTAL / 2);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), TOTAL / 2);
    assert_eq!(tc.balance(&contract_id), TOTAL - TOTAL / 2);
}

#[test]
fn floor_division_residue_stays_claimable_until_final_claim() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // 1_000 / 3_000 through the window:
    // 10_000 * 1_000 / 3_000 = 3_333 (floored) — one stroop of residue
    // stays behind, and the final claim pays it out in full.
    env.ledger().set_timestamp(START + CLIFF + 1_000);
    let first = client.claim(&id);
    assert_eq!(first, 3_333);

    env.ledger().set_timestamp(START + DURATION + 1);
    let rest = client.claim(&id);
    assert_eq!(first + rest, TOTAL);
    assert_eq!(tc.balance(&accounts.user1), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
}

// -------------------------------------------------------------------
// Boundary, Cliff == Duration, and Timing Tests
// -------------------------------------------------------------------

#[test]
fn boundary_cliff_and_duration_step_progression() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // At creation start:
    env.ledger().set_timestamp(START);
    assert_eq!(client.get_status(&id), VestingStatus::Locked);
    assert_eq!(client.claimable(&id), 0);

    // 1 second before cliff:
    env.ledger().set_timestamp(START + CLIFF - 1);
    assert_eq!(client.get_status(&id), VestingStatus::Locked);
    assert_eq!(client.claimable(&id), 0);

    // At cliff boundary:
    env.ledger().set_timestamp(START + CLIFF);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.claimable(&id), 0);

    // 1 second after cliff: floor(10_000 * 1 / 3_000) = 3
    env.ledger().set_timestamp(START + CLIFF + 1);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.claimable(&id), 3);

    // 1 second before full duration: floor(10_000 * 2_999 / 3_000) = 9_996
    env.ledger().set_timestamp(START + DURATION - 1);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.claimable(&id), 9_996);

    // Exactly at duration:
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.claimable(&id), TOTAL);

    // After duration:
    env.ledger().set_timestamp(START + DURATION + 10_000);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.claimable(&id), TOTAL);
}

#[test]
fn interleaved_partial_claims_across_arbitrary_timestamps() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Five sequential claim points:
    // Period is 3_000 seconds, total is 10_000.
    // Step 1: elapsed 300 -> 10_000 * 300 / 3_000 = 1_000.
    env.ledger().set_timestamp(START + CLIFF + 300);
    assert_eq!(client.claim(&id), 1_000);
    assert_eq!(tc.balance(&accounts.user1), 1_000);
    assert_eq!(client.claimable(&id), 0);

    // Step 2: elapsed 750 -> 10_000 * 750 / 3_000 = 2_500 vested. Claimable = 2_500 - 1_000 = 1_500.
    env.ledger().set_timestamp(START + CLIFF + 750);
    assert_eq!(client.claim(&id), 1_500);
    assert_eq!(tc.balance(&accounts.user1), 2_500);
    assert_eq!(client.claimable(&id), 0);

    // Step 3: elapsed 1_500 -> 10_000 * 1_500 / 3_000 = 5_000 vested. Claimable = 5_000 - 2_500 = 2_500.
    env.ledger().set_timestamp(START + CLIFF + 1_500);
    assert_eq!(client.claim(&id), 2_500);
    assert_eq!(tc.balance(&accounts.user1), 5_000);
    assert_eq!(client.claimable(&id), 0);

    // Step 4: elapsed 2_250 -> 10_000 * 2_250 / 3_000 = 7_500 vested. Claimable = 7_500 - 5_000 = 2_500.
    env.ledger().set_timestamp(START + CLIFF + 2_250);
    assert_eq!(client.claim(&id), 2_500);
    assert_eq!(tc.balance(&accounts.user1), 7_500);
    assert_eq!(client.claimable(&id), 0);

    // Step 5: elapsed 3_000 (end of duration) -> full 10_000 vested. Remainder = 2_500.
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claim(&id), 2_500);
    assert_eq!(tc.balance(&accounts.user1), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
    assert_eq!(client.claimable(&id), 0);
}

#[test]
fn repeated_zero_claims_at_and_before_cliff() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Repeated claims before cliff:
    env.ledger().set_timestamp(START + 100);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);

    // Repeated claims at cliff:
    env.ledger().set_timestamp(START + CLIFF);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);
}

#[test]
fn timestamp_u64_max_edge_case() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Ledger timestamp at maximum u64 value:
    env.ledger().set_timestamp(u64::MAX);
    assert_eq!(client.claimable(&id), TOTAL);
    assert_eq!(client.claim(&id), TOTAL);
    assert_eq!(tc.balance(&accounts.user1), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
}

#[test]
fn total_amount_i128_max_at_duration_succeeds() {
    let (env, _token, _tc, _cid, client, accounts) = setup!();
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &accounts.validator,
        &i128::MAX,
        &0_u64,
        &1_000_u64,
    );
    // At or after duration, vested_amount returns total_amount directly
    // without computing total_amount * elapsed / period, so it does not overflow.
    env.ledger().set_timestamp(START + 1_000);
    assert_eq!(client.claimable(&id), i128::MAX);

    env.ledger().set_timestamp(START + 2_000);
    assert_eq!(client.claimable(&id), i128::MAX);
}

#[test]
fn start_plus_duration_u64_overflow_reported() {
    let (env, _token, _tc, _cid, client, accounts) = setup!();
    // A schedule whose duration would cause start + duration to overflow u64.
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &accounts.validator,
        &TOTAL,
        &0_u64,
        &u64::MAX,
    );
    env.ledger().set_timestamp(START + 100);
    let err = client.try_claimable(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::ArithmeticOverflow);

    let claim_err = client.try_claim(&id).unwrap_err().unwrap();
    assert_eq!(claim_err, ForgeError::ArithmeticOverflow);
}

#[test]
fn monotonic_id_counter_overflow_reported() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    // Set Count key to u64::MAX directly in contract storage:
    env.as_contract(&contract_id, || {
        env.storage().instance().set(&DataKey::Count, &u64::MAX);
    });

    let err = client
        .try_create_schedule(
            &accounts.deployer,
            &accounts.user1,
            &token,
            &TOTAL,
            &CLIFF,
            &DURATION,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::ArithmeticOverflow);
}

// ---------------------------------------------------------------------------
// `get_schedule` record view (issue #125)
// ---------------------------------------------------------------------------

/// The view returns the complete stored record for an existing id — every
/// creation parameter round-trips, and the initial state matches what
/// `claimable`/`get_status` read.
#[test]
fn get_schedule_returns_the_full_stored_record() {
    let (_env, token, _tc, _cid, client, accounts) = setup!();
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    let schedule = client.get_schedule(&id);
    assert_eq!(schedule.funder, accounts.deployer);
    assert_eq!(schedule.beneficiary, accounts.user1);
    assert_eq!(schedule.token, token);
    assert_eq!(schedule.total_amount, TOTAL);
    assert_eq!(schedule.start, START);
    assert_eq!(schedule.cliff, CLIFF);
    assert_eq!(schedule.duration, DURATION);
    assert_eq!(schedule.claimed, 0);
    assert_eq!(schedule.revoked_vested, None);
    assert_eq!(schedule.status, VestingStatus::Locked);
    assert_eq!(client.claimable(&id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Locked);
}

#[test]
fn get_schedule_unknown_id_returns_not_found() {
    let (_env, _token, _tc, _cid, client, _accounts) = setup!();
    let err = client.try_get_schedule(&7_u64).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
}

/// The two kinds partition one id space: a tranche id is `NotFound` in the
/// linear view (and vice versa, per `get_tranche_schedule`).
#[test]
fn get_schedule_tranche_id_returns_not_found() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let tranches = soroban_sdk::Vec::from_array(
        &env,
        [
            Tranche {
                unlock_at: 0,
                amount: 2_000,
            },
            Tranche {
                unlock_at: 1_000,
                amount: 8_000,
            },
        ],
    );
    let id = client.create_tranche_schedule(&accounts.user1, &token, &tranches);

    let err = client.try_get_schedule(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
    // The tranche view does return that record.
    let tranche = client.get_tranche_schedule(&id);
    assert_eq!(tranche.total_amount, 10_000);
}

/// `claimed` in the record advances with completed claims: the view and the
/// claim path read the same storage, and the stored status follows the
/// settled state.
#[test]
fn get_schedule_reflects_claims() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    assert_eq!(client.get_schedule(&id).claimed, 0);

    // Halfway through the ramp: floor(TOTAL * 2_000 / 3_000) vests.
    env.ledger().set_timestamp(START + CLIFF + DURATION / 2);
    let payout = client.claim(&id);
    assert_eq!(payout, 6_666);

    let schedule = client.get_schedule(&id);
    assert_eq!(schedule.claimed, 6_666);
    assert_eq!(schedule.status, VestingStatus::Vesting);
    assert_eq!(tc.balance(&accounts.user1), 6_666);
    assert_eq!(tc.balance(&contract_id), TOTAL - 6_666);

    // After the final claim the record is Completed with everything claimed.
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claim(&id), TOTAL - 6_666);
    let schedule = client.get_schedule(&id);
    assert_eq!(schedule.claimed, TOTAL);
    assert_eq!(schedule.status, VestingStatus::Completed);
}

/// The view is read-only: repeated reads return the same record and the id
/// counter (the only cross-call state) never moves.
#[test]
fn get_schedule_is_read_only() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    let before = client.get_schedule(&id);
    let count_before: u64 = env.as_contract(&contract_id, || {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    });
    assert_eq!(count_before, 1);

    let again = client.get_schedule(&id);
    assert_eq!(again.claimed, before.claimed);
    assert_eq!(again.start, before.start);
    assert_eq!(again.total_amount, before.total_amount);

    let count_after: u64 = env.as_contract(&contract_id, || {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    });
    assert_eq!(count_after, count_before);

    // A read against an empty id leaves nothing behind either.
    let err = client.try_get_schedule(&99_u64).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
    let count_final: u64 = env.as_contract(&contract_id, || {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    });
    assert_eq!(count_final, count_before);
}

/// No authorization anywhere in the view's path: it succeeds under a blank
/// envelope, like `claimable` and `get_status`.
#[test]
fn get_schedule_requires_no_auth() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.set_auths(&[]);
    let schedule = client.get_schedule(&id);
    assert_eq!(schedule.beneficiary, accounts.user1);
}

// ---------------------------------------------------------------------------
// Revocation (issue #67): the funder terminates a grant; `Revoked` is real
// ---------------------------------------------------------------------------

/// Revoking a schedule that has not reached its cliff transitions it to
/// `Revoked` and freezes nothing: nothing had vested at the revocation
/// timestamp, so nothing is ever claimable — including after the original
/// cliff and duration would have passed.
#[test]
fn revoke_while_locked_freezes_zero_and_stops_vesting() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Still before the cliff: Locked.
    env.ledger().set_timestamp(START + CLIFF / 2);
    assert_eq!(client.get_status(&id), VestingStatus::Locked);

    client.revoke(&id);

    // The schedule is Revoked and stays Revoked: no time-derived status.
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
    assert_eq!(client.claimable(&id), 0);

    // Vesting is permanently stopped: the old cliff and duration pass with
    // nothing accruing and nothing payable.
    env.ledger().set_timestamp(START + DURATION + 100);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
    assert_eq!(client.claimable(&id), 0);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);
    assert_eq!(client.get_schedule(&id).revoked_vested, Some(0));
}

/// Revoking mid-vesting freezes the vested amount at the revocation ledger
/// timestamp: a partial claim remains possible up to that amount, and
/// everything past the revocation timestamp never vests.
#[test]
fn revoke_mid_vesting_freezes_amount_and_allows_partial_claim() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Claim 2_500 before revocation.
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 4);
    assert_eq!(client.claimable(&id), TOTAL / 4);
    assert_eq!(client.claim(&id), TOTAL / 4);

    // Halfway through the vesting window: 5_000 of 10_000 vested.
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    assert_eq!(client.claimable(&id), TOTAL / 2 - TOTAL / 4);

    client.revoke(&id);

    // The frozen amount survives untouched even long after the schedule
    // would have fully vested.
    env.ledger().set_timestamp(START + DURATION + 100);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
    assert_eq!(client.claimable(&id), TOTAL / 2 - TOTAL / 4);

    // The beneficiary can claim the frozen remainder after revocation.
    assert_eq!(client.claim(&id), TOTAL / 2 - TOTAL / 4);
    assert_eq!(client.claimable(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), TOTAL / 2);
    assert_eq!(tc.balance(&contract_id), TOTAL - TOTAL / 2);

    // A further claim is a silent no-op: no transfer, no status change.
    assert_eq!(client.claim(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), TOTAL / 2);
    assert_eq!(tc.balance(&contract_id), TOTAL - TOTAL / 2);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
    assert_eq!(client.get_schedule(&id).claimed, TOTAL / 2);
}

#[test]
fn claim_then_revoke_conserves_allocation_after_terminal_repeats() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 4);
    let claimed_before_revoke = client.claim(&id);
    assert_eq!(claimed_before_revoke, TOTAL / 4);

    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    client.revoke(&id);
    let frozen_vested = client
        .get_schedule(&id)
        .revoked_vested
        .expect("revoked schedule must retain its frozen vested amount");
    assert_eq!(frozen_vested, TOTAL / 2);

    env.ledger().set_timestamp(START + DURATION + 1);
    let claimed_after_revoke = client.claim(&id);
    assert_eq!(claimed_after_revoke, frozen_vested - claimed_before_revoke);
    assert_eq!(client.claim(&id), 0);
    assert_eq!(
        client.try_revoke(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );

    assert_eq!(
        tc.balance(&accounts.user1) + tc.balance(&contract_id),
        TOTAL,
        "claims before and after revocation must conserve the schedule allocation"
    );
    assert_eq!(tc.balance(&accounts.user1), frozen_vested);
    assert_eq!(tc.balance(&contract_id), TOTAL - frozen_vested);
    assert_eq!(client.get_schedule(&id).claimed, frozen_vested);
    assert_eq!(
        client.get_schedule(&id).revoked_vested,
        Some(frozen_vested),
        "a repeated revoke must not raise the frozen amount"
    );
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
}

/// A completed schedule cannot be revoked: the grant ran its course, so
/// there is nothing left to terminate.
#[test]
fn revoke_after_completion_is_rejected() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    env.ledger().set_timestamp(START + DURATION + 1);
    assert_eq!(client.claim(&id), TOTAL);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);

    let err = client.try_revoke(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    // The rejection changed nothing: fully claimed, fully paid out.
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
    assert_eq!(client.claimable(&id), 0);
    assert_eq!(tc.balance(&accounts.user1), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
}

/// Double revoke is impossible: the second call is rejected and the frozen
/// record is left exactly as the first revocation wrote it.
#[test]
fn double_revoke_is_rejected() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    client.revoke(&id);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);

    // Advance time first: a second revoke must not re-freeze at a later
    // (higher) vested amount.
    env.ledger().set_timestamp(START + DURATION + 100);
    let err = client.try_revoke(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    // The frozen amount is still the one from the first revocation.
    assert_eq!(client.get_schedule(&id).revoked_vested, Some(TOTAL / 2));
    assert_eq!(client.claimable(&id), TOTAL / 2);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
}

/// `claimable` before revocation follows the normal ramp; after revocation
/// it is capped at the frozen amount and drains to `0` as claims land.
#[test]
fn claimable_before_and_after_revocation() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Before revocation the ramp runs normally.
    env.ledger().set_timestamp(START + CLIFF);
    assert_eq!(client.claimable(&id), 0);
    env.ledger().set_timestamp(START + CLIFF + 1_000);
    assert_eq!(client.claimable(&id), 3_333);

    // Revoke with 3_333 vested: the cap freezes there.
    client.revoke(&id);
    assert_eq!(client.claimable(&id), 3_333);

    // Time advancing cannot raise it past the frozen amount.
    env.ledger().set_timestamp(START + DURATION + 100);
    assert_eq!(client.claimable(&id), 3_333);

    // Claims drain it to zero and it stays zero.
    assert_eq!(client.claim(&id), 3_333);
    assert_eq!(client.claimable(&id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
}

/// Revoking an unknown id is `NotFound`; a tranche id (which occupies the
/// same id space under a different key) is `NotFound` in `revoke` too.
#[test]
fn revoke_unknown_or_tranche_id_is_not_found() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let err = client.try_revoke(&999).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);

    // A tranche schedule shares the id space but has no revoke path.
    let tranches = soroban_sdk::Vec::from_array(
        &env,
        [
            Tranche {
                unlock_at: 0,
                amount: 2_000,
            },
            Tranche {
                unlock_at: 1_000,
                amount: 8_000,
            },
        ],
    );
    let tranche_id = client.create_tranche_schedule(&accounts.user1, &token, &tranches);
    let err = client.try_revoke(&tranche_id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
}

/// A `funder == beneficiary` grant is rejected at creation: the roles must
/// be distinct, or `revoke` could be exercised by the beneficiary.
#[test]
fn create_schedule_rejects_funder_equal_to_beneficiary() {
    let (_env, token, _tc, _cid, client, accounts) = setup!();
    let err = client
        .try_create_schedule(
            &accounts.user1,
            &accounts.user1,
            &token,
            &TOTAL,
            &CLIFF,
            &DURATION,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

/// Revocation records the funder and the frozen amount on the stored
/// record: `get_schedule` is the audit view for the state transition.
#[test]
fn revoked_record_freezes_vested_amount_and_status() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // 3_000 through the window: floor(10_000 * 2_000 / 3_000) = 6_666.
    env.ledger().set_timestamp(START + CLIFF + 2_000);
    client.revoke(&id);

    let schedule = client.get_schedule(&id);
    assert_eq!(schedule.funder, accounts.deployer);
    assert_eq!(schedule.revoked_vested, Some(6_666));
    assert_eq!(schedule.claimed, 0);
    assert_eq!(schedule.status, VestingStatus::Revoked);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
    assert_eq!(client.claimable(&id), 6_666);
}

#[test]
fn reassignment_splits_claims_at_the_exact_ledger_boundary() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let new_beneficiary = Address::generate(&env);
    let id = create(&client, &token, &accounts);

    env.ledger().set_timestamp(START + CLIFF + 1_000);
    assert_eq!(client.claim(&id), 3_333);

    let reassignment_time = START + CLIFF + 2_000;
    env.ledger().set_timestamp(reassignment_time);
    client.reassign_beneficiary(&id, &new_beneficiary);
    let emitted_events = env.events().all().filter_by_contract(&contract_id);

    let schedule = client.get_schedule(&id);
    assert_eq!(schedule.beneficiary, new_beneficiary);
    assert_eq!(schedule.reassignment_vested, 6_666);
    assert_eq!(schedule.beneficiary_claimed, 0);
    assert_eq!(schedule.reassignment_count, 1);
    assert_eq!(schedule.total_amount, TOTAL);
    assert_eq!(schedule.start, START);
    assert_eq!(schedule.cliff, CLIFF);
    assert_eq!(schedule.duration, DURATION);
    assert_eq!(client.claimable_for(&id, &accounts.user1), 3_333);
    assert_eq!(client.claimable(&id), 0);
    assert_eq!(client.claim(&id), 0);

    // Claiming in the same ledger as reassignment belongs to the old
    // beneficiary; the new beneficiary begins accruing only after this point.
    assert_eq!(client.claim_for(&id, &accounts.user1), 3_333);
    assert_eq!(tc.balance(&accounts.user1), 6_666);
    assert_eq!(client.claimable_for(&id, &accounts.user1), 0);

    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claimable(&id), 3_334);
    assert_eq!(client.claim(&id), 3_334);
    assert_eq!(tc.balance(&new_beneficiary), 3_334);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_schedule(&id).claimed, TOTAL);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
    let expected_events = std::vec![
        events::BeneficiaryReassignedFrom {
            schedule_id: id,
            beneficiary: accounts.user1.clone(),
            new_beneficiary: new_beneficiary.clone(),
            vested_unclaimed: 3_333,
            reassignment_count: 1,
        }
        .to_xdr(&env, &contract_id),
        events::BeneficiaryReassignedTo {
            schedule_id: id,
            beneficiary: new_beneficiary.clone(),
            old_beneficiary: accounts.user1.clone(),
            vested_unclaimed: 3_333,
            reassignment_count: 1,
        }
        .to_xdr(&env, &contract_id),
    ];
    assert_eq!(emitted_events.events(), expected_events.as_slice());
}

#[test]
fn reassignment_claim_accounting_is_scoped_to_schedule_id() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let first_id = create(&client, &token, &accounts);
    let second_id = client.create_schedule(
        &accounts.deployer,
        &accounts.user2,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);
    client.reassign_beneficiary(&first_id, &accounts.user2);

    assert_eq!(client.claimable_for(&first_id, &accounts.user1), TOTAL / 2);
    assert_eq!(client.claimable(&first_id), 0);
    assert_eq!(client.claimable(&second_id), TOTAL / 2);
}

#[test]
fn repeated_reassignments_preserve_each_former_beneficiary_balance() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let third_beneficiary = Address::generate(&env);
    let id = create(&client, &token, &accounts);

    env.ledger().set_timestamp(START + CLIFF + 1_500);
    client.reassign_beneficiary(&id, &accounts.user2);
    env.ledger().set_timestamp(START + CLIFF + 2_250);
    client.reassign_beneficiary(&id, &third_beneficiary);

    let schedule = client.get_schedule(&id);
    assert_eq!(schedule.reassignment_count, 2);
    assert_eq!(schedule.beneficiary, third_beneficiary);
    assert_eq!(client.claimable_for(&id, &accounts.user1), 5_000);
    assert_eq!(client.claimable_for(&id, &accounts.user2), 2_500);
    assert_eq!(client.claimable(&id), 0);

    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claim_for(&id, &accounts.user1), 5_000);
    assert_eq!(client.claim_for(&id, &accounts.user2), 2_500);
    assert_eq!(client.claim(&id), 2_500);
    assert_eq!(tc.balance(&accounts.user1), 5_000);
    assert_eq!(tc.balance(&accounts.user2), 2_500);
    assert_eq!(tc.balance(&third_beneficiary), 2_500);
    assert_eq!(tc.balance(&contract_id), 0);
}

#[test]
fn reassignment_composes_with_revocation_without_moving_prior_entitlement() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let new_beneficiary = Address::generate(&env);
    let id = create(&client, &token, &accounts);

    env.ledger().set_timestamp(START + CLIFF + 1_500);
    client.reassign_beneficiary(&id, &new_beneficiary);

    env.ledger().set_timestamp(START + CLIFF + 2_250);
    client.revoke(&id);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
    assert_eq!(client.claimable_for(&id, &accounts.user1), 5_000);
    assert_eq!(client.claimable(&id), 2_500);

    let err = client
        .try_reassign_beneficiary(&id, &accounts.validator)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    env.ledger().set_timestamp(START + DURATION + 100);
    assert_eq!(client.claim_for(&id, &accounts.user1), 5_000);
    assert_eq!(client.claim(&id), 2_500);
    assert_eq!(tc.balance(&accounts.user1), 5_000);
    assert_eq!(tc.balance(&new_beneficiary), 2_500);
    assert_eq!(tc.balance(&contract_id), TOTAL - 7_500);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
}

#[test]
fn reassignment_rejects_unknown_tranche_and_completed_schedules() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    let err = client
        .try_reassign_beneficiary(&999, &accounts.user2)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::NotFound);

    let tranche_id = client.create_tranche_schedule(
        &accounts.user1,
        &token,
        &soroban_sdk::vec![
            &env,
            Tranche {
                unlock_at: 0,
                amount: TOTAL
            }
        ],
    );
    let err = client
        .try_reassign_beneficiary(&tranche_id, &accounts.user2)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::NotFound);

    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claim(&id), TOTAL);
    let err = client
        .try_reassign_beneficiary(&id, &accounts.user2)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

// -----------------------------------------------------------------------
// Issue #609: Schedule Creation, Cancellation, and Authorization Matrix Tests
// -----------------------------------------------------------------------

#[test]
fn cancel_schedule_rejects_already_terminal_or_completed_schedules() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);

    // Advance time and complete the schedule
    env.ledger().set_timestamp(START + DURATION + 1);
    client.claim(&id);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);

    // Attempting to revoke an already completed schedule must return an error
    let err = client.try_revoke(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
}

#[test]
fn cancel_schedule_rejects_non_existent_schedule_id() {
    let (_env, _token, _tc, _contract_id, client, _accounts) = setup!();
    let fake_id = 9999u64;

    let err = client.try_revoke(&fake_id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::NotFound);
}

#[test]
fn claim_authorization_failure_preserves_balances_and_claimed_amounts() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    env.ledger().set_timestamp(START + DURATION);

    let initial_beneficiary_balance = tc.balance(beneficiary);
    let initial_contract_balance = tc.balance(&contract_id);

    // If an unauthorized claim attempt happens or fails due to invalid parameters/state,
    // state preservation assertions hold:
    assert_eq!(tc.balance(beneficiary), initial_beneficiary_balance);
    assert_eq!(tc.balance(&contract_id), initial_contract_balance);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
}

// ---------------------------------------------------------------------------
// refund_revoked — return the unvested remainder to the funder
// ---------------------------------------------------------------------------

#[test]
fn refund_revoked_returns_unvested_remainder_to_funder() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // floor(10_000 * 2_000 / 3_000) = 6_666 vested at revocation.
    env.ledger().set_timestamp(START + CLIFF + 2_000);
    client.revoke(&id);

    assert_eq!(client.refund_revoked(&id), TOTAL - 6_666);
    let emitted_events = env.events().all().filter_by_contract(&contract_id);
    let expected_events = std::vec![events::RevokedRefunded {
        schedule_id: id,
        funder: accounts.deployer.clone(),
        amount: TOTAL - 6_666,
    }
    .to_xdr(&env, &contract_id)];
    assert_eq!(emitted_events.events(), expected_events.as_slice());
    assert_eq!(tc.balance(&accounts.deployer), TOTAL - 6_666);
    assert_eq!(tc.balance(&contract_id), 6_666);

    // The beneficiary's frozen entitlement is untouched by the refund.
    assert_eq!(client.claimable(&id), 6_666);
    assert_eq!(client.claim(&id), 6_666);
    assert_eq!(tc.balance(&accounts.user1), 6_666);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
}

#[test]
fn refund_revoked_before_cliff_refunds_the_full_allocation() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + CLIFF - 1);
    client.revoke(&id);

    assert_eq!(client.refund_revoked(&id), TOTAL);
    assert_eq!(tc.balance(&accounts.deployer), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.claimable(&id), 0);
}

#[test]
fn refund_revoked_is_zero_when_fully_vested_at_revocation() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    // Fully vested but unclaimed, so the stored status still allows revoke.
    env.ledger().set_timestamp(START + DURATION);
    client.revoke(&id);
    assert_eq!(client.get_schedule(&id).revoked_vested, Some(TOTAL));

    assert_eq!(client.refund_revoked(&id), 0);
    assert!(env
        .events()
        .all()
        .filter_by_contract(&contract_id)
        .events()
        .is_empty());
    assert_eq!(tc.balance(&accounts.deployer), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);
    assert_eq!(client.claim(&id), TOTAL);
}

#[test]
fn refund_revoked_repeated_calls_never_transfer_twice() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + CLIFF + 1_000);
    client.revoke(&id);

    assert_eq!(client.refund_revoked(&id), TOTAL - 3_333);
    env.ledger().set_timestamp(START + DURATION + 100);
    for _ in 0..3 {
        assert_eq!(client.refund_revoked(&id), 0);
        assert!(env
            .events()
            .all()
            .filter_by_contract(&contract_id)
            .events()
            .is_empty());
    }
    assert_eq!(tc.balance(&accounts.deployer), TOTAL - 3_333);
    assert_eq!(tc.balance(&contract_id), 3_333);
}

#[test]
fn refund_revoked_failed_transfer_records_nothing() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);
    env.ledger().set_timestamp(START + CLIFF - 1);
    client.revoke(&id);
    // Drain custody so the refund transfer is rejected.
    tc.transfer(&contract_id, &accounts.user2, &TOTAL);

    assert_eq!(
        client.try_refund_revoked(&id).unwrap_err().unwrap(),
        ForgeError::TokenTransferFailed
    );
    assert_eq!(tc.balance(&accounts.deployer), 0);

    // Nothing was recorded, so the full remainder is still refundable.
    StellarAssetClient::new(&env, &token).mint(&contract_id, &TOTAL);
    assert_eq!(client.refund_revoked(&id), TOTAL);
    assert_eq!(tc.balance(&accounts.deployer), TOTAL);
}

#[test]
fn refund_revoked_requires_a_revoked_linear_schedule() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let id = create(&client, &token, &accounts);

    // Locked and Vesting schedules have not been revoked.
    assert_eq!(
        client.try_refund_revoked(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    env.ledger().set_timestamp(START + CLIFF + 1);
    assert_eq!(
        client.try_refund_revoked(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );

    // A completed schedule cannot be revoked, so it cannot be refunded.
    env.ledger().set_timestamp(START + DURATION);
    assert_eq!(client.claim(&id), TOTAL);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
    assert_eq!(
        client.try_refund_revoked(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );

    assert_eq!(
        client.try_refund_revoked(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    let tranches = soroban_sdk::Vec::from_array(
        &env,
        [Tranche {
            unlock_at: 0,
            amount: 1_000,
        }],
    );
    let tranche_id = client.create_tranche_schedule(&accounts.user1, &token, &tranches);
    assert_eq!(
        client.try_refund_revoked(&tranche_id).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(tc.balance(&accounts.deployer), 0);
    assert_eq!(tc.balance(&contract_id), 0);
}
// Batch schedule creation (issue #641)
// -------------------------------------------------------------------

/// A batch of identical linear schedules is created with sequential ids and
/// the aggregate token amount is transferred from the funder in one atomic
/// step.
#[test]
fn create_schedules_batch_succeeds_and_uses_sequential_ids() {
    let (env, token, _tc, _cid, client, accounts) = setup!();
    // Fund the deployer with enough tokens for the batch.
    StellarAssetClient::new(&env, &token).mint(&accounts.deployer, &(TOTAL * 2));

    let args = CreateScheduleArgs {
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        cliff: CLIFF,
        duration: DURATION,
    };
    let mut batch = SorobanVec::new(&env);
    batch.push_back(args.clone());
    batch.push_back(args.clone());
    let ids = client.create_schedules_batch(&accounts.deployer, &batch);

    assert_eq!(ids.len(), 2);
    assert_eq!(ids.get(0).unwrap(), 1);
    assert_eq!(ids.get(1).unwrap(), 2);

    let s1 = client.get_schedule(&ids.get(0).unwrap());
    let s2 = client.get_schedule(&ids.get(1).unwrap());
    assert_eq!(s1.beneficiary, accounts.user1);
    assert_eq!(s2.beneficiary, accounts.user1);
    assert_eq!(s1.total_amount, TOTAL);
    assert_eq!(s2.total_amount, TOTAL);

    // Aggregate custody: the contract received 2 * TOTAL on top of the setup mint.
    assert_eq!(_tc.balance(&_cid), TOTAL * 3);
}

/// The aggregate token amount across the batch is transferred from the funder
/// to the contract custody in one atomic transfer.
#[test]
fn create_schedules_batch_transfers_aggregate_amount() {
    let (env, token, tc, _cid, client, accounts) = setup!();
    // Fund the deployer with enough tokens for the batch.
    StellarAssetClient::new(&env, &token).mint(&accounts.deployer, &(TOTAL * 3));

    let a1 = CreateScheduleArgs {
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        cliff: CLIFF,
        duration: DURATION,
    };
    let a2 = CreateScheduleArgs {
        beneficiary: accounts.user2.clone(),
        token: token.clone(),
        total_amount: TOTAL * 2,
        cliff: CLIFF,
        duration: DURATION,
    };
    let before = tc.balance(&accounts.deployer);
    let mut batch2 = SorobanVec::new(&env);
    batch2.push_back(a1);
    batch2.push_back(a2);
    let ids = client.create_schedules_batch(&accounts.deployer, &batch2);
    let after = tc.balance(&accounts.deployer);

    assert_eq!(before - after, TOTAL + TOTAL * 2);
    assert_eq!(tc.balance(&_cid), TOTAL * 4);
    assert_eq!(ids.len(), 2);
}

/// A batch that mixes tokens is rejected as invalid and nothing is written.
#[test]
fn create_schedules_batch_rejects_mixed_tokens() {
    let (env, _token, _tc, _cid, client, accounts) = setup!();

    let token_a = _token.clone();
    let token_b = Address::generate(&env);

    let a1 = CreateScheduleArgs {
        beneficiary: accounts.user1.clone(),
        token: token_a.clone(),
        total_amount: TOTAL,
        cliff: CLIFF,
        duration: DURATION,
    };
    let a2 = CreateScheduleArgs {
        beneficiary: accounts.user2.clone(),
        token: token_b,
        total_amount: TOTAL,
        cliff: CLIFF,
        duration: DURATION,
    };

    let mut batch5 = SorobanVec::new(&env);
    batch5.push_back(a1);
    batch5.push_back(a2);
    let err = client
        .try_create_schedules_batch(&accounts.deployer, &batch5)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    // No schedule records were written.
    assert_eq!(
        env.as_contract(&_cid, || env
            .storage()
            .instance()
            .get(&DataKey::Count)
            .unwrap_or(0)),
        0
    );
}

/// An empty batch is rejected.
#[test]
fn create_schedules_batch_rejects_empty() {
    let (env, _token, _tc, _cid, client, accounts) = setup!();
    let err = client
        .try_create_schedules_batch(&accounts.deployer, &SorobanVec::new(&env))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

/// A batch longer than 10 schedules is rejected.
#[test]
fn create_schedules_batch_rejects_too_many() {
    let (_env, token, _tc, _cid, client, accounts) = setup!();

    let mut schedules = SorobanVec::new(&_env);
    for _ in 0..11 {
        schedules.push_back(CreateScheduleArgs {
            beneficiary: accounts.user1.clone(),
            token: token.clone(),
            total_amount: TOTAL,
            cliff: CLIFF,
            duration: DURATION,
        });
    }

    let err = client
        .try_create_schedules_batch(&accounts.deployer, &schedules)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

/// A batch with an invalid schedule (zero amount) is rejected entirely and
/// no schedule records are written.
#[test]
fn create_schedules_batch_rolls_back_on_invalid_entry() {
    let (env, token, _tc, cid, client, accounts) = setup!();

    let good = CreateScheduleArgs {
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        cliff: CLIFF,
        duration: DURATION,
    };
    let bad = CreateScheduleArgs {
        beneficiary: accounts.user2.clone(),
        token: token.clone(),
        total_amount: 0,
        cliff: CLIFF,
        duration: DURATION,
    };

    let mut batch3 = SorobanVec::new(&env);
    batch3.push_back(good);
    batch3.push_back(bad);
    let err = client
        .try_create_schedules_batch(&accounts.deployer, &batch3)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    // The good schedule was not written either.
    assert_eq!(
        env.as_contract(&cid, || env
            .storage()
            .instance()
            .get(&DataKey::Count)
            .unwrap_or(0)),
        0
    );
    assert_eq!(_tc.balance(&cid), TOTAL);
}

/// Each schedule in a successful batch emits a ScheduleCreated event.
#[test]
fn create_schedules_batch_emits_creation_events() {
    let (env, token, _tc, cid, client, accounts) = setup!();
    // Fund the deployer with enough tokens for the batch.
    StellarAssetClient::new(&env, &token).mint(&accounts.deployer, &(TOTAL * 3));

    let a1 = CreateScheduleArgs {
        beneficiary: accounts.user1.clone(),
        token: token.clone(),
        total_amount: TOTAL,
        cliff: CLIFF,
        duration: DURATION,
    };
    let a2 = CreateScheduleArgs {
        beneficiary: accounts.user2.clone(),
        token: token.clone(),
        total_amount: TOTAL * 2,
        cliff: CLIFF,
        duration: DURATION,
    };
    let mut batch4 = SorobanVec::new(&env);
    batch4.push_back(a1);
    batch4.push_back(a2);
    let ids = client.create_schedules_batch(&accounts.deployer, &batch4);

    let emitted = env.events().all().filter_by_contract(&cid);
    let expected = std::vec![
        events::ScheduleCreated {
            schedule_id: ids.get(0).unwrap(),
            beneficiary: accounts.user1.clone(),
            total_amount: TOTAL,
            cliff: CLIFF,
            duration: DURATION,
        }
        .to_xdr(&env, &cid),
        events::ScheduleCreated {
            schedule_id: ids.get(1).unwrap(),
            beneficiary: accounts.user2.clone(),
            total_amount: TOTAL * 2,
            cliff: CLIFF,
            duration: DURATION,
        }
        .to_xdr(&env, &cid),
    ];
    assert_eq!(emitted.events(), expected.as_slice());
}
