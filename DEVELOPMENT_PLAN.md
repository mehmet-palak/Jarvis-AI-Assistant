# JARVIS Development Plan

Bu dosya JARVIS v2.3 Architecture Frozen / Implementation Baseline sürecinin canlı kayıt dosyasıdır.

## Çalışma kuralları

- Her ana madde alt görevlere ayrılır.
- Bir alt görev, kodu yazıldığı için tamamlanmış sayılmaz.
- Alt görev ancak ilgili testler ve mümkünse gerçek akış kanıtı başarılı olduktan sonra `[x]` yapılır.
- Test başarısızsa checkbox işaretlenmez; hata, sebep ve sonraki adım bu dosyaya yazılır.
- Ana madde, bütün zorunlu alt maddeleri ve test kanıtlarını tamamlamadan `TAMAMLANDI` olmaz.
- Mimari contract değişikliği gerekirse ADR, migration etkisi, test planı ve rollback planı eklenir.
- Her geliştirme turunun sonunda bu dosya gerçek kod durumuyla eşitlenir.

Durumlar: `BEKLENİYOR` · `DEVAM EDİYOR` · `BLOCKED` · `TAMAMLANDI`

---

## Bugünkü Desktop MVP exit gate — PDF §31 ve §34

Durum: TAMAMLANDI — 13 Ağustos 2026

Bu gate, Phase 2–4 işlerini MVP’ye katmaz. Hedef; günlük kullanılabilir, local-first ve kanıtlı ilk desktop dikey dilimidir.

- [x] CLI request → Intent → Decision/Policy → Task → Tool → Verifier → CLI output zinciri
- [x] Bir primary local CPU model ve deterministic fast-path
- [x] Approval, interruption/cancel, SQLite persistence/recovery ve audit
- [x] Initial MCP typed core adapter
- [x] Correlation ID’li structured log/observability ilk dilimi
  - Kanıt: Her audit event task ID’yi correlation ID olarak taşıyan timestamped structured log eventine dönüşüyor; unit test geçti.
- [x] İlk RAG/workspace zero-trust: typed provenance, untrusted-content isolation ve prompt-injection testi
  - Kanıt: `ContentRef`, `UntrustedProjectFile` provenance, path-contained workspace retrieval ve XML-benzeri data isolation eklendi; inject edilmiş komut metni hiçbir tool authority üretmiyor.
- [x] Teacher escalation contract testi; private context için approval gate
  - Kanıt: Private context `ApprovalRequired`, public context `LocalOnly` kararı veriyor; bu katman henüz cloud teacher çağrısı yapmıyor.
- [x] MCP stdio/JSON-RPC transport smoke ve policy-bypass testi
  - Kanıt: `initialize`, `tools/list`, `tools/call` stdio transportu eklendi; gerçek smoke’ta health PASS, `jarvis.shell.exec` DENY.
- [x] Read-only coding/docs capability dikey dilimi
  - Kanıt: `code.project_outline` ve `docs.workspace_summary` registry → policy → verifier zincirinde PASS.
- [x] HUD/voice basics
  - Kanıt: CLI `hud` kompakt desktop durumunu gösteriyor; `voice <transcript>` aynı governed pipeline’a `InputType::Voice` ile giriyor.
- [x] MVP regression + release smoke
  - Kanıt: `cargo fmt`, 45 test, `cargo clippy --all-targets -- -D warnings`, `cargo build --release`, CLI/HUD/voice smoke ve MCP JSON-RPC initialize/list/call smoke geçti.

Gate dışı (sonraki rota): fine-tuning/LoRA, gelişmiş autonomous pentest, mobile/remote, cross-device handoff, tam Agent Desk, derin voice/perception.

---

## MVP tamamlanma kaydı ve ana ürün yol haritası

Durum: PLANLANDI — 14 Ağustos 2026

MVP tamamlandı: local CPU sohbeti, kalıcı model yaşam döngüsü, terminal sohbet ekranı, onay/policy zinciri, SQLite audit/recovery, ilk MCP ve güvenli read-only capability'ler çalışıyor. Bu, “JARVIS fikrinin ispatı”dır; henüz tam masaüstü ürünü veya eğitilmiş özel model değildir.

Bu noktadan sonra mimariyi yeniden tasarlamak yerine aşağıdaki dikey dilimler sırayla teslim edilecek. Her dilim, kendi kullanıcı akışı, güvenlik sınırı, otomatik testleri ve gerçek smoke kanıtı olmadan `[x]` olmayacak.

### Program çalışma ilkeleri

- Mevcut Rust core (`Request → Policy → Task → Tool → Verifier → Audit`) korunur; yeni arayüz veya model adapterı bu zinciri bypass edemez.
- Bir model, embedding modeli veya sistem paketi indirilmeden önce boyut, RAM/VRAM etkisi, lisans ve ne için gerektiği kullanıcıya açıkça söylenir; indirme kullanıcı onayıyla başlar.
- Önce günlük kullanım değeri ve veri güvenliği, sonra otonomi gelir. Fine-tuning ve aktif pentest, ölçüm/scope/sandbox katmanlarından önce başlatılmaz.
- Her yeni yetenek için en az bir başarı, bir reddetme/edge-case ve bir gerçek local smoke testi gerekir.
- TUI MVP olarak korunur; native masaüstü kabuğu aynı core'a ikinci bir istemci olarak eklenir. Core yeniden yazılmaz.

### Tüm fazlar — bağımlılık haritası

| Faz | Hedef | Durum | Başlamadan önce gerekli olan |
| --- | --- | --- | --- |
| F0 | Mimari, typed core, persistence ve güvenlik contractları | TAMAMLANDI | — |
| F1 | Local-first Desktop MVP | TAMAMLANDI | F0 |
| F2 | Günlük masaüstü ürün deneyimi, native UI ve görsel/dosya ekleri | DEVAM EDİYOR | F1 |
| F3 | Kontrollü bellek, profil ve gerçek RAG | BEKLENİYOR — F2 exit gate | F2 attachment/provenance temeli |
| F4 | Onaylı, izole coding ve yerel iş workbench'i | BEKLENİYOR — F2 exit gate | F2 + OS-isolated worker |
| F5 | Push-to-talk ses ve çoklu algı arayüzü | BEKLENİYOR | F2 native UI |
| F6 | Benchmark, dataset governance ve geri alınabilir model adaptasyonu | BEKLENİYOR | F3/F4 gerçek eval verisi |
| F7 | Yazılı yetkili ve teknik olarak sınırlı security/pentest | BEKLENİYOR | F4 isolation + F9 operasyon kapıları |
| F8 | MCP ekosistemi, entegrasyonlar ve güvenli remote/mobile | BEKLENİYOR | F3, F7 trust/permission temeli |
| F9 | Operasyonel olgunluk, release ve uzun dönem bakım | BEKLENİYOR | F2–F8 boyunca sürekli yürür |

Programın bitiş tanımı: F2–F9'un zorunlu maddeleri, güvenlik/kalite kapıları ve kullanıcı kabul senaryoları tamamlanmış olacak. F10 araştırma/deney alanı ise ürünün zorunlu teslim kriteri değildir; yalnız stabil sürümden sonra kontrollü deneyler içindir.

### Mevcut mimari backlog eşlemesi

Bu dosyanın aşağısındaki ayrıntılı mimari bölümleri korunur; aşağıdaki tablo her açık maddenin hangi ürün fazında kapanacağını gösterir.

| Mevcut bölüm | Sahip faz | Kapanış hedefi |
| --- | --- | --- |
| 3. Capability Registry ve güvenli tool runtime | F4 + F9 | Isolated worker, gerçek iptal/cleanup, retry/idempotency, backup/dry-run |
| 4. Local model adapter ve routing | F2 + F6 + F9 | UX/health, benchmark, model registry ve rollback |
| 5. RAG, workspace ve memory | F3 | Ingestion, sensitivity, retrieval, context budget ve silme |
| 6. Teacher–Student learning ve dataset governance | F6 | Dataset sürümü, deletion marker, eval, LoRA/QLoRA ve rollback |
| 7. Yetkili security/pentest | F7 | Authorization, network enforcement, sandbox, evidence ve deny testleri |
| 8. Remote device trust ve task handoff | F8 | Pairing, public key, replay koruması, revoke ve handoff |
| 9. MCP vertical slice | F8 | Credential filtresi, provenance, extension trust ve permission UX |
| 10. Observability, audit integrity ve recovery | F9 | Retention, metrikler, witness/export ve config/model rollback |
| 11. Test ve kalite kapısı | F2.0 + F9 | E2E, concurrency/cancel/lock, release gate ve sürekli regression |

### F0 — Mimari ve güvenli core

Durum: TAMAMLANDI

- [x] Mimari referans: v2.3 frozen architecture, ADR kararları ve implementation baseline.
- [x] Typed contracts: `Request`, `Task`, `ToolResult`, `VerifierResult`, `PolicyResult`, `CapabilityManifest` ve `ConversationMessage`.
- [x] Governed pipeline: intent → policy → task → tool → verifier → audit sırası; registry dışı capability reddi.
- [x] Policy/approval temel akışı: risk sınıfları, task-bound approval, expiry/scope hash ve cancel/resume.
- [x] Kalıcı local store: SQLite migration, task/approval/audit tabloları, startup recovery ve snapshot API.
- [x] Audit integrity: sequence, SHA-256 hash-chain, tampering detection ve correlation-scoped structured event.
- [x] Model güvenlik sınırı: provider adapterı tool/policy authority kazanmaz; native user/assistant context data olarak tutulur.
- [x] Zero-trust content başlangıcı: workspace path containment, provenance ve prompt-injection isolation.
- [x] İlk typed MCP ingress ve policy-bypass testleri.
- [x] Core quality gate: format, strict Clippy, optimize release build ve unit/contract testleri.

