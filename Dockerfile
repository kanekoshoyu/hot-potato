# --- build stage ---
FROM rust:1.97-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY dashboard ./dashboard
RUN cargo build --release -p hot-potato --bin hot-potato-server

# --- runtime stage ---
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/hot-potato-server /usr/local/bin/hot-potato-server
EXPOSE 8080
ENV HOT_POTATO_ADDR=0.0.0.0:8080
ENTRYPOINT ["hot-potato-server"]
