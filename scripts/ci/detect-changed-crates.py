#!/usr/bin/env python3
"""Map changed repository paths to the smallest relevant Soroban Forge CI checks.

Reads the Cargo workspace graph directly from `Cargo.toml` and each member's
`Cargo.toml` so crate dependencies stay in sync without hardcoding reverse edges.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

SHARED_UTILS_PACKAGE = "soroban-forge-shared-utils"
CLI_PACKAGE = "soroban-forge-cli"

FULL_WORKSPACE_EXACT_FILES = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "deny.toml",
        "Makefile",
        "rustfmt.toml",
        ".rustfmt.toml",
        "clippy.toml",
        ".clippy.toml",
    }
)

FULL_WORKSPACE_PREFIXES = (
    ".cargo/",
    ".github/workflows/",
    "scripts/ci/",
)

SHARED_UTILS_EXTRA_FILES = frozenset(
    {
        "errors.json",
        "scripts/generate-error-registry.sh",
        "scripts/check-error-registry-fresh.sh",
        "packages/typescript-sdk/src/errors.generated.ts",
    }
)

GENERATED_CLIENT_EXACT_FILES = frozenset(
    {
        "scripts/generate-clients.sh",
    }
)

GENERATED_CLIENT_PREFIXES = (
    "packages/typescript-sdk/",
    "packages/vesting-client/",
    "packages/multi-sig-wallet-client/",
    "packages/dao-governance-client/",
    "packages/subscription-payments-client/",
    "packages/marketplace-royalties-client/",
)

INDEXER_FIXTURE_TO_CRATE = {
    "indexer/fixtures/escrow-events.json": "soroban-forge-escrow",
    "indexer/fixtures/vesting-events.json": "soroban-forge-vesting",
    "indexer/fixtures/multi-sig-wallet-events.json": "soroban-forge-multi-sig-wallet",
    "indexer/fixtures/dao-governance-events.json": "soroban-forge-dao-governance",
    "indexer/fixtures/subscription-payments-events.json": "soroban-forge-subscription-payments",
    "indexer/fixtures/marketplace-royalties-events.json": "soroban-forge-marketplace-royalties",
}

ZERO_SHA = "0" * 40


@dataclass(frozen=True)
class CrateInfo:
    name: str
    rel_dir: str
    is_contract: bool
    normal_deps: frozenset[str]
    all_deps: frozenset[str]


@dataclass(frozen=True)
class WorkspaceGraph:
    crates_by_name: dict[str, CrateInfo]
    dir_to_name: dict[str, str]
    all_reverse_deps: dict[str, frozenset[str]]
    normal_reverse_deps: dict[str, frozenset[str]]

    @property
    def all_packages(self) -> list[str]:
        return sorted(self.crates_by_name.keys())

    @property
    def contract_packages(self) -> list[str]:
        return sorted(
            name for name, info in self.crates_by_name.items() if info.is_contract
        )

    def reverse_closure(self, package: str, *, normal_only: bool = False) -> set[str]:
        graph = self.normal_reverse_deps if normal_only else self.all_reverse_deps
        visited = {package}
        stack = [package]
        while stack:
            current = stack.pop()
            for dep in graph.get(current, ()):
                if dep not in visited:
                    visited.add(dep)
                    stack.append(dep)
        return visited


def normalize_path(path: str) -> str:
    cleaned = path.strip().replace("\\", "/")
    while cleaned.startswith("./"):
        cleaned = cleaned[2:]
    return cleaned.strip("/")


def _extract_dep_tables(manifest: dict, key: str) -> list[dict]:
    tables: list[dict] = []
    top = manifest.get(key)
    if isinstance(top, dict):
        tables.append(top)
    target_section = manifest.get("target")
    if isinstance(target_section, dict):
        for target_cfg in target_section.values():
            if isinstance(target_cfg, dict) and isinstance(target_cfg.get(key), dict):
                tables.append(target_cfg[key])
    return tables


def _resolve_workspace_deps(
    manifest: dict,
    member_dir: Path,
    repo_root: Path,
    dir_to_name: dict[str, str],
    all_member_names: set[str],
    dep_keys: tuple[str, ...],
) -> set[str]:
    found: set[str] = set()
    for dep_key in dep_keys:
        for table in _extract_dep_tables(manifest, dep_key):
            for alias, spec in table.items():
                pkg_name = alias
                if isinstance(spec, dict):
                    if isinstance(spec.get("package"), str):
                        pkg_name = spec["package"]
                    if isinstance(spec.get("path"), str):
                        resolved = (member_dir / spec["path"]).resolve()
                        try:
                            rel = resolved.relative_to(repo_root.resolve()).as_posix()
                        except ValueError:
                            rel = ""
                        if rel in dir_to_name:
                            found.add(dir_to_name[rel])
                            continue
                if pkg_name in all_member_names:
                    found.add(pkg_name)
    return found


def load_workspace_graph(repo_root: Path) -> WorkspaceGraph:
    root_manifest_path = repo_root / "Cargo.toml"
    with root_manifest_path.open("rb") as handle:
        root_manifest = tomllib.load(handle)

    members = root_manifest.get("workspace", {}).get("members", [])
    if not isinstance(members, list) or not members:
        raise ValueError(f"No [workspace].members found in {root_manifest_path}")

    raw_manifests: dict[str, tuple[str, dict]] = {}
    dir_to_name: dict[str, str] = {}
    for member in members:
        rel_dir = normalize_path(str(member))
        member_manifest_path = repo_root / rel_dir / "Cargo.toml"
        with member_manifest_path.open("rb") as handle:
            manifest = tomllib.load(handle)
        pkg_name = manifest["package"]["name"]
        raw_manifests[pkg_name] = (rel_dir, manifest)
        dir_to_name[rel_dir] = pkg_name

    all_member_names = set(raw_manifests.keys())
    crates_by_name: dict[str, CrateInfo] = {}
    all_rev: dict[str, set[str]] = {name: set() for name in all_member_names}
    normal_rev: dict[str, set[str]] = {name: set() for name in all_member_names}

    for pkg_name, (rel_dir, manifest) in raw_manifests.items():
        lib_section = manifest.get("lib", {})
        crate_types = lib_section.get("crate-type", []) if isinstance(lib_section, dict) else []
        is_contract = isinstance(crate_types, list) and "cdylib" in crate_types
        member_dir = repo_root / rel_dir
        normal_deps = _resolve_workspace_deps(
            manifest,
            member_dir,
            repo_root,
            dir_to_name,
            all_member_names,
            ("dependencies", "build-dependencies"),
        )
        all_deps = _resolve_workspace_deps(
            manifest,
            member_dir,
            repo_root,
            dir_to_name,
            all_member_names,
            ("dependencies", "build-dependencies", "dev-dependencies"),
        )
        crates_by_name[pkg_name] = CrateInfo(
            name=pkg_name,
            rel_dir=rel_dir,
            is_contract=is_contract,
            normal_deps=frozenset(normal_deps),
            all_deps=frozenset(all_deps),
        )
        for dep in all_deps:
            all_rev[dep].add(pkg_name)
        for dep in normal_deps:
            normal_rev[dep].add(pkg_name)

    return WorkspaceGraph(
        crates_by_name=crates_by_name,
        dir_to_name=dir_to_name,
        all_reverse_deps={k: frozenset(v) for k, v in all_rev.items()},
        normal_reverse_deps={k: frozenset(v) for k, v in normal_rev.items()},
    )


def _match_member_crate(path: str, graph: WorkspaceGraph) -> str | None:
    for rel_dir, pkg_name in graph.dir_to_name.items():
        if path == rel_dir or path.startswith(f"{rel_dir}/"):
            return pkg_name
    return None


def compute_plan(
    changed_files: Iterable[str],
    graph: WorkspaceGraph,
    *,
    force_full_workspace: bool = False,
    force_reason: str | None = None,
) -> dict:
    normalized = sorted({normalize_path(p) for p in changed_files if normalize_path(p)})

    full_workspace = force_full_workspace
    affected_packages: set[str] = set()
    run_shared_utils = False
    run_generated_clients = False
    reasons: list[str] = []

    if force_full_workspace:
        reasons.append(force_reason or "full-workspace fallback explicitly requested")

    for path in normalized:
        # 1. Full-workspace fallback triggers: root config, lockfile, workflows, CI scripts
        if path in FULL_WORKSPACE_EXACT_FILES or any(
            path.startswith(prefix) for prefix in FULL_WORKSPACE_PREFIXES
        ):
            full_workspace = True
            reasons.append(f"{path} -> full-workspace fallback")
            continue

        # 2. Direct workspace crate paths
        matched_crate = _match_member_crate(path, graph)
        if matched_crate is not None:
            closure = graph.reverse_closure(matched_crate, normal_only=False)
            affected_packages.update(closure)
            extras = sorted(closure - {matched_crate})

            normal_closure = graph.reverse_closure(matched_crate, normal_only=True)
            touches_contract = any(
                graph.crates_by_name[pkg].is_contract for pkg in normal_closure
            )
            if touches_contract:
                run_generated_clients = True
            if matched_crate == SHARED_UTILS_PACKAGE:
                run_shared_utils = True

            detail = f"{path} -> {matched_crate}"
            if extras:
                detail += f" (+ dependents: {', '.join(extras)})"
            if matched_crate == SHARED_UTILS_PACKAGE:
                detail += " + shared-utils"
            if touches_contract:
                detail += " + generated-clients"
            reasons.append(detail)
            continue

        # 3. Unknown path inside crates/ -> safe fallback to full workspace
        if path == "crates" or path.startswith("crates/"):
            full_workspace = True
            reasons.append(f"{path} -> unrecognized crate path (full-workspace fallback)")
            continue

        # 4. Scaffold templates are embedded into soroban-forge-cli via include_str!
        if path == "templates" or path.startswith("templates/"):
            if CLI_PACKAGE in graph.crates_by_name:
                closure = graph.reverse_closure(CLI_PACKAGE, normal_only=False)
                affected_packages.update(closure)
                reasons.append(f"{path} -> {CLI_PACKAGE} (embedded templates)")
            else:
                full_workspace = True
                reasons.append(f"{path} -> full-workspace fallback (cli crate missing)")
            continue

        # 5. Indexer fixtures validate against contract tests & TypeScript SDK decoders
        if path in INDEXER_FIXTURE_TO_CRATE:
            target_pkg = INDEXER_FIXTURE_TO_CRATE[path]
            if target_pkg in graph.crates_by_name:
                closure = graph.reverse_closure(target_pkg, normal_only=False)
                affected_packages.update(closure)
            run_generated_clients = True
            reasons.append(f"{path} -> {target_pkg} + generated-clients")
            continue

        if path.startswith("indexer/fixtures/"):
            for contract_pkg in graph.contract_packages:
                affected_packages.update(
                    graph.reverse_closure(contract_pkg, normal_only=False)
                )
            run_generated_clients = True
            reasons.append(f"{path} -> all contract crates + generated-clients")
            continue

        # 6. Error registry files coupled to shared-utils and TypeScript SDK
        if path in SHARED_UTILS_EXTRA_FILES:
            run_shared_utils = True
            run_generated_clients = True
            if SHARED_UTILS_PACKAGE in graph.crates_by_name:
                affected_packages.add(SHARED_UTILS_PACKAGE)
            reasons.append(f"{path} -> shared-utils + generated-clients")
            continue

        # 7. Generated TypeScript clients & SDK packages
        if path in GENERATED_CLIENT_EXACT_FILES or any(
            path.startswith(prefix) for prefix in GENERATED_CLIENT_PREFIXES
        ):
            run_generated_clients = True
            reasons.append(f"{path} -> generated-clients")
            continue

    if full_workspace:
        packages = graph.all_packages
        matrix_packages: list[str] = []
        run_shared_utils = True
        run_generated_clients = True
    else:
        packages = sorted(affected_packages)
        matrix_packages = [
            pkg
            for pkg in packages
            if not (run_shared_utils and pkg == SHARED_UTILS_PACKAGE)
        ]

    if not reasons:
        if normalized:
            reasons.append("no Rust crates, shared-utils, or generated-client paths changed")
        else:
            reasons.append("no changed files detected")

    return {
        "full_workspace": full_workspace,
        "packages": packages,
        "matrix_packages": matrix_packages,
        "has_crates": len(matrix_packages) > 0,
        "run_shared_utils": run_shared_utils,
        "run_generated_clients": run_generated_clients,
        "changed_files_count": len(normalized),
        "reasons": reasons,
    }


def git_changed_files(
    repo_root: Path, base: str, head: str, *, diff_mode: str = "two-dot"
) -> tuple[list[str], str | None]:
    if not base or base == ZERO_SHA:
        return [], "initial push or missing base commit (full-workspace fallback)"
    diff_args = (
        ["git", "diff", "--name-only", f"{base}...{head}"]
        if diff_mode == "three-dot"
        else ["git", "diff", "--name-only", base, head]
    )
    proc = subprocess.run(
        diff_args,
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        return [], f"git diff {base}..{head} failed (full-workspace fallback)"
    return [line.strip() for line in proc.stdout.splitlines() if line.strip()], None


def write_github_output(output_path: Path, plan: dict) -> None:
    lines = [
        f"full_workspace={'true' if plan['full_workspace'] else 'false'}",
        f"has_crates={'true' if plan['has_crates'] else 'false'}",
        f"packages={json.dumps(plan['packages'], separators=(',', ':'))}",
        f"matrix_packages={json.dumps(plan['matrix_packages'], separators=(',', ':'))}",
        f"run_shared_utils={'true' if plan['run_shared_utils'] else 'false'}",
        f"run_generated_clients={'true' if plan['run_generated_clients'] else 'false'}",
        f"changed_files_count={plan['changed_files_count']}",
    ]
    with output_path.open("a", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")


def write_github_summary(summary_path: Path, plan: dict) -> None:
    mode = "Full-workspace fallback" if plan["full_workspace"] else "Focused crate matrix"
    matrix_display = ", ".join(f"`{p}`" for p in plan["matrix_packages"]) or "_none_"
    packages_display = ", ".join(f"`{p}`" for p in plan["packages"]) or "_none_"
    lines = [
        "### Focused Crate Validation Plan",
        "",
        "| Field | Value |",
        "| --- | --- |",
        f"| Mode | **{mode}** |",
        f"| Changed files | {plan['changed_files_count']} |",
        f"| Affected packages | {packages_display} |",
        f"| Focused matrix packages | {matrix_display} |",
        f"| Run `shared-utils` & error-registry checks | `{str(plan['run_shared_utils']).lower()}` |",
        f"| Run generated-client & SDK checks | `{str(plan['run_generated_clients']).lower()}` |",
        "",
        "#### Matching rules",
    ]
    for reason in plan["reasons"][:30]:
        lines.append(f"- `{reason}`")
    if len(plan["reasons"]) > 30:
        lines.append(f"- ... and {len(plan['reasons']) - 30} more")
    lines.append("")
    with summary_path.open("a", encoding="utf-8") as handle:
        handle.write("\n".join(lines))


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compute the smallest relevant crate/client CI validation matrix."
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="Path to the repository root (default: auto-detected).",
    )
    parser.add_argument(
        "--files",
        nargs="*",
        default=None,
        help="Explicit list of changed repository-relative file paths.",
    )
    parser.add_argument(
        "--files-file",
        type=Path,
        default=None,
        help="File containing newline-separated changed repository-relative paths.",
    )
    parser.add_argument(
        "--base",
        default=None,
        help="Git base ref/SHA for diffing changed files.",
    )
    parser.add_argument(
        "--head",
        default="HEAD",
        help="Git head ref/SHA for diffing changed files (default: HEAD).",
    )
    parser.add_argument(
        "--diff-mode",
        choices=("two-dot", "three-dot"),
        default="two-dot",
        help="Git diff mode: 'three-dot' (merge-base for PRs) or 'two-dot' (for pushes).",
    )
    parser.add_argument(
        "--force-full-workspace",
        action="store_true",
        help="Force full-workspace fallback regardless of changed paths.",
    )
    parser.add_argument(
        "--github-output",
        type=Path,
        default=None,
        help="Append GitHub Actions step outputs to this path.",
    )
    parser.add_argument(
        "--github-summary",
        type=Path,
        default=None,
        help="Append a Markdown summary to this path.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    repo_root = args.repo_root.resolve()
    graph = load_workspace_graph(repo_root)

    force_full = args.force_full_workspace
    force_reason: str | None = None
    changed: list[str] = []

    if args.files is not None:
        changed.extend(args.files)
    if args.files_file is not None:
        with args.files_file.open("r", encoding="utf-8") as handle:
            changed.extend(line.strip() for line in handle if line.strip())
    if args.base is not None:
        diff_files, fallback_reason = git_changed_files(
            repo_root, args.base, args.head, diff_mode=args.diff_mode
        )
        changed.extend(diff_files)
        if fallback_reason is not None:
            force_full = True
            force_reason = fallback_reason

    plan = compute_plan(
        changed,
        graph,
        force_full_workspace=force_full,
        force_reason=force_reason,
    )

    if args.github_output is not None:
        write_github_output(args.github_output, plan)
    if args.github_summary is not None:
        write_github_summary(args.github_summary, plan)

    print(json.dumps(plan, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
