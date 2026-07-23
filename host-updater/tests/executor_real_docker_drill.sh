#!/usr/bin/env bash
set -Eeuo pipefail

# Runs the fixed executor against real Docker, PostgreSQL, Redis, MinIO and a
# Cosign-signed image. The caller owns the registry and signed image so this
# test never pushes to a production registry.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
EXECUTOR_SCRIPT=${EXECUTOR_SCRIPT:-"$ROOT/host-updater/scripts/execute-update.sh"}
TARGET_IMAGE_TAG=${TARGET_IMAGE_TAG:-${SIGNED_IMAGE_TAG:-}}
TARGET_IMAGE_DIGEST=${TARGET_IMAGE_DIGEST:-${SIGNED_IMAGE_DIGEST:-}}
CURRENT_IMAGE_TAG=${CURRENT_IMAGE_TAG:-$TARGET_IMAGE_TAG}
CURRENT_IMAGE_DIGEST=${CURRENT_IMAGE_DIGEST:-$TARGET_IMAGE_DIGEST}
CURRENT_SCHEMA_VERSION=${CURRENT_SCHEMA_VERSION:-10}
DRILL_SCENARIO=${DRILL_SCENARIO:-success}
DRILL_TRIGGER=${DRILL_TRIGGER:-executor}
HOST_UPDATER_BIN=${HOST_UPDATER_BIN:-}
COSIGN_BIN=${COSIGN_BIN:-$(command -v cosign || true)}
COSIGN_PUBLIC_KEY=${COSIGN_PUBLIC_KEY:?COSIGN_PUBLIC_KEY is required}
PUBLIC_PORT=${PUBLIC_PORT:-3410}
CANDIDATE_PORT=${CANDIDATE_PORT:-3411}
MANIFEST_PORT=${MANIFEST_PORT:-3443}

[[ -x "$EXECUTOR_SCRIPT" ]] || { printf 'executor is not executable: %s\n' "$EXECUTOR_SCRIPT" >&2; exit 1; }
[[ -x "$COSIGN_BIN" ]] || { printf 'cosign is not executable: %s\n' "$COSIGN_BIN" >&2; exit 1; }
[[ -f "$COSIGN_PUBLIC_KEY" ]] || { printf 'Cosign public key does not exist: %s\n' "$COSIGN_PUBLIC_KEY" >&2; exit 1; }
[[ -n "$TARGET_IMAGE_TAG" ]] || { printf 'TARGET_IMAGE_TAG is required\n' >&2; exit 1; }
[[ "$TARGET_IMAGE_DIGEST" =~ ^sha256:[0-9a-fA-F]{64}$ ]] || { printf 'invalid target image digest\n' >&2; exit 1; }
[[ "$CURRENT_IMAGE_DIGEST" =~ ^sha256:[0-9a-fA-F]{64}$ ]] || { printf 'invalid current image digest\n' >&2; exit 1; }
[[ "$CURRENT_SCHEMA_VERSION" =~ ^[0-9]+$ ]] || { printf 'invalid current schema version\n' >&2; exit 1; }
case "$DRILL_SCENARIO" in
  success|digest_failure|signature_failure) FIXTURE_FAILURE_MODE="" ;;
  migration_failure) FIXTURE_FAILURE_MODE=migration ;;
  candidate_failure) FIXTURE_FAILURE_MODE=candidate ;;
  active_failure) FIXTURE_FAILURE_MODE=active ;;
  *)
    printf 'unsupported DRILL_SCENARIO: %s\n' "$DRILL_SCENARIO" >&2
    exit 1
    ;;
esac
[[ "$DRILL_TRIGGER" == executor || "$DRILL_TRIGGER" == web ]] \
  || { printf 'DRILL_TRIGGER must be executor or web\n' >&2; exit 1; }
if [[ "$DRILL_TRIGGER" == web ]]; then
  [[ "$DRILL_SCENARIO" == success ]] || { printf 'web trigger currently supports the success scenario only\n' >&2; exit 1; }
  [[ -x "$HOST_UPDATER_BIN" ]] || { printf 'HOST_UPDATER_BIN must be an executable Linux binary\n' >&2; exit 1; }
fi