Çıkış kanıtı: F1'in güvenle üzerine inşa edildiği MVP core ve regression seti.

### F1 — Local-first Desktop MVP

Durum: TAMAMLANDI

- [x] Kalıcı CPU-only `llama-server`: loopback-only servis, `-ngl 0`, RAM lifecycle ve `/health` kontrolü.
- [x] Terminal sohbeti: salt-okunur geçmiş, dinamik taslak alanı, loading state, scrollbar ve uzun tur görünürlüğü.
- [x] Girdi ergonomisi: native paste, `Ctrl+V`, kelime silme, draft temizleme, klavye/mouse ile history navigation.
- [x] Yanıt davranışı: bounded chat history, `finish_reason=length` continuation ve Hyprland yanıt bildirimi.
- [x] Local model runtime: Qwen3-8B-Q4_K_M ile CPU sohbeti, model açık/kapalı kullanıcı semantiği ve VRAM=0.
- [x] Güvenli ilk capability'ler: system health/time, workspace read, project/coding/docs summary ve approval-gated note.
- [x] MCP stdio transportu: initialize, tool list, typed call ve bilinmeyen tool deny.
- [x] İlk security scope contractı ve teacher-example intake contractı.
- [x] MVP regression gate: 51 test, Clippy, release build, service health ve interaktif smoke.

Çıkış kanıtı: `jarvis` günlük metin sohbeti ve ilk governed capability'leri local-first olarak çalıştırır.

### F2 — Günlük masaüstü ürünü ve multimodal ekler

Durum: DEVAM EDİYOR — attachment/TUI ilk dilimi tamamlandı; native pencere ve vision açık.

Amaç: Terminal MVP'yi korurken, gerçek günlük kullanım için native masaüstü deneyimi, dosya/görsel ekleri ve ölçülebilir UX kalitesi oluşturmak.

#### F2.0 — MVP stabilizasyonu ve ürün kalite kapısı

Durum: DEVAM EDİYOR

Amaç: Yeni büyük yetenek eklemeden önce günlük kullanım regresyonlarını görünür ve tekrar üretilebilir hale getirmek.

- [ ] Girdi otomasyonu: paste, `Ctrl+V`, `Ctrl+Backspace`, `Ctrl+W`, `Ctrl+U`, `Esc`, UTF-8/Türkçe ve çok satırlı metin senaryoları.
- [ ] Geçmiş otomasyonu: klavye, mouse wheel, `Home/End`, taşan uzun kullanıcı/model turu ve yeni yanıt geldiğinde en alta dönüş.
- [ ] Yaşam döngüsü otomasyonu: ilk açılış, servis yükleme, `/quit`, `Ctrl+C`, terminal kapanması, `exit`, yeniden açılış ve DB recovery.
- [ ] Bildirim otomasyonu: yanıt hazır, model/servis hatası, approval bekleme; notification daemon yokken graceful fallback.
- [ ] TUI görsel smoke: küçük/büyük terminal, resize, yüksek DPI/font farklılığı, okunabilir kontrast ve odak/cursor davranışı.
- [ ] Sürümlü sohbet kalite seti: Türkçe selamlaşma, kısa takip sorusu, uzun bağlam, konu değişimi, belirsizlik, tool-iddiası ve güvenli reddetme.
- [ ] Her kalite örneği için beklenen davranış, model/prompt sürümü, latency limiti ve insan değerlendirme alanı.
- [ ] Hata/backlog şablonu: kullanıcı raporu, tekrar adımı, beklenen/gerçek sonuç, log/task ID, düzeltme commit'i ve regression testi.
- [ ] Tek release komutu: format, test, clippy, dependency check, release build, servis health, kritik E2E smoke ve özet rapor.
- [ ] F2.0 exit review: açık P0/P1 kullanım hatası kalmadığının manuel kabulü.

Tamamlanma ölçütü: Günlük metin giriş/çıkış davranışı en az 20 senaryoda tekrar üretilebilir; yeni dilimler bu regression setini geçmeden birleşmez.

#### F2.1 — Native desktop kabuğu ve gerçek görsel ekler

Durum: DEVAM EDİYOR — güvenli attachment intake ve TUI kuyruğu eklendi; native GUI/vision açık.

Amaç: Terminal MVP'yi terk etmeden, fotoğraf ve dosya eklemeye uygun gerçek masaüstü deneyimini kurmak.

- [ ] UI teknoloji spike: `egui/eframe` penceresi; açılış süresi, bellek kullanımı, Wayland/Hyprland uyumu ve paketleme riski ölçülür.
- [ ] UI/core sınırı: native UI yalnız client olur; `jarvis-core` Request/Policy/Task/Verifier zincirini doğrudan kullanır, ikinci runtime yaratmaz.
- [ ] Sohbet ekranı: message card, streaming/typing state, ayrı draft composer, scroll-to-latest, arama/filtre hazırlığı ve erişilebilir klavye odağı.
- [ ] Pencere yaşam döngüsü: resize, minimize, tekrar odak, tek-instance davranışı, servis durumu, bildirim tıklamasında pencereyi öne alma.
- [ ] Görsel tasarım sistemi: renk/typography/spacing tokenları, açık-koyu tema, kontrast kontrolü ve Türkçe metin taşma davranışı.
- [ ] Yerel ayarlar: UI tercihleri, tema, font scale ve notification seçeneği için versioned config; reset/export akışı.
- [ ] Attachment contract: `AttachmentRef` (ID, canonical local path, MIME, byte size, SHA-256, oluşturulma zamanı, provenance, sensitivity) ve task/audit ilişkisi.
- [ ] Attachment storage policy: orijinal dosya yerinde referans mı yoksa uygulama kasasında kopya mı; retention, local delete ve stale-reference davranışı için ADR.
- [ ] Güvenli dosya seçimi: `Ctrl+O`/ataç düğmesi, kullanıcı görünür dosya adı/önizleme ve gönderimden önce kaldırma.
- [ ] Dosya doğrulama: MIME magic-byte kontrolü, canonical path, allowlist, boyut/piksel limitleri, decode bomb/bozuk dosya reddi ve SHA-256.
- [ ] Metin/doküman ekleri: ilk aşamada yalnız güvenli metadata + ayrı RAG ingestion kuyruğu; ekin ham içeriği tool talimatı sayılmaz.
- [ ] Vision model kararı: CPU uyumlu multimodal GGUF + eşleşen `mmproj` adayları, lisans, disk/RAM/latency karşılaştırması. **İndirme ancak kullanıcı onayından sonra.**

> Kullanıcı kararı — 14 Ağustos 2026: normal geliştirme indirimi önce boyutu bildirilerek **en fazla 100–200 MB** olabilir. Vision GGUF (yaklaşık 2–4 GB) ve `mmproj` (yaklaşık 0.4–1 GB) şimdilik ertelendi; birkaç saat sonraki durum güncellemesinde kullanıcıya yeniden hatırlatılacak. Bu dosyalar için açık “indir” onayı olmadan hiçbir indirme başlatılmaz.
- [ ] Vision service: text modelinden ayrı loopback-only endpoint, health/lifecycle, attachment byte/path passing ve timeout/cancel sınırı.
- [ ] Vision response policy: yalnız görüntü açıklaması/analizi; görüntü OCR metni untrusted data, tool authority yok; desteklenmeyen/hassas içerik için açık hata.
- [ ] Attachment privacy UX: ek geçmişini görme, tekli/tüm ekleri silme, ek gönderilmeden önce local-only uyarısı ve export.
- [ ] E2E: JPEG/PNG başarı; bozuk MIME, boyut/piksel limiti, stale dosya, model kapalı, injection/EXIF izolasyonu ve TUI fallback.

Bu turda kanıtlanan alt dilim:

- [x] Attachment typed core: `AttachmentRef` canonical path, MIME, byte/pixel boyutu, SHA-256, provenance ve sensitivity ile Request/audit zincirine bağlandı.
- [x] PNG/JPEG magic-byte/header doğrulaması, boyut/piksel limiti, path containment, stale/replaced-file reddi ve attribute-escaping regresyon testleri eklendi.
- [x] TUI ek kuyruğu: `/attach <PNG/JPEG-yolu>`, `/attachments` ve `/attachments clear`; gönderilen ekler yalnız metadata/data envelope olarak modele taşınır. Text-only model için görsel analiz iddiası yapılmaz.

#### F2 güncel çalışma kaydı — henüz exit gate değildir

- [x] Yerel release kontrolü: `bash scripts/release_check.sh` format, kilitli/offline bağımlılık çözümü, 72 test, strict Clippy ve release build çalıştırır. `--with-service`, yalnız kullanıcının açık tuttuğu loopback model servisinin health kontrolünü ekler; servis başlatmaz ve İnternet'e çıkmaz.
- [x] TUI davranış regresyonu: çok satırlı paste, Türkçe/UTF-8 kelime silme, `Ctrl+V`, `Ctrl+Backspace`, `Ctrl+W`, `Ctrl+U`, terminal-control karakterleri, klavye/mouse scroll, `Home`/`End`, küçük terminalde scrollbar ve en yeni turun görünürlüğü testlere bağlandı.
- [x] Native UI temel kodu: `jarvis-desktop` aynı `Runtime` örneği üzerinde salt-okunur kartlar, ayrı composer, typing state, `Ctrl+O` görsel seçimi, güvenli önizleme/kaldırma, model-RAM kontrolü ve versioned yerel UI tercihleri (reset/export dahil) sunar. Pencereyi kapatmak servisi durdurmaz.
- [ ] Native UI Wayland/Hyprland gerçek smoke: açılış, resize/minimize/focus, `Ctrl+O` picker, mesaj gönderme, bildirim tercihi ve pencere kapanışının model servisini canlı bırakması kullanıcı masaüstüsünde doğrulanacak.
- [ ] Vision modeli/multimodal E2E: kullanıcı onayıyla indirilecek model + `mmproj` olmadan görsel pikselleri modele verilemez; mevcut davranış güvenli metadata-only fallback'tir.

