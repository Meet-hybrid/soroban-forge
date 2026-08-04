# Release checklist

How to cut a versioned release of Soroban Forge. The CI `Release` workflow
(`.github/workflows/release.yml`) builds the six contract crates to WASM and
drafts a GitHub release **when a `v*` tag is pushed**; the steps below prepare
and trigger that.

## Before you start

- Confirm the working tree is clean and `main` is up to date:
  `git status && git pull --ff-only origin main`.
- Confirm every member crate shares the version:
  `cargo metadata --no-deps --format-version 1` — all packages should report
  the same `0.1.0` (they inherit `version.workspace = true`).

## Quality gates

Run all of these locally on the pinned toolchain (1.96.0):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo audit
```

CI enforces the first four plus the WASM Size Check on every pull request, so
a green PR is the primary gate.

## Prepare the release PR

1. Move the entries under `## [Unreleased]` in `CHANGELOG.md` into a new
   dated section (`## [1.2.3] - YYYY-MM-DD`) and add an empty
   `## [Unreleased]` back on top.
2. Bump the version in `Cargo.toml` (`[workspace.package] version`); all
   members inherit it. Keep `Cargo.lock` in sync by running
   `cargo build --workspace --locked` and committing the diff.
3. Open a PR titled `chore: release v<version>` that includes the changelog
   and version bump. Do **not** push the tag inside the PR.
4. Merge after CI is green (one approving review; maintainers use the admin
   bypass for their own PRs).

## Cut the release

After the release PR is merged:

```bash
git fetch origin && git checkout main && git pull --ff-only origin main
git tag -a v0.1.0 -m "Soroban Forge v0.1.0"
git push origin v0.1.0
```

Pushing the tag triggers `Release`:

- `publish` builds the six contract crates for `wasm32-unknown-unknown`
  (`--locked`, pinned toolchain) and uploads the artifacts; it fails loudly
  if no `.wasm` files are produced.
- `github-release` drafts a GitHub release with the WASM artifacts attached.

## After the release

- Open the draft release on GitHub, sanity-check the notes and attached
  `.wasm` artifacts, then publish it.
- Update the version reference in the issue backlog / application packet if
  they mention a specific release.
- Close the release issue (e.g. #22) as completed.
