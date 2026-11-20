#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Soroban Forge — WASM provenance manifest generator/verifier.
#
# Produces `provenance-manifest.json`: a SHA-256 manifest of every contract
# WASM artifact produced by the pinned toolchain at a named git revision.
# Reviewers and integrators can reproduce the build and confirm the deployed
# WASM hash matches the source they are reading.
#
# Usage:
#   scripts/provenance.sh build              # build artifacts + write manifest
#   scripts/provenance.sh verify [MANIFEST]  # rebuild + compare hashes (exit 1 on drift)
#
# The manifest is intentionally dependency-free (sha256sum + awk), so it runs
# in CI and on a plain checkout with no jq/python requirement.
#
# NOTE: keep PACKAGES in sync with the contract crates in the wasm-size job
# in .github/workflows/ci.yml.
# ---------------------------------------------------------------------------
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET="wasm32v1-none"
MANIFEST="${2:-$ROOT/provenance-manifest.json}"

PACKAGES=(
  soroban-forge-escrow
  soroban-forge-vesting
  soroban-forge-multi-sig-wallet
  soroban-forge-dao-governance
  soroban-forge-subscription-payments
  soroban-forge-marketplace-royalties
)

build_artifacts() {
  # soroban-sdk 28 requires a spec-shaking build system (stellar-cli 25.2+);
  # plain `cargo build --target wasm32v1-none` is rejected by the SDK build
  # script. The workspace's cdylib crates are exactly the contract crates,
  # so a workspace-wide build covers the artifact set. --optimize=false
  # keeps the output deterministic (no wasm-opt in the hash chain) while
  # staying far below the 150KB size budget.
  stellar contract build --locked --profile release --optimize=false
}

write_manifest() {
  local rev generated_at
  rev="$(git rev-parse HEAD 2>/dev/null || echo "unknown")"
  generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

  {
    printf '{\n'
    printf '  "schema": "soroban-forge/provenance@1",\n'
    printf '  "git_commit": "%s",\n' "$rev"
    printf '  "generated_at": "%s",\n' "$generated_at"
    printf '  "target": "%s",\n' "$TARGET"
    printf '  "artifacts": [\n'
    local first=1
    # Glob is alphabetical, so the artifact list is deterministic per tree.
    for wasm in target/"$TARGET"/release/*.wasm; do
      local name hash bytes
      name="$(basename "$wasm")"
      hash="$(sha256sum "$wasm" | cut -d' ' -f1)"
      bytes="$(stat -c%s "$wasm")"
      if [ "$first" -eq 1 ]; then
        first=0
      else
        printf ',\n'
      fi
      printf '    { "name": "%s", "sha256": "%s", "bytes": %s }' "$name" "$hash" "$bytes"
    done
    printf '\n  ]\n'
    printf '}\n'
  } > "$MANIFEST"

  echo "wrote $MANIFEST"
  cat "$MANIFEST"
}

verify_manifest() {
  local failures=0
  # Each artifact is a single line: { "name": "...", "sha256": "...", "bytes": N }
  while IFS= read -r line; do
    local name hash actual
    name="$(printf '%s' "$line" | sed -n 's/.*"name": "\([^"]*\)".*/\1/p')"
    hash="$(printf '%s' "$line" | sed -n 's/.*"sha256": "\([a-f0-9]*\)".*/\1/p')"
    if [ -z "$name" ] || [ -z "$hash" ]; then
      continue
    fi
    if [ ! -f "target/$TARGET/release/$name" ]; then
      echo "MISSING artifact: $name"
      failures=$((failures + 1))
      continue
    fi
    actual="$(sha256sum "target/$TARGET/release/$name" | cut -d' ' -f1)"
    if [ "$actual" = "$hash" ]; then
      echo "OK       $name $hash"
    else
      echo "MISMATCH $name manifest=$hash actual=$actual"
      failures=$((failures + 1))
    fi
  done < <(grep '"sha256"' "$MANIFEST")

  if [ "$failures" -ne 0 ]; then
    echo "provenance verification FAILED: $failures artifact(s) drifted"
    exit 1
  fi
  echo "provenance verification passed"
}

case "${1:-}" in
  build)
    build_artifacts
    write_manifest
    ;;
  verify)
    build_artifacts
    verify_manifest
    ;;
  *)
    echo "usage: $0 {build|verify} [MANIFEST_PATH]" >&2
    exit 2
    ;;
esac

