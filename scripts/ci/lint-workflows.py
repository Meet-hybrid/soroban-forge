#!/usr/bin/env python3
"""Static linter for GitHub Actions workflow files in `.github/workflows/`.

Checks YAML validity (including duplicate-key rejection), required GitHub
Actions structure, job dependency graphs (`needs`), step contracts, action
version pinning, `${{ ... }}` expression syntax and `needs.*.outputs.*`
references, security hygiene, and inline bash syntax (`bash -n`). Runs
`actionlint` as an additional pass when available on PATH.
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import yaml

VALID_PERMISSION_SCOPES = frozenset(
    {
        "actions",
        "attestations",
        "checks",
        "contents",
        "deployments",
        "discussions",
        "id-token",
        "issues",
        "models",
        "packages",
        "pages",
        "pull-requests",
        "repository-projects",
        "security-events",
        "statuses",
    }
)
VALID_PERMISSION_VALUES = frozenset({"read", "write", "none"})
JOB_ID_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_-]*$")
EXPR_RE = re.compile(r"\$\{\{(.*?)\}\}", re.DOTALL)
NEEDS_OUTPUT_RE = re.compile(
    r"\bneeds\.([A-Za-z_][A-Za-z0-9_-]*)\.outputs\.([A-Za-z_][A-Za-z0-9_-]*)\b"
)
NEEDS_RESULT_RE = re.compile(r"\bneeds\.([A-Za-z_][A-Za-z0-9_-]*)\.result\b")
UNSAFE_RUN_INTERPOLATIONS = (
    "github.event.pull_request.title",
    "github.event.pull_request.body",
    "github.event.pull_request.head.ref",
    "github.event.pull_request.head.label",
    "github.event.issue.title",
    "github.event.issue.body",
    "github.event.comment.body",
    "github.head_ref",
)


class DuplicateKeyError(yaml.YAMLError):
    pass


class UniqueKeyLoader(yaml.SafeLoader):
    """SafeLoader that rejects duplicate keys in any YAML mapping."""


def _construct_mapping_unique(
    loader: UniqueKeyLoader, node: yaml.MappingNode, deep: bool = False
) -> dict:
    mapping: dict = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            mark = key_node.start_mark
            raise DuplicateKeyError(
                f"duplicate YAML key {key!r} at line {mark.line + 1}, column {mark.column + 1}"
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    _construct_mapping_unique,
)


def _iter_strings(obj: Any, path: str = "") -> list[tuple[str, str]]:
    found: list[tuple[str, str]] = []
    if isinstance(obj, str):
        found.append((path, obj))
    elif isinstance(obj, list):
        for idx, item in enumerate(obj):
            found.extend(_iter_strings(item, f"{path}[{idx}]"))
    elif isinstance(obj, dict):
        for key, value in obj.items():
            sub = f"{path}.{key}" if path else str(key)
            found.extend(_iter_strings(value, sub))
    return found


def _validate_permissions(perms: Any, location: str, errors: list[str]) -> None:
    if perms is None:
        return
    if isinstance(perms, str):
        if perms == "write-all":
            errors.append(f"{location}: 'write-all' permissions are forbidden")
        elif perms != "read-all":
            errors.append(f"{location}: invalid string permission {perms!r}")
        return
    if not isinstance(perms, dict):
        errors.append(f"{location}: permissions must be a mapping or 'read-all'")
        return
    for scope, access in perms.items():
        if scope not in VALID_PERMISSION_SCOPES:
            errors.append(f"{location}: unknown permission scope {scope!r}")
        if access not in VALID_PERMISSION_VALUES:
            errors.append(
                f"{location}: invalid permission level {access!r} for scope {scope!r}"
            )


def _normalize_needs(raw_needs: Any, job_id: str, errors: list[str]) -> list[str]:
    if raw_needs is None:
        return []
    if isinstance(raw_needs, str):
        return [raw_needs]
    if isinstance(raw_needs, list) and all(isinstance(x, str) for x in raw_needs):
        return list(raw_needs)
    errors.append(f"jobs.{job_id}.needs: must be a string or list of strings")
    return []


def _check_cycles(needs_map: dict[str, list[str]], errors: list[str]) -> None:
    state: dict[str, int] = {}  # 0=unvisited, 1=visiting, 2=done

    def dfs(node: str, trail: list[str]) -> None:
        state[node] = 1
        for dep in needs_map.get(node, []):
            if dep not in needs_map:
                continue
            if state.get(dep, 0) == 1:
                cycle = " -> ".join(trail + [dep])
                errors.append(f"circular job dependency detected: {cycle}")
            elif state.get(dep, 0) == 0:
                dfs(dep, trail + [dep])
        state[node] = 2

    for job_id in needs_map:
        if state.get(job_id, 0) == 0:
            dfs(job_id, [job_id])


def _validate_expressions(
    text: str,
    location: str,
    declared_needs: set[str],
    job_outputs: dict[str, set[str]],
    errors: list[str],
) -> None:
    opens = text.count("${{")
    closes = text.count("}}")
    if opens != closes:
        errors.append(
            f"{location}: unbalanced GitHub Actions expression delimiters (${{{{ vs }}}})"
        )
        return

    # Check both `${{ ... }}` expressions and bare `if:` condition strings
    candidates = [match.group(1) for match in EXPR_RE.finditer(text)]
    if location.endswith(".if") and not candidates:
        candidates = [text]

    for expr in candidates:
        for dep_job, out_key in NEEDS_OUTPUT_RE.findall(expr):
            if dep_job not in declared_needs:
                errors.append(
                    f"{location}: references needs.{dep_job}.outputs.{out_key} "
                    f"but '{dep_job}' is not listed in job 'needs'"
                )
            elif out_key not in job_outputs.get(dep_job, set()):
                errors.append(
                    f"{location}: references needs.{dep_job}.outputs.{out_key} "
                    f"which is not declared in jobs.{dep_job}.outputs"
                )
        for dep_job in NEEDS_RESULT_RE.findall(expr):
            if dep_job not in declared_needs:
                errors.append(
                    f"{location}: references needs.{dep_job}.result "
                    f"but '{dep_job}' is not listed in job 'needs'"
                )


def _check_bash_syntax(script: str, location: str, errors: list[str]) -> None:
    if "${{" in script:
        return
    proc = subprocess.run(
        ["bash", "-n"],
        input=script,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        detail = proc.stderr.strip().splitlines()[0] if proc.stderr.strip() else "syntax error"
        errors.append(f"{location}: bash syntax check failed: {detail}")


def lint_workflow_file(path: Path) -> list[str]:
    errors: list[str] = []
    try:
        raw_text = path.read_text(encoding="utf-8")
    except OSError as exc:
        return [f"unable to read file: {exc}"]

    try:
        doc = yaml.load(raw_text, Loader=UniqueKeyLoader)
    except yaml.YAMLError as exc:
        return [f"invalid YAML: {exc}"]

    if not isinstance(doc, dict):
        return ["workflow root must be a YAML mapping"]

    # Top-level name
    name = doc.get("name")
    if not isinstance(name, str) or not name.strip():
        errors.append("missing or empty top-level 'name'")

    # Top-level `on` (YAML 1.1 SafeLoader parses bare `on:` as boolean True)
    triggers = doc.get("on", doc.get(True))
    if not triggers:
        errors.append("missing or empty top-level 'on' trigger definition")

    has_pr_target = False
    if isinstance(triggers, str):
        has_pr_target = triggers == "pull_request_target"
    elif isinstance(triggers, list):
        has_pr_target = "pull_request_target" in triggers
    elif isinstance(triggers, dict):
        has_pr_target = "pull_request_target" in triggers

    _validate_permissions(doc.get("permissions"), "permissions", errors)

    jobs = doc.get("jobs")
    if not isinstance(jobs, dict) or not jobs:
        errors.append("missing or empty top-level 'jobs' mapping")
        return errors

    job_outputs: dict[str, set[str]] = {}
    needs_map: dict[str, list[str]] = {}

    for job_id, job in jobs.items():
        if not isinstance(job_id, str) or not JOB_ID_RE.match(job_id):
            errors.append(f"invalid job id {job_id!r}")
            continue
        if not isinstance(job, dict):
            errors.append(f"jobs.{job_id}: must be a mapping")
            continue

        outputs = job.get("outputs")
        if outputs is not None:
            if isinstance(outputs, dict):
                job_outputs[job_id] = {str(k) for k in outputs.keys()}
            else:
                errors.append(f"jobs.{job_id}.outputs: must be a mapping")
                job_outputs[job_id] = set()
        else:
            job_outputs[job_id] = set()

        needs_map[job_id] = _normalize_needs(job.get("needs"), job_id, errors)

    for job_id, deps in needs_map.items():
        for dep in deps:
            if dep not in jobs:
                errors.append(f"jobs.{job_id}.needs: references unknown job {dep!r}")
    _check_cycles(needs_map, errors)

    for job_id, job in jobs.items():
        if not isinstance(job, dict):
            continue

        _validate_permissions(
            job.get("permissions"), f"jobs.{job_id}.permissions", errors
        )

        declared_needs = set(needs_map.get(job_id, []))
        for field_path, text in _iter_strings(job, f"jobs.{job_id}"):
            _validate_expressions(
                text, field_path, declared_needs, job_outputs, errors
            )

        is_reusable = isinstance(job.get("uses"), str)
        if is_reusable:
            continue

        if "runs-on" not in job:
            errors.append(f"jobs.{job_id}: missing 'runs-on'")

        steps = job.get("steps")
        if not isinstance(steps, list) or not steps:
            errors.append(f"jobs.{job_id}: must define at least one step")
            continue

        for idx, step in enumerate(steps):
            step_loc = f"jobs.{job_id}.steps[{idx}]"
            if not isinstance(step, dict):
                errors.append(f"{step_loc}: step must be a mapping")
                continue

            has_uses = "uses" in step
            has_run = "run" in step
            if has_uses == has_run:
                errors.append(f"{step_loc}: step must set exactly one of 'uses' or 'run'")
                continue

            if has_uses:
                uses = step["uses"]
                if not isinstance(uses, str) or not uses.strip():
                    errors.append(f"{step_loc}.uses: must be a non-empty string")
                elif not uses.startswith("./") and not uses.startswith("docker://"):
                    if "@" not in uses or uses.endswith("@"):
                        errors.append(
                            f"{step_loc}.uses: action {uses!r} must pin a ref with '@'"
                        )
                if has_pr_target and isinstance(uses, str) and uses.startswith("actions/checkout@"):
                    step_with = step.get("with") or {}
                    ref_val = str(step_with.get("ref", ""))
                    if "pull_request.head" in ref_val:
                        errors.append(
                            f"{step_loc}: pull_request_target must not check out untrusted PR head"
                        )

            if has_run:
                script = step["run"]
                if not isinstance(script, str) or not script.strip():
                    errors.append(f"{step_loc}.run: must be a non-empty string")
                else:
                    for expr_match in EXPR_RE.finditer(script):
                        expr_body = expr_match.group(1)
                        for unsafe in UNSAFE_RUN_INTERPOLATIONS:
                            if unsafe in expr_body:
                                errors.append(
                                    f"{step_loc}.run: untrusted expression '{unsafe}' "
                                    "must be passed through env:, not interpolated into run:"
                                )
                    shell = step.get("shell", "bash")
                    if shell in ("bash", "sh"):
                        _check_bash_syntax(script, f"{step_loc}.run", errors)

    return errors


def run_actionlint(files: list[Path], *, require: bool) -> list[str]:
    actionlint = shutil.which("actionlint")
    if actionlint is None:
        if require:
            return ["actionlint not found on PATH (--require-actionlint was set)"]
        return []
    proc = subprocess.run(
        [actionlint, *(str(f) for f in files)],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode == 0:
        return []
    output = (proc.stdout + "\n" + proc.stderr).strip()
    return [line for line in output.splitlines() if line.strip()]


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Lint GitHub Actions workflow files."
    )
    parser.add_argument(
        "files",
        nargs="*",
        type=Path,
        help="Specific workflow files to lint (default: all files in .github/workflows/).",
    )
    parser.add_argument(
        "--workflows-dir",
        type=Path,
        default=Path(__file__).resolve().parents[2] / ".github" / "workflows",
        help="Directory to scan when no explicit files are given.",
    )
    parser.add_argument(
        "--require-actionlint",
        action="store_true",
        help="Fail if actionlint is not installed on PATH.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.files:
        files = [f.resolve() for f in args.files]
    else:
        files = sorted(
            [
                *args.workflows_dir.glob("*.yml"),
                *args.workflows_dir.glob("*.yaml"),
            ]
        )

    if not files:
        print("Error: no workflow files found to lint", file=sys.stderr)
        return 1

    total_errors = 0
    for wf in files:
        errs = lint_workflow_file(wf)
        if errs:
            total_errors += len(errs)
            for err in errs:
                print(f"ERROR {wf}: {err}", file=sys.stderr)
        else:
            print(f"OK    {wf.name}")

    al_errs = run_actionlint(files, require=args.require_actionlint)
    if al_errs:
        total_errors += len(al_errs)
        for err in al_errs:
            print(f"ERROR actionlint: {err}", file=sys.stderr)

    if total_errors:
        print(f"\nWorkflow lint failed with {total_errors} error(s).", file=sys.stderr)
        return 1

    print(f"\nWorkflow lint passed ({len(files)} workflow file(s) checked).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
