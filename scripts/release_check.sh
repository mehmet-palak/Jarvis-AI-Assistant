#!/usr/bin/env bash
# Local-only release gate. No command in this script may resolve or download dependencies.

set -euo pipefail

mode="${1:---offline}"
if [[ "$mode" != "--offline" && "$mode" != "--with-service" && "$mode" != "--with-vision" ]]; then
    printf 'Kullanım: bash scripts/release_check.sh [--offline|--with-service|--with-vision]\n' >&2
    exit 2
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
printf '%s\n' "JARVIS local release gate başlıyor — sürüm ${version} (Cargo offline)."
# Her adım ayrı bir kapı; herhangi biri başarısız olursa set -e ile hemen durur ve raporun sonundaki
# PASS satırı hiç basılmaz. Adım adım "PASS" izleri, tek bir birleşik raporun satırlarıdır.
cargo fmt --check;                                          printf '%s\n' '  [1/7] format          : PASS'
cargo metadata --offline --locked --no-deps --format-version 1 >/dev/null; printf '%s\n' '  [2/7] bağımlılık kilidi: PASS'
cargo test --offline >/dev/null;                           printf '%s\n' '  [3/7] test (+migration bütünlüğü + audit witness): PASS'
cargo clippy --offline --all-targets -- -D warnings 2>/dev/null; printf '%s\n' '  [4/7] clippy          : PASS'
cargo build --release --offline >/dev/null;                printf '%s\n' '  [5/7] release build   : PASS'

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
printf '%s\n' '  [6/7] MCP policy smoke: PASS'

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

printf '%s\n' '  [7/7] loopback health : (yalnız --with-service/--with-vision)'
printf '%s\n' '------------------------------------------------------------'
printf '%s\n' "JARVIS local release gate: PASS (sürüm ${version})"
printf '%s\n' 'Not: bağımlılık güvenlik denetimi ayrı ve İNTERNET erişir: bash scripts/security_audit.sh'
