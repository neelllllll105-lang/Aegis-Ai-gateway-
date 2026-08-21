/**
 * Load test for the Aegis gateway — MASTER_BUILD.md P4.8.
 *
 *   k6 run -e BASE_URL=https://staging.aegis.dev -e API_KEY=aegis_sk_... k6-gateway.js
 *
 * # What this measures, and what it does not
 *
 * The claim under test is Principle 1: **under 1ms of P99 overhead added by Aegis
 * itself**. That is not the same as end-to-end latency, which is dominated by the upstream
 * provider and would tell us nothing about our own code.
 *
 * So the assertion is on `X-Aegis-Latency`, which reports the gateway's measured overhead
 * with provider time excluded. We publish that number on every response precisely so it
 * can be checked — including by a customer running this script against us.
 *
 * Requests are sent with `X-Aegis-Routing-Hint: passthrough` and a cache-busting nonce, so
 * the test measures the full pipeline rather than a cache hit. A load test that
 * accidentally measures cache hits reports wonderful numbers and proves nothing.
 */

import http from "k6/http";
import { check, sleep } from "k6";
import { Trend, Rate, Counter } from "k6/metrics";

const BASE_URL = __ENV.BASE_URL || "http://localhost:8080";
const API_KEY = __ENV.API_KEY || "";

/** Gateway overhead, parsed from the response header. The number that matters. */
const gatewayOverhead = new Trend("aegis_gateway_overhead_ms");
/** Requests that returned a usable response. */
const successRate = new Rate("aegis_success_rate");
/** Rate-limit rejections, which are correct behaviour rather than failures. */
const rateLimited = new Counter("aegis_rate_limited");
/** Cache hits, which should be near zero given the nonce. */
const cacheHits = new Counter("aegis_cache_hits");

export const options = {
  scenarios: {
    // Ramp to 1000 RPS and hold for ten minutes, per P4.8.
    sustained_load: {
      executor: "ramping-arrival-rate",
      startRate: 50,
      timeUnit: "1s",
      preAllocatedVUs: 200,
      maxVUs: 2000,
      stages: [
        { target: 200, duration: "1m" },
        { target: 1000, duration: "2m" },
        { target: 1000, duration: "10m" },
        { target: 0, duration: "30s" },
      ],
    },
  },

  thresholds: {
    // The claim, asserted. If this fails, the product's central performance promise is
    // not true and the number on the landing page has to change.
    "aegis_gateway_overhead_ms": ["p(99)<1", "p(50)<0.5"],

    // Error budget. Rate-limit rejections are excluded because they are the system
    // working correctly under load, not failing.
    "aegis_success_rate": ["rate>0.99"],

    // A backstop on total latency, generous because the provider dominates it.
    "http_req_duration": ["p(99)<30000"],

    // A 5xx is our fault and should essentially never happen.
    "http_req_failed": ["rate<0.01"],
  },
};

export function setup() {
  if (!API_KEY) {
    throw new Error(
      "API_KEY is required. Run with: k6 run -e API_KEY=aegis_sk_... k6-gateway.js",
    );
  }

  // Fail fast on an unreachable or unhealthy target rather than reporting ten minutes of
  // connection errors as a performance result.
  const health = http.get(`${BASE_URL}/health`);
  if (health.status !== 200) {
    throw new Error(`${BASE_URL}/health returned ${health.status}; aborting.`);
  }

  console.log(`Target: ${BASE_URL}`);
  console.log(`Health: ${health.body}`);
  return { baseUrl: BASE_URL };
}

/** A range of prompt shapes, so the classifier sees realistic variety. */
const PROMPTS = [
  "What is the capital of France?",
  "Translate good morning into Spanish",
  "Summarize the benefits of connection pooling in two sentences.",
  "Write a Python function that reverses a linked list.",
  "Explain the difference between TCP and UDP.",
];

export default function (data) {
  // A nonce per iteration defeats the cache. Without it this measures cache hits and
  // reports a latency figure that means nothing.
  const nonce = `${__VU}-${__ITER}-${Date.now()}`;
  const prompt = PROMPTS[__ITER % PROMPTS.length];

  const payload = JSON.stringify({
    model: "gpt-4o",
    messages: [{ role: "user", content: `${prompt} (nonce ${nonce})` }],
    max_tokens: 16,
  });

  const response = http.post(`${data.baseUrl}/v1/chat/completions`, payload, {
    headers: {
      Authorization: `Bearer ${API_KEY}`,
      "Content-Type": "application/json",
      // Passthrough keeps the routing decision constant, so the overhead figure reflects
      // the pipeline rather than varying by which model was selected.
      "X-Aegis-Routing-Hint": "passthrough",
    },
    timeout: "60s",
    tags: { name: "chat_completions" },
  });

  // 429 is the rate limiter working, not a failure. Counting it as an error would make
  // a correctly-behaving system look broken under exactly the load it is designed for.
  if (response.status === 429) {
    rateLimited.add(1);
    successRate.add(true);
    sleep(1);
    return;
  }

  const ok = check(response, {
    "status is 200": (r) => r.status === 200,
    "body has choices": (r) => {
      try {
        return Array.isArray(JSON.parse(r.body).choices);
      } catch {
        return false;
      }
    },
    "overhead header present": (r) => r.headers["X-Aegis-Latency"] !== undefined,
  });

  successRate.add(ok);

  // Parse "842ms (overhead: 0.371ms)".
  const latencyHeader = response.headers["X-Aegis-Latency"];
  if (latencyHeader) {
    const match = latencyHeader.match(/overhead:\s*([\d.]+)ms/);
    if (match) {
      gatewayOverhead.add(parseFloat(match[1]));
    }
  }

  const cacheHeader = response.headers["X-Aegis-Cache"];
  if (cacheHeader === "exact" || cacheHeader === "semantic") {
    cacheHits.add(1);
  }
}

export function teardown() {
  console.log("");
  console.log("Load test complete.");
  console.log("");
  console.log("Read aegis_gateway_overhead_ms first — that is the Principle 1 claim.");
  console.log("A high cache-hit count means the nonce failed and the result is invalid.");
  console.log("");
  console.log("Also check on the server side:");
  console.log("  curl $BASE_URL/api/admin/metrics   (metering gap must be null)");
  console.log("  curl $BASE_URL/metrics | grep overhead");
}