# ---------------------------------------------------------------------------
# Soroban Forge — deployment-to-source provenance attestation bundles.
# ---------------------------------------------------------------------------
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET="wasm32v1-none"
MANIFEST="${MANIFEST:-$ROOT/provenance-manifest.json}"
DEPLOYMENT_RECEIPT="${DEPLOYMENT_RECEIPT:-}"
JSON_OUTPUT="${PROVENANCE_JSON:-0}"
NETWORK_VERIFY="0"

require_command() {
  command -v "$1" >/dev/null 2>&1 || { echo "required command not found: $1" >&2; exit 127; }
}

build_artifacts() {
  require_command stellar
  stellar contract build --locked --profile release --optimize=false
}

artifact_rows() {
  python3 - "$TARGET" <<'PY'
import glob, hashlib, json, os, sys
for path in sorted(glob.glob(os.path.join("target", sys.argv[1], "release", "*.wasm"))):
    data = open(path, "rb").read()
    print(json.dumps({
        "name": os.path.basename(path),
        "sha256": hashlib.sha256(data).hexdigest(),
        "bytes": len(data),
    }, sort_keys=True, separators=(",", ":")))
PY
}

write_manifest() {
  require_command python3
  require_command sha256sum

  local rev rustc_version cargo_version stellar_version generated_at lock_sha source_dirty
  rev="$(git rev-parse HEAD 2>/dev/null || echo "unknown")"
  generated_at="$(git show -s --format=%cI HEAD 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ)"
  source_dirty="false"
  if ! git diff --quiet || ! git diff --cached --quiet; then source_dirty="true"; fi

  rustc_version="$(rustc --version 2>/dev/null | awk '{print $2}' || echo "unknown")"
  cargo_version="$(cargo --version 2>/dev/null | awk '{print $2}' || echo "unknown")"
  stellar_version="$(stellar --version 2>/dev/null | awk '{print $2}' || echo "unknown")"
  lock_sha="$(sha256sum Cargo.lock | awk '{print $1}')"

  python3 - "$MANIFEST" "$rev" "$generated_at" "$source_dirty" "$rustc_version" "$cargo_version" "$stellar_version" "$lock_sha" "$TARGET" "$DEPLOYMENT_RECEIPT" <<'PY'
import glob, hashlib, json, os, sys

manifest_path, rev, generated_at, source_dirty, rustc_version, cargo_version, stellar_version, lock_sha, target, receipt = sys.argv[1:]
artifacts = []
for path in sorted(glob.glob(os.path.join("target", target, "release", "*.wasm"))):
    data = open(path, "rb").read()
    artifacts.append({"name": os.path.basename(path), "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)})
if not artifacts:
    raise SystemExit("no WASM artifacts found")

deployments = []
if receipt and os.path.isfile(receipt):
    doc = json.load(open(receipt, encoding="utf-8"))
    network = doc.get("network")
    rpc_url = os.environ.get("STELLAR_RPC_URL") or os.environ.get("SOROBAN_RPC_URL")
    passphrase = os.environ.get("NETWORK_PASSPHRASE")
    for contract in doc.get("contracts", []):
        wasm = contract.get("wasm") or {}
        deployments.append({
            "network": network,
            "contract_id": contract.get("contract_id") if contract.get("contract_id") not in (None, "configured-at-runtime") else None,
            "contract_name": contract.get("name") or contract.get("key"),
            "artifact": wasm.get("path") or "",
            "artifact_sha256": wasm.get("sha256"),
            "rpc_url": rpc_url,
            "network_passphrase": passphrase,
            "deployment_transaction": contract.get("deployment_transaction") or None,
        })

bundle = {
    "schema": "soroban-forge/provenance@2",
    "git_commit": rev,
    "source_revision": rev,
    "source": {"revision": rev, "dirty": source_dirty == "true"},
    "generated_at": generated_at,
    "target": target,
    "toolchain": {
        "rustc": rustc_version,
        "cargo": cargo_version,
        "stellar_cli": stellar_version,
        "target": target,
        "profile": "release",
    },
    "cli_version": stellar_version,
    "lockfile": {"path": "Cargo.lock", "sha256": lock_sha},
    "lockfile_sha256": lock_sha,
    "artifacts": artifacts,
    "deployments": deployments,
}
canonical = json.dumps(bundle, sort_keys=True, separators=(",", ":")).encode()
bundle["bundle_sha256"] = hashlib.sha256(canonical).hexdigest()

os.makedirs(os.path.dirname(manifest_path) or ".", exist_ok=True)
with open(manifest_path, "w", encoding="utf-8") as f:
    json.dump(bundle, f, indent=2, sort_keys=True)
    f.write("\n")
PY

  echo "wrote $MANIFEST"
  cat "$MANIFEST"
}

verify_manifest() {
  require_command python3
  require_command sha256sum

  python3 - "$MANIFEST" "$TARGET" "$NETWORK_VERIFY" <<'PY'
import glob, hashlib, json, os, subprocess, sys

manifest_path, target, network_verify = sys.argv[1:]
bundle = json.load(open(manifest_path, encoding="utf-8"))
failures = []

if bundle.get("schema") != "soroban-forge/provenance@2":
    failures.append("unsupported manifest schema")

bundle_copy = dict(bundle)
recorded_integrity = bundle_copy.pop("bundle_sha256", None)
canonical = json.dumps(bundle_copy, sort_keys=True, separators=(",", ":")).encode()
actual_integrity = hashlib.sha256(canonical).hexdigest()
if recorded_integrity != actual_integrity:
    failures.append("bundle_sha256 mismatch (manifest was tampered)")

rev = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
if rev and bundle.get("source_revision") != rev:
    failures.append("source revision mismatch: manifest=%s checkout=%s" % (bundle.get("source_revision"), rev))

lock_sha = hashlib.sha256(open("Cargo.lock", "rb").read()).hexdigest()
if bundle.get("lockfile_sha256") != lock_sha:
    failures.append("Cargo.lock hash mismatch")

for artifact in bundle.get("artifacts", []):
    path = os.path.join("target", target, "release", artifact["name"])
    if not os.path.isfile(path):
        failures.append("MISSING " + artifact["name"])
        continue
    data = open(path, "rb").read()
    actual = hashlib.sha256(data).hexdigest()
    if actual != artifact.get("sha256"):
        failures.append("MISMATCH " + artifact["name"] + " manifest=" + str(artifact.get("sha256")) + " actual=" + actual)
    elif len(data) != artifact.get("bytes"):
        failures.append("SIZE MISMATCH " + artifact["name"])
    else:
        print("OK       " + artifact["name"] + " " + actual)

if network_verify == "1":
    for deployment in bundle.get("deployments", []):
        contract_id = deployment.get("contract_id")
        rpc_url = deployment.get("rpc_url")
        if not contract_id:
            continue
        if not rpc_url:
            failures.append("network verification missing RPC URL for " + str(contract_id))
            continue
        passphrase = deployment.get("network_passphrase")
        out = "/tmp/soroban-forge-provenance-%s.wasm" % contract_id
        cmd = ["stellar", "contract", "fetch", "--id", contract_id, "--rpc-url", rpc_url, "--out-file", out]
        if passphrase:
            cmd.extend(["--network-passphrase", passphrase])
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode:
            failures.append("network fetch failed for " + contract_id + ": " + result.stderr.strip())
            continue
        actual = hashlib.sha256(open(out, "rb").read()).hexdigest()
        expected = deployment.get("artifact_sha256")
        if actual != expected:
            failures.append("DEPLOYMENT MISMATCH " + str(contract_id) + " expected=" + str(expected) + " deployed=" + actual)
        else:
            print("DEPLOYED OK " + str(contract_id) + " " + actual)

if failures:
    print("provenance verification FAILED:", file=sys.stderr)
    for failure in failures:
        print("- " + failure, file=sys.stderr)
    raise SystemExit(1)
print("provenance verification passed")
PY

  if [ "$JSON_OUTPUT" = "1" ]; then
    python3 - "$MANIFEST" <<'PY'
import json, sys
m = json.load(open(sys.argv[1], encoding="utf-8"))
print(json.dumps({"ok": True, "schema": m.get("schema"), "source_revision": m.get("source_revision"), "bundle_sha256": m.get("bundle_sha256"), "artifacts": m.get("artifacts", []), "deployments": m.get("deployments", [])}, sort_keys=True))
PY
  fi
}

case "${1:-}" in
  build)
    shift
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --manifest) MANIFEST="$2"; shift 2 ;;
        *) MANIFEST="$1"; shift ;;
      esac
    done
    build_artifacts
    write_manifest
    ;;
  verify)
    shift
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --network) NETWORK_VERIFY="1"; shift ;;
        --json) JSON_OUTPUT="1"; shift ;;
        --manifest) MANIFEST="$2"; shift 2 ;;
        *) MANIFEST="$1"; shift ;;
      esac
    done
    build_artifacts
    verify_manifest
    ;;
  *)
    echo "usage: $0 build [MANIFEST_PATH] | verify [MANIFEST_PATH] [--network] [--json]" >&2
    exit 2
    ;;
esac