for command_name in curl jq docker openssl sha256sum base64 realpath; do
  command -v "$command_name" >/dev/null || { printf '%s is required\n' "$command_name" >&2; exit 1; }
done
docker compose version >/dev/null

WORK_ROOT=$(mktemp -d -t aiis-real-updater-drill.XXXXXXXX)
DRILL_ID=$(cat /proc/sys/kernel/random/uuid)
JOB_ID=$DRILL_ID
PROJECT_NAME="aiis-real-${DRILL_ID%%-*}"
SOCKET_GID=$(id -g)
HTTPS_PID=""
HOST_UPDATER_PID=""
COMPOSE_READY=false

compose() {
  docker compose --project-name "$PROJECT_NAME" --project-directory "$WORK_ROOT/app" \
    --env-file "$WORK_ROOT/app/base.env" --env-file "$WORK_ROOT/app/app.env" \
    --env-file "$WORK_ROOT/app/release.env" --file "$WORK_ROOT/app/docker-compose.yml" "$@"
}

cleanup() {
  local exit_code=$?
  trap - EXIT
  set +e
  if [[ -n "$HTTPS_PID" ]]; then
    kill "$HTTPS_PID" >/dev/null 2>&1
    wait "$HTTPS_PID" >/dev/null 2>&1
  fi
  if [[ -n "$HOST_UPDATER_PID" ]]; then
    kill "$HOST_UPDATER_PID" >/dev/null 2>&1
    wait "$HOST_UPDATER_PID" >/dev/null 2>&1
  fi
  if [[ "$COMPOSE_READY" == true ]]; then
    compose down --volumes --remove-orphans >/dev/null 2>&1
  fi
  local resolved
  resolved=$(realpath -m -- "$WORK_ROOT")
  if [[ "$resolved" == /tmp/aiis-real-updater-drill.* && -d "$resolved" ]]; then
    rm -rf -- "$resolved"
  fi
  exit "$exit_code"
}
trap cleanup EXIT

install -d -m 700 \
  "$WORK_ROOT/app" "$WORK_ROOT/backups" \
  "$WORK_ROOT/state/releases" "$WORK_ROOT/tools" \
  "$WORK_ROOT/updater-service"
install -d -m 755 "$WORK_ROOT/app/images" "$WORK_ROOT/manifest"
install -d -m 770 "$WORK_ROOT/updater-socket"
ln -s "$COSIGN_BIN" "$WORK_ROOT/tools/cosign"
cp -- "$COSIGN_PUBLIC_KEY" "$WORK_ROOT/cosign.pub"

openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -keyout "$WORK_ROOT/manifest/ca.key" -out "$WORK_ROOT/manifest/ca.crt" \
  -subj '/CN=AI Image Studio real drill CA' \
  -addext 'basicConstraints=critical,CA:TRUE' \
  -addext 'keyUsage=critical,keyCertSign,cRLSign' >/dev/null 2>&1
openssl req -new -newkey rsa:2048 -nodes \
  -keyout "$WORK_ROOT/manifest/server.key" -out "$WORK_ROOT/manifest/server.csr" \
  -subj '/CN=manifest' >/dev/null 2>&1
cat >"$WORK_ROOT/manifest/server.ext" <<'EOF'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:manifest,DNS:localhost,IP:127.0.0.1
EOF
openssl x509 -req -days 1 -sha256 \
  -in "$WORK_ROOT/manifest/server.csr" \
  -CA "$WORK_ROOT/manifest/ca.crt" -CAkey "$WORK_ROOT/manifest/ca.key" -CAcreateserial \
  -extfile "$WORK_ROOT/manifest/server.ext" \
  -out "$WORK_ROOT/manifest/server.crt" >/dev/null 2>&1
cp -- "$WORK_ROOT/manifest/ca.crt" "$WORK_ROOT/app/images/drill-ca.crt"
cat >"$WORK_ROOT/manifest/nginx.conf" <<'EOF'
events {}
http {
  server {
    listen 3443 ssl;
    server_name manifest;
    ssl_certificate /srv/manifest/server.crt;
    ssl_certificate_key /srv/manifest/server.key;
    location / {
      root /srv/manifest;
    }
  }
}
EOF

