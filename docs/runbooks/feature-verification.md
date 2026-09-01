# Runbook: verifying every feature Aegis actually offers

**Purpose:** a complete, testable inventory. Every route below is copied from
[`src/router.rs`](../../apps/gateway/src/router.rs) — the actual list of what's wired up and
reachable, not a marketing description. If a claim doesn't have a route here, it isn't
built yet, however it reads on the website.

**How to read the status tags:**

- `[LIVE ✓]` — verified working, this session, on this machine, commands below are exactly
  what was run.
- `[NEEDS DB]` — code exists and is unit-tested, but requires `DATABASE_URL` (Postgres) to
  exercise for real. This machine doesn't have Docker installed, so these are un-run.
  Commands are provided and ready to use once you have Postgres.
- `[NEEDS PROVIDER KEY]` — requires a real upstream API key (OpenAI/Anthropic/Google/etc.)
  to actually route a completion. Everything up to that call can be verified without one.

---

## 0. Prerequisites

```bash
# Terminal 1 — the gateway
cd apps/gateway && cargo run --bin aegis-gateway

# Terminal 2 — the dashboard
cd apps/web && npm run dev
```

Without `DATABASE_URL`/`REDIS_URL` set, the gateway still starts — `db`/`store` fall back to
`None`/in-memory, and it logs exactly what's degraded on boot:

```
WARN REDIS_URL not set — using the in-process store...
WARN DATABASE_URL not set — management endpoints will return an error and usage will not be persisted.
WARN QDRANT_URL/QDRANT_GRPC_URL not set — semantic cache runs in-process only...
WARN workers not started: no database configured
```

**To unlock everything below marked `[NEEDS DB]`:**

```bash
docker compose -f infra/docker-compose.yml up -d   # postgres, redis, qdrant
export DATABASE_URL="postgres://aegis:aegis@localhost:5432/aegis"
export REDIS_URL="redis://localhost:6379"
export QDRANT_URL="http://localhost:6333"
cd apps/gateway && cargo run --bin aegis-gateway
```

Migrations run automatically on startup (`db::pool::migrate`) — no separate migrate step.

**To unlock `[NEEDS PROVIDER KEY]`:** set at least one of `OPENAI_API_KEY`,
`ANTHROPIC_API_KEY`, `GOOGLE_API_KEY` etc. as a *shared pool* key (`AEGIS_SHARED_OPENAI_KEYS=sk-...`,
comma-separated for more than one), or add your own via `POST /api/providers` after signup
(section 5 below).

---

## 1. Health, status, and observability — `[LIVE ✓]`, no auth, no DB required

```bash
curl -s http://localhost:8080/health | python3 -m json.tool
```
Expect: `{"status":"ok","version":"...","dependencies":[{"name":"store","status":"ok",...},{"name":"database","status":"not_configured",...}],"degraded_providers":[]}`.
Every dependency is reported individually — this is what tells you *which* piece is missing,
not just "something's wrong."

```bash
curl -s http://localhost:8080/ready
```
Expect `200 OK` if the process can serve traffic, `503` if a required dependency (in
production config) is down. Different from `/health`: this is the liveness/readiness probe
Kubernetes would poll, `/health` is the human-readable diagnostic.

```bash
curl -s http://localhost:8080/status | python3 -m json.tool
```
Expect: `{"status":"operational","uptime_seconds":N,"gateway_overhead_p50_ms":...,"gateway_overhead_p99_ms":...,"providers":[]}`.
This is the public status-page feed — no auth, safe to expose externally. `providers` lists
only *degraded* ones (circuit open/half-open); empty means everything's healthy.

```bash
curl -s http://localhost:8080/metrics | head -40
```
Expect Prometheus text-format output — request counts, latency histograms, cache hit rates,
circuit-breaker state, dependency-up gauges. Point a real Prometheus at this in production;
`curl` is enough to confirm the format is right.

**Correlation ids** — send your own, get it echoed:
```bash
curl -sD - -o /dev/null -H "x-aegis-request-id: my-trace-123" http://localhost:8080/health | grep -i x-aegis-request-id
```
Expect the response header to echo `my-trace-123` back. Send nothing and a UUID is minted
for you instead — check the gateway's own log line for the same request; every log line for
that request carries the identical id.

---

## 2. Signup, login, sessions — `[NEEDS DB]`

