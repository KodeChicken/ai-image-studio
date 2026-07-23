FROM node:22-alpine AS frontend-builder
WORKDIR /src/frontend
RUN corepack enable
COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY frontend/ ./
RUN pnpm typecheck && pnpm build

FROM rust:1.96-bookworm AS backend-builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY host-updater/Cargo.toml host-updater/Cargo.toml
COPY backend/src backend/src
COPY backend/migrations backend/migrations
COPY host-updater/src host-updater/src
RUN cargo build --locked --release \
    --package ai-image-studio \
    --package ai-image-studio-host-updater

FROM backend-builder AS host-updater-test
RUN cargo test --locked --release --package ai-image-studio-host-updater

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /app/data/images \
    && chown -R 10001:10001 /app
WORKDIR /app
COPY --from=backend-builder /src/target/release/ai-image-studio /usr/local/bin/ai-image-studio
COPY --from=frontend-builder /src/frontend/dist /app/static
USER 10001:10001
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/ai-image-studio"]
CMD ["serve"]
