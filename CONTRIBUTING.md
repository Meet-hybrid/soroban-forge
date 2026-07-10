# Contributing to Soroban Forge

Thank you for your interest in contributing to Soroban Forge! This document provides guidelines and steps for contributing.

## Code of Conduct

By participating, you agree to uphold our Code of Conduct. Be respectful, inclusive, and collaborative.

## How to Contribute

### Reporting Bugs

- Use the **bug_report** issue template.
- Include steps to reproduce, expected behavior, actual behavior, and environment details (OS, Rust version, Soroban SDK version).
- Attach logs or minimal reproduction repositories when possible.

### Feature Requests

- Use the **feature_request** issue template.
- Describe the use case, proposed solution, and alternatives considered.
- Label the issue with `enhancement` and `good first issue` if appropriate.

### Security Issues

- **Do NOT open a public issue** for security vulnerabilities.
- Report confidentially via GitHub Security Advisories or email `security@teachlink.org`.
- See [SECURITY.md](SECURITY.md) for details.

## Development Setup

```bash
# Install Rust (1.75+)
rustup update stable
rustup component add rustfmt clippy
rustup target add wasm32-unknown-unknown

# Install Soroban CLI
cargo install soroban-cli

# Clone and build
git clone https://github.com/teachlink/soroban-forge.git
cd soroban-forge
make build
make test
```

## Code Standards

- Run `cargo fmt --all` before committing.
- Run `cargo clippy --workspace --all-targets -- -D warnings`.
- Write unit tests for all business logic and integration tests for contract interactions.
- Update `CHANGELOG.md` for any user-facing changes.
- Update relevant `docs/` for new features or breaking changes.

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat(escrow): add refund timeout enforcement`
- `fix(vesting): correct spelling error on vest_message`
- `docs(readme): add troubleshooting section`
- `refactor(shared-utils): simplify ForgeError conversions`
- `test(dao-governance): add proposal execution tests`
- `chore: upgrade soroban-sdk to 21.5.1`
- `perf(subscription): reduce WASM binary size by 12%`
- `ci: add audit workflow`

## Pull Request Process

1. Fork the repository and create your branch from `main`.
2. If you've added code that should be tested, add tests.
3. Ensure `make format lint test` passes.
4. Update documentation if needed.
5. Open a Pull Request with a clear title and description.
6. Request review from at least one maintainer.

## Maintainer Onboarding

- Two maintainers must approve non-trivial PRs.
- Breaking changes require a discussion issue and 72-hour review period.
- Security fixes follow our Security Policy and are fast-tracked.
