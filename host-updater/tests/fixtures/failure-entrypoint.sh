#!/bin/sh
set -eu

case "${HOST_UPDATER_DRILL_FAILURE:-}" in
  migration)
    if [ "${1:-}" = migrate ]; then
      printf 'intentional migration failure for Host Updater drill\n' >&2
      exit 42
    fi
    ;;
  candidate)
    if [ "${1:-}" = serve ]; then
      printf 'intentional candidate failure for Host Updater drill\n' >&2
      exit 42
    fi
    ;;
  active)
    if [ "${1:-}" = serve ] && [ "${HOST_UPDATER_DRILL_ACTIVE_RELEASE:-false}" = true ]; then
      printf 'intentional active release failure for Host Updater drill\n' >&2
      exit 42
    fi
    ;;
  '')
    ;;
  *)
    printf 'unsupported Host Updater drill failure mode: %s\n' "$HOST_UPDATER_DRILL_FAILURE" >&2
    exit 64
    ;;
esac

exec /usr/local/bin/ai-image-studio "$@"
