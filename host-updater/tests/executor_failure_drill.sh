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
mkdir -p "$FAKE_BIN" "$IMAGE_ROOT" "$APP_DIR"

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
cp "$STATE_ROOT-release.env" "$WORK_ROOT/expected-release.env"

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
FAKE_RESTORE_MARKER="$WORK_ROOT/database-restored"
export FAKE_DOCKER_LOG FAKE_RESTORE_MARKER FAKE_MANIFEST

cat >"$FAKE_BIN/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$FAKE_DOCKER_LOG"
joined=" $* "
case "$joined" in
  *" compose version "*) exit 0 ;;
  *" pg_isready "*) exit 0 ;;
  *" psql "*) printf '9\n'; exit 0 ;;
  *" pg_dump "*) printf 'fake-postgresql-custom-dump'; exit 0 ;;
  *" run --rm --no-deps migrate "*) exit 42 ;;
  *" pg_restore "*) cat >/dev/null; touch "$FAKE_RESTORE_MARKER"; exit 0 ;;
  *" pull "*|*" image inspect "*|*" stop app worker "*|*" up --detach --no-deps app worker "*) exit 0 ;;
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
STORAGE_DRIVER=local
UPDATE_MANIFEST_URL=https://example.invalid/release-manifest.json
INITIAL_APP_IMAGE=ghcr.io/example/ai-image-studio:v0.1.0
INITIAL_APP_VERSION=0.1.0
INITIAL_APP_DIGEST=$OLD_DIGEST
INITIAL_SCHEMA_MIN_SUPPORTED=1
INITIAL_SCHEMA_MAX_SUPPORTED=9
POSTGRES_USER=ai_image_studio
POSTGRES_DB=ai_image_studio
PUBLIC_HEALTH_URL=http://127.0.0.1:3100/api/v1/ready
CANDIDATE_PORT=3198
KEEP_PREVIOUS_RELEASES=3
MIN_FREE_BYTES=0
EOF

JOB_ID=11111111-1111-4111-8111-111111111111
set +e
OUTPUT=$(bash "$REPO_ROOT/host-updater/scripts/execute-update.sh" \
  --config "$CONFIG_FILE" --job-id "$JOB_ID" --action upgrade --target-version 0.2.0 \
  2>"$WORK_ROOT/executor.stderr")
STATUS=$?
set -e

[[ "$STATUS" -ne 0 ]] || { printf 'executor unexpectedly succeeded\n' >&2; exit 1; }
[[ -f "$FAKE_RESTORE_MARKER" ]] || { printf 'database restore was not attempted\n' >&2; exit 1; }
cmp "$STATE_ROOT-release.env" "$WORK_ROOT/expected-release.env"
[[ "$(cat "$IMAGE_ROOT/history.bin")" == historical-image ]]
grep -q 'run --rm --no-deps migrate' "$FAKE_DOCKER_LOG"
grep -q 'pg_restore' "$FAKE_DOCKER_LOG"
grep -q 'up --detach --no-deps app worker' "$FAKE_DOCKER_LOG"
grep -q '"currentStep":"recovery"' <<<"$OUTPUT"
if grep -q '"type":"result"' <<<"$OUTPUT"; then
  printf 'failed executor emitted a success result\n' >&2
  exit 1
fi
jq -e '.schemaVersion == 9 and .storage.driver == "local"' \
  "$BACKUP_ROOT/job-$JOB_ID/backup-manifest.json" >/dev/null

printf 'HOST_UPDATER_FAILURE_DRILL_OK migration_failure=1 database_restore=1 image_restore=1 old_release_restart=1\n'
