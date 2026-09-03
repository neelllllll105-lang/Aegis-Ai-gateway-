"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type RequestLogRow } from "@/lib/api";
import {
  AttributionChip,
  Badge,
  Card,
  EmptyState,
  ErrorState,
  SectionHeader,
  Stamp,
  TableShell,
  Td,
  Th,
  type StampTone,
} from "@/components/ui";
import {
  bareModelName,
  formatLatency,
  formatTimestamp,
  formatUsd,
} from "@/lib/format";

/**
 * The request log.
 *
 * There is no content column and there never will be. Principle 4: we store operational
 * metadata only, so there is nothing to show even if someone asked. Saying that on the
 * page is better than leaving people to wonder whether we simply have not built it yet.
 */
export default function RequestsPage() {
  const [rows, setRows] = useState<RequestLogRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    api
      .requests(200)
      .then((response) => {
        if (cancelled) return;
        setRows(response.requests);
        setLoading(false);
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        setError(
          caught instanceof ApiError ? caught.message : "Could not load the request log.",
        );
        setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <>
      <SectionHeader
        title="Requests"
        description="Metadata for every request. Prompt and response content is never stored, so it is not shown here — that is by design, not an omission."
      />

      {error ? (
        <ErrorState message={error} />
      ) : (
        <Card>
          {loading ? (
            <p className="px-6 py-10 text-center text-sm text-[var(--color-muted-light)]">
              Loading…
            </p>
          ) : rows.length === 0 ? (
            <EmptyState
              title="No requests in this period"
              description="Once traffic flows through the gateway, every request appears here with its routing decision and cost."
            />
          ) : (
            <TableShell>
              <thead>
                <tr>
                  <Th>Time</Th>
                  <Th>Requested</Th>
                  <Th>Served</Th>
                  <Th>Routing</Th>
                  <Th align="right">Tokens</Th>
                  <Th align="right">Cost</Th>
                  <Th align="right">Saved</Th>
                  <Th align="right">Latency</Th>
                  <Th align="right">Status</Th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr key={row.request_id}>
                    <Td muted mono>{formatTimestamp(row.created_at)}</Td>
                    <Td>
                      <AttributionChip actor="you" label={bareModelName(row.requested_model)} />
                    </Td>
                    <Td>
                      <AttributionChip
                        actor="agent"
                        label={bareModelName(row.served_model)}
                      />
                    </Td>
                    <Td>
                      <div className="flex flex-wrap items-center gap-1">
                        <RoutingBadge
                          reason={row.routing_reason}
                          cacheType={row.cache_type}
                        />
                        {row.tokens_saved_by_compression && row.tokens_saved_by_compression > 0 ? (
                          <Stamp tone="agent">
                            compressed
                          </Stamp>
                        ) : null}
                      </div>
                    </Td>
                    <Td align="right" mono>
                      <div className="flex flex-col items-end leading-tight">
                        <span>
                          {row.input_tokens.toLocaleString()} in / {row.output_tokens.toLocaleString()} out
                        </span>
                        {row.cached_input_tokens && row.cached_input_tokens > 0 ? (
                          <span className="text-[11px] text-[var(--color-positive)] font-sans">
                            {row.cached_input_tokens.toLocaleString()} cached
                          </span>
                        ) : null}
                        {row.tokens_saved_by_compression && row.tokens_saved_by_compression > 0 ? (
                          <span className="text-[11px] text-amber-500 dark:text-amber-400 font-sans">
                            -{row.tokens_saved_by_compression.toLocaleString()} saved
                          </span>
                        ) : null}
                      </div>
                    </Td>
                    <Td align="right" mono>
                      <div className="flex flex-col items-end leading-tight">
                        <span className="font-semibold">{formatUsd(row.actual_cost_mc)}</span>
                        {(row.input_cost_mc !== undefined || row.output_cost_mc !== undefined) && row.actual_cost_mc > 0 ? (
                          <span className="text-[11px] text-[var(--color-muted-light)] font-sans">
                            in: {formatUsd(row.input_cost_mc ?? 0)} · out: {formatUsd(row.output_cost_mc ?? 0)}
                          </span>
                        ) : null}
                      </div>
                    </Td>
                    <Td align="right" mono>
                      {row.gross_savings_mc > 0 ? (
                        <span className="text-[var(--color-positive)] font-bold">
                          {formatUsd(row.gross_savings_mc)}
                        </span>
                      ) : (
                        <span className="text-[var(--color-muted-light)]">—</span>
                      )}
                    </Td>
                    <Td align="right" mono muted>
                      {formatLatency(row.latency_ms)}
                    </Td>
                    <Td align="right" mono>
                      <StatusBadge status={row.status_code} />
                    </Td>
                  </tr>
                ))}
              </tbody>
            </TableShell>
          )}
        </Card>
      )}
    </>
  );
}

function RoutingBadge({
  reason,
  cacheType,
}: {
  reason: string;
  cacheType: string | null;
}) {
  if (cacheType) {
    return <Stamp tone="verdict">cache · {cacheType}</Stamp>;
  }
  const REASON_TONE: Record<string, StampTone> = {
    complexity: "muted",
    policy: "other",
    fallback: "pending",
    user_override: "agent",
  };
  const REASON_LABEL: Record<string, string> = {
    complexity: "routed",
    policy: "policy",
    fallback: "fallback",
    user_override: "override",
  };
  return (
    <Stamp tone={REASON_TONE[reason] ?? "muted"}>{REASON_LABEL[reason] ?? "passthrough"}</Stamp>
  );
}

function StatusBadge({ status }: { status: number }) {
  if (status >= 500) return <Badge tone="danger">{status}</Badge>;
  if (status >= 400) return <Badge tone="warn">{status}</Badge>;
  return <span className="text-[var(--color-muted-light)]">{status}</span>;
}