Tamamlanma ölçütü: Kullanıcı masaüstü penceresinden tek bir fotoğraf seçip ne gördüğünü sorabilir; ek hem UI'da görünür hem de core policy/audit zincirinde güvenli data olarak kalır.

### F3 — Kullanıcı profili, kontrollü bellek ve gerçek RAG

Durum: BEKLENİYOR — F2 exit gate kapanana kadar yeni F3 işi yapılmaz; mevcut temel kod park edilmiştir.

Amaç: JARVIS'in kişisel bilgiyi hard-code etmeden, izinli ve açıklanabilir biçimde hatırlaması; belgelerden kaynaklı cevap vermesi.

- [ ] Profile schema/ADR: ad, hitap biçimi, dil, rol/tercih, sensitivity, source ve updated-at alanları; sohbetten otomatik persistent write varsayılan olarak kapalı.
- [ ] Profile CRUD UX: kullanıcı açıkça ekler/düzenler/siler; her alan için “modele dahil etme” anahtarı ve export/reset seçeneği.
- [ ] Profile injection boundary: profile alanları da system prompt değil typed data olarak taşınır; model profile üzerinden tool yetkisi kazanamaz.
- [ ] Memory namespace'leri: session, user-profile, project, task ve ephemeral tool-output fiziksel/şematik olarak ayrılır.
- [ ] Memory write policy: önerilen kayıt → kullanıcı preview/onay → sensitivity/TTL seçimi → audit; model kendiliğinden kalıcı anı yazamaz.
- [ ] Memory retrieval policy: namespace/sensitivity/TTL filtreleri, kullanıcıya “neden kullanıldı” bilgisi ve kaynaklı cevapta görünür attribution.
- [ ] Memory deletion: tek kayıt, namespace, proje ve “her şeyi unut” silme; tombstone/backup etkisi ve doğrulama testi.
- [ ] Memory migration/backup: versioned schema, encrypted-secret ayrımı gerekiyorsa ADR, export/import ve rollback.
- [ ] Workspace izin UX'i: klasör seçimi, kök sınırı, indeks kapsamı, exclude pattern ve indeks boyutu tahmini kullanıcıya gösterilir.
- [ ] Document parser katmanı: Markdown/TXT/PDF başlangıcı; sonradan Office/HTML için ayrı parser ve sandbox kararı.
- [ ] Ingestion pipeline: canonical path, content hash, MIME/size limiti, chunking, dosya değişiklik algısı ve incremental re-index.
- [ ] Metadata/FTS index: SQLite metadata-first retrieval, belge/chunk ID, konum, hash, provenance ve indeks sürümü.
- [ ] Embedding/re-rank kararı: FTS baseline ölçülür; embedding model gerekiyorsa boyut/RAM/lisans bilgisi ve kullanıcı onayıyla indirilir.
- [ ] Secret/hassas filtre: `.env`, private key, credential, binary, çok büyük dosya ve kullanıcı exclude listesi indeks dışı; filtre loglanır ama sır saklanmaz.
- [ ] Retrieval policy: relevance threshold, result sayısı, token/context budget, duplicate suppression ve kaynağı olmayan cevabı engelleme.
- [ ] Citation UX: yanıtın hangi belge/parçadan geldiği, kısa alıntı, dosya konumu ve “kaynağı aç” davranışı.
- [ ] Untrusted-content isolation: doküman/OCR/web metni data envelope içinde kalır; prompt injection, tool call ve data exfiltration denemeleri reddedilir.
- [ ] RAG eval seti: doğru kaynak, yanlış kaynak, secret exclusion, eski indeks, çelişen belge, injection ve silinmiş bellek senaryoları.

Bu turda kanıtlanan alt dilim:

- [x] Kontrollü bellek persistence: user-profile/project/task namespace'leri, schema migration, model-context opt-in, TTL filtresi, explicit proposal/onay, audit ve tekli/tüm kayıt silme.
- [x] TUI bellek UX'i: `/remember anahtar = değer` → preview → `/remember approve|reject`; `/memory` görünürlüğü ve `/forget <id>|all` silme akışı.
- [x] İlk gerçek RAG: explicit `/index <proje-içi-göreli-dosya>`, canonical root/path, SHA-256, chunking, SQLite FTS, source citation, stale chunk replacement, secret/binary/büyük dosya reddi ve untrusted-content isolation.

Tamamlanma ölçütü: Kullanıcı bir klasörü izinle indeksleyip kaynak gösteren cevap alabilir ve saklanan tüm kişisel veriyi görüntüleyip silebilir.

### F4 — Güvenli coding ve yerel iş workbench'i

Durum: BEKLENİYOR — F2 exit gate kapanana kadar yeni F4 işi yapılmaz; mevcut temel kod park edilmiştir.

Amaç: JARVIS'in kod tabanını anlaması, değişiklik önermesi ve yalnız onayla izole ortamda doğrulaması.

- [ ] Worker threat model/ADR: host çalışma alanı, ağ, shell, secret, process tree ve resource exhaustion için saldırı modeli ve seçilen izolasyon yaklaşımı.
- [ ] Isolated worker bootstrap: workspace snapshot/overlay, ayrı çalışma dizini, read-only input ve sınırlandırılmış output artifact alanı.
- [ ] OS izolasyonu: kullanıcı namespace/container/bubblewrap seçimi, ağ=kapalı varsayılanı, mount allowlist, no-new-privileges ve seccomp fizibilitesi.
- [ ] Resource kontrolü: CPU/RAM/disk/PID/time quota, process group, watchdog, stdout/stderr limitleri ve güvenli cleanup.
- [ ] Gerçek cancellation: task cancel → child process signal → grace period → kill → snapshot cleanup → audit/verifier sonucu.
- [ ] Allowlist command runner: her komut için manifest, argüman schema, cwd scope, env allowlist, dry-run ve evidence capture.
- [ ] Read-only project analyst: repo overview, dependency/test discovery, riskli dosya uyarısı ve hiçbir yazma yapmadan plan üretme.
- [ ] Coding plan UX: yapılacaklar, etkilenen dosyalar, varsayımlar, test planı, tahmini risk ve kullanıcı soruları.
- [ ] Patch generator: unified diff, dosya/path containment, diff hash, maksimum değişiklik limiti ve binary/secret dosya reddi.
- [ ] Patch preview/review: satır bazlı görünüm, seçilebilir dosya scope'u, kullanıcı değişiklik notu ve explicit approve/reject.
- [ ] Patch apply transaction: approval'a bağlı diff hash, snapshot/backup, atomic write, başarısızlıkta rollback ve audit.
- [ ] Test/verifier runner: allowlisted test komutu, exit code/log özeti, değiştirilen dosya hash'i ve mevcut test regresyon raporu.
- [ ] Coding evaluation seti: küçük hata düzeltme, test ekleme, yanlış patch reddi, timeout/cancel, secret exposure ve mevcut-test regression senaryoları.
- [ ] Yerel üretkenlik tool framework: takvim, not, dosya düzenleme gibi her yeni tool için capability manifest, minimum scope, preview, approval ve verifier.
- [ ] Çok-adımlı workflow runner: planı kullanıcıya gösterme, her yan etkili adımdan önce policy/approval, retry/idempotency, iptalde cleanup ve audit özeti.

Bu turda kanıtlanan alt dilim:

- [x] Worker threat-model ADR: [ADR-0001](docs/adr/0001-isolated-coding-worker.md) host fallback yasağını, ağ=kapalı worker kararını ve açık kalan quota/cancel sınırlarını tanımlar.
- [x] Coding plan/patch contract: workspace-relative scope, network-denied limitler, unified-diff/path/hash doğrulaması, proposal-bound approval, snapshot, `git apply --check`, dosya SHA-256 verifier kanıtı, rollback ve audit bağı kuruldu.
- [x] Release worker policy: Bubblewrap/network namespace kurulamazsa patch execution reddedilir; host shell fallback yoktur. Test harness'i container `CLONE_NEWNET` kısıtı nedeniyle yalnız semantic patch testini kontrollü geçici klasörde çalıştırır.

Tamamlanma ölçütü: JARVIS bir değişikliği önce gösterir, kullanıcı onayı olmadan yazmaz; onay sonrası yalnız scope içindeki patch'i uygular ve test kanıtını döndürür.

### F5 — Sesli etkileşim ve algı arayüzü

Durum: BEKLENİYOR

Amaç: Her zaman dinleyen bir sistem yerine açık, mahremiyeti koruyan push-to-talk ses akışı.