cat >"$WORK_ROOT/app/base.env" <<EOF
COMPOSE_PROJECT_NAME=$PROJECT_NAME
POSTGRES_DB=studio_drill
POSTGRES_USER=studio_drill
POSTGRES_PASSWORD=studio_drill_password
MINIO_ROOT_USER=studio_minio
MINIO_ROOT_PASSWORD=studio_minio_password
SOCKET_GID=$SOCKET_GID
PUBLIC_PORT=$PUBLIC_PORT
MINIO_PORT=3490
EOF

cat >"$WORK_ROOT/app/app.env" <<'EOF'
APP_NAME=AI Image Studio Drill
APP_ENV=development
LISTEN_ADDR=0.0.0.0:3000
STATIC_DIR=/app/static
BOOTSTRAP_ADMIN_ENABLED=true
BOOTSTRAP_ADMIN_USERNAME=admin
BOOTSTRAP_ADMIN_PASSWORD=123456
BOOTSTRAP_ADMIN_FORCE_PASSWORD_CHANGE=true
SESSION_SECRET=real-drill-session-secret-at-least-32-characters
SESSION_COOKIE_SECURE=false
CREDENTIAL_MASTER_KEY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
DATABASE_URL=postgres://studio_drill:studio_drill_password@db:5432/studio_drill
DATABASE_MAX_CONNECTIONS=10
STORAGE_DRIVER=s3
STORAGE_LOCAL_PATH=/app/data/images
STORAGE_S3_ENABLED=true
STORAGE_S3_BUCKET=studio-images
STORAGE_S3_REGION=us-east-1
STORAGE_S3_ENDPOINT=http://minio:9000
STORAGE_S3_PREFIX=ai-image-studio/
STORAGE_S3_ACCESS_KEY_ID=studio_minio
STORAGE_S3_SECRET_ACCESS_KEY=studio_minio_password
STORAGE_S3_FORCE_PATH_STYLE=true
STORAGE_CONSISTENCY_SCAN_ENABLED=false
STORAGE_CONSISTENCY_SCAN_INTERVAL_SECONDS=86400
STORAGE_ORPHAN_GRACE_SECONDS=86400
TASK_EXECUTION_MODE=redis
REDIS_URL=redis://redis:6379/0
TASK_QUEUE_KEY=ai-image-studio:drill
TASK_MAX_RETRIES=0
RATE_LIMIT_ENABLED=false
KEEP_PREVIOUS_RELEASES=3
ALLOW_CUSTOM_BASE_URL=true
RUST_LOG=ai_image_studio=warn
EOF
printf 'HOST_UPDATER_DRILL_FAILURE=%s\n' "$FIXTURE_FAILURE_MODE" >>"$WORK_ROOT/app/app.env"
cat >>"$WORK_ROOT/app/app.env" <<EOF
HTTP_CA_CERT_FILE=/app/data/images/drill-ca.crt
UPDATE_MANIFEST_URL=https://manifest:3443/release-manifest.json
HOST_UPDATER_SOCKET=/run/ai-image-studio-updater/host-updater.sock
HOST_UPDATER_TOKEN=real-drill-host-updater-token-at-least-32-bytes
EOF

cat >"$WORK_ROOT/app/release.env" <<EOF
APP_IMAGE=$CURRENT_IMAGE_TAG@$CURRENT_IMAGE_DIGEST
APP_IMAGE_REFERENCE=$CURRENT_IMAGE_TAG@$CURRENT_IMAGE_DIGEST
APP_VERSION=0.1.0
APP_IMAGE_DIGEST=$CURRENT_IMAGE_DIGEST
APP_SCHEMA_VERSION=$CURRENT_SCHEMA_VERSION
EOF

cat >"$WORK_ROOT/app/docker-compose.yml" <<'EOF'
name: ${COMPOSE_PROJECT_NAME}

