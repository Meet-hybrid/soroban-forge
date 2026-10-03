# Multi-Signature Wallet Contract Reference

- **Crate**: `crates/multi-sig-wallet`
- **Package**: `soroban-forge-multi-sig-wallet`
- **Client**: `SorobanForgeMultiSigWalletClient`
- **Contract Type**: `MultiSigWallet`
- **Trait**: `SorobanForgeMultiSigWallet`

---

## Overview

The Multi-Signature Wallet provides M-of-N quorum governance over arbitrary transactions, typed cross-contract invocations, token custody, and rate-limited withdrawals. It features built-in self-governance over its owner set and threshold, single-rejection vetoes, transaction expiration deadlines, and per-token rolling withdrawal rate limits.

### Key Capabilities
- **Flexible Transaction Payloads**: Supports opaque raw payloads (`TxKind::Opaque`), typed cross-contract calls (`TxKind::Call`), and native token withdrawals (`TxKind::Withdrawal`).
- **Owner Self-Governance**: Owners propose and execute changes to add owners, remove owners, or adjust approval thresholds through the standard threshold approval process.
- **Single-Rejection Veto Policy**: Any single owner can formally reject a pending transaction, immediately vetoing it into a permanent `Rejected` state.
- **Rolling Withdrawal Limits**: Configurable per-token rate limits (`amount` per `window_seconds`) that admit withdrawals at submission time.
- **Transaction Expiry**: Optional ledger timestamp deadlines that mark unapproved transactions as `Expired`.
- **Confirmation Scrubbing**: When an owner is removed, their signatures are automatically purged from all pending transactions.

---

## State Machine & Lifecycle