```bash
curl -s -X POST http://localhost:8080/api/auth/signup \
  -H "Content-Type: application/json" \
  -d '{"email":"you@example.com","password":"a-real-passphrase-12+chars","name":"Your Name"}' \
  -c cookies.txt | python3 -m json.tool
```
Expect `201`, a user + a default organisation created, session cookie written to
`cookies.txt`. Password minimum is 12 characters, length-only (no composition rules — NIST
guidance, see the code comment on `MIN_PASSWORD_LENGTH`).

**Without a database, confirm the documented failure mode instead** (this is what Session 9
found and fixed — a real regression test now guards it):
```bash
curl -s -o /dev/null -w "%{http_code}\n" -X POST http://localhost:8080/api/auth/signup \
  -H "Content-Type: application/json" \
  -d '{"email":"you@example.com","password":"whatever-12-chars"}'
```
Expect `503`, not a bare `500` — "This service is temporarily unavailable — the database is
not reachable."

```bash
curl -s -X POST http://localhost:8080/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"you@example.com","password":"a-real-passphrase-12+chars"}' \
  -c cookies.txt -b cookies.txt | python3 -m json.tool

curl -s http://localhost:8080/api/auth/me -b cookies.txt | python3 -m json.tool
```
Expect `/api/auth/me` to return your user + org, using the session cookie — no bearer token
needed once logged in. This is what every dashboard page authenticates with.

**TOTP (2FA)** — enroll, get a `secret` + QR-encodable URI, confirm with a code from any
authenticator app pointed at that secret, then login requires `totp_code` from then on:
```bash
curl -s -X POST http://localhost:8080/api/auth/totp/enroll -b cookies.txt | python3 -m json.tool
curl -s -X POST http://localhost:8080/api/auth/totp/confirm -b cookies.txt \
  -H "Content-Type: application/json" -d '{"code":"123456"}'
```

---

## 3. API keys — `[NEEDS DB]`

```bash
curl -s -X POST http://localhost:8080/api/keys -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"name":"my first key"}' | python3 -m json.tool
```
Expect a `key` field shaped `aegis_sk_...` — **shown exactly once**, at creation, never
retrievable again (only a stored hash + last-4 hint remain). Save it:
```bash
export AEGIS_KEY="aegis_sk_...."
```

```bash
curl -s http://localhost:8080/api/keys -b cookies.txt | python3 -m json.tool
```
Expect a list with `name`, `last_four`, `created_at`, `revoked_at: null` — never the raw key
again.

**Per-key limits**, set at creation or via `PATCH`:
```bash
curl -s -X PATCH http://localhost:8080/api/keys/<id> -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"rate_limit_per_minute": 30, "monthly_budget_mc": 5000000, "allowed_models": ["openai/gpt-4o-mini"]}'
```
`allowed_models` is an allowlist — a request for anything else on this key is rejected
before it reaches a provider. `monthly_budget_mc` is a hard per-key spend cap in
micro-cents (5,000,000 = $5.00).

```bash
curl -s -X DELETE http://localhost:8080/api/keys/<id> -b cookies.txt -o /dev/null -w "%{http_code}\n"
```
Expect `204`. A revoked key fails auth on the gateway surface immediately — no cache to
wait out (see `middleware/auth::KeyCache` below).

---

## 4. The gateway surface itself — chat completions, routing, fallback

This is the actual product: `/v1/chat/completions` (OpenAI-shaped), `/v1/messages`
(Anthropic-shaped), `/v1/embeddings`, `/v1/models`.

**`[NEEDS PROVIDER KEY]`** for a real completion:
```bash
curl -s -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "x-api-key: $AEGIS_KEY" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [{"role": "user", "content": "Say hello in five words."}]
  }' -D - | tee /tmp/response.json
```
Expect a normal OpenAI-shaped completion body, **plus** these response headers — this is
the whole product's value proposition, made inspectable on every single call:

| Header | What it proves |
|---|---|
| `X-Aegis-Requested-Model` | what you asked for |
| `X-Aegis-Served-By` | what actually served it — may differ, that's the router working |
| `X-Aegis-Routing-Reason` | `complexity`, `policy`, `fallback`, `user_override`, or passthrough |
| `X-Aegis-Baseline-Cost` | what the requested model would have cost |
| `X-Aegis-Actual-Cost` | what the served model actually cost |
| `X-Aegis-Savings` | the difference |
| `X-Aegis-Overhead-Ms` | Aegis's own added latency — the pitch is sub-millisecond |