services:
  migrate:
    image: ${APP_IMAGE}
    command: ["migrate"]
    env_file: [./base.env, ./app.env, ./release.env]
    restart: "no"

  app:
    image: ${APP_IMAGE}
    command: ["serve"]
    user: "10001:${SOCKET_GID}"
    env_file: [./base.env, ./app.env, ./release.env]
    environment:
      HOST_UPDATER_DRILL_ACTIVE_RELEASE: "true"
    ports: ["127.0.0.1:${PUBLIC_PORT}:3000"]
    volumes:
      - ./images:/app/data/images
      - ../updater-socket:/run/ai-image-studio-updater:ro
    read_only: true
    tmpfs: ["/tmp:size=256m"]
    security_opt: ["no-new-privileges:true"]

  worker:
    image: ${APP_IMAGE}
    command: ["worker"]
    env_file: [./base.env, ./app.env, ./release.env]
    volumes: ["./images:/app/data/images"]
    read_only: true
    tmpfs: ["/tmp:size=256m"]
    security_opt: ["no-new-privileges:true"]

  db:
    image: postgres:17-alpine
    environment:
      POSTGRES_DB: ${POSTGRES_DB}
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes: ["db-data:/var/lib/postgresql/data"]

  redis:
    image: redis:8-alpine
    command: ["redis-server", "--appendonly", "yes"]
    volumes: ["redis-data:/data"]

  minio:
    image: minio/minio:RELEASE.2024-10-13T13-34-11Z
    command: ["server", "/data", "--console-address", ":9001"]
    environment:
      MINIO_ROOT_USER: ${MINIO_ROOT_USER}
      MINIO_ROOT_PASSWORD: ${MINIO_ROOT_PASSWORD}
    ports: ["127.0.0.1:${MINIO_PORT}:9000"]
    volumes: ["minio-data:/data"]

  manifest:
    image: nginx:alpine
    volumes:
      - ../manifest/nginx.conf:/etc/nginx/nginx.conf:ro
      - ../manifest:/srv/manifest:ro

volumes:
  db-data:
  redis-data:
  minio-data:
EOF

COMPOSE_READY=true
compose up --detach db redis minio manifest

for _ in $(seq 1 60); do
  if compose exec -T db pg_isready -U studio_drill -d studio_drill >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
compose exec -T db pg_isready -U studio_drill -d studio_drill >/dev/null

for _ in $(seq 1 60); do
  if curl --fail --silent --max-time 2 "http://127.0.0.1:3490/minio/health/ready" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl --fail --silent --max-time 2 "http://127.0.0.1:3490/minio/health/ready" >/dev/null

DB_CONTAINER=$(compose ps -q db)
COMPOSE_NETWORK=$(docker inspect --format '{{range $name, $_ := .NetworkSettings.Networks}}{{$name}}{{"\n"}}{{end}}' \
  "$DB_CONTAINER" | head -n1)
[[ -n "$COMPOSE_NETWORK" ]] || { printf 'Compose network was not created\n' >&2; exit 1; }

run_mc() {
  docker run --rm --network "$COMPOSE_NETWORK" \
    --env 'MC_HOST_drill=http://studio_minio:studio_minio_password@minio:9000' \
    minio/mc:RELEASE.2024-10-08T09-37-26Z "$@"
}

run_mc mb --ignore-existing drill/studio-images >/dev/null
run_mc mb --ignore-existing drill/studio-backups >/dev/null

compose run --rm --no-deps migrate >/dev/null
compose up --detach --no-deps app worker

for _ in $(seq 1 60); do
  if curl --fail --silent --max-time 2 "http://127.0.0.1:${PUBLIC_PORT}/api/v1/ready" \
    | jq -e '.status == "ready"' >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
if ! curl --fail --silent "http://127.0.0.1:${PUBLIC_PORT}/api/v1/ready" \
  | jq -e '.status == "ready"' >/dev/null; then
  printf 'initial application did not become ready\n' >&2
  compose ps >&2
  compose logs --no-color app worker >&2
  exit 1
fi

COOKIE_JAR="$WORK_ROOT/cookies.txt"
curl --fail --silent --show-error --cookie-jar "$COOKIE_JAR" \
  --header 'Content-Type: application/json' \
  --data '{"username":"admin","password":"123456"}' \
  "http://127.0.0.1:${PUBLIC_PORT}/api/v1/auth/login" \
  | jq -e '.mustChangePassword == true' >/dev/null

