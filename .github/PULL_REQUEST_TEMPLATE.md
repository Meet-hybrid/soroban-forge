# Pull Request Template

## Description

Please include a summary of the change and which issue is fixed.

Fixes #

## Type of Change

- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update
- [ ] Refactoring (no functional changes)
- [ ] Performance improvement
- [ ] Test addition or modification
- [ ] CI/CD or build system change

## Checklist

- [ ] `cargo fmt --all` has been run
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New dependencies are audited and documented
- [ ] `cargo audit` passes
- [ ] WASM binary size impact is documented (if applicable)
- [ ] Documentation has been updated (if applicable)
- [ ] CHANGELOG.md has been updated (if applicable)
- [ ] I have read [CONTRIBUTING.md](CONTRIBUTING.md)

## Contract Changes (if applicable)

- [ ] Contract interface changes are backward-compatible
- [ ] Storage schema migration is documented
- [ ] Test coverage >= 90%
- [ ] Upgrade path is described

## Security Considerations

- [ ] No introduction of unsafe code
- [ ] No hardcoded secrets or keys
- [ ] Input validation covers all public functions
- [ ] Authorization checks are enforced on all sensitive operations
- [ ] Reentrancy and overflow considerations are documented
