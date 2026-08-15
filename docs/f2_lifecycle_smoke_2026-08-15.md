# F2 model lifecycle smoke — ilk açılış / yeniden açılış / DB recovery — 15 Ağustos 2026

Bu koşum, [14 Ağustos 2026 kaydında](f2_lifecycle_smoke_2026-08-14.md) kapatılan `exit`/`/quit`/
`Ctrl+C`/`SIGHUP` RAM-koruma yollarına ek olarak, o kayıtta açık kalan üç maddeyi gerçek release
binary'siyle kapatır: ilk açılış (DB yok), yeniden açılış (DB var) ve DB recovery (bozuk audit
zinciri).

Gerçek kullanıcı `jarvis.db` dosyası test öncesi `sha256sum` ile doğrulandı, kenara alındı ve test
bitince aynı checksum ile eksiksiz geri yüklendi. Testler boyunca kullanıcı verisine hiçbir zaman
yazılmadı; tüm işlemler geçici, atılan bir DB üzerinde yapıldı.

## 1. İlk açılış (DB yok)

- `jarvis.db` geçici olarak kenara alındı.
- `target/release/jarvis`, pseudo-terminal (`script`) içinde başlatıldı, `/quit` gönderildi.
- Sonuç: `jarvis.db` sıfırdan oluşturuldu (82 KiB), beklenen tüm tablolar (`tasks`, `approvals`,
  `audit_events`, `teacher_examples`, `memories`, `workspace_documents`, `workspace_chunks*`,
  `schema_migrations`) mevcuttu. Süreç temiz çıktı (`COMMAND_EXIT_CODE=0`), artakalan süreç yoktu.

## 2. Yeniden açılış (DB var)

- Aynı taze `jarvis.db` üzerinde `target/release/jarvis` ikinci kez başlatıldı, `/quit` gönderildi.
- Sonuç: uygulama mevcut şemayı sorunsuz açtı, migration'lar `INSERT OR IGNORE` ile no-op geçti,
  temiz çıktı ve artakalan süreç yoktu.

## 3. DB recovery (eşzamanlı audit yarışı)

`repair_concurrent_audit_chain`, her `SqliteStore::open()` çağrısında otomatik çalışır ve yalnız
bilinen yarış şeklini (yinelenen `event_sequence`) onarır. Bunu gerçek release binary'siyle
tetiklemek için:

1. Taze DB'ye elle iki satır eklendi, ikisi de `event_sequence=1` (14 Ağustosta düzeltilen çoklu
   süreç yarışının simülasyonu).
2. `target/release/jarvis` üçüncü kez başlatıldı, `/quit` gönderildi.

Sonuç — DB, uygulama tarafından otomatik onarıldı:

| id | event_sequence (önce) | event_sequence (sonra) | event |
| -- | -- | -- | -- |
| 1 | 1 | 1 | `test.event.a` |
| 2 | 1 | **2** | `test.event.b` |
| 3 | — | **3** | `system-audit-recovery` / `audit.recovered.concurrent_sequence` (eklendi) |

Sıra ekleme sırasına göre yeniden kuruldu, hash zinciri baştan hesaplandı ve görünür bir kurtarma
olayı eklendi — tam olarak [audit integrity düzeltme kaydı](f2_conversation_qa.md#audit-integrity-d%C3%BCzeltme-kayd%C4%B1)
bölümünde tarif edilen davranış. Uygulama crash olmadı, temiz çıktı verdi.

## Kapanış

- Test DB'si atıldı; gerçek kullanıcı `jarvis.db` dosyası geri yüklendi ve `sha256sum` ile test
  öncesi/sonrası **birebir aynı** olduğu doğrulandı
  (`3c2dfa75beb66a8b6b053ce7c6c38d8a053aa97d77b6ef3a4505fc899cd876c2`).
- Model servisleri bu koşum boyunca durdurulmadı/başlatılmadı; yalnız zaten `active` olan
  `jarvis-llama.service` health kontrolünden geçti.

Bu üç senaryo, [DEVELOPMENT_PLAN.md](../DEVELOPMENT_PLAN.md) F2.0 "Yaşam döngüsü otomasyonu"
maddesindeki son açık koşumları kapatır.
