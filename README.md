# 🛡️ Aegis: High-Performance AI Cost Optimization Gateway

<p align="center">
  <strong>Cut your enterprise AI bills by 40% to 70% without sacrificing response quality.</strong><br>
  <em>Engineered for the Razorpay Buildathon 2026 — Financial precision, microsecond latency, and enterprise-grade infrastructure.</em>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Language-Rust_2021-DEA584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Gateway_Overhead-P99_%3C_0.9ms-00D26A?style=for-the-badge" alt="Latency" />
  <img src="https://img.shields.io/badge/Accounting-Integer_Micro--Cents-0070F3?style=for-the-badge" alt="Financial Math" />
  <img src="https://img.shields.io/badge/Frontend-Next.js_15_App_Router-black?style=for-the-badge&logo=next.js&logoColor=white" alt="Next.js" />
  <img src="https://img.shields.io/badge/Storage-PostgreSQL_Partitioned_%2B_Redis-336791?style=for-the-badge&logo=postgresql&logoColor=white" alt="Postgres" />
  <img src="https://img.shields.io/badge/Vector_Engine-Qdrant_%2B_ONNX_Local-E6194B?style=for-the-badge&logo=qdrant&logoColor=white" alt="Qdrant" />
</p>

---

## ⚡ Executive Summary

Enterprises and AI scale-ups are bleeding capital on unoptimized LLM infrastructure. Over **60% of all enterprise LLM spend is pure waste**:
- Overpowered frontier models (`GPT-4o`, `Claude 3.5 Sonnet`) assigned to mechanical tasks like summarization, JSON parsing, or factual Q&A.
- Identical or semantically equivalent prompts repeatedly sent to upstream providers with zero cache hit leverage.
- Massive, bloated context windows containing duplicate corporate system guidelines and whitespace padding.
- Lack of financial attribution, real-time budget circuit breakers, or verifiable cost accounting.

**Aegis is an intelligent, high-throughput financial proxy and AI cost optimization gateway.** Built from the ground up in memory-safe **Rust**, Aegis intercepts LLM API traffic via a **single-line drop-in integration** (`baseURL`), enforcing real-time prompt compression, multi-tier semantic caching, and dynamic complexity routing in **less than 1 millisecond of gateway overhead**.

---

## 🏗️ System Architecture

```mermaid
flowchart TD
    Client["Client Applications<br/>(OpenAI / Anthropic SDKs)"]
    
    subgraph Aegis["🛡️ Aegis Gateway (Rust Tokio / Axum Core)"]
        Ingress["Unified Ingress Proxy<br/>(/v1/chat/completions)"]
        Auth["[1] Auth & RBAC Guard<br/>Scoped by org_id"]
        Budget["[2] Budget Circuit Breaker<br/>Hard spend limits & token caps"]
        Compressor["[3] Context Compression<br/>Prompt deduplication & whitespace stripping"]
        
        subgraph Caching["Triple-Tier Hybrid Cache Engine"]
            HotCache["Hot Tier: Redis<br/>SHA-256 exact fingerprint (<0.5ms)"]
            DurableCache["Durable Tier: PostgreSQL<br/>AES-GCM encrypted long-tail cache"]
            SemanticCache["Semantic Tier: Qdrant + ONNX<br/>Local bge-small-en-v1.5 embeddings (<35ms)"]
        end
        
        Router["[5] Complexity Classifier & Smart Router<br/>0.0 - 1.0 heuristic syntactic scoring"]
        Ledger["[6] Micro-Cent Financial Ledger<br/>Usage writer & reconciliation stream"]
    end
    
    subgraph Upstream["Upstream Model Providers"]
        OpenAI["OpenAI (GPT-4o, o3, mini)"]
        Anthropic["Anthropic (Claude 3.5/3.7)"]
        Google["Google Gemini (2.5 Flash/Pro)"]
        OpenRouter["OpenRouter / DeepSeek / Groq"]
    end
    
    Dashboard["Next.js 15 Management Console<br/>(Real-time analytics, receipts, budgets, keys)"]

    Client -->|1-line baseURL change| Ingress
    Ingress --> Auth --> Budget --> Compressor
    Compressor --> HotCache
    HotCache -->|Miss| DurableCache
    DurableCache -->|Miss| SemanticCache
    SemanticCache -->|Hit: 0ms LLM Call, $0 Cost| Ingress
    SemanticCache -->|Miss| Router
    Router -->|Passthrough / Routed| Upstream
    Upstream --> Ingress
    Ingress --> Ledger
    Ledger --> Dashboard
```

