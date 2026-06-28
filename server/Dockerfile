FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/guideng-server /usr/local/bin/guideng-server
ENV GUIDENG_BIND=0.0.0.0:8080
ENV GUIDENG_DATA_PATH=/data/guideng.json
EXPOSE 8080
CMD ["guideng-server"]
