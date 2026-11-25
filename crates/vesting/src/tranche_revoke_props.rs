//! Property-based tests for tranche schedule revocation.
//!
//! Exercises random tranche unlock tables, monotonic pre- and post-revocation
//! claim schedules, revocation timestamps (before, between, at, and after
//! unlocks), and reassignment interactions (both rejected tranche
//! reassignments and interleaved linear reassignments sharing the same
//! contract and token pool) under fixed [`RngSeed`] values for deterministic
//! CI execution.
//!
//! Core invariants verified across every generated scenario:
//! - **Conservation**: `claimed + refunded_amount + still_claimable == total_amount`
//!   at every observation point after revocation, and
//!   `beneficiary + funder + contract == total_amount`.
//! - **Nothing claimable after refund/drain**: locked tranches refunded to the
//!   funder never become claimable as time advances to `FAR_FUTURE`, and
//!   attempting to claim locked tranches after revocation is rejected with
//!   `ForgeError::InvalidInput`.
//! - **Error discipline**: double-revoke and revoking once every tranche has
//!   already unlocked are rejected with `ForgeError::InvalidInput`; calling
//!   `reassign_beneficiary` on a tranche id is rejected with
//!   `ForgeError::NotFound` without altering state.

use crate::{SorobanForgeVestingClient, Tranche, Vesting, VestingStatus};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::vec;
use std::vec::Vec;

const START: u64 = 1_000_000;
const MAX_AMOUNT: i128 = 1_000_000_000_000;
const MAX_DURATION: u64 = 10 * 365 * 24 * 3600;
const MAX_TRANCHES: u32 = 32;
const FAR_FUTURE: u64 = u64::MAX / 2;

struct World {
    env: Env,
    token: Address,
    vesting: Address,
    funder: Address,
    beneficiary: Address,
}

fn setup_world() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let vesting = env.register(Vesting, ());

    World {
        token,
        vesting,
        funder: Address::generate(&env),
        beneficiary: Address::generate(&env),
        env,
    }
}

impl World {
    fn client(&self) -> SorobanForgeVestingClient<'_> {
        SorobanForgeVestingClient::new(&self.env, &self.vesting)
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }

    fn mint_to_contract(&self, amount: i128) {
        StellarAssetClient::new(&self.env, &self.token).mint(&self.vesting, &amount);
    }

    fn balance(&self, account: &Address) -> i128 {
        self.token_client().balance(account)
    }
}

fn to_tranche_vec(env: &Env, table: &[(u64, i128)]) -> soroban_sdk::Vec<Tranche> {
    let mut tranches = soroban_sdk::Vec::new(env);
    for &(unlock_at, amount) in table {
        tranches.push_back(Tranche { unlock_at, amount });
    }
    tranches
}

fn table_total(table: &[(u64, i128)]) -> i128 {
    table.iter().map(|(_, amount)| *amount).sum()
}

fn expected_unlocked_at(table: &[(u64, i128)], elapsed: u64) -> i128 {
    table
        .iter()
        .filter(|(unlock_at, _)| *unlock_at <= elapsed)
        .map(|(_, amount)| *amount)
        .sum()
}

fn strictly_increasing_offsets(n: usize, lo: u64, hi: u64) -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(lo..=hi, n).prop_map(|mut offsets| {
        offsets.sort_unstable();
        for i in 1..offsets.len() {
            if offsets[i] <= offsets[i - 1] {
                offsets[i] = offsets[i - 1].saturating_add(1);
            }
        }
        offsets
    })
}

fn positive_amounts(n: usize, cap: i128) -> impl Strategy<Value = Vec<i128>> {
    prop::collection::vec(1i128..=cap, n)
}

/// Generate random valid tranche tables including TGE (`unlock_at == 0`),
/// arbitrary positive gaps, and unreachable `u64::MAX` tail tranches.
fn arb_tranche_table() -> impl Strategy<Value = Vec<(u64, i128)>> {
    prop_oneof![
        // Standard strictly increasing offsets in 0..=MAX_DURATION.
        4 => (1u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = MAX_AMOUNT / n as i128;
            strictly_increasing_offsets(n as usize, 0, MAX_DURATION).prop_flat_map(move |offsets| {
                positive_amounts(n as usize, cap)
                    .prop_map(move |amounts| offsets.clone().into_iter().zip(amounts).collect())
            })
        }),
        // Explicit TGE tranche at offset 0 followed by later tranches.
        2 => (2u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = MAX_AMOUNT / n as i128;
            strictly_increasing_offsets(n as usize - 1, 1, MAX_DURATION).prop_flat_map(
                move |offsets| {
                    positive_amounts(n as usize, cap).prop_map(move |amounts| {
                        let mut table = vec![(0u64, amounts[0])];
                        for (i, offset) in offsets.iter().enumerate() {
                            table.push((*offset, amounts[i + 1]));
                        }
                        table
                    })
                },
            )
        }),
        // Table ending with an unreachable u64::MAX tranche (always revocable).
        2 => (2u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = MAX_AMOUNT / n as i128;
            strictly_increasing_offsets(n as usize - 1, 0, MAX_DURATION).prop_flat_map(
                move |offsets| {
                    positive_amounts(n as usize, cap).prop_map(move |amounts| {
                        let mut table: Vec<(u64, i128)> = offsets
                            .iter()
                            .copied()
                            .zip(amounts.iter().copied())
                            .collect();
                        table.push((u64::MAX, *amounts.last().unwrap()));
                        table
                    })
                },
            )
        }),
    ]
}

