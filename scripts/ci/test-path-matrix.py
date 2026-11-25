#!/usr/bin/env python3
"""Regression tests for focused crate path-matrix resolution and workflow linting.

Usage:
    python3 scripts/ci/test-path-matrix.py
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CI_SCRIPTS_DIR = Path(__file__).resolve().parent


def _load_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, CI_SCRIPTS_DIR / filename)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


detector = _load_module("detect_changed_crates", "detect-changed-crates.py")
linter = _load_module("lint_workflows", "lint-workflows.py")

CONTRACT_PACKAGES = [
    "soroban-forge-dao-governance",
    "soroban-forge-escrow",
    "soroban-forge-marketplace-royalties",
    "soroban-forge-multi-sig-wallet",
    "soroban-forge-subscription-payments",
    "soroban-forge-vesting",
]

ALL_WORKSPACE_PACKAGES = sorted(
    [
        *CONTRACT_PACKAGES,
        "soroban-forge-cli",
        "soroban-forge-composition-tests",
        "soroban-forge-shared-utils",
        "soroban-forge-test-utils",
    ]
)


class PathMatrixTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.graph = detector.load_workspace_graph(REPO_ROOT)

    def test_workspace_graph_discovers_all_members_and_contracts(self) -> None:
        self.assertEqual(self.graph.all_packages, ALL_WORKSPACE_PACKAGES)
        self.assertEqual(self.graph.contract_packages, CONTRACT_PACKAGES)

    def test_single_contract_crate_maps_to_smallest_matrix_and_generated_clients(self) -> None:
        cases = [
            ("crates/escrow/src/lib.rs", "soroban-forge-escrow"),
            ("crates/vesting/src/lib.rs", "soroban-forge-vesting"),
            ("crates/multi-sig-wallet/src/batch.rs", "soroban-forge-multi-sig-wallet"),
            ("crates/dao-governance/src/props.rs", "soroban-forge-dao-governance"),
            (
                "crates/subscription-payments/src/billing.rs",
                "soroban-forge-subscription-payments",
            ),
            (
                "crates/marketplace-royalties/src/tier_props.rs",
                "soroban-forge-marketplace-royalties",
            ),
        ]
        for changed_path, expected_pkg in cases:
            with self.subTest(path=changed_path):
                plan = detector.compute_plan([changed_path], self.graph)
                expected = sorted([expected_pkg, "soroban-forge-composition-tests"])
                self.assertFalse(plan["full_workspace"])
                self.assertEqual(plan["packages"], expected)
                self.assertEqual(plan["matrix_packages"], expected)
                self.assertTrue(plan["has_crates"])
                self.assertFalse(plan["run_shared_utils"])
                self.assertTrue(plan["run_generated_clients"])

    def test_cli_and_templates_map_only_to_cli(self) -> None:
        for path in ("crates/cli/src/main.rs", "templates/contract.rs"):
            with self.subTest(path=path):
                plan = detector.compute_plan([path], self.graph)
                self.assertFalse(plan["full_workspace"])
                self.assertEqual(plan["packages"], ["soroban-forge-cli"])
                self.assertEqual(plan["matrix_packages"], ["soroban-forge-cli"])
                self.assertTrue(plan["has_crates"])
                self.assertFalse(plan["run_shared_utils"])
                self.assertFalse(plan["run_generated_clients"])

    def test_composition_tests_only_runs_composition_tests(self) -> None:
        plan = detector.compute_plan(["crates/composition-tests/src/dao_dispute.rs"], self.graph)
        self.assertFalse(plan["full_workspace"])
        self.assertEqual(plan["packages"], ["soroban-forge-composition-tests"])
        self.assertEqual(plan["matrix_packages"], ["soroban-forge-composition-tests"])
        self.assertTrue(plan["has_crates"])
        self.assertFalse(plan["run_shared_utils"])
        self.assertFalse(plan["run_generated_clients"])

    def test_shared_utils_change_runs_shared_utils_dependents_and_generated_clients(self) -> None:
        for path in ("crates/shared-utils/src/math.rs", "crates/shared-utils/Cargo.toml"):
            with self.subTest(path=path):
                plan = detector.compute_plan([path], self.graph)
                expected_packages = sorted(
                    [
                        *CONTRACT_PACKAGES,
                        "soroban-forge-composition-tests",
                        "soroban-forge-shared-utils",
                        "soroban-forge-test-utils",
                    ]
                )
                expected_matrix = [
                    p for p in expected_packages if p != "soroban-forge-shared-utils"
                ]
                self.assertFalse(plan["full_workspace"])
                self.assertEqual(plan["packages"], expected_packages)
                self.assertEqual(plan["matrix_packages"], expected_matrix)
                self.assertNotIn("soroban-forge-cli", plan["packages"])
                self.assertTrue(plan["has_crates"])
                self.assertTrue(plan["run_shared_utils"])
                self.assertTrue(plan["run_generated_clients"])

    def test_test_utils_change_skips_shared_utils_cli_and_generated_clients(self) -> None:
        plan = detector.compute_plan(["crates/test-utils/src/mocks.rs"], self.graph)
        expected = sorted(
            [
                *CONTRACT_PACKAGES,
                "soroban-forge-composition-tests",
                "soroban-forge-test-utils",
            ]
        )
        self.assertFalse(plan["full_workspace"])
        self.assertEqual(plan["packages"], expected)
        self.assertEqual(plan["matrix_packages"], expected)
        self.assertNotIn("soroban-forge-shared-utils", plan["packages"])
        self.assertNotIn("soroban-forge-cli", plan["packages"])
        self.assertTrue(plan["has_crates"])
        self.assertFalse(plan["run_shared_utils"])
        self.assertFalse(plan["run_generated_clients"])

    def test_error_registry_files_trigger_shared_utils_and_generated_clients(self) -> None:
        for path in (
            "errors.json",
            "scripts/generate-error-registry.sh",
            "scripts/check-error-registry-fresh.sh",
            "packages/typescript-sdk/src/errors.generated.ts",
        ):
            with self.subTest(path=path):
                plan = detector.compute_plan([path], self.graph)
                self.assertFalse(plan["full_workspace"])
                self.assertEqual(plan["packages"], ["soroban-forge-shared-utils"])
                self.assertEqual(plan["matrix_packages"], [])
                self.assertFalse(plan["has_crates"])
                self.assertTrue(plan["run_shared_utils"])
                self.assertTrue(plan["run_generated_clients"])

    def test_generated_client_and_sdk_paths_trigger_only_generated_clients(self) -> None:
        for path in (
            "packages/typescript-sdk/src/helpers.ts",
            "packages/vesting-client/src/index.ts",
            "packages/multi-sig-wallet-client/package.json",
            "packages/dao-governance-client/src/index.ts",
            "packages/subscription-payments-client/src/index.ts",
            "packages/marketplace-royalties-client/src/index.ts",
            "scripts/generate-clients.sh",
        ):
            with self.subTest(path=path):
                plan = detector.compute_plan([path], self.graph)
                self.assertFalse(plan["full_workspace"])
                self.assertEqual(plan["packages"], [])
                self.assertEqual(plan["matrix_packages"], [])
                self.assertFalse(plan["has_crates"])
                self.assertFalse(plan["run_shared_utils"])
                self.assertTrue(plan["run_generated_clients"])

    def test_indexer_fixture_maps_to_owning_contract_and_generated_clients(self) -> None:
        plan = detector.compute_plan(["indexer/fixtures/escrow-events.json"], self.graph)
        self.assertFalse(plan["full_workspace"])
        self.assertEqual(
            plan["packages"],
            ["soroban-forge-composition-tests", "soroban-forge-escrow"],
        )
        self.assertFalse(plan["run_shared_utils"])
        self.assertTrue(plan["run_generated_clients"])

        generic_plan = detector.compute_plan(
            ["indexer/fixtures/shared-schema.json"], self.graph
        )
        self.assertFalse(generic_plan["full_workspace"])
        self.assertEqual(
            generic_plan["packages"],
            sorted([*CONTRACT_PACKAGES, "soroban-forge-composition-tests"]),
        )
        self.assertTrue(generic_plan["run_generated_clients"])

    def test_multiple_crates_produce_minimal_union(self) -> None:
        plan = detector.compute_plan(
            ["crates/cli/src/cli.rs", "crates/vesting/src/lib.rs"],
            self.graph,
        )
        self.assertFalse(plan["full_workspace"])
        self.assertEqual(
            plan["packages"],
            [
                "soroban-forge-cli",
                "soroban-forge-composition-tests",
                "soroban-forge-vesting",
            ],
        )
        self.assertFalse(plan["run_shared_utils"])
        self.assertTrue(plan["run_generated_clients"])

    def test_full_workspace_fallback_triggers(self) -> None:
        fallback_paths = [
            "Cargo.lock",
            "Cargo.toml",
            "rust-toolchain.toml",
            "deny.toml",
            "Makefile",
            "rustfmt.toml",
            ".cargo/audit.toml",
            ".github/workflows/ci.yml",
            ".github/workflows/focused-checks.yml",
            "scripts/ci/detect-changed-crates.py",
            "crates/new-unregistered-crate/src/lib.rs",
        ]
        for path in fallback_paths:
            with self.subTest(path=path):
                plan = detector.compute_plan([path], self.graph)
                self.assertTrue(plan["full_workspace"])
                self.assertEqual(plan["packages"], ALL_WORKSPACE_PACKAGES)
                self.assertEqual(plan["matrix_packages"], [])
                self.assertFalse(plan["has_crates"])
                self.assertTrue(plan["run_shared_utils"])
                self.assertTrue(plan["run_generated_clients"])

    def test_mixed_crate_and_lockfile_uses_full_workspace_fallback(self) -> None:
        plan = detector.compute_plan(["crates/cli/src/main.rs", "Cargo.lock"], self.graph)
        self.assertTrue(plan["full_workspace"])
        self.assertEqual(plan["packages"], ALL_WORKSPACE_PACKAGES)
        self.assertEqual(plan["matrix_packages"], [])
        self.assertFalse(plan["has_crates"])

    def test_docs_and_unrelated_paths_skip_heavy_checks(self) -> None:
        plan = detector.compute_plan(
            [
                "README.md",
                "CHANGELOG.md",
                "docs/FOCUSED-CI.md",
                "packages/nextjs-example/src/app/page.tsx",
            ],
            self.graph,
        )
        self.assertFalse(plan["full_workspace"])
        self.assertEqual(plan["packages"], [])
        self.assertEqual(plan["matrix_packages"], [])
        self.assertFalse(plan["has_crates"])
        self.assertFalse(plan["run_shared_utils"])
        self.assertFalse(plan["run_generated_clients"])

    def test_git_diff_zero_sha_falls_back_to_full_workspace(self) -> None:
        files, fallback_reason = detector.git_changed_files(
            REPO_ROOT, detector.ZERO_SHA, "HEAD"
        )
        self.assertEqual(files, [])
        self.assertIsNotNone(fallback_reason)

    def test_cli_github_output_and_summary(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            out_file = Path(tmpdir) / "github_output.txt"
            summary_file = Path(tmpdir) / "summary.md"
            proc = subprocess.run(
                [
                    sys.executable,
                    str(CI_SCRIPTS_DIR / "detect-changed-crates.py"),
                    "--files",
                    "crates/escrow/src/lib.rs",
                    "--github-output",
                    str(out_file),
                    "--github-summary",
                    str(summary_file),
                ],
                capture_output=True,
                text=True,
                check=True,
            )
            payload = json.loads(proc.stdout)
            self.assertEqual(
                payload["matrix_packages"],
                ["soroban-forge-composition-tests", "soroban-forge-escrow"],
            )
            kv = dict(
                line.split("=", 1)
                for line in out_file.read_text(encoding="utf-8").splitlines()
                if "=" in line
            )
            self.assertEqual(kv["full_workspace"], "false")
            self.assertEqual(kv["has_crates"], "true")
            self.assertEqual(kv["run_shared_utils"], "false")
            self.assertEqual(kv["run_generated_clients"], "true")
            self.assertEqual(
                json.loads(kv["matrix_packages"]),
                ["soroban-forge-composition-tests", "soroban-forge-escrow"],
            )
            self.assertIn("Focused Crate Validation Plan", summary_file.read_text(encoding="utf-8"))


class WorkflowLintTests(unittest.TestCase):
    def test_repository_workflows_pass_lint(self) -> None:
        workflows = sorted((REPO_ROOT / ".github" / "workflows").glob("*.yml"))
        self.assertGreaterEqual(len(workflows), 1)
        for wf in workflows:
            with self.subTest(workflow=wf.name):
                self.assertEqual(linter.lint_workflow_file(wf), [])

    def _lint_yaml(self, content: str) -> list[str]:
        with tempfile.TemporaryDirectory() as tmpdir:
            wf_path = Path(tmpdir) / "test.yml"
            wf_path.write_text(content, encoding="utf-8")
            return linter.lint_workflow_file(wf_path)

    def test_rejects_duplicate_keys(self) -> None:
        errs = self._lint_yaml(
            """
