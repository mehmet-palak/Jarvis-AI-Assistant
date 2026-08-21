# Kullanıcı Kabul / Release Kapısı Checklist'i (F9 madde 7)

Bir sürümün "yayınlanabilir" sayılması için aşağıdaki kriterlerin her biri **kanıtıyla** doğrulanmalı.
Bu checklist bir slogan değil: her satır, o kriteri kanıtlayan somut bir teste ya da komuta bağlı.
Yeni bir sürüm çıkarmadan önce `bash scripts/release_check.sh --offline` ve
`bash scripts/security_audit.sh` çalıştırılır; aşağıdaki elle-doğrulanabilir maddeler de gözden
geçirilir.

## Otomatik kapı (release_check.sh + security_audit.sh)

- [x] **Format / test / clippy / release build** — `scripts/release_check.sh` 7 adımlı birleşik rapor.
- [x] **Şema göç bütünlüğü** — `a_fresh_database_passes_schema_migration_integrity` (cargo test'in parçası).
- [x] **Audit zinciri witness** — `audit_export_writes_the_chain_and_rejects_a_tampered_one`.
- [x] **Bağımlılık güvenlik denetimi** — `scripts/security_audit.sh` (cargo audit, RustSec). Kabul edilen uyarılar [security_dependency_audit.md](security_dependency_audit.md)'de.

## Çevrimdışı çalışma

- [x] **Hiçbir çekirdek capability ağ gerektirmiyor** — `no_baseline_capability_requires_network_access` testi kanıtlıyor; release gate `--offline` modda tüm zinciri çevrimdışı koşuyor.

## Veri egemenliği (silme / dışa aktarma)

- [x] **Veri silme** — `/forget <id>`, `/forget all`, `/forget namespace <...>`; `/secret forget`.
- [x] **Veri dışa aktarma** — `/memory export`, `/dataset export`, `/audit-export`, `/profile export`.
- [x] **Sır egemenliği** — sırlar `secrets` tablosunda; `/secret show` dışında modele/belleğe hiç gitmez; `reject_secret_like_*` filtreleri her yazma yolunda.

## Model kontrolü

- [x] **Model kapatma** — `exit` komutu modeli RAM'den çıkarır (`/quit`/Ctrl+C yalnız arayüzü kapatır).
- [x] **Konfigürasyon ölçüm şeffaflığı** — model/prompt değişince açılışta + `/status`'ta "bu konfigürasyon ÖLÇÜLMEDİ" uyarısı (`configuration_is_measured`).

## Sürüm & geri alma

- [x] **Semantic version** — `JARVIS_VERSION` (`/status`), `CHANGELOG.md`.
- [x] **Sürüm uyumluluk güvencesi** — eski binary daha yeni şemalı DB'yi açmayı reddeder (`an_older_binary_refuses_to_open_a_newer_schema_database`).
- [x] **Doğrulanmış yedek + restore tatbikatı** — `/backup`; `a_verified_backup_actually_contains_the_data_and_can_be_reopened`.

## Operasyonel dayanıklılık

- [x] **Timeout/cancellation + süreç grubu** — `a_timed_out_worker_group_leaves_no_orphaned_child_processes`, `wait_with_timeout_kills_a_process_that_outlives_its_quota`.
- [x] **Stuck-task recovery** — açılışta RUNNING→INTERRUPTED (`recover_interrupted_tasks`, testli).
- [x] **Gizlilik-güvenli metrikler** — `/metrics` (latency + başarı oranı + policy dağılımı, içerik yok).

## Erişilebilirlik & UX (elle gözden geçirilir)

- [x] **Türkçe UX** — tüm arayüz metinleri Türkçe.
- [x] **Erişilebilirlik** — F5 erişilebilirlik boşlukları 20 Ağustos 2026'da kapatıldı (klavye kısayolları, ses seviyesi göstergesi, yazılı onay zorunluluğu).
- [ ] **Performans hedefleri (sürüme özel elle kontrol)** — `/eval` ile model kalite ölçümü + `/metrics` latency; bir sürüm öncesi hedef değerler kullanıcı tarafından gözden geçirilir (bu, her sürümde tekrarlanan elle bir adımdır, kalıcı bir `[x]` değil).

## Kullanım

```bash
bash scripts/release_check.sh --offline   # çevrimdışı kapı (format/test/clippy/build/smoke)
bash scripts/security_audit.sh            # bağımlılık güvenlik denetimi (internet erişir)
# + yukarıdaki elle maddeleri gözden geçir (özellikle performans hedefleri)
```
