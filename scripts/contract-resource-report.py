#!/usr/bin/env python3
"""Generate and compare deterministic Soroban contract resource reports."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
REPORT_MARKER = "CONTRACT_RESOURCE_REPORT:"
SCHEMA = "soroban-forge/contract-resource-report@1"
BASELINE_PATH = ROOT / "reports" / "contract-resource-baseline.json"
MAX_REGRESSION_PERCENT = 20
SCENARIOS = {
    "soroban-forge-escrow": "escrow.create_escrow",
    "soroban-forge-vesting": "vesting.create_schedule",
    "soroban-forge-multi-sig-wallet": "multi-sig-wallet.initialize",
    "soroban-forge-dao-governance": "dao-governance.propose",
    "soroban-forge-subscription-payments": "subscription-payments.subscribe",
    "soroban-forge-marketplace-royalties": "marketplace-royalties.set_royalty",
}
METRICS = (
    "footprint_entries",
    "read_entries",
    "write_entries",
    "instructions",
    "memory_bytes",
    "transaction_fee_stroops",
)


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_report(path: Path) -> dict[str, Any]:
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"cannot read resource report {path}: {error}") from error
    if not isinstance(report, dict) or report.get("schema") != SCHEMA:
        raise SystemExit(f"{path} has a missing or unsupported report schema")
    scenarios = report.get("scenarios")
    if not isinstance(scenarios, dict) or set(scenarios) != set(SCENARIOS.values()):
        missing = sorted(set(SCENARIOS.values()) - set(scenarios or {}))
        unexpected = sorted(set(scenarios or {}) - set(SCENARIOS.values()))
        raise SystemExit(
            f"{path} must contain exactly the six expected scenarios "
            f"(missing: {missing}; unexpected: {unexpected})"
        )
    for scenario, values in scenarios.items():
        if not isinstance(values, dict):
            raise SystemExit(f"{path}: {scenario} metrics are missing")
        for metric in METRICS:
            value = values.get(metric)
            if type(value) is not int or value < 0:
                raise SystemExit(
                    f"{path}: {scenario}.{metric} is missing or invalid: {value!r}"
                )
    return report


def generate_report() -> dict[str, Any]:
    scenarios: dict[str, dict[str, int]] = {}
    for package, scenario in SCENARIOS.items():
        command = [
            "cargo",
            "test",
            "--locked",
            "-p",
            package,
            "--lib",
            "resource_report::resource_report_scenario",
            "--",
            "--exact",
            "--nocapture",
        ]
        result = subprocess.run(
            command,
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
        if result.returncode != 0:
            print(result.stdout, file=sys.stderr)
            raise SystemExit(
                f"resource scenario {scenario} failed with exit code {result.returncode}"
            )

        records = []
        for line in result.stdout.splitlines():
            if REPORT_MARKER in line:
                payload = line.split(REPORT_MARKER, 1)[1]
                try:
                    records.append(json.loads(payload))
                except json.JSONDecodeError as error:
                    raise SystemExit(
                        f"resource scenario {scenario} emitted malformed metrics: {error}"
                    ) from error
        if len(records) != 1:
            raise SystemExit(
                f"resource scenario {scenario} emitted {len(records)} records; expected exactly one"
            )
        record = records[0]
        if record.get("scenario") != scenario:
            raise SystemExit(
                f"resource scenario {scenario} emitted unexpected name "
                f"{record.get('scenario')!r}"
            )
        scenarios[scenario] = record.get("metrics")

    report = {"schema": SCHEMA, "scenarios": scenarios}
    validate_report(report)
    return report


def validate_report(report: dict[str, Any]) -> None:
    for scenario, values in report["scenarios"].items():
        if not isinstance(values, dict):
            raise SystemExit(f"{scenario} metrics are missing")
        for metric in METRICS:
            value = values.get(metric)
            if type(value) is not int or value < 0:
                raise SystemExit(f"{scenario}.{metric} is missing or invalid: {value!r}")


def compare_reports(
    baseline: dict[str, Any], current: dict[str, Any]
) -> tuple[dict[str, Any], bool]:
    comparison: dict[str, Any] = {
        "schema": "soroban-forge/contract-resource-comparison@1",
        "max_regression_percent": MAX_REGRESSION_PERCENT,
        "scenarios": {},
    }
    failed = False
    lines = [
        f"## Contract resource report (maximum regression: {MAX_REGRESSION_PERCENT}%)",
        "",
        "| Entrypoint | Metric | Base | PR | Delta | Result |",
        "|---|---|---:|---:|---:|---|",
    ]
    print(f"Resource regression threshold: {MAX_REGRESSION_PERCENT}% per metric.")
    for scenario in sorted(SCENARIOS.values()):
        base_metrics = baseline["scenarios"][scenario]
        current_metrics = current["scenarios"][scenario]
        metric_results: dict[str, Any] = {}
        for metric in METRICS:
            base = base_metrics[metric]
            actual = current_metrics[metric]
            delta = actual - base
            if base == 0:
                delta_percent = 0.0 if actual == 0 else None
                regressed = actual > 0
            else:
                delta_percent = round(delta * 100 / base, 2)
                regressed = actual * 100 > base * (100 + MAX_REGRESSION_PERCENT)
            failed = failed or regressed
            result = "FAIL" if regressed else "pass"
            delta_display = (
                f"{delta_percent:+.2f}%" if delta_percent is not None else "new usage"
            )
            print(
                f"{scenario} {metric}: base={base} pr={actual} "
                f"delta={delta_display} [{result}]"
            )
            lines.append(
                f"| `{scenario}` | `{metric}` | {base} | {actual} | "
                f"{delta_display} | {result} |"
            )
            metric_results[metric] = {
                "baseline": base,
                "current": actual,
                "delta": delta,
                "delta_percent": delta_percent,
                "regression": regressed,
            }
        comparison["scenarios"][scenario] = metric_results

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with Path(summary).open("a", encoding="utf-8") as output:
            output.write("\n".join(lines) + "\n")
    return comparison, failed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    generate = subparsers.add_parser("generate", help="run scenarios and write a report")
    generate.add_argument("--output", type=Path, required=True)

    compare = subparsers.add_parser("compare", help="compare current metrics with a baseline")
    compare.add_argument("--baseline", type=Path, required=True)
    compare.add_argument("--current", type=Path, required=True)
    compare.add_argument("--output", type=Path, required=True)

    subparsers.add_parser("update-baseline", help="regenerate the checked-in baseline")
    args = parser.parse_args()

    if args.command == "generate":
        report = generate_report()
        write_json(args.output, report)
        print(f"Wrote resource report: {args.output}")
        return 0
    if args.command == "compare":
        baseline = read_report(args.baseline)
        current = read_report(args.current)
        comparison, failed = compare_reports(baseline, current)
        write_json(args.output, comparison)
        print(f"Wrote resource comparison: {args.output}")
        return 1 if failed else 0

    report = generate_report()
    write_json(BASELINE_PATH, report)
    print(f"Updated checked-in resource baseline: {BASELINE_PATH}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