/// Generate a monotonic timeline partitioned into pre-revocation claim offsets,
/// a revocation offset, and post-revocation claim offsets.
fn arb_timeline(table: &[(u64, i128)]) -> impl Strategy<Value = (Vec<u64>, u64, Vec<u64>)> {
    let reachable_max = table
        .iter()
        .filter(|(u, _)| *u != u64::MAX)
        .map(|(u, _)| *u)
        .max()
        .unwrap_or(0);
    let upper = reachable_max.saturating_add(1_000).min(MAX_DURATION * 2);

    // Choose revoke_offset across the full range (including before first,
    // exactly on a tranche boundary, between tranches, and past the last).
    let boundaries: Vec<u64> = table
        .iter()
        .filter(|(u, _)| *u != u64::MAX)
        .map(|(u, _)| *u)
        .collect();

    let revoke_strategy = if boundaries.is_empty() {
        (0u64..=upper).boxed()
    } else {
        prop_oneof![
            3 => 0u64..=upper,
            2 => prop::sample::select(boundaries),
        ]
        .boxed()
    };

    revoke_strategy.prop_flat_map(move |revoke_offset| {
        let pre_claims = prop::collection::vec(0u64..=revoke_offset, 0..=4).prop_map(|mut v| {
            v.sort_unstable();
            v
        });
        let post_upper = revoke_offset
            .saturating_add(upper.max(10_000))
            .min(FAR_FUTURE);
        let post_claims =
            prop::collection::vec(revoke_offset..=post_upper, 1..=5).prop_map(|mut v| {
                v.sort_unstable();
                v
            });
        (pre_claims, Just(revoke_offset), post_claims)
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x5452_414e_4348_4531u64),
        ..ProptestConfig::default()
    })]

    /// Random tranche layouts, monotonic claim times, revocation times, and
    /// attempted reassignments satisfy exact conservation (`claimed + refunded
    /// + still_claimable == total`), reject revoking after full unlock and
    /// double-revoking, and guarantee nothing is claimable after the refund
    /// once any unlocked tranches are drained.
    #[test]
    fn tranche_revocation_conservation_and_drain(
        (table, (pre_claim_offsets, revoke_offset, post_claim_offsets)) in
            arb_tranche_table().prop_flat_map(|t| {
                let timeline = arb_timeline(&t);
                (Just(t), timeline)
            }),
        try_reassign_before in any::<bool>(),
        try_reassign_after in any::<bool>(),
    ) {
        let w = setup_world();
        let client = w.client();
        let total = table_total(&table);
        w.mint_to_contract(total);

        let id = client.create_tranche_schedule(
            &w.funder,
            &w.beneficiary,
            &w.token,
            &to_tranche_vec(&w.env, &table),
        );

        let mut cumulative_claimed: i128 = 0;

        // Pre-revocation monotonic claims.
        for offset in pre_claim_offsets {
            w.env.ledger().set_timestamp(START.saturating_add(offset));
            let expected_claimable = expected_unlocked_at(&table, offset) - cumulative_claimed;
            let got_claimable = client.claimable(&id);
            prop_assert_eq!(got_claimable, expected_claimable);

            let paid = client.claim(&id);
            prop_assert_eq!(paid, expected_claimable);
            cumulative_claimed += paid;

            prop_assert_eq!(w.balance(&w.beneficiary), cumulative_claimed);
            prop_assert_eq!(w.balance(&w.vesting), total - cumulative_claimed);
        }

        // Reassignment on a tranche schedule must always be rejected without side effects.
        if try_reassign_before {
            let candidate = Address::generate(&w.env);
            let err = client
                .try_reassign_beneficiary(&id, &candidate)
                .unwrap_err()
                .unwrap();
            prop_assert_eq!(err, ForgeError::NotFound);
        }

        // Attempt revocation at `revoke_offset`.
        let revoke_time = START.saturating_add(revoke_offset);
        w.env.ledger().set_timestamp(revoke_time);
        let unlocked_at_revoke = expected_unlocked_at(&table, revoke_offset);

        if unlocked_at_revoke >= total {
            // Everything has already unlocked: revocation must be rejected.
            let err = client
                .try_revoke_tranche_schedule(&id)
                .unwrap_err()
                .unwrap();
            prop_assert_eq!(err, ForgeError::InvalidInput);

            let record = client.get_tranche_schedule(&id);
            prop_assert_eq!(record.revoked_unlocked, None);
            prop_assert_eq!(record.refunded_amount, 0);
            prop_assert_eq!(w.balance(&w.funder), 0);

            // Beneficiary can still claim any remaining unlocked balance to completion.
            let remaining = total - cumulative_claimed;
            prop_assert_eq!(client.claimable(&id), remaining);
            let final_paid = client.claim(&id);
            prop_assert_eq!(final_paid, remaining);
            prop_assert_eq!(client.get_status(&id), VestingStatus::Completed);
            prop_assert_eq!(w.balance(&w.beneficiary), total);
            prop_assert_eq!(w.balance(&w.vesting), 0);
            return Ok(());
        }

        // Revocation must succeed and atomically refund the locked remainder.
        let expected_refund = total - unlocked_at_revoke;
        let refunded = client.revoke_tranche_schedule(&id);
        prop_assert_eq!(refunded, expected_refund);

        // Record & status views must report revoked state and refunded amount.
        prop_assert_eq!(client.get_status(&id), VestingStatus::Revoked);
        let record = client.get_tranche_schedule(&id);
        prop_assert_eq!(record.status, VestingStatus::Revoked);
        prop_assert_eq!(record.revoked_unlocked, Some(unlocked_at_revoke));
        prop_assert_eq!(record.refunded_amount, expected_refund);
        prop_assert_eq!(record.claimed, cumulative_claimed);

        let still_claimable = client.claimable(&id);
        prop_assert_eq!(still_claimable, unlocked_at_revoke - cumulative_claimed);
        prop_assert_eq!(
            record.claimed + record.refunded_amount + still_claimable,
            total,
            "claimed + refunded + still_claimable must equal total right after revoke"
        );
        prop_assert_eq!(
            w.balance(&w.beneficiary) + w.balance(&w.funder) + w.balance(&w.vesting),
            total
        );
        prop_assert_eq!(w.balance(&w.funder), expected_refund);

        // Double-revoke is always rejected.
        let double_err = client
            .try_revoke_tranche_schedule(&id)
            .unwrap_err()
            .unwrap();
        prop_assert_eq!(double_err, ForgeError::InvalidInput);

        // Reassignment on a revoked tranche schedule is rejected with no state change.
        if try_reassign_after {
            let candidate = Address::generate(&w.env);
            let err = client
                .try_reassign_beneficiary(&id, &candidate)
                .unwrap_err()
                .unwrap();
            prop_assert_eq!(err, ForgeError::NotFound);
        }

        // Post-revocation monotonic claim attempts: unlocked portion stays
        // claimable once, and subsequent claims against locked tranches fail.
        for offset in post_claim_offsets {
            w.env.ledger().set_timestamp(START.saturating_add(offset));
            let expected_still = unlocked_at_revoke - cumulative_claimed;
            let got_still = client.claimable(&id);
            prop_assert_eq!(got_still, expected_still);
            prop_assert_eq!(
                cumulative_claimed + expected_refund + got_still,
                total,
                "conservation must hold before every post-revocation claim"
            );

            if expected_still > 0 {
                let paid = client.claim(&id);
                prop_assert_eq!(paid, expected_still);
                cumulative_claimed += paid;
            } else {
                // Claiming locked tranches after revocation is rejected.
                let err = client.try_claim(&id).unwrap_err().unwrap();
                prop_assert_eq!(err, ForgeError::InvalidInput);
            }

            let post_rec = client.get_tranche_schedule(&id);
            let post_claimable = client.claimable(&id);
            prop_assert_eq!(
                post_rec.claimed + post_rec.refunded_amount + post_claimable,
                total,
                "conservation must hold after every post-revocation claim"
            );
            prop_assert_eq!(
                w.balance(&w.beneficiary) + w.balance(&w.funder) + w.balance(&w.vesting),
                total
            );
        }

        // Advance to FAR_FUTURE: nothing further unlocks, nothing is claimable
        // after the refund and unlocked drain, and locked claims remain rejected.
        w.env.ledger().set_timestamp(FAR_FUTURE);
        let remaining_unlocked = unlocked_at_revoke - cumulative_claimed;
        prop_assert_eq!(client.claimable(&id), remaining_unlocked);
        if remaining_unlocked > 0 {
            let paid = client.claim(&id);
            prop_assert_eq!(paid, remaining_unlocked);
            cumulative_claimed += paid;
        }
        prop_assert_eq!(cumulative_claimed, unlocked_at_revoke);

        prop_assert_eq!(client.claimable(&id), 0);
        let locked_claim_err = client.try_claim(&id).unwrap_err().unwrap();
        prop_assert_eq!(locked_claim_err, ForgeError::InvalidInput);

        let final_rec = client.get_tranche_schedule(&id);
        prop_assert_eq!(final_rec.status, VestingStatus::Revoked);
        prop_assert_eq!(client.get_status(&id), VestingStatus::Revoked);
        prop_assert_eq!(final_rec.claimed, unlocked_at_revoke);
        prop_assert_eq!(final_rec.refunded_amount, expected_refund);
        prop_assert_eq!(final_rec.claimed + final_rec.refunded_amount, total);
        prop_assert_eq!(w.balance(&w.vesting), 0);
        prop_assert_eq!(w.balance(&w.beneficiary), unlocked_at_revoke);
        prop_assert_eq!(w.balance(&w.funder), expected_refund);
    }

    /// Interleaved linear beneficiary reassignments and tranche revocation in
    /// the same contract instance never cross-contaminate custody, ids, or
    /// claimable accounting.
    #[test]
    fn tranche_revocation_with_interleaved_linear_reassignments(
        (table, (_, revoke_offset, _)) in
            arb_tranche_table().prop_flat_map(|t| {
                let timeline = arb_timeline(&t);
                (Just(t), timeline)
            }),
        linear_total in 1_000i128..=1_000_000i128,
        linear_duration in 100u64..=100_000u64,
        reassign_step in 1u64..=50_000u64,
    ) {
        let w = setup_world();
        let client = w.client();
        let tranche_total = table_total(&table);
        let pool_total = tranche_total + linear_total;
        w.mint_to_contract(pool_total);

        let linear_beneficiary_1 = Address::generate(&w.env);
        let linear_beneficiary_2 = Address::generate(&w.env);
        let cliff = linear_duration / 4;

        let linear_id = client.create_schedule(
            &w.funder,
            &linear_beneficiary_1,
            &w.token,
            &linear_total,
            &cliff,
            &linear_duration,
        );
        let tranche_id = client.create_tranche_schedule(
            &w.funder,
            &w.beneficiary,
            &w.token,
            &to_tranche_vec(&w.env, &table),
        );

        // Perform a linear reassignment before or at the tranche revocation point.
        let t1 = START.saturating_add(reassign_step.min(revoke_offset));
        w.env.ledger().set_timestamp(t1);
        client.reassign_beneficiary(&linear_id, &linear_beneficiary_2);

        // Attempting to reassign the tranche schedule fails cleanly.
        prop_assert_eq!(
            client
                .try_reassign_beneficiary(&tranche_id, &linear_beneficiary_2)
                .unwrap_err()
                .unwrap(),
            ForgeError::NotFound
        );

        // Revoke tranche schedule at revoke_offset.
        w.env.ledger().set_timestamp(START.saturating_add(revoke_offset));
        let unlocked = expected_unlocked_at(&table, revoke_offset);
        if unlocked < tranche_total {
            let refunded = client.revoke_tranche_schedule(&tranche_id);
            prop_assert_eq!(refunded, tranche_total - unlocked);
            let rec = client.get_tranche_schedule(&tranche_id);
            prop_assert_eq!(
                rec.claimed + rec.refunded_amount + client.claimable(&tranche_id),
                tranche_total
            );
            if unlocked > 0 {
                prop_assert_eq!(client.claim(&tranche_id), unlocked);
            }
            prop_assert_eq!(client.claimable(&tranche_id), 0);
            prop_assert_eq!(
                client.try_claim(&tranche_id).unwrap_err().unwrap(),
                ForgeError::InvalidInput
            );
        } else {
            prop_assert_eq!(
                client
                    .try_revoke_tranche_schedule(&tranche_id)
                    .unwrap_err()
                    .unwrap(),
                ForgeError::InvalidInput
            );
            prop_assert_eq!(client.claim(&tranche_id), tranche_total);
        }

        // Advance past linear duration and drain both linear beneficiaries.
        w.env
            .ledger()
            .set_timestamp(START.saturating_add(revoke_offset.max(linear_duration).saturating_add(1)));
        let _ = client.claim_for(&linear_id, &linear_beneficiary_1);
        let _ = client.claim_for(&linear_id, &linear_beneficiary_2);

        // Whole-pool conservation across all participants.
        let total_held = w.balance(&w.funder)
            + w.balance(&w.beneficiary)
            + w.balance(&linear_beneficiary_1)
            + w.balance(&linear_beneficiary_2)
            + w.balance(&w.vesting);
        prop_assert_eq!(total_held, pool_total);
        prop_assert_eq!(w.balance(&w.vesting), 0);
    }
}
