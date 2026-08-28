/**
 * Aegis for VS Code — the smallest useful integration, on purpose.
 *
 * This extension does not reimplement AI chat, does not add its own chat panel, and does
 * not proxy anything through itself. Aegis is already an OpenAI-compatible endpoint
 * (`/v1/chat/completions`) — the entire integration is: store a key securely, and make it
 * trivial to point an editor's *existing* AI assistant (Continue, Cline, or anything else
 * that accepts a custom OpenAI-compatible base URL) at it. That was the explicit scope
 * decision — "point the IDE's own AI assistant through Aegis," not a dedicated panel.
 *
 * No default endpoint is ever assumed. Aegis has no public production URL as of this
 * writing (see `MEMORY.md` — pre-revenue, nothing deployed publicly yet), so the base URL
 * is always asked for explicitly, never silently defaulted to a domain that may not exist
 * or may not be yours.
 */

import * as vscode from "vscode";

const SECRET_KEY = "aegis.apiKey";
const CONFIG_SECTION = "aegis";
const BASE_URL_SETTING = "baseUrl";

/** The chat model id to suggest in generated config — matches Aegis's own routing model
 * naming convention (`provider/model`), and `auto` tells Aegis's router to pick, which is
 * the whole point of routing through Aegis rather than naming one model directly. */
const SUGGESTED_MODEL = "auto";

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("aegis.setApiKey", () => setApiKey(context)),
    vscode.commands.registerCommand("aegis.clearApiKey", () => clearApiKey(context)),
    vscode.commands.registerCommand("aegis.setBaseUrl", () => setBaseUrl()),
    vscode.commands.registerCommand("aegis.copyContinueConfig", () =>
      copyContinueConfig(context),
    ),
    vscode.commands.registerCommand("aegis.showConnectionValues", () =>
      showConnectionValues(context),
    ),
    vscode.commands.registerCommand("aegis.openDashboard", () => openDashboard()),
  );
}

export function deactivate(): void {
  // Nothing to clean up — no background connections, no timers, no state this extension
  // owns beyond what VS Code's own SecretStorage/configuration already persist safely.
}

async function setApiKey(context: vscode.ExtensionContext): Promise<void> {
  const key = await vscode.window.showInputBox({
    title: "Aegis API Key",
    prompt: "Paste your Aegis API key (starts with aegis_sk_). Stored in VS Code's " +
      "encrypted secret storage — never written to settings.json, never logged.",
    password: true,
    ignoreFocusOut: true,
    validateInput: (value) => {
      if (!value.trim()) {
        return "API key cannot be empty.";
      }
      if (!value.startsWith("aegis_sk_")) {
        return 'Aegis API keys start with "aegis_sk_" — double-check you copied the ' +
          "right value from the Aegis dashboard.";
      }
      return null;
    },
  });

  if (!key) {
    return; // User cancelled — say nothing, don't nag.
  }

  await context.secrets.store(SECRET_KEY, key.trim());
  vscode.window.showInformationMessage(
    "Aegis API key stored. Next: run \"Aegis: Set Gateway URL\" if you're self-hosting, " +
      'then "Aegis: Copy Continue.dev Configuration" or "Aegis: Show Connection Values".',
  );
}

async function clearApiKey(context: vscode.ExtensionContext): Promise<void> {
  await context.secrets.delete(SECRET_KEY);
  vscode.window.showInformationMessage("Aegis API key cleared from secret storage.");
}

async function setBaseUrl(): Promise<void> {
  const current = getConfiguredBaseUrl();
  const url = await vscode.window.showInputBox({
    title: "Aegis Gateway URL",
    prompt:
      "The base URL of the Aegis gateway you're connecting to — your own self-hosted " +
      "instance, or wherever your team's Aegis deployment runs. Include the scheme " +
      '(https:// or http://). Example: http://localhost:8080 for a local dev gateway.',
    value: current ?? "",
    placeHolder: "http://localhost:8080",
    ignoreFocusOut: true,
    validateInput: (value) => {
      if (!value.trim()) {
        return "Gateway URL cannot be empty.";
      }
      try {
        const parsed = new URL(value.trim());
        if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
          return "URL must start with http:// or https://";
        }
      } catch {
        return "That doesn't look like a valid URL.";
      }
      return null;
    },
  });

  if (!url) {
    return;
  }

  await vscode.workspace
    .getConfiguration(CONFIG_SECTION)
    .update(BASE_URL_SETTING, normalizeBaseUrl(url.trim()), vscode.ConfigurationTarget.Global);
  vscode.window.showInformationMessage(`Aegis gateway URL set to ${normalizeBaseUrl(url.trim())}`);
}

/** Strip a trailing slash and any trailing `/v1`, since every generated config appends
 * `/v1` itself — avoids a doubled `/v1/v1` if someone pastes the API-root URL either way. */
