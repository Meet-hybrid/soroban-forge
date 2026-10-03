# DAO Governance Contract Reference

- **Crate**: `crates/dao-governance`
- **Package**: `soroban-forge-dao-governance`
- **Client**: `SorobanForgeDaoGovernanceClient`
- **Contract Type**: `DaoGovernance`
- **Trait**: `SorobanForgeDaoGovernance`

---

## Overview

The DAO Governance contract implements on-chain decentralized decision making. Governance token holders submit proposals, cast balance-weighted votes, and execute approved cross-contract actions. The contract incorporates anti-spam proposal bonds, DAG execution order dependencies, proposer cooldowns, and immutable treasury forfeiture.

### Key Capabilities
- **Proposal Bonds (Anti-Spam)**: Requires proposers to post an upfront bond in a designated SEP-41 token. Bonds are refunded upon successful execution or cancellation, and forfeited to the treasury if defeated.
- **DAG Execution Dependencies**: Supports execution dependencies (`requires`) and mutual exclusions (`conflicts_with`) with iterative DFS cycle detection.
- **Token-Weighted Voting**: Voting power is proportional to the voter's SEP-41 governance token balance at the time of vote casting.
- **Strict Majority Quorum**: Proposals pass if and only if $\text{for\_votes} > \text{against\_votes}$ when voting closes. Ties and zero-vote proposals are defeated.
- **Proposer Concurrency Limits**: Limits proposers to a maximum of 5 concurrent active proposals (`DEFAULT_MAX_ACTIVE_PROPOSALS`).

---

## State Machine & Lifecycle

