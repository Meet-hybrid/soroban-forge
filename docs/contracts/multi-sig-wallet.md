# Multi-Sig Wallet Contract Reference

- **Crate**: `soroban-forge-multi-sig-wallet`
- **Wasm Target**: `wasm32v1-none`
- **Rust Client**: `SorobanForgeMultiSigWalletClient`
- **TypeScript Client**: `@soroban-forge/multi-sig-wallet-client` (`Client`)

---

## 1. Overview & Architectural Role

The Multi-Sig Wallet contract implements $M$-of-$N$ multi-signature governance over token custody, contract configuration, and cross-contract invocations.
Key operational features include:
1. **Multi-Kind Execution**: Dispatches opaque byte payloads, typed cross-contract calls, native token withdrawals, limit modifications, and owner-set adjustments.
2. **Velocity Limits**: Configurable per-token rolling withdrawal caps that bound maximum outflows across sliding time windows.
3. **Single-Veto Objection**: Any owner can immediately veto and terminate a pending transaction via `reject`.
4. **Self-Governing Threshold & Membership**: Additions, removals, and threshold updates require approval under the existing quorum.

---

## 2. API Reference

### Configuration & Membership

#### `initialize`
Configures initial owners and threshold. Callable exactly once.
