#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
exec 3>&1

CONFIG_FILE=""
JOB_ID=""
ACTION=""
TARGET_VERSION=""
CONFIRM_DESTRUCTIVE=false
LAST_PROGRESS=0
APP_STOPPED=false
BACKUP_COMPLETE=false
MIGRATION_STARTED=false
CANDIDATE_NAME=""
BACKUP_DIR=""
PREVIOUS_ACTIVE_IMAGE_ID=""
APP_DATA_DOCKER_PATH=""

fail() {
  printf 'Host Updater: %s\n' "$*" >&2
  return 1
}

while (($#)); do
  case "$1" in
    --config) CONFIG_FILE=${2:-}; shift 2 ;;
    --job-id) JOB_ID=${2:-}; shift 2 ;;
    --action) ACTION=${2:-}; shift 2 ;;
    --target-version) TARGET_VERSION=${2:-}; shift 2 ;;
    --confirm-destructive-migration) CONFIRM_DESTRUCTIVE=true; shift ;;
    *) fail "unsupported argument: $1" ;;
  esac
done

[[ -n "$CONFIG_FILE" && -f "$CONFIG_FILE" ]] || fail "--config must name an existing file"
[[ "$CONFIG_FILE" = /* ]] || fail "--config must be absolute"
[[ "$JOB_ID" =~ ^[0-9a-fA-F-]{36}$ ]] || fail "invalid job id"
[[ "$ACTION" == upgrade || "$ACTION" == rollback ]] || fail "invalid action"
[[ "$TARGET_VERSION" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] || fail "invalid target version"

# The file is a fixed, root-owned deployment configuration, not request data.
# shellcheck source=/dev/null
source "$CONFIG_FILE"

if [[ -n "${UPDATER_TOOL_DIR:-}" ]]; then
  [[ "$UPDATER_TOOL_DIR" = /* && -d "$UPDATER_TOOL_DIR" ]] \
    || fail "UPDATER_TOOL_DIR must be an existing absolute directory"
  PATH="$(realpath -m -- "$UPDATER_TOOL_DIR"):$PATH"
  export PATH
fi

COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-ai-image-studio}
ACTIVE_APP_IMAGE=${ACTIVE_APP_IMAGE:-${INITIAL_APP_IMAGE:-ai-image-studio:active}}
USE_ACTIVE_IMAGE_ALIAS=${USE_ACTIVE_IMAGE_ALIAS:-false}
LOCAL_STORAGE_DOCKER_PATH=${LOCAL_STORAGE_DOCKER_PATH:-${LOCAL_STORAGE_PATH:-}}

required=(
  APP_DIR COMPOSE_FILE BASE_ENV_FILE APP_ENV_FILE RELEASE_ENV_FILE STATE_ROOT
  BACKUP_ROOT LOCAL_STORAGE_PATH LOCAL_STORAGE_DOCKER_PATH STORAGE_DRIVER UPDATE_MANIFEST_URL
  COMPOSE_PROJECT_NAME ACTIVE_APP_IMAGE
  INITIAL_APP_IMAGE INITIAL_APP_VERSION INITIAL_APP_DIGEST
  INITIAL_SCHEMA_MIN_SUPPORTED INITIAL_SCHEMA_MAX_SUPPORTED
  POSTGRES_USER POSTGRES_DB PUBLIC_HEALTH_URL CANDIDATE_PORT KEEP_PREVIOUS_RELEASES
  MIN_FREE_BYTES
)
for name in "${required[@]}"; do
  [[ -n "${!name:-}" ]] || fail "$name is required"
done

for command_name in curl jq docker sha256sum tar find stat df realpath; do
  command -v "$command_name" >/dev/null || fail "$command_name is required"
done
docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 is required"

safe_absolute_path() {
  local value=$1
  [[ "$value" = /* ]] || fail "path must be absolute: $value"
  value=$(realpath -m -- "$value")
  [[ "$value" != / ]] || fail "root directory is not an allowed target"
  printf '%s' "$value"
}

APP_DIR=$(safe_absolute_path "$APP_DIR")
COMPOSE_FILE=$(safe_absolute_path "$COMPOSE_FILE")
BASE_ENV_FILE=$(safe_absolute_path "$BASE_ENV_FILE")
APP_ENV_FILE=$(safe_absolute_path "$APP_ENV_FILE")
RELEASE_ENV_FILE=$(safe_absolute_path "$RELEASE_ENV_FILE")
STATE_ROOT=$(safe_absolute_path "$STATE_ROOT")
BACKUP_ROOT=$(safe_absolute_path "$BACKUP_ROOT")
LOCAL_STORAGE_PATH=$(safe_absolute_path "$LOCAL_STORAGE_PATH")

[[ -d "$APP_DIR" && -f "$COMPOSE_FILE" && -f "$BASE_ENV_FILE" ]] \
  || fail "application directory, Compose file, and base environment file must exist"
[[ "$STORAGE_DRIVER" == local || "$STORAGE_DRIVER" == s3 ]] || fail "STORAGE_DRIVER must be local or s3"
[[ "$USE_ACTIVE_IMAGE_ALIAS" == true || "$USE_ACTIVE_IMAGE_ALIAS" == false ]] \
  || fail "USE_ACTIVE_IMAGE_ALIAS must be true or false"
[[ "$UPDATE_MANIFEST_URL" == https://* ]] || fail "UPDATE_MANIFEST_URL must use HTTPS"
CANDIDATE_USE_NETWORK_DNS=${CANDIDATE_USE_NETWORK_DNS:-false}
[[ "$CANDIDATE_USE_NETWORK_DNS" == true || "$CANDIDATE_USE_NETWORK_DNS" == false ]] \
  || fail "CANDIDATE_USE_NETWORK_DNS must be true or false"
if [[ "$CANDIDATE_USE_NETWORK_DNS" == false ]]; then
  [[ "$CANDIDATE_PORT" =~ ^[0-9]+$ && "$CANDIDATE_PORT" -ge 1024 && "$CANDIDATE_PORT" -le 65535 ]] \
    || fail "CANDIDATE_PORT must be between 1024 and 65535"
fi
[[ "$KEEP_PREVIOUS_RELEASES" =~ ^[1-3]$ ]] || fail "KEEP_PREVIOUS_RELEASES must be between 1 and 3"
[[ "$MIN_FREE_BYTES" =~ ^[0-9]+$ ]] || fail "MIN_FREE_BYTES must be an integer"
[[ "$INITIAL_APP_VERSION" == auto || "$INITIAL_APP_VERSION" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] \
  || fail "INITIAL_APP_VERSION is invalid"
[[ "$INITIAL_APP_DIGEST" == auto || "$INITIAL_APP_DIGEST" =~ ^sha256:[0-9a-fA-F]{64}$ ]] \
  || fail "INITIAL_APP_DIGEST is invalid"
[[ "$INITIAL_SCHEMA_MIN_SUPPORTED" =~ ^[0-9]+$ && "$INITIAL_SCHEMA_MAX_SUPPORTED" =~ ^[0-9]+$ ]] \
  || fail "initial schema compatibility bounds must be non-negative integers"

install -d -m 700 -- "$STATE_ROOT" "$STATE_ROOT/releases" "$BACKUP_ROOT"
[[ -d "$LOCAL_STORAGE_PATH" ]] || fail "LOCAL_STORAGE_PATH must be an existing directory"

if [[ "$LOCAL_STORAGE_DOCKER_PATH" == auto ]]; then
  APP_CONTAINER=$(docker ps \
    --filter "label=com.docker.compose.project=$COMPOSE_PROJECT_NAME" \
    --filter 'label=com.docker.compose.service=app' \
    --format '{{.ID}}' | head -n1)
  [[ -n "$APP_CONTAINER" ]] || fail "running Compose app container was not found"
  APP_DATA_DOCKER_PATH=$(docker inspect --format \
    '{{range .Mounts}}{{if eq .Destination "/app/data"}}{{.Source}}{{end}}{{end}}' \
    "$APP_CONTAINER")
  APP_DATA_DOCKER_PATH=$(safe_absolute_path "$APP_DATA_DOCKER_PATH")
  LOCAL_STORAGE_DOCKER_PATH="$APP_DATA_DOCKER_PATH/images"
else
  LOCAL_STORAGE_DOCKER_PATH=$(safe_absolute_path "$LOCAL_STORAGE_DOCKER_PATH")
  APP_DATA_DOCKER_PATH=$(safe_absolute_path "${APP_DATA_DOCKER_PATH:-$(dirname "$LOCAL_STORAGE_DOCKER_PATH")}")
fi
if [[ "$USE_ACTIVE_IMAGE_ALIAS" == true ]]; then
  PREVIOUS_ACTIVE_IMAGE_ID=$(docker image inspect --format '{{.Id}}' "$ACTIVE_APP_IMAGE")
  [[ "$PREVIOUS_ACTIVE_IMAGE_ID" =~ ^sha256:[0-9a-fA-F]{64}$ ]] \
    || fail "active application image could not be resolved"
  if [[ "$INITIAL_APP_VERSION" == auto ]]; then
    INITIAL_APP_VERSION=$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' \
      "$ACTIVE_APP_IMAGE" | awk -F= '$1 == "IMAGE_APP_VERSION" {print substr($0, index($0, "=") + 1); exit}')
    [[ "$INITIAL_APP_VERSION" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] \
      || fail "active application image does not contain a valid IMAGE_APP_VERSION"
  fi
fi

HISTORY_FILE="$STATE_ROOT/history.json"
CANDIDATE_NAME="ai-image-studio-candidate-${JOB_ID}"
BACKUP_DIR="$BACKUP_ROOT/job-${JOB_ID}"
[[ ! -e "$BACKUP_DIR" ]] || fail "backup directory already exists"
install -d -m 700 -- "$BACKUP_DIR"

emit_progress() {
  local progress=$1
  local step=$2
  LAST_PROGRESS=$progress
  jq -nc --argjson progress "$progress" --arg currentStep "$step" \
    '{type:"progress", progress:$progress, currentStep:$currentStep}' >&3
}

write_release_env() {
  local release_image=$1
  local version=$2
  local digest=$3
  local schema=$4
  local temporary="${RELEASE_ENV_FILE}.tmp.${JOB_ID}"
  local image_reference="${release_image%@*}@${digest}"
  local runtime_image="$image_reference"
  if [[ "$USE_ACTIVE_IMAGE_ALIAS" == true ]]; then
    runtime_image="$ACTIVE_APP_IMAGE"
  fi
  printf 'AI_IMAGE_STUDIO_RUNTIME_IMAGE=%s\nAPP_IMAGE=%s\nAPP_IMAGE_REFERENCE=%s\nAPP_RELEASE_IMAGE=%s\nAPP_VERSION=%s\nAPP_IMAGE_DIGEST=%s\nAPP_SCHEMA_VERSION=%s\n' \
    "$runtime_image" "$runtime_image" "$image_reference" "$release_image" "$version" "$digest" "$schema" \
    >"$temporary"
  chmod 600 "$temporary"
  mv -f -- "$temporary" "$RELEASE_ENV_FILE"
}

if [[ ! -f "$RELEASE_ENV_FILE" ]]; then
  if [[ "$INITIAL_APP_DIGEST" == auto ]]; then
    INITIAL_APP_DIGEST="$PREVIOUS_ACTIVE_IMAGE_ID"
  fi
  write_release_env "$INITIAL_APP_IMAGE" "$INITIAL_APP_VERSION" "$INITIAL_APP_DIGEST" "0"
fi
cp -- "$RELEASE_ENV_FILE" "$BACKUP_DIR/previous-release.env"

# shellcheck source=/dev/null
source "$RELEASE_ENV_FILE"
: "${APP_IMAGE:?APP_IMAGE missing from release environment}"
CURRENT_RELEASE_IMAGE=${APP_RELEASE_IMAGE:-${APP_IMAGE_REFERENCE:?APP_IMAGE_REFERENCE missing from release environment}}
CURRENT_VERSION=${APP_VERSION:?APP_VERSION missing from release environment}
CURRENT_DIGEST=${APP_IMAGE_DIGEST:?APP_IMAGE_DIGEST missing from release environment}
CURRENT_MIN_SCHEMA=$INITIAL_SCHEMA_MIN_SUPPORTED
CURRENT_MAX_SCHEMA=$INITIAL_SCHEMA_MAX_SUPPORTED
if [[ -f "$HISTORY_FILE" ]] && jq -e --arg version "$CURRENT_VERSION" \
  '.releases[] | select(.version == $version)' "$HISTORY_FILE" >/dev/null; then
  CURRENT_MIN_SCHEMA=$(jq -r --arg version "$CURRENT_VERSION" \
    '.releases[] | select(.version == $version) | .schema_min_supported' "$HISTORY_FILE" | head -n1)
  CURRENT_MAX_SCHEMA=$(jq -r --arg version "$CURRENT_VERSION" \
    '.releases[] | select(.version == $version) | .schema_max_supported' "$HISTORY_FILE" | head -n1)
fi
[[ "$CURRENT_MIN_SCHEMA" =~ ^[0-9]+$ && "$CURRENT_MAX_SCHEMA" =~ ^[0-9]+$ ]] \
  || fail "current release lacks schema compatibility evidence"

compose() {
  local env_args=(--env-file "$BASE_ENV_FILE")
  if [[ -f "$APP_ENV_FILE" ]]; then
    env_args+=(--env-file "$APP_ENV_FILE")
  fi
  env_args+=(--env-file "$RELEASE_ENV_FILE")
  AI_IMAGE_STUDIO_DATA_DIR="$APP_DATA_DOCKER_PATH" \
    docker compose --project-name "$COMPOSE_PROJECT_NAME" --project-directory "$APP_DIR" \
      "${env_args[@]}" --file "$COMPOSE_FILE" "$@"
}

compose_target() {
  local image=$1
  shift
  local env_args=(--env-file "$BASE_ENV_FILE")
  if [[ -f "$APP_ENV_FILE" ]]; then
    env_args+=(--env-file "$APP_ENV_FILE")
  fi
  env_args+=(--env-file "$RELEASE_ENV_FILE")
  AI_IMAGE_STUDIO_RUNTIME_IMAGE="$image" APP_IMAGE="$image" \
    APP_IMAGE_REFERENCE="$image" APP_VERSION="$TARGET_VERSION" \
    AI_IMAGE_STUDIO_DATA_DIR="$APP_DATA_DOCKER_PATH" \
    docker compose --project-name "$COMPOSE_PROJECT_NAME" --project-directory "$APP_DIR" \
      "${env_args[@]}" --file "$COMPOSE_FILE" "$@"
}

database_schema() {
  compose exec -T db psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atqc \
    "SELECT COALESCE(MAX(version), 0)::BIGINT FROM _sqlx_migrations WHERE success"
}

wait_ready() {
  local url=$1
  local attempts=${2:-60}
  local index
  for ((index = 1; index <= attempts; index++)); do
    if curl --fail --silent --max-time 3 "$url" 2>/dev/null | jq -e '.status == "ready"' >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  return 1
}

restore_after_failure() {
  local original_exit=$?
  trap - ERR
  set +e
  printf 'Host Updater failed; starting automatic recovery for job %s\n' "$JOB_ID" >&2
  if ((LAST_PROGRESS < 95)); then
    emit_progress 95 recovery
  fi
  if [[ -n "$CANDIDATE_NAME" ]]; then
    docker rm --force "$CANDIDATE_NAME" >/dev/null 2>&1
  fi
  if [[ "$APP_STOPPED" == true ]]; then
    compose stop app worker >&2
  fi
  cp -- "$BACKUP_DIR/previous-release.env" "$RELEASE_ENV_FILE"
  if [[ "$USE_ACTIVE_IMAGE_ALIAS" == true && -n "$PREVIOUS_ACTIVE_IMAGE_ID" ]]; then
    docker tag "$PREVIOUS_ACTIVE_IMAGE_ID" "$ACTIVE_APP_IMAGE" >&2
  fi
  if [[ "$MIGRATION_STARTED" == true && "$BACKUP_COMPLETE" == true ]]; then
    compose exec -T db pg_restore -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
      --clean --if-exists --no-owner --no-privileges <"$BACKUP_DIR/database.dump" >&2
    if [[ "$STORAGE_DRIVER" == local && -f "$BACKUP_DIR/images.tar.gz" ]]; then
      find "$LOCAL_STORAGE_PATH" -mindepth 1 -depth -delete
      tar --extract --gzip --file "$BACKUP_DIR/images.tar.gz" --directory "$LOCAL_STORAGE_PATH"
    fi
  fi
  if [[ "$APP_STOPPED" == true ]]; then
    compose up --detach --no-deps app worker >&2
    wait_ready "$PUBLIC_HEALTH_URL" 60 || printf 'automatic recovery health check failed\n' >&2
  fi
  exit "$original_exit"
}
trap restore_after_failure ERR

emit_progress 3 validating_environment
compose exec -T db pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB" >/dev/null
CURRENT_SCHEMA=$(database_schema)
[[ "$CURRENT_SCHEMA" =~ ^[0-9]+$ ]] || fail "could not read the current schema version"

TARGET_IMAGE_TAG=""
TARGET_DIGEST=""
TARGET_IMAGE=""
TARGET_SCHEMA="$CURRENT_SCHEMA"
MANIFEST_FILE="$BACKUP_DIR/release-manifest.json"

if [[ "$ACTION" == upgrade ]]; then
  CURL_AUTH_ARGS=()
  if [[ -n "${UPDATE_MANIFEST_TOKEN:-}" ]]; then
    CURL_AUTH_ARGS=(--header "Authorization: Bearer $UPDATE_MANIFEST_TOKEN")
  fi
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    "${CURL_AUTH_ARGS[@]}" --max-filesize 1048576 --output "$MANIFEST_FILE" \
    "$UPDATE_MANIFEST_URL"
  [[ $(stat -c %s "$MANIFEST_FILE") -le 1048576 ]] || fail "release manifest exceeds 1 MiB"
  jq -e '
    (.version | type == "string") and
    (.image | type == "string") and
    (.image_digest | test("^sha256:[0-9a-fA-F]{64}$")) and
    (.schema_target | type == "number" and . >= 0) and
    (.schema_min_supported | type == "number" and . >= 0) and
    (.schema_max_supported | type == "number") and
    (.schema_max_supported >= .schema_target) and
    (.minimum_updater_version | type == "string")
  ' "$MANIFEST_FILE" >/dev/null || fail "release manifest validation failed"
  MANIFEST_VERSION=$(jq -r '.version' "$MANIFEST_FILE")
  [[ "${MANIFEST_VERSION#v}" == "${TARGET_VERSION#v}" ]] || fail "release manifest version does not match the request"
  TARGET_IMAGE_TAG=$(jq -r '.image' "$MANIFEST_FILE")
  TARGET_DIGEST=$(jq -r '.image_digest | ascii_downcase' "$MANIFEST_FILE")
  TARGET_SCHEMA=$(jq -r '.schema_target' "$MANIFEST_FILE")
  MIN_SCHEMA=$(jq -r '.schema_min_supported' "$MANIFEST_FILE")
  MAX_SCHEMA=$(jq -r '.schema_max_supported' "$MANIFEST_FILE")
  MIN_UPDATER=$(jq -r '.minimum_updater_version' "$MANIFEST_FILE")
  DESTRUCTIVE_MIGRATION=$(jq -r '.destructive_migration // false' "$MANIFEST_FILE")
  [[ "$CURRENT_SCHEMA" -ge "$MIN_SCHEMA" ]] || fail "current schema is below schema_min_supported"
  [[ "$TARGET_SCHEMA" -ge "$CURRENT_SCHEMA" ]] || fail "upgrade manifest attempts to lower the schema"
  [[ "$CURRENT_SCHEMA" -le "$MAX_SCHEMA" ]] || fail "current schema exceeds schema_max_supported"
  [[ "$DESTRUCTIVE_MIGRATION" != true || "$CONFIRM_DESTRUCTIVE" == true ]] \
    || fail "destructive migration was not explicitly confirmed"
  if [[ "$(printf '%s\n%s\n' "$MIN_UPDATER" "0.1.0" | sort -V | head -n1)" != "$MIN_UPDATER" ]]; then
    fail "release requires a newer Host Updater"
  fi
else
  [[ -f "$HISTORY_FILE" ]] || fail "no Host Updater deployment history exists"
  jq -e --arg version "$TARGET_VERSION" '.releases[] | select(.version == $version)' \
    "$HISTORY_FILE" >/dev/null || fail "rollback target is not retained"
  TARGET_IMAGE_TAG=$(jq -r --arg version "$TARGET_VERSION" \
    '.releases[] | select(.version == $version) | .image_reference' "$HISTORY_FILE" | head -n1)
  TARGET_DIGEST=$(jq -r --arg version "$TARGET_VERSION" \
    '.releases[] | select(.version == $version) | .image_digest' "$HISTORY_FILE" | head -n1)
  TARGET_SCHEMA=$(jq -r --arg version "$TARGET_VERSION" \
    '.releases[] | select(.version == $version) | .schema_version' "$HISTORY_FILE" | head -n1)
  MIN_SCHEMA=$(jq -r --arg version "$TARGET_VERSION" \
    '.releases[] | select(.version == $version) | .schema_min_supported' "$HISTORY_FILE" | head -n1)
  MAX_SCHEMA=$(jq -r --arg version "$TARGET_VERSION" \
    '.releases[] | select(.version == $version) | .schema_max_supported' "$HISTORY_FILE" | head -n1)
  [[ "$MIN_SCHEMA" =~ ^[0-9]+$ && "$MAX_SCHEMA" =~ ^[0-9]+$ ]] \
    || fail "rollback target lacks schema compatibility evidence"
  [[ "$CURRENT_SCHEMA" -ge "$MIN_SCHEMA" && "$CURRENT_SCHEMA" -le "$MAX_SCHEMA" ]] \
    || fail "current schema is outside the retained release compatibility window"
fi

[[ "$TARGET_IMAGE_TAG" =~ ^[A-Za-z0-9._/:@-]+$ ]] || fail "target image reference is invalid"
[[ "$TARGET_DIGEST" =~ ^sha256:[0-9a-fA-F]{64}$ ]] || fail "target image digest is invalid"
TARGET_NEEDS_PULL=true
if [[ "$USE_ACTIVE_IMAGE_ALIAS" == true && "$TARGET_IMAGE_TAG" == "$ACTIVE_APP_IMAGE" ]] \
  && docker image inspect "$TARGET_DIGEST" >/dev/null 2>&1; then
  TARGET_IMAGE="$TARGET_DIGEST"
  TARGET_NEEDS_PULL=false
else
  TARGET_IMAGE="${TARGET_IMAGE_TAG%@*}@${TARGET_DIGEST}"
fi

AVAILABLE_KB=$(df -Pk "$BACKUP_ROOT" | awk 'NR == 2 {print $4}')
[[ "$AVAILABLE_KB" =~ ^[0-9]+$ ]] || fail "could not determine free disk space"
((AVAILABLE_KB * 1024 >= MIN_FREE_BYTES)) || fail "insufficient free disk space for a safe update"

emit_progress 10 pulling_image
if [[ "$TARGET_NEEDS_PULL" == true ]]; then
  docker pull "$TARGET_IMAGE" >&2
fi
docker image inspect "$TARGET_IMAGE" >/dev/null

emit_progress 40 entering_maintenance
compose stop app worker >&2
APP_STOPPED=true

emit_progress 48 backing_up_database
compose exec -T db pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
  --format=custom --no-owner --no-privileges >"$BACKUP_DIR/database.dump"
[[ -s "$BACKUP_DIR/database.dump" ]] || fail "database backup is empty"

STORAGE_BACKUP_REFERENCE=""
if [[ "$STORAGE_DRIVER" == local ]]; then
  emit_progress 55 backing_up_local_images
  : >"$BACKUP_DIR/images-manifest.jsonl"
  while IFS= read -r -d '' path; do
    relative=${path#"$LOCAL_STORAGE_PATH"/}
    jq -nc --arg key "$relative" --arg sha256 "$(sha256sum "$path" | awk '{print $1}')" \
      --argjson size "$(stat -c %s "$path")" '{key:$key,size:$size,sha256:$sha256}' \
      >>"$BACKUP_DIR/images-manifest.jsonl"
  done < <(find "$LOCAL_STORAGE_PATH" -type f -print0)
  jq -s '.' "$BACKUP_DIR/images-manifest.jsonl" >"$BACKUP_DIR/images-manifest.json"
  tar --create --gzip --file "$BACKUP_DIR/images.tar.gz" --directory "$LOCAL_STORAGE_PATH" .
  STORAGE_BACKUP_REFERENCE="$BACKUP_DIR/images.tar.gz"
else
  S3_BACKUP_REFERENCE_FILE=${S3_BACKUP_REFERENCE_FILE:-}
  S3_BACKUP_MAX_AGE_SECONDS=${S3_BACKUP_MAX_AGE_SECONDS:-86400}
  [[ -n "$S3_BACKUP_REFERENCE_FILE" ]] || fail "S3_BACKUP_REFERENCE_FILE is required for S3 storage"
  S3_BACKUP_REFERENCE_FILE=$(safe_absolute_path "$S3_BACKUP_REFERENCE_FILE")
  [[ -s "$S3_BACKUP_REFERENCE_FILE" ]] || fail "S3 backup reference is missing"
  BACKUP_AGE=$(( $(date +%s) - $(stat -c %Y "$S3_BACKUP_REFERENCE_FILE") ))
  ((BACKUP_AGE <= S3_BACKUP_MAX_AGE_SECONDS)) || fail "S3 backup evidence is too old"
  STORAGE_BACKUP_REFERENCE=$(tr -d '\r\n' <"$S3_BACKUP_REFERENCE_FILE")
  [[ -n "$STORAGE_BACKUP_REFERENCE" ]] || fail "S3 backup reference is empty"
fi

DATABASE_SHA256=$(sha256sum "$BACKUP_DIR/database.dump" | awk '{print $1}')
jq -n \
  --arg jobId "$JOB_ID" --arg action "$ACTION" --arg fromVersion "$CURRENT_VERSION" \
  --arg targetVersion "$TARGET_VERSION" --arg databaseFile "$BACKUP_DIR/database.dump" \
  --arg databaseSha256 "$DATABASE_SHA256" --arg storageDriver "$STORAGE_DRIVER" \
  --arg storageReference "$STORAGE_BACKUP_REFERENCE" --argjson schemaVersion "$CURRENT_SCHEMA" \
  --arg createdAt "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{jobId:$jobId,action:$action,fromVersion:$fromVersion,targetVersion:$targetVersion,
    schemaVersion:$schemaVersion,database:{file:$databaseFile,sha256:$databaseSha256},
    storage:{driver:$storageDriver,reference:$storageReference},createdAt:$createdAt}' \
  >"$BACKUP_DIR/backup-manifest.json"
BACKUP_COMPLETE=true

if [[ "$ACTION" == upgrade ]]; then
  emit_progress 65 running_migrations
  MIGRATION_STARTED=true
  compose_target "$TARGET_IMAGE" run --rm --no-deps migrate >&2
  MIGRATED_SCHEMA=$(database_schema)
  [[ "$MIGRATED_SCHEMA" == "$TARGET_SCHEMA" ]] || fail "database schema does not match manifest schema_target"
fi

emit_progress 75 starting_candidate
DB_CONTAINER=$(compose ps -q db)
[[ -n "$DB_CONTAINER" ]] || fail "database container is not running"
COMPOSE_NETWORK=$(docker inspect --format '{{range $name, $_ := .NetworkSettings.Networks}}{{$name}}{{"\n"}}{{end}}' \
  "$DB_CONTAINER" | head -n1)
[[ -n "$COMPOSE_NETWORK" ]] || fail "could not determine the Compose network"
candidate_env_args=(--env-file "$BASE_ENV_FILE")
if [[ -f "$APP_ENV_FILE" ]]; then
  candidate_env_args+=(--env-file "$APP_ENV_FILE")
fi
candidate_publish_args=()
if [[ "$CANDIDATE_USE_NETWORK_DNS" == true ]]; then
  CANDIDATE_HEALTH_URL="http://${CANDIDATE_NAME}:3000/api/v1/ready"
else
  candidate_publish_args=(--publish "127.0.0.1:${CANDIDATE_PORT}:3000")
  CANDIDATE_HEALTH_URL="http://127.0.0.1:${CANDIDATE_PORT}/api/v1/ready"
fi
docker run --detach --rm --name "$CANDIDATE_NAME" --network "$COMPOSE_NETWORK" \
  "${candidate_env_args[@]}" \
  --env "APP_VERSION=$TARGET_VERSION" --env "APP_IMAGE_REFERENCE=$TARGET_IMAGE" \
  --env 'LISTEN_ADDR=0.0.0.0:3000' \
  "${candidate_publish_args[@]}" \
  --volume "$LOCAL_STORAGE_DOCKER_PATH:/app/data/images" --read-only --tmpfs /tmp:size=256m \
  --security-opt no-new-privileges:true "$TARGET_IMAGE" serve >&2
wait_ready "$CANDIDATE_HEALTH_URL" 60 || fail "candidate readiness check failed"
docker rm --force "$CANDIDATE_NAME" >/dev/null
CANDIDATE_NAME=""

emit_progress 85 switching_release
FINAL_SCHEMA=$(database_schema)
if [[ "$USE_ACTIVE_IMAGE_ALIAS" == true ]]; then
  docker tag "$TARGET_IMAGE" "$ACTIVE_APP_IMAGE" >&2
fi
write_release_env "$TARGET_IMAGE_TAG" "$TARGET_VERSION" "$TARGET_DIGEST" "$FINAL_SCHEMA"
compose up --detach --no-deps app worker >&2
wait_ready "$PUBLIC_HEALTH_URL" 60 || fail "active release readiness check failed"

emit_progress 92 recording_deployment
cp -- "$MANIFEST_FILE" "$STATE_ROOT/releases/${TARGET_VERSION}.json" 2>/dev/null || true
if [[ -f "$HISTORY_FILE" ]]; then
  EXISTING_HISTORY=$(cat "$HISTORY_FILE")
else
  EXISTING_HISTORY='{"current":null,"releases":[]}'
fi
jq -n \
  --argjson existing "$EXISTING_HISTORY" --arg targetVersion "$TARGET_VERSION" \
  --arg targetImage "$TARGET_IMAGE_TAG" --arg targetDigest "$TARGET_DIGEST" \
  --argjson targetSchema "$FINAL_SCHEMA" --argjson targetMinSchema "$MIN_SCHEMA" \
  --argjson targetMaxSchema "$MAX_SCHEMA" --arg targetBackup "$BACKUP_DIR/backup-manifest.json" \
  --arg currentVersion "$CURRENT_VERSION" --arg currentImage "${CURRENT_RELEASE_IMAGE%@*}" \
  --arg currentDigest "$CURRENT_DIGEST" --argjson currentSchema "$CURRENT_SCHEMA" \
  --argjson currentMinSchema "$CURRENT_MIN_SCHEMA" \
  --argjson currentMaxSchema "$CURRENT_MAX_SCHEMA" \
  --argjson keep "$((KEEP_PREVIOUS_RELEASES + 1))" --arg now "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '
    def add_unique($items): reduce $items[] as $item ([];
      if any(.[]; .version == $item.version) then . else . + [$item] end);
    {current:$targetVersion,releases:(add_unique(
      [{version:$targetVersion,image_reference:$targetImage,image_digest:$targetDigest,
        schema_version:$targetSchema,schema_min_supported:$targetMinSchema,
        schema_max_supported:$targetMaxSchema,backup_reference:$targetBackup,status:"active",deployed_at:$now}]
      + (if $currentVersion == $targetVersion then [] else
          [{version:$currentVersion,image_reference:$currentImage,image_digest:$currentDigest,
            schema_version:$currentSchema,schema_min_supported:$currentMinSchema,
            schema_max_supported:$currentMaxSchema,backup_reference:null,status:"rollback_available",deployed_at:$now}]
        end)
      + [$existing.releases[] | select(.version != $targetVersion and .version != $currentVersion)
          | .status="rollback_available"]
    )[:$keep])}
  ' >"$HISTORY_FILE.tmp"
mv -f -- "$HISTORY_FILE.tmp" "$HISTORY_FILE"

mapfile -t OLD_BACKUPS < <(
  find "$BACKUP_ROOT" -mindepth 1 -maxdepth 1 -type d -name 'job-*' -printf '%T@ %f\n' \
    | sort -rn | awk 'NR > '"$((KEEP_PREVIOUS_RELEASES + 1))"' {print $2}'
)
for directory in "${OLD_BACKUPS[@]}"; do
  [[ "$directory" =~ ^job-[0-9a-fA-F-]{36}$ ]] || continue
  find "$BACKUP_ROOT/$directory" -depth -delete
done

trap - ERR
emit_progress 99 finalizing
jq -nc --arg appVersion "$TARGET_VERSION" --arg imageReference "$TARGET_IMAGE_TAG" \
  --arg imageDigest "$TARGET_DIGEST" --argjson schemaVersion "$FINAL_SCHEMA" \
  --arg backupReference "$BACKUP_DIR/backup-manifest.json" \
  '{type:"result",appVersion:$appVersion,imageReference:$imageReference,
    imageDigest:$imageDigest,schemaVersion:$schemaVersion,backupReference:$backupReference}' >&3
