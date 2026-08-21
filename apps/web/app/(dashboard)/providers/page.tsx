"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type ProviderCredential } from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorState,
  Field,
  SectionHeader,
} from "@/components/ui";
import { formatRelative } from "@/lib/format";

const PROVIDERS = [
  "openai",
  "anthropic",
  "google",
  "openrouter",
  "deepseek",
  "mistral",
  "groq",
  "moonshot",
  "custom",
] as const;

/**
 * BYOK credential management.
 *
 * Stored keys are never shown again — the API returns only a four-character hint, because
 * the ciphertext is marked non-serialisable on the server. The Test button exists because
 * customers mistype keys constantly, and discovering that during a production request is
 * considerably worse than discovering it here.
 */
export default function ProvidersPage() {
  const [credentials, setCredentials] = useState<ProviderCredential[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<Record<string, string>>({});

  const [provider, setProvider] = useState<string>("openai");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [label, setLabel] = useState("");
  const [saving, setSaving] = useState(false);

  async function load() {
    try {
      const response = await api.listProviders();
      setCredentials(response.providers);
      setError(null);
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not load provider keys.",
      );
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function handleAdd(event: React.FormEvent) {
    event.preventDefault();
    if (!apiKey.trim()) return;

    setSaving(true);
    try {
      await api.createProvider({
        provider,
        api_key: apiKey.trim(),
        base_url: baseUrl.trim() || undefined,
        label: label.trim() || undefined,
      });
      // Clear the key from component state immediately. It has been sent and encrypted;
      // there is no reason for it to linger in the browser.
      setApiKey("");
      setBaseUrl("");
      setLabel("");
      await load();
    } catch (caught) {
      setError(caught instanceof ApiError ? caught.message : "Could not save the key.");
    } finally {
      setSaving(false);
    }
  }

  async function handleTest(credential: ProviderCredential) {
    setTesting(credential.id);
    try {
      const result = await api.testProvider(credential.id);
      setTestResult((previous) => ({
        ...previous,
        [credential.id]: result.ok ? "Working" : (result.error ?? "Failed"),
      }));
      await load();
    } catch (caught) {
      setTestResult((previous) => ({
        ...previous,
        [credential.id]:
          caught instanceof ApiError ? caught.message : "Test failed",
      }));
    } finally {
      setTesting(null);
    }
  }

  async function handleDelete(credential: ProviderCredential) {
    const confirmed = window.confirm(
      `Remove the ${credential.provider} key? Requests will fall back to shared models or fail if none are available.`,
    );
    if (!confirmed) return;

    try {
      await api.deleteProvider(credential.id);
      await load();
    } catch (caught) {
      setError(caught instanceof ApiError ? caught.message : "Could not remove the key.");
    }
  }

  return (
    <>
      <SectionHeader
        title="Provider keys"
        description="Bring your own keys and settle directly with each provider — we never mark up their pricing. Keys are encrypted with AES-256-GCM and never returned to the browser."
      />

      {error && (
        <div className="mb-6">
          <ErrorState message={error} />
        </div>
      )}

      <Card className="mb-6 p-5">
        <form onSubmit={handleAdd} className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <div>
              <label
                htmlFor="provider"
                className="block text-sm font-medium text-[var(--color-ink-muted)]"
              >
                Provider
              </label>
              <select
                id="provider"
                value={provider}
                onChange={(event) => setProvider(event.target.value)}
                className="mt-1.5 w-full rounded-[var(--radius)] border border-[var(--color-line-strong)] bg-[var(--color-base)] px-3 py-2 text-sm text-[var(--color-ink)]"
              >
                {PROVIDERS.map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))}
              </select>
            </div>

            <Field
              label="Label"
              id="label"
              value={label}
              onChange={setLabel}
              placeholder="Optional — e.g. production"
            />
          </div>

          <Field
            label="API key"
            id="api-key"
            type="password"
            value={apiKey}
            onChange={setApiKey}
            required
            hint="Encrypted at rest. Only the last four characters are ever displayed again."
          />

          {provider === "custom" && (
            <Field
              label="Base URL"
              id="base-url"
              value={baseUrl}
              onChange={setBaseUrl}
              required
              placeholder="https://my-endpoint.example.com/v1"
              hint="Any OpenAI-compatible endpoint: vLLM, Together, Azure OpenAI, or your own proxy."
            />
          )}

          <Button type="submit" disabled={saving || !apiKey.trim()}>
            {saving ? "Saving…" : "Add key"}
          </Button>
        </form>
      </Card>

      <Card>
        {loading ? (
          <p className="px-6 py-10 text-center text-sm text-[var(--color-ink-subtle)]">
            Loading…
          </p>
        ) : credentials.length === 0 ? (
          <EmptyState
            title="No provider keys"
            description="Without your own keys, free-tier requests run on our shared models. Add a key to use the full model range and settle directly with the provider."
          />
        ) : (
          <ul className="divide-y divide-[var(--color-line)]">
            {credentials.map((credential) => (
              <li
                key={credential.id}
                className="flex flex-wrap items-center justify-between gap-4 px-5 py-4"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-[var(--color-ink)]">
                      {credential.provider}
                    </span>
                    {credential.is_default && <Badge tone="accent">default</Badge>}
                    {credential.last_test_ok === true && <Badge tone="accent">verified</Badge>}
                    {credential.last_test_ok === false && (
                      <Badge tone="danger">failing</Badge>
                    )}
                  </div>
                  <div className="mt-1 text-xs text-[var(--color-ink-subtle)]">
                    <span className="tabular">{credential.key_hint ?? "••••"}</span>
                    {credential.label && <> · {credential.label}</>}
                    {credential.base_url && <> · {credential.base_url}</>}
                    <> · added {formatRelative(credential.created_at)}</>
                  </div>
                  {testResult[credential.id] && (
                    <div className="mt-1.5 text-xs text-[var(--color-ink-muted)]">
                      {testResult[credential.id]}
                    </div>
                  )}
                </div>

                <div className="flex shrink-0 gap-2">
                  <Button
                    variant="secondary"
                    onClick={() => handleTest(credential)}
                    disabled={testing === credential.id}
                  >
                    {testing === credential.id ? "Testing…" : "Test"}
                  </Button>
                  <Button variant="danger" onClick={() => handleDelete(credential)}>
                    Remove
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
