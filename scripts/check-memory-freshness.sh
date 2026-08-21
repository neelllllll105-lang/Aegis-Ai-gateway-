#!/usr/bin/env bash
# Fail CI when the handoff documents have gone stale.
#
# `CLAUDE.md` states that an out-of-date MEMORY.md is a build failure. This is what makes
# that a real rule rather than an aspiration. A project whose handoff document has quietly
# rotted is one nobody can pick up — which is the specific failure this whole mechanism
# exists to prevent.
#
# The checks are deliberately loose. The goal is to catch neglect, not to nag: a
# documentation-only commit does not need a memory update, and neither does a typo fix.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

failures=0
warnings=0

# Find a Python interpreter that actually runs.
#
# `command -v python3` is not enough on Windows: the App Execution Alias shim exists on
# PATH and exits with an install prompt, so the only reliable test is to run it.
find_python() {
  for candidate in python3 python py; do
    if command -v "$candidate" >/dev/null 2>&1        && "$candidate" -c "import sys" >/dev/null 2>&1; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  return 1
}
PYTHON=$(find_python || true)


fail() { printf '::error::%s\n' "$1"; failures=$((failures + 1)); }
warn() { printf '::warning::%s\n' "$1"; warnings=$((warnings + 1)); }
pass() { printf '  ok: %s\n' "$1"; }

printf 'Checking handoff document freshness...\n\n'

# --- The documents must exist -----------------------------------------------------
for doc in MEMORY.md MASTER_BUILD.md CLAUDE.md docs/PHASES.md .aegis/state.json; do
  if [ ! -f "$doc" ]; then
    fail "$doc is missing. It is part of the handoff contract."
  else
    pass "$doc exists"
  fi
done

# --- MEMORY.md must not lag far behind the code -------------------------------------
if [ -f MEMORY.md ] && git rev-parse --git-dir >/dev/null 2>&1; then
  # How many commits touching real source have landed since MEMORY.md was last updated?
  memory_sha=$(git log -1 --format=%H -- MEMORY.md 2>/dev/null || true)

  if [ -z "$memory_sha" ]; then
    warn "MEMORY.md has never been committed."
  else
    code_commits=$(git rev-list --count "${memory_sha}..HEAD" -- \
      apps/ scripts/ infra/ 2>/dev/null || echo 0)

    if [ "$code_commits" -gt 10 ]; then
      fail "MEMORY.md has not been updated in ${code_commits} code commits. Update it before merging — see CLAUDE.md."
    elif [ "$code_commits" -gt 4 ]; then
      warn "MEMORY.md is ${code_commits} code commits behind. Update it soon."
    else
      pass "MEMORY.md is current (${code_commits} code commits behind)"
    fi
  fi
fi

# --- The state file must parse and be internally consistent --------------------------
if [ -f .aegis/state.json ] && [ -n "$PYTHON" ]; then
  if "$PYTHON" - <<'PY'
import json
import sys

try:
    with open(".aegis/state.json", encoding="utf-8") as handle:
        state = json.load(handle)
except Exception as error:  # noqa: BLE001 - any parse failure is fatal here
    print(f"::error::.aegis/state.json does not parse: {error}")
    sys.exit(1)

required = {"current_phase", "phases", "last_updated"}
missing = required - set(state)
if missing:
    print(f"::error::.aegis/state.json is missing keys: {sorted(missing)}")
    sys.exit(1)

valid_statuses = {"complete", "in_progress", "not_started", "blocked"}
for phase in state["phases"]:
    if phase.get("status") not in valid_statuses:
        print(f"::error::Phase {phase.get('id')} has invalid status {phase.get('status')!r}")
        sys.exit(1)

# A phase cannot be complete while an earlier one is not. Phases are strictly ordered
# (MASTER_BUILD.md Part 15 rule 2), and a state file claiming otherwise means either the
# rule was broken or the file was edited carelessly. Both are worth catching.
seen_incomplete = None
for phase in sorted(state["phases"], key=lambda p: p["id"]):
    if phase["status"] != "complete" and seen_incomplete is None:
        seen_incomplete = phase["id"]
    elif phase["status"] == "complete" and seen_incomplete is not None:
        print(
            f"::warning::Phase {phase['id']} is complete but Phase {seen_incomplete} "
            "is not. Phases are strictly ordered; confirm this is intentional."
        )

print("  ok: .aegis/state.json is valid")
PY
  then
    :
  else
    failures=$((failures + 1))
  fi
elif [ -f .aegis/state.json ]; then
  warn "no working Python interpreter found; skipped .aegis/state.json validation"
fi

# --- MEMORY.md must still contain its load-bearing sections ---------------------------
if [ -f MEMORY.md ]; then
  for heading in "Phase Status" "What Actually Works" "Next Steps" "Known Limitations"; do
    if ! grep -qi "$heading" MEMORY.md; then
      warn "MEMORY.md no longer has a '${heading}' section. It is part of the template for a reason."
    fi
  done
  pass "MEMORY.md structure checked"
fi

printf '\n'
if [ "$failures" -gt 0 ]; then
  printf 'FAILED: %d error(s), %d warning(s)\n' "$failures" "$warnings"
  exit 1
fi

printf 'PASSED: handoff documents are current (%d warning(s)).\n' "$warnings"
exit 0
