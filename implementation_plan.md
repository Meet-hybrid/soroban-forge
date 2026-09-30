# Implementation Plan: Safe Owner Rotation with Pending-Queue Revalidation (Issue #247)

## 1. Problem Overview & Scope
`crates/multi-sig-wallet` requires robust, production-ready owner rotation and threshold governance that preserves all smart contract invariants:
1. Entrypoints `add_owner(submitter, new_owner)`, `remove_owner(submitter, existing_owner)`, and `set_threshold(submitter, new_threshold)` are self-governed via the wallet's typed governance transaction queue (`TxKind::AddOwner`, `TxKind::RemoveOwner`, `TxKind::SetThreshold`).
2. Revalidation & queue effects:
   - When an owner is removed, all their confirmations on any `Pending` transaction must be purged from persistent storage (`clear_confirmations_of`).
   - If a pending transaction had met threshold, but losing the removed owner's confirmation drops its count below threshold, execution is rejected until new owners confirm.
   - If all remaining confirmers are still valid owners and count meets threshold, it remains executable.
   - Newly added owners can immediately confirm existing pending transactions.
   - Threshold invariants (`1 <= threshold <= owners.len()`) must hold at every state transition. A removal that would leave `threshold > remaining_owners` must be rejected at execution.
3. Soroban contract events:
   - Emit `OwnerAdded`, `OwnerRemoved`, and `ThresholdChanged` upon execution of the corresponding governance transaction.
   - Verify payload parity with contract views.
4. Comprehensive test coverage:
   - Unit tests covering edge cases: empty queue, dropping below threshold, remaining above threshold, immediate new owner confirmations, event parity.
   - Negative authorization tests in `authz.rs` for `add_owner`, `remove_owner`, `set_threshold`.
   - Property tests in `props.rs` verifying `1 <= threshold <= owners.len()` across arbitrary rotation sequences.
5. Documentation updates in `docs/contracts/multi-sig-wallet.md` and `docs/FEATURE-STATUS.md`.

---

## 2. Implementation Steps

### Step 1: Contract Events in `crates/multi-sig-wallet/src/lib.rs`
Define contract events in `mod events`:
```rust
#[contractevent]
pub struct OwnerAdded {
    #[topic]
    pub tx_id: u64,
    pub owner: Address,
    pub total_owners: u32,
}

#[contractevent]
pub struct OwnerRemoved {
    #[topic]
    pub tx_id: u64,
    pub owner: Address,
    pub total_owners: u32,
}

#[contractevent]
pub struct ThresholdChanged {
    #[topic]
    pub tx_id: u64,
    pub old_threshold: u32,
    pub new_threshold: u32,
}
```
Expose event publishers:
- `events::owner_added(env, tx_id, owner, total_owners)`
- `events::owner_removed(env, tx_id, owner, total_owners)`
- `events::threshold_changed(env, tx_id, old_threshold, new_threshold)`

In `execute` method of `MultiSigWallet`:
- On `TxKind::AddOwner(new_owner)`: Emit `events::owner_added(&env, tx_id, new_owner, Self::owner_count(&env));`
- On `TxKind::RemoveOwner(existing_owner)`: Emit `events::owner_removed(&env, tx_id, existing_owner, Self::owner_count(&env));`
- On `TxKind::SetThreshold(new_threshold)`: Read `old_threshold` before applying, then emit `events::threshold_changed(&env, tx_id, old_threshold, *new_threshold);`

### Step 2: Negative Authorization Tests in `crates/multi-sig-wallet/src/authz.rs`
Add full negative authorization coverage for:
1. `add_owner`:
   - `add_owner_accepts_owner_signature`
   - `add_owner_rejects_wrong_owner_signature`
   - `add_owner_rejects_signature_replayed_for_another_candidate`
   - `blank_envelope_aborts_add_owner_without_writing_tx`
   - `add_owner_authorization_tree_is_the_owner_entrypoint_frame`
2. `remove_owner`:
   - `remove_owner_accepts_owner_signature`
   - `remove_owner_rejects_wrong_owner_signature`
   - `remove_owner_rejects_signature_replayed_for_another_target`
   - `blank_envelope_aborts_remove_owner_without_writing_tx`
   - `remove_owner_authorization_tree_is_the_owner_entrypoint_frame`
3. `set_threshold`:
   - `set_threshold_accepts_owner_signature`
   - `set_threshold_rejects_wrong_owner_signature`
   - `set_threshold_rejects_signature_replayed_for_another_threshold`
   - `blank_envelope_aborts_set_threshold_without_writing_tx`
   - `set_threshold_authorization_tree_is_the_owner_entrypoint_frame`

### Step 3: Specific Unit Tests in `crates/multi-sig-wallet/src/lib.rs`
1. `rotation_with_empty_queue_succeeds`: Removal of owner when no other pending tx exists.
2. `removal_drops_threshold_met_pending_tx_below_threshold`: Tx had 2/2 confirmations; one confirmer is removed; tx drops to 1/2 and fails execution until a third owner confirms.
3. `removal_keeps_threshold_met_pending_tx_executable_if_confirmers_remain`: Tx had 2/2 confirmations from A and B; owner C is removed; tx remains 2/2 and executes cleanly.
4. `new_owner_can_confirm_preexisting_pending_tx`: Pending tx submitted before rotation; new owner is added; new owner confirms and helps reach threshold.
5. `rotation_events_payload_parity`: Verify emitted `OwnerAdded`, `OwnerRemoved`, and `ThresholdChanged` topics, data payload, and parity with `get_owners()` and `get_threshold()`.

### Step 4: Property Tests in `crates/multi-sig-wallet/src/props.rs`
Add `P4 — Rotation preserves threshold and owner invariants`:
- Generate arbitrary valid and invalid sequences of `AddOwner`, `RemoveOwner`, `SetThreshold`.
- Assert that at every step:
  `1 <= client.get_threshold() <= client.get_owners().len()`
- Verify that invalid transitions (threshold > owners or threshold == 0) are rejected and state is untouched.

### Step 5: Documentation Updates
- Update `docs/contracts/multi-sig-wallet.md` with:
  - Full explanation of safe rotation lifecycle and queue revalidation.
  - Events emitted (`OwnerAdded`, `OwnerRemoved`, `ThresholdChanged`).
- Update `docs/FEATURE-STATUS.md` reflecting tests count and status.

### Step 6: Verification
Execute:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test -p soroban-forge-multi-sig-wallet --all-targets --locked`
- `cargo test --workspace --all-targets --locked`
