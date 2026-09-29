
// Copyright 2024 Meet Hybrid
// SPDX-License-Identifier: Apache-2.0

use soroban_sdk::{contracttype, Bytes, Address};

/// Escrow contract events
#[contracttype]
pub enum Event {
    Deposit(Deposit),
    Dispute(Dispute),
    Resolve(Resolve),
    Refund(Refund),
    RefundExpired(RefundExpired),
}

/// Deposit event
#[contracttype]
pub struct Deposit {
    pub escrow_id: Bytes,
    pub buyer: Address,
    pub seller: Address,
    pub amount: i128,
    pub deadline: i64,
}

/// Dispute event
#[contracttype]
pub struct Dispute {
    pub escrow_id: Bytes,
    pub disputer: Address,
}

/// Resolve event
#[contracttype]
pub struct Resolve {
    pub escrow_id: Bytes,
    pub resolver: Address,
    pub refund: bool,
}

/// Party refund event
#[contracttype]
pub struct Refund {
    pub escrow_id: Bytes,
    pub refunding_party: Address,
    pub amount: i128,
}

/// Permissionless expiry refund event
#[contracttype]
pub struct RefundExpired {
    pub escrow_id: Bytes,
    pub buyer: Address,
    pub amount: i128,
    pub timestamp: i64,
}