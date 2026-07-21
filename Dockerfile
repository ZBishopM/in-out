# Build + run the finance ingest API (and the CLI worker) for agapornis.
FROM rust:1-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release -p finance-worker --bins

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/ingest_api /usr/local/bin/ingest_api
COPY --from=build /app/target/release/finance-worker /usr/local/bin/finance-worker
ENV INGEST_BIND=0.0.0.0:8090
EXPOSE 8090
CMD ["ingest_api"]