**Force a specific routing outcome to see each reason fire:**
```bash
# A trivially simple prompt — expect served_model to downgrade, reason=complexity
curl -s -X POST http://localhost:8080/v1/chat/completions -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o","messages":[{"role":"user","content":"What is 2+2?"}]}' \
  -D - -o /dev/null | grep -i x-aegis

# Ask twice, identical prompt — second call should be a cache hit
curl -s -X POST http://localhost:8080/v1/chat/completions -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o","messages":[{"role":"user","content":"What is the capital of France?"}]}' \
  -D - -o /dev/null | grep -i x-aegis
# run the exact same command again — expect X-Aegis-Actual-Cost: 0 or near-0, an exact-cache header

# Force passthrough (no downgrade) via a routing hint header
curl -s -X POST http://localhost:8080/v1/chat/completions -H "x-api-key: $AEGIS_KEY" \
  -H "x-aegis-routing-hint: passthrough" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hi"}]}' \
  -D - -o /dev/null | grep -i x-aegis-served-by
# expect served_model == requested_model
```

**Streaming** (SSE):
```bash
curl -N -X POST http://localhost:8080/v1/chat/completions -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o","stream":true,"messages":[{"role":"user","content":"Count to 5."}]}'
```
Expect `data: {...}\n\n` chunks, ending `data: [DONE]`. Usage/cost is metered *after* the
stream closes — even if the client disconnects mid-stream, the already-produced tokens are
still billed (they were produced, so they're billable — check the audit log after).

**Streaming + tool calling together, specifically:**
```bash
curl -N -X POST http://localhost:8080/v1/chat/completions -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o","stream":true,
       "messages":[{"role":"user","content":"What is the weather in Paris?"}],
       "tools":[{"type":"function","function":{"name":"get_weather","parameters":{"type":"object","properties":{"location":{"type":"string"}}}}}]}'
```
Expect `delta.tool_calls` to actually appear in the SSE chunks. This was broken until this
session — every chunk that carried *only* a tool-call delta (no text `content` on that
same chunk, which is the normal shape for a streamed function call) was silently dropped,
because the "does this chunk carry anything" check only ever looked at `delta.content`.
Exactly the failure mode that would hit Cursor, Continue, Cline, and Roo hardest, since
agentic coding tools lean on streaming + tool-calling together. Fixed in
`providers/openai.rs::parse_stream_chunk`, covering every provider that shares it (OpenAI,
OpenRouter, DeepSeek, Mistral, Groq, Moonshot) — regression-tested with the exact wire
shape, not just asserted fixed.

**The same combination on the native Anthropic surface is fixed too, same session.**
`POST /v1/messages` with `stream: true` and `tools` now correctly emits real, independently
indexed `content_block_start`/`content_block_delta`/`content_block_stop` events for each
tool_use block, interleaved correctly with any text blocks — driven by
`routes/anthropic_compat.rs`'s `BlockTracker`, not a hardcoded single text block. Google
Gemini's streaming had the identical class of bug (`functionCall` parts silently dropped)
and was fixed the same way. See the Session 11 log entry in `MEMORY.md` for the full
technical detail, including a genuine pre-existing bug the fix's own tests caught.

```bash
curl -N -X POST http://localhost:8080/v1/messages -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"anthropic/claude-sonnet-4-5","max_tokens":200,"stream":true,
       "messages":[{"role":"user","content":"What is the weather in Paris?"}],
       "tools":[{"name":"get_weather","input_schema":{"type":"object","properties":{"location":{"type":"string"}}}}]}'
```
Expect a `content_block_start` with `"content_block":{"type":"tool_use",...}`, one or more
`content_block_delta` events with `"delta":{"type":"input_json_delta",...}`, and a matching
`content_block_stop` — each on its own block index, distinct from any text block that
preceded it.

**Anthropic-shaped surface:**
```bash
curl -s -X POST http://localhost:8080/v1/messages -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"anthropic/claude-sonnet-4-5","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}'
```

**Model catalogue** — requires an API key, same as any other `/v1/*` route (confirmed live,
session 12; this used to say "no key needed" here, which was wrong):
```bash
curl -s http://localhost:8080/v1/models -H "x-api-key: $AEGIS_KEY" | python3 -m json.tool
```
Filtered to what your plan and key actually allow — tier-ceilinged for free plans,
narrowed further by a key's `allowed_models` if one is set.

**Provider outage / fallback** — the honest way to test this without waiting for a real
outage: force a circuit open via the admin endpoint (section 9), then retry a completion and
watch it fail over to the next candidate in the chain automatically. `X-Aegis-Served-By`
will show a *different provider* than requested, not just a different model.

**Embeddings:**
```bash
curl -s -X POST http://localhost:8080/v1/embeddings -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/text-embedding-3-small","input":"hello world"}'
```

---

## 5. Multi-provider support — `[NEEDS DB]` to store, `[NEEDS PROVIDER KEY]` to test

Built-in adapters: `openai`, `anthropic`, `google` (Gemini), `vertex` (Google Vertex AI,
service-account auth), `deepseek`, `groq`, `mistral`, `moonshot`, `openrouter`, `custom`
(any OpenAI-compatible endpoint you point it at).

**Bring your own key:**
```bash
curl -s -X POST http://localhost:8080/api/providers -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"provider":"openai","api_key":"sk-...","label":"my openai account","is_default":true}'

curl -s http://localhost:8080/api/providers -b cookies.txt | python3 -m json.tool
```
Expect the credential listed with only a `hint` (`...abcd`), never the raw key —
`crypto::encrypt` with the org's derived key, `sqlx` never returns the ciphertext to a
serializer by design (there's a dedicated test for exactly this:
`credentials_never_serialize_their_ciphertext`).

```bash
curl -s -X POST http://localhost:8080/api/providers/<id>/test -b cookies.txt
```
Expect a real, cheap call against the actual provider to confirm the key works — "test
connection" as a first-class action, not just "hope it works on the first real request."

**Custom/self-hosted endpoint** (e.g., a local vLLM or Ollama):
```bash
curl -s -X POST http://localhost:8080/api/providers -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"provider":"custom","api_key":"whatever-your-endpoint-needs","base_url":"http://localhost:11434/v1","label":"local vLLM"}'
```
This is SSRF-guarded (`middleware::ssrf_guard`) — try pointing `base_url` at
`http://169.254.169.254` (a cloud metadata endpoint) or `http://localhost:8080` (Aegis
itself) and confirm it's rejected, not silently accepted.

---

## 6. Caching — exact-match and semantic

**Exact-match (hot tier, Redis)** — `[NEEDS DB]` in production shape, but works right now
with the in-memory fallback store too:
Run the identical `/v1/chat/completions` request twice in a row (section 4's cache example
above). Second call: near-zero `X-Aegis-Actual-Cost`, a cache-outcome routing reason.

**Semantic/durable cache (Qdrant-backed)** — `[NEEDS DB]` + `QDRANT_URL`, Pro+ plan only
(`smart_caching_enabled = auth.plan != "free"`):
Send two *differently worded* but semantically identical prompts on a paid-plan key —
e.g. `"What's the capital of France?"` then `"Tell me France's capital city."` — expect
the second to register as a semantic hit, not a miss, once above the similarity threshold.

---

## 7. Budgets and spend control — `[NEEDS DB]`

```bash
curl -s -X POST http://localhost:8080/api/budgets -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"period":"monthly","limit_mc":50000000,"hard_limit":true}'
```
$50.00/month, hard limit ($1 = 1,000,000 micro-cents). `team_id`/`api_key_id`/`region` are
mutually exclusive scopes — omit all three for an org-wide budget.

**Prove the hard limit actually rejects**: set a tiny limit, then exceed it —
```bash
curl -s -X POST http://localhost:8080/api/budgets -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"period":"daily","limit_mc":100,"hard_limit":true}'   # $0.0001 — one token, basically
# then make any real completion call — expect HTTP 402, not a silent overspend
curl -s -o /dev/null -w "%{http_code}\n" -X POST http://localhost:8080/v1/chat/completions \
  -H "x-api-key: $AEGIS_KEY" -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hi"}]}'
```
Expect `402 Payment Required`. This is evaluated *before* the provider is called — a hard
limit costs nothing to enforce, it doesn't burn the last dollar finding out.

**Anomaly detection** (spend abnormal vs. historical baseline):
```bash
curl -s http://localhost:8080/api/usage/anomalies -b cookies.txt | python3 -m json.tool
```
Expect `observed_mc`, `baseline_mean_mc`, `z_score`, `is_anomalous`. Needs a few days of
real usage history to be meaningful — a fresh org will show "not anomalous, insufficient
baseline."

**Atomic reservation** (the thing that prevents a race where two concurrent requests both
squeak under a budget that neither alone would have exceeded) — this is what
`tests/redis_concurrency.rs`'s `budget_reservation_is_atomic_against_real_redis` proves;
fire two requests concurrently against a budget with exactly enough headroom for one:
```bash
(curl -s -o /dev/null -w "%{http_code}\n" -X POST http://localhost:8080/v1/chat/completions \
  -H "x-api-key: $AEGIS_KEY" -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}' &
 curl -s -o /dev/null -w "%{http_code}\n" -X POST http://localhost:8080/v1/chat/completions \
  -H "x-api-key: $AEGIS_KEY" -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}' &
 wait)
```
Expect exactly one `200` and one `402`, never two `200`s.

---

## 8. Rate limiting — `[NEEDS DB]` for org-wide, works with in-memory store too for per-key

```bash
for i in $(seq 1 15); do
  curl -s -o /dev/null -w "%{http_code} " -X POST http://localhost:8080/v1/chat/completions \
    -H "x-api-key: $AEGIS_KEY" -H "Content-Type: application/json" \
    -d '{"model":"openai/gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}'
done; echo
```
On a free-plan key with a low `rate_limit_per_minute`, expect `200` responses to switch to
`429` once the window is exhausted, within the same run. Per-plan org-wide ceilings:
free=10/min, pro=600/min, team=3,000/min, enterprise=10,000/min
(`middleware::rate_limit::org_limit_for_plan`).

---

## 9. Admin console — staff-only, `require_admin`, `[NEEDS DB]`

Every route below 404s (not 401/403 — deliberately, so the surface doesn't confirm its own
existence) for a non-admin caller, and 401s with no session at all:
```bash
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8080/api/admin/metrics
# expect 401 with no cookie at all; 404 if logged in but not staff
```

With a real admin session (`users.is_admin = true`, set directly in the database — never
grantable through the API, on purpose):
```bash
curl -s http://localhost:8080/api/admin/metrics -b admin_cookies.txt | python3 -m json.tool
curl -s http://localhost:8080/api/admin/routing -b admin_cookies.txt | python3 -m json.tool
curl -s http://localhost:8080/api/admin/pricing -b admin_cookies.txt | python3 -m json.tool
curl -s http://localhost:8080/api/admin/audit -b admin_cookies.txt | python3 -m json.tool
```
- `metrics`: uptime, pool stats, degraded providers, request/usage counters.
- `routing`: what the outcome-learning bandit has learned per complexity band — pulls,
  success rate, mean reward, established/not.
- `pricing`: the live table with provenance, **plus new this session**: `loaded_at`,
  `loaded_from` (`database`/`seed_fallback`), `unverified_count`.
- `audit`: any organisation's audit log, by `?org_id=`.

**This session's new pricing-hot-reload endpoints:**
```bash
curl -s -X POST http://localhost:8080/api/admin/pricing/reload -b admin_cookies.txt
```
Expect `{"reloaded": true, "models": N}` with a database configured, `503` without one.
This is the "I just ran `docs/runbooks/pricing-update.md`, don't make me restart the
gateway" button.

```bash
curl -s -X POST http://localhost:8080/api/admin/pricing/openrouter/refresh -b admin_cookies.txt
```
Expect `{"fetched": N, "stored": N}`, N in the hundreds. Populates
`openrouter_pricing_reference` — **a reference table Aegis never bills from**, see
`docs/runbooks/pricing-update.md` for exactly why. Confirm it landed:
```sql
SELECT model_id, input_per_mtok_mc, output_per_mtok_mc, fetched_at
FROM openrouter_pricing_reference ORDER BY model_id LIMIT 10;
```

**Force a circuit breaker open, then reset it:**
```bash
curl -s -X POST http://localhost:8080/api/admin/providers/openai/reset -b admin_cookies.txt
```

---

## 10. Multi-tenancy — orgs, teams, members, RBAC — `[NEEDS DB]`

```bash
curl -s http://localhost:8080/api/org -b cookies.txt | python3 -m json.tool
curl -s -X POST http://localhost:8080/api/org/teams -b cookies.txt \
  -H "Content-Type: application/json" -d '{"name":"Platform Team"}'
curl -s -X POST http://localhost:8080/api/org/members/invite -b cookies.txt \
  -H "Content-Type: application/json" -d '{"email":"teammate@example.com","role":"writer"}'
```
Three roles exist: `reader` (view-only), `writer` (can change config), `admin` (platform
staff only, not the same as an org's own owner — see `require_admin` vs `require_writer`).

**Prove tenant isolation** (the thing `tests/tenant_isolation.rs`'s six tests exist for):
create two orgs, try to read org B's data using org A's session — every one of these must
fail, not just return empty:
```bash
# as org A, try to fetch org B's API key by id
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8080/api/keys/<org-B-key-id> -b org_a_cookies.txt
# expect 404, never org B's key
```

---

## 11. Custom routing policies — `[NEEDS DB]`

```bash
curl -s -X POST http://localhost:8080/api/policies -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"name":"block-frontier-tier","rules":{"max_tier":"premium"}}'
```
A per-org policy that constrains what the router is allowed to pick, independent of the
plan-level tier ceiling — e.g. cap a specific team below what their plan would otherwise
permit.

---

## 12. Enterprise: SSO, SCIM, zero-retention, data residency — `[NEEDS DB]`

**SSO** (SAML/OIDC via an identity provider — Okta, Azure AD, Google Workspace):
```bash
curl -s http://localhost:8080/api/auth/sso/connections -b cookies.txt
# then, from a browser (this is a redirect flow, not a plain curl):
open "http://localhost:8080/api/auth/sso/start?org=your-org-slug"
```
Expect a redirect to the configured IdP, and a callback at `/api/auth/sso/callback` that
completes a session — this exact round-trip was completely broken before session 8's audit
(registered at the wrong URL entirely; nothing could ever complete it). Confirm both legs
actually connect, not just that the routes respond.

**SCIM 2.0** (automated user provisioning from an IdP):
```bash
curl -s -X POST http://localhost:8080/api/scim-tokens -b cookies.txt | python3 -m json.tool
# use the returned token as a bearer, in the shape a real IdP would send:
curl -s http://localhost:8080/scim/v2/Users \
  -H "Authorization: Bearer <scim-token>" \
  -H "Content-Type: application/scim+json"
```

**Zero-retention mode** — toggle per-org, then confirm no prompt/response content is
persisted anywhere, only operational metadata:
```bash
curl -s -X PATCH http://localhost:8080/api/org -b cookies.txt \
  -H "Content-Type: application/json" -d '{"zero_retention": true}'
```
After this, make a completion call, then check `/api/requests` (section 13) — expect
routing/cost/latency fields present, **no prompt or response text anywhere**, by
construction (the schema has no column for it — this isn't a redaction step that could
fail, there's nowhere to put it).

**Data residency pinning**: `region` on org creation/update (`eu-central`, `us-east`, etc.)
— confirm usage rows for that org land in the matching partition
(`db::pool::maintain_partitions`).

---

## 13. Usage, requests, savings reporting — `[NEEDS DB]`

```bash
curl -s http://localhost:8080/api/usage/summary -b cookies.txt | python3 -m json.tool
curl -s http://localhost:8080/api/requests -b cookies.txt | python3 -m json.tool
curl -s http://localhost:8080/api/savings/report.csv -b cookies.txt -o savings.csv && cat savings.csv
curl -s http://localhost:8080/api/audit-log.jsonl -b cookies.txt
curl -s http://localhost:8080/api/usage/chargeback -b cookies.txt | python3 -m json.tool
curl -s http://localhost:8080/api/usage/chargeback.csv -b cookies.txt
```
`/api/requests`: every request's routing decision, cost, savings, latency, cache outcome —
the exact data the dashboard's Requests page renders. `chargeback`: per-team/per-key spend
breakdown, for internal cost allocation. `audit-log.jsonl`: your own org's audit trail,
newline-delimited JSON, SIEM-ingestible.

---

## 14. Billing and plans — `[NEEDS DB]`, Stripe calls need a real Stripe test key

```bash
curl -s http://localhost:8080/api/billing/plan -b cookies.txt | python3 -m json.tool
```
Expect `plan`, `savings_share_bp` (basis points — 2000 = 20%), `subscription_mc`, and
`limits: {requests_per_minute, monthly_request_allowance, byok}` — this is exactly what
backs the pricing page's numbers; cross-check them against
[`apps/web/app/(marketing)/pricing/page.tsx`](../../apps/web/app/(marketing)/pricing/page.tsx)
directly, don't just trust the copy.

```bash
curl -s http://localhost:8080/api/billing/credits -b cookies.txt
curl -s -X POST http://localhost:8080/api/billing/referral -b cookies.txt \
  -H "Content-Type: application/json" -d '{"code":"FRIEND2026"}'
```
Referral credit: one claim per org, ever — try claiming twice and confirm the second is
rejected (`has_claimed_referral`).

**The savings-share formula, verified by hand, not just trusted:**
```
gross_savings = baseline_cost - actual_cost     # floored at 0, never negative
aegis_share   = gross_savings * your_plan_rate
net_savings   = gross_savings - aegis_share
```
Pull a few rows from `/api/requests`, compute this yourself, and confirm it matches
`/api/usage/summary`'s totals exactly — integer micro-cents throughout, so there's no
rounding-drift excuse for a mismatch.

---

## 15. Security — encryption, SSRF, redaction, headers

**Provider credentials are encrypted at rest, never returned in plaintext** — already
covered in section 5; the dedicated test is
`credentials_never_serialize_their_ciphertext` in `db/repo.rs`.

**SSRF guard** on any BYOK `base_url` — already covered in section 5. Try:
```bash
curl -s -o /dev/null -w "%{http_code}\n" -X POST http://localhost:8080/api/providers -b cookies.txt \
  -H "Content-Type: application/json" \
  -d '{"provider":"custom","api_key":"x","base_url":"http://169.254.169.254/latest/meta-data/"}'
```
Expect a `400`, not a stored credential pointed at a cloud metadata service.

**Secret redaction in logs** — start the gateway with `RUST_LOG=debug`, make a request with
a real-looking key in a header, grep the log output:
```bash
cargo run --bin aegis-gateway 2>&1 | grep -i "sk-\|api_key" 
```
Expect nothing resembling a raw key ever appears — `telemetry.rs`'s redaction layer is
tested against OpenAI/Anthropic/Google/Stripe/Groq key shapes, PEM blocks, connection
strings, and bearer headers, case-insensitively.

**Security headers on every response:**
```bash
curl -sD - -o /dev/null http://localhost:8080/health | grep -iE "strict-transport|x-content-type|x-frame|content-security"
```

**CORS** — only the configured `AEGIS_APP_URL` origin is allowed with credentials. `curl`
doesn't enforce CORS at all (it's a browser-side restriction, not a server-side rejection),
so a bare `curl -H "Origin: ..."` isn't a real test — it'll show you *what header the
server sends*, but not whether a browser would actually block the response. What the
header being a fixed, single value proves:
```bash
curl -sD - -o /dev/null http://localhost:8080/api/auth/me | grep -i access-control-allow-origin
```
Expect `Access-Control-Allow-Origin: http://localhost:3000` (or whatever `AEGIS_APP_URL`
is set to) — a **static allow-list of exactly one origin**, never a wildcard `*` and never
a reflection of whatever `Origin` the request sent. `*` is invalid with
`Access-Control-Allow-Credentials: true` anyway (the two are mutually exclusive per spec),
which is *why* it has to be a fixed value — but confirm it's actually pinned to your real
dashboard origin, not something looser. To verify the browser-enforced half for real: open
a page on a *different* origin (e.g. `http://localhost:5555` via `python3 -m http.server
5555`), `fetch("http://localhost:8080/api/auth/me", {credentials:"include"})` from its
console, and confirm the browser reports a CORS error — that's the actual protection,
`curl` can only show you the header value that produces it.

---

## 16. Reliability — timeouts, circuit breakers, graceful shutdown

**Request deadline** — every request is bounded, even a fallback chain that would otherwise
run for minutes:
```bash
time curl -s -o /dev/null http://localhost:8080/v1/chat/completions \
  -H "x-api-key: $AEGIS_KEY" -H "Content-Type: application/json" \
  -d '{"model":"a-model-that-does-not-exist","messages":[{"role":"user","content":"hi"}]}'
```
Expect a `504` (not a hang) if every candidate in a fallback chain times out — `real`
elapsed time bounded by `AEGIS_REQUEST_DEADLINE_SECS`, not the sum of every provider's
individual timeout.

**Graceful shutdown** — send SIGTERM mid-request and confirm in-flight requests finish and
usage events flush before the process exits:
```bash
cargo run --bin aegis-gateway &
GATEWAY_PID=$!
# fire a request, then immediately signal
kill -TERM $GATEWAY_PID
```
Expect the log to show it waiting for in-flight work before exiting, not an abrupt kill.

**Token circuit breaker** (`AEGIS_MAX_TOKENS_PER_REQUEST`):
```bash
AEGIS_MAX_TOKENS_PER_REQUEST=100 cargo run --bin aegis-gateway &
curl -s -X POST http://localhost:8080/v1/chat/completions -H "x-api-key: $AEGIS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o","max_tokens":100000,"messages":[{"role":"user","content":"hi"}]}'
```
Expect the request's `max_tokens` silently clamped to 100 before it ever reaches the
provider — a single runaway request can't blow past this regardless of what the caller
asked for, independent of any budget check.

---

## 17. The dashboard (web app) — every page

With the dev server running (`npm run dev`, `http://localhost:3000`) and a real session:

| Page | What to check |
|---|---|
| `/dashboard` | Overview stats match `/api/usage/summary` |
| `/requests` | Table matches `/api/requests`; attribution chips show requested vs. served |
| `/usage` | Token/cost breakdown over time |
| `/savings` | Matches the hand-computed formula from section 14 |
| `/keys` | Create/revoke a key here, confirm it appears/disappears via `/api/keys` too |
| `/providers` | Add a BYOK credential, confirm "Test connection" actually calls the provider |
| `/models` | Model catalogue with tier/pricing, matches `/api/models` |
| `/policies` | Create a routing policy, confirm it actually constrains routing |
| `/budgets` | Create a budget, exceed it via curl, confirm the anomaly/breach shows here |
| `/team` | Invite a member, confirm role (reader/writer) actually gates what they can do |
| `/billing` | Plan, share rate, and limits match `/api/billing/plan` exactly |
| `/settings` | Zero-retention toggle, region, org name |

Every page: check DevTools console for errors, check the Network tab for the actual API
calls and status codes, don't just trust that it "looks right."

---

## 18. Marketing site and IDE integration

- `/` — hero, live routing simulator (client-side demo data, not a real backend call —
  labeled as illustrative), savings calculator (real formula, hypothetical inputs).
- `/pricing` — cross-check every number against `/api/billing/plan` and
  `middleware::rate_limit::org_limit_for_plan` directly, not just the copy.
- `/connect` — setup instructions for pointing an existing tool (Cursor, Continue, aider,
  the OpenAI/Anthropic SDKs, Claude Code) at Aegis by changing one base URL.
- **VS Code extension** (`apps/vscode-extension/`) — configures *existing* AI assistants
  (Copilot, Continue, etc.) to route through Aegis; it is not itself an AI assistant.
  Verify: install the `.vsix`, run its configure command, confirm it writes the target
  extension's settings to point at `http://localhost:8080/v1`.

---

## What's genuinely NOT built yet, so you don't go looking for it

- **A full end-to-end HTTP-level test of the streaming tool-call fix** — `BlockTracker`
  (the Anthropic-side block state machine) and both outbound renderers are each directly
  unit-tested, and that's what actually caught a real bug in the tracker before it shipped,
  but nothing yet drives the real `/v1/messages` or `/v1/chat/completions` HTTP handler
  with `providers::mock::MockBehavior::SucceedWithToolCall` and reads the resulting SSE
  bytes. Reasonable next addition.
- **Provider invoice reconciliation** (checking Aegis's stored prices against what OpenAI/
  Google actually billed) — discussed this session as the strongest lever for reducing
  manual price verification, not implemented.
- **Automated pricing updates** — by design. `docs/runbooks/pricing-update.md` requires a
  human for every real price change; nothing writes to `model_pricing` automatically, ever.
- **k6 load test, backup/restore drill** — both require live infrastructure this machine
  doesn't have; scripted but never run for real.
- **Person-level spend aggregation across API keys** — currently per-key/per-team, not
  per-individual across keys; a known open founder decision.
