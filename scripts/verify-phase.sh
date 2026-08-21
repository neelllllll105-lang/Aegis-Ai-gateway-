#!/usr/bin/env bash
# Verify a phase against its acceptance criteria.
#
#   bash scripts/verify-phase.sh 3
#
# `MASTER_BUILD.md` Part 14 defines what "production ready" means for a phase. This runs
# the checks that can be automated and then lists the ones that cannot, because several
# criteria — a restore drill, a security review, a load test against real traffic — are
# genuinely human judgements and pretending otherwise would turn this into theatre.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

PHASE="${1:-}"
if [ -z "$PHASE" ]; then
  printf 'Usage: bash scripts/verify-phase.sh <0-7>\n'
  exit 2
fi

bold=$(printf '\033[1m'); reset=$(printf '\033[0m')
green=$(printf '\033[32m'); red=$(printf '\033[31m'); yellow=$(printf '\033[33m')

PASSED=0
FAILED=0

ok()   { printf '  %s✓%s %s\n' "$green" "$reset" "$1"; PASSED=$((PASSED + 1)); }
bad()  { printf '  %s✗%s %s\n' "$red" "$reset" "$1"; FAILED=$((FAILED + 1)); }
todo() { printf '  %s?%s %s\n' "$yellow" "$reset" "$1"; }

printf '%s=== VERIFYING PHASE %s ===%s\n\n' "$bold" "$PHASE" "$reset"

# --- Universal gates ------------------------------------------------------------------
# Every phase must satisfy these before its own criteria matter.
printf '%sUniversal gates (Part 14)%s\n' "$bold" "$reset"

if cargo fmt --check >/dev/null 2>&1; then
  ok "formatting clean"
else
  bad "formatting drift — run: cargo fmt"
fi

if cargo clippy --all-targets -- -D warnings >/dev/null 2>&1; then
  ok "clippy clean"
else
  bad "clippy warnings — run: cargo clippy --all-targets -- -D warnings"
fi

TEST_RESULT=$(cargo test --lib 2>&1 | grep -E "^test result" | tail -1)
if printf '%s' "$TEST_RESULT" | grep -q "0 failed"; then
  ok "unit tests: $(printf '%s' "$TEST_RESULT" | grep -oE '[0-9]+ passed')"
else
  bad "unit tests failing: ${TEST_RESULT:-could not determine}"
fi

if [ -d apps/web/node_modules ]; then
  if (cd apps/web && npm run typecheck >/dev/null 2>&1); then
    ok "dashboard typechecks"
  else
    bad "dashboard type errors — run: cd apps/web && npm run typecheck"
  fi
else
  todo "dashboard dependencies not installed (cd apps/web && npm install)"
fi

if bash scripts/check-memory-freshness.sh >/dev/null 2>&1; then
  ok "handoff documents current"
else
  bad "handoff documents stale — run: bash scripts/check-memory-freshness.sh"
fi

# --- Phase-specific criteria ------------------------------------------------------------
printf '\n%sPhase %s criteria%s\n' "$bold" "$PHASE" "$reset"

# Assert that a named test exists and passes.
require_test() {
  local name="$1" description="$2"
  if cargo test --lib "$name" 2>&1 | grep -qE "^test result: ok"; then
    ok "$description"
  else
    bad "$description (test: $name)"
  fi
}

require_file() {
  local path="$1" description="$2"
  if [ -e "$path" ]; then
    ok "$description"
  else
    bad "$description (missing: $path)"
  fi
}

case "$PHASE" in
  0)
    require_file infra/docker-compose.yml "local dev stack defined"
    require_file apps/gateway/migrations "schema migrations present"
    require_file .github/workflows/ci.yml "CI configured"
    require_test "health" "health endpoints tested"
    require_test "config::tests" "typed configuration tested"
    require_file docs/adr/0001-rust-axum-gateway.md "ADR-001 written"
    ;;
  1)
    require_test "crypto::tests" "key generation and hashing tested"
    require_test "middleware::auth" "authentication tested"
    require_file apps/gateway/tests/tenant_isolation.rs "cross-tenant tests present"
    require_file "apps/web/app/(auth)/login/page.tsx" "login page present"
    require_file "apps/web/app/(dashboard)/keys/page.tsx" "keys page present"
    ;;
  2)
    require_test "providers::" "provider translation tested"
    require_test "rate_limit" "rate limiting tested"
    require_test "budget" "budget enforcement tested"
    require_test "usage" "usage emission tested"
    require_file infra/grafana/aegis-gateway.json "Grafana dashboard committed"
    ;;
  3)
    require_test "classifier_accuracy_meets_bar" "classifier accuracy >= 85%"
    require_test "no_complex_request_is_ever_classified_simple" "complex requests never downgraded"
    require_test "router::tests" "routing decisions tested"
    require_test "cache::" "caching tested"
    require_test "savings::tests" "savings attribution tested"
    require_test "the_passthrough_hint_always_wins" "passthrough escape hatch works"
    ;;
  4)
    require_test "billing::" "invoice and Stripe logic tested"
    require_file infra/loadtest/k6-gateway.js "load test committed"
    require_file docs/runbooks/restore.md "restore runbook written"
    require_file docs/runbooks/deploy.md "deploy runbook written"
    require_file scripts/backup-restore-drill.sh "restore drill scripted"
    todo "load test EXECUTED against staging (human: run k6 and record the P99)"
    todo "restore drill EXECUTED this month (human: run the drill, record in MEMORY.md)"
    ;;
  5)
    require_test "providers::openrouter\|providers::groq\|providers::deepseek" "expansion providers tested"
    require_test "classifier_v2" "classifier v2 evaluated"
    require_file scripts/train_classifier.py "classifier training pipeline present"
    todo "launch assets prepared (human: see docs/runbooks/launch.md)"
    ;;
  6)
    require_test "license::tests\|enterprise::license" "license validation tested"
    require_test "enterprise::scim" "SCIM shapes tested"
    require_test "enterprise::sso" "SSO assertion validation tested"
    require_test "tenant_keys_are_deterministic_and_isolated" "per-tenant keys tested"
    todo "SSO verified against a real Okta or Entra tenant (human)"
    ;;
  7)
    require_test "bandit_outperforms_static_routing_on_replayed_data" "bandit beats static routing"
    require_file sdks/typescript/src/index.ts "TypeScript SDK present"
    require_file sdks/python/aegis/__init__.py "Python SDK present"
    ;;
  *)
    printf '  Unknown phase %s. Valid phases are 0 through 7.\n' "$PHASE"
    exit 2
    ;;
esac

# --- Result ---------------------------------------------------------------------------
printf '\n'
if [ "$FAILED" -gt 0 ]; then
  printf '%sPHASE %s NOT VERIFIED%s — %d passed, %d failed.\n' \
    "$red" "$PHASE" "$reset" "$PASSED" "$FAILED"
  printf 'Fix the failures above, or record the blocker in MEMORY.md and report it.\n\n'
  exit 1
fi

printf '%sPHASE %s: automated checks passed%s (%d checks).\n\n' \
  "$green" "$PHASE" "$reset" "$PASSED"
printf 'Any %s?%s items above still need a human. A phase is done when those are done\n' "$yellow" "$reset"
printf 'too, docs/PHASES.md is ticked with evidence, and .aegis/state.json says complete.\n\n'