- [ ] Audio ADR: PipeWire/Wayland cihaz erişimi, mikrofon izinleri, örnekleme formatı, gecikme hedefi ve recording retention varsayılanı.
- [ ] STT aday değerlendirmesi: Türkçe doğruluk, CPU/RAM, model boyutu, lisans, offline destek ve warm-start süreleri. **İndirme kullanıcı onayıyla.**
- [ ] Push-to-talk capture: tuş basılıyken kayıt, ses seviyesi/VAD göstergesi, bırakınca transkript kuyruğu ve kolay iptal.
- [ ] Transkript editörü: gönderim öncesi metni görme, düzeltme, silme, yeniden deneme ve normal `InputType::Voice` pipeline'ına dönüştürme.
- [ ] Voice privacy: ham ses varsayılan olarak kalıcı değil; kullanıcı isterse geçici dosyanın yeri/silme zamanı görünür.
- [ ] TTS aday değerlendirmesi: Türkçe ses kalitesi, lisans, CPU kullanımı, ses modeli boyutu ve offline çalışma; indirme onaylı.
- [ ] TTS playback: yanıt bitince opt-in oynatma, duraklat/durdur, hız/ses seçimi, kulaklık cihaz değişimi ve sessiz mod.
- [ ] Sesli approval UX: yüksek riskli aksiyon için yalnız ses değil, ekranda açık yazılı onay veya güvenli ikinci doğrulama.
- [ ] Wake word araştırma spike: ayrı feature flag, lokal algılama, görünür dinleme göstergesi, fiziksel/klavye kill switch ve retention=off.
- [ ] Accessibility: klavye-only kullanım, ekran okuyucu metinleri, işitme/görme farklılıkları için eşdeğer metin kontrolleri.
- [ ] E2E: mikrofon izin reddi, cihaz yok, model yok, sessizlik/gürültü, Türkçe transkript, iptal, sesli tool approval ve kayıt silme.

Tamamlanma ölçütü: Kullanıcı bir tuşa basıp konuşur, gönderilecek transkripti görür/onaylar ve yanıtı isterse sesli duyar.

### F6 — Model kalite, dataset governance ve adaptasyon

Durum: BEKLENİYOR

Amaç: Sohbeti hard-code etmek yerine ölçmek; gerekiyorsa küçük, geri alınabilir bir model adaptasyonu yapmak.

- [ ] Sürümlü benchmark: Türkçe diyalog, takip sorusu, güvenlik sınırı, RAG doğruluğu ve coding görevleri için golden set + latency/quality raporu.
- [ ] Dataset export/versioning: yalnız human-reviewed, verifier-passed, sensitivity etiketli örnekler; silme/poisoned-example marker'ları ve dataset manifest hash'i.
- [ ] Model karşılaştırması: mevcut Qwen3 baseline ile aday modellerin CPU/RAM gecikmesi ve kalite ölçümü.
- [ ] LoRA/QLoRA fizibilite kararı: VRAM/RAM, eğitim süresi, lisans, eval hedefi ve rollback artifact'i kullanıcıya sunulmadan eğitim başlamaz.
- [ ] Old-vs-new regresyonu ve tek komutla model/adaptor rollback.
- [ ] Kullanıcı geri bildirimi intake'i: beğen/beğenme veya düzeltme sinyali doğrudan eğitim verisi olmaz; sensitivity, provenance ve human review kuyruğundan geçer.
- [ ] Prompt/model konfigürasyon registry'si: her deneyin model hash'i, prompt sürümü, benchmark sonucu ve rollback hedefi kaydedilir.

Tamamlanma ölçütü: Her model veya adapter değişikliği, sürümlü eval'de hedef metriği iyileştirir ve güvenlik/latency regresyonu üretmez; aksi halde kullanılmaz.

### F7 — Yetkili security/pentest hazırlığı

Durum: BEKLENİYOR — F4 izolasyonundan önce execution açılmaz

Amaç: “sızma testi yapabilen” değil, yalnız yazılı yetki ve teknik sınırlar altında güvenli değerlendirme yapabilen bir capability oluşturmak.

- [ ] İmzalı authorization/scope manifest, hedef canonicalization, CIDR semantiği, DNS pinning/rebinding savunması ve expiry/revoke.
- [ ] Network-scoped sandbox worker: yalnız allowlist egress, rate/runtime limiti, kill switch, dry-run ve gerçek cancellation/cleanup.
- [ ] Önce SAFE/read-only envanter ve raporlama; ACTIVE/INTRUSIVE/DESTRUCTIVE modları varsayılan olarak kapalı kalır.
- [ ] Evidence tabanlı finding formatı, insan onayı, audit export ve scope dışı/secret hedef deny testleri.

Tamamlanma ölçütü: Scope dışı hiçbir hedefe trafik çıkamaz; SAFE modda üretilen her bulgu kanıt ve audit ile ilişkilidir. Bu gate geçmeden aktif test capability'si eklenmez.

### F8 — MCP ekosistemi, entegrasyonlar ve güvenli remote/mobile

Durum: BEKLENİYOR

- [ ] MCP production hardening: protocol sürümleme, extension/tool manifest imzası, credential/raw-secret response filtresi, untrusted output provenance ve tool permission ekranı.
- [ ] Yerel entegrasyonlar: takvim, e-posta, mesajlaşma veya dosya sağlayıcısı yalnız explicit OAuth/secret store, minimum scope, dry-run/preview, approval ve revoke ile eklenir.
- [ ] Plugin/skill ekosistemi: signed/allowlisted paketler, capability sandbox profile, sürüm uyumluluğu, kilitleme dosyası ve tek tıkla devre dışı bırakma.
- [ ] Remote/mobile yalnız explicit device pairing, public key, nonce/replay koruması, revoke, bağlantı şifreleme ve server-side kill switch'ten sonra ele alınır.
- [ ] Cross-device handoff: task scope'u genişletmeyen typed handoff, offline queue/conflict policy ve alıcı cihazda yeniden policy değerlendirmesi.

Tamamlanma ölçütü: Yerel desktop sürümü güvenilir olmadan hiçbir eklenti, entegrasyon veya remote cihaz tool yetkisi ya da kişisel bellek erişimi almaz.

### F9 — Operasyonel olgunluk, release ve uzun dönem bakım

Durum: BEKLENİYOR — F2 ile birlikte başlar, ürün yayınından önce kapanır

- [ ] Release pipeline: format, test, clippy, dependency/security denetimi, release build, migration kontrolü ve E2E smoke'u tek raporda birleştirme.
- [ ] Metrikler: latency, model yükleme, token üretimi, başarı/verification oranı, iptal, hata sınıfı, CPU/RAM/disk kullanımı; kişisel içerik toplamadan yerel telemetry.
- [ ] Gerçek timeout/cancellation worker: process group, cleanup handles, resource quota, watchdog ve stuck-task recovery.
- [ ] Backup/retention komutları, config/model/dataset rollback, audit export/witness stratejisi ve restore tatbikatı.
- [ ] Sürüm/migration yönetimi: semantic version, changelog, config migration, compatibility check ve kullanıcı verisi geri dönüş planı.
- [ ] Güvenlik bakım döngüsü: bağımlılık güncellemesi, secret scanning, threat-model review, penetration-test bulgu takibi ve responsible disclosure kanalı.
- [ ] Kullanıcı kabul/release gate: offline çalışma, veri silme/export, model kapatma, erişilebilirlik, Türkçe UX ve performans hedefleri için checklist.

Tamamlanma ölçütü: Yeni sürüm kurulabilir, geri alınabilir, yedekten döndürülebilir ve kritik kullanıcı akışları kanıtlı biçimde çalışır.

### F10 — Kontrollü araştırma ve uzun vadeli evrim

Durum: BEKLENİYOR — v1 teslimi değildir

- [ ] Daha büyük/özel modeller, çoklu ajan koordinasyonu, federated/on-device learning ve ileri perception yalnız benchmark + threat model + maliyet değerlendirmesi sonrası deney dalında değerlendirilir.
- [ ] Her araştırma deneyi ana sürümden feature flag, ayrı artifact ve rollback ile ayrılır; kullanıcı verisi deney setine varsayılan olarak girmez.
- [ ] Başarılı deneyler yalnız F6 eval kapısını ve F9 release kapısını geçerse ana ürüne taşınır.

### Önerilen uygulama sırası

1. **F2.0 stabilizasyonu** — önce eldeki kullanım hatalarını test edilebilir hale getiririz.
2. **F2.1 native desktop + vision** — fotoğraf/dosya ihtiyacını ve terminal sınırlarını doğrudan çözer.
3. **F3 memory + RAG** — kişiselleşme ve dokümanlarla gerçek çalışma bu katmandan gelir.
4. **F4 coding/workflow workbench** — sadece izolasyon ve onay temeli üstünde.
5. **F5 voice** — arayüz temelinin üzerine eklenir.
6. **F6 quality/LoRA** — hangi eğitimin gerçekten gerekli olduğunu benchmark gösterdikten sonra.
7. **F7 security** ve **F8 integrations/remote** — en son, çünkü hata maliyetleri daha yüksektir.
8. **F9 release/operations** — her fazda yürür; v1 yayınından önce zorunlu olarak kapanır.

İlk somut iş: F2.0'ın küçük regresyon paketiyle birlikte F2.1'in native desktop/attachment contract spike'ı. Vision modeli indirme noktasına geldiğimizde burada durup kullanıcıdan açık onay alınır.

---

## Desktop usability ve local model lifecycle hardening

Durum: TAMAMLANDI — 14 Ağustos 2026

Bu dilim MVP'nin yetki sınırlarını genişletmez. Amaç, ilk günlük kullanımın beklenebilir, düşük gecikmeli ve kaynak açısından açık davranmasıdır.

- [x] Kalıcı CPU-only `llama-server` katmanı
  - Yapılan: `LlamaServerProvider`, loopback OpenAI-compatible `/v1/chat/completions` adapterı ve `jarvis-llama.service` eklendi. Sunucu `-ngl 0`, 8 CPU thread, 2048 context ve normal sohbet için en fazla 256 output token ile çalışıyor.
  - Sonuç: Model her mesajda yeniden RAM'e yüklenmiyor; VRAM katmanı kullanılmıyor.
