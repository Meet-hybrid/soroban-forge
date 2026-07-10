# DAO Governance Contract

On-chain proposal system with voting, quorum, and execution delay.

## Interface

```rust
fn create_proposal(proposer, title, description, start_time, end_time) -> Result<u64, ForgeError>
fn vote(proposal_id, voter, support, weight) -> Result<(), ForgeError>
fn execute(proposal_id) -> Result<bool, ForgeError>
fn cancel(proposal_id) -> Result<bool, ForgeError>
fn get_proposal(proposal_id) -> Result<Proposal, ForgeError>
fn get_vote(proposal_id, voter) -> Result<Vote, ForgeError>
```

## States

- `Active` — Voting in progress
- `Succeeded` — Quorum reached, passed
- `Defeated` — Quorum reached, failed
- `Executed` — On-chain action executed
- `Cancelled` — Revoked by proposer
- `Expired` — Voting ended without quorum