PASSWORD_STATUS=$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
  --cookie "$COOKIE_JAR" --cookie-jar "$COOKIE_JAR" \
  --header 'Content-Type: application/json' \
  --data '{"currentPassword":"123456","newPassword":"RealDrill123!"}' \
  "http://127.0.0.1:${PUBLIC_PORT}/api/v1/users/me/change-password")
[[ "$PASSWORD_STATUS" == 204 ]] || { printf 'password change failed with %s\n' "$PASSWORD_STATUS" >&2; exit 1; }

printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl2sAAAAASUVORK5CYII=' \
  | base64 --decode >"$WORK_ROOT/source.png"
SOURCE_SHA256=$(sha256sum "$WORK_ROOT/source.png" | awk '{print $1}')
curl --fail --silent --show-error --cookie "$COOKIE_JAR" \
  --form "file=@$WORK_ROOT/source.png;type=image/png" \
  "http://127.0.0.1:${PUBLIC_PORT}/api/v1/image-assets/uploads" >"$WORK_ROOT/upload.json"
ASSET_ID=$(jq -r '.id' "$WORK_ROOT/upload.json")
[[ "$ASSET_ID" =~ ^[0-9a-fA-F-]{36}$ ]] || { printf 'asset upload did not return an id\n' >&2; exit 1; }
ASSET_KEY=$(compose exec -T db psql -U studio_drill -d studio_drill -Atqc \
  "SELECT storage_key FROM image_assets WHERE id = '$ASSET_ID' AND storage_driver = 's3'")
[[ -n "$ASSET_KEY" ]] || { printf 'uploaded asset was not persisted to S3\n' >&2; exit 1; }
SCHEMA_BEFORE=$(compose exec -T db psql -U studio_drill -d studio_drill -Atqc \
  "SELECT COALESCE(MAX(version), 0)::BIGINT FROM _sqlx_migrations WHERE success")

run_mc mirror --overwrite drill/studio-images "drill/studio-backups/snapshots/$DRILL_ID" >/dev/null
printf 's3://studio-backups/snapshots/%s\n' "$DRILL_ID" >"$WORK_ROOT/s3-backup-reference.txt"
run_mc stat "drill/studio-backups/snapshots/$DRILL_ID/ai-image-studio/$ASSET_KEY" >/dev/null

cat >"$WORK_ROOT/manifest/release-manifest.json" <<EOF
{
  "version": "0.2.0",
  "image": "$TARGET_IMAGE_TAG",
  "image_digest": "$TARGET_IMAGE_DIGEST",
  "schema_target": 10,
  "schema_min_supported": 0,
  "schema_max_supported": 10,
  "rollback_compatible_to": "0.1.0",
  "requires_backup": true,
  "destructive_migration": false,
  "minimum_updater_version": "0.1.0",
  "release_notes": "real Docker isolation drill"
}
EOF

(
  cd "$WORK_ROOT/manifest"
  openssl s_server -accept "0.0.0.0:$MANIFEST_PORT" -cert server.crt -key server.key \
    -WWW -quiet >"$WORK_ROOT/https.log" 2>&1
) &
HTTPS_PID=$!
for _ in $(seq 1 30); do
  if CURL_CA_BUNDLE="$WORK_ROOT/manifest/ca.crt" \
    curl --fail --silent --max-time 2 "https://localhost:${MANIFEST_PORT}/release-manifest.json" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
CURL_CA_BUNDLE="$WORK_ROOT/manifest/ca.crt" \
  curl --fail --silent "https://localhost:${MANIFEST_PORT}/release-manifest.json" >/dev/null

