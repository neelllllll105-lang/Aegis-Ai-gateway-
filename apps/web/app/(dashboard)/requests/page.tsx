"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type RequestLogRow } from "@/lib/api";
import {
  Badge,
  Card,
  EmptyState,
  ErrorState,
  SectionHeader,
  TableShell,
  Td,
  Th,
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
            <p className="px-6 py-10 text-center text-sm text-[var(--color-ink-subtle)]">
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
                    <Td muted>{formatTimestamp(row.created_at)}</Td>
                    <Td mono>{bareModelName(row.requested_model)}</Td>
                    <Td mono>
                      <span
                        className={
                          row.served_model !== row.requested_model
                            ? "text-[var(--color-accent)]"
                            : undefined
                        }
                      >
                        {bareModelName(row.served_model)}
                      </span>
                    </Td>
                    <Td>
                      <RoutingBadge
                        reason={row.routing_reason}
                        cacheType={row.cache_type}
                      />
                    </Td>
                    <Td align="right" mono>
                      {row.input_tokens + row.output_tokens}
                    </Td>
                    <Td align="right" mono>
                      {formatUsd(row.actual_cost_mc)}
                    </Td>
                    <Td align="right" mono>
                      {row.gross_savings_mc > 0 ? (
                        <span className="text-[var(--color-accent)]">
                          {formatUsd(row.gross_savings_mc)}
                        </span>
                      ) : (
                        <span className="text-[var(--color-ink-faint)]">—</span>
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
    return <Badge tone="accent">cache · {cacheType}</Badge>;
  }
  switch (reason) {
    case "complexity":
      return <Badge tone="accent">routed</Badge>;
    case "policy":
      return <Badge tone="info">policy</Badge>;
    case "fallback":
      return <Badge tone="warn">fallback</Badge>;
    case "user_override":
      return <Badge>override</Badge>;
    default:
      return <Badge>passthrough</Badge>;
  }
}

function StatusBadge({ status }: { status: number }) {
  if (status >= 500) return <Badge tone="danger">{status}</Badge>;
  if (status >= 400) return <Badge tone="warn">{status}</Badge>;
  return <span className="text-[var(--color-ink-faint)]">{status}</span>;
}
