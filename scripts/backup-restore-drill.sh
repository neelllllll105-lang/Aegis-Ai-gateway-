#!/usr/bin/env bash
# Monthly restore drill.
#
# `MASTER_BUILD.md` Part 13 item 6: an untested backup is not a backup. This script
# restores the most recent backup into a scratch database and reports what came back.
#
# It never touches production. The scratch database is created fresh and dropped at the
# end, so running this at any time is safe.
#
#   bash scripts/backup-restore-drill.sh
#
# Environment:
#   BACKUP_DIR   local directory holding backups (default ./backups)
#   PGHOST etc.  standard libpq variables for the target server
#
# The check that matters is the row counts. A restore that "succeeds" into an empty
# database exits zero and proves nothing — which is exactly the failure this drill exists
# to catch, so the counts are printed and asserted rather than merely the exit status.

set -uo pipefail

BACKUP_DIR="${BACKUP_DIR:-./backups}"
SCRATCH_DB="aegis_restore_drill_$(date +%s)"

bold=$(printf '\033[1m'); reset=$(printf '\033[0m')
green=$(printf '\033[32m'); red=$(printf '\033[31m'); yellow=$(printf '\033[33m')

ok()   { printf '  %s✓%s %s\n' "$green" "$reset" "$1"; }
bad()  { printf '  %s✗%s %s\n' "$red" "$reset" "$1"; }
warn() { printf '  %s!%s %s\n' "$yellow" "$reset" "$1"; }

cleanup() {
  if [ -n "${SCRATCH_CREATED:-}" ]; then
    dropdb --if-exists "$SCRATCH_DB" 2>/dev/null && ok "scratch database dropped"
  fi
  rm -f "${PLAIN_DUMP:-}" 2>/dev/null
}
trap cleanup EXIT

printf '%s=== RESTORE DRILL ===%s\n' "$bold" "$reset"
printf 'Started %s\n\n' "$(date -u '+%Y-%m-%d %H:%M UTC')"

# --- Find a backup -------------------------------------------------------------------
if [ ! -d "$BACKUP_DIR" ]; then
  bad "No backup directory at $BACKUP_DIR"
  printf '\nNothing to restore. If backups live in object storage, fetch the newest one\n'
  printf 'into %s first — see docs/runbooks/restore.md.\n' "$BACKUP_DIR"
  exit 1
fi

BACKUP=$(ls -t "$BACKUP_DIR"/*.sql "$BACKUP_DIR"/*.sql.gz 2>/dev/null | head -1)
if [ -z "${BACKUP:-}" ]; then
  bad "No backup files found in $BACKUP_DIR"
  exit 1
fi

BACKUP_AGE_HOURS=$(( ( $(date +%s) - $(date -r "$BACKUP" +%s 2>/dev/null || echo 0) ) / 3600 ))
ok "Using $(basename "$BACKUP") (${BACKUP_AGE_HOURS}h old)"

# A backup job that silently stopped running is the most common way backups fail, and
# nobody notices until a restore is needed.
if [ "$BACKUP_AGE_HOURS" -gt 48 ]; then
  warn "This backup is over 48 hours old. Check that the nightly job is still running."
fi

# --- Restore into a scratch database ---------------------------------------------------
printf '\n%sRestoring into %s%s\n' "$bold" "$SCRATCH_DB" "$reset"

if ! createdb "$SCRATCH_DB" 2>/dev/null; then
  bad "Could not create the scratch database. Check your PG* connection variables."
  exit 1
fi
SCRATCH_CREATED=1
ok "scratch database created"

case "$BACKUP" in
  *.gz)
    PLAIN_DUMP=$(mktemp)
    gunzip -c "$BACKUP" > "$PLAIN_DUMP"
    ;;
  *)
    PLAIN_DUMP="$BACKUP"
    ;;
esac

if ! psql -q -d "$SCRATCH_DB" -f "$PLAIN_DUMP" > /tmp/restore-drill.log 2>&1; then
  bad "Restore reported errors. Last 20 lines:"
  tail -20 /tmp/restore-drill.log | sed 's/^/      /'
  exit 1
fi
ok "restore completed without errors"

# --- Verify the data actually arrived ----------------------------------------------------
printf '\n%sRow counts%s\n' "$bold" "$reset"

FAILED=0
for table in organizations users api_keys usage_records model_pricing invoices; do
  count=$(psql -tA -d "$SCRATCH_DB" -c "SELECT COUNT(*) FROM ${table};" 2>/dev/null || echo "ERROR")

  if [ "$count" = "ERROR" ]; then
    bad "${table}: table missing from the restore"
    FAILED=1
  elif [ "$count" = "0" ] && [ "$table" != "invoices" ]; then
    # An empty invoices table is normal early on; an empty organizations table is not.
    bad "${table}: 0 rows — an empty restore is the failure this drill exists to catch"
    FAILED=1
  else
    ok "${table}: ${count} rows"
  fi
done

# --- Confirm the schema is complete, not just the data -----------------------------------
printf '\n%sSchema%s\n' "$bold" "$reset"

partitions=$(psql -tA -d "$SCRATCH_DB" -c \
  "SELECT COUNT(*) FROM pg_class WHERE relname LIKE 'usage_records_%';" 2>/dev/null || echo 0)
if [ "$partitions" -gt 0 ]; then
  ok "usage_records partitions: ${partitions}"
else
  bad "no usage_records partitions — inserts would be rejected after a real restore"
  FAILED=1
fi

# The constraints are a load-bearing part of billing correctness, so a restore that loses
# them is a restore that lost a safety net.
constraints=$(psql -tA -d "$SCRATCH_DB" -c \
  "SELECT COUNT(*) FROM pg_constraint WHERE conname IN
     ('fee_within_savings', 'cache_hits_are_free', 'retention_settings_are_consistent');" \
  2>/dev/null || echo 0)
if [ "$constraints" -eq 3 ]; then
  ok "billing constraints present"
else
  bad "expected 3 billing constraints, found ${constraints}"
  FAILED=1
fi

# --- Report the recovery point ------------------------------------------------------------
printf '\n%sRecovery point%s\n' "$bold" "$reset"
latest=$(psql -tA -d "$SCRATCH_DB" -c "SELECT MAX(created_at) FROM usage_records;" 2>/dev/null)
if [ -n "$latest" ]; then
  ok "most recent usage record: ${latest}"
  printf '      Data after this timestamp would be lost in a real restore.\n'
else
  warn "no usage records — cannot determine the recovery point"
fi

# --- Result ---------------------------------------------------------------------------------
printf '\n'
if [ "$FAILED" -eq 1 ]; then
  printf '%sDRILL FAILED.%s The backup is not usable. Investigate before relying on it.\n' "$red" "$reset"
  exit 1
fi

printf '%sDRILL PASSED.%s\n\n' "$green" "$reset"
printf 'Record this in MEMORY.md:\n'
printf '  Last restore drill: %s — passed\n\n' "$(date -u '+%Y-%m-%d')"