cat >"$WORK_ROOT/executor.env" <<EOF
APP_DIR=$WORK_ROOT/app
COMPOSE_FILE=$WORK_ROOT/app/docker-compose.yml
BASE_ENV_FILE=$WORK_ROOT/app/base.env
APP_ENV_FILE=$WORK_ROOT/app/app.env
RELEASE_ENV_FILE=$WORK_ROOT/app/release.env
STATE_ROOT=$WORK_ROOT/state
BACKUP_ROOT=$WORK_ROOT/backups
LOCAL_STORAGE_PATH=$WORK_ROOT/app/images
STORAGE_DRIVER=s3
UPDATE_MANIFEST_URL=https://localhost:$MANIFEST_PORT/release-manifest.json
S3_BACKUP_REFERENCE_FILE=$WORK_ROOT/s3-backup-reference.txt
S3_BACKUP_MAX_AGE_SECONDS=300
COSIGN_PUBLIC_KEY=$WORK_ROOT/cosign.pub
INITIAL_APP_IMAGE=$CURRENT_IMAGE_TAG
INITIAL_APP_VERSION=0.1.0
INITIAL_APP_DIGEST=$CURRENT_IMAGE_DIGEST
INITIAL_SCHEMA_MIN_SUPPORTED=0
INITIAL_SCHEMA_MAX_SUPPORTED=10
POSTGRES_USER=studio_drill
POSTGRES_DB=studio_drill
PUBLIC_HEALTH_URL=http://127.0.0.1:$PUBLIC_PORT/api/v1/ready
CANDIDATE_PORT=$CANDIDATE_PORT
KEEP_PREVIOUS_RELEASES=3
MIN_FREE_BYTES=1048576
UPDATER_TOOL_DIR=$WORK_ROOT/tools
EOF

if [[ "$DRILL_TRIGGER" == executor ]]; then
  set +e
  CURL_CA_BUNDLE="$WORK_ROOT/manifest/ca.crt" \
    "$EXECUTOR_SCRIPT" --config "$WORK_ROOT/executor.env" --job-id "$JOB_ID" \
      --action upgrade --target-version 0.2.0 \
      2> >(tee "$WORK_ROOT/executor-error.log" >&2) | tee "$WORK_ROOT/executor-output.jsonl"
  EXECUTOR_STATUS=${PIPESTATUS[0]}
  set -e
else
  CURL_CA_BUNDLE="$WORK_ROOT/manifest/ca.crt" \
  HOST_UPDATER_UNIX_SOCKET="$WORK_ROOT/updater-socket/host-updater.sock" \
  HOST_UPDATER_SOCKET_GID="$SOCKET_GID" \
  HOST_UPDATER_TOKEN='real-drill-host-updater-token-at-least-32-bytes' \
  HOST_UPDATER_STATE_DIR="$WORK_ROOT/updater-service" \
  HOST_UPDATER_EXECUTOR_PATH="$EXECUTOR_SCRIPT" \
  HOST_UPDATER_EXECUTOR_CONFIG="$WORK_ROOT/executor.env" \
  HOST_UPDATER_JOB_TIMEOUT_SECONDS=600 \
  RUST_LOG=ai_image_studio_host_updater=warn \
    "$HOST_UPDATER_BIN" >"$WORK_ROOT/host-updater.log" 2>&1 &
  HOST_UPDATER_PID=$!
  for _ in $(seq 1 30); do
    if [[ -S "$WORK_ROOT/updater-socket/host-updater.sock" ]]; then
      break
    fi
    sleep 1
  done
  [[ -S "$WORK_ROOT/updater-socket/host-updater.sock" ]] \
    || { printf 'Host Updater Unix socket was not created\n' >&2; exit 1; }

  UPDATE_CHECK_STATUS=$(curl --silent --show-error --output "$WORK_ROOT/update-check.json" \
    --write-out '%{http_code}' --cookie "$COOKIE_JAR" --request POST \
    "http://127.0.0.1:${PUBLIC_PORT}/api/v1/admin/updates/check")
  if [[ "$UPDATE_CHECK_STATUS" != 200 ]]; then
    printf 'Web manifest check returned HTTP %s: ' "$UPDATE_CHECK_STATUS" >&2
    cat "$WORK_ROOT/update-check.json" >&2
    printf '\n' >&2
    compose logs --no-color app manifest >&2
    docker run --rm --network "$COMPOSE_NETWORK" \
      --volume "$WORK_ROOT/manifest/ca.crt:/usr/local/share/ca-certificates/drill.crt:ro" \
      nginx:alpine sh -c \
        'update-ca-certificates >/dev/null && wget -S -O- https://manifest:3443/release-manifest.json' \
      >&2 || true
    exit 1
  fi
  jq -e '.hasUpdate == true and .schemaCompatible == true and .manifest.version == "0.2.0"' \
    "$WORK_ROOT/update-check.json" >/dev/null

  curl --fail --silent --show-error --cookie "$COOKIE_JAR" \
    --header 'Content-Type: application/json' --header 'X-AI-Studio-Action: update' \
    --data '{"action":"upgrade","targetVersion":"0.2.0","currentPassword":"RealDrill123!","confirmDestructiveMigration":false}' \
    "http://127.0.0.1:${PUBLIC_PORT}/api/v1/admin/updates/jobs" \
    >"$WORK_ROOT/web-job-created.json"
  JOB_ID=$(jq -r '.id' "$WORK_ROOT/web-job-created.json")
  [[ "$JOB_ID" =~ ^[0-9a-fA-F-]{36}$ ]] || { printf 'Web update API did not return a job id\n' >&2; exit 1; }

  WEB_JOB_STATUS=""
  for _ in $(seq 1 180); do
    if curl --fail --silent --cookie "$COOKIE_JAR" \
      "http://127.0.0.1:${PUBLIC_PORT}/api/v1/admin/updates/jobs/${JOB_ID}" \
      >"$WORK_ROOT/web-job-final.json" 2>/dev/null; then
      WEB_JOB_STATUS=$(jq -r '.status' "$WORK_ROOT/web-job-final.json")
      if [[ "$WEB_JOB_STATUS" == succeeded || "$WEB_JOB_STATUS" == failed ]]; then
        break
      fi
    fi
    sleep 1
  done
  [[ "$WEB_JOB_STATUS" == succeeded ]] || {
    printf 'Web-triggered Host Updater job ended as %s\n' "${WEB_JOB_STATUS:-unavailable}" >&2
    if [[ -f "$WORK_ROOT/web-job-final.json" ]]; then
      cat "$WORK_ROOT/web-job-final.json" >&2
      printf '\n' >&2
    fi
    cat "$WORK_ROOT/host-updater.log" >&2
    compose logs --no-color app worker >&2
    exit 1
  }
  EXECUTOR_STATUS=0
