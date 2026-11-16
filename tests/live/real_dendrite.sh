#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
workspace=$(CDPATH= cd -- "$root/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/tellus-live.XXXXXX")
pid=
cleanup() { if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi; rm -rf "$tmp"; }
trap cleanup EXIT INT TERM
(cd "$workspace/dendrite" && DENDRITE_GROUNDING_PORT_FILE="$tmp/port" MIX_ENV=test mix run scripts/grounding_fixture.exs) >"$tmp/out" 2>"$tmp/err" &
pid=$!
i=0
while [ ! -s "$tmp/port" ]; do i=$((i+1)); if [ "$i" -ge 200 ]; then cat "$tmp/err" >&2; exit 1; fi; sleep 0.05; done
endpoint="127.0.0.1:$(cat "$tmp/port")"
output=$(cargo run --quiet --manifest-path "$root/Cargo.toml" -p tellus-cli -- headless "$endpoint")
printf '%s\n' "$output" | grep '"complete": true' >/dev/null
printf '%s\n' "$output" | grep '"capability": "grounding.observe.v1"' >/dev/null
printf '%s\n' "$output" | grep '"load_id": "fixture/sample"' >/dev/null