- [x] Model servisini JARVIS açılışına bağlama
  - Yapılan: `jarvis`, servis aktif değilse `systemctl --user start jarvis-llama.service` ile otomatik başlatıyor; sohbet ekranı yükleme durumunu gösteriyor ve mesajı kaybetmiyor.
  - Sonuç: Uygulama hangi dizinden açılırsa açılsın local servis kullanılabiliyor.
- [x] Dinamik giriş alanlı terminal sohbet ekranı
  - Yapılan: ratatui/crossterm ile salt-okunur mesaj geçmişi, altta tek satırdan başlayıp yukarı doğru dinamik büyüyen input alanı, arka plan yanıt worker'ı, loading durumu ve ayrı scrollbar eklendi.
  - Sonuç: Kullanıcı model yanıtını düzenleyemez veya silemez; `Enter` mesajı ayrı bir konuşma öğesi olarak gönderir. Uzun taslak, history alanını kontrollü olarak küçülterek büyür; ekran sınırında en yeni bölüm ve cursor görünür kalır. Geçmişin metin genişliği ile scroll hesabı eşleşir; uzun mesajlar kaybolmaz ve `↑/↓` ile bütünü okunur.
- [x] Uzun tur görünürlüğü ve yanıt tamamlama
  - Yapılan: Scrollbar için ayrılmış metin kolonu ile satır/yükseklik hesabı Ratatui'nin kendi Unicode-aware word-wrapper'ına bağlandı; yeni mesaj gönderildiğinde görünüm en yeni tura döner. Servis bağlamı 2048 tokena, normal sohbet üretim bütçesi 256 tokena çıkarıldı. Sunucu yine `length` döndürürse adapter, içerik-özel kural olmadan bir adet bounded continuation üretir.
  - Sonuç: Uzun kullanıcı mesajı geçmişte eksiksiz saklanır ve kaydırılarak okunur. `finish_reason=length` artık model adapterında ele alınır; yanıt, mümkün olduğunda aynı turun devamıyla tamamlanır.
- [x] Native conversation context ve personal-data sınırı
  - Yapılan: `ConversationMessage` contractı eklendi; `llama-server` geçmişi tek bir string yerine gerçek `user`/`assistant` message dizisi olarak alıyor. Geçmiş artık en son 8 tam konuşma çiftini koruyor; yeni kullanıcı turu öncesinde en eski çift birlikte çıkarılıyor.
  - Sonuç: Cevaplar veya Mehmet/Tony Stark gibi kişisel bilgiler uygulama koduna gömülmüyor. Generic system boundary son kullanıcı mesajına öncelik, kısa takip sorularında yakın bağlam çözümü ve önceki cevabı gereksiz tekrarlamama davranışı verir; kalıcı kullanıcı profili/bellek sonraki rotadadır.
- [x] Yanıt hazır masaüstü bildirimi
  - Yapılan: TUI worker'ı tamamlanan, boş olmayan yanıt için `notify-send` üzerinden Hyprland bildirimine kısa önizleme gönderiyor.
  - Sonuç: JARVIS terminali arka plandayken yanıtın geldiği görünür; bildirim daemonu yoksa görev sonucu veya UI etkilenmez.
- [x] Terminal metin düzenleme kısayolları
  - Yapılan: Bracketed paste ve mouse capture etkinleştirildi; `Ctrl+V` Wayland panosundan, terminalin native paste olayı ise doğrudan taslağa ekleniyor. `Ctrl+Backspace`/`Ctrl+W` önceki kelimeyi ve ayırıcı boşluklarını, `Ctrl+U` taslağın tamamını siler; mouse tekerleği history scrollbar'ını hareket ettirir.
  - Sonuç: Çok satırlı yapıştırmalar istemeden mesaj göndermez; boşluklar güvenli biçimde tek satır taslağa dönüştürülür ve UTF-8 Türkçe metin sınırları korunur. Klavye ve mouse ile geçmiş, model yanıtı beklenirken de gezilebilir.
- Vision attachment dilimi MVP dışındadır; ayrıntılı uygulama sırası ve güvenlik gate'i **F2.1 — Native desktop kabuğu ve gerçek görsel ekler** altında planlandı.
- [x] Approval UX'in yeni ekrana taşınması
  - Yapılan: `/approvals`, `/approve`, `/approve <task-id>`, `/cancel`, `/cancel <task-id>` komutları TUI'ye bağlandı.
  - Sonuç: Kalıcı işlem onayı, eski satır CLI'sındaki güvenlik contractını koruyor.
- [x] Açık RAM lifecycle semantiği
  - Yapılan: `exit` servisi kontrollü durdurur ve model RAM'den çıkar; `/quit`, `Ctrl+C` veya Hyprland `Super + Q` yalnız kullanıcı arayüzünü sonlandırır.
  - Sonuç: Arayüz hızlı yeniden açılış için modeli açık bırakabilir; kullanıcı istediğinde RAM'i tek komutla boşaltabilir.
- [x] Regression, service ve interaktif smoke
  - Kanıt: 51 test, `cargo clippy --all-targets -- -D warnings`, release build, aktif servis `/health` cevabı, gerçek TUI'de mesaj gönderme/yanıt alma, dinamik uzun-taslak smoke, `exit → inactive`, sonraki `jarvis → active`, `Ctrl+C → active` akışları geçti. Son uzun-Türkçe-turn smoke'u `finish_reason=stop` verdi.

---

## 0. Architecture baseline ve repository hazırlığı

Durum: TAMAMLANDI

- [x] v2.3_final mimari referans olarak belirlendi.
  - Yapılan: Policy, Task, Tool, Verifier, Model Zero-Trust ve typed contract sınırları referans alındı.
  - Kanıt: `JARVIS_Master_Architecture_v2.3_final.pdf`
  - Sonuç: Yeni geliştirmeler implementation baseline üzerinden ilerliyor.

- [x] Temiz Rust core crate oluşturuldu.
  - Yapılan: `jarvis/Cargo.toml`, `src/lib.rs`, `src/main.rs` oluşturuldu.
  - Testler: `cargo test`
  - Sonuç: 3 test geçti.

---

## 1. İlk dikey request pipeline

Durum: DEVAM EDİYOR

- [x] 1.1 Typed `Request` contractı
  - Yapılan: schema version, request id, input type ve content alanları tanımlandı.
  - Testler: `cargo test`
  - Sonuç: Request runtime’a alınabiliyor.

- [x] 1.2 Typed `Task` contractı ve task state’leri
  - Yapılan: QUEUED, RUNNING, WAITING_FOR_USER, CANCELLED, COMPLETED, FAILED ve INTERRUPTED state’leri eklendi.
  - Testler: `cargo test`, cancel/recovery unit testleri.
  - Sonuç: Bekleyen approval task’ı yan etki başlamadan CANCELLED durumuna geçebiliyor; restart’ta RUNNING task ise INTERRUPTED olur.

- [x] 1.3 Intent/capability sınıflandırmasının ilk sürümü
  - Yapılan: `system.health`, `system.time`, `file.read_workspace`, `project.info`, `note.create` ve bilinmeyen istek ayrımı eklendi.
  - Testler: health, time, workspace-read, project-info, approval ve unknown request unit testleri.
  - Sonuç: Deterministic fast path çalışıyor; belirsiz isteklerde local-model fallback yalnız registry’deki tam capability ID’sini kabul ediyor.

- [x] 1.4 Policy Gate’in ilk sürümü
  - Yapılan: düşük riskli health için ALLOW, kalıcı not için ASK_USER, bilinmeyen capability için DENY.
  - Testler: `persistent_note_requires_approval`, `unknown_request_is_denied`.
  - Sonuç: Policy sonucu tool execution’dan önce uygulanıyor; bypass yolu yok.

- [x] 1.5 Typed `ToolResult` contractı
  - Yapılan: status, output, error, state_changed ve evidence alanları eklendi.
  - Testler: health execution testi.
  - Sonuç: Tool sonucu yapılandırılmış biçimde dönüyor.

- [x] 1.6 İlk deterministic tool: `system.health`
  - Yapılan: read-only health sonucu ve `health-check:ok` evidence üretildi.
  - Testler: `health_uses_fast_path_and_verifies`.
  - Sonuç: Tool başarılı çalışıyor.

- [x] 1.7 Typed `VerifierResult` contractı
  - Yapılan: PASS, FAIL, UNCERTAIN durumları ve evidence kontrolü eklendi.
  - Testler: health PASS, approval/deny FAIL senaryoları.
  - Sonuç: Tool success ile hedef success ayrıldı.

- [x] 1.8 İlk audit event akışı
  - Yapılan: task queued, policy kararı, tool execution, verification, blocked, approval ve cancellation eventleri runtime’da ve SQLite’da kaydediliyor.
  - Testler: persistence, approval, cancellation ve runtime testleri.
  - Sonuç: İlk append akışı kalıcı; kriptografik bütünlük zinciri henüz uygulanmadı.

- [x] 1.9 CLI smoke akışı
  - Yapılan: CLI’dan satır bazlı request alınıyor ve task/tool/verifier sonucu gösteriliyor.
  - Testler: `printf ... | cargo run --quiet`
  - Sonuç: health tamamlandı, note approval bekledi, unknown deny edildi.

- [x] 1.10 İlk approval/resume akışı
  - Yapılan: task-bound approval kaydı, `approve task-id` CLI komutu ve tek kullanımlık resume akışı eklendi.
  - Testler: approval resume, unknown/completed task reddi ve replay reddi.
  - Sonuç: Onaylanan task kontrollü şekilde devam ediyor; geniş scope veya tekrar kullanım yok.
  - Son güncelleme: expiry ve scope_hash doğrulaması da eklendi.

---

## 2. Persistence ve schema güvenliği

