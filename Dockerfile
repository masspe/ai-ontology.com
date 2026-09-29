# syntax=docker/dockerfile:1.7
# One image, one process per tenant (STORAGE.md §10.8, ROADMAP §3.8):
# 1. web stage builds the React UI (same-origin API, no env baked in);
# 2. cargo-chef stage caches the dependency build keyed off the lockfile;
# 3. builder compiles the `ontology` binary;
# 4. distroless runtime ships the binary and the built UI, nothing else.
#
# Run:  docker run -v ontology-data:/data -p 5000:5000 ontology
# The binary serves the API and the UI on :5000, keeps its users in
# /data/users.json (first account = administrator) and its JWT secret in
# /data/jwt.secret (generated on first start; to provide your own, add
# `--jwt-secret-env ONTOLOGY_JWT_SECRET` to the command and set that
# variable). See compose.yaml and docs/DEPLOIEMENT.md.

FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json web/.npmrc ./
RUN npm ci
COPY web/ ./
# Same-origin: the UI calls the API and /auth on the host that served it.
ENV VITE_API_BASE="" VITE_AUTH_API_BASE=""
RUN npx vite build

# std::fs::File::try_lock (store LOCK) needs Rust >= 1.89; CI runs current stable.
FROM rust:1.98-slim AS chef
RUN cargo install --locked cargo-chef
WORKDIR /workspace

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /workspace/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin ontology
RUN strip /workspace/target/release/ontology

FROM gcr.io/distroless/cc-debian12 AS runtime
LABEL org.opencontainers.image.title="ontology"
LABEL org.opencontainers.image.source="https://github.com/masspe/ai-ontology.com"
COPY --from=builder /workspace/target/release/ontology /usr/local/bin/ontology
COPY --from=web /web/dist /srv/web
USER nonroot
WORKDIR /data
VOLUME ["/data"]
EXPOSE 5000
ENTRYPOINT ["/usr/local/bin/ontology", "--data", "/data"]
# --memory-mode strict: a store that does not fit the container's budget
# refuses to start with both figures instead of being killed later (R17).
CMD ["--memory-mode", "strict", "serve", "--bind", "0.0.0.0:5000", "--web", "/srv/web", "--login"]
