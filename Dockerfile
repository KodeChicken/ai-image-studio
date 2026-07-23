# syntax=docker/dockerfile:1.7

FROM node:22-alpine AS frontend-builder
WORKDIR /src/frontend
RUN corepack enable
COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN --mount=type=cache,id=ai-image-studio-pnpm-store,target=/root/.local/share/pnpm/store,sharing=locked \
    pnpm install --frozen-lockfile --prefer-offline
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
RUN --mount=type=cache,id=ai-image-studio-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=ai-image-studio-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=ai-image-studio-cargo-target,target=/src/target,sharing=locked \
    cargo build --locked --release \
      --package ai-image-studio \
      --package ai-image-studio-host-updater \
    && install -D -m 0755 /src/target/release/ai-image-studio /out/ai-image-studio \
    && install -D -m 0755 /src/target/release/ai-image-studio-host-updater /out/ai-image-studio-host-updater

FROM backend-builder AS host-updater-test
RUN --mount=type=cache,id=ai-image-studio-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=ai-image-studio-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=ai-image-studio-cargo-target,target=/src/target,sharing=locked \
    cargo test --locked --release --package ai-image-studio-host-updater

FROM docker:28-cli AS docker-cli

FROM debian:bookworm-slim AS runtime
ARG APP_VERSION=0.1.3-dev
ARG APP_IMAGE_REFERENCE=ai-image-studio:local
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /app/data/images \
    && chown -R 10001:10001 /app
WORKDIR /app
COPY --from=backend-builder /out/ai-image-studio /usr/local/bin/ai-image-studio
COPY --from=frontend-builder /src/frontend/dist /app/static
ENV IMAGE_APP_VERSION=${APP_VERSION} \
    IMAGE_APP_REFERENCE=${APP_IMAGE_REFERENCE}
USER 10001:10001
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/ai-image-studio"]
CMD ["serve"]

FROM debian:bookworm-slim AS updater-runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      bash ca-certificates coreutils curl findutils gzip jq tar \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -m 0755 /usr/local/libexec/ai-image-studio /etc/ai-image-studio-updater \
    && install -d -m 0770 -o root -g 10001 /run/ai-image-studio-updater
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/docker-compose /usr/local/libexec/docker/cli-plugins/docker-compose
COPY --from=backend-builder /out/ai-image-studio-host-updater /usr/local/bin/ai-image-studio-host-updater
COPY host-updater/scripts/execute-update.sh /usr/local/libexec/ai-image-studio/execute-update.sh
COPY host-updater/scripts/docker-entrypoint.sh /usr/local/bin/host-updater-entrypoint
COPY host-updater/config/executor.compose.env /etc/ai-image-studio-updater/executor.env
RUN chmod 0755 \
      /usr/local/bin/ai-image-studio-host-updater \
      /usr/local/bin/host-updater-entrypoint \
      /usr/local/libexec/ai-image-studio/execute-update.sh \
    && chmod 0644 /etc/ai-image-studio-updater/executor.env
ENTRYPOINT ["/usr/local/bin/host-updater-entrypoint"]
CMD ["/usr/local/bin/ai-image-studio-host-updater"]

FROM runtime AS final