Durum: DEVAM EDİYOR

- [x] 2.1 SQLite bağlantısı ve ilk migration altyapısı
  - Yapılan: `rusqlite` bundled SQLite kullanıldı; task ve audit tabloları oluşturuluyor.
  - Testler: `sqlite_store_persists_task_and_audit`, `cargo test`.
  - Sonuç: In-memory SQLite schema başarıyla açılıyor.

- [x] 2.2 Task persistence ilk sürümü
  - Yapılan: Store-enabled Runtime task state’i SQLite `tasks` tablosuna yazıyor; CLI `jarvis.db` kullanıyor.
  - Testler: task count assertion, CLI smoke + SQLite row count, `cargo test`.
  - Sonuç: İlk dikey request sonrası task kalıcı store’a yazılıyor.

- [x] 2.3 Approval persistence ilk sürümü
  - Yapılan: approval_id, task_id, action_id, approved, expires_at ve scope_hash SQLite’a yazılıyor.
  - Testler: approval runtime testleri ve SQLite migration testi.
  - Sonuç: Approval kaydı task’a bağlanıyor; süre ve scope değiştirilemezliği kontrol ediliyor.

- [x] 2.4 Audit persistence ilk sürümü
  - Yapılan: queued, policy, tool ve verify olayları SQLite `audit_events` tablosuna yazılıyor.
  - Testler: audit count assertion, CLI smoke + SQLite row count, `cargo test`.
  - Sonuç: İlk akışın dört audit olayı kalıcı store’a yazılıyor.

- [x] 2.5 `schema_version` strict validation ilk sürümü
  - Yapılan: schema version, request id ve boş content parse aşamasında reddediliyor.
  - Testler: `invalid_request_is_rejected_before_policy_and_tool`.
  - Sonuç: Geçersiz request Policy ve Tool katmanına ulaşmıyor.
- [x] 2.6 Idempotent migration ilk sürümü
  - Yapılan: `schema_migrations` tablosu eklendi; approval/audit kolonları önce şema üzerinden kontrol edilip yalnız eksikse ekleniyor. v2’de `teacher_examples`, v3’te SHA-256 audit-chain kolonları eklendi.
  - Testler: SQLite schema version assertion, teacher-example persistence/rejection testleri, `cargo test`.
  - Sonuç: Mevcut database’te tekrar açılış migration hatası veya sessiz kolon kaybı üretmiyor.
- [x] 2.7 İlk crash-recovery testleri
  - Yapılan: SQLite’da `RUNNING` kalan task’lar açılışta `INTERRUPTED` durumuna alınıyor.
  - Testler: `sqlite_recovery_marks_running_task_interrupted`, `runtime_startup_recovers_interrupted_task_state`.
  - Sonuç: Restart sonrası yarım task başarılı varsayılmıyor; recovery idempotent.

Tamamlanma ölçütü: Restart sonrası task, approval ve audit kayıtları güvenli şekilde okunmalı; sessiz data loss olmamalı.

### ADR-001 — Doğrulanmış eğitim örnekleri için ayrı SQLite tablosu

- Durum: Kabul edildi (13 Ağustos 2026).
- Karar: Eğitim adayları mevcut audit kaydına gömülmek yerine versioned `teacher_examples` tablosunda tutulur.
- Neden: Örnek kabulü için verifier evidence, provenance, human review ve sensitivity metadata’sını ayrı ve sorgulanabilir biçimde zorunlu kılmak.
- Migration etkisi: `schema_migrations` v2 idempotent olarak `teacher_examples` tablosunu oluşturur. Mevcut task/audit/approval kayıtları değişmez.
- Test planı: başarılı verified/reviewed kayıt, failed verifier, review eksikliği, registry dışı capability ve schema version kontrolleri unit testte kapsandı.
- Rollback: Kod v1’e geri alınırsa eski binary yeni tabloyu kullanmaz; mevcut data silinmez. Mantıksal rollback, v2’yi okumayan sürümde training intake’in devre dışı bırakılmasıdır; fiziksel tablo silme yalnız açık bakım/backup prosedürüyle yapılabilir.

### ADR-002 — SQLite audit bütünlüğü için SHA-256 hash-chain

- Durum: Kabul edildi (13 Ağustos 2026).
- Karar: `audit_events` sıralı event sequence, previous hash ve SHA-256 event hash saklar; runtime açılışında zinciri kontrol eder.
- Neden: Task/policy/tool/verifier olaylarının sessizce değiştirilmesini local persistence katmanında görünür kılmak.
- Migration etkisi: v3 üç audit kolonu ekler. Eski, zincirsiz olaylar yalnız bir kez deterministik olarak backfill edilir; zaten hash’i olan zincir otomatik olarak yeniden yazılmaz.
- Test planı: normal chain doğrulama, event tampering tespiti, mevcut `jarvis.db` ile CLI startup smoke ve tüm regression suite çalıştırıldı.
- Rollback: Eski binary v3 kolonlarını görmezden gelir; kayıt silinmez. Güvenli rollback, v3 verification yapan sürümü kullanmaya devam etmek veya snapshot’tan restore etmektir; hash kolonları fiziksel olarak kaldırılmaz.

---

## 3. Capability Registry ve güvenli tool runtime

Durum: DEVAM EDİYOR

- [x] 3.1 Versioned `CapabilityManifest` ilk sürümü
  - Yapılan: capability id/version/risk/effect/network/sandbox/verifier metadata’sı ve baseline registry eklendi.
  - Testler: `manifests_describe_supported_capabilities`, `registry_contains_only_baseline_capabilities`.
  - Sonuç: Runtime yalnız kayıtlı capability’leri policy/execution aşamasına alıyor.
- [x] 3.2 `system.health` manifesti
  - Yapılan: read-only, düşük risk, network-off ve health verifier metadata’sı kaydedildi.
  - Testler: `manifests_describe_supported_capabilities`, `default_runtime_keeps_baseline_registry`.
  - Sonuç: Varsayılan Runtime baseline manifest registry ile başlıyor; `system.time` de aynı güvenli read-only profili kullanıyor.
- [x] 3.3 `note.create` capability’si ilk sürümü
  - Yapılan: yalnız approval sonrası `notes/<task-id>.md` oluşturuluyor; path task id’den güvenli biçimde üretiliyor.
  - Testler: approval resume + verifier PASS; CLI smoke.
  - Sonuç: Kalıcı dosya değişikliği policy ve approval arkasında.
- [x] 3.4 Workspace-root containment/path traversal kontrolü ilk sürümü
  - Yapılan: note root canonicalize ediliyor; üretilen dosya root containment ile sınırlandırılıyor.
  - Testler: approval note creation, verifier file existence ve traversal-like request id edge case.
  - Sonuç: Note capability’si approval sonrası belirlenen root dışına yazmıyor; gerçek dosya kanıtı verifier tarafından kontrol ediliyor.
- [x] 3.5 NO_EXEC/READ_ONLY sandbox profili (in-process enforcement)
  - Yapılan: `system.health`, `system.time`, `file.read_workspace` ve `project.info` dispatch öncesinde manifestte `NO_EXEC_READ_ONLY` zorunlu kılındı.
  - Testler: manifest profile mismatch sonucu execution FAIL testi.
  - Sınır: Bu OS process sandbox’ı değildir; ayrı worker/namespace izolasyonu sonraki katmandır.
- [x] 3.6 LOCAL_RESTRICTED sandbox profili (in-process enforcement)
  - Yapılan: Kalıcı `note.create` yalnız approval sonrası manifestte `LOCAL_RESTRICTED` ise dispatch ediliyor.
  - Testler: note profile mismatch sonucu approval sonrası FAIL testi.
- [ ] 3.7 Timeout, cancellation ve cleanup
  - [x] Model fallback process’i için 30 sn hard timeout ve `SIGKILL` eklendi.
  - [x] Approval bekleyen task için cancellation eklendi; pending input temizleniyor, approval görünmez oluyor ve audit’e `task.cancelled` yazılıyor.
  - [ ] Asenkron tool worker’ı, çalışmakta olan tool’un gerçek iptali ve cleanup handle’ları henüz yok.
- [ ] 3.8 Tool retry/idempotency kuralları
- [ ] 3.9 Backup/checkpoint ve dry-run contractı
  - [x] SQLite `VACUUM INTO` ile transaction-consistent snapshot API’si eklendi; mevcut snapshot’ın üzerine yazmak reddediliyor.
  - Testler: snapshot açılıp task/audit sayıları doğrulandı; overwrite denemesi başarısız oldu.
  - [ ] Kullanıcıya sunulan backup command, retention ve dry-run semantics henüz yok.

Tamamlanma ölçütü: En az üç düşük riskli capability manifest, policy, execution ve verifier zincirinden geçmeli.

---

## 4. Local model adapter ve routing

Durum: DEVAM EDİYOR

- [x] 4.1 Provider-neutral `ModelProvider` contractı ilk sürümü
  - Yapılan: provider/model metadata, completion sonucu ve hata contractı eklendi.
  - Testler: `model_provider_contract_returns_structured_metadata_without_authority`.
  - Sonuç: Model katmanı policy/tool authority taşımadan core’a bağlanabiliyor.
- [x] 4.2 Local runtime hazırlığı ve model artifact doğrulaması
  - Yapılan: `models/Qwen3-8B-Q4_K_M.gguf` indirildi; SHA-256 doğrulandı: `d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785`.
  - Yapılan: llama.cpp CPU-only derlendi; `llama-cli` ve `llama-server` hazır.
  - Çalıştırma profili: `-ngl 0` (GPU katmanı yok, CPU/RAM).
  - Testler: Model load smoke testi; llama.cpp GPU katmanlarının kapalı olduğunu doğruladı.
  - Sonuç: Model artifact ve CPU runtime hazır. Kısa üretim CPU’da yüksek yük/yavaşlık gösterebildiği için kapsamlı latency benchmarkı ayrıca yapılacak.
