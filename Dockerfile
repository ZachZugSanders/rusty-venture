# ── Stage 1: cache external dependencies with stub sources ───────────────────
# This layer is only invalidated when Cargo.toml / Cargo.lock change.
FROM rust:1.85-slim-bookworm AS deps

WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Workspace manifest and lock file
COPY Cargo.toml Cargo.lock ./

# Per-crate manifests only — no real source yet
COPY crates/rusty-venture-core/Cargo.toml     crates/rusty-venture-core/
COPY crates/rusty-venture-llm/Cargo.toml      crates/rusty-venture-llm/
COPY crates/rusty-venture-actions/Cargo.toml  crates/rusty-venture-actions/
COPY crates/rusty-venture-store/Cargo.toml    crates/rusty-venture-store/
COPY crates/rusty-venture-vcs/Cargo.toml      crates/rusty-venture-vcs/
COPY crates/rusty-venture-improve/Cargo.toml  crates/rusty-venture-improve/
COPY crates/rusty-venture-cli/Cargo.toml      crates/rusty-venture-cli/
COPY crates/rusty-venture-server/Cargo.toml   crates/rusty-venture-server/

# Stub sources — just enough for Cargo to compile all external deps.
# rusty-venture-actions declares a bench, so stub that too.
# rusty-venture-store uses sqlx::migrate!() but only in the real db.rs, not
# the stub lib.rs, so an empty migrations/ dir is sufficient at this stage.
RUN set -e; \
    for c in core llm actions store vcs improve; do \
    mkdir -p crates/rusty-venture-${c}/src; \
    printf '' > crates/rusty-venture-${c}/src/lib.rs; \
    done; \
    mkdir -p crates/rusty-venture-cli/src; \
    printf 'fn main(){}' > crates/rusty-venture-cli/src/main.rs; \
    mkdir -p crates/rusty-venture-server/src; \
    printf 'fn main(){}' > crates/rusty-venture-server/src/main.rs; \
    mkdir -p crates/rusty-venture-actions/benches; \
    printf 'fn main(){}' > crates/rusty-venture-actions/benches/graph_bench.rs; \
    mkdir -p crates/rusty-venture-store/migrations

# Compile all external deps.  The stub workspace crates compile fine and link
# into a dummy binary; the important output is the .rlib files for every
# third-party crate in target/release/deps/.
RUN cargo build --release -p rusty-venture-server

# Remove only our own workspace-crate fingerprints and artifacts so the next
# stage is forced to recompile them against the real source.
RUN find target/release/.fingerprint -maxdepth 1 -name 'rusty*' -exec rm -rf {} + 2>/dev/null || true; \
    find target/release/deps -maxdepth 1 -name 'rusty_venture*' -exec rm -f {} + 2>/dev/null || true; \
    find target/release/deps -maxdepth 1 -name 'librusty_venture*' -exec rm -f {} + 2>/dev/null || true

# ── Stage 2: full build with real sources ────────────────────────────────────
FROM deps AS builder

# Real migration SQL files must be present because sqlx::migrate!("./migrations")
# embeds them at compile time.
COPY crates/ crates/

RUN cargo build --release -p rusty-venture-server

# ── Stage 3: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/rusty-venture-server /usr/local/bin/rusty-venture-server

# /data holds the SQLite database file across restarts
VOLUME ["/data"]

ENV DATABASE_URL=sqlite:///data/rusty-venture.db \
    LISTEN_ADDR=0.0.0.0:3002 \
    RUST_LOG=info

EXPOSE 3002

ENTRYPOINT ["rusty-venture-server"]
