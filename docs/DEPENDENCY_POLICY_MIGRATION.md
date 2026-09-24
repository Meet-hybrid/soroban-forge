Dependency Policy Migration Plan

Goal

Prepare a focused follow-up PR that pins/updates `cargo-deny` and reintroduces the `licenses` enforcement in `deny.toml` in a schema-compatible way.

Why

The repository's current `deny.toml` used fields that don't parse against the installed `cargo-deny` (v0.20.2). To avoid breaking CI we temporarily removed the `licenses` check. This follow-up will safely restore license enforcement.

Steps for the follow-up PR

1. Pin `cargo-deny` in CI and local testing
   - Update the CI `dependency-policy` job to install a specific `cargo-deny` version (e.g. `0.20.2`) with:

```bash
cargo install --locked cargo-deny --version 0.20.2
```

- This ensures the CLI and its expected `deny.toml` schema are stable across CI runs and local testing.

2. Draft a `licenses` block compatible with `cargo-deny` v0.20.x
   - Example snippet (validate before committing):

```toml
[licenses]
include-dev = true
allow = [
  "MIT",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Unicode-DFS-2016",
  "CC0-1.0",
]
# If you need to allow common dual-license expressions, list them individually
# by crate in `exceptions` rather than relying on a parser to accept "MIT OR Apache-2.0".
exceptions = [
  # { name = "ahash", allow = ["MIT", "Apache-2.0"] },  # justification
]
```

- Note: Some crates use the `MIT OR Apache-2.0` SPDX expression. If v0.20.x does not accept that in `allow`, prefer per-crate exceptions that explicitly list allowed licenses.

3. Validate locally

```bash
# install pinned cargo-deny locally
cargo install --locked cargo-deny --version 0.20.2

# run license check
cargo deny check licenses
```

4. Re-enable CI license check
   - Re-add `cargo deny check licenses` to `.github/workflows/ci.yml` once local validation passes.
   - Ensure CI installs the pinned `cargo-deny` version.

5. Documentation & PR description
   - Explain in the PR why we pinned the version and how to update the `licenses` block in future.
   - Include the `cargo deny` output from local runs as the baseline.

Notes & rationale

- We deliberately delegate advisories to `cargo audit` to avoid duplicate noise.
- Dual-license SPDX expressions can be handled either by updating the `allow` list if supported by the pinned `cargo-deny` version or by adding explicit per-crate exceptions (preferred for clarity).
- The PR should be small and focused: pin/tooling + `deny.toml` license block + verification output.

If you want, I can draft the follow-up PR branch content now (the `licenses` snippet and the CI change) and run the validation locally using `cargo-deny 0.20.2` to capture the baseline output for the PR body.
