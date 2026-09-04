# Aegis for VS Code

The smallest useful integration between Aegis and your editor: point your existing AI
assistant (Continue, Cline, or anything else that accepts a custom OpenAI-compatible base
URL) at Aegis, so its requests get cost-optimized routing, caching, and metering — with
zero change to the assistant itself.

This is deliberately **not** a chat panel, a code-completion engine, or a proxy running
inside VS Code. Aegis already speaks the OpenAI-compatible `/v1/chat/completions`
protocol every one of these tools already expects; the entire job here is making the
three values they need (base URL, API key, model) trivial to get right, and keeping the
key out of plaintext settings files.

## Setup

1. **`Aegis: Set API Key`** — paste your Aegis API key (`aegis_sk_...`, from the Aegis
   dashboard's Keys page). Stored in VS Code's encrypted `SecretStorage`, never written to
   `settings.json`, never logged.
2. **`Aegis: Set Gateway URL`** — the base URL of *your* Aegis gateway. There is no
   default: Aegis has no public production URL as of this writing, and even once one
   exists, silently assuming it's yours would be wrong for anyone self-hosting. For local
   development this is typically `http://localhost:8080`.
3. Pick the command matching your assistant:
   - **Continue.dev**: `Aegis: Copy Continue.dev Configuration` — copies a ready-to-paste
     `config.yaml` snippet. Paste it into `~/.continue/config.yaml` (Continue's own
     "Open config.yaml" command finds the file for you) under the `models:` list.
   - **Cline, or anything else with an "OpenAI Compatible" provider option**:
     `Aegis: Show Connection Values` — displays the base URL, API key, and suggested model
     to enter into that tool's own settings UI. Most of these tools configure through
     clicked-in fields, not a file, so there's nothing to paste.

`Aegis: Open Dashboard` opens your configured gateway's dashboard in the browser.
`Aegis: Clear Stored API Key` removes it from secret storage.

## Why `model: auto`

The generated config suggests `auto` as the model name, not a specific one like
`gpt-4o-mini`. Aegis's own router picks the cheapest model that preserves quality for each
request — naming one model explicitly defeats that. If your assistant requires a
non-`auto` value, any real model id works too (Aegis's `X-Aegis-Requested-Model` vs.
`X-Aegis-Model` response headers show what was actually used vs. what was asked for).

## What this extension does not do

- Does not store or transmit your key anywhere except VS Code's own secret storage and,
  when you use it, directly to the gateway URL you configured.
- Does not phone home, does not collect telemetry, does not talk to any Aegis-operated
  service on its own — it only ever talks to the gateway URL you explicitly set.
- Does not modify Continue's or Cline's configuration files/settings automatically. Every
  command produces something for you to review and apply yourself, deliberately — an
  extension silently rewriting another extension's config file is exactly the kind of
  surprising behaviour this project avoids elsewhere in the codebase too.

## Status

`npm install`, `npm run typecheck`, `npm run compile`, and `npm run lint` all verified
clean against the real `@types/vscode`, `eslint@9`, and `@typescript-eslint` APIs — not
just written and assumed correct. **Not yet run inside an actual VS Code Extension
Development Host** — this environment has no way to launch VS Code itself, so the six
commands' interactive behavior (input boxes, secret storage, clipboard) is reviewed by
hand against the VS Code API, not exercised live. Before trusting this against a real
assistant:

```bash
cd apps/vscode-extension
npm install   # already verified clean here; re-run if node_modules isn't present
npm run compile
# Then, in VS Code itself: Run > Start Debugging (F5) opens an Extension Development
# Host window with this extension loaded, for real interactive testing.
```

Packaging for distribution (`vsce package`) and publishing to the VS Code Marketplace
needs a Microsoft publisher account this environment doesn't have — that's a deliberate
step for a human, not something to attempt here.
