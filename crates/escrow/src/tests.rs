
// Copyright 2024 Meet Hybrid
// SPDX-License-Identifier: Apache-2.0

use soroban_sdk::{testutils::*, Address, Bytes, Duration};
use super::*;

#[test]
fn test_refund_expired_happy_path() {
    let env = Env::default();
    let contract_id = env.register_contract(None, Client {});
    let client = Client::new(&env, &contract_id);

    let (buyer, seller) = (Address::generate(&env), Address::generate(&env));
    let token = Address::generate(&env);
    let escrow_id = Bytes::from_array(&env, &[1u8; 32]);

    // Setup escrow
    client.deposit(
        &env,
        escrow_id.clone(),
        buyer,
        seller,
        token,
        1000,
        env.ledger().timestamp() + 100,
    );

    // Fast-forward past deadline
    env.ledger().set_timestamp(env.ledger().timestamp() + 101);

    // Permissionless refund should succeed
    client.refund_expired(&env, escrow_id, Address::generate(&env));

    // Verify state
    let escrow = client.get_escrow(&env, &escrow_id);
    assert_eq!(escrow.state, EscrowState::Refunded);
}

#[test]
fn test_refund_expired_boundary_exactly_at_deadline() {
    let env = Env::default();
    let contract_id = env.register_contract(None, Client {});
    let client = Client::new(&env, &contract_id);

    let (buyer, seller) = (Address::generate(&env), Address::generate(&env));
    let token = Address::generate(&env);
    let escrow_id = Bytes::from_array(&env, &[2u8; 32]);

    // Setup escrow with exact deadline
    let deadline = env.ledger().timestamp() + 1;
    client.deposit(
        &env,
        escrow_id.clone(),
        buyer,
        seller,
        token,
        1000,
        deadline,
    );

    // At exact deadline - should fail
    env.expect_panic(ForgeError::DeadlineNotReached);
    client.refund_expired(&env, escrow_id.clone(), Address::generate(&env));

    // One ledger after deadline - should succeed
    env.ledger().set_timestamp(deadline + 1);
    client.refund_expired(&env, escrow_id, Address::generate(&env));
}

#[test]
fn test_refund_expired_disputed_escrow() {
    let env = Env::default();
    let contract_id = env.register_contract(None, Client {});
    let client = Client::new(&env, &contract_id);

    let (buyer, seller, disputer) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    let token = Address::generate(&env);
    let escrow_id = Bytes::from_array(&env, &[3u8; 32]);

    // Setup and dispute escrow
    client.deposit(
        &env,
        escrow_id.clone(),
        buyer,
        seller,
        token,
        1000,
        env.ledger().timestamp() + 100,
    );
    client.dispute(&env, escrow_id.clone(), disputer);

    // Fast-forward past deadline
    env.ledger().set_timestamp(env.ledger().timestamp() + 101);

    // Permissionless refund should fail for disputed escrow
    env.expect_panic(ForgeError::DisputedEscrow);
    client.refund_expired(&env, escrow_id, Address::generate(&env));
}

#[test]
fn test_refund_expired_pre_deadline() {
    let env = Env::default();
    let contract_id = env.register_contract(None, Client {});
    let client = Client::new(&env, &contract_id);

    let (buyer, seller) = (Address::generate(&env), Address::generate(&env));
    let token = Address::generate(&env);
    let escrow_id = Bytes::from_array(&env, &[4u8; 32]);

    // Setup escrow
    client.deposit(
        &env,
        escrow_id.clone(),
        buyer,
        seller,
        token,
        1000,
        env.ledger().timestamp() + 100,
    );

    // Before deadline - should fail
    env.expect_panic(ForgeError::DeadlineNotReached);
    client.refund_expired(&env, escrow_id, Address::generate(&env));
}

#[test]
fn test_refund_expired_event_emission() {
    let env = Env::default();
    let contract_id = env.register_contract(None, Client {});
    let client = Client::new(&env, &contract_id);

    let (buyer, seller) = (Address::generate(&env), Address::generate(&env));
    let token = Address::generate(&env);
    let escrow_id = Bytes::from_array(&env, &[5u8; 32]);

    // Setup escrow
    client.deposit(
        &env,
        escrow_id.clone(),
        buyer,
        seller,
        token,
        1000,
        env.ledger().timestamp() + 100,
    );

    // Fast-forward past deadline
    env.ledger().set_timestamp(env.ledger().timestamp() + 101);

    // Verify event emission
    let events = client.refund_expired(&env, escrow_id, Address::generate(&env));
    assert!(events.iter().any(|e| matches!(e, Event::RefundExpired(_))));
}