fi

if [[ "$DRILL_SCENARIO" == success ]]; then
  [[ "$EXECUTOR_STATUS" == 0 ]] || { printf 'successful executor scenario returned %s\n' "$EXECUTOR_STATUS" >&2; exit 1; }
  if [[ "$DRILL_TRIGGER" == executor ]]; then
    jq -s -e --arg digest "$TARGET_IMAGE_DIGEST" '
      any(.[]; .type == "result" and .appVersion == "0.2.0" and
          .imageDigest == $digest and .schemaVersion == 10)
    ' "$WORK_ROOT/executor-output.jsonl" >/dev/null
  else
    jq -e '.status == "succeeded" and .progress == 100 and .currentStep == "completed"' \
      "$WORK_ROOT/web-job-final.json" >/dev/null
    WEB_DEPLOYMENT_COUNT=$(compose exec -T db psql -U studio_drill -d studio_drill -Atqc \
      "SELECT COUNT(*) FROM deployment_history WHERE source_job_id = '$JOB_ID' AND deployment_status = 'active'")
    [[ "$WEB_DEPLOYMENT_COUNT" == 1 ]] || { printf 'Web did not synchronize deployment evidence\n' >&2; exit 1; }
  fi
  grep -Fx "APP_VERSION=0.2.0" "$WORK_ROOT/app/release.env" >/dev/null
  grep -Fx "APP_IMAGE_DIGEST=$TARGET_IMAGE_DIGEST" "$WORK_ROOT/app/release.env" >/dev/null
  jq -e '.current == "0.2.0" and .releases[0].status == "active"' \
    "$WORK_ROOT/state/history.json" >/dev/null
else
  [[ "$EXECUTOR_STATUS" != 0 ]] || { printf 'failure scenario unexpectedly succeeded\n' >&2; exit 1; }
  jq -s -e 'any(.[]; .type == "progress" and .currentStep == "recovery") and
    all(.[]; .type != "result")' "$WORK_ROOT/executor-output.jsonl" >/dev/null
  grep -Fx "APP_VERSION=0.1.0" "$WORK_ROOT/app/release.env" >/dev/null
  grep -Fx "APP_IMAGE_DIGEST=$CURRENT_IMAGE_DIGEST" "$WORK_ROOT/app/release.env" >/dev/null
  [[ ! -e "$WORK_ROOT/state/history.json" ]] || { printf 'failed update wrote deployment history\n' >&2; exit 1; }
  SCHEMA_AFTER=$(compose exec -T db psql -U studio_drill -d studio_drill -Atqc \
    "SELECT COALESCE(MAX(version), 0)::BIGINT FROM _sqlx_migrations WHERE success")
  [[ "$SCHEMA_AFTER" == "$SCHEMA_BEFORE" ]] || { printf 'failed update changed the database schema\n' >&2; exit 1; }
  curl --fail --silent "http://127.0.0.1:${PUBLIC_PORT}/api/v1/ready" \
    | jq -e '.status == "ready"' >/dev/null
