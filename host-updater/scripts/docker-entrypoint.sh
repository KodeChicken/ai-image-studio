#!/bin/sh
set -eu

umask 077
token_file=${HOST_UPDATER_TOKEN_FILE:-/run/ai-image-studio-updater/token}
runtime_dir=$(dirname "$token_file")

mkdir -p "$runtime_dir"
chown root:10001 "$runtime_dir"
chmod 0770 "$runtime_dir"

if [ ! -s "$token_file" ]; then
  temporary="${token_file}.tmp.$$"
  head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$temporary"
  chown root:10001 "$temporary"
  chmod 0640 "$temporary"
  mv -f "$temporary" "$token_file"
fi

chown root:10001 "$token_file"
chmod 0640 "$token_file"
exec "$@"
