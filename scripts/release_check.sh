#!/usr/bin/env bash
# Local-only release gate. No command in this script may resolve or download dependencies.

set -euo pipefail

mode="${1:---offline}"
if [[ "$mode" != "--offline" && "$mode" != "--with-service" && "$mode" != "--with-vision" ]]; then
    printf 'Kullanım: bash scripts/release_check.sh [--offline|--with-service|--with-vision]\n' >&2
    exit 2
fi

printf '%s\n' 'JARVIS local release gate başlıyor (Cargo offline).'
cargo fmt --check
cargo metadata --offline --locked --no-deps --format-version 1 >/dev/null
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo build --release --offline

smoke_directory="$(mktemp -d /tmp/jarvis-release-smoke.XXXXXX)"
trap 'rm -rf -- "$smoke_directory"' EXIT
smoke_output="$(printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"jarvis.system.health","arguments":{"input":""},"request_id":"release-health"}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"jarvis.unknown","arguments":{"input":""},"request_id":"release-deny"}}' \
    | JARVIS_DB_PATH="$smoke_directory/jarvis.db" target/release/mcp_stdio)"
if ! grep -q '"capability":"system.health".*"verification":"Pass"' <<<"$smoke_output"; then
    printf '%s\n%s\n' 'MCP health smoke başarısız:' "$smoke_output" >&2
    exit 1
fi
if ! grep -q '"capability":"unknown"' <<<"$smoke_output"; then
    printf '%s\n%s\n' 'MCP unknown-tool deny smoke başarısız:' "$smoke_output" >&2
    exit 1
fi
printf '%s\n' 'MCP policy smoke: PASS'

if [[ "$mode" == "--with-service" || "$mode" == "--with-vision" ]]; then
    if ! command -v curl >/dev/null; then
        printf '%s\n' 'curl bulunamadı; loopback model health doğrulanamadı.' >&2
        exit 1
    fi
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:8088/health >/dev/null
    printf '%s\n' 'Loopback model health: PASS'
fi

if [[ "$mode" == "--with-vision" ]]; then
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:8089/health >/dev/null
    printf '%s\n' 'Loopback vision health: PASS'
fi

printf '%s\n' 'JARVIS local release gate: PASS'
