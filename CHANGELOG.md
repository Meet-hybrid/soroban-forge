# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - YYYY-MM-DD

### Added
- Initial release of Soroban Forge.
- Escrow contract with arbiter dispute resolution, deposit, release, and refund flows.
- Vesting contract with linear and custom schedules, cliff support.
- Multi-Signature Wallet contract with configurable threshold and transaction types.
- DAO Governance contract with proposals, weighted voting, quorum, and execution delay.
- Subscription Payments contract with plan management, billing cycles, and auto-renewal.
- Marketplace Royalties contract with creator splits, secondary sale handling, and payout logic.
- Shared utility crates (`shared-utils`, `test-utils`) for consistent error handling and testing.
- TypeScript SDK for off-chain contract interaction.
- Next.js reference application demonstrating wallet connection and contract calls.
- Deployment templates for Stellar Testnet and Mainnet.
- Comprehensive documentation, tutorials, and best practices guide.
- GitHub Actions CI with build, test, clippy, and `cargo audit`.
- Security policy and contribution guidelines.
- MIT / Apache-2.0 dual license.

### Security
- All `unsafe` code is forbidden.
- Dependencies scanned with `cargo audit` in CI.
