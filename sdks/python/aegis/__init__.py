"""Aegis Python SDK.

Why this is thin on purpose
---------------------------
The integration story is "change one base URL". An SDK that wraps chat completions in its
own types would undercut that: it becomes a thing to learn, a thing to migrate to, and a
thing to migrate away from — which is exactly the lock-in we tell customers we do not
create.

So this package deliberately does not wrap the chat API. Keep using the ``openai`` or
``anthropic`` package you already have, pointed at Aegis. What this adds is the part those
packages cannot give you: typed access to the savings attribution on every response, and
the management API.

Example
-------
>>> from openai import OpenAI
>>> from aegis import parse_attribution
>>>
>>> client = OpenAI(base_url="https://api.aegis.dev/v1", api_key="aegis_sk_...")
>>> response = client.chat.completions.with_raw_response.create(
...     model="gpt-4o",
...     messages=[{"role": "user", "content": "What is 2+2?"}],
... )
>>> attribution = parse_attribution(response.headers)
>>> if attribution:
...     print(f"served by {attribution.served_model}, saved {attribution.savings_usd}")
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Mapping
from urllib.parse import urlencode

__all__ = [
    "AegisAttribution",
    "AegisClient",
    "AegisError",
    "UsageSummary",
    "parse_attribution",
    "routing_headers",
    "format_usd",
    "DEFAULT_BASE_URL",
    "MICRO_CENTS_PER_USD",
]

__version__ = "0.1.0"

#: Default gateway URL.
DEFAULT_BASE_URL = "https://api.aegis.dev"

#: Micro-cents in one US dollar. Every monetary value crosses the API as an integer.
MICRO_CENTS_PER_USD = 1_000_000


class AegisError(Exception):
    """An error returned by the Aegis API.

    Carries the machine-readable ``type`` so callers can branch on it rather than
    matching on message text, which changes.
    """

    def __init__(self, status: int, error_type: str, message: str) -> None:
        super().__init__(f"{error_type}: {message}")
        self.status = status
        self.error_type = error_type
        self.message = message


@dataclass(frozen=True)
class AegisAttribution:
    """The savings attribution Aegis reports on every response.

    All monetary fields are integer micro-cents, mirroring the gateway, so arithmetic on
    them stays exact.
    """

    served_model: str
    requested_model: str
    cost_micro_cents: int
    baseline_cost_micro_cents: int
    savings_micro_cents: int
    cache: str
    routing: str
    overhead_ms: float | None
    latency_ms: float | None
    request_id: str

    @property
    def savings_usd(self) -> str:
        """The saving, formatted for display."""
        return format_usd(self.savings_micro_cents)

    @property
    def was_routed(self) -> bool:
        """True when a model other than the requested one served the request."""
        return self.served_model != self.requested_model

    @property
    def was_cached(self) -> bool:
        """True when the response came from cache and cost nothing."""
        return self.cache in ("exact", "semantic")


def parse_attribution(headers: Mapping[str, str] | Any) -> AegisAttribution | None:
    """Extract the savings attribution from response headers.

    Accepts any mapping, including the header objects from ``httpx``, ``requests``, and
    the OpenAI SDK raw-response wrapper.

    Returns ``None`` when the headers are absent — meaning the response did not come
    through Aegis, which is worth distinguishing from a request that saved nothing.
    """
    get = _build_header_reader(headers)

    served_model = get("x-aegis-model")
    request_id = get("x-aegis-request-id")

    # Both are present on every Aegis response.
    if not served_model or not request_id:
        return None

    total_ms, overhead_ms = _parse_latency(get("x-aegis-latency"))

    return AegisAttribution(
        served_model=served_model,
        requested_model=get("x-aegis-requested-model") or served_model,
        cost_micro_cents=_parse_usd(get("x-aegis-cost")),
        baseline_cost_micro_cents=_parse_usd(get("x-aegis-baseline-cost")),
        savings_micro_cents=_parse_usd(get("x-aegis-savings")),
        cache=get("x-aegis-cache") or "unknown",
        routing=get("x-aegis-routing") or "unknown",
        overhead_ms=overhead_ms,
        latency_ms=total_ms,
        request_id=request_id,
    )


def _build_header_reader(headers: Mapping[str, str] | Any):
    """Return a case-insensitive lookup over whatever header container was passed."""
    # httpx and requests header objects are already case-insensitive.
    if hasattr(headers, "get"):
        return lambda name: headers.get(name) or headers.get(name.title())

    lowered = {str(key).lower(): value for key, value in dict(headers).items()}
    return lambda name: lowered.get(name.lower())


def _parse_usd(value: str | None) -> int:
    """Parse a ``$0.007050`` header into integer micro-cents."""
    if not value:
        return 0
    try:
        return round(float(value.replace("$", "").replace(",", "")) * MICRO_CENTS_PER_USD)
    except ValueError:
        return 0


def _parse_latency(value: str | None) -> tuple[float | None, float | None]:
    """Parse ``842ms (overhead: 0.371ms)`` into (total, overhead)."""
    if not value:
        return None, None

    total_match = re.match(r"^([\d.]+)ms", value)
    overhead_match = re.search(r"overhead:\s*([\d.]+)ms", value)

    return (
        float(total_match.group(1)) if total_match else None,
        float(overhead_match.group(1)) if overhead_match else None,
    )


def format_usd(micro_cents: int) -> str:
    """Format micro-cents as USD, with precision suited to the magnitude.

    A per-request saving is often a fraction of a cent; rendering it as ``$0.00`` makes
    the arithmetic look wrong when a customer checks it.
    """
    usd = micro_cents / MICRO_CENTS_PER_USD
    if usd == 0:
        return "$0.00"
    if abs(usd) < 0.01:
        return f"${usd:.6f}"
    if abs(usd) < 1:
        return f"${usd:.4f}"
    return f"${usd:.2f}"


def routing_headers(hint: str = "auto") -> dict[str, str]:
    """Headers that force a routing decision.

    ``passthrough`` guarantees the model you asked for. That escape hatch is permanent.

    Raises:
        ValueError: if the hint is not one of ``auto``, ``passthrough``, or ``cheap``.
    """
    if hint not in ("auto", "passthrough", "cheap"):
        raise ValueError(
            f"unknown routing hint {hint!r}; expected auto, passthrough, or cheap"
        )
    return {"X-Aegis-Routing-Hint": hint}


@dataclass(frozen=True)
class UsageSummary:
    """Usage and savings for a period. All monetary values are micro-cents."""

    requests: int
    cache_hits: int
    input_tokens: int
    output_tokens: int
    baseline_cost_micro_cents: int
    actual_cost_micro_cents: int
    gross_savings_micro_cents: int
    aegis_fee_micro_cents: int
    customer_net_micro_cents: int
    savings_percent: float
    cache_hit_rate: float


class AegisClient:
    """A minimal client for the Aegis management API.

    For reading your own usage and savings programmatically. It does not wrap chat
    completions; use your existing SDK for that.

    Requires ``httpx``, which is already a dependency of both the ``openai`` and
    ``anthropic`` packages, so this adds nothing new to a typical install.
    """

    def __init__(
        self,
        api_key: str,
        base_url: str = DEFAULT_BASE_URL,
        timeout: float = 30.0,
    ) -> None:
        if not api_key:
            raise ValueError("AegisClient requires an api_key")

        self.api_key = api_key
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def usage_summary(
        self,
        start: datetime | None = None,
        end: datetime | None = None,
    ) -> UsageSummary:
        """Usage and savings for a period, defaulting to the last 30 days."""
        params: dict[str, str] = {}
        if start:
            params["start"] = start.isoformat()
        if end:
            params["end"] = end.isoformat()

        query = f"?{urlencode(params)}" if params else ""
        body = self._request(f"/api/usage/summary{query}")

        summary = body["summary"]
        derived = body["derived"]

        return UsageSummary(
            requests=summary["requests"],
            cache_hits=summary["cache_hits"],
            input_tokens=summary["input_tokens"],
            output_tokens=summary["output_tokens"],
            baseline_cost_micro_cents=summary["baseline_cost_mc"],
            actual_cost_micro_cents=summary["actual_cost_mc"],
            gross_savings_micro_cents=summary["gross_savings_mc"],
            aegis_fee_micro_cents=summary["aegis_fee_mc"],
            customer_net_micro_cents=derived["customer_net_mc"],
            savings_percent=derived["savings_percent"],
            cache_hit_rate=derived["cache_hit_rate"],
        )

    def models(self) -> list[dict[str, Any]]:
        """Models available to your organisation, with their prices."""
        return self._request("/v1/models")["data"]

    def _request(self, path: str) -> dict[str, Any]:
        try:
            import httpx
        except ImportError as error:  # pragma: no cover - import guard
            raise ImportError(
                "AegisClient needs httpx. Install it with: pip install httpx"
            ) from error

        response = httpx.get(
            f"{self.base_url}{path}",
            headers={
                "Authorization": f"Bearer {self.api_key}",
                "Content-Type": "application/json",
            },
            timeout=self.timeout,
        )

        if response.status_code >= 400:
            error_type = "unknown_error"
            message = f"request failed with status {response.status_code}"
            try:
                body = response.json().get("error", {})
                error_type = body.get("type", error_type)
                message = body.get("message", message)
            except Exception:  # noqa: BLE001 - a non-JSON body keeps the default
                pass
            raise AegisError(response.status_code, error_type, message)

        return response.json()
