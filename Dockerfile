# syntax=docker/dockerfile:1.7
# Multi-stage build:
# 1. cargo-chef stage that caches dependency builds keyed off the lockfile.
# 2. builder that compiles the workspace.
# 3. distroless runtime that ships only the `ontology` binary.

FROM rust:1.82-slim AS chef
RUN cargo install --locked cargo-chef
WORKDIR /workspace

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /workspace/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin ontology
RUN strip /workspace/target/release/ontology

FROM gcr.io/distroless/cc-debian12 AS runtime
LABEL org.opencontainers.image.title="ontology"
LABEL org.opencontainers.image.source="https://github.com/masspe/ai-ontology.com"
COPY --from=builder /workspace/target/release/ontology /usr/local/bin/ontology
USER nonroot
WORKDIR /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/ontology", "--data", "/data"]
CMD ["serve", "--bind", "0.0.0.0:8080"]
