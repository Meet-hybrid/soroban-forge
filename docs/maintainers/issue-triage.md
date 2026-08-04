# Maintainer issue workflow

Soroban Forge uses small, reviewable issues so outside contributors can make
progress without needing private context.

## Write the issue

Every contributor-facing task should include:

- one concrete outcome;
- the reason the work matters;
- explicit in-scope and out-of-scope boundaries;
- acceptance criteria that can be checked in a pull request;
- affected crates, packages, docs, or workflows;
- the verification commands to run.

Use the **Maintainer Task** template for implementation, test, documentation,
and CI work. Title issues with conventional commits (`feat(escrow): …`,
`test: …`, `docs: …`) so the issue list reads like a changelog, exactly as the
Stellar Wave reference repository does.

Apply exactly **one** complexity label, plus any relevant type labels
(`enhancement`, `bug`, `documentation`, `refactor`, `test`, `chore`,
`dependencies`). Create the labels with these descriptions so the list view
explains itself:

| Label | GitHub label description | Use for |
| --- | --- | --- |
| `complexity: trivial` | `Wave: typos, small bug fixes, minor copy changes (100 pts)` | Small documentation or isolated fixes |
| `complexity: medium` | `Wave: standard features or involved bug fixes (150 pts)` | A normal feature, test improvement, or involved bug fix |
| `complexity: high` | `Wave: complex features, refactors, or new integrations (200 pts)` | Cross-crate changes, refactors, or new integrations |
| `good first issue` | `Good for newcomers` | Small, well-scoped tasks for new contributors |
| `Stellar Wave` | `Issues in the Stellar wave program` | Only after the repository is accepted into the program |

The points convention is useful for Wave-style programs, but it is not a
promise of payment. A program organizer must approve the repository and fund
the campaign separately. The `Stellar Wave` label is added post-acceptance
only.

## Assign and unassign

1. A contributor comments with their proposed approach or applies through the
   active campaign dashboard.
2. Review their relevant GitHub work and confirm they understand the scope.
3. Assign the issue to exactly one contributor in GitHub. Assignment is the
   source of truth that the work is reserved.
4. The contributor opens a focused pull request containing `Closes #<number>`.
5. Review, request changes when needed, and merge only when the acceptance
   criteria are met.

Contributors work in **forks**: the PR arrives from `contributor:soroban-forge`
against `main`. Review fork PRs exactly like in-repo PRs. One fork-specific
step: GitHub does not run workflows on the first PR from a new contributor if
it modifies `.github/workflows/` — approve the "Approve and run" prompt after
quickly scanning the diff.
6. If the contributor cannot continue, ask them to confirm and remove the
   assignee. Add a short comment explaining that the issue is available again;
   do not silently replace the assignment.
7. Close the issue as completed when the work is merged and verified. Close it
   as not planned only when the scope is intentionally abandoned.

Do not assign several people to the same funded task unless the issue is
explicitly split into independent subtasks. If multiple applications arrive,
acknowledge the others after selecting one contributor.

## Maintainer quality bar

Before marking a task complete, confirm that CI passes, tests cover behavior
changes, public contract/storage changes are documented, and security-sensitive
changes receive a second review where practical.