name: Bad
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
  a:
    runs-on: ubuntu-latest
    steps:
      - run: echo duplicate
"""
        )
        self.assertTrue(any("duplicate YAML key" in e for e in errs), errs)

    def test_rejects_missing_output_reference(self) -> None:
        errs = self._lint_yaml(
            """
name: Bad Outputs
on: push
jobs:
  detect:
    runs-on: ubuntu-latest
    outputs:
      known: ${{ steps.s.outputs.known }}
    steps:
      - id: s
        run: echo "known=true" >> "$GITHUB_OUTPUT"
  run_check:
    needs: detect
    if: needs.detect.outputs.missing_key == 'true'
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
"""
        )
        self.assertTrue(any("missing_key" in e for e in errs), errs)

    def test_rejects_circular_needs(self) -> None:
        errs = self._lint_yaml(
            """
name: Cycle
on: push
jobs:
  a:
    needs: b
    runs-on: ubuntu-latest
    steps:
      - run: echo a
  b:
    needs: a
    runs-on: ubuntu-latest
    steps:
      - run: echo b
"""
        )
        self.assertTrue(any("circular job dependency" in e for e in errs), errs)

    def test_rejects_unpinned_action_and_write_all_permissions(self) -> None:
        errs = self._lint_yaml(
            """
name: Unpinned
on: push
permissions: write-all
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout
"""
        )
        self.assertTrue(any("write-all" in e for e in errs), errs)
        self.assertTrue(any("must pin a ref" in e for e in errs), errs)

    def test_rejects_untrusted_pr_title_in_run_and_bad_bash(self) -> None:
        errs = self._lint_yaml(
            """
name: Unsafe
on: pull_request
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: echo "${{ github.event.pull_request.title }}"
      - run: if [ 1 -eq 1 ]; then echo broken
"""
        )
        self.assertTrue(any("untrusted expression" in e for e in errs), errs)
        self.assertTrue(any("bash syntax check failed" in e for e in errs), errs)


if __name__ == "__main__":
    unittest.main(verbosity=2)