---

## 🚀 Key Technological Pillars

### 1. 🧠 Complexity-Aware Smart Routing
Aegis evaluates prompt difficulty on-the-fly using an in-process heuristic classifier. It scores syntax density, multi-step chain logic, reasoning verbs, and code markers between `0.0` (trivial) and `1.0` (complex reasoning).
- **Simple / Medium queries (score ≤ 0.50)** are intelligently routed from expensive `$2.50/Mtok` models to high-speed `$0.15/Mtok` models (`GPT-4o-mini`, `Gemini 2.5 Flash`).
- **Complex queries (> 0.50)** remain on the requested frontier model.
- **Strict Quality Guarantee**: The router resolves every edge-case ambiguity toward the requested model. Savings are extracted exclusively from the long tail of easy requests, never by degrading complex tasks.

### 2. ⚡ Triple-Tier Dual Caching Engine
Aegis implements three complementary cache tiers:
1. **Hot Tier (Redis)**: Sub-millisecond exact fingerprint match with automatic sliding TTL.
2. **Durable Tier (PostgreSQL)**: AES-GCM encrypted persistence for repeat historical queries promoted from the hot tier.
3. **Semantic Vector Tier (Qdrant + ONNX Runtime)**: Powered by an in-process local neural embedding model (`bge-small-en-v1.5`, 384 dimensions). When a user asks an already-answered query using different vocabulary (e.g. *"What is the capital of Australia?"* vs *"Can you tell me Australia's capital city?"*), Aegis detects the **> 85% cosine similarity match** and responds in **~30ms with $0.00 provider cost**.

### 3. 📉 Context Compression Engine
Redundant system messages, repeated multi-agent corporate directives, and excessive whitespace are compressed before the payload hits the upstream network:
- Safely strips token bloat while protecting code blocks, JSON schemas, and technical identifiers.
- Delivers **20% to 50% input token savings** on multi-turn conversations.

### 4. 💰 Financial Precision & Integer Micro-Cent Math
Aegis treats AI compute like financial transactions:
- **No Floating-Point Drift**: All pricing, billing, and savings are computed strictly in **64-bit integer Micro-Cents** (`1 USD = 100,000,000 micro-cents`). Invoices, export CSVs, and audit logs reconcile to the exact micro-cent.
- **Continuous Double-Entry Reconciliation**: A background worker continuously audits Redis usage streams against PostgreSQL partitioned tables, ensuring zero missing or duplicated usage events.
- **Automated Hard Circuit Breakers**: Configurable monthly budget caps immediately clamp or gracefully downgrade traffic before cost overruns occur.

### 5. 🔒 Enterprise Security & Data Sovereignty
- **Multi-Tenant Cryptographic Isolation**: Every database query, vector collection, and cache key is physically partitioned and scoped by `org_id`.
- **Zero Data Retention (ZDR) Mode**: When enabled, prompts and completions are processed strictly in volatile RAM and immediately discarded—essential for BFSI, healthcare, and GDPR compliance.
- **Credential Redaction**: Built-in regex filters strip 9 distinct API key formats and sensitive credentials from all system logs and distributed traces.

---

## 🔌 1-Line Drop-in Integration

Aegis conforms 100% to the standard OpenAI and Anthropic REST specifications. Developers only swap their client configuration:

### Python (OpenAI SDK)
```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",  # Point to Aegis Gateway
    api_key="aegis_sk_your_org_key_here"  # Your Aegis Key
)

response = client.chat.completions.create(
    model="openrouter/openai/gpt-4o",
    messages=[{"role": "user", "content": "Explain microservices architecture in 2 sentences."}]
)
```

### TypeScript / Node.js
```typescript
import OpenAI from "openai";

const openai = new OpenAI({
  baseURL: "http://localhost:8080/v1",
  apiKey: "aegis_sk_your_org_key_here",
});

const completion = await openai.chat.completions.create({
  model: "openrouter/openai/gpt-4o",
  messages: [{ role: "user", content: "Summarize this quarterly memo." }],
});
```

