FROM rust:1.97.1-trixie AS builder
WORKDIR /build
ENV RUSTUP_TOOLCHAIN=1.97.1
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY apps/cli ./apps/cli
COPY apps/desktop/src-tauri ./apps/desktop/src-tauri
COPY apps/runtime-installer ./apps/runtime-installer
COPY benchmarks ./benchmarks
COPY installer ./installer
COPY specs ./specs
RUN cargo build --locked --release -p safe-dedupe

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 safe-dedupe \
    && mkdir --parents /data /scan /reports \
    && chown --recursive safe-dedupe:safe-dedupe /data /reports
COPY --from=builder /build/target/release/safe-dedupe /usr/local/bin/safe-dedupe
COPY docker/entrypoint.sh /usr/local/bin/safe-dedupe-entrypoint
RUN chmod 0555 /usr/local/bin/safe-dedupe /usr/local/bin/safe-dedupe-entrypoint
USER safe-dedupe
ENV SAFE_DEDUPE_DATABASE=/data/state.db \
    SAFE_DEDUPE_LOG_DIRECTORY=/data/logs \
    SAFE_DEDUPE_SOURCE_ROOT=/scan \
    SAFE_DEDUPE_REPORT_DIRECTORY=/reports \
    SAFE_DEDUPE_MODE=scan \
    SAFE_DEDUPE_QUARANTINE_ROOT=""
VOLUME ["/data", "/reports"]
ENTRYPOINT ["/usr/local/bin/safe-dedupe-entrypoint"]
CMD ["check"]
