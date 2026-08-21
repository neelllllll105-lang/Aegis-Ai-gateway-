#!/usr/bin/env bash
# Machine-generated project status.
#
# Run this at the start of every session, before reading anything else. MEMORY.md tells
# you what the last person *believed*; this tells you what the repository actually
# contains right now. When they disagree, this is right and MEMORY.md is stale.
#
#   bash scripts/status.sh

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

bold=$(printf '\033[1m'); dim=$(printf '\033[2m'); reset=$(printf '\033[0m')
green=$(printf '\033[32m'); yellow=$(printf '\033[33m'); red=$(printf '\033[31m')


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

section() { printf '\n%s%s%s\n' "$bold" "$1" "$reset"; }
ok()      { printf '  %s✓%s %s\n' "$green" "$reset" "$1"; }
warn()    { printf '  %s!%s %s\n' "$yellow" "$reset" "$1"; }
bad()     { printf '  %s✗%s %s\n' "$red" "$reset" "$1"; }
info()    { printf '    %s%s%s\n' "$dim" "$1" "$reset"; }

printf '%s=== AEGIS PROJECT STATUS ===%s\n' "$bold" "$reset"
printf '%sGenerated %s%s\n' "$dim" "$(date -u '+%Y-%m-%d %H:%M UTC')" "$reset"

# --- Phase state --------------------------------------------------------------------
section "Phase state (.aegis/state.json)"
if [ -f .aegis/state.json ]; then
  if [ -n "$PYTHON" ]; then
    "$PYTHON" - <<'PY'
import json
with open(".aegis/state.json", encoding="utf-8") as handle:
    state = json.load(handle)
marks = {"complete": "\033[32m●\033[0m", "in_progress": "\033[33m◐\033[0m",
         "not_started": "\033[2m○\033[0m", "blocked": "\033[31m✗\033[0m"}
for phase in state.get("phases", []):
    mark = marks.get(phase.get("status", ""), "?")
    print(f"  {mark} Phase {phase['id']}: {phase['name']} — {phase.get('status')}")
print(f"\n  Current phase: {state.get('current_phase')}")
print(f"  Next task:     {state.get('next_task', 'see MEMORY.md')}")
PY
  else
    info "no working Python interpreter; raw file follows"
    cat .aegis/state.json
  fi
else
  bad ".aegis/state.json is missing — the phase tracker has been deleted"
fi

# --- Build and tests ----------------------------------------------------------------
section "Gateway build"
if command -v cargo >/dev/null 2>&1; then
  if cargo check --quiet --all-targets 2>/dev/null; then
    ok "compiles"
  else
    bad "does NOT compile — run: cargo check --all-targets"
  fi

  test_output=$(cargo test --lib 2>&1 | grep -E "^test result" | tail -1)
  if printf '%s' "$test_output" | grep -q "0 failed"; then
    passed=$(printf '%s' "$test_output" | grep -oE '[0-9]+ passed' | head -1)
    ok "unit tests: $passed"
  elif [ -n "$test_output" ]; then
    bad "unit tests failing: $test_output"
  else
    warn "could not determine test status"
  fi

  if cargo fmt --check >/dev/null 2>&1; then
    ok "formatting clean"
  else
    warn "formatting drift — run: cargo fmt"
  fi
else
  warn "cargo not found — install Rust to build the gateway"
fi

# --- Source size ---------------------------------------------------------------------
section "Source"
rust_files=$(find apps/gateway/src -name '*.rs' 2>/dev/null | wc -l | tr -d ' ')
rust_lines=$(find apps/gateway/src -name '*.rs' -exec cat {} + 2>/dev/null | wc -l | tr -d ' ')
test_count=$(grep -rn "#\[test\]\|#\[tokio::test\]" apps/gateway/src apps/gateway/tests 2>/dev/null | wc -l | tr -d ' ')
migrations=$(find apps/gateway/migrations -name '*.sql' 2>/dev/null | wc -l | tr -d ' ')
info "gateway:    ${rust_files} files, ${rust_lines} lines"
info "tests:      ${test_count} test functions"
info "migrations: ${migrations}"

if [ -d apps/web/app ]; then
  web_files=$(find apps/web -name '*.tsx' -o -name '*.ts' 2>/dev/null | grep -v node_modules | wc -l | tr -d ' ')
  info "dashboard:  ${web_files} files"
else
  info "dashboard:  not yet created"
fi

# --- Infrastructure -------------------------------------------------------------------
section "Local infrastructure"
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  running=$(docker compose -f infra/docker-compose.yml ps --services --filter status=running 2>/dev/null | tr '\n' ' ')
  if [ -n "${running// }" ]; then
    ok "running: ${running}"
  else
    warn "docker is available but no services are up"
    info "start them: docker compose -f infra/docker-compose.yml up -d"
  fi
else
  warn "docker unavailable — the gateway still builds and unit-tests without it"
  info "DB-backed integration tests are skipped unless AEGIS_TEST_DATABASE_URL is set"
fi

# --- Documentation freshness -----------------------------------------------------------
section "Handoff documents"
for doc in MEMORY.md MASTER_BUILD.md CLAUDE.md docs/PHASES.md docs/HANDOFF.md; do
  if [ -f "$doc" ]; then
    ok "$doc"
  else
    bad "$doc is MISSING"
  fi
done

if [ -f MEMORY.md ] && command -v git >/dev/null 2>&1; then
  memory_commit=$(git log -1 --format=%ct -- MEMORY.md 2>/dev/null || echo 0)
  last_commit=$(git log -1 --format=%ct 2>/dev/null || echo 0)
  if [ "$memory_commit" -gt 0 ] && [ "$last_commit" -gt 0 ]; then
    age_days=$(( (last_commit - memory_commit) / 86400 ))
    if [ "$age_days" -gt 3 ]; then
      warn "MEMORY.md is ${age_days} days behind the newest commit — treat it with suspicion"
    else
      ok "MEMORY.md is current with recent commits"
    fi
  fi
fi

# --- Recent history ---------------------------------------------------------------------
section "Recent commits"
git log --oneline -8 2>/dev/null | sed 's/^/  /' || info "not a git repository"

printf '\n%sNext:%s read MEMORY.md, then docs/PHASES.md for the current phase.\n\n' "$bold" "$reset"
