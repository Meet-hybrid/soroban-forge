# Soroban Forge

[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache-2.0-blue.svg)](LICENSE)
[![CI](https://github.com/teachlink/soroban-forge/actions/workflows/ci.yml/badge.svg)](https://github.com/teachlink/soroban-forge/actions)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange)](https://www.rust-lang.org)

**Soroban Forge** is a community-driven, production-ready library of reusable Soroban smart contracts and developer tooling for the Stellar ecosystem.

## Vision

Eliminate the need to repeatedly build common smart contracts from scratch by providing audited, reusable, modular, and well-documented implementations for the Stellar ecosystem.

## Initial Contracts

| Contract | Description |
|----------|-------------|
| **Escrow** | Secure buyer-seller transactions with arbiter dispute resolution |
| **Vesting** | Time-locked token release with cliff and schedule support |
| **Multi-Sig Wallet** | Multi-owner wallet with configurable approval thresholds |
| **DAO Governance** | On-chain proposals, voting, quorum, and execution |
| **Subscription Payments** | Recurring payment plans with auto-renewal |
| **Marketplace Royalties** | NFT/asset sales with configurable royalty distribution |

## Architecture

```
soroban-forge/
├── crates/                 # Rust smart contracts and libraries
│   ├── shared-utils/       # Shared types, error handling, storage patterns
│   ├── test-utils/         # Shared test harnesses and soroban test helpers
│   ├── escrow/
│   ├── vesting/
│   ├── multi-sig-wallet/
│   ├── dao-governance/
│   ├── subscription-payments/
│   └── marketplace-royalties/
├── packages/               # Language bindings and example apps
│   ├── typescript-sdk/     # TypeScript SDK for contract interaction
│   ├── nextjs-example/     # Next.js reference application
│   └── deployment-templates/ # Docker, Kubernetes, and CI templates
├── docs/                   # Architecture, tutorials, best practices
├── examples/               # Integration examples
├── templates/              # Contract scaffolding templates
└── tests/                  # Integration and E2E tests
```

## Quick Start

```bash
# Clone the repository
git clone https://github.com/teachlink/soroban-forge.git
cd soroban-forge

# Build all contracts
cargo build --workspace

# Run tests
cargo test --workspace

# Run linters
make lint
```

## Documentation

- [Getting Started](docs/tutorials/getting-started.md)
- [Architecture](docs/architecture/index.md)
- [Security Best Practices](docs/best-practices/smart-contract-security.md)
- [Contract Overview](docs/contracts/index.md)
- [Contributing](CONTRIBUTING.md)

## Contributing

We welcome contributions! Please read [CONTRIBUTING.md](CONTRIBUTING.md) for details on our code of conduct and the process for submitting pull requests.

## Security

Please read [SECURITY.md](SECURITY.md) for how to report security vulnerabilities.

## License

This project is licensed under either of

- [Apache License, Version 2.0](LICENSE-Apache)
- [MIT license](LICENSE-MIT)

at your option.

## Acknowledgments

Built for the Stellar developer community.
