# Soroban Forge — Contract Template

Use this template to bootstrap a new Soroban Forge contract crate.

```bash
cp -r templates/contract crates/my-contract
```

Then:

1. Update `Cargo.toml` name and description.
2. Replace contract type, state, and methods in `src/lib.rs`.
3. Add tests in `src/tests.rs`.
4. Register the new crate in the workspace `Cargo.toml`.
5. Add documentation under `docs/contracts/`.
