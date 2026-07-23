#!/usr/bin/env bash
set -Eeuo pipefail

WORK_ROOT=$(mktemp -d)
cleanup() {
  [[ -n "$WORK_ROOT" && "$WORK_ROOT" == /tmp/* && -d "$WORK_ROOT" ]] && rm -rf -- "$WORK_ROOT"
}
trap cleanup EXIT

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
FAKE_BIN="$WORK_ROOT/fake-bin"
APP_DIR="$WORK_ROOT/app"
STATE_ROOT="$WORK_ROOT/state"
BACKUP_ROOT="$STATE_ROOT/backups"
IMAGE_ROOT="$APP_DIR/data/images"
mkdir -p "$FAKE_BIN" "$IMAGE_ROOT" "$APP_DIR" "$STATE_ROOT/releases" "$BACKUP_ROOT"

OLD_DIGEST="sha256:$(printf '1%.0s' {1..64})"
NEW_DIGEST="sha256:$(printf '2%.0s' {1..64})"
printf 'historical-image' >"$IMAGE_ROOT/history.bin"
printf 'services: {}\n' >"$APP_DIR/docker-compose.yml"
printf 'BASE=1\n' >"$APP_DIR/.env.example"
printf 'APP=1\n' >"$APP_DIR/.env"
cat >"$STATE_ROOT-release.env" <<EOF
APP_IMAGE=ghcr.io/example/ai-image-studio:v0.1.0@$OLD_DIGEST
APP_IMAGE_REFERENCE=ghcr.io/example/ai-image-studio:v0.1.0@$OLD_DIGEST
APP_VERSION=0.1.0
APP_IMAGE_DIGEST=$OLD_DIGEST
APP_SCHEMA_VERSION=9
EOF

cat >"$STATE_ROOT/history.json" <<EOF
{
  "current": "0.1.0",
  "releases": [
    {"version":"0.1.0","image_reference":"ghcr.io/example/ai-image-studio:v0.1.0","image_digest":"$OLD_DIGEST","schema_version":9,"schema_min_supported":1,"schema_max_supported":9,"backup_reference":null,"status":"active","deployed_at":"2026-01-04T00:00:00Z"},
    {"version":"0.0.9","image_reference":"ghcr.io/example/ai-image-studio:v0.0.9","image_digest":"$OLD_DIGEST","schema_version":8,"schema_min_supported":1,"schema_max_supported":9,"backup_reference":null,"status":"rollback_available","deployed_at":"2026-01-03T00:00:00Z"},
    {"version":"0.0.8","image_reference":"ghcr.io/example/ai-image-studio:v0.0.8","image_digest":"$OLD_DIGEST","schema_version":7,"schema_min_supported":1,"schema_max_supported":8,"backup_reference":null,"status":"rollback_available","deployed_at":"2026-01-02T00:00:00Z"},
    {"version":"0.0.7","image_reference":"ghcr.io/example/ai-image-studio:v0.0.7","image_digest":"$OLD_DIGEST","schema_version":6,"schema_min_supported":1,"schema_max_supported":7,"backup_reference":null,"status":"rollback_available","deployed_at":"2026-01-01T00:00:00Z"}
  ]
}
EOF

OLD_BACKUP_IDS=(
  00000000-0000-4000-8000-000000000001
  00000000-0000-4000-8000-000000000002
  00000000-0000-4000-8000-000000000003
  00000000-0000-4000-8000-000000000004
)
for index in "${!OLD_BACKUP_IDS[@]}"; do
  directory="$BACKUP_ROOT/job-${OLD_BACKUP_IDS[$index]}"
  mkdir -p "$directory"
  touch -d "2026-01-0$((index + 1)) 00:00:00 UTC" "$directory"
done

FAKE_MANIFEST="$WORK_ROOT/release-manifest.json"
cat >"$FAKE_MANIFEST" <<EOF
{
  "version": "0.2.0",
  "image": "ghcr.io/example/ai-image-studio:v0.2.0",
  "image_digest": "$NEW_DIGEST",
  "schema_target": 10,
  "schema_min_supported": 9,
  "schema_max_supported": 12,
  "rollback_compatible_to": "0.1.0",
  "requires_backup": true,
  "destructive_migration": false,
  "minimum_updater_version": "0.1.0"
}
EOF

FAKE_DOCKER_LOG="$WORK_ROOT/docker.log"
FAKE_MIGRATION_MARKER="$WORK_ROOT/migration-complete"
FAKE_APP_DATA_DOCKER_PATH="$APP_DIR/data"
export FAKE_DOCKER_LOG FAKE_MIGRATION_MARKER FAKE_MANIFEST FAKE_APP_DATA_DOCKER_PATH OLD_DIGEST

cat >"$FAKE_BIN/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$FAKE_DOCKER_LOG"
joined=" $* "
case "$joined" in
  *" compose version "*) exit 0 ;;
  *" ps --filter "*"com.docker.compose.service=app"*) printf 'fake-app-container\n'; exit 0 ;;
  *" inspect --format "*" fake-app-container "*) printf '%s\n' "$FAKE_APP_DATA_DOCKER_PATH"; exit 0 ;;
  *" image inspect --format "*" ai-image-studio:active "*)
    if [[ "$joined" == *".Config.Env"* ]]; then
      printf 'IMAGE_APP_VERSION=0.1.0\n'
    else
      printf '%s\n' "$OLD_DIGEST"
    fi
    exit 0
    ;;
  *" pg_isready "*) exit 0 ;;
  *" psql "*) [[ -f "$FAKE_MIGRATION_MARKER" ]] && printf '10\n' || printf '9\n'; exit 0 ;;
  *" pg_dump "*) printf 'fake-postgresql-custom-dump'; exit 0 ;;
  *" run --rm --no-deps migrate "*) touch "$FAKE_MIGRATION_MARKER"; exit 0 ;;
  *" ps -q db "*) printf 'fake-db-container\n'; exit 0 ;;
  *" inspect --format "*" fake-db-container "*) printf 'fake-compose-network\n'; exit 0 ;;
  *" run --detach --rm --name ai-image-studio-candidate-"*) printf 'fake-candidate-container\n'; exit 0 ;;
  *" pull "*|*" image inspect "*|*" tag "*|*" stop app worker "*|*" up --detach --no-deps app worker "*|*" rm --force "*) exit 0 ;;
esac
printf 'unexpected fake docker command: %s\n' "$*" >&2
exit 1
EOF

cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=""
while (($#)); do
  if [[ "$1" == --output ]]; then
    output=$2
    shift 2
  else
    shift
  fi
done
if [[ -n "$output" ]]; then
  cp "$FAKE_MANIFEST" "$output"
else
  printf '{"status":"ready"}\n'
fi
EOF

chmod 0755 "$FAKE_BIN/docker" "$FAKE_BIN/curl"

CONFIG_FILE="$WORK_ROOT/executor.env"
cat >"$CONFIG_FILE" <<EOF
UPDATER_TOOL_DIR=$FAKE_BIN
APP_DIR=$APP_DIR
COMPOSE_FILE=$APP_DIR/docker-compose.yml
BASE_ENV_FILE=$APP_DIR/.env.example
APP_ENV_FILE=$APP_DIR/.env
RELEASE_ENV_FILE=$STATE_ROOT-release.env
STATE_ROOT=$STATE_ROOT
BACKUP_ROOT=$BACKUP_ROOT
LOCAL_STORAGE_PATH=$IMAGE_ROOT
LOCAL_STORAGE_DOCKER_PATH=auto
STORAGE_DRIVER=local
COMPOSE_PROJECT_NAME=ai-image-studio
ACTIVE_APP_IMAGE=ai-image-studio:active
USE_ACTIVE_IMAGE_ALIAS=true
CANDIDATE_USE_NETWORK_DNS=true
UPDATE_MANIFEST_URL=https://example.invalid/release-manifest.json
INITIAL_APP_IMAGE=ghcr.io/example/ai-image-studio:v0.1.0
INITIAL_APP_VERSION=auto
INITIAL_APP_DIGEST=auto
INITIAL_SCHEMA_MIN_SUPPORTED=1
INITIAL_SCHEMA_MAX_SUPPORTED=9
POSTGRES_USER=ai_image_studio
POSTGRES_DB=ai_image_studio
PUBLIC_HEALTH_URL=http://127.0.0.1:3100/api/v1/ready
CANDIDATE_PORT=3198
KEEP_PREVIOUS_RELEASES=3
MIN_FREE_BYTES=0
EOF

JOB_ID=22222222-2222-4222-8222-222222222222
OUTPUT=$(bash "$REPO_ROOT/host-updater/scripts/execute-update.sh" \
  --config "$CONFIG_FILE" --job-id "$JOB_ID" --action upgrade --target-version 0.2.0 \
  2>"$WORK_ROOT/executor.stderr")

jq -e --arg digest "$NEW_DIGEST" '
  .current == "0.2.0" and (.releases | length) == 4 and
  .releases[0].status == "active" and .releases[0].schema_version == 10 and
  .releases[0].schema_min_supported == 9 and .releases[0].schema_max_supported == 12 and
  .releases[0].image_digest == $digest and
  .releases[1].version == "0.1.0" and .releases[1].status == "rollback_available" and
  .releases[1].schema_min_supported == 1 and .releases[1].schema_max_supported == 9
' "$STATE_ROOT/history.json" >/dev/null
jq -e '.schemaVersion == 9 and .storage.driver == "local"' \
  "$BACKUP_ROOT/job-$JOB_ID/backup-manifest.json" >/dev/null
jq -e '.version == "0.2.0" and .schema_target == 10' \
  "$STATE_ROOT/releases/0.2.0.json" >/dev/null
grep -q '^APP_VERSION=0.2.0$' "$STATE_ROOT-release.env"
grep -q '^APP_SCHEMA_VERSION=10$' "$STATE_ROOT-release.env"
grep -q "^APP_IMAGE_DIGEST=$NEW_DIGEST$" "$STATE_ROOT-release.env"
grep -q '^AI_IMAGE_STUDIO_RUNTIME_IMAGE=ai-image-studio:active$' "$STATE_ROOT-release.env"
[[ "$(cat "$IMAGE_ROOT/history.bin")" == historical-image ]]
[[ -s "$BACKUP_ROOT/job-$JOB_ID/database.dump" ]]
[[ -s "$BACKUP_ROOT/job-$JOB_ID/images.tar.gz" ]]
[[ $(find "$BACKUP_ROOT" -mindepth 1 -maxdepth 1 -type d -name 'job-*' | wc -l) -eq 4 ]]
[[ ! -e "$BACKUP_ROOT/job-${OLD_BACKUP_IDS[0]}" ]]
grep -q 'run --rm --no-deps migrate' "$FAKE_DOCKER_LOG"
grep -q 'run --detach --rm --name ai-image-studio-candidate-' "$FAKE_DOCKER_LOG"
grep -q -- "--volume $FAKE_APP_DATA_DOCKER_PATH/images:/app/data/images" "$FAKE_DOCKER_LOG"
grep -q "tag ghcr.io/example/ai-image-studio:v0.2.0@$NEW_DIGEST ai-image-studio:active" "$FAKE_DOCKER_LOG"
if grep -q -- '--publish' "$FAKE_DOCKER_LOG"; then
  printf 'container-mode candidate unexpectedly published a host port\n' >&2
  exit 1
fi
grep -q 'up --detach --no-deps app worker' "$FAKE_DOCKER_LOG"
jq -e '.type == "result" and .appVersion == "0.2.0" and .schemaVersion == 10' \
  <<<"$(tail -n1 <<<"$OUTPUT")" >/dev/null

ROLLBACK_JOB_ID=33333333-3333-4333-8333-333333333333
set +e
bash "$REPO_ROOT/host-updater/scripts/execute-update.sh" \
  --config "$CONFIG_FILE" --job-id "$ROLLBACK_JOB_ID" --action rollback --target-version 0.1.0 \
  >"$WORK_ROOT/rollback.stdout" 2>"$WORK_ROOT/rollback.stderr"
ROLLBACK_STATUS=$?
set -e
[[ "$ROLLBACK_STATUS" -ne 0 ]]
grep -q 'current schema is outside the retained release compatibility window' \
  "$WORK_ROOT/rollback.stderr"

printf 'HOST_UPDATER_SUCCESS_DRILL_OK migration=1 candidate_ready=1 active_switch=1 history=1 backup_retention=1 rollback_window=1\n'
