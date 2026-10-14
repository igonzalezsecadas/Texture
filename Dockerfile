# texture — self-hosted PokerStars NLHE cash-game tracker
#
# Multi-stage build:
#   stage 1 (build):  compiles the release binary against the full Rust toolchain.
#   stage 2 (runtime): a minimal Debian image that ships only the binary, the
#                      bundled-SQLite statically-usable build, and a tiny HTTP
#                      checker for the HEALTHCHECK.
#
# Notes:
# - rusqlite is built with the `bundled` feature, so libsqlite3 is compiled
#   into the binary and no SQLite shared library is needed at runtime.
# - The binary links glibc dynamically, which is why the runtime stage uses
#   debian:bookworm-slim (the same base the rust:1-slim builder image is
#   built on). This keeps the glibc versions matched.
# - No ca-certificates and no tzdata are installed: the process performs no
#   outbound TLS and all timestamps are parsed from hand histories in UTC.
# - The container runs as a non-root user `texture` (uid/gid 1000). Host bind
#   mounts for the data directory must be writable by uid 1000.
# - ENTRYPOINT is exec-form and CMD is `serve`, so SIGTERM/SIGINT reach the
#   Rust runtime directly and `serve` can shut down cleanly.

# ---------------------------------------------------------------------------
# Stage 1 — build
# ---------------------------------------------------------------------------
FROM rust:1-slim AS build

WORKDIR /app

# Copy the whole source tree (the .dockerignore keeps target/, .git/,
# .scratch/, data/, hands/ and *.sqlite* out of the context).
COPY . .

# Make sure the templates directory exists even before the web layer ships
# one, so the runtime-stage COPY below cannot fail on a missing path. When
# src/web starts loading templates from disk at runtime, this directory is
# the one that must be populated; if templates are embedded at compile time
# instead, this COPY is simply redundant and copies nothing of consequence.
RUN mkdir -p /app/templates

# Build the release profile as configured in Cargo.toml (opt-level 3,
# thin LTO, codegen-units = 1). No build-time data directory or hand-history
# directory is needed: nothing is touched at build time.
RUN cargo build --release

# ---------------------------------------------------------------------------
# Stage 2 — runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# wget is the smallest honest HTTP client available for the healthcheck: it
# speaks enough HTTP to see the /healthz status code and has a tiny
# dependency footprint. curl is deliberately not installed.
RUN apt-get update \
    && apt-get install -y --no-install-recommends wget \
    && rm -rf /var/lib/apt/lists/* \
    # Non-root user. uid/gid 1000 keeps host bind-mount ownership simple.
    && groupadd --gid 1000 texture \
    && useradd --uid 1000 --gid texture --create-home --shell /usr/sbin/nologin texture

WORKDIR /app

# The binary (bundled SQLite, no runtime SQLite dependency). Templates, CSS and
# the charting library are all embedded in the binary with include_str!, so
# nothing else needs to be copied: there is no runtime asset path to get wrong.
COPY --from=build /app/target/release/texture /usr/local/bin/texture

# Data and hand-history directories. In practice these are bind-mounted from
# the host (see compose.yaml); the mkdir only makes bare `docker run` useful.
RUN mkdir -p /data /hands \
    && chown -R texture:texture /app /data /hands

USER texture

# The web listener. compose.yaml publishes this on 127.0.0.1 on the host.
EXPOSE 8080

# /healthz is served by `serve` (web layer). wget exits 0 on HTTP 200 and
# non-zero otherwise, which is exactly what a healthcheck needs.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD wget -q --spider http://127.0.0.1:8080/healthz || exit 1

# Exec-form so the binary is PID 1 and receives signals directly. The Rust
# runtime in `serve` handles SIGINT/SIGTERM for a clean shutdown: it stops
# the watcher and flushes the in-flight snapshot before exiting.
ENTRYPOINT ["/usr/local/bin/texture"]
CMD ["serve"]