- [x] 4.2b llama.cpp binary’sini Rust `ModelProvider` adapterına bağlama
  - Yapılan: `LlamaCliProvider` eklendi; CPU-only `-ngl 0`, 4-thread/512-context/8-token düşük kaynak profili, single-turn ve child-stdin isolation uygulandı.
  - Testler: missing runtime/model güvenlik testi; gerçek Qwen routing smoke; `cargo test`.
  - Sonuç: Qwen3 belirsiz Türkçe isteği `system.time` olarak route etti; local modelden gelen `note.create` önerisi Policy tarafından approval beklemeye alındı.
- [x] 4.3 Structured model response metadata ve strict intent output
  - Yapılan: `ModelResponse` text, optional structured JSON ve finish reason alanları eklendi.
  - Testler: model provider contract testi.
  - Sonuç: Model yalnızca tam eşleşen, Registry’de kayıtlı capability kimliği üretebildiğinde routing önerisi kabul ediliyor.
- [x] 4.4 Invalid model output/tool hallucination handling ilk sürümü
  - Yapılan: Registry dışı veya exact capability olmayan model çıktısı `unknown` olarak reddediliyor.
  - Testler: `local_model_can_route_only_registered_exact_capabilities`.
  - Sonuç: `shell.exec --unsafe` benzeri model çıktısı tool çağrısına dönüşmüyor.
- [x] 4.5 Deterministic fast path ile model path ayrımı
  - Yapılan: Bilinen health/time/note ifadeleri modele gitmiyor; yalnız deterministic route `unknown` olduğunda local model çağrılıyor.
  - Testler: deterministic ve local model source testleri; gerçek `zaman nedir` CLI smoke.
  - Sonuç: Fast path korunuyor, model yalnız fallback router olarak kullanılıyor.
- [x] 4.6 Model health, loading ve resource state
  - Yapılan: İlk `LlamaCliProvider` runtime kontrolüne ek olarak persistent `LlamaServerProvider` loopback health kontrolü eklendi; TUI modelin hazır/yükleniyor durumunu gösteriyor.
  - Testler: missing executable/model unit testleri, servis `/health` smoke ve TUI startup smoke.
  - Sonuç: `ready`, local server health endpointinin erişilebilir olduğunu ifade eder. Model RAM'de kalıcıdır; `exit` bu servisi kontrollü biçimde durdurur.
- [x] 4.7 İlk Jarvis routing benchmark baseline’ı
  - Yapılan: `router_benchmark` binary’si eklendi; belirsiz Türkçe time/health/note isteklerini expected capability ile karşılaştırıyor ve latency ölçüyor.
  - Testler: gerçek Qwen CLI smoke: `zaman nedir → system.time`, `bilgisayarım iyi çalışıyor mu → system.health`, `alışveriş için not yaz → note.create → approval`.
  - Son smoke ölçümü: CPU-only, 4 thread, context 512, 8 token; üç gerçek Türkçe routing örneği 3/3 PASS, ortalama 4.324 sn (4.143–4.432 sn).
  - Not: `-st` (single-turn) ve child stdin isolation zorunlu; aksi halde CLI interactive moda dönüp sınırsız çıktı üretebilir veya ana CLI girdisini tüketebilir.
  - Sonuç: Dar MVP routing baseline’ı kanıtlandı. Coding/security/uzun-context benchmarkları sonraki benchmark genişletmesinde eklenecek.

Model edinme durumu: İlk model artifact’i indirildi, doğrulandı ve CPU adapter/routing baseline’ı geçti. Model lifecycle preload/unload ve uzun süreli resource metrikleri sonraki sürümün konusudur.

Tamamlanma ölçütü: Model hiçbir policy kararını tek başına verememeli; geçersiz model çıktısı güvenli biçimde reddedilmeli.

---

## 5. RAG, workspace ve memory

Durum: BEKLENİYOR

- [ ] 5.1 ContentRef typed union
- [x] 5.1 ContentRef typed union (ilk sürüm)
  - Yapılan: source, provenance ve content alanlarıyla typed `ContentRef` eklendi.
- [x] 5.2 WorkspaceRef provenance/trust metadata (ilk sürüm)
  - Yapılan: workspace dosyası varsayılan `UntrustedProjectFile` olarak işaretleniyor; kullanıcı talimatı sayılmıyor.
- [ ] 5.3 Read-only document ingestion
  - [x] İlk dilim: çalışma dizini içinden path-contained, UTF-8 ve 64 KiB sınırına sahip tek dosya okuma (`file.read_workspace`) ve `project.info` eklendi.
  - Testler: başarı, verifier evidence ve `../` path traversal reddi.
  - [ ] Çoklu belge indeksleme, ContentRef/provenance ve retrieval henüz yok.
- [x] 5.4 Prompt-injection isolation testleri (ilk sürüm)
  - Kanıt: injected instruction içeren content yalnız `<untrusted-content>` data envelope’unda tutuluyor.
- [ ] 5.5 Secret scanning ve hassas dosya exclusion
- [ ] 5.6 Session/project/task memory ayrımı
- [ ] 5.7 Memory write/usefulness/sensitivity policy
- [ ] 5.8 SQLite metadata-first retrieval

Tamamlanma ölçütü: Repo/PDF/web içeriği bilgi olarak kullanılabilmeli ancak kullanıcı talimatı veya tool yetkisi sayılmamalı.

---

## 6. Teacher–Student learning ve dataset governance

Durum: DEVAM EDİYOR

- [x] 6.1 `TeacherExample`/dataset record schema
  - Yapılan: versioned example id, prompt, expected registered capability, response, verifier status, evidence, provenance, human-review ve sensitivity alanları eklendi.
  - Testler: kabul edilen örneğin SQLite’a yazılması.
- [x] 6.2 Tool/evidence/test/verifier provenance kaydı (intake contract)
  - Yapılan: `PASS` verifier status, boş olmayan evidence ve provenance olmadan dataset kaydı kabul edilmiyor.
  - Testler: failed verifier ve eksik/uygunsuz kayıt reddi.
- [x] 6.3 Human review ve sensitivity işaretleri (intake contract)
  - Yapılan: human_reviewed ve PUBLIC/INTERNAL/SENSITIVE metadata’sı zorunlu typed alana alındı; review yoksa kayıt reddediliyor.
  - Testler: unreviewed example rejection.
- [ ] 6.4 Dataset versioning
- [ ] 6.5 Deletion marker ve poisoned-example removal
- [ ] 6.6 Teacher çıktısı için tekrar doğrulama
- [ ] 6.7 Baseline model benchmark’ı
- [ ] 6.8 İlk LoRA/QLoRA deneyi
- [ ] 6.9 Old-vs-new regression ve rollback testi

Kural: Verification’dan geçmeyen veya provenance’ı eksik örnek eğitim datasına giremez.

Not: Bu yalnız güvenli dataset intake temelidir; otomatik teacher çağrısı, dataset export/versioning ve LoRA/QLoRA eğitimi henüz başlatılmadı.

---

## 7. Yetkili security/pentest capability’leri

Durum: DEVAM EDİYOR

- [x] 7.1 Machine-readable `PentestScope`
  - Yapılan: schema version, authorization reference, allowlist/exclusion listesi, expiry, maximum mode ve runtime limit typed contract’a alındı.
  - Sınır: authorization reference imza/kurum doğrulaması henüz yapmaz; bu yüzden gerçek security capability henüz yoktur.
- [x] 7.2 Target allowlist/excluded target kontrolü
  - Yapılan: exact canonical ASCII host/IP eşleşmesi, exclusion-first ve scope expiry kontrolü eklendi.
  - Testler: allowlisted target geçiyor; excluded veya scope dışı target reddediliyor.
- [ ] 7.3 CIDR, wildcard, punycode ve DNS rebinding testleri
  - [x] İlk dar kural: wildcard, CIDR, Unicode/punycode target’lar reddediliyor.
  - [ ] CIDR semantiği ve DNS resolution/pinning ile rebinding savunması henüz yok.
- [ ] 7.4 Network egress enforcement
- [x] 7.5 SAFE/ACTIVE/INTRUSIVE/DESTRUCTIVE sınıfları
  - Yapılan: requested mode, scope maximum mode’u aşarsa işlem authorization katmanında reddediliyor.
  - Testler: ACTIVE scope altında INTRUSIVE escalation reddi.
- [ ] 7.6 Rate/runtime limitleri
- [ ] 7.7 Evidence tabanlı finding formatı
- [ ] 7.8 Security tool sandbox worker’ı
- [ ] 7.9 Scope dışı hedef için deny testleri

Tamamlanma ölçütü: Kullanıcının sözlü yetki iddiası tek başına yeterli olmamalı; scope runtime tarafından enforce edilmeli. Mevcut contract yalnız ilk adımdır; imzalı authorization ve network enforcement eklenmeden security tool açılmayacak.

---

## 8. Remote device trust ve task handoff

Durum: BEKLENİYOR

- [ ] 8.1 Device identity ve public key kayıtları
- [ ] 8.2 Explicit pairing
- [ ] 8.3 Key rotation/revoke/expiry
- [ ] 8.4 Nonce/sequence/timestamp replay protection
- [ ] 8.5 TaskHandoff contractı
- [ ] 8.6 Permission scope genişletme engeli
- [ ] 8.7 Offline queue ve conflict policy
- [ ] 8.8 Server-side kill switch

