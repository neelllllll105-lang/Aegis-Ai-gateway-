#!/usr/bin/env bash
# Refresh the generated portions of MEMORY.md, and report what still needs a human.
#
#   bash scripts/update-memory.sh
#
# `CLAUDE.md` requires this at the end of every session. It updates what a machine can
# know — test counts, file counts, recent commits — and then tells you which hand-written
# sections it cannot update for you.
#
# The division is deliberate. A script that rewrote "Known Issues" from a template would
# produce a document that looks maintained and says nothing, which is worse than an
# obviously stale one.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

bold=$(printf '\033[1m'); reset=$(printf '\033[0m')
green=$(printf '\033[32m'); yellow=$(printf '\033[33m')

printf '%s=== MEMORY REFRESH ===%s\n\n' "$bold" "$reset"

# --- Collect the machine-knowable facts -------------------------------------------
TODAY=$(date -u '+%Y-%m-%d')

TEST_OUTPUT=$(cargo test --lib 2>&1 | grep -E "^test result" | tail -1)
TESTS_PASSING=$(printf '%s' "$TEST_OUTPUT" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' || echo "unknown")
TESTS_FAILING=$(printf '%s' "$TEST_OUTPUT" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+' || echo "unknown")

RUST_FILES=$(find apps/gateway/src -name '*.rs' 2>/dev/null | wc -l | tr -d ' ')
RUST_LINES=$(find apps/gateway/src -name '*.rs' -exec cat {} + 2>/dev/null | wc -l | tr -d ' ')
WEB_FILES=$(find apps/web/app apps/web/components apps/web/lib -name '*.tsx' -o -name '*.ts' 2>/dev/null | wc -l | tr -d ' ')
MIGRATIONS=$(find apps/gateway/migrations -name '*.sql' 2>/dev/null | wc -l | tr -d ' ')
ADRS=$(find docs/adr -name '[0-9]*.md' 2>/dev/null | wc -l | tr -d ' ')

printf '%sCurrent state%s\n' "$bold" "$reset"
printf '  Date:            %s\n' "$TODAY"
printf '  Unit tests:      %s passing, %s failing\n' "$TESTS_PASSING" "$TESTS_FAILING"
printf '  Gateway source:  %s files, %s lines\n' "$RUST_FILES" "$RUST_LINES"
printf '  Dashboard:       %s files\n' "$WEB_FILES"
printf '  Migrations:      %s\n' "$MIGRATIONS"
printf '  ADRs:            %s\n' "$ADRS"

if [ "$TESTS_FAILING" != "0" ] && [ "$TESTS_FAILING" != "unknown" ]; then
  printf '\n%sWARNING:%s %s tests are failing. Record that in MEMORY.md under Blockers\n' \
    "$yellow" "$reset" "$TESTS_FAILING"
  printf '         rather than leaving the next person to discover it.\n'
fi

# --- Update the header ---------------------------------------------------------------
if [ -f MEMORY.md ]; then
  if command -v sed >/dev/null 2>&1; then
    sed -i.bak -E "s/^> \*\*Last updated:\*\* .*/> **Last updated:** ${TODAY}/" MEMORY.md
    rm -f MEMORY.md.bak
    printf '\n%s✓%s MEMORY.md date stamp updated\n' "$green" "$reset"
  fi
fi

# --- Recent work, for the session log --------------------------------------------------
printf '\n%sCommits since MEMORY.md was last updated%s\n' "$bold" "$reset"
MEMORY_SHA=$(git log -1 --format=%H -- MEMORY.md 2>/dev/null || true)
if [ -n "$MEMORY_SHA" ]; then
  COMMITS=$(git log --oneline "${MEMORY_SHA}..HEAD" 2>/dev/null | head -20)
  if [ -n "$COMMITS" ]; then
    printf '%s\n' "$COMMITS" | sed 's/^/  /'
    printf '\n  Summarise these in the Session Log section.\n'
  else
    printf '  (none — MEMORY.md is current)\n'
  fi
fi

# --- What only a human can do ------------------------------------------------------------
cat <<'CHECKLIST'

────────────────────────────────────────────────────────────────────────
These sections need YOU. A script cannot know them, and guessing would
produce a document that looks maintained and says nothing.

  [ ] Current Focus      — what you were actually working on
  [ ] What Actually Works — only VERIFIED capabilities. A test you did not
                            run is not a test that passed.
  [ ] Blockers            — anything stopping progress, with the workaround
  [ ] Known Limitations   — the uncomfortable truths. This section exists so
                            there is somewhere to put them rather than
                            omitting them.
  [ ] Next Steps          — concrete and ordered, so the next person can
                            start without deciding what to do
  [ ] Gotchas             — anything that cost you time today. Write it down
                            and it costs the next person nothing.
  [ ] Session Log         — a dated entry for this session

Then update .aegis/state.json to match, and verify:

  bash scripts/check-memory-freshness.sh
────────────────────────────────────────────────────────────────────────
CHECKLIST
