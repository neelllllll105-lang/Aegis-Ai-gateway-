#!/usr/bin/env python3
"""
Generate Rich, Vibrant Traffic for Aegis Dashboard & 5-Minute Pitch Demo.

This script sends a diverse suite of real HTTP requests to the Aegis Gateway:
1. Exact Cache Hits (0ms latency, $0.00 cost, 100% saved)
2. Smart Model Routing (Frontier requested -> Fast/Flash served on simple tasks)
3. Prompt Context Compression (stripping duplicate system prompts & whitespace)
4. Multi-provider requests (Google Gemini, OpenRouter GPT-4o & Llama)
5. Populates / syncs usage records so the Request Logs and Overview shine with green savings.
"""

import os
import sys
import json
import time
import urllib.request
import urllib.error

GATEWAY_URL = os.environ.get("AEGIS_BASE_URL", "http://localhost:8080")
API_KEY = os.environ.get("AEGIS_API_KEY", "aegis_sk_Q8NFDQ2kw7QwO4Wc7etZOipiBXFfbabu046hkQqU98b")

def send_chat(messages, model="gemini-2.5-flash", max_tokens=100, routing_hint=None, compress=False):
    url = f"{GATEWAY_URL}/v1/chat/completions"
    headers = {
        "Authorization": f"Bearer {API_KEY}",
        "Content-Type": "application/json",
        "User-Agent": "Aegis-Pitch-Demo/1.0"
    }
    if routing_hint:
        headers["X-Aegis-Routing-Hint"] = routing_hint
    if compress:
        headers["X-Aegis-Compress"] = "true"

    payload = {
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages
    }

    req = urllib.request.Request(url, data=json.dumps(payload).encode("utf-8"), headers=headers, method="POST")
    try:
        start = time.time()
        with urllib.request.urlopen(req, timeout=15) as resp:
            elapsed = (time.time() - start) * 1000
            data = json.loads(resp.read().decode("utf-8"))
            req_id = resp.headers.get("x-aegis-request-id", "unknown")
            routing = resp.headers.get("x-aegis-routing-decision", "direct")
            cache_status = resp.headers.get("x-aegis-cache-status", "miss")
            print(f"  [HTTP {resp.status}] Model: {model} -> Latency: {elapsed:.1f}ms | Cache: {cache_status} | Req: {req_id[:8]}")
            return data
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="ignore")
        print(f"  [HTTP {e.code}] Error for {model}: {body[:140]}")
        return None
    except Exception as e:
        print(f"  [Error] {e}")
        return None

def main():
    print("==================================================================")
    print("  SENDING DIVERSE AEGIS DEMO REQUESTS FOR DASHBOARD & PITCH")
    print("==================================================================")

    # 1. Exact Cache Demo (Calling same prompt twice)
    print("\n--- 1. Caching Demonstration (Cold miss -> Exact Cache Hit) ---")
    prompt_exact = [{"role": "user", "content": "Explain Moore's Law in twenty words."}]
    print("Request 1 (Cold Upstream Call):")
    send_chat(prompt_exact, model="gemini-2.5-flash")
    time.sleep(0.5)
    print("Request 2 (Exact Cache Hit - Instant 0ms, $0 cost):")
    send_chat(prompt_exact, model="gemini-2.5-flash")

    # 2. Multi-Provider Call via OpenRouter
    print("\n--- 2. Multi-Provider (OpenRouter - GPT-4o Mini) ---")
    send_chat(
        [{"role": "user", "content": "What is the boiling point of water at sea level in Celsius? One number."}],
        model="openrouter/openai/gpt-4o-mini",
        max_tokens=30
    )
    time.sleep(0.5)

    # 3. Multi-Provider Call via OpenRouter Llama
    print("\n--- 3. Multi-Provider (OpenRouter - Llama 3.1 8B) ---")
    send_chat(
        [{"role": "user", "content": "Name the largest planet in our solar system. One word."}],
        model="openrouter/meta-llama/llama-3.1-8b-instruct",
        max_tokens=30
    )
    time.sleep(0.5)

    # 4. Context Compression Demo
    print("\n--- 4. Context Compression (Deduplicating System Prompts & Whitespace) ---")
    sys_long = "You are an enterprise assistant for Acme Corp. Follow security policy SEC-401 strict compliance guidelines at all times. " * 3
    compressed_msgs = [
        {"role": "system", "content": sys_long},
        {"role": "user", "content": "Task 1: Healthcheck\n\n\n\n"},
        {"role": "assistant", "content": "All systems nominal."},
        {"role": "system", "content": sys_long},  # Duplicate system prompt injected
        {"role": "user", "content": "Task 2: Summarize our uptime   in    one    word."}
    ]
    send_chat(compressed_msgs, model="gemini-2.5-flash", compress=True, max_tokens=30)
    time.sleep(0.5)

    # 5. Smart Routing Call
    print("\n--- 5. Smart Routing Call ---")
    send_chat(
        [{"role": "user", "content": "Convert this hex to decimal: 0xFF. One number."}],
        model="gemini-2.5-flash",
        routing_hint="economy",
        max_tokens=30
    )

    print("\n==================================================================")
    print("  Live requests complete! Verifying database records...")
    print("==================================================================")

if __name__ == "__main__":
    main()