Tamamlanma ölçütü: Yeni veya revoke edilmiş cihaz trusted capability kullanamamalı; handoff yeni yetki yaratmamalı.

---

## 9. MCP vertical slice

Durum: DEVAM EDİYOR

- [x] 9.1 Typed MCP ingress adapterı (core)
  - Yapılan: `McpIngressRequest` eklendi; transport/JSON-RPC katmanı dışarıda tutularak local core’a typed giriş sağlandı.
- [x] 9.2 MCP capability manifest mapping (allowlist)
  - Yapılan: yalnız `jarvis.system.health`, `jarvis.system.time`, `jarvis.file.read_workspace`, `jarvis.project.info` ve `jarvis.note.create` map ediliyor.
  - Testler: bilinmeyen `jarvis.shell.exec` tool ID’si deny ediliyor.
- [x] 9.3 MCP → Policy → Task → Tool → Verifier akışı (core)
  - Yapılan: MCP ingress ayrı bir shortcut yaratmıyor; normal typed Request/Policy/Task/Tool/Verifier zincirini kullanıyor.
  - Testler: MCP health tamamlanıyor; MCP note yine WAITING_FOR_USER; invalid schema execution’dan önce reddediliyor.
- [ ] 9.4 Secret/raw credential response engeli
- [ ] 9.5 Untrusted MCP output provenance
- [x] 9.6 MCP policy bypass security testleri (core)
  - Kanıt: unknown tool deny, invalid schema reject ve note approval testleri.

---

## 10. Observability, audit integrity ve recovery

Durum: BEKLENİYOR

- [ ] 10.1 Correlation/task/device ID’leri
- [x] 10.2 Structured logger (MVP)
  - Yapılan: timestamp, level, correlation_id, task_id ve event alanlarıyla in-memory structured log eklendi; audit eventinden türetiliyor.
- [x] 10.3 Append-only audit API (core)
  - Yapılan: runtime audit eventleri tek `record_audit` yolu üzerinden sequence/previous-hash/event-hash atanarak SQLite’a ekleniyor.
  - Sınır: Database dosyasına işletim sistemi düzeyinde yazma yetkisi olan bir saldırgana karşı immutable storage/remote witness henüz yok.
- [x] 10.4 SHA-256 hash-chain bütünlük kanıtı
  - Yapılan: Her audit olayı canonical alan uzunluklarıyla SHA-256 hash’lenip önceki olaya bağlanıyor; startup’ta zincir doğrulanmadan runtime açılmıyor.
  - Testler: `sqlite_audit_hash_chain_detects_event_tampering`; event metni değiştirildiğinde chain invalid oluyor.
- [ ] 10.5 Retention/tombstone politikası
- [ ] 10.6 Resource/latency/success/verifier metrikleri
- [x] 10.7 SQLite snapshot ve transaction-consistent backup (ilk sürüm)
  - Kanıt: `sqlite_backup_is_consistent_and_never_overwrites` unit testi.
- [x] 10.8 RUNNING task recovery → INTERRUPTED (MVP)
  - Kanıt: SQLite startup recovery testleri; RECOVERING worker semantics F4/F9 kapsamındadır.
- [ ] 10.9 Model/dataset/config rollback

---

## 11. Test ve kalite kapısı

Durum: DEVAM EDİYOR

- [x] 11.1 İlk unit test seti
  - Kanıt: 3 test geçti (`cargo test`).
- [x] 11.2 İlk CLI smoke testi
  - Kanıt: health, approval bekleme ve deny çıktıları gözlemlendi.
- [x] 11.3 Contract tests (baseline)
  - Kanıt: `baseline_capability_contracts_keep_manifest_and_policy_in_sync` manifest ID/version/risk/sandbox profile ile policy risk/decision/required-control sözleşmesini beş capability için doğruluyor.
- [x] 11.4 Policy bypass security tests (ilk dilim)
  - Kanıt: Registry dışı model çıktısı reddi; modelden gelen `note.create` yine approval bekliyor; manifest sandbox profile mismatch execution’ı reddediyor.
- [x] 11.5 Path traversal/secret scope/sandbox tests (ilk dilim)
  - Kanıt: workspace read `../` traversal reddi ve note filename containment testi geçti.
- [ ] 11.6 Concurrency/cancel/lock tests
- [x] 11.7 Persistence/recovery tests
  - Kanıt: SQLite task/audit persistence; RUNNING → INTERRUPTED startup recovery, idempotency ve overwrite-safe snapshot testi geçti.
- [x] 11.8 MCP integration test (stdio MVP)
  - Kanıt: release `mcp_stdio` binary’sinde JSON-RPC initialize/tools-list/tools-call smoke; registry dışı tool deny.
- [x] 11.9 E2E smoke test (CLI approval/cancel)
  - Kanıt: gerçek `cargo run --quiet` oturumunda `not oluştur` WAITING_FOR_USER üretti; `cancel <task-id>` ardından task CANCELLED oldu ve `approvals` boş kaldı.
- [x] 11.10 Regression suite (MVP)
  - Kanıt: 40 unit test, strict Clippy ve release smoke; policy, approval, persistence, audit-chain, RAG isolation, MCP deny ve scope testleri kapsandı.

Tamamlanma ölçütü: Her yeni capability en az route → policy → execution → verifier → audit zincirinde test edilmeli.

---

## 12. MVP release gate

Durum: TAMAMLANDI — 13 Ağustos 2026

- [x] Rust core stabil (desktop CLI baseline)
  - Kanıt: 31 unit test, strict clippy ve optimize edilmiş `cargo build --release` sonrası `status → system health` smoke geçti.
- [x] SQLite task/approval/audit persistence (MVP scope)
  - Kanıt: migration v3, persistence/recovery/backup testleri ve gerçek CLI startup.
- [x] En az 3 güvenli capability
  - Kanıt: health, time, workspace read, project info, coding outline, docs summary ve approval-gated note.
- [x] Gerçek approval/resume
  - Kanıt: task-bound expiry/scope hash/replay reddi ve CLI E2E testleri.
- [x] Local model adapter
  - Kanıt: CPU-only Qwen3 route benchmarkı ve model health status.
- [x] Teacher escalation için güvenli placeholder/adapter
  - Kanıt: private context için approval-required contract testi; bu placeholder cloud provider çağrısı yapmaz.
- [x] İlk MCP vertical slice (stdio JSON-RPC)
  - Sınır: External MCP server registry/discovery ve untrusted response provenance F8 kapsamındadır.
- [x] Development plan güncel
- [x] Kritik security testleri geçiyor (MVP)
  - Kanıt: policy/model/MCP bypass, approval scope/replay, path traversal, sandbox profile, audit tampering ve pentest scope unit testleri.
- [x] Backup/recovery smoke testi geçiyor (core API)
  - Kanıt: in-memory store → SQLite snapshot → yeniden açma ile task/audit doğrulandı; RUNNING recovery testleri geçti.

MVP tamamlanma kararı yalnızca bütün zorunlu maddeler ve test kanıtları görüldükten sonra verilir.

---

## Güncel durum özeti — 13 Ağustos 2026

Desktop MVP tamamlandı. Sistem CLI/HUD/voice-transcript veya MCP stdio isteğini typed request olarak alıyor, deterministic/local-model
routing yapıyor, policy kararı veriyor, yedi kayıtlı capability’yi controlled runtime’da çalıştırıyor, evidence/verifier sonucu çıkarıyor,
task state güncelliyor ve SQLite’a task/approval/audit yazıyor. Structured correlation log, SHA-256 audit chain, zero-trust workspace
content, teacher privacy gate, approval/resume/cancel ve CPU-only local model çalışıyor. Gerçek sandbox worker, derin retrieval/memory,
eğitim/fine-tuning, advanced pentest ve mobile/remote F3–F8 kapsamındadır.

Son doğrulama komutları:

```text
cargo fmt
cargo test
printf 'system health\ndosya oku: Cargo.toml\nnot oluştur\nunknown\nexit\n' | cargo run --quiet
```

Sonuç: 51 test geçti; tool intentleri deterministic/policy-gated, serbest doğal sohbet ise local modelin bounded session-history taşıyan, native user/assistant rollü data-only conversation path’inden yürür. Qwen3-8B CPU-only çalışır; sohbet çıktısı tool veya policy authority kazanmaz ve reasoning kapalıdır. SQLite migration v3, restart recovery, overwrite-safe snapshot, SHA-256 audit-chain, correlation log, zero-trust workspace content, teacher privacy gate, MCP stdio transportu, coding/docs ve HUD/voice basics kanıtlandı. İlk platform Linux-first desktop terminal UI’dır.

### Tarihsel MVP sonrası rota — ana faz haritası tarafından kapsandı

1. **RAG’i gerçek retrieval’a taşı:** ContentRef provenance, hassas dosya exclusion, SQLite metadata index ve context budget.
2. **Coding agent güvenliği:** isolated worker içinde test/check/diff üretme; patch preview + explicit approval + verifier.
3. **Training governance:** dataset export/versioning, review kuyruğu, benchmark harness; ardından küçük LoRA/QLoRA deneyi ve rollback.
4. **Pentest readiness:** imzalı scope manifest, CIDR/DNS/egress enforcement ve network-scoped worker; ancak sonra SAFE capability’ler.
5. **Operasyonel sertleştirme:** gerçek timeout/cancel worker’ı, backup command/retention, metric dashboard ve audit witness/export.

Bu ilk taslak rota, yukarıdaki F2–F9 ana faz haritasına taşındı. Her yeni faza geçmeden önce MVP'nin güvenlik/kalite gate'i yeniden çalıştırılır; fine-tuning ve advanced pentest aynı anda başlatılmaz.
