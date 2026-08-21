# Aegis gateway image.
#
# Two stages: a build stage with the full Rust toolchain, and a runtime stage containing
# the single static-ish binary and nothing else. The result is a few tens of megabytes
# with no compiler, no package manager, and no shell utilities for an attacker to use.

# ---- Build -------------------------------------------------------------------------
FROM rust:1-slim-bookworm AS builder

# pkg-config and libssl-dev are needed by some transitive crates even though we use
# rustls; ca-certificates lets the build fetch crates over TLS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Dependency manifests first, so a source-only change reuses the cached dependency build.
# This is the difference between a 30-second rebuild and a 5-minute one.
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY apps/gateway/Cargo.toml apps/gateway/Cargo.toml

# A stub source tree lets `cargo build` compile dependencies without the real code.
RUN mkdir -p apps/gateway/src/bin \
    && echo "fn main() {}" > apps/gateway/src/main.rs \
    && echo "" > apps/gateway/src/lib.rs \
    && echo "fn main() {}" > apps/gateway/src/bin/dump_features.rs \
    && cargo build --release --locked \
    && rm -rf apps/gateway/src

# Now the real source.
COPY apps/gateway/src apps/gateway/src
COPY apps/gateway/migrations apps/gateway/migrations
COPY apps/gateway/fixtures apps/gateway/fixtures

# Touch the entrypoints so cargo rebuilds them rather than reusing the stub artifacts.
RUN touch apps/gateway/src/main.rs apps/gateway/src/lib.rs \
    && cargo build --release --locked --bin aegis-gateway \
    && strip target/release/aegis-gateway

# ---- Runtime -----------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    # A non-root user with no login shell. If the process is ever compromised, the
    # attacker lands as an unprivileged account that cannot log in.
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin aegis

COPY --from=builder /build/target/release/aegis-gateway /usr/local/bin/aegis-gateway

USER aegis
WORKDIR /app

EXPOSE 8080

ENV AEGIS_BIND=0.0.0.0:8080 \
    AEGIS_ENV=prod \
    RUST_BACKTRACE=1

# Readiness, not liveness: the orchestrator should stop routing to an instance whose
# store is unreachable, without restarting a process that is working perfectly.
HEALTHCHECK --interval=15s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/aegis-gateway", "--health-check"]

ENTRYPOINT ["/usr/local/bin/aegis-gateway"]