function normalizeBaseUrl(raw: string): string {
  return raw.replace(/\/+$/, "").replace(/\/v1$/, "");
}

function getConfiguredBaseUrl(): string | undefined {
  return vscode.workspace.getConfiguration(CONFIG_SECTION).get<string>(BASE_URL_SETTING);
}

interface ConnectionValues {
  apiKey: string;
  apiBase: string;
}

/** Load the stored key and configured base URL, or explain exactly what's missing rather
 * than failing silently or guessing a default that might not be the user's own gateway. */
async function requireConnectionValues(
  context: vscode.ExtensionContext,
): Promise<ConnectionValues | undefined> {
  const apiKey = await context.secrets.get(SECRET_KEY);
  if (!apiKey) {
    const choice = await vscode.window.showWarningMessage(
      "No Aegis API key stored yet.",
      "Set API Key Now",
    );
    if (choice === "Set API Key Now") {
      await setApiKey(context);
    }
    return undefined;
  }

  const baseUrl = getConfiguredBaseUrl();
  if (!baseUrl) {
    const choice = await vscode.window.showWarningMessage(
      "No Aegis gateway URL configured yet.",
      "Set Gateway URL Now",
    );
    if (choice === "Set Gateway URL Now") {
      await setBaseUrl();
    }
    return undefined;
  }

  return { apiKey, apiBase: `${baseUrl}/v1` };
}

/**
 * Continue.dev's own docs (docs.continue.dev/customize/model-providers/top-level/openai)
 * moved `config.yaml` to the primary format in 2026 — `config.json` is documented as
 * deprecated. Generating YAML here rather than the older JSON shape is a deliberate
 * choice, verified against Continue's current docs rather than assumed from memory.
 */
function buildContinueConfigSnippet(values: ConnectionValues): string {
  return [
    "# Paste this into ~/.continue/config.yaml, under the top-level `models:` list.",
    "# If Continue already has other models configured, add this as one more entry",
    "# rather than replacing the file.",
    "models:",
    "  - name: Aegis (auto-routed)",
    "    provider: openai",
    `    model: ${SUGGESTED_MODEL}`,
    `    apiBase: ${values.apiBase}`,
    `    apiKey: ${values.apiKey}`,
    "    roles:",
    "      - chat",
    "      - edit",
    "      - apply",
    "",
    "# If chat requests fail to authenticate once apiBase is set, Continue's own docs",
    "# note some setups need the key sent as a header explicitly instead:",
    "#     requestOptions:",
    "#       headers:",
    "#         Authorization: Bearer " + values.apiKey,
  ].join("\n");
}

async function copyContinueConfig(context: vscode.ExtensionContext): Promise<void> {
  const values = await requireConnectionValues(context);
  if (!values) {
    return;
  }

  const snippet = buildContinueConfigSnippet(values);
  await vscode.env.clipboard.writeText(snippet);
  vscode.window.showInformationMessage(
    "Continue.dev config copied to clipboard. Paste it into ~/.continue/config.yaml " +
      '(open it via Continue\'s own "Open config.yaml" command) and save.',
  );
}

/**
 * Cline (and several other assistants) configure through their own settings UI rather
 * than a pasteable file — its "OpenAI Compatible" provider has explicit Base URL / API
 * Key / Model fields (confirmed against Cline's own docs, docs.cline.bot/provider-config/
 * openai-compatible; note their plain "OpenAI" provider has a currently-open bug where
 * the Base URL field doesn't render at all — "OpenAI Compatible" is the one that works).
 * The most useful thing this extension can do for that shape of tool is show the three
 * values clearly, not synthesize a file nothing will read.
 */
async function showConnectionValues(context: vscode.ExtensionContext): Promise<void> {
  const values = await requireConnectionValues(context);
  if (!values) {
    return;
  }

  const message =
    `Provider: OpenAI Compatible\nBase URL: ${values.apiBase}\n` +
    `API Key: ${values.apiKey}\nModel: ${SUGGESTED_MODEL}`;

  const choice = await vscode.window.showInformationMessage(
    "Enter these into your assistant's settings (e.g. Cline: gear icon → API Provider → " +
      '"OpenAI Compatible"). The key is shown in full — copy it now, this dialog won\'t ' +
      "reopen it.",
    { modal: true, detail: message },
    "Copy All",
  );

  if (choice === "Copy All") {
    await vscode.env.clipboard.writeText(message);
  }
}

async function openDashboard(): Promise<void> {
  const baseUrl = getConfiguredBaseUrl();
  if (!baseUrl) {
    vscode.window.showWarningMessage(
      'No Aegis gateway URL configured yet — run "Aegis: Set Gateway URL" first.',
    );
    return;
  }
  await vscode.env.openExternal(vscode.Uri.parse(baseUrl));
}
