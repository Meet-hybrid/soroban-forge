
// Copyright 2024 Meet Hybrid
// SPDX-License-Identifier: Apache-2.0

use soroban_sdk::{contractimport, contractimpl, contracttype, env, symbol_short, vec, Address, Bytes, Duration, Symbol, Vec};
use soroban_sdk::token::Client as TokenClient;
use soroban_sdk::token::TransferArgs;

mod events;
use events::*;

/// Escrow contract state
#[derive(Clone, Debug, PartialEq, Eq)]
#[contracttype]
pub enum EscrowState {
    Active,
    Disputed,
    Refunded,
    Resolved,
}

/// Escrow contract error types
#[derive(Clone, Debug, PartialEq, Eq, soroban_sdk::error::Error)]
#[contracterror]
pub enum ForgeError {
    /// Deadline not yet reached
    DeadlineNotReached,
    /// Deadline already passed but escrow is not in refundable state
    DeadlineReachedButNotRefundable,
    /// Escrow already in terminal state
    AlreadyTerminal,
    /// Escrow is disputed and cannot be refunded
    DisputedEscrow,
    /// Invalid escrow ID
    InvalidEscrowId,
    /// Insufficient funds
    InsufficientFunds,
    /// Token transfer failed
    TokenTransferFailed,
}

/// Escrow contract storage
#[derive(Clone, Debug, PartialEq, Eq)]
#[contracttype]
pub struct Escrow {
    pub buyer: Address,
    pub seller: Address,
    pub token: Address,
    pub amount: i128,
    pub deadline: i64,
    pub state: EscrowState,
    pub dispute_resolution: Option<Address>,
}

/// Escrow contract client
#[contractclient]
pub struct Client;

impl Client {
    /// Deposit funds into escrow
    pub fn deposit(env: &env::Env, escrow_id: Bytes, buyer: Address, seller: Address, token: Address, amount: i128, deadline: i64) {
        // ... existing implementation ...
    }

    /// Dispute the escrow
    pub fn dispute(env: &env::Env, escrow_id: Bytes, disputer: Address) {
        // ... existing implementation ...
    }

    /// Resolve a dispute
    pub fn resolve(env: &env::Env, escrow_id: Bytes, resolver: Address, refund: bool) {
        // ... existing implementation ...
    }

    /// Party-gated refund (unchanged)
    pub fn refund(env: &env::Env, escrow_id: Bytes, refunding_party: Address) {
        let escrow = Self::get_escrow(env, &escrow_id);
        Self::validate_refund_conditions(env, &escrow);

        // Check deadline for party refunds
        if env.ledger().timestamp() < escrow.deadline {
            env.bail(ForgeError::DeadlineNotReached);
        }

        // ... existing refund logic ...
    }

    /// Permissionless expiry refund
    pub fn refund_expired(env: &env::Env, escrow_id: Bytes, caller: Address) {
        let escrow = Self::get_escrow(env, &escrow_id);
        Self::validate_refund_expired_conditions(env, &escrow);

        // Transfer funds to buyer
        TokenClient::new(env, &escrow.token)
            .transfer(&TransferArgs {
                to: escrow.buyer,
                amount: escrow.amount,
                ..Default::default()
            })
            .expect("Token transfer failed");

        // Update escrow state
        Self::set_escrow(env, &escrow_id, &escrow.with_state(EscrowState::Refunded));

        // Emit event
        env.events().publish(
            Event::RefundExpired(RefundExpired {
                escrow_id,
                buyer: escrow.buyer,
                amount: escrow.amount,
                timestamp: env.ledger().timestamp(),
            }),
        );
    }

    // ... helper methods ...
}

#[contractimpl]
impl Client {
    // ... existing implementations ...

    /// Validate conditions for permissionless expiry refund
    fn validate_refund_expired_conditions(env: &env::Env, escrow: &Escrow) {
        let now = env.ledger().timestamp();

        // Deadline check
        if now <= escrow.deadline {
            env.bail(ForgeError::DeadlineNotReached);
        }

        // State check
        match escrow.state {
            EscrowState::Active => (),
            EscrowState::Disputed => env.bail(ForgeError::DisputedEscrow),
            _ => env.bail(ForgeError::AlreadyTerminal),
        }
    }

    // ... other implementations ...
}