fi

BACKUP_MANIFEST="$WORK_ROOT/backups/job-$JOB_ID/backup-manifest.json"
jq -e --arg reference "s3://studio-backups/snapshots/$DRILL_ID" '
  .storage.driver == "s3" and .storage.reference == $reference and
  (.database.sha256 | test("^[0-9a-f]{64}$"))
' "$BACKUP_MANIFEST" >/dev/null
EXPECTED_DATABASE_SHA=$(jq -r '.database.sha256' "$BACKUP_MANIFEST")
ACTUAL_DATABASE_SHA=$(sha256sum "$WORK_ROOT/backups/job-$JOB_ID/database.dump" | awk '{print $1}')
[[ "$ACTUAL_DATABASE_SHA" == "$EXPECTED_DATABASE_SHA" ]] || { printf 'database backup checksum mismatch\n' >&2; exit 1; }

compose exec -T db createdb -U studio_drill restore_check
compose exec -T db pg_restore -U studio_drill -d restore_check --no-owner --no-privileges \
  <"$WORK_ROOT/backups/job-$JOB_ID/database.dump"
RESTORED_ASSET=$(compose exec -T db psql -U studio_drill -d restore_check -Atqc \
  "SELECT COUNT(*) FROM image_assets WHERE id = '$ASSET_ID'")
[[ "$RESTORED_ASSET" == 1 ]] || { printf 'database backup restore lost the uploaded asset\n' >&2; exit 1; }

run_mc cat "drill/studio-backups/snapshots/$DRILL_ID/ai-image-studio/$ASSET_KEY" \
  >"$WORK_ROOT/restored-from-s3.png"
RESTORED_S3_SHA256=$(sha256sum "$WORK_ROOT/restored-from-s3.png" | awk '{print $1}')
[[ "$RESTORED_S3_SHA256" == "$SOURCE_SHA256" ]] || { printf 'S3 backup copy checksum mismatch\n' >&2; exit 1; }

curl --fail --silent --show-error --cookie "$COOKIE_JAR" \
  "http://127.0.0.1:${PUBLIC_PORT}/api/v1/image-assets/${ASSET_ID}/content" \
  >"$WORK_ROOT/restored-from-app.png"
RESTORED_APP_SHA256=$(sha256sum "$WORK_ROOT/restored-from-app.png" | awk '{print $1}')
[[ "$RESTORED_APP_SHA256" == "$SOURCE_SHA256" ]] || { printf 'active release could not read the historical S3 image\n' >&2; exit 1; }

if [[ "$DRILL_SCENARIO" == success ]]; then
  if [[ "$DRILL_TRIGGER" == web ]]; then
    printf 'HOST_UPDATER_WEB_REAL_DOCKER_DRILL_OK unix_socket=1 hmac=1 web_job=1 deployment_sync=1 signed_digest=1 postgres_backup_restore=1 s3_backup_restore=1 migration=1 candidate_ready=1 active_ready=1 historical_s3_image=1\n'
  else
    printf 'HOST_UPDATER_REAL_DOCKER_DRILL_OK signed_digest=1 https_manifest=1 postgres_backup_restore=1 s3_backup_restore=1 migration=1 candidate_ready=1 active_ready=1 historical_s3_image=1\n'
  fi
else
  printf 'HOST_UPDATER_REAL_DOCKER_FAILURE_OK scenario=%s rejected=1 schema_unchanged=1 old_release_ready=1 postgres_backup_restore=1 s3_backup_restore=1 historical_s3_image=1\n' "$DRILL_SCENARIO"
fi
