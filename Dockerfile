# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /workspace
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /usr/sbin/nologin tempcheck
COPY --from=builder /workspace/target/release/tempcheck /usr/local/bin/tempcheck
RUN mkdir -p /data /var/log/tempcheck \
    && chown -R tempcheck:tempcheck /data /var/log/tempcheck
USER tempcheck
WORKDIR /data
VOLUME ["/data", "/var/log/tempcheck"]
ENV RUST_LOG=tempcheck=info
ENTRYPOINT ["tempcheck"]
CMD ["daemon", "--db", "/data/tempcheck.db"]
