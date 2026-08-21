"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type ApiKey, type CreatedKey } from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorState,
  Field,
  SectionHeader,
  TableShell,
  Td,
  Th,
} from "@/components/ui";
import { formatRelative, formatUsd } from "@/lib/format";

/**
 * API key management.
 *
 * The critical interaction is key creation: the plaintext exists exactly once, in the
 * response, and is never recoverable. The UI has to make that unmissable — a user who
 * dismisses the dialog without copying has to create a new key, and if the message was
 * ambiguous that is our fault rather than theirs.
 */
export default function KeysPage() {
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [newKeyName, setNewKeyName] = useState("");
  const [created, setCreated] = useState<CreatedKey | null>(null);
  const [copied, setCopied] = useState(false);

  async function load() {
    try {
      const response = await api.listKeys();
      setKeys(response.keys);
      setError(null);
    } catch (caught) {
      setError(caught instanceof ApiError ? caught.message : "Could not load keys.");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function handleCreate(event: React.FormEvent) {
    event.preventDefault();
    if (!newKeyName.trim()) return;

    setCreating(true);
    try {
      const response = await api.createKey({ name: newKeyName.trim() });
      setCreated(response);
      setNewKeyName("");
      setCopied(false);
      await load();
    } catch (caught) {
      setError(caught instanceof ApiError ? caught.message : "Could not create the key.");
    } finally {
      setCreating(false);
    }
  }

  async function handleRevoke(key: ApiKey) {
    // A revoked key stops working immediately and cannot be restored, so this is one of
    // the few places a confirmation is genuinely warranted rather than reflexive.
    const confirmed = window.confirm(
      `Revoke "${key.name}"? Any application using it will start receiving 401 errors immediately. This cannot be undone.`,
    );
    if (!confirmed) return;

    try {
      await api.revokeKey(key.id);
      await load();
    } catch (caught) {
      setError(caught instanceof ApiError ? caught.message : "Could not revoke the key.");
    }
  }

  async function copyKey() {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.key);
      setCopied(true);
    } catch {
      // Clipboard access can be denied. The key is visible and selectable regardless,
      // so this is a convenience failure rather than a blocking one.
      setCopied(false);
    }
  }

  return (
    <>
      <SectionHeader
        title="API keys"
        description="Use these in place of your provider key. The base URL change is the only other thing your application needs."
      />

      {error && (
        <div className="mb-6">
          <ErrorState message={error} />
        </div>
      )}

      {/* The key is shown once. Everything about this panel says so. */}
      {created && (
        <Card className="mb-6 border-[var(--color-accent-dim)] bg-[var(--color-accent-wash)] p-5">
          <h3 className="text-sm font-medium text-[var(--color-accent)]">
            Copy this key now — it will not be shown again
          </h3>
          <p className="mt-1 text-xs text-[var(--color-ink-muted)]">
            {created.warning}
          </p>

          <div className="mt-4 flex flex-wrap items-center gap-2">
            <code className="tabular flex-1 overflow-x-auto rounded-[var(--radius)] border border-[var(--color-line-strong)] bg-[var(--color-base)] px-3 py-2 text-xs text-[var(--color-ink)]">
              {created.key}
            </code>
            <Button variant="secondary" onClick={copyKey}>
              {copied ? "Copied" : "Copy"}
            </Button>
            <Button variant="ghost" onClick={() => setCreated(null)}>
              Done
            </Button>
          </div>
        </Card>
      )}

      <Card className="mb-6 p-5">
        <form onSubmit={handleCreate} className="flex flex-wrap items-end gap-3">
          <div className="min-w-[220px] flex-1">
            <Field
              label="Create a new key"
              id="key-name"
              value={newKeyName}
              onChange={setNewKeyName}
              placeholder="production-api"
              hint="A name you will recognise in six months."
            />
          </div>
          <Button type="submit" disabled={creating || !newKeyName.trim()}>
            {creating ? "Creating…" : "Create key"}
          </Button>
        </form>
      </Card>

      <Card>
        {loading ? (
          <p className="px-6 py-10 text-center text-sm text-[var(--color-ink-subtle)]">
            Loading…
          </p>
        ) : keys.length === 0 ? (
          <EmptyState
            title="No keys yet"
            description="Create one above to start routing requests through Aegis."
          />
        ) : (
          <TableShell>
            <thead>
              <tr>
                <Th>Name</Th>
                <Th>Key</Th>
                <Th>Rate limit</Th>
                <Th>Budget</Th>
                <Th>Last used</Th>
                <Th align="right">&nbsp;</Th>
              </tr>
            </thead>
            <tbody>
              {keys.map((key) => {
                const revoked = key.revoked_at !== null;
                return (
                  <tr key={key.id} className={revoked ? "opacity-50" : undefined}>
                    <Td>
                      <span className="text-[var(--color-ink)]">{key.name}</span>
                      {revoked && (
                        <span className="ml-2">
                          <Badge tone="danger">revoked</Badge>
                        </span>
                      )}
                    </Td>
                    <Td mono muted>
                      {key.key_prefix}…
                    </Td>
                    <Td mono>{key.rate_limit_per_minute}/min</Td>
                    <Td mono>
                      {key.monthly_budget_mc === null
                        ? "—"
                        : formatUsd(key.monthly_budget_mc)}
                    </Td>
                    <Td muted>{formatRelative(key.last_used_at)}</Td>
                    <Td align="right">
                      {!revoked && (
                        <button
                          type="button"
                          onClick={() => handleRevoke(key)}
                          className="text-sm text-[var(--color-ink-subtle)] transition-colors hover:text-[var(--color-danger)]"
                        >
                          Revoke
                        </button>
                      )}
                    </Td>
                  </tr>
                );
              })}
            </tbody>
          </TableShell>
        )}
      </Card>
    </>
  );
}
