# DAO Governance Contract

On-chain proposal system with voting and permissionless execution of approved
opaque actions.

## Interface

```rust
fn propose(proposer, target, action, duration) -> Result<u64, ForgeError>
fn vote(proposal_id, voter, support) -> Result<(), ForgeError>
fn execute(proposal_id) -> Result<(), ForgeError>
fn cancel_proposal(proposal_id, proposer) -> Result<(), ForgeError>
fn get_proposal(proposal_id) -> Result<Proposal, ForgeError>
fn get_proposal_count() -> u64
fn get_proposals(offset, limit) -> Result<Vec<Proposal>, ForgeError>
fn has_voted(proposal_id, voter) -> Result<bool, ForgeError>
```


`action` is forwarded as a single `Bytes` argument to the target contract's
`execute` entrypoint. Voting remains one vote per voter with a strict
majority. After the deadline, the first `execute` call finalises the vote; a
succeeded proposal is then dispatched by a subsequent permissionless
`execute` call. Only a successful target invocation changes `Succeeded` to
`Executed`. A target revert returns
`ForgeError::ContractInvocationFailed` and leaves the proposal retryable in
`Succeeded`.

The DAO call itself is permissionless after voting has ended. A target's own
`require_auth` is not implicitly satisfied by the DAO's cross-contract call;
targets that require authorization must receive an authorization path that
the target contract accepts.

## States

- `Active` — Voting in progress
- `Succeeded` — Majority reached, passed and ready for dispatch
- `Defeated` — Quorum reached, failed
- `Executed` — On-chain action executed
- `Cancelled` — Revoked by proposer
- `Expired` — Voting ended without quorum

## Events

The contract emits structured events for all state changes using the standard `proposal_id` topic pattern:

- **`Proposed`**: Emitted when a new proposal is created.
  - Topics: `proposal_id: u64`
  - Data: `data: Proposal` (the full initial proposal record)
- **`VoteCast`**: Emitted when a voter casts a vote.
  - Topics: `proposal_id: u64`
  - Data: `voter: Address`, `support: bool`
- **`Finalised`**: Emitted when a proposal transitions state during execution (e.g. `Active` → `Succeeded`, `Active` → `Defeated`, or `Succeeded` → `Executed`).
  - Topics: `proposal_id: u64`
  - Data: `state: ProposalState`, `for_votes: i128`, `against_votes: i128`

