# Bağımlılık güvenlik denetimi (F9 "Güvenlik bakım döngüsü")

JARVIS'in kendi Rust bağımlılıkları `cargo audit` (RustSec advisory veritabanı) ile denetleniyor.
Bu, bir güvenlik aracının kendi tedarik zincirinde bilinen açıklar taşımamasını sağlar.

## Nasıl çalıştırılır

```bash
bash scripts/security_audit.sh
```

(cargo-audit gerekir: `cargo install cargo-audit --locked`. Bu adım İNTERNET erişir — RustSec
advisory veritabanını çeker — bu yüzden çevrimdışı `scripts/release_check.sh` kapısından AYRIDIR.)

## İlk denetim (21 Ağustos 2026)

İlk denetim 6 uyarı buldu: 2 yüksek-önemli açık + 4 bakım/güvenlik uyarısı. `cargo update` ile
semver-uyumlu güncellemeler yapıldı (tüm testler + offline release build hâlâ geçiyor), ama
kalan uyarılar transitif bağımlılıkların major sürüm pinlemeleri nedeniyle otomatik çözülemedi.

Her biri aşağıda gerekçesiyle **bilinçli olarak kabul edildi** ve `.cargo/audit.toml`'un `ignore`
listesine eklendi. Bu sessiz bir bastırma değildir: liste sayesinde `cargo audit` YENİ/beklenmedik
bir açıkta hâlâ başarısız olur (gerçek bir kapı), ama incelenip düşük-maruziyet olarak kabul edilen
bu transitif uyarılarda geçer.

| Advisory | Crate | Nereden | Karar / gerekçe |
|---|---|---|---|
| RUSTSEC-2026-0194, RUSTSEC-2026-0195 | quick-xml 0.30.0 | zbus_xml ← atspi ← eframe (masaüstü erişilebilirlik) | **Kabul (düşük maruziyet).** DoS, kötü niyetli XML ayrıştırmayla tetiklenir; buradaki XML yerel D-Bus introspection verisidir, saldırgan kontrolünde değil. Erişilebilirlik yığını quick-xml 0.41'e taşındığında temizlenecek. |
| RUSTSEC-2024-0436 | paste 1.0.15 | ratatui (TUI) | **Kabul.** Bakımı bırakılmış bir proc-macro; çalışma zamanı saldırı yüzeyi yok. |
| RUSTSEC-2026-0192 | ttf-parser 0.25.1 | pdf-extract (lopdf) + eframe (font) | **Kabul.** Güvenlik açığı değil, bakım uyarısı. |
| RUSTSEC-2026-0002, RUSTSEC-2026-0253 | lru 0.12.5 | ratatui'nin iç layout önbelleği | **Kabul.** `unsound` uyarıları; lru doğrudan kullanılmıyor, ratatui güncellemesi bekleniyor. |

## Yeniden değerlendirme kuralı

Bir bağımlılık güncellemesi bu ID'lerden birini çözerse (ör. eframe/atspi major bump quick-xml'i
0.41'e taşırsa), hem `.cargo/audit.toml`'dan hem bu tablodan ilgili satır çıkarılmalı — böylece
kapı o açığı tekrar zorlamaya başlar. Kabul listesi minimumda tutulmalı; her giriş bir borçtur.
