# Runbook: deploy

Target topology is `MASTER_BUILD.md` Part 10 and `docs/adr/0003-deployment-topology.md`:
one Hetzner CPX31 running Coolify, Cloudflare in front, roughly $40/month.

---

## First-time server setup

### 1. Provision

Hetzner CPX31 (4 vCPU / 8GB / 80GB NVMe), Ubuntu 24.04, in an EU region unless a customer
contract requires otherwise.

```bash
# SSH key authentication only. Password auth on a public IP is found by scanners within
# minutes of the machine coming up.
sed -i 's/^#*PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
sed -i 's/^#*PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config
systemctl reload ssh

ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw enable

apt update && apt upgrade -y
apt install -y unattended-upgrades
dpkg-reconfigure -plow unattended-upgrades
```

Note what is **not** exposed: Postgres, Redis, and Qdrant have no public port. They are
reachable only from the Docker network. A development compose file binds them to
`127.0.0.1`; on the server they should not be published at all.

### 2. Install Coolify

```bash
curl -fsSL https://cdn.coollabs.io/coolify/install.sh | bash
```

Set a strong admin password and enable 2FA immediately — the Coolify dashboard can deploy
arbitrary containers, so it is effectively root on this machine.

### 3. Cloudflare

- A record for `api.aegis.dev` → server IP, **proxied** (orange cloud).
- A record for `app.aegis.dev` → server IP, proxied.
- SSL/TLS mode: **Full (strict)**. "Flexible" leaves the origin leg unencrypted, which
  defeats the point.
- Enable "Always Use HTTPS" and the managed WAF ruleset.

### 4. Secrets

In Coolify's environment configuration, not in a file on disk:

```bash
# Generate the master key. This encrypts every customer BYOK credential; rotating it
# makes all stored credentials undecryptable, so store it in the password manager as
# well as here.
openssl rand -base64 32
```

Required in production — the gateway refuses to start without them:

```
AEGIS_ENV=prod
AEGIS_BASE_URL=https://api.aegis.dev
AEGIS_APP_URL=https://app.aegis.dev
DATABASE_URL=postgres://aegis:<password>@postgres:5432/aegis
REDIS_URL=redis://redis:6379
AEGIS_MASTER_KEY=<the generated key>
```

### 5. Services

Add through Coolify, in this order:

1. **PostgreSQL 16** with a persistent volume. Set `max_connections=200` and enable
   `pg_stat_statements` (Part 4 requires both).
2. **Redis 7** with AOF persistence. The usage stream is the only copy of a metered
   request between emission and persistence — losing it on restart loses revenue.
3. **Qdrant** with a volume.
4. **aegis-gateway** from the GitHub repository, Dockerfile at `apps/gateway/Dockerfile`.
5. **aegis-web** from `apps/web/Dockerfile`.

### 6. Migrate and seed

Migrations run automatically at gateway startup. Then:

```bash
docker exec -i aegis-postgres psql -U aegis aegis < scripts/seed.sql
```

**Then immediately follow `docs/runbooks/pricing-update.md`.** The seed prices are marked
`UNVERIFIED` and must not be used for billing.

### 7. Verify

```bash
curl -s https://api.aegis.dev/health | jq
curl -s https://api.aegis.dev/ready -o /dev/null -w "%{http_code}\n"

# Security headers should be present on every response.
curl -sD - https://api.aegis.dev/health -o /dev/null | grep -iE "strict-transport|content-security|x-frame"
```

---

## Routine deploys

Push to `main`. GitHub Actions runs the full suite, builds a multi-arch image, pushes it to
GHCR, and triggers a Coolify webhook for a rolling restart.

```
push → CI (fmt, clippy, tests, audit, licence gate, memory freshness)
     → build image → GHCR → Coolify webhook → rolling restart
```

Deploys are zero-downtime because the gateway is stateless and Coolify starts the new
container before stopping the old one.

### Before merging

```bash
cd apps/gateway && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd apps/web && npm run lint && npm run typecheck && npm run build
bash scripts/check-memory-freshness.sh
```

### After deploying

```bash
curl -s https://api.aegis.dev/health | jq '.status, .version'

# The metering gap must be null. A non-null value means requests were served without a
# usage record, which violates Principle 2 and means somebody is not being billed.
curl -s https://api.aegis.dev/api/admin/metrics -H "Cookie: $ADMIN_SESSION" | jq '.metering_gap'

# P99 overhead is the Principle 1 claim.
curl -s https://api.aegis.dev/api/admin/metrics -H "Cookie: $ADMIN_SESSION" | jq '.gateway_overhead_p99_ms'
```

---

## Rollback

```bash
# Through Coolify: Deployments → select the previous deployment → Redeploy.
# Or directly:
docker service update --rollback aegis-gateway
```

**Migrations are not automatically reversed.** If the bad deploy included a migration,
check whether it is backward-compatible before rolling back the image — an old binary
against a new schema usually works if the migration only added columns, and usually does
not if it renamed or dropped one. This is why migrations should only add.

---

## Health check semantics

Two endpoints, easily confused, with different consequences:

| Endpoint | Question | Behaviour on a sick dependency |
|---|---|---|
| `/health` | Is the process alive? | Still 200. Restarting will not fix a sick database, and a restart loop turns a partial outage into a total one. |
| `/ready` | Should traffic route here? | 503 when the store is unreachable, so the load balancer drains this instance without killing it. |

Point the container healthcheck at `/ready` and the orchestrator liveness probe at
`/health`. Reversing them is how a database blip becomes a crash loop.

---

## Scaling

Do not scale ahead of a trigger. `MASTER_BUILD.md` Part 10 lists them:

| Trigger | Action |
|---|---|
| CPU > 60% sustained, or P99 overhead > 3ms | Second server, gateway only |
| Redis > 512MB or > 5k cmd/s | Dedicated Redis host |
| Postgres > 60% CPU or > 20GB | Dedicated database host + read replica |
| > 5k RPS or multi-region demand | Third region + Cloudflare load balancing |
| > 10k RPS sustained | *Now* consider Kubernetes — not before |

The discipline is the point. Adding orchestration at 100 RPS takes the bill from $40 to
$400 and buys nothing a user can see.
