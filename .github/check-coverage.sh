#!/usr/bin/env bash
set -euo pipefail

# Verifies 100% function/line/region/branch coverage across the crate.
# Requires nightly Rust + llvm-tools-preview and cargo-llvm-cov.
# Used by both the local pre-commit hook and CI to keep them in lockstep.

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo ""
  echo "ERROR: cargo-llvm-cov is not installed."
  echo "Install it with: cargo install cargo-llvm-cov"
  exit 1
fi

PYTHON_BIN=""
for candidate in python python3 py; do
  if command -v "$candidate" >/dev/null 2>&1; then
    PYTHON_BIN="$candidate"
    break
  fi
done
if [ -z "$PYTHON_BIN" ]; then
  echo ""
  echo "ERROR: No Python interpreter found (looked for python, python3, py)."
  echo "Install Python 3 to enable the coverage gate."
  exit 1
fi

COVERAGE_JSON="$(mktemp -t coverage.XXXXXX)"
trap 'rm -f "$COVERAGE_JSON"' EXIT

if ! cargo llvm-cov --branch --json --summary-only --output-path "$COVERAGE_JSON" >/dev/null; then
  echo ""
  echo "ERROR: cargo llvm-cov failed."
  echo "Branch coverage requires the nightly toolchain. Install it with:"
  echo "  rustup toolchain install nightly --component llvm-tools-preview"
  exit 1
fi

# We require 100% on functions, lines, regions and branches across the
# whole crate, checked on the report-wide totals. count == 0 on a given
# metric is treated as vacuously satisfied (e.g. a crate with no
# conditional branches has 0/0 branches and that is fine). As a sanity
# check, an entry whose *every* metric reports count == 0 is rejected:
# that is what a bogus report looks like (broken instrumentation, no
# tests collected, branch coverage silently disabled by a stable
# toolchain) and it must not pass.
if ! "$PYTHON_BIN" - "$COVERAGE_JSON" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as fh:
        data = json.load(fh)
except (OSError, ValueError) as exc:
    print(f"  FAIL  could not read coverage JSON: {exc}")
    sys.exit(1)

entries = data.get("data") or []
if not entries:
    print("  FAIL  coverage report contains no data entries")
    sys.exit(1)

metrics = ("functions", "lines", "regions", "branches")
failed = False

for index, entry in enumerate(entries):
    totals = entry.get("totals") or {}
    files = entry.get("files") or []
    if not files:
        print(f"  FAIL  entry {index} has no files")
        failed = True
        continue
    nonzero_metrics = 0
    for metric in metrics:
        bucket = totals.get(metric) or {}
        count = bucket.get("count", 0)
        covered = bucket.get("covered", 0)
        percent = bucket.get("percent", 0.0)
        if not isinstance(count, int) or not isinstance(covered, int):
            print(f"  FAIL  {metric}: malformed entry {bucket!r}")
            failed = True
            continue
        if count == 0:
            print(f"  OK    {metric}: 0/0 (vacuous)")
            continue
        nonzero_metrics += 1
        if covered != count:
            print(f"  FAIL  {metric}: {covered}/{count} ({percent:.4f}%)")
            failed = True
        else:
            print(f"  OK    {metric}: {covered}/{count} (100%)")
    if nonzero_metrics == 0:
        print(
            f"  FAIL  entry {index} reports count 0 on every metric "
            "(likely broken instrumentation or no tests collected)"
        )
        failed = True

sys.exit(1 if failed else 0)
PY
then
  echo ""
  echo "ERROR: Coverage is not 100%."
  echo "Run 'cargo llvm-cov --branch --html' for the HTML report (target/llvm-cov/html/index.html) and add the missing tests."
  exit 1
fi

echo "Coverage is 100%."
