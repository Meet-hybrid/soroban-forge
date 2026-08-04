---
name: Maintainer Task
about: A scoped implementation or documentation task suitable for community contribution
title: "type(scope): short imperative summary"
labels: "complexity: medium"
assignees: ""
---

> **Campaign label note:** the `Stellar Wave` campaign label must only be added
> once the repository has been accepted by the campaign organizers. Draft
> issues ship without it.

## Description

Explain the single outcome, why it matters, and what is currently missing.
Reference the affected contract, SDK, CLI, documentation, or CI surface.

## What "done" looks like

- [ ] The concrete, checkable outcome of this issue.
- [ ] Behavior changes are covered by tests.
- [ ] `make lint` and `make test` pass.

## Implementation guidelines

- Concrete first steps or file pointers a contributor can act on.
- Anything to verify rather than assume.
- Dependencies or migrations involved, if any.

## PR guidelines

- Get assigned before starting; do not start work on a reserved issue.
- Work in a fork of the repository; open the PR from your fork against `main`.
- PR description must include: `Closes #<this issue>`.
- State any before/after data (sizes, gas, behavior) explicitly.

## 📋 Before you start

Read our Code Quality Standards in `CONTRIBUTING.md`, then run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

## Metadata

- **Affected crates/files:** _fill in_
- **Complexity label:** exactly one of `complexity: trivial` / `complexity: medium` / `complexity: high`
- **Type labels:** one or more of `enhancement`, `bug`, `documentation`, `refactor`, `test`, `chore`, `dependencies`
