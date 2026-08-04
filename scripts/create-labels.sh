#!/usr/bin/env bash
#
# create-labels.sh — create/update the Soroban Forge issue labels.
#
# The label names, colors, and descriptions below are the canonical set used
# by the maintainer workflow (see docs/maintainers/issue-triage.md). Run this
# before creating issues from docs/maintainers/issue-backlog.md.
#
# Usage:
#   ./scripts/create-labels.sh                      # dry run (prints changes)
#   ./scripts/create-labels.sh --apply              # create/update on GitHub
#   ./scripts/create-labels.sh --apply OWNER/REPO   # target a specific repo
#
# Options:
#   --apply              Actually create/update labels (default is a dry run).
#   --include-campaign   Also create the `Stellar Wave` campaign label.
#                        Leave OFF until the repository is accepted into the
#                        program — organizers add it on acceptance.
#
# Requirements (only for --apply): GitHub CLI (gh) installed and authenticated
# (`gh auth login`). The script never deletes labels and never touches issues.

set -euo pipefail

APPLY=0
INCLUDE_CAMPAIGN=0
REPO=""

for arg in "$@"; do
  case "$arg" in
    --apply) APPLY=1 ;;
    --include-campaign) INCLUDE_CAMPAIGN=1 ;;
    -*) echo "error: unknown option: $arg" >&2; exit 1 ;;
    *) REPO="$arg" ;;
  esac
done

if [ -z "$REPO" ]; then
  REPO="${GITHUB_REPOSITORY:-Meet-hybrid/soroban-forge}"
fi

if [ "$APPLY" -eq 1 ]; then
  if ! command -v gh >/dev/null 2>&1; then
    echo "error: GitHub CLI (gh) is required for --apply" >&2
    exit 1
  fi
  if ! gh auth status >/dev/null 2>&1; then
    echo "error: not authenticated — run 'gh auth login' first" >&2
    exit 1
  fi
fi

# name|color|description
LABELS=(
  "complexity: trivial|0e8a16|Wave: typos, small bug fixes, minor copy changes (100 pts)"
  "complexity: medium|fbca04|Wave: standard features or involved bug fixes (150 pts)"
  "complexity: high|b60205|Wave: complex features, refactors, or new integrations (200 pts)"
  "enhancement|a2eeef|New feature or request"
  "bug|d73a4a|Something isn't working"
  "documentation|0075ca|Improvements or additions to documentation"
  "refactor|7057ff|A code change that neither fixes a bug nor adds a feature"
  "test|008672|Adding missing tests or correcting existing tests"
  "chore|ffffff|Maintenance tasks and build/CI changes"
  "dependencies|0366d6|Pull requests that update a dependency file"
  "good first issue|7057ff|Good for newcomers"
)

if [ "$INCLUDE_CAMPAIGN" -eq 1 ]; then
  LABELS+=("Stellar Wave|5319e7|Issues in the Stellar wave program")
fi

echo "Target repository: $REPO"
for entry in "${LABELS[@]}"; do
  IFS='|' read -r name color desc <<<"$entry"
  if [ "$APPLY" -eq 1 ]; then
    echo "-> $name"
    gh label create "$name" --repo "$REPO" --color "$color" --description "$desc" --force
  else
    printf 'would create/update: %-22s %s\n' "$name" "$desc"
  fi
done

if [ "$APPLY" -eq 1 ]; then
  echo
  echo "Done. Verify with: gh label list --repo $REPO"
else
  echo
  echo "Dry run complete. Re-run with --apply to create/update the labels."
  if [ "$INCLUDE_CAMPAIGN" -eq 0 ]; then
    echo "Note: the 'Stellar Wave' campaign label was omitted; add --include-campaign once the repository is accepted."
  fi
fi
