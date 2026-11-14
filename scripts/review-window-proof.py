#!/usr/bin/env python3
"""Live proof of escrow release with a review window, on Stellar testnet.

Deploys the escrow contract from this repository and runs, with native XLM:

  A. seller requests release; the window ends; a third party executes; seller paid
  B. seller requests; buyer approves early; seller paid at once
  C. seller requests; buyer disputes inside the window; executing is refused;
     the arbiter rules for the buyer, who is refunded
  D. milestones: 30% requested and executed, then 70% requested and approved
  E. seller requests just before the deadline; after the deadline the buyer's
     refund is refused (ReleasePending); after the window the seller is paid
  F. the seller tries the old direct `release` on a funded escrow: refused

then checks that the contract holds nothing extra. Writes a JSON receipt.

Needs the Stellar CLI with funded identities (default prefix `forge-oz`:
forge-oz-buyer, forge-oz-seller, forge-oz-arbiter) and the built WASM:

  stellar contract build --package soroban-forge-escrow
  python3 scripts/review-window-proof.py \\
      --wasm target/wasm32v1-none/release/soroban_forge_escrow.wasm \\
      --receipt docs/testnet/review-window-receipt.json

Every transaction is signed by the Stellar CLI; no secret key is read here.
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys
import time
from datetime import datetime, timezone

NET = ["--network", "testnet"]
EXPLORER = "https://stellar.expert/explorer/testnet/tx/"
XLM = 10_000_000  # stroops per XLM
REVIEW = 60

p = argparse.ArgumentParser()
p.add_argument("--wasm", required=True)
p.add_argument("--receipt", required=True)
p.add_argument("--prefix", default="forge-oz")
args = p.parse_args()

ROLES = ("buyer", "seller", "arbiter")
ident = {r: f"{args.prefix}-{r}" for r in ROLES}
addr = {
    r: subprocess.run(["stellar", "keys", "address", ident[r]], check=True, capture_output=True, text=True).stdout.strip()
    for r in ROLES
}
token = subprocess.run(["stellar", "contract", "id", "asset", "--asset", "native", *NET],
                       check=True, capture_output=True, text=True).stdout.strip()
with open(args.wasm, "rb") as f:
    wasm_sha = hashlib.sha256(f.read()).hexdigest()
commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()

receipt = {
    "network": "testnet",
    "date": datetime.now(timezone.utc).strftime("%Y-%m-%d"),
    "token": {"asset": "native XLM", "contract": token},
    "wasm_sha256": wasm_sha,
    "built_from": commit,
    "review_period_seconds": REVIEW,
    "roles": addr,
    "steps": [],
}


def cli(argv, expect_ok=True):
    out = subprocess.run(argv, capture_output=True, text=True)
    m = re.search(r"([0-9a-f]{64})", out.stderr)
    if expect_ok and out.returncode != 0:
        sys.exit(f"command failed: {' '.join(argv[:8])}…\n{out.stderr[-1500:]}")
    return out.returncode == 0, out.stdout.strip(), m.group(1) if m else None, out.stderr


def invoke(role, fn, *kv, expect_ok=True, send=True):
    argv = ["stellar", "contract", "invoke", "--id", escrow, "--source", ident[role], *NET]
    if not send:
        argv += ["--send", "no"]
    return cli(argv + ["--", fn, *kv], expect_ok)


def balance(who):
    argv = ["stellar", "contract", "invoke", "--id", token, "--source", ident["buyer"], *NET, "--send", "no",
            "--", "balance", "--id", who]
    return int(cli(argv)[1].strip('"'))


def record(eid):
    """The escrow record: `released` and `status` say exactly what was paid,
    independent of the transaction fees the parties spend in XLM."""
    _, out, _, _ = invoke("buyer", "get_escrow", "--escrow_id", eid, send=False)
    r = json.loads(out)
    return int(r["released"]), r["status"] if isinstance(r["status"], str) else list(r["status"])[0]


def custody():
    return balance(escrow) - held_before


def error_of(err):
    found = re.findall(r"Error\((?:Contract, #\d+|Auth, \w+)\)", err)
    return found[0] if found else ("auth" if "auth" in err.lower() else "refused")


def step(what, **data):
    receipt["steps"].append({"what": what, **data})
    print("✓", what, json.dumps(data))


def tx(h):
    return EXPLORER + h if h else None


def ledger_time():
    """Latest closed ledger's close time, from Horizon."""
    import urllib.request
    with urllib.request.urlopen("https://horizon-testnet.stellar.org/ledgers?order=desc&limit=1") as r:
        rec = json.load(r)["_embedded"]["records"][0]
    return int(datetime.strptime(rec["closed_at"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc).timestamp())


def wait_until(t):
    while ledger_time() < t:
        time.sleep(5)


# ------------------------------------------------------------------ deploy
ok, out, h, _ = cli(["stellar", "contract", "deploy", "--wasm", args.wasm, "--source", ident["buyer"], *NET])
escrow = out.splitlines()[-1]
receipt["escrow"] = escrow
step("deploy the escrow with review windows", contract=escrow, tx=tx(h))
held_before = balance(escrow)


def funded(amount, timeout=86400):
    _, out, h1, _ = invoke("buyer", "create_escrow_with_review", "--buyer", addr["buyer"], "--seller", addr["seller"],
                           "--arbiter", addr["arbiter"], "--token", token, "--amount", str(amount),
                           "--timeout", str(timeout), "--review_period", str(REVIEW))
    eid = out.strip('"')
    _, _, h2, _ = invoke("buyer", "deposit", "--escrow_id", eid)
    return eid, tx(h1), tx(h2)


# ------------------------------------------------------------------ A
eid, c, d = funded(10 * XLM)
_, rel_at, hr, _ = invoke("seller", "request_release", "--escrow_id", eid, "--amount", str(10 * XLM))
rel_at = int(rel_at.strip('"'))
early_ok, _, _, early_err = invoke("arbiter", "execute_release", "--escrow_id", eid, expect_ok=False)
held_early = custody()
wait_until(rel_at)
_, _, he, _ = invoke("arbiter", "execute_release", "--escrow_id", eid)
released, status = record(eid)
step("A. request, window ends, a third party executes", escrow_id=eid, create=c, deposit=d, request=tx(hr),
     execute=tx(he), execute_before_window_refused=not early_ok, early_error=error_of(early_err),
     contract_still_held_xlm_before_window=held_early / XLM, released_xlm=released / XLM, status=status,
     ok=(not early_ok) and held_early == 10 * XLM and released == 10 * XLM and status == "Completed" and custody() == 0)

# ------------------------------------------------------------------ B
eid, c, d = funded(10 * XLM)
_, _, hr, _ = invoke("seller", "request_release", "--escrow_id", eid, "--amount", str(10 * XLM))
_, _, ha, _ = invoke("buyer", "approve_release", "--escrow_id", eid)
released, status = record(eid)
step("B. request, buyer approves early", escrow_id=eid, create=c, deposit=d, request=tx(hr), approve=tx(ha),
     released_xlm=released / XLM, status=status,
     ok=released == 10 * XLM and status == "Completed" and custody() == 0)

# ------------------------------------------------------------------ C
eid, c, d = funded(10 * XLM)
_, rel_at, hr, _ = invoke("seller", "request_release", "--escrow_id", eid, "--amount", str(10 * XLM))
rel_at = int(rel_at.strip('"'))
_, _, hd, _ = invoke("buyer", "dispute", "--escrow_id", eid, "--claimant", addr["buyer"])
wait_until(rel_at)
ex_ok, _, _, ex_err = invoke("seller", "execute_release", "--escrow_id", eid, expect_ok=False)
_, _, hz, _ = invoke("arbiter", "resolve", "--escrow_id", eid, "--in_favor_of_seller", "false")
_, status = record(eid)
step("C. request, buyer disputes, executing is refused, arbiter refunds the buyer", escrow_id=eid, create=c,
     deposit=d, request=tx(hr), dispute=tx(hd), resolve=tx(hz), execute_after_dispute_refused=not ex_ok,
     execute_error=error_of(ex_err), status=status,
     ok=(not ex_ok) and status == "Refunded" and custody() == 0)

# ------------------------------------------------------------------ D
eid, c, d = funded(10 * XLM)
_, rel_at, h1, _ = invoke("seller", "request_release", "--escrow_id", eid, "--amount", str(3 * XLM))
wait_until(int(rel_at.strip('"')))
_, _, h2, _ = invoke("seller", "execute_release", "--escrow_id", eid)
after_first, status_mid = record(eid)
_, _, h3, _ = invoke("seller", "request_release", "--escrow_id", eid, "--amount", str(7 * XLM))
_, _, h4, _ = invoke("buyer", "approve_release", "--escrow_id", eid)
total, status_end = record(eid)
step("D. milestones: 3 XLM after the window, then 7 XLM approved", escrow_id=eid, create=c, deposit=d,
     requests=[tx(h1), tx(h3)], execute=tx(h2), approve=tx(h4), released_after_first_xlm=after_first / XLM,
     status_after_first=status_mid, released_at_end_xlm=total / XLM, status_at_end=status_end,
     ok=after_first == 3 * XLM and status_mid == "Funded" and total == 10 * XLM and status_end == "Completed"
     and custody() == 0)

# ------------------------------------------------------------------ E
TIMEOUT = 40
eid, c, d = funded(10 * XLM, timeout=TIMEOUT)
_, esc_json, _, _ = invoke("buyer", "get_escrow", "--escrow_id", eid, send=False)
deadline = int(json.loads(esc_json)["created_at"]) + TIMEOUT
_, rel_at, hr, _ = invoke("seller", "request_release", "--escrow_id", eid, "--amount", str(10 * XLM))
rel_at = int(rel_at.strip('"'))
wait_until(deadline + 1)
rf_ok, _, _, rf_err = invoke("buyer", "refund", "--escrow_id", eid, expect_ok=False)
rx_ok, _, _, rx_err = invoke("buyer", "refund_expired", "--escrow_id", eid, expect_ok=False)
wait_until(rel_at)
_, _, he, _ = invoke("seller", "execute_release", "--escrow_id", eid)
released, status = record(eid)
step("E. after the deadline the buyer cannot refund around a pending request", escrow_id=eid, create=c, deposit=d,
     request=tx(hr), execute=tx(he), deadline=deadline, release_at=rel_at,
     refund_refused=not rf_ok, refund_error=error_of(rf_err),
     refund_expired_refused=not rx_ok, refund_expired_error=error_of(rx_err),
     released_xlm=released / XLM, status=status,
     ok=(not rf_ok) and (not rx_ok) and "#16" in error_of(rf_err) and "#16" in error_of(rx_err)
     and released == 10 * XLM and status == "Completed" and custody() == 0)

# ------------------------------------------------------------------ F
eid, c, d = funded(10 * XLM)
rl_ok, _, _, rl_err = invoke("seller", "release", "--escrow_id", eid, expect_ok=False)
released_f, status_f = record(eid)
step("F. the seller cannot pay themselves with a direct release", escrow_id=eid, create=c, deposit=d,
     release_refused=not rl_ok, error=error_of(rl_err), released_xlm=released_f / XLM, status=status_f,
     ok=(not rl_ok) and released_f == 0 and status_f == "Funded" and custody() == 10 * XLM)
_, _, hb, _ = invoke("buyer", "release", "--escrow_id", eid)
released, status = record(eid)
step("F2. the buyer can still pay directly", escrow_id=eid, release=tx(hb), released_xlm=released / XLM,
     status=status, ok=released == 10 * XLM and status == "Completed" and custody() == 0)

# ------------------------------------------------------------------ custody
held = custody()
step("the contract holds nothing extra after every round", held_change=held, ok=held == 0)

receipt["all_ok"] = all(s.get("ok", True) for s in receipt["steps"])
with open(args.receipt, "w") as f:
    json.dump(receipt, f, indent=2)
    f.write("\n")
print("receipt written:", args.receipt, "all_ok =", receipt["all_ok"])
sys.exit(0 if receipt["all_ok"] else 1)