---

## 🧾 Verifiable Per-Request Receipt Headers

Every HTTP response returned by Aegis includes transparent metadata and financial accounting:

```http
HTTP/1.1 200 OK
content-type: application/json
x-aegis-model: openrouter/openai/gpt-4o-mini
x-aegis-requested-model: openrouter/openai/gpt-4o
x-aegis-cost: $0.000053
x-aegis-baseline-cost: $0.001080
x-aegis-savings: $0.001027
x-aegis-cache: miss
x-aegis-routing: complexity
x-aegis-compression-before-tokens: 200
x-aegis-compression-after-tokens: 118
x-aegis-compression-tokens-saved: 82
x-aegis-compression-ratio: 41.0%
x-aegis-latency: 1744ms (overhead: 0.381ms)
```

---

## 📊 Live Benchmarks

| Metric | Direct Provider Call | Aegis Gateway Proxy | Improvement |
|---|---|---|---|
| **Frontier Query Cost** | $0.001080 | $0.000053 | **95.1% Saved** |
| **Exact / Semantic Cache Hit** | $0.000076 | **$0.000000** | **100.0% Saved** |
| **Semantic Cache Latency** | 2,850ms | **32ms** | **89x Faster** |
| **Context Window Consumption** | 200 tokens | 118 tokens | **41% Reduction** |
| **Gateway Routing Overhead** | N/A | **0.38ms (P99 < 1ms)**| Negligible |

---

## 💼 Business Model & GTM

Aegis aligns incentives completely with customers through a **Value-Share Monetization Model**:

1. **Pure Performance Sharing (Core Model)**:
   - We charge a **15% to 20% fee on the verified gross savings** generated.
   - *Example*: If an enterprise spends $100,000/month on LLMs and Aegis reduces that bill to $40,000 (saving $60,000), Aegis collects $9,000 – $12,000. The customer retains **$48,000+ in pure net monthly budget**. If Aegis saves zero dollars, the customer pays zero dollars.
2. **Enterprise Tier Subscriptions**:
   - Dedicated on-premises / private VPC deployment.
   - Custom SLAs, Zero Data Retention (ZDR) verification, and SAML/SSO directory sync.
3. **Go-To-Market (GTM)**:
   - **Bottom-Up Developer Adoption**: Zero-friction SDK drop-in allows individual engineers to start routing test traffic in 2 minutes.
   - **Top-Down Financial Mandate**: CFOs and Heads of Infrastructure can enforce global policy rules, hard spend limits, and provider failover across all internal business units without requiring engineers to rewrite application code.

---

## 🛠️ Quickstart: Running Locally

### Prerequisites
- [Rust](https://www.rust-lang.org/) (version 1.80+)
- [Docker & Docker Compose](https://www.docker.com/)
- [Node.js](https://nodejs.org/) (version 20+)

### 1. Clone & Provision Infrastructure
```bash
# Start PostgreSQL, Redis, and Qdrant vector database
docker compose -f infra/docker-compose.yml up -d
```

### 2. Launch the High-Performance Rust Gateway
```bash
# Build and run with local ONNX neural embeddings enabled
cargo run -p aegis-gateway --features local-embeddings
```
*The gateway will initialize and listen on `http://localhost:8080`.*

### 3. Launch the Management Dashboard
```bash
cd apps/web
npm install
npm run dev
```
*Open `http://localhost:3000` to view the landing page, live request logs, savings analytics, and API key management.*

---

## 🧪 Rigorous Verification & Code Quality

Aegis is held together by an exhaustive automated test matrix:
- **843 Passing Unit Tests** executed in **~1.2 seconds**.
- Zero compiler warnings, enforced with `cargo clippy --all-targets -- -D warnings`.
- Strict type-checking and design-token consistency across the Next.js web application.

```bash
# Run the complete test suite
cargo test --lib
```

---

## 👥 Team & Acknowledgements
Crafted with passion for the **Razorpay Buildathon 2026**. Designed to turn AI cost optimization from an afterthought into an exact financial engineering science.
