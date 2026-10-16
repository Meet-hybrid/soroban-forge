#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Soroban Forge — intentional resource-baseline updater.
#
# Regenerates budgets/resource-baseline.json from fresh measurements of every
# benchmark flow in crates/resource-budget. This is the ONLY supported way to
# change the baseline: CI never rewrites it, so a PR that shifts execution
# costs without a reviewed baseline update fails the Resource Budget Gate.
#
# Usage:
#   scripts/update-resource-baseline.sh
#
# Review the resulting diff and commit the baseline in the same PR as the
# change that motivated the refresh. See docs/resource-budgets.md.
# ---------------------------------------------------------------------------
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

UPDATE_RESOURCE_BASELINE=1 cargo test \
  -p soroban-forge-resource-budget \
  --locked \
  --test resource_gate \
  -- --nocapture contract_entrypoint_resources_within_baseline

echo
echo "Baseline regenerated. Review the diff before committing:"
git --no-pager diff -- budgets/resource-baseline.json || true
