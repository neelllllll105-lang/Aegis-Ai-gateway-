# Runbook: restore from backup

**Criticality:** highest. `MASTER_BUILD.md` Part 13 item 6: *an untested backup is not a
backup.*

This runbook covers both the drill (which must be run monthly) and the real thing.

---

## The rule

**Run the drill on the first Monday of every month.** Record the date and the row counts
in `MEMORY.md`. A backup that has never been restored is a hypothesis, not a recovery
plan, and the moment you discover it does not work is the moment you need it.

---

## What is backed up

| Data | Method | Retention | Recovery point |
|---|---|---|---|
| PostgreSQL | Nightly `pg_dump`, encrypted, to Backblaze B2 | 30 days | Up to 24h loss |
| Redis | AOF with `everysec` fsync, on the server volume | Volume snapshot | Up to 1s loss |
| Qdrant | Volume snapshot | 7 days | Rebuildable |

Redis and Qdrant are recoverable but **not critical**: Redis holds counters and the usage
stream, both of which the reconciliation job rebuilds from PostgreSQL, and the semantic
cache simply refills. PostgreSQL is the only irreplaceable store — it is the billing
source of truth.

---

## The monthly drill

Restores into a scratch database. It never touches production.

```bash
bash scripts/backup-restore-drill.sh
```

The script fetches the most recent backup, restores it into a throwaway database, and
reports row counts for the tables that matter. Read the output — a restore that "succeeds"
into an empty database is the failure this drill exists to catch, which is why row counts
are printed rather than just an exit code.

Record the result in `MEMORY.md` under a "Last restore drill" line.

---

## A real restore

### 1. Stop writes before anything else

```bash
docker compose -f infra/docker-compose.yml stop gateway web
```

Do this first. A gateway still writing during a restore produces a database that is
neither the backup nor the current state, and reconciling that afterwards is far worse
than the extra minute of downtime.

### 2. Identify the backup

```bash
b2 ls aegis-backups --long | tail -20
```

Pick the most recent backup **from before the incident**. If you are recovering from data
corruption rather than data loss, the most recent backup may already contain the
corruption — check the timestamps against when the problem started.

### 3. Restore

```bash
export BACKUP_FILE="aegis-2026-08-20.sql.gz.age"

# Decrypt. The key lives in the password manager, not on the server.
age --decrypt -i ~/.aegis-backup-key "$BACKUP_FILE" | gunzip > restore.sql

# Restore into a NEW database first, never over the live one.
createdb aegis_restored
psql aegis_restored < restore.sql
```

Restoring into a new database rather than over the live one means that if the backup turns
out to be bad, you still have the damaged-but-present original to work from.

### 4. Verify before switching

```bash
psql aegis_restored -c "
  SELECT 'organizations' AS t, COUNT(*) FROM organizations
  UNION ALL SELECT 'users', COUNT(*) FROM users
  UNION ALL SELECT 'api_keys', COUNT(*) FROM api_keys
  UNION ALL SELECT 'usage_records', COUNT(*) FROM usage_records
  UNION ALL SELECT 'invoices', COUNT(*) FROM invoices;"

# The most recent usage record tells you exactly how much data the restore loses.
psql aegis_restored -c "SELECT MAX(created_at) FROM usage_records;"
```

Compare against what you expect. If `usage_records` is dramatically short, stop and
investigate rather than proceeding — you are about to make that the permanent record.

### 5. Switch over

```bash
psql -c "ALTER DATABASE aegis RENAME TO aegis_damaged;"
psql -c "ALTER DATABASE aegis_restored RENAME TO aegis;"

# Partitions must exist before the gateway accepts traffic, or usage inserts are rejected.
psql aegis -c "SELECT maintain_usage_partitions();"

docker compose -f infra/docker-compose.yml start gateway web
curl -s https://api.aegis.dev/health
```

Keep `aegis_damaged` until you are certain. Disk is cheaper than a second incident.

### 6. Reconcile billing

This is the step people skip, and it is the one that matters commercially.

Usage records written between the backup and the incident are gone. The Redis counters may
still hold them — Redis and PostgreSQL fail independently, so a database restore does not
necessarily lose the counter state.

```bash
curl -s https://api.aegis.dev/api/admin/metrics | grep -i drift
```

Drift between the counters and the restored records is the size of the gap. For any
affected organisation, decide explicitly whether to bill the gap or absorb it — and record
the decision. Silently under-billing is defensible; silently over-billing is not.

### 7. Write the postmortem

Include: what was lost, the exact recovery point, which customers were affected, and what
would have shortened the window. File it in `docs/runbooks/incidents/`.

---

## If the backup itself is bad

1. Try the previous night's backup. Retention is 30 days, so there are alternatives.
2. Redis AOF may contain recent usage events not yet persisted — check
   `aegis:usage_events` before flushing anything.
3. Provider-side billing records are an independent source of truth for what was actually
   spent. They will not reconstruct our savings attribution, but they bound the loss.

---

## Prevention

- The drill is monthly for a reason. Do not let it slip.
- Alert on backup age: a job that stops running silently is the most common way backups
  fail, and nobody notices until a restore is needed.
- The decryption key must not live only on the server being backed up.
