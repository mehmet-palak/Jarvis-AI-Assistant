#!/usr/bin/env bash
# Local-only release gate. No command in this script may resolve or download dependencies.

set -euo pipefail

mode="${1:---offline}"
if [[ "$mode" != "--offline" && "$mode" != "--with-service" ]]; then
    printf 'Kullanım: bash scripts/release_check.sh [--offline|--with-service]\n' >&2
    exit 2
fi

printf '%s\n' 'JARVIS local release gate başlıyor (Cargo offline).'
cargo fmt --check
cargo metadata --offline --locked --no-deps --format-version 1 >/dev/null
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo build --release --offline

if [[ "$mode" == "--with-service" ]]; then
    if ! command -v curl >/dev/null; then
        printf '%s\n' 'curl bulunamadı; loopback model health doğrulanamadı.' >&2
        exit 1
    fi
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:8088/health >/dev/null
    printf '%s\n' 'Loopback model health: PASS'
fi

printf '%s\n' 'JARVIS local release gate: PASS'
