# DAO Governance Contract Reference

- **Crate**: `soroban-forge-dao-governance`
- **Wasm Target**: `wasm32v1-none`
- **Rust Client**: `SorobanForgeDaoGovernanceClient`
- **TypeScript Client**: `@soroban-forge/dao-governance-client` (`Client`)

---

## 1. Overview & Architectural Role

The DAO Governance contract implements on-chain token-weighted governance with integrated anti-spam token bonds and dependency DAG management:
1. **Balance-Weighted Voting**: Members vote with voting power determined by real-time SEP-41 token balances at the moment of voting.
2. **Anti-Spam Proposal Bonds**: Every proposal requires posting a fixed bond upfront. Defeated proposals forfeit their bond to a designated treasury; executed and cancelled proposals refund the bond to the proposer.
3. **Dependency Graphs (DAGs)**: Proposals can specify prerequisite proposals (`requires`) and mutual exclusions (`conflicts_with`), enforced via iterative DFS cycle detection.
4. **Proposer Cooldown**: Limits concurrent active proposals per address to prevent spam.

---

## 2. API Reference

### Initialization & Configuration

#### `initialize`
Sets the SEP-41 governance token used for voting weights. One-time call.
