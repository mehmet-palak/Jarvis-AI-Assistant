#!/usr/bin/env bash
# F9 "Güvenlik bakım döngüsü": JARVIS'in kendi Rust bağımlılıklarını RustSec advisory veritabanına
# karşı denetler. `scripts/release_check.sh`'ten AYRIDIR çünkü bu adım İNTERNET erişir (advisory
# veritabanını çeker) — çevrimdışı release kapısının çevrimdışı garantisini bozmamak için ayrı.
#
# Kabul edilen (belgeli) uyarılar .cargo/audit.toml'da; gerekçeleri docs/security_dependency_audit.md'de.
# Yeni/beklenmedik bir açık çıkarsa bu script başarısız olur (exit != 0).

set -euo pipefail

if ! command -v cargo-audit >/dev/null 2>&1; then
    printf '%s\n' 'cargo-audit kurulu değil. Kurmak için: cargo install cargo-audit --locked' >&2
    exit 2
fi

printf '%s\n' 'JARVIS bağımlılık güvenlik denetimi (cargo audit, RustSec)...'
cargo audit
printf '%s\n' 'Bağımlılık güvenlik denetimi: PASS (yeni/kabul-edilmemiş açık yok).'
