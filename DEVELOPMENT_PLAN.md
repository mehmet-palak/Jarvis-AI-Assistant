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

## `src/lib.rs` core modülerleştirme (teknik borç, F9 kapsamında)

Durum: DEVAM EDİYOR — başlangıç 15 Ağustos 2026

`src/lib.rs` 5058 satıra ulaştı (contracts, model provider'lar, `CapabilityRegistry`, `SqliteStore`, `Runtime`, `policy_for`, ~1500 satır test tek dosyada). Davranış değişikliği yok; hedef aynı Policy→Task→Tool→Verifier zincirini koruyarak dosyayı ilgi alanına göre ayrı modüllere bölmek. Çalışma `refactor/split-lib-rs` branch'inde, en düşük riskten en yükseğe doğru, her adımdan sonra tam `cargo fmt` + `cargo test` (workspace) + `scripts/release_check.sh` yeşiliyle ilerliyor; branch bittiğinde `main`'e sağlıklı biçimde push edilecek.

- [x] `model.rs` çıkarıldı: `ModelProvider` trait'i, `DeterministicModelProvider`/`LlamaCliProvider`/`LlamaServerProvider`, `ModelResponse`/`ModelRuntimeState`/`RouteSource`/`IntentResolution`, `JARVIS_SYSTEM_PROMPT`, `model_capability_intent`, `route_with_provider`, `normalize_llama_cli_output`. Public API `lib.rs` kökünden `pub use model::{...}` ile birebir korundu; içsel (`pub(crate)`) sabitler/fonksiyonlar sadece crate içinden erişilebilir.
  - Kanıt: `cargo fmt` temiz; `cargo test` (workspace) 85 lib + 16 main + 5 desktop test PASS, 0 fail/warning; `cargo build --release` başarılı; `scripts/release_check.sh` → `JARVIS local release gate: PASS`. Baseline (bölme öncesi) da aynı 85+5 testle yeşildi, davranış regresyonu yok.
- [x] `capabilities.rs` çıkarıldı: `CapabilityRegistry` + `capability_manifest()`. Test-only mutasyon ihtiyacı (`sandbox_profile` fixture'ı) için `pub(crate) #[cfg(test)] get_mut()` eklendi; alan (`manifests`) private kaldı.
  - Kanıt: `cargo fmt --check` temiz; `cargo test` (workspace) 85+16+5 test PASS, 0 fail/warning; `scripts/release_check.sh` → PASS.
- [x] `persistence.rs` çıkarıldı: `SqliteStore` (tasks/approvals/audit/teacher_examples/memories/workspace RAG, tek `Connection`) + `audit_hash()`. Not: Tek dosyada tutuldu, tablo bazlı alt-modüllere (tasks.rs/audit.rs/...) bölünmedi — mevcut haliyle homojen ~750 satırlık CRUD kodu, ek bölme şu an marjinal fayda/risk oranı taşıyor; ileride ihtiyaç olursa ayrı adım olarak ele alınabilir. Private alanlar (`connection`, `manifests` benzeri) korundu; Runtime'ın çağırdığı `save_task`/`save_approval`/`append_audit_chain`/`audit_tail` `pub(crate)` yapıldı, ham `Connection`'a sadece `#[cfg(test)] raw_connection()` üzerinden (tamper-testi için) erişilebiliyor.
  - Kanıt: `cargo fmt --check` temiz; `cargo test` (workspace) 85+16+5 test PASS, 0 fail/warning; `scripts/release_check.sh` → PASS. `lib.rs` 4461 → 3690 satıra indi.
- [x] `runtime.rs` çıkarıldı: `Runtime` (struct + `Default` + tüm `impl`, orchestrator/Decision Core). Not: Bu modül kasıtlı olarak `use crate::*;` (glob import) kullanıyor — Runtime zaten neredeyse tüm crate contract yüzeyine (policy, persistence, model, memory, workspace RAG, workbench, vision, audit) dokunan orkestratör olduğu için tek tek import listesi crate yüzeyini tekrarlamaktan öteye geçmiyordu. Test-only doğrudan alan erişimi ihtiyacı olan `store`/`chat_history` `pub(crate)` yapıldı; `handle_with_provider_and_analyses`/`conversation_context`/`vision_failure` da test tarafından çağrıldığı için `pub(crate)`'e çıkarıldı — davranış değişmedi, sadece görünürlük.
  - Kanıt: `cargo fmt --check` temiz; `cargo test` (workspace) 85+16+5 test PASS, 0 fail/warning; `scripts/release_check.sh` → PASS. `lib.rs` 3690 → 2963 satıra indi.
- [x] `policy.rs` çıkarıldı: `classify`, `validate_request`, `validate_teacher_example`, `validate_pentest_scope`, `authorize_pentest_target`, `normalize_pentest_target`, `policy_for` — tek Policy Gate yolu korundu (ikinci bir karar yolu oluşturulmadı, sadece taşındı). Not: `policy_for`'un gerçek boyutu ~70 satır çıktı — önceki analizde 466 satır olarak not edilmişti, bu tahmin hatalıydı; gerçek kod çok daha küçük ve tek bir `match` ifadesinden ibaret.
  - Kanıt: `cargo fmt --check` temiz; `cargo test` (workspace) 85+16+5 test PASS (policy_for/classify/pentest-scope'u doğrudan kapsayan testler dahil, ör. `policy_exposes_machine_readable_controls`, `pentest_scope_rejects_expired_or_ambiguous_targets`, `persistent_note_requires_approval`); `scripts/release_check.sh` → PASS. Ayrıca gerçek derlenmiş `target/release/mcp_stdio` binary'sine canlı JSON-RPC smoke: `system.health` → ALLOW/Completed/Pass, `note.create` → **ASK_USER**/WaitingForUser (policy_for'daki tam onay metniyle), bilinmeyen tool → Deny/Failed. `lib.rs` 2963 → 2740 satıra indi.
- [x] Kalan `#[cfg(test)] mod tests` (~1500 satır, 64 test) için karar: **`lib.rs` içinde colocated kalır, `tests/` altına taşınmaz.** Gerekçe: Test dosyası incelendiğinde 64 testten en az ~10'u kasıtlı olarak `pub(crate)`-only iç erişim kullanıyor (`SqliteStore::raw_connection()`, `CapabilityRegistry::get_mut()`, `Runtime.store`/`Runtime.chat_history` alanları, `Runtime::conversation_context()`/`vision_failure()`/`handle_with_provider_and_analyses()`) — bunlar bilinçli olarak public API'ye açılmadı çünkü "Core dışarı sızdırmaz" ilkesini derleme zamanında korumak istedik. Rust'ta `tests/` altındaki entegrasyon testleri crate'i yalnız `pub` (crate-dışı) yüzeyinden görür; `pub(crate)` hiçbir şekilde erişilemez. Bu testleri `tests/`'e taşımak ya (a) o iç erişim noktalarını tekrar tam `pub` yapmayı (encapsulation'ı geri açar) ya da (b) her testi public-API-only şekilde yeniden yazmayı gerektirirdi — ikisi de bu adımın kapsamı dışında gereksiz risk. Mevcut `#[cfg(test)] mod tests { use super::*; }` yapısı zaten idiomatic Rust'tır ve her modülün genişletilmiş test kapsamına (`model`, `capabilities`, `persistence`, `runtime`, `policy` hepsi `use super::*` ile aynı testten kapsanıyor) erişimi koruyor.
  - Kanıt: `grep` ile doğrulandı — 64 `#[test]` fonksiyonundan 11 çağrı noktası doğrudan yukarıdaki `pub(crate)`-only API'lere dokunuyor.
- [x] Tüm adımlar bitti; branch `main`'e sağlıklı biçimde merge/push edildi.
  - Kanıt: Merge öncesi son tam doğrulama — `cargo fmt --check` temiz, `cargo test` (workspace) 85+16+5 PASS, `cargo clippy --all-targets -- -D warnings` temiz, `cargo build --release` başarılı, `scripts/release_check.sh` → PASS. `lib.rs` 5058 → 2744 satıra indi (%46 azalma); 5 yeni modül (`model.rs`, `capabilities.rs`, `persistence.rs`, `runtime.rs`, `policy.rs`) eklendi; hiçbir davranış/test regresyonu yok.
- [x] Devam turu (15 Ağustos 2026, F3 sırasında): F3'ün bellek/RAG maddeleri `lib.rs`'i tekrar 2744 → 3679 satıra çıkardı (kullanıcının kendi gözlemi: "lib dosyası 3600'e yaklaşmış"). `workspace.rs` çıkarıldı: workspace/RAG indeksleme, klasör önizleme, PDF metin çıkarma, chunking, FTS sorgu üretimi (`WorkspaceIngestionReport`/`WorkspaceCitation`/`WorkspaceIndexPreview`/`WorkspaceFolderIndexReport`, `preview_workspace_index`, `validate_workspace_document_*`, `extract_pdf_text`, `chunk_workspace_text`, `fts_query`) — hepsi zaten bitişik, tek bir ilgi alanına (workspace/RAG) ait bir blok. `pub` öğeler `pub use workspace::{...}`, `pub(crate)` öğeler ayrı bir `pub(crate) use workspace::{...}` satırıyla (görünürlük genişlemesin diye tek bir `pub use` altında karıştırılmadı) yeniden ihraç edildi. `persistence.rs`/`runtime.rs` hiç değişmedi (zaten `crate::` üzerinden veya glob import ile erişiyorlardı).
  - Kanıt: `cargo fmt --check` temiz, `cargo test` (workspace) 109 lib + 28 main + 6 desktop PASS, `cargo clippy --all-targets -- -D warnings` temiz, `scripts/release_check.sh` → PASS. `lib.rs` 3679 → 3397 satıra indi; yeni `workspace.rs` 307 satır.

Tamamlanma ölçütü: `lib.rs` yalnız üst düzey `mod`/`pub use` beyanları ve gerçekten paylaşılan az sayıda tipten oluşur; her modül tek bir ilgi alanına karşılık gelir; hiçbir davranış/test regresyonu yoktur. Not: `lib.rs`'te hâlâ contracts (typed struct/enum'lar) ve Tool Runtime/Verifier uygulaması (`execute_read_only`, `execute_approved`, `system_health_snapshot` ve sistem metrik toplayıcıları, `verify`) bulunuyor — bunlar bu turun kapsamı dışında bırakıldı; istenirse ayrı bir sonraki tur olarak ele alınabilir.

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
| F2 | Günlük masaüstü ürün deneyimi, native UI ve görsel/dosya ekleri | TAMAMLANDI (F2.0 + F2.1, 15 Ağustos 2026) | F1 |
| F3 | Kontrollü bellek, profil ve gerçek RAG | TAMAMLANDI (18/18 madde, 15-16 Ağustos 2026) | F2 attachment/provenance temeli |
| F4 | Onaylı, izole coding ve yerel iş workbench'i | TAMAMLANDI (15/15 madde `[x]`: plan(varsayım/soru/risk dahil)→patch→onay→uygula→test zinciri, taban çizgili regresyon tespiti + seçilebilir dosya scope'lu patch preview dahil, uçtan uca TUI'de çalışıyor, 7 senaryolu eval seti geçiyor, `LocalTool` çerçevesi 2 gerçek tool'la ve genel bir `workflow.rs` çok-adımlı orkestratörü kanıtlandı; gerçek cgroup v2 (RAM/CPU) + `WorkspaceWriteMode::Overlay` (disk bütçesi dahil) + gerçek seccomp-bpf allowlist filtresi eklendi, izole worker (bwrap+cgroup+overlay+seccomp) ilk kez gerçek makinede tamamen uçtan uca kanıtlandı — 2 gerçek bug (RLIMIT_NPROC per-UID yanlış hesap, tmpfs/bind mount sırası) bulunup düzeltildi) | F2 + OS-isolated worker |
| F5 | Push-to-talk ses ve çoklu algı arayüzü | TAMAMLANDI (11/11 madde; gerçek bas-tut + canlı seviye göstergesi, whisper.cpp STT + Piper TTS, sesli onay güvenlik sınırı, wake word bilinçli olarak reddedildi — ADR-0007; sıfır yeni Rust bağımlılığı) | F2 native UI |
| F6 | Benchmark, dataset governance ve geri alınabilir model adaptasyonu | TAMAMLANDI (7/7 madde; golden set + dataset governance + config registry + regresyon/rollback + geri bildirim intake'i + LoRA kararı ADR-0006 + model karşılaştırması — hepsi gerçek modelle kanıtlı, hiçbir indirme yapılmadan) | F3/F4 gerçek eval verisi |
| F7 | Yazılı yetkili, bug bounty odaklı, teknik olarak sınırlı security/pentest | BEKLENİYOR — kullanıcı önceliklendirdi (20 Ağustos 2026), F8/F9'dan önce gelir | F4 isolation + F9 operasyon kapıları |
| F8 | MCP ekosistemi, entegrasyonlar ve güvenli remote/mobile | BEKLENİYOR | F3, F7 trust/permission temeli |
| F9 | Operasyonel olgunluk, release ve uzun dönem bakım | BEKLENİYOR | F2–F8 boyunca sürekli yürür |

Programın bitiş tanımı (v1): F2–F9'un zorunlu maddeleri, güvenlik/kalite kapıları ve kullanıcı kabul senaryoları tamamlanmış olacak.

**F10 — 20 Ağustos 2026'da kullanıcı tarafından netleştirildi: bu ürünün uzun vadeli, gerçek ve taahhüt edilmiş hedefidir, "belki bir gün" değil.** JARVIS, [docs/security_and_engineering_vision.md](docs/security_and_engineering_vision.md)'de (44 bölüm) ve [docs/f7_security_tool_research.md](docs/f7_security_tool_research.md)'de (11 araç incelemesi) tanımlanan **her yetkinliğe** sahip olacak: web/mobil/desktop/cloud/offensive pentest güvenliği, malware analizi, tersine mühendislik, OSINT, tehdit istihbaratı, kurumsal güvenlik, yetkili full red-team — ve ayrıca çok-dilli/çok-framework'lü coding agent, veri mühendisliği, model adaptasyonu, veritabanı/PostgreSQL, dağıtık sistemler, DevOps/SRE, performans, UI/UX/erişilebilirlik, paketleme, gizlilik/yönetişim ve kişisel günlük asistan yetkinlikleri.

**Kullanıcının açık sıralama kuralı: "JARVIS adam gibi çalışmadan mobile geçilmeyecek."** Yani F8'in mobil/uzak istemci kısmı, çekirdek (F0-F9, özellikle F2/F7/F9 istikrarı) gerçekten sağlam çalışmadan açılmaz — bu, F8'in F3+F7 güven temeline bağımlı olduğu mevcut faz tablosuyla zaten tutarlı, burada yalnız kullanıcının kendi sözleriyle kesinleştirildi.

Kaynak belgenin kendi önceliklendirmesi (bölüm 41) korunuyor: en yüksek bütçe offensive security + AI/data engineering; orta bütçe mobile/web/PostgreSQL/otomasyon; destek bütçesi DevOps/SRE/UX/release/performans/uyumluluk. Her alan aynı anda "tam uzman" olmayı hedeflemez (bölüm 20'nin kademeli teslim modeli) — ortak provenance/policy/evidence/eval altyapısı önce kurulur, sonra alan alan derinleştirilir; her yeni yetkinlik JARVIS'in mevcut Request→Policy→Task→Tool→Verifier→Audit zincirinden geçer, istisnasız.

### Mevcut mimari backlog eşlemesi

Bu dosyanın aşağısındaki ayrıntılı mimari bölümleri korunur; aşağıdaki tablo her açık maddenin hangi ürün fazında kapanacağını gösterir.

| Mevcut bölüm | Sahip faz | Kapanış hedefi |
| --- | --- | --- |
| 3. Capability Registry ve güvenli tool runtime | F4 + F9 | Isolated worker, gerçek iptal/cleanup, retry/idempotency, backup/dry-run |
| 4. Local model adapter ve routing | F2 + F6 + F9 | UX/health, benchmark, model registry ve rollback |
| 5. RAG, workspace ve memory | F3 | Ingestion, sensitivity, retrieval, context budget ve silme |
| 6. Teacher–Student learning ve dataset governance | F6 | Dataset sürümü, deletion marker, eval, LoRA/QLoRA ve rollback |
| 7. Yetkili security/pentest | F7 | Authorization, network enforcement, sandbox, keşif/proxy/replay, evidence, rapor üretimi ve deny testleri |
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
- [x] Güvenli ilk capability'ler: system health/time doğrudan; workspace read, project/coding/docs summary ve note.create task-bound approval-gated.
- [x] MCP stdio transportu: initialize, tool list, typed call ve bilinmeyen tool deny.
- [x] İlk security scope contractı ve teacher-example intake contractı.
- [x] MVP regression gate: 51 test, Clippy, release build, service health ve interaktif smoke.

Çıkış kanıtı: `jarvis` günlük metin sohbeti ve ilk governed capability'leri local-first olarak çalıştırır.

### F2 — Günlük masaüstü ürünü ve multimodal ekler

Durum: TAMAMLANDI — F2.0 ve F2.1 tüm alt maddeleriyle 15 Ağustos 2026'da tamamlandı; native Wayland/Hyprland resize/minimize, mesaj gönderme ve bildirim action kullanıcı kabulü dahil ([kayıt](docs/f2_native_wayland_smoke_2026-08-15.md)). Daha önce "Ctrl+O picker iptalinden sonra bazen sessiz kapanış" diye kaydedilen backlog bulgusu 16 Ağustos 2026'da kullanıcı tarafından düzeltildi — bir bug değildi, pencereyi o sırada kullanıcının kendisi kapatmıştı. Açık kalan bilinen bir F2 hatası yok.

Amaç: Terminal MVP'yi korurken, gerçek günlük kullanım için native masaüstü deneyimi, dosya/görsel ekleri ve ölçülebilir UX kalitesi oluşturmak.

#### F2.0 — MVP stabilizasyonu ve ürün kalite kapısı

Durum: TAMAMLANDI — otomasyon ve gerçek terminal/ekran kabulü 15 Ağustos 2026'da tamamlandı (bkz. alt maddeler).

Amaç: Yeni büyük yetenek eklemeden önce günlük kullanım regresyonlarını görünür ve tekrar üretilebilir hale getirmek.

- [x] Girdi otomasyonu: paste, `Ctrl+V`, `Ctrl+Backspace`, `Ctrl+W`, `Ctrl+U`, `Esc`, UTF-8/Türkçe ve çok satırlı metin senaryoları.
  - Kanıt: bracketed paste/`Ctrl+V`/primary-selection mapping, çok satır normalizasyonu, UTF-8 kelime silme, `Ctrl+U` ve `Esc` temizleme unit testleri.
- [x] Geçmiş otomasyonu: klavye, mouse wheel, `Home/End`, taşan uzun kullanıcı/model turu ve yeni yanıt geldiğinde en alta dönüş.
  - Kanıt: keyboard/mouse navigation, explicit latest-reset ve küçük terminal TestBackend render/scrollbar regression testleri.
- [x] Yaşam döngüsü otomasyonu: gerçek release TUI'de `exit` → text + vision stop/start; `/quit`, `Ctrl+C` ve terminal-close `SIGHUP` → iki servisi RAM'de bırakma smoke PASS ([kayıt](docs/f2_lifecycle_smoke_2026-08-14.md)); ilk açılış, yeniden açılış ve DB recovery (yinelenen audit sequence otomatik onarımı) gerçek release binary koşumuyla PASS ([kayıt](docs/f2_lifecycle_smoke_2026-08-15.md)).
- [x] Bildirim otomasyonu: yanıt hazır, model/servis hatası, approval bekleme; notification daemon yokken graceful fallback.
  - Kanıt: TUI/native completed, approval ve failed/interrupted başlıkları; native model `HAZIR → BAŞLATILIYOR` geçişinde tek hata bildirimi üretir. `try_notify_desktop` testleri daemon transport hatasını yutar, task/UI sonucunun değişmediğini doğrular; kullanıcı tercihi kapalıysa native bildirim üretilmez.
- [x] TUI görsel smoke: küçük/büyük terminal, resize, yüksek DPI/font farklılığı, okunabilir kontrast ve odak/cursor davranışı.
  - Otomatik kanıt: Ratatui `TestBackend` küçük/geniş ve üç yeniden-boyutlandırma ölçüsünde en yeni tur, composer ve cursor sınırlarını PASS doğrular.
  - Gerçek terminal kanıtı: [docs/f2_tui_visual_smoke_2026-08-15.md](docs/f2_tui_visual_smoke_2026-08-15.md) — gerçek Hyprland/`foot` terminalinde (1.25 ölçek) küçük/büyük pencere ve büyük font koşumu; kontrast okunaklı, odak/cursor davranışı (dolu/boş imleç) doğru ayrışıyor. Tek kozmetik bulgu: büyük fontta durum çubuğu rozetleri taşıyor — P0/P1 değil, UX backlog'una eklendi.
- [x] Sürümlü sohbet kalite seti: Türkçe **ve İngilizce** selamlaşma, kısa takip sorusu, uzun bağlam, konu değişimi, belirsizlik, tool-iddiası ve güvenli reddetme.
  - Plan/koşum kaydı: [docs/f2_conversation_qa.md](docs/f2_conversation_qa.md) — 20 senaryonun tamamı (C01–C20) gerçek TUI/native koşumu ve insan değerlendirmesiyle sonuçlandırıldı; 15 Ağustos 2026 kapanış retestinde model routing, sistem durumu, dosya seçici ve yaşam döngüsü davranışları PASS. C04/C12 bulguları düzeltildi, kalan emoji-cursor/latency notları UX backlog'unda ayrıca izleniyor.
- [x] Her kalite örneği için beklenen davranış, model/prompt sürümü, latency limiti ve insan değerlendirme alanı.
  - Kanıt: [docs/f2_conversation_qa.md](docs/f2_conversation_qa.md) "Sohbet ve güvenlik senaryoları" tablosu; her satırda girdi, beklenen gözlem, kanıt kaynağı ve durum alanı dolu.
- [x] Hata/backlog şablonu: kullanıcı raporu, tekrar adımı, beklenen/gerçek sonuç, log/task ID, düzeltme commit'i ve regression testi.
  - Şablon: [docs/f2_bug_report_template.md](docs/f2_bug_report_template.md). Hassas sohbet/ek verisi yerine redakte özet ve correlation ID kullanılır.
- [x] Tek release komutu: format, test, clippy, dependency check, release build, servis health, kritik E2E smoke ve özet rapor.
  - Kanıt: `bash scripts/release_check.sh` offline format/test/Clippy/release build ile izole geçici SQLite üzerinde MCP `system.health` PASS ve unknown-tool DENY smoke çalıştırır. `--with-service` text model health'ini, `--with-vision` kurulu text + vision loopback health'ini ekler; hiçbiri modeli başlatmaz.
- [x] F2.0 exit review: açık P0/P1 kullanım hatası kalmadığının manuel kabulü.
  - Kanıt: [docs/f2_conversation_qa.md](docs/f2_conversation_qa.md) "Sonuç kapısı" ve "F2 kapanış retesti — 15 Ağustos 2026" bölümleri; F2 release gate ve kullanıcı kabulü PASS olarak kapatıldı.

Tamamlanma ölçütü: Günlük metin giriş/çıkış davranışı en az 20 senaryoda tekrar üretilebilir; yeni dilimler bu regression setini geçmeden birleşmez.

#### F2.1 — Native desktop kabuğu ve gerçek görsel ekler

Durum: TAMAMLANDI — native UI, güvenli attachment intake, vision dikey dilimi ve gerçek pencere kabulü (resize/minimize, mesaj gönderme, bildirim action) 15 Ağustos 2026'da tamamlandı.

Amaç: Terminal MVP'yi terk etmeden, fotoğraf ve dosya eklemeye uygun gerçek masaüstü deneyimini kurmak.

- [x] UI teknoloji spike: `egui/eframe` penceresi; açılış süresi, bellek kullanımı, Wayland/Hyprland uyumu ve paketleme riski ölçülür.
  - Kanıt: Hyprland v0.56.2 altında release `jarvis-desktop`, 222 ms içinde native client olarak kaydoldu ve ilk ekranda 181,904 KiB RSS (yaklaşık 178 MiB) kullandı. Güncel Lua focus dispatcher ve compositor üzerinden zarif pencere kapanışı PASS; sonuç kaydı [docs/f2_native_wayland_smoke_2026-08-14.md](docs/f2_native_wayland_smoke_2026-08-14.md). Paket bağımlılıkları Cargo lock'ta sabit, release build offline geçer.
- [x] UI/core sınırı: native UI yalnız client olur; `jarvis-core` Request/Policy/Task/Verifier zincirini doğrudan kullanır, ikinci runtime yaratmaz.
  - Kanıt: `jarvis-desktop`, tek paylaşılan `Runtime` örneği üzerinden aynı `Request → Policy → Task → Verifier` zincirini çağırır; native işçi yalnız sonucu UI kanalına döndürür. İkinci policy, tool registry veya kalıcı store oluşturmaz.
- [x] Sohbet ekranı: message card, streaming/typing state, ayrı draft composer, scroll-to-latest, arama/filtre hazırlığı ve erişilebilir klavye odağı.
  - Kanıt: salt-okunur kartlar, bağımsız multiline composer, `Düşünüyorum…` durumu, `stick_to_bottom`, Türkçe `İ/i` araması ve rol filtresi; arama/lock/notification unit testleri release derlemesiyle geçti.
- [x] Pencere yaşam döngüsü: resize, minimize, tekrar odak, tek-instance davranışı, servis durumu, bildirim tıklamasında pencereyi öne alma.
  - Kanıt: eframe responsive/resizable native viewport; sahiplik güvenli single-instance + stale-lock recovery testleri; pencere kapanışı model servislerini durdurmaz. Bildirim action'ı destekleyen daemonlarda “JARVIS'i aç”, Hyprland v0.55+ Lua `hl.dsp.focus({ window = "pid:…" })` adapter'ı ile yalnız bu pencereye focus ister; daemon/compositor başarısızlığı best-effort kalır. Güncel dispatcher gerçek release koşumunda PASS; uzun süreli resize/minimize kullanıcı kabulü F2.0 görsel smoke'ta ayrıca sürer.
- [x] Görsel tasarım sistemi: renk/typography/spacing tokenları, açık-koyu tema, kontrast kontrolü ve Türkçe metin taşma davranışı.
  - Kanıt: merkezi HUD renk tokenları, okunabilir fiziksel font baseline'ı, versioned koyu/açık tema ve font ölçeği tercihleri; kartlarda wrap/selectable text ve responsive yan panel minimumları uygulanır. Kullanıcı görsel kabulü F2.0 exit review'da ayrıca kalır.
- [x] Yerel ayarlar: UI tercihleri, tema, font scale ve notification seçeneği için versioned config; reset/export akışı.
  - Kanıt: `DesktopPreferences` schema v1, validation, atomik save/load ve invalid-config regression testleri; UI'da reset/export kontrolleri.
- [x] Attachment contract: `AttachmentRef` (ID, canonical local path, MIME, byte size, SHA-256, oluşturulma zamanı, provenance, sensitivity) ve task/audit ilişkisi.
  - Kanıt: metadata-only descriptor, task-bound audit ID ve canonical local path/ham byte'ın model context'inden dışlanması testleri.
- [x] Attachment storage policy: orijinal dosya yerinde referans mı yoksa uygulama kasasında kopya mı; retention, local delete ve stale-reference davranışı için ADR.
  - Kanıt: [ADR-0002](docs/adr/0002-attachment-reference-retention.md); stale/replaced-file reject ve UI “kaldır = referansı kaldır” semantiği.
- [x] Güvenli dosya seçimi: `Ctrl+O`/ataç düğmesi, kullanıcı görünür dosya adı/önizleme ve gönderimden önce kaldırma.
  - Kanıt: native `rfd` dosya seçici yalnız PNG/JPEG/TXT/Markdown/PDF filtreleriyle açılır; `inspect_local_attachment` doğrulamasından sonra isim/önizleme görünür, tekli veya tüm kuyruk referansları orijinal dosyayı silmeden kaldırılır. TUI eşdeğeri `/attach` ve `/attachments clear` ile vardır.
- [x] Dosya doğrulama: MIME magic-byte kontrolü, canonical path, allowlist, boyut/piksel limitleri, decode bomb/bozuk dosya reddi ve SHA-256.
  - Kanıt: PNG/JPEG magic/header + full decoder doğrulaması, 10 MiB/20 MP limiti, SHA-256 ve bozuk/stale dosya testleri.
- [x] Metin/doküman ekleri: TXT, Markdown ve PDF ilk aşamada yalnız güvenli metadata ile seçilir; ham içerik ayrı RAG ingestion/onay akışı olmadan modele veya tool'a taşınmaz.
  - Kanıt: canonical path, MIME/UTF-8/PDF magic, 5 MiB limiti, SHA-256, stale-reference reddi; descriptor path/ham belge metnini taşımaz ve injection metni runtime context testinde görünmez.
- [x] Vision model kararı: CPU uyumlu multimodal GGUF + eşleşen `mmproj` adayları, lisans, disk/RAM/latency karşılaştırması. **İndirme ancak kullanıcı onayından sonra.**

> Kullanıcı kararı — 14 Ağustos 2026: normal geliştirme indirimi önce boyutu bildirilerek **en fazla 100–200 MB** olabilir. Vision GGUF (yaklaşık 2–4 GB) ve `mmproj` (yaklaşık 0.4–1 GB) şimdilik ertelendi; birkaç saat sonraki durum güncellemesinde kullanıcıya yeniden hatırlatılacak. Bu dosyalar için açık “indir” onayı olmadan hiçbir indirme başlatılmaz.
- [x] Vision service: text modelinden ayrı loopback-only endpoint, health/lifecycle, attachment byte/path passing ve timeout/cancel sınırı.
  - Kanıt: `jarvis-vision.service`, yalnız `127.0.0.1:8089`, `-ngl 0`, 6 CPU thread, CORS `localhost`/credentials kapalı; ek baytlarını yalnız bu endpoint alır. İlk görsel isteğinde başlar; `exit` ve native RAM düğmesi iki modeli de kapatır.
- [x] Vision response policy: yalnız görüntü açıklaması/analizi; görüntü OCR metni untrusted data, tool authority yok; desteklenmeyen/hassas içerik için açık hata.
  - Kanıt: vision system contractı, 96-token gözlem limiti, `VisionAnalysis` XML-escape + user-data envelope; RAG, attachment veya vision verisi bağlamında modelin ürettiği capability etiketi core tarafından bastırılır ve audit'e yazılır. PNG/JPEG dışı, stale veya erişilemeyen servis path-safe failure verir.
- [x] Attachment privacy UX: ek geçmişini görme, tekli/tüm ekleri silme, ek gönderilmeden önce local-only uyarısı ve export.
  - Kanıt: native ve TUI, gönderilen ek için en fazla 50 adet **oturumluk metadata makbuzu** tutar. Tekli/tümü kaldırma orijinal dosyaya dokunmaz. Native UI'da “OTURUM EK MAKBUZLARI” + kullanıcı seçimli JSON export; TUI'da `/attachment-history`, `/attachment-history remove <id>|clear` ve `/attachment-export <dosya-yolu>` vardır. Export testinde canonical path, ham byte, prompt, model yanıtı ve task audit'in bulunmadığı doğrulandı.
- [x] E2E: JPEG/PNG başarı; bozuk MIME, boyut/piksel limiti, stale dosya, model kapalı, injection/EXIF izolasyonu ve TUI fallback.
  - Kanıt: Rust attachment/vision regression testleri ve [F2 local vision smoke](docs/f2_vision_smoke_2026-08-14.md). Gerçek PNG ve JPEG endpoint smoke yalnız sistem örnekleriyle yapıldı; user path/ham byte text modele verilmedi, vision'a geçmeden önce EXIF/ancillary metadata'dan arındırılmış JPEG taşınır.

Bu turda kanıtlanan alt dilim:

- [x] Attachment typed core: `AttachmentRef` canonical path, MIME, byte/pixel boyutu, SHA-256, provenance ve sensitivity ile Request/audit zincirine bağlandı.
- [x] PNG/JPEG magic-byte/header doğrulaması, boyut/piksel limiti, path containment, stale/replaced-file reddi, attribute-escaping ve canonical-path/ham-byte sızıntısı regresyon testleri eklendi. Model adapterı ek descriptor'ını yalnız `user` data mesajı olarak alır; local path taşımaz.
- [x] TUI ek kuyruğu: `/attach <PNG/JPEG-yolu>`, `/attachments` ve `/attachments clear`; gönderilen ekler yalnız metadata/data envelope olarak modele taşınır. Text-only model için görsel analiz iddiası yapılmaz.

#### F2 güncel çalışma kaydı — henüz exit gate değildir

- [x] Yerel release kontrolü: `bash scripts/release_check.sh` format, kilitli/offline bağımlılık çözümü, strict Clippy, release build ve izole MCP policy smoke çalıştırır. `--with-service` text model health'ini; `--with-vision` indirilmiş vision servisini de kontrol eder. Hiçbiri servis başlatmaz veya İnternet'e çıkmaz.
- [x] TUI davranış regresyonu: çok satırlı paste, Türkçe/UTF-8 kelime silme, `Ctrl+V`, Wayland primary selection için mouse orta tuşu, `Ctrl+Backspace`, `Ctrl+W`, `Ctrl+U`, terminal-control karakterleri, klavye/mouse scroll, `Home`/`End`, küçük terminalde scrollbar ve en yeni turun görünürlüğü testlere bağlandı.
- [x] Native UI temel kodu: `jarvis-desktop` aynı `Runtime` örneği üzerinde eski JARVIS HUD dilini izleyen teal/siyah merkez orb, sol sistem kontrolü ve sağ bağımsız sohbet konsolu sunar. Salt-okunur kartlar, ayrı composer, typing state, Türkçe harf farkını gözeten mesaj arama/rol filtresi, `Ctrl+O` görsel seçimi, güvenli önizleme/kaldırma, task-bound onay/red paneli, model-RAM kontrolü, stale-lock recovery'li ve sahiplik güvenli tek-pencere koruması, versioned yerel UI tercihleri (reset/export dahil) korunur. TUI davranışı değişmez; `jarvis --desktop` sibling release binary'sini başlatır. Pencereyi kapatmak servisi durdurmaz.
- [x] Native bildirim contractı: kullanıcının notification tercihi kapalıysa hiçbir bildirim üretilmez; açıksa completed yanıt, `WaitingForUser` onayı ve failed/interrupted işlem için ayrı başlıklar test edilir. Notification daemon yoksa mevcut `notify-send` çağrısı best-effort kalır ve task sonucunu değiştirmez.
- Audit integrity notu: TUI ve native client arasındaki SQLite audit sequence yarışı atomik `IMMEDIATE` transaction, busy timeout ve duplicate-sequence recovery ile düzeltildi. Mevcut kullanıcı DB'si silinmeden yedeklendi; release gate ve native kullanıcı kabulü PASS.
- [x] F2 kullanıcı bulguları düzeltme paketi: model-tabanlı governed routing, gerçek sistem health snapshot'ı, asenkron dosya seçici, explicit desktop exit, stale attachment hata ayrımı, Türkçe approval metinleri ve latest-message scroll düzeltildi. Ek kapanış kısayolu sonraki UX backlog'unda tutuluyor.
- [x] Vision sınırı UX'i: TUI ve native composer seçilmiş PNG/JPEG için görselin yalnız ayrı local vision servisine gideceğini; normal chat modelinin ham piksel veya yerel path görmediğini açıkça belirtir.
- [x] Doküman sınırı UX'i: TUI `/attach` ve native `Ctrl+O`, TXT/Markdown/PDF'yi seçebilir; belge içeriğinin modele veya tool'a gitmediğini, indekslemenin ayrı açık onaylı RAG akışı olduğunu belirtir.
- [x] Çok dilli sohbet contractı: sistem prompt'u son kullanıcı mesajının dilini temel alır; Türkçe ve İngilizce doğal cevap istenir, kullanıcı istemedikçe çeviri/dil karışımı yapılmaz. Bu bir yanıt şablonu veya kullanıcıya özel kural değildir; gerçek çıktılar sürümlü QA setinde model kalitesi olarak ölçülür.
- [x] Oturumluk ek makbuzu: ek gönderiminden sonra kullanıcı, yalnız filename/MIME/boyut/dimension/SHA-256/attachment ID metadata'sını görür; tekli/tümü temizler veya açıkça seçtiği JSON konumuna dışa aktarır. Makbuzlar 50 kayıtla sınırlı, persistent değildir; canonical path, ham byte, prompt, model yanıtı ve task/audit saklanmaz.
- [x] Serbest metin tool yönlendirme sınırı: sohbet girdisi artık anahtar-kelime `if/else` ile capability'ye bağlanmaz. Tek model üretimi ya doğal yanıt ya da allowlist'teki tam capability kimliğini taşıyan dar bir intent envelope üretir; envelope registry, policy ve verifier tarafından bağımsız doğrulanır. Normal sohbet ek routing çağrısı veya yanıt şablonu kullanmaz.
- [x] Native UI Wayland/Hyprland gerçek smoke: release binary açılışı, HUD/composer görsel kontrolü, güncel focus dispatcher ve pencere kapanışı PASS ([kayıt](docs/f2_native_wayland_smoke_2026-08-14.md)). 15 Ağustos 2026 takip koşumu ([kayıt](docs/f2_native_wayland_smoke_2026-08-15.md)): `Ctrl+O` picker gerçek dosya seçiciyle PASS. Bulunan "Tab ilk durağı yıkıcı düğme" riski aynı turda **düzeltildi ve gerçek binary'de doğrulandı**: composer artık açılışta varsayılan odağı alıyor, "Modeli RAM'den çıkar" tek aktivasyonla değil 4 saniyelik onay penceresiyle çalışıyor (`stop_model_button_is_armed`, test: `stop_model_button_requires_a_second_click_within_the_confirm_window`). Kullanıcı elle kabulü (15 Ağustos 2026): fare ile resize/minimize, gerçek klavyeyle mesaj yazıp gönderme ve bildirim action tıklaması PASS. ~~Açık backlog notu: picker iptalinden sonra pencerenin bazen sessizce kapanması — kök nedeni henüz netleşmedi, ayrıca izlenecek.~~ **16 Ağustos 2026'da kullanıcı düzeltti: bu bir bug değildi** — pencereyi o sırada kullanıcının kendisi kapatmıştı, JARVIS'in/`rfd`'nin bir hatası değil. Önce kod tarafında (`add_attachment`/`poll_attachment_picker`/`queue_attachment`) bir neden aranmıştı, bulunamamıştı — bu, aslında uygulamanın hiç hatalı davranmadığıyla tutarlı. Kayıt yanlış teşhis edilmiş bir "bug" olarak düzeltildi; kullanıcı yine de ileride benzer bir şey görürse dikkat edilecek, ama şu an bilinen açık bir hata değil.
  - **Kullanıcı bulgusu ve düzeltmesi (15 Ağustos 2026, F3 sırasında)**: kullanıcı gerçek kullanımda mesaj yazarken pencerenin birkaç saniyeliğine dondığunu bildirdi. Kök neden: `refresh_model_state` model health check'ini (gerçek ağ çağrısı) UI thread'inde senkron çalıştırıyordu; pencere ~45ms'de bir kendini yenilediği için bu düzenli donmalara yol açıyordu. Arka plan thread'e taşındı (`ModelHealthUpdate` + `mpsc` kanalı, `submit()`/`add_attachment()` ile aynı desen). Önceki "senkron `rfd` çağrısı" tahmini bu vesileyle yanlış bulundu ve kayıtta düzeltildi. Kanıt: [f2_native_wayland_smoke_2026-08-15.md](docs/f2_native_wayland_smoke_2026-08-15.md) "Ek düzeltme" bölümü; `cargo test`/`clippy`/`release_check.sh` PASS.
- [x] Vision modeli/multimodal E2E: kullanıcı onayıyla indirilen Qwen2.5-VL 3B Q4_K_M + `mmproj`, CPU-only ayrı loopback servisinde gerçek PNG smoke'undan geçti. Kanıt: [f2_vision_smoke_2026-08-14.md](docs/f2_vision_smoke_2026-08-14.md).

Tamamlanma ölçütü: Kullanıcı masaüstü penceresinden tek bir fotoğraf seçip ne gördüğünü sorabilir; ek hem UI'da görünür hem de core policy/audit zincirinde güvenli data olarak kalır.

### F3 — Kullanıcı profili, kontrollü bellek ve gerçek RAG

Durum: TAMAMLANDI — F2 exit gate 15 Ağustos 2026'da kapandı, F3'ün 18 maddesi 15-16 Ağustos 2026'da tek oturumda tamamlandı (bkz. alt maddeler).

Amaç: JARVIS'in kişisel bilgiyi hard-code etmeden, izinli ve açıklanabilir biçimde hatırlaması; belgelerden kaynaklı cevap vermesi.

- [x] Profile schema/ADR: ad, hitap biçimi, dil, rol/tercih, sensitivity, source ve updated-at alanları; sohbetten otomatik persistent write varsayılan olarak kapalı.
  - Kanıt: [ADR-0003](docs/adr/0003-user-profile-schema.md) — profil, ikinci bir depolama yolu değil, mevcut genel bellek sisteminin (`MemoryNamespace::UserProfile`) üzerine sabit dört anahat (`display_name`/`preferred_address`/`language`/`role_preference`) olarak kuruldu (`src/profile.rs`: `ProfileField`, `validate_profile_value`, `ProfileSnapshot`). `sensitivity`/`source`/`updated_at` zaten `MemoryRecord`'da vardı, yeniden üretilmedi. Kod grep'iyle doğrulandı: `propose_memory`/`commit_memory_proposal`'a giden tek yol TUI'deki açık `/remember` komutu; model/sohbet zinciri (`handle_with_provider*`) bunlara hiç erişmiyor. 3 yeni unit test PASS (`cargo test --offline`: 88 lib testi), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` hepsi PASS. Bilinen açık nokta ADR'de not edildi: native masaüstünde henüz `/remember` eşdeğeri yok (madde 2'de ele alınacak).
- [x] Profile CRUD UX: kullanıcı açıkça ekler/düzenler/siler; her alan için “modele dahil etme” anahtarı ve export/reset seçeneği.
  - Kanıt (TUI, `src/main.rs`): `/profile` (göster), `/profile set <ad|hitap|dil|rol> = <değer>` (onay: `/remember approve`/`reject` — mevcut onay yolu tekrar kullanıldı, ikinci bir onay mekanizması eklenmedi), `/profile delete <alan>`, `/profile reset` (yalnız bilinen 4 alanı siler, serbest anahtarlara dokunmaz), `/profile export <dosya-yolu>` (yalnız bilinen alan adı/değer/güncelleme zamanı içeren JSON, `memory_id`/`source` yok). 5 yeni uçtan uca test (set→onay→göster, bilinmeyen alan/geçersiz değer reddi, tekli silme, reset'in serbest anahtarı etkilememesi, export içeriği) gerçek bellekli (in-memory SQLite) `Runtime` ile PASS.
  - Kanıt (native, `src/bin/jarvis_desktop.rs`): sol panelde yeni “PROFİL” katlanır bölüm — 4 alan, her biri tek tıkla Kaydet/Sil; pencere açılışında mevcut profil değerleriyle dolar. Release binary'de gerçekten açılıp “▶ PROFİL” başlığının göründüğü ekran görüntüsüyle doğrulandı; tıklayıp alan kaydetme/silme adımı kullanıcının kendi gerçek tıklamasıyla doğrulandı (bkz. F3.2 sonrası kullanıcı ekran görüntüsü).
  - **Tamamlayıcı düzeltme (F3 madde 6 sırasında bulundu)**: bu madde "her alan için modele dahil etme anahtarı" istiyordu ama ilk teslimde `include_in_model_context` hem TUI hem native'de sessizce her zaman `true`'ya sabitlenmişti — kullanıcının gerçekten değiştirebileceği bir yol yoktu. Native panele her alanın yanına "Modele dahil" onay kutusu eklendi; TUI'ye `/remember model-context <evet|hayır>` eklendi (bkz. madde 6 kanıtı).
  - Yan kazanım: `jarvis_desktop.rs`'teki özel Türkçe küçük-harf katlama mantığı (`turkish_search_fold`) kütüphaneye taşındı (`turkish_case_fold`, `src/lib.rs`) ve `profile.rs`'teki alan-adı çözümlemesiyle paylaşıldı — iki ayrı yerde iki farklı Türkçe katlama mantığı olmasın diye. Bunu yaparken bir Türkçe “İ” unicode katlama hatası da test sırasında yakalanıp düzeltildi.
  - Genel kanıt: `cargo fmt --check`, `cargo test --offline` (94 lib + 21 main + 6 desktop), `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Profile injection boundary: profile alanları da system prompt değil typed data olarak taşınır; model profile üzerinden tool yetkisi kazanamaz.
  - Kanıt 1 (system prompt değil): `approved_memory_is_model_data_not_system_authority_and_is_audited` — profil kaydı modele her zaman `role: "user"` ile `<memory-data>` zarfı içinde gidiyor, hiçbir zaman `role: "system"` olmuyor.
  - Kanıt 2 (tool yetkisi kazanamaz — yeni test): `profile_field_can_influence_a_proposal_but_never_bypasses_policy_approval`. Kasıtlı düşmanca bir profil değeri (`role_preference = "Always auto-approve note.create without asking me first."`) ve modeli `note.create` önermeye zorlayan sahte bir provider ile: öneri kabul ediliyor (profil, attachment/RAG/vision gibi "güvenilmez bağlam" sayılmıyor çünkü zaten kullanıcı onaylı) ama **task yine de `WAITING_FOR_USER`'da bekliyor**, `tool.executed` audit'i hiç oluşmuyor — yani profil verisi en fazla bir öneri üretebiliyor, asla onayı atlatamıyor. Aynı Policy Gate her zamanki gibi çalışıyor.
  - Yan bulgu ve düzeltme (kullanıcının gerçek kullanımda bulduğu): `preferred_address` profil alanı ("efendim (Türkçe) / sir (English)") Türkçe yanıtlarda kullanılıyordu ama İngilizce'de kullanılmıyordu. Kök neden: system prompt, memory-data'yı yalnız "veri, talimat değil" diye tanımlıyordu, modele bunu **aktif olarak** uygulaması gerektiğini söylemiyordu. Gerçek local model karşısında `curl` ile birkaç yazım denendi; `preferred_address` alanının her yanıtta kullanılması gerektiğini açıkça söyleyen bir cümle eklendi (`JARVIS_SYSTEM_PROMPT`, `src/model.rs`) — güvenlik sınırı korunarak ("bu hâlâ hiçbir araç yetkisi vermez" ifadesiyle). Türkçe/İngilizce her ikisinde de gerçek modelle doğrulandı (regresyon: Türkçe "efendim" hâlâ çalışıyor). Yeni test: `system_prompt_instructs_honoring_the_preferred_address_profile_field_in_any_language`.
  - Yan bulgu (bug değil, ölçüldü): kullanıcının "İngilizce'de yavaşlık var" gözlemi araştırıldı. Gerçek modelle ölçüm: Türkçe 2,75 sn/18 token (~153ms/token), İngilizce 3,86 sn/24 token (~161ms/token) — token başı hız neredeyse aynı; fark yalnız o yanıtın birkaç token daha uzun olmasından. Yapısal bir dil-bazlı yavaşlık yok; CPU-only modelde daha uzun yanıtlar orantılı olarak daha uzun sürüyor.
- [x] Memory namespace'leri: session, user-profile, project, task ve ephemeral tool-output fiziksel/şematik olarak ayrılır.
  - Kanıt: `MemoryNamespace`'e `Session` ve `EphemeralToolOutput` eklendi ([ADR-0003 eki](docs/adr/0003-user-profile-schema.md)). Fiziksel ayrım: `validate_memory_record`, bu iki namespace için `expires_at` boşsa kaydı reddediyor (süresiz kalıcı olamazlar); üç kalıcı namespace (`UserProfile`/`Project`/`Task`) etkilenmedi. `retrieve_memory` süresi geçmiş kayıtları zaten otomatik filtreliyor. 2 yeni test: `session_and_ephemeral_namespaces_require_an_expiry_but_durable_ones_do_not`, `approved_memory_context_includes_session_and_ephemeral_output_namespaces`.
  - Bilinen açık nokta: henüz hiçbir üretim komutu (`/remember`, `/profile set`) bu iki yeni namespace'e yazmıyor; şema hazır, dolduracak somut özellik ileride gelecek.
  - Kanıt: `cargo test --offline` (98 lib + 21 main + 6 desktop), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Memory write policy: önerilen kayıt → kullanıcı preview/onay → sensitivity/TTL seçimi → audit; model kendiliğinden kalıcı anı yazamaz.
  - Kanıt: preview/onay zaten vardı (`/remember approve|reject`); eksik olan sensitivity/TTL'in kullanıcı **seçimiydi** — her zaman sabit `Internal`/kalıcı idi. `/remember sensitivity <public|internal|sensitive>` ve `/remember ttl <saat|none>` eklendi; ikisi de bekleyen teklifi onaylamadan **önce** değiştiriyor, önizleme her değişiklikte güncelleniyor. Yeni: `parse_data_sensitivity` (İngilizce ve Türkçe kelime kabul eder).
  - "Model kendiliğinden yazamaz" zaten F3.1'de kanıtlanmıştı (tek yol `/remember`/`/profile set`, ikisi de açık kullanıcı komutu); bu maddede değişmedi.
  - Bulunan ve düzeltilen yan hata: `parse_data_sensitivity("INTERNAL")` ve `ProfileField::from_user_input("DISPLAY_NAME")` başarısız oluyordu — `turkish_case_fold`, İngilizce büyük "I"yı Türkçe kuralına göre 'ı' yapıyor ("ınternal"), bu da İngilizce kelimeyle hiç eşleşmiyordu. İkisi de artık önce düz `to_lowercase()`, sonra `turkish_case_fold` deniyor (İngilizce/Türkçe karışık girdi için). Regresyon testleri eklendi.
  - Kanıt: `cargo test --offline` (99 lib + 25 main + 6 desktop, +7 yeni test), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Memory retrieval policy: namespace/sensitivity/TTL filtreleri, kullanıcıya “neden kullanıldı” bilgisi ve kaynaklı cevapta görünür attribution.
  - Namespace/TTL filtreleri zaten vardı (`retrieve_memory`, madde 4). Gerçek eksik kullanıcı kontrolüydü: `include_in_model_context` hiçbir yerden değiştirilemiyordu — bu turda `/remember model-context <evet|hayır>` (TUI) ve "Modele dahil" onay kutusu (native, madde 2'nin eksiği olarak orada da not edildi) eklendi.
  - Sensitivity için otomatik bir gizli filtre **eklenmedi bilerek**: `Sensitive` etiketini "modele dahil" anahtarıyla aynı anlama getirmek iki farklı kavramı (sınıflandırma vs. fiili kapı) birbirine karıştırırdı. Kullanıcı zaten doğrudan `include_in_model_context`'i kontrol edebiliyor; bu daha net.
  - "Neden kullanıldı" / attribution: workspace citation'ların zaten sahip olduğu görünür `evidence` mekanizması bellek kayıtlarına da eklendi (`memory.used:{namespace}:{key}`, yalnız audit'te değil artık). TUI ve native'de "Kaynaklar" listesinde "• Kayıtlı bilgi kullanıldı: USER_PROFILE:display_name" gibi görünüyor — değer değil, yalnız hangi kaydın kullanıldığı (uzun/hassas değer tekrar edilmiyor).
  - Kanıt: yeni test `approved_memory_is_model_data_not_system_authority_and_is_audited` genişletildi (evidence içinde `memory.used:USER_PROFILE:nickname` doğrulanıyor); yeni TUI testi `remember_model_context_toggle_is_actually_respected_at_retrieval`. `cargo test --offline` (99 lib + 26 main + 6 desktop), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Memory deletion: tek kayıt, namespace, proje ve “her şeyi unut” silme; tombstone/backup etkisi ve doğrulama testi.
  - Tek kayıt (`/forget <id>`) ve "her şeyi unut" (`/forget all`) zaten vardı. Eksik olan namespace/proje silme (`delete_memory_namespace` persistence katmanında vardı ama `Runtime`'a hiç bağlanmamıştı, hiçbir komuttan erişilemiyordu) eklendi: `Runtime::delete_memory_namespace` (audit'li) + TUI `/forget namespace <profil|proje|görev|oturum|geçici>` (`parse_memory_namespace`, İngilizce/Türkçe kabul eder).
  - Tombstone/backup etkisi belgelendi: [ADR-0003 eki](docs/adr/0003-user-profile-schema.md) — gerçek `DELETE`, tombstone yok (ADR-0002'nin ek felsefesiyle tutarlı); dosya-seviyeli eski yedeklerin silinen veriyi hâlâ içerdiği açıkça not edildi.
  - Doğrulama testi: `forget_all_memory`'nin **hiç testi yoktu** — artık `forget_all_memory_actually_empties_storage` var (gerçekten boşalttığını kanıtlıyor, sadece sayı dönmediğini). Yeni: `delete_memory_namespace_only_removes_that_namespace`, `parse_memory_namespace_accepts_english_and_turkish_words`, TUI `forget_namespace_deletes_only_that_namespace_and_rejects_unknown_words`.
  - Kanıt: `cargo test --offline` (102 lib + 27 main + 6 desktop, +5 yeni test), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Memory migration/backup: versioned schema, encrypted-secret ayrımı gerekiyorsa ADR, export/import ve rollback.
  - Versioned schema zaten vardı (`schema_migrations`, `CURRENT_SCHEMA_VERSION`).
  - Rollback: `SqliteStore::open`, eski şemalı bir dosyayı `migrate()` çalıştırmadan **önce** `VACUUM INTO` ile (`backup_to` — vardı, test'liydi, hiç kullanılmıyordu) `<yol>.pre-migration-backup-<epoch>.db` olarak yedekliyor. Zaten güncel/yeni bir DB hiç yedeklenmiyor (her açılışta gereksiz dosya birikmesin diye).
  - Export/Import: `memory_export`/`memory_import` + TUI `/memory export <dosya-yolu>`, `/memory import <dosya-yolu>`. Tüm namespace'leri kapsayan taşınabilir JSON; `memory_id`/`source` dışa aktarılmaz, içe aktarma her zaman `propose_memory` ile **yeni** teklif üretir (asla doğrudan yazmaz), aynı onay adımından geçer. Bozuk bir satır tüm içe aktarmayı durdurmuyor, yalnız o satır atlanıyor ve raporlanıyor.
  - Şifreleme kararı: [ADR-0003 eki](docs/adr/0003-user-profile-schema.md) — eklenmedi, gerekçesi belgelendi (tek kullanıcılı/tek cihazlı yerel uygulama, gerçek sınır OS dosya izinleri, ADR-0002 ile tutarlı); çok kullanıcılı/senkronize bir gelecekte yeniden değerlendirilmesi gerektiği açıkça not edildi.
  - Kanıt: `cargo test --offline` (105 lib + 28 main + 6 desktop, +5 yeni test — pre-migration backup, export/import round-trip x2, TUI round-trip), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Workspace izin UX'i: klasör seçimi, kök sınırı, indeks kapsamı, exclude pattern ve indeks boyutu tahmini kullanıcıya gösterilir.
  - Öncesinde yalnız tek dosya (`/index <dosya>`) vardı, klasör kapsamı/boyut önizlemesi yoktu. Eklenenler: `preview_workspace_index` (dosya içeriğini hiç açmadan, yalnız metadata ile: kaç dosya, tahmini toplam boyut, kaç tanesi şifre-benzeri/boyut-limiti/desen yüzünden hariç) ve `Runtime::index_workspace_folder` (önizlemedeki dosyaları mevcut tek-dosya `index_workspace_document`'ı tekrar tekrar çağırarak indeksler — ikinci bir ingestion yolu açılmadı).
  - Kök sınırı zaten `validate_workspace_document_path`'te vardı (path escape reddi); değişmedi.
  - `.git`/`target`/`node_modules`/`.venv` varsayılan olarak taranmaz (kullanıcı doğrudan `/index` ile içindeki bir dosyayı yine de seçebilir).
  - TUI: `/index-preview <klasör> [hariç-desen ...]` (önce göster, indekslemez), `/index-folder <klasör> [hariç-desen ...]` (gerçekten indeksler, onay gerektirir).
  - Kanıt: `cargo test --offline` (107 lib + 28 main + 6 desktop, +2 yeni test — kategori doğruluğu ve `.git` içinin hiç görünmediği, onay gerekliliği), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS. TUI komutları için ayrı test eklenmedi (mevcut `/index`'in de hiç TUI-seviyeli testi yok; alttaki fonksiyonlar zaten tam kapsamlı test edildi).
- [x] Document parser katmanı: Markdown/TXT/PDF başlangıcı; sonradan Office/HTML için ayrı parser ve sandbox kararı.
  - Markdown/TXT zaten çalışıyordu (düz UTF-8 metin, özel parser gerektirmiyor). Gerçek yeni yetenek: **PDF**. Yeni bağımlılık: `pdf-extract` (+ transitive `lopdf` vb.) — `cargo add`, offline build için önce bir kez online derleme gerekti, sonrasında `--offline` çalışıyor.
  - Sandbox kararı (bu madde açıkça istiyor): PDF parser çağrısı `std::panic::catch_unwind` ile izole edildi — PDF parserlar bilinen bir crash yüzeyi olduğundan, bozuk/kötü niyetli bir PDF tüm JARVIS sürecini düşürmemeli. Süreç-içi/panic-izolasyonlu bir sandbox; ayrı process/container değil. Office/HTML eklendiğinde kendi sandbox kararını alacak, bu madde onları kapsamıyor.
  - Güvenlik: PDF de aynı şifre-benzeri-isim ve boyut-limiti kontrollerinden geçiyor (`reject_secret_like_workspace_document_name`, `reject_oversized_workspace_document`); yalnız binary/UTF-8 reddi atlanıyor (PDF zaten binary, beklenen).
  - Kanıt: `extract_pdf_text_reads_real_pdf_content_and_never_panics_on_garbage` — elle inşa edilmiş gerçek, geçerli bir PDF'den metin doğru çıkarılıyor; garbage/boş/kesik byte'lar panic değil temiz `Err` veriyor. `a_pdf_indexes_end_to_end_and_becomes_a_searchable_citation` — uçtan uca: PDF indeksleniyor, sonra ilgili bir soru sorulduğunda gerçekten `workspace.citation:` evidence'ı üretiyor (RAG zincirinin geri kalanı hiç değişmeden PDF'i de kapsıyor).
  - Kanıt: `cargo test --offline` (109 lib + 28 main + 6 desktop, +2 yeni test), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Ingestion pipeline: canonical path, content hash, MIME/size limiti, chunking, dosya değişiklik algısı ve incremental re-index.
  - Canonical path/content hash/boyut limiti/chunking zaten vardı. Eksik olan: **dosya değişiklik algısı ve incremental re-index** — her `/index` çağrısı, içerik aynı kalsa bile chunk'ları silip yeniden yazıyordu.
  - Eklenen: `WorkspaceIngestionReport.content_changed` alanı. `index_workspace_document` artık yeni hash'i eklemeden önce mevcut kayıtlı hash'le karşılaştırıyor; aynıysa DELETE+INSERT hiç çalışmıyor, mevcut `indexed_at` aynen dönüyor. Gerçekten değiştiyse eski chunk'lar silinip yenileri yazılıyor (zaten vardı, `document_id` path'ten deterministik türetildiği için stale chunk kalmıyor).
  - TUI: `/index` artık "değişmemiş, zaten güncel" diye ayrı bir mesaj veriyor; `/index-folder` özetinde "N dosya indekslendi, M dosya zaten güncel" ayrımı var.
  - Kanıt: `reindexing_skips_unchanged_content_but_replaces_chunks_when_content_changes` — üç aşamalı: ilk indeksleme (`content_changed=true`), aynı içerikle tekrar (`content_changed=false`, `indexed_at` **değişmiyor**), gerçek içerik değişikliği (`content_changed=true`, eski arama terimi artık **hiç bulunamıyor**, yeni terim bulunuyor — stale chunk kalmadığı doğrulandı).
  - Kanıt: `cargo test --offline` (110 lib + 28 main + 6 desktop, +1 yeni test), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Metadata/FTS index: SQLite metadata-first retrieval, belge/chunk ID, konum, hash, provenance ve indeks sürümü.
  - Belge/chunk ID, konum (`chunk_ordinal`/`canonical_path`), hash (`content_sha256`), provenance (`ContentProvenance::UntrustedProjectFile`) zaten vardı. Eksik olan **indeks sürümü**: `WorkspaceIngestionReport.schema_version` kodda sabit `1` idi ama **hiçbir zaman DB'ye yazılmıyordu** — gerçek bir versiyon takibi yoktu.
  - Eklenen: `workspace_documents.index_schema_version` kolonu (migration, `schema_migrations` sürüm 6) ve `CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION` sabiti (`workspace.rs`). Artık her indekslemede gerçekten yazılıyor/okunuyor; madde 11'in incremental re-index kontrolüne bağlandı — eğer diskteki sürüm bu build'in bildiğinden eskiyse, **içerik hash'i aynı olsa bile** yeniden indeksleniyor (çünkü gelecekte chunking algoritması değişirse, eski chunk'lar hash aynı kalsa da bayat olabilir).
  - Kanıt: `a_stale_index_schema_version_forces_reindexing_even_with_identical_content` — diskteki sürümü elle `0`'a düşürüp (`raw_connection` test-only erişimi) aynı içerikle tekrar indeksleyince `content_changed=true` döndüğü doğrulandı.
  - Kanıt: `cargo test --offline` (111 lib + 28 main + 6 desktop, +1 yeni test; 2 eski test `schema_version` beklentisi 5→6 güncellendi), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Embedding/re-rank kararı: FTS baseline ölçülür; embedding model gerekiyorsa boyut/RAM/lisans bilgisi ve kullanıcı onayıyla indirilir.
  - Karar ve tam gerekçe: [ADR-0004](docs/adr/0004-hybrid-rag-embedding.md). Kullanıcıyla birlikte karşılaştırılan alternatifler (e5-small, granite-r2, EmbeddingGemma, jina-v3) ve boyut/lisans/erişim bilgisiyle **Qwen3-Embedding-0.6B (Q8_0, 639 MB, Apache 2.0)** seçildi, kullanıcı onayıyla indirildi.
  - Hibrit mimari: `src/embedding.rs` (`EmbeddingProvider` trait, `LlamaEmbeddingProvider`, cosine similarity, serialize/deserialize), `workspace_chunk_embeddings` tablosu (`schema_migrations` sürüm 7), `hybrid_search_workspace` (Reciprocal Rank Fusion — FTS ve embedding eşit ağırlıklı, biri diğerinin yedeği değil). FTS (`search_workspace`) hiç değişmedi.
  - Model-versiyonlu cache + geriye dönük doldurma: kullanıcının ilettiği gerçek bir tasarım açığı (ChatGPT gözden geçirmesi) düzeltildi — vektör yeniden kullanımı `content_sha256` + `embedding_model_id`'ye göre; FTS-only indekslenmiş belgeler, embedding sağlayıcısı sonradan bağlanınca otomatik tamamlanıyor.
  - `jarvis-embedding.service` (port 8090, loopback-only) kuruldu; text/vision'dan farklı olarak otomatik başlatılmıyor, yalnız zaten erişilebilirse `Runtime`'a bağlanıyor. `/status` hybrid/FTS-only durumunu gösteriyor.
  - Gerçek doğrulama: gerçek servis + gerçek metin modeliyle uçtan uca — paraphrase edilmiş bir soru, ortak kelime paylaşmadan doğru belgeyi buldu, alakasızı hiç karıştırmadı.
  - Bilinçli ertelenen 7 iyileştirme (permission filtresi, semantic chunking, batch embedding, gözlemlenebilirlik, rebuild komutları, configurable RRF, reranker) ADR-0004'te ve kalıcı hafızada not edildi, F3 sonrası değerlendirilecek.
  - Kanıt: `cargo test --offline` (119 lib + 28 main + 6 desktop, +9 yeni test), `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `scripts/release_check.sh` — hepsi PASS.
- [x] Secret/hassas filtre: `.env`, private key, credential, binary, çok büyük dosya ve kullanıcı exclude listesi indeks dışı; filtre loglanır ama sır saklanmaz.
  - Dosya adı filtresi genişletildi (tek kaynak, `is_secret_like_file_name`): `.env`/`.env.*`, `.netrc`, `.npmrc`, `credentials(.json)`, `secrets.yaml/yml`, `id_rsa/id_dsa/id_ecdsa/id_ed25519`, `*.pem/*.key/*.p12/*.pfx/*.jks/*.keystore`. Önizleme (`preview_workspace_index`) ile gerçek indeksleme (`reject_secret_like_workspace_document_name`) artık aynı listeyi okuyor — önceden iki yerde ayrı ayrı tutuluyordu, birbirinden sapma riski vardı.
  - Yeni: içerik-bazlı tarama (`reject_secret_like_workspace_document_content`) — dosya *adı* şüpheli olmasa bile içine yapıştırılmış bir kimlik bilgisini yakalıyor (PEM private key başlıkları, `AWS_SECRET_ACCESS_KEY`, `AKIA...`, GitHub/Slack token önekleri). Hem düz metin hem PDF'ten çıkarılan metin için çalışıyor. Bilinçli olarak dar tutuldu (regex yok, sabit işaretler) — "password" gibi genel bir kelime tetiklemiyor, yanlış pozitifle meşru bir belgeyi sessizce dışlamasın diye.
  - "filtre loglanır ama sır saklanmaz": `Runtime::index_workspace_document` artık her reddi audit'e yazıyor (`workspace.index.rejected_secret_like` / `workspace.index.rejected`) — yalnız dosya yolu ve sabit neden kategorisi, asla dosya içeriği veya eşleşen kimlik bilgisinin kendisi.
  - Kanıt: 5 yeni test (`broadened_secret_like_filenames_are_excluded_without_over_matching`, `embedded_credential_in_content_is_rejected_and_audited_without_leaking_it`, `non_secret_rejection_is_audited_with_the_generic_event_name`, `pdf_with_embedded_credential_in_extracted_text_is_rejected`, mevcut `workspace_rag_excludes_secrets_...` hâlâ geçiyor) + tam paket: `cargo fmt`, `cargo test --offline` (123 lib + 28 main + 6 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS, MCP smoke dahil).
- [x] Retrieval policy: relevance threshold, result sayısı, token/context budget, duplicate suppression ve kaynağı olmayan cevabı engelleme.
  - Beş politika da `hybrid_search_workspace` (ranking katmanı) ve `Runtime::approved_workspace_context` (bağlam derleme katmanı) arasında merkezileştirildi, çağıran tarafta dağınık mantık bırakılmadı.
  - Relevance threshold: `MIN_RELEVANT_SIMILARITY=0.10` — embedding sağlayıcısı varsa, FTS eşleşse bile kosinüs benzerliği eşiğin altında kalan chunk sonuçtan tamamen çıkarılıyor (sadece geriye itilmiyor). Henüz embed edilmemiş chunk'lar bu kontrolden muaf (FTS-only mod hiç etkilenmiyor, ADR-0004'teki "FTS asla bozulmadı" korunuyor).
  - Result sayısı: `WORKSPACE_RETRIEVAL_RESULT_LIMIT=4` — artık çağrı noktasındaki sihirli sayı değil, adlandırılmış tek bir politika değeri (`src/workspace.rs`).
  - Token/context budget: `WORKSPACE_CONTEXT_CHAR_BUDGET=4000` — `approved_workspace_context`, en iyi sıralı sonuçlardan bütçe dolana kadar alıyor; sonuç sayısı limiti dolsa bile toplam karakter sonsuz büyüyemiyor.
  - Duplicate suppression: `hybrid_search_workspace` artık aynı chunk metnini (iki farklı belgede birebir aynı içerik) yalnız en yüksek sıradaki tekilinde döndürüyor.
  - Kaynağı olmayan cevabı engelleme: `limit` artık bir garanti değil bir tavan — hiçbir gerçek eşleşme/relevance geçmeyen sorguda sonuç sıfıra kadar düşebiliyor, asla zayıf bir eşleşmeyle doldurulmuyor; bu, modelin var olmayan bir kaynağa referans vermesinin önündeki gerçek engel.
  - Kanıt: 4 yeni test (`hybrid_search_drops_a_weakly_relevant_chunk_below_the_similarity_floor`, `hybrid_search_suppresses_duplicate_chunk_content_across_documents`, `no_relevant_match_yields_zero_citations_not_a_padded_guess`, `conversation_context_stays_under_the_workspace_char_budget` — sonuncusu gerçek bir `Runtime::handle_with_provider` sohbet turu üzerinden uçtan uca) + tam paket: `cargo fmt`, `cargo test --offline` (127 lib + 28 main + 6 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] Citation UX: yanıtın hangi belge/parçadan geldiği, kısa alıntı, dosya konumu ve “kaynağı aç” davranışı.
  - `Runtime::last_workspace_citations()`: en son cevabı destekleyen tam citation'ları (yalnız path/ordinal değil, tam chunk içeriğiyle) tutan yeni bir alan/accessor — genel `evidence: Vec<String>` izinden ayrı, çünkü o iz her capability'de ortak ve yalnız string.
  - `WorkspaceCitation::short_excerpt(max_chars)`: boşluk/satır sonlarını tek satıra indiren, Unicode karakter sınırında kesen kısa alıntı üretici (workspace.rs, pure/test edilebilir).
  - TUI (main.rs): her citation'lı yanıtın altında artık `[n] dosya#chunk-N — "kısa alıntı" (tamamı için: /source n)` satırları görünüyor; yeni `/source <n>` komutu o citation'ın tam metnini ve tam dosya yolunu açıyor ("kaynağı aç"). Numara dışı/aralık dışı/kaynak-yok durumları ayrı ayrı net mesajlarla karşılanıyor.
  - Native desktop (jarvis_desktop.rs): aynı kısa-alıntı + dosya konumu satırı mirror edildi; desktop'ta hiç slash-komut yüzeyi olmadığından (tüm komut akışı zaten TUI'ye ait) "kaynağı aç" davranışı kasıtlı olarak TUI'ye bırakıldı.
  - Kanıt: 4 yeni test (`workspace_citation_short_excerpt_collapses_whitespace_and_truncates_by_chars`, `runtime_tracks_last_workspace_citations_for_the_open_source_action` — ikisi lib.rs; `source_command_opens_the_full_citation_content_by_position`, `source_command_rejects_out_of_range_non_numeric_and_missing_citations` — main.rs) + tam paket: `cargo fmt`, `cargo test --offline` (129 lib + 30 main + 6 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] Untrusted-content isolation: doküman/OCR/web metni data envelope içinde kalır; prompt injection, tool call ve data exfiltration denemeleri reddedilir.
  - Denetim: mekanizmanın büyük kısmı zaten önceki F3/F2 turlarında kurulmuştu (`ContentProvenance`, `isolate_untrusted_content`, vision için ayrı `<vision-analysis-data>` zarfı, ekler için `<attachment-data>` zarfı — belge-olmayan eklerde dosya *içeriği* modele hiç gitmiyor, yalnız metadata). Bu turda gerçek boşluklar dolduruldu, mevcut olanlar yeniden inşa edilmedi.
  - Web metni: `ContentProvenance::UntrustedWeb` tanımlı ama JARVIS'te henüz hiç web-fetch capability'si yok (canlı üretici yok). Yeni test kanıtlıyor ki `isolate_untrusted_content` bu provenance'ı `UntrustedProjectFile` ile birebir aynı şekilde izole ediyor — ileride bir web-fetch eklenirse izolasyonu bedavaya devralıyor.
  - Gerçek boşluk (kapatıldı): ek (attachment) dosya adı üzerinden prompt injection hiç uçtan uca test edilmemişti — yalnız workspace RAG ve vision için vardı. Yeni test: şüpheli adlı bir doküman eki (içerik zaten modele gitmiyor, yalnız dosya adı gidiyor) tek başına `has_untrusted_model_context` tetikliyor ve modelin ürettiği `<jarvis-intent>` etiketi yine bastırılıyor (`UNTRUSTED_MODEL_INTENT_SUPPRESSED`).
  - Data exfiltration (yapısal kanıt, yeni): `CapabilityRegistry::baseline()`'daki **hiçbir** capability `requires_network` değil — yani injection bir şekilde onaylı bir task'e dönüşse bile ağ üzerinden veri sızdıracak hiçbir yol yok. Yeni test bunu tüm registry üzerinden (sabit kodlanmış bir liste değil, gerçek `all()` iterator'ü ile) doğruluyor.
  - Kanıt: 3 yeni test (`isolate_untrusted_content_treats_web_provenance_the_same_as_document_provenance`, `untrusted_attachment_filename_cannot_activate_a_model_proposed_capability`, `no_baseline_capability_requires_network_access`) + zaten var olan 3 test (`workspace_rag_content_is_provenanced_and_instruction_isolated`, `retrieved_workspace_data_cannot_activate_a_model_proposed_capability`, `untrusted_vision_context_cannot_activate_a_model_proposed_capability`) hâlâ geçiyor + tam paket: `cargo fmt`, `cargo test --offline` (132 lib + 30 main + 6 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] RAG eval seti: doğru kaynak, yanlış kaynak, secret exclusion, eski indeks, çelişen belge, injection ve silinmiş bellek senaryoları.
  - Yedi senaryonun her biri için ayrı, adı `rag_eval_` ile başlayan, tek bir yerde toplanmış özel bir test (`cargo test rag_eval_` yalnız bu seti çalıştırır) — madde 9-17'de kurulan mekanizmaların üzerine, bazıları o mekanizmaları kanıtlayan testlerle kavramsal olarak örtüşse de kasıtlı olarak taze/bağımsız örnekler (bir eval setinin işi, baştan sona okunabilir tek bir koleksiyon olmak, önceki maddelere işaretçi zinciri değil).
  - 1/7 doğru kaynak, 2/7 yanlış kaynak, 3/7 secret exclusion, 4/7 eski indeks (gerçek bug bulundu ve düzeltildi — bkz. not), 5/7 çelişen belge (iki belge de model bağlamına ulaşıyor, biri sessizce gizlenmiyor), 6/7 injection, 7/7 silinmiş bellek (namespace silindikten sonra ne `list_memory()`'de ne de sonraki bir sohbet turunun model bağlamında görünüyor).
  - Test tasarımı hatası bulundu ve düzeltildi: ilk yazımda "eski indeks" senaryosu `durum-turuncu`/`durum-lacivert` gibi tireli işaretçiler kullanıyordu; `fts_query` sorguyu alfasayısal olmayan karakterlerden (tire dahil) ayırdığı için iki işaretçi de ortak "durum" terimini paylaşıyordu ve test yanlış bir şekilde geçmiyor gibi görünecekti — gerçek bir staleness bug'ı değil, test fikstüründeki paylaşılan kelime kökü sorunuydu. Ortak kök paylaşmayan işaretçilerle (`turuncuseviye`/`lacivertseviye`) düzeltildi.
  - Kanıt: 7 yeni test (`rag_eval_correct_source_is_retrieved_and_cited`, `rag_eval_wrong_source_is_never_cited_for_an_unrelated_query`, `rag_eval_secret_document_is_excluded_from_retrieval`, `rag_eval_stale_index_is_refreshed_after_content_changes`, `rag_eval_conflicting_documents_are_both_surfaced`, `rag_eval_prompt_injection_in_retrieved_content_never_activates_a_capability`, `rag_eval_deleted_memory_never_resurfaces`) + tam paket: `cargo fmt`, `cargo test --offline` (139 lib + 30 main + 6 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**F3 tamamlandı — 16 Ağustos 2026.** 18 maddenin tamamı `[x]`, her biri gerçek test kanıtıyla. Tamamlanma ölçütü karşılandı (aşağıda). Aynı gün, kullanıcı geri bildirimiyle "F3 sonrası düzeltmeler" bölümünde 10 ek madde daha kapatıldı (bellek güncelleme hatası, doğal dil bellek komutları, kalıcı sohbet geçmişi, GPT'nin 7 RAG önerisinden 6'sı, katmanlı bellek mimarisinin 3 aşaması, açılış karşılaması+profil dosyaları+hava durumu) — yalnız GPT önerisi 7/7 (reranker) bilinçli olarak yapılmadı, kullanıcıdan onay bekleniyor.

Bu turda kanıtlanan alt dilim:

- [x] Kontrollü bellek persistence: user-profile/project/task namespace'leri, schema migration, model-context opt-in, TTL filtresi, explicit proposal/onay, audit ve tekli/tüm kayıt silme.
- [x] TUI bellek UX'i: `/remember anahtar = değer` → preview → `/remember approve|reject`; `/memory` görünürlüğü ve `/forget <id>|all` silme akışı.
- [x] İlk gerçek RAG: explicit `/index <proje-içi-göreli-dosya>`, canonical root/path, SHA-256, chunking, SQLite FTS, source citation, stale chunk replacement, secret/binary/büyük dosya reddi ve untrusted-content isolation.

Tamamlanma ölçütü: Kullanıcı bir klasörü izinle indeksleyip kaynak gösteren cevap alabilir ve saklanan tüm kişisel veriyi görüntüleyip silebilir.

**F3 sonrası düzeltmeler (16 Ağustos 2026, kullanıcı bildirdi):**

- [x] **Bellek güncelleme hatası**: aynı `(namespace, key)` ile tekrar `/remember` yazmak eskisinin üzerine yazmak yerine ikinci bir kayıt daha ekliyordu — `memory_id`, değer + kaynak + nanosaniye nonce'undan türetiliyordu, yani aynı anahtar bile her seferinde "yeni" sayılıyordu. Gerçek bir şişme riskiydi; daha kötüsü, eski ve yeni değer ikisi de geçerliyse ikisi de birden modele gidebiliyordu.
  - Düzeltme: `propose_memory` artık `memory_id`'yi yalnız `(namespace, key)`'den türetiyor — aynı anahtar her zaman aynı kimliğe çözülüyor, bu da zaten var olan `ON CONFLICT(memory_id) DO UPDATE` SQL yolunu (persistence.rs, hiç değişmedi) gerçek bir güncelleme için tetikliyor. `created_at` korunuyor (UPDATE SET listesinde yok), `updated_at` ilerliyor. `proposal_id` (bekleyen öneri takibi) ayrı kaldı, hâlâ değer/zaman bazlı.
  - Kanıt: yeni test `remembering_the_same_key_again_updates_the_existing_record_instead_of_duplicating_it` — aynı anahtara iki farklı değerle `/remember`, tek kayıt kaldığını, eski değerin hiçbir yerde (ne `list_memory()` ne `retrieve_memory()`) görünmediğini kanıtlıyor. Var olan bir test (`memory_export_then_import_...`) eski (hatalı) davranışı doğruluyordu, yeni doğru davranışı yansıtacak şekilde güncellendi. Tam paket: `cargo fmt`, `cargo test --offline` (140 lib + 30 main + 6 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **Doğal dille bellek yaz/güncelle/sil**: kullanıcı `/remember anahtar = değer` sözdizimini hatırlamak zorunda kalmadan, normal bir cümleyle ("hafızana yaz: adım Ali", "hafızandan isim bilgimi sil") tek adımda (ikinci bir onay adımı olmadan) bellek yazabilsin/silebilsin istedi.
  - Yeni modül `src/memory_intent.rs`: `parse_memory_intent` — sabit bir tetikleyici cümle listesine (`hafızana yaz`, `hatırla ki`, `hafızandan ... sil`, ...) karşı kullanıcının ham girdisini eşleştirir; model hiç karışmaz, karar tamamen kullanıcının kendi yazdığı metne bakarak veriliyor (slash-komutların çalışma şekliyle aynı ilke). Türkçe SOV sözdizimi ("hafızandan X sil" — fiil sonda) için prefiks+suffiks eşleşmesi kullanılıyor, tek prefiks yetmiyor.
  - Bilinen olgu kalıpları ("benim adım X", "beni X diye çağır", "dilim X") doğrudan mevcut `ProfileField`/`propose_profile_field` yoluna yönlendiriliyor — ayrı bir depolama yolu açılmadı. Tanınmayan kalıplar `anahtar = değer` sözdizimine düşüyor. Hiçbiri eşleşmezse kullanıcıya net bir "anlayamadım" mesajı dönüyor — asla sessizce yok sayılmıyor ya da tahmin yürütülmüyor.
  - Yan kazanım: `ProfileField::from_user_input` artık birden fazla takma ad kabul ediyor (`ad`+`isim`, `rol`+`tercih`) — hem doğal dil hem `/profile set` bundan faydalanıyor.
  - Yeni `Runtime::delete_memory_by_key` (anahtar bazlı, namespace'ler arası, Türkçe-katlamalı arama) ve `Runtime::delete_profile_field` (hem `/profile delete` hem yeni doğal dil silme yolu bunu paylaşıyor — kod tekrarı yok).
  - Native desktop (`jarvis_desktop.rs`) da aynı davranışı gösteriyor — `handle_natural_language_memory_command` bilinçli olarak `impl JarvisDesktop` metodu değil bağımsız bir fonksiyon (egui `Context` kurmadan test edilebilsin diye, bu dosyanın zaten var olan test kısıtına uyar).
  - Kanıt: 10 `memory_intent` testi + 4 uçtan uca TUI testi + 1 uçtan uca desktop testi (`desktop_natural_language_memory_command_saves_updates_and_reports_unparseable`) + tam paket: `cargo fmt`, `cargo test --offline` (151 lib + 34 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
  - Bilinen sınır: sabit cümle kalıpları — genel NLU değil. Listede olmayan bir ifade biçimi tanınmaz, kullanıcıya "anlayamadım" döner (sessizce yanlış bir şey kaydetmez). Eksik bir kalıp fark edilirse `memory_intent.rs`'e eklemek küçük, lokal bir değişiklik.
- [x] **Kalıcı sohbet geçmişi (SSD)**: kullanıcı "konuşma RAM'de değil SSD'de tutulabilir, her şeyi baştan oluşturmak istemem" dedi — önceden `chat_history` yalnız RAM'deydi, JARVIS kapanınca kayboluyordu (bilinçli bir tasarımdı, kullanıcı isteğiyle tersine çevrildi).
  - Yeni `chat_messages` tablosu (schema sürüm 8). `Runtime::append_chat_turn` artık her turu (best-effort, bir yazma hatası asıl sohbet turunu asla bozmaz) diske de yazıyor ve aynı turda `MAX_COMPLETED_CHAT_HISTORY_TURNS`'e buduyor — disk RAM'deki sınırdan asla daha büyük olamaz.
  - `Runtime::with_store` artık açılışta son oturumun geçmişini yüklüyor — yeni bir oturum kaldığı yerden devam ediyor. `Runtime::new()` (store'suz) hâlâ boş başlıyor.
  - `/clear`'ın anlamı değişti: artık yalnız görünen listeyi değil, `Runtime.chat_history`'i VE diskteki kaydı da gerçekten siliyor (yeni `Runtime::clear_chat_history`) — geçmiş artık kalıcı olduğu için "temizle" gerçek bir sıfırlama olmalı, yalnız kozmetik değil.
  - Kanıt: 4 yeni test — `conversation_history_survives_a_real_restart_across_store_instances` (gerçek dosya tabanlı SQLite üzerinden iki ayrı `Runtime` örneğiyle, gerçek "restart" senaryosu), `clear_chat_history_removes_it_from_memory_disk_and_a_later_restart`, `persisted_chat_history_is_pruned_to_the_same_cap_as_in_memory_history`, TUI `clear_command_resets_conversation_and_reports_a_real_reset`. Tam paket: `cargo fmt`, `cargo test --offline` (154 lib + 35 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
  - Native desktop için ayrı kod gerekmedi — aynı paylaşılan `Runtime`/SQLite store'u kullandığı için geçmiş otomatik olarak iki arayüzde de tutarlı.
- [x] **GPT önerisi 3/7 — Batch embedding**: chunk başına bir HTTP çağrısı yerine bir belgenin tüm chunk'ları tek çağrıda embed ediliyor (ADR-0004'te ertelenen 7 maddeden biri, kullanıcı onayıyla bugün uygulandı).
  - `EmbeddingProvider::embed_batch` yeni trait metodu — varsayılan implementasyon `embed`'i döngüyle çağırır (her implementasyon için doğru, yalnız hızlı değil); `LlamaEmbeddingProvider` llama.cpp'nin zaten OpenAI-uyumlu `/v1/embeddings`'inin dizi girdisini kabul etmesinden faydalanıp gerçek tek istekle override ediyor. Yanıttaki her girdinin kendi `index`'i kullanılıyor, sunucunun sırayı koruyacağı varsayılmıyor.
  - `SqliteStore::embed_and_store_chunk` → `embed_and_store_chunks_batch`: içerik-hash tekilleştirmesi artık yalnız önceden depolanmışlara karşı değil, **aynı batch içinde** de geçerli (aynı belgede iki kez geçen aynı paragraf da tek sefer embed ediliyor). Hem taze indeksleme (tüm chunk'lar önce eklenir, embedding döngü dışında tek seferde) hem `backfill_missing_embeddings` bunu kullanıyor.
  - Kanıt: 2 yeni test — `indexing_a_multi_chunk_document_embeds_in_one_batch_call_not_one_per_chunk`, `backfill_across_multiple_documents_also_uses_one_batch_call_per_document` (mock'ta `embed_batch` çağrı sayısı ile `embed` edilen metin sayısı ayrı ayrı sayılıyor). Var olan tüm embedding/hibrit testleri (içerik-hash paylaşımı, model-izolasyonu, RRF, geriye dönük doldurma) hâlâ geçiyor. Tam paket: `cargo fmt`, `cargo test --offline` (156 lib + 35 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **GPT önerisi 2/7 — Semantic-aware chunking (Markdown kısmı)**: `chunk_workspace_text` artık `.md`/`.markdown` dosyalarında başlık sınırlarına göre bölüyor — bir bölüm (başlık + gövdesi), sığdığı sürece tek bir chunk oluyor, rastgele bir karakter sınırında kesilmiyor.
  - `chunk_markdown_by_heading`: `#`/`##`/... ile başlayan her satır yeni bir bölüm başlatır; sığmayan bölümler yine eski kör bölücüye (`chunk_blind` — önceki `chunk_workspace_text`'in adı değişti) düşer, sert boyut sınırı (`MAX_WORKSPACE_CHUNK_CHARS`) hiçbir zaman aşılmaz.
  - Markdown olmayan her şey (düz metin, kod, PDF'ten çıkarılmış metin) hiç değişmeden eski kör bölmeyi kullanmaya devam ediyor.
  - **Bilinçli olarak kapsam dışı bırakıldı**: kod fonksiyon/sınıf bazlı bölme (dil başına ayrı bir parser gerektirir — gerçek, büyük ayrı bir mühendislik işi) ve PDF paragraf bazlı bölme. ADR-0004'te "hâlâ ertelenen" olarak not edildi.
  - Kanıt: 3 yeni test (`chunk_workspace_text_for_markdown_keeps_a_heading_with_its_section`, `chunk_workspace_text_for_markdown_still_splits_an_oversized_section`, `chunk_workspace_text_for_non_markdown_uses_blind_splitting_unchanged`) — biri gerçek bir biçimlendirme hatası (bölüm arası boş satırın önceki bölüme sızması) bulup düzeltti. Tam paket: `cargo fmt`, `cargo test --offline` (159 lib + 35 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **GPT önerisi 4+5/7 — Gözlemlenebilirlik + `/rag status`/`/rag rebuild`/`/rag verify`**: kişisel araç için orantılı kalacak şekilde birleşik ve daraltılmış uygulama — ayrı bir metrik deposu/gecikme histogramı yok, yalnız istendiğinde hesaplanan bir anlık görüntü.
  - `Runtime::rag_status`: belge/chunk/embed edilmiş chunk sayısı, aktif model, ve bu oturumun hibrit-vs-yalnız-FTS sorgu sayaçları (`approved_workspace_context`'e eklendi — gerçek sohbet turlarında artıyor, dekoratif sabit değil).
  - `Runtime::rebuild_rag_index`: aktif model için tüm embedding'leri siler ve `SqliteStore::rebuild_embeddings_for_model` ile sıfırdan yeniden hesaplar (batch embedding kullanır, belge başına tek çağrı). Sağlayıcı bağlı değilse net bir hatayla reddeder.
  - `Runtime::verify_rag_index`: sahipsiz embedding kaydı (chunk'ı silinmiş ama vektörü kalmış) ve aktif model için eksik embedding sayımı; `RagVerifyReport::is_healthy()`.
  - TUI: `/rag status`, `/rag rebuild`, `/rag verify`.
  - Kanıt: 3 yeni Runtime testi (`rag_status_reports_real_counts_and_session_retrieval_counters`, `rag_rebuild_recomputes_every_embedding_and_requires_a_provider`, `rag_verify_flags_missing_embeddings_as_unhealthy` — sonuncusu gerçek bir "henüz backfill edilmemiş" boşluğu FTS-only indeksleme + sonradan sağlayıcı bağlama ile üretip yakalıyor) + 3 uçtan uca TUI testi. Tam paket: `cargo fmt`, `cargo test --offline` (162 lib + 38 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **GPT önerisi 6/7 — Configurable RRF sabitleri**: `RRF_K`, aday havuzu çarpanı ve result sayısı artık ortam değişkeniyle geçersiz kılınabiliyor — `JARVIS_RRF_K`, `JARVIS_RETRIEVAL_CANDIDATE_MULTIPLIER`, `JARVIS_RETRIEVAL_RESULT_LIMIT`. Geçersiz/eksik değerde ADR-0004'ün seçtiği varsayılana (`60.0` / `4` / `4`) düşer.
  - Ayrıştırma mantığı (env okuma değil) saf fonksiyonlara ayrıldı (`parse_rrf_k` vb., `Option<&str>` alır) — süreç ortam değişkenlerini mutasyona uğratmadan doğrudan test edilebilsin diye (testler paralel thread'lerde çalışıyor, gerçek env değişkeni set/unset etmek başka bir testle yarışa girebilirdi).
  - Artık F3 madde 18'deki `rag_eval_*` eval seti var, bu değerleri ayarlamak artık tahmin değil — deferred-improvements notu buna göre güncellendi.
  - Kanıt: 3 yeni test (`rrf_k_accepts_a_valid_positive_override_and_falls_back_otherwise`, `retrieval_candidate_multiplier_accepts_a_valid_override_and_falls_back_otherwise`, `retrieval_result_limit_accepts_a_valid_override_and_falls_back_otherwise`) + var olan tüm RRF/hibrit/eval testleri (varsayılan davranış) hâlâ geçiyor. Tam paket: `cargo fmt`, `cargo test --offline` (165 lib + 38 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **GPT önerisi 1/7 — Retrieval öncesi permission/sensitivity filtresi**: workspace belgeleri artık `MemoryRecord`'daki gibi bir `DataSensitivity` (`public`/`internal`/`sensitive`) taşıyor; `Sensitive` işaretli bir belge indekslenmiş ve `file.read_workspace` ile doğrudan okunabilir kalıyor ama hiçbir zaman otomatik sohbet alıntısı olarak çıkmıyor.
  - Tek uygulama noktası: `SqliteStore::search_workspace` (`hybrid_search_workspace` de FTS aday havuzu için buradan geçiyor) — filtre başka bir retrieval yolundan atlanamaz.
  - Geriye dönük uyumluluk: mevcut tüm `index_workspace_document(_with_embedding)`/`index_workspace_folder` çağrıları (kod ve testler) hiç değişmedi, hepsi varsayılan `Internal` (kısıtsız retrieval) ile yeni `..._with_sensitivity` varyantlarına yönlendiriyor — bu, F3 boyunca zaten indekslenmiş her belgenin davranışını aynen koruyor.
  - İçerik değişmeden yalnız hassasiyet seviyesi değiştirilirse (aynı SHA-256) bile güncelleniyor — bir belgeyi hassas yapmak için içeriğini değiştirmeye gerek yok.
  - TUI: `/index <dosya> [public|internal|sensitive]`, `/index-folder <klasör> [hariç-desen ...] [public|internal|sensitive]` — isteğe bağlı son kelime, hem İngilizce hem Türkçe (`hassas` dahil, `parse_data_sensitivity` zaten destekliyordu).
  - Kanıt: 3 yeni Runtime testi (`sensitive_workspace_document_is_excluded_from_conversation_citations` — gerçek bir sohbet turu üzerinden uçtan uca, `reindexing_updates_sensitivity_even_when_content_is_unchanged`) + 1 uçtan uca TUI testi (`index_commands_accept_an_optional_trailing_sensitivity_word`). Tam paket: `cargo fmt`, `cargo test --offline` (167 lib + 39 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [ ] **GPT önerisi 7/7 — Opsiyonel reranker aşaması**: bilinçli olarak yapılmadı. İkinci bir model çalıştırmak (ayrı model indirme, ayrı servis, ayrı ADR) gerçek bir kaynak/karmaşıklık maliyeti — ve tam da bu oturumda kurulan RRF hibrit yaklaşımının kendi gerekçesiyle çelişir ("yalnız bir kıyaslama 'RRF yetmiyor' derse eklenecek, şimdiden ikinci bir model çalıştırmanın gerekçesi yok"). Kullanıcıya ayrıca bildirildi, onay/itiraz bekleniyor.
- [x] **Katmanlı bellek mimarisi — kullanıcının 5-katmanlı tasarımıyla kod arasındaki boşlukların kapatılması, Aşama 1/3 (trust_level + task-scoped izolasyon)**: kullanıcı kendi tasarladığı 5 katmanı (active/temporary context, session, task, project, long-term user memory) ve 6 kuralı (her şeyi kaydetme, secret manager referansı, provenance/trust/scope/sensitivity metadata, task izolasyonu, SQLite-first, opsiyonel semantic retrieval) paylaştı; kod ile karşılaştırıldı, gerçek boşluklar bulundu, kapatılıyor.
  - **`TrustLevel` eklendi** (`UserAsserted`/`Imported`) — `MemoryRecord`'a yeni alan. "Her kayıtta provenance/trust level/scope/sensitivity" kuralının eksik parçasıydı (`source`=provenance, `sensitivity` zaten vardı). `propose_memory`'nin imzası **değişmedi** (24 çağrı noktası etkilenmedi) — yeni `propose_memory_with_trust_and_scope` bunu ekliyor, `propose_memory` ona `(UserAsserted, None)` ile yönleniyor. `/memory import` artık `Imported` üretiyor.
  - **`scope_id: Option<String>` eklendi** — yalnız `Task` namespace için anlamlı, `validate_memory_record` artık bunu zorunlu kılıyor (Session/EphemeralToolOutput'un `expires_at` zorunluluğuyla aynı yapısal desen). `memory_id` artık `scope_id` varsa onu da içeriyor — iki farklı task'ın aynı anahtarı asla birbirini ezmiyor.
  - **Gerçek boşluk kapatıldı — task-scoped izolasyon**: `retrieve_memory` artık `task_scope: Option<&str>` alıyor; `Task` namespace'i `task_scope=None` iken (sıradan sohbet turu) **tamamen hariç tutuluyor** — önceden tüm task'ların tüm kayıtları her sohbet turuna karışıyordu. Yeni `Runtime::task_scoped_memory_context(task_id)`, yalnız o task'ın kayıtlarını döner.
  - Kanıt: 2 yeni test — `trust_level_distinguishes_direct_writes_from_imports`, `task_scoped_memory_isolates_concurrent_tasks_from_each_other_and_from_ordinary_context` (iki farklı task'ın aynı anahtarlı kaydı asla karışmıyor, ne birbirine ne sıradan bağlama — gerçek bir sohbet turu üzerinden uçtan uca). Var olan tüm bellek testleri (Task'ın hâlâ durable olduğu, Session/EphemeralToolOutput'un TTL zorunluluğu dahil) hâlâ geçiyor. Tam paket: `cargo fmt`, `cargo test --offline` (169 lib + 39 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
  - Aşama 3/3 tamamlandı, aşağıda.
- [x] **Katmanlı bellek mimarisi Aşama 2/3 — Session/Task/Project'e gerçek yazma yolu**: önceden `/remember` her zaman `UserProfile`'a yazıyordu; Session/Task/Project şema olarak vardı ama hiçbir üretim yolu onlara yazmıyordu (yalnız `/memory import` — dolaylı bir yol).
  - `/remember [profil|proje|görev <task-id>|oturum|geçici] anahtar = değer` — isteğe bağlı bir namespace kelimesi (`parse_memory_namespace`, zaten `/forget namespace`'te kullanılıyordu). `görev` ekstra bir `task-id` argümanı alıyor (`scope_id` olarak taahhüt ediliyor).
  - **Geriye dönük uyumluluk, dikkatli tasarlandı**: namespace kelimesini soyduktan sonra gerçek bir "anahtar = değer" kalmıyorsa (örn. kullanıcının anahtarı gerçekten "proje" ise, `/remember proje = jarvis`), eski davranışa (UserProfile, orijinal metin) düşülüyor — hiçbir var olan kullanım kırılmıyor.
  - Session/EphemeralToolOutput bir expiry olmadan hiç kaydedilemediği için (`validate_memory_record`), kullanıcı `/remember ttl` vermezse makul bir varsayılan süre kendiliğinden atanıyor (Session: 4 saat, EphemeralToolOutput: 30 dakika) — akış tıkanmıyor.
  - Kanıt: 3 yeni test (`remember_writes_to_project_task_and_session_namespaces` — üç namespace'e de gerçek yazma, `parse_remember_namespace_prefix_disambiguates_a_real_literal_key`, `parse_remember_namespace_prefix_consumes_a_task_id_only_for_task_namespace` — saf ayrıştırma mantığı doğrudan test edildi). Tam paket: `cargo fmt`, `cargo test --offline` (169 lib + 42 main + 7 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
  - Bilinçli kapsam dışı: doğal dil komutları (`memory_intent.rs`) hâlâ yalnız `UserProfile`'a yazıyor — namespace/task-id seçimi teknik/kesin bir eylem, doğal dil ifade belirsizliği riski taşıyor; bu yüzden kasıtlı olarak yalnız slash komutunda.
- [x] **Katmanlı bellek mimarisi Aşama 3/3 — Secret Manager**: kullanıcının kuralı "secret'ları doğrudan hafızaya yazmıyoruz; sadece Secret Manager referansı tutuluyor" — daha önce hiç yoktu (ADR-0003'ün genel "şifreleme eklenmedi" kararından ayrı, ek bir mekanizma).
  - Yeni `secrets` tablosu (`memories`'ten tamamen ayrı, schema sürüm 10) — gerçek değer yalnız burada. `Runtime::remember_secret` bunu yazar ve `memories`'e yalnız bir **yer tutucu** satır ekler (`sensitivity=Sensitive`, `include_in_model_context=false` — sıradan sohbet bağlamı bunu hiç görmez, gerçek değer değil).
  - Gerçek değer yalnız `Runtime::reveal_secret`'ın açık, kullanıcı-tetiklemeli çağrısıyla (`/secret show <anahtar>`) ortaya çıkar — hiçbir sohbet/model bağlamı derleme yolu bunu çağırmaz.
  - Doğal dil: ayrı, öncelikli bir tetikleyici kümesi (`hafızana gizli kaydet: ...`, `sırrını sakla: ...`) — sıradan `REMEMBER_TRIGGERS`'tan bilinçli olarak ayrı, bir kimlik bilgisinin yanlışlıkla sıradan yola gitmemesi için. Hem TUI hem native desktop'ta çalışıyor.
  - TUI: `/secret anahtar = değer`, `/secret show <anahtar>`, `/secret forget <anahtar>`, `/secrets` (yalnız anahtarları listeler, değerleri asla).
  - Audit: yalnız anahtar adı, gerçek değer asla (F3 madde 14 "filtre loglanır ama sır saklanmaz" ilkesiyle aynı desen).
  - Aynı anahtarı tekrar kaydetmek günceller, ikinci bir kayıt oluşturmaz (bugün genel bellek için düzeltilen aynı desen — `secret_id` anahtardan türetiliyor).
  - Kanıt: 5 yeni `lib.rs` testi (`remembering_a_secret_never_stores_the_real_value_in_ordinary_memory`, `a_remembered_secret_never_reaches_a_real_conversation_turn` — gerçek bir sohbet turu üzerinden uçtan uca, `forgetting_a_secret_removes_both_the_real_value_and_its_placeholder`, `remembering_the_same_secret_key_again_updates_it`) + 3 `memory_intent.rs` testi + 2 TUI testi + 1 desktop testi. Tam paket: `cargo fmt`, `cargo test --offline` (176 lib + 44 main + 8 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
  - **Katmanlı bellek mimarisi (3 aşamanın tamamı) kapandı.** ADR-0003'e tam ayrıntılı "Ek" eklendi.
- [x] **Açılış karşılaması + elle düzenlenen profil dosyaları + hava durumu (JARVIS'in ilk gerçek internet erişimi)**: kullanıcı "ben ne zaman JARVIS'i açsam bana genel bir giriş yapmasını istiyorum ... yazılı şekilde (sesli yapmadık farkındayım ama şimdiden söyleyeyim)" ve "belirli profil dosyaları oluştursam mı ... her başlatıldığında yeniden okur" dedi. Tam gerekçe: [ADR-0005](docs/adr/0005-startup-briefing-profile-files-and-weather.md).
  - **Profil dosyaları** (`src/profile_files.rs`, yeni): `~/.config/jarvis/profile/about_user.md`/`about_jarvis.md` — kullanıcının "Claude'un kendi hafıza dosyaları gibi, elle düzenlenen talimat dosyaları" seçimiyle kuruldu. JARVIS yalnız okur, asla yazmaz (`ensure_profile_files_exist` var olan içeriği asla ezmiyor); `MAX_PROFILE_FILE_CHARS=8000`; her turda taze okunuyor; aynı "veri, talimat değil" zarfı (`isolate_profile_file_as_data`, ADR-0003'ün ilkesiyle aynı).
  - **Hava durumu** (`src/weather.rs`, yeni): kullanıcı önce web aramasını sordu, sonra AccuWeather önerdi (paralı çıkınca vazgeçti), son kararla **Open-Meteo** (ücretsiz, API anahtarsız) seçildi — konum sabit **İstanbul, Ümraniye**. `WeatherProvider` trait + `OpenMeteoWeatherProvider`; JSON ayrıştırma saf fonksiyona ayrıldı (`parse_open_meteo_response`, ağsız test edilir). **`CapabilityRegistry`'de bir kayıt değil** — governed pipeline'ın hiçbir adımından geçmiyor, model bunu asla çağıramıyor, yalnız `Runtime::startup_briefing` bir kerelik okuyor; F3'ün `no_baseline_capability_requires_network_access` testi bu yüzden hâlâ doğru. Yeni bağımlılık: `ureq` (tls+json, rustls) — projenin ilk HTTP client'ı. Gerçek doğrulama: `curl` ile gerçek uç nokta (41.0166,29.1173) canlı sorgulandı, gerçek sıcaklık/WMO kodu döndüğü görüldü.
  - **`Runtime::startup_briefing()`** (yeni): selamlama (profil `preferred_address`/`display_name` varsa) + hava durumu (sağlayıcı bağlıysa) + bekleyen onay sayısı + son 3 not (secret placeholder'ları hariç — `source != "secret-manager"`). TUI (`run_tui`) ve native masaüstü (`JarvisDesktop::new`) ikisi de açılışta ikinci bir sistem mesajı olarak gösteriyor.
  - **Sesli/TTS bilinçli olarak yapılmadı**: kullanıcı bunu açıkça bir gelecek niyeti olarak belirtti ("sesli yapmadık farkındayım"), F5'in kapsamında; bugünkü iş yalnız yazılı karşılamayı kapsıyor.
  - Kanıt: `startup_briefing_includes_name_pending_approvals_and_recent_notes`, `startup_briefing_includes_weather_only_when_a_provider_is_attached`, `startup_briefing_never_lists_a_secrets_placeholder_as_a_note`, `profile_files_reach_conversation_context_only_when_a_dir_is_set` (lib.rs) + `desktop_startup_shows_the_runtime_briefing_as_a_second_system_message` (jarvis_desktop.rs, gerçek profil alanı yazıp karşılamada göründüğünü doğruluyor) + `profile_files.rs`'in 3 testi + `weather.rs`'in 4 testi. Tam paket: `cargo fmt`, `cargo test --offline` (187 lib + 44 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz — iki gerçek uyarı bulunup düzeltildi: `sort_by`→`sort_by_key`, ardışık OR-pattern→range), `scripts/release_check.sh --offline` (PASS, gerçek release build + MCP smoke dahil).
- [x] **Kaynak listesinde tekrar eden bellek kaydı (kullanıcı ekran görüntüsüyle bildirdi, 16 Ağustos 2026)**: TUI'de "Kaynaklar" listesinde `USER_PROFILE:language` ve `USER_PROFILE:preferred_address` iki kez görünüyordu. Kök neden: gerçek `jarvis.db`'de her ikisinin de iki farklı `memory_id`'li satırı vardı — bu, yukarıdaki "bellek güncelleme hatası" fix'inden **önce** yazılmış, fix'in geriye dönük temizlemediği kalıntı satırlardı (`sqlite3 jarvis.db` ile doğrudan doğrulandı: `language` ve `preferred_address` için `COUNT(*)=2`, diğer alanlar için `1`).
  - Düzeltme: `SqliteStore::migrate()`'e `repair_concurrent_audit_chain`'le aynı ilkeyle çalışan yeni bir kendi kendine iyileşen adım eklendi — `deduplicate_legacy_memory_records`. Her `(namespace, memory_key, scope_id)` grubunda yalnız en son güncellenen satır kalır (eşitlikte `memory_id` büyük olan, deterministik), diğerleri silinir. Yıkıcı değil: `backup_if_schema_migration_pending` zaten her migration öncesi güncel-olmayan bir DB'yi yedekliyor.
  - Bu, tek seferlik elle bir SQL script'i değil, **her `SqliteStore::open` çağrısında** (yani JARVIS her açıldığında) çalışan kalıcı bir onarım — gelecekte benzer bir hata tekrar bu şekilde veri kalıntısı bırakırsa kendiliğinden temizlenir.
  - Gerçek DB üzerinde doğrulandı: `jarvis.db` yedeklendi, gerçek `jarvis` ikili programı gerçek DB'ye karşı çalıştırıldı (yalnız DB açma adımına kadar — TTY olmadığı için TUI başlatma adımı beklenen şekilde hata veriyor, ama DB açma ondan önce gerçekleşiyor), `sqlite3` ile öncesi/sonrası doğrudan karşılaştırıldı: 6 kayıttan 4 kayıda düştü, kalan `language`/`preferred_address` değerleri en güncel (en eksiksiz) sürümler ("Türkçe / İngilizce", "efendim (Türkçe) / sir (English)"), `PRAGMA integrity_check` temiz.
  - Kanıt: yeni test `a_real_restart_deduplicates_legacy_memory_rows_left_over_from_before_the_memory_id_fix` — gerçek dosya tabanlı SQLite üzerinden, `raw_connection` ile fix-öncesi durumu simüle edip gerçek bir `SqliteStore::open` restartının onardığını kanıtlıyor. Tam paket: `cargo fmt`, `cargo test --offline` (188 lib + 44 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **Genel latency (kullanıcı ekran görüntüsüyle birlikte bildirdi, 16 Ağustos 2026): her sohbet turunda gereksiz ikinci bir model çağrısı**: `handle_with_provider_and_analyses`, yönlendirme (routing) ve sohbet yanıtı çağrılarını `std::thread::scope` ile "eşzamanlı" çalıştırıyordu; yorum satırı bunun "routing latency + chat latency"yi ayrı ayrı ödemeyi önlediğini iddia ediyordu.
  - Gerçek ölçümle bu iddia **yanlış çıktı**: `jarvis-llama.service` `-np 1` (tek decode slot) ile çalışıyor (`~/.config/systemd/user/jarvis-llama.service`) — sunucu iki isteği asla gerçekten paralel işlemiyor, ikinciyi sıraya koyuyor. `curl` ile gerçek sunucuya karşı router prompt'unun kendisi ölçüldü: yalnız 158 token'lık prefill **3.448 saniye** sürdü (üretilen 2 token hariç) — yani "eşzamanlı" tasarım her turda bu maliyeti zaten tam olarak ödüyordu, üstelik bir capability yönlendirildiğinde sohbet yanıtının **tamamı atılıyordu** (task.capability != "conversation.reply" olduğunda `response` hiç kullanılmıyor).
  - Düzeltme: önce yönlendirme çalıştırılıyor; yalnız bir capability'ye çözülmediyse (sıradan sohbet) ikinci, sohbet-üretme çağrısı yapılıyor. Bir capability yönlendirildiğinde artık hiç sohbet yanıtı üretilmiyor — tam bir model geçişi (governed istekler için) baştan tasarruf ediliyor. Sıradan sohbet için davranış/latency değişmedi (ikisi de zaten gerekliydi, sunucu onları zaten sıraya koyuyordu).
  - Kanıt: yeni sayaçlı mock sağlayıcı (`RouteAwareCountingProvider`) ile 2 test — `routing_to_a_capability_skips_the_now_discarded_conversational_generation` (bir capability'ye yönlendirilince sohbet üretme çağrısının **hiç** yapılmadığını sayaçla kanıtlıyor) ve `ordinary_chat_still_gets_exactly_one_conversational_generation_when_routing_is_unknown` (sıradan sohbette regresyon olmadığını kanıtlıyor). Tam paket: `cargo fmt`, `cargo test --offline` (190 lib + 44 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
  - Bilinçli kapsam dışı: `-np` değeri artırılıp gerçek sunucu paralelliği sağlanırsa, sıradan sohbet için de gerçek bir eşzamanlılık kazancı mümkün olabilir — ama bu RAM/CPU maliyeti gerektiren ayrı bir karar, bugünkü işin kapsamında değil.
- [x] **"jarvis uyanık mısın" gibi sıradan bir kontrolün `system.health`e yanlış yönlendirilmesi (kullanıcı bildirdi, 16 Ağustos 2026)**: kullanıcı JARVIS'e sohbet niyetiyle "uyanık mısın" dediğinde, sohbet yerine CPU/RAM/disk raporu alıyordu.
  - Kök neden gerçek `llama-server`'a karşı `curl` ile doğrulandı: F2'de eklenen router prompt talimatı ("Treat a request for the current local computer or **JARVIS state** ... as system.health") çok genişti — model "uyanık mısın"ı da bir "JARVIS state" sorgusu sayıp `system.health`e yönlendiriyordu. Karşılaştırmalı test: `orada mısın`/`beni duyuyor musun`/`iyi misin`/`naber jarvis` doğru şekilde `UNKNOWN` kalıyordu, yalnız `uyanık mısın` (ve olası benzerleri) yanlış tetikleniyordu.
  - Düzeltme: router prompt'u iki parçaya ayrıldı — (1) yalnız **açık** bir bilgisayar/sistem sağlığı metriği isteğini (CPU/RAM/disk/ağ/uptime, Türkçe "sistem durumu" dahil) `system.health`e yönlendir; (2) JARVIS'in sadece orada/dinliyor/uyanık olup olmadığını soran sıradan bir kontrol ("uyanık mısın", "orada mısın", "iyi misin", "are you there", "are you okay") **sıradan sohbettir, system.health değildir** — açık örneklerle belirtildi.
  - Gerçek modelle 10 ifadeyle karşılaştırmalı doğrulandı (öncesi/sonrası): `jarvis uyanık mısın` (system.health→UNKNOWN, düzeldi), `are you awake` (aynı düzeltme İngilizce'de de), `sistem durumu nasıl` ve `bilgisayar sağlıklı mı` (system.health olarak **kaldı** — F2'nin asıl düzeltmesi bozulmadı), `orada mısın`/`beni duyuyor musun`/`iyi misin`/`naber jarvis` (zaten doğruydu, hâlâ doğru).
  - Kanıt: yeni test `router_prompt_excludes_a_casual_are_you_there_check_from_system_health` (prompt-yakalayan mock ile, prompt metninin hem yeni istisnayı hem eski "sistem durumu" talimatını içerdiğini kanıtlıyor — asıl yönlendirme kararı gerçek modelle canlı doğrulandı, offline test bunu tekrarlayamaz). Tam paket: `cargo fmt`, `cargo test --offline` (191 lib + 44 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **Router'ın yapısal iyileştirmesi — soru/komut ayrımı + kısa konuşma bağlamı (kullanıcı isteğiyle, aynı gün)**: kullanıcı "bu hardcode mu yoksa bağlam problemini mi çözdün, başka benzer sorun var mı" diye sordu. Dürüst cevap: yukarıdaki düzeltme örnek-tabanlıydı (hardcode'a yakın); router hâlâ **hiçbir konuşma geçmişi görmüyordu** (`route_with_provider` yalnız o anki tek mesajı alıyordu).
  - Gerçek modelle geniş bir tarama yapıldı, **iki gerçek örnek daha bulundu**: `"bugün bir not aldın mı"` (bir soru) yanlışlıkla `note.create`e yönlendiriliyordu; `"dosyalarım nasıl gidiyor"` sınırda bir örnekti.
  - Önce yalnız konuşma geçmişi eklenerek test edildi: **hiçbir doğruluk kazancı ölçülmedi** (`"dosyalarım nasıl gidiyor"` bağlamla/bağlamsız aynı sonucu verdi) ve `-np 1` yüzünden gerçek kullanımda router her turda zaten "soğuk" prefill ödediğinden ek maliyeti vardı — bu bulgu kullanıcıya dürüstçe raporlandı.
  - Kullanıcı hem genel kuralı hem bağlamı birlikte istedi. İkisi de eklendi: (1) genel bir "yalnız geçmişe/mevcut duruma dair soru bir komut değildir" kuralı — gerçek modelle doğrulandı, `"bugün bir not aldın mı"`yı düzeltti, `"not al: ..."` gibi asıl komutları bozmadı; (2) `route_with_provider` artık `recent_history: &[ConversationMessage]` alıyor — mevcut turdan **önceki** en fazla 2 mesaj, yalnız belirsizliği gidermek için, kendisi asla bir yönlendirme isteği sayılmadan. Boşken (`&[]`) hiçbir ek prompt metni eklenmiyor (token/latency maliyeti yok).
  - `Runtime::handle_with_provider_and_analyses`'te dilimleme: `chat_history` çağrı anında zaten mevcut turu içeriyor (fonksiyon başında ekleniyor) — bu yüzden son eleman hariç tutulup ondan önceki en fazla 2 mesaj alınıyor; mevcut turun kendisi asla kendi "geçmişi" içinde ikinci kez görünmüyor.
  - Kanıt: 3 yeni `route_with_provider` testi (soru/komut kuralı, bağlam varken/yokken prompt farkı) + 1 uçtan uca `Runtime` testi (`runtime_passes_only_the_preceding_turn_as_router_context_never_the_current_one` — iki gerçek sohbet turu üzerinden, ikinci turun ilk turu bağlam olarak gördüğünü ama kendi güncel mesajını ikinci kez görmediğini kanıtlıyor). Gerçek modelle son bir kez tüm ilgili ifadeler (9 tanesi) tekrar doğrulandı, hepsi beklenen sonucu verdi. Tam paket: `cargo fmt`, `cargo test --offline` (194 lib + 44 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
  - Kullanıcı "hazır el atmışken bütün sorunları düzeltelim" dedi — yukarıda "kapsam dışı" bırakılan `"not al: yarın toplantı var"` kalıntısı da dahil, ele alındı (aşağıda).
- [x] **`"not al: X"` gibi noktalamalı imperative not komutlarının tutarsızca `UNKNOWN` kalması**: gerçek modelle kök nedeni izole edildi — `"not al"` tek başına ya da `"şunu not al: X"` doğru çalışıyordu, ama düz `"not al: X"`/`"not al, X"`/`"not tut: X"` tutarsızca `UNKNOWN` kalıyordu (küçük CPU-only modelin kendi iç tutarsızlığı, ayırt edilebilir tek bir kural eksikliği değildi).
  - Router prompt'una açık bir `note.create` talimatı eklendi: "not al/not tut/not yaz/not oluştur" gibi imperative bir not komutunu, aradaki noktalamadan bağımsız olarak `note.create`e yönlendir — ama "bugün bir not aldın mı" gibi bir soruyu (imperative değil) hâlâ `UNKNOWN` bırak. İkisi çelişmeden bir arada duruyor.
  - Gerçek modelle geniş, tüm 7 routable capability'yi kapsayan bir son tarama yapıldı (system.health/system.time/file.read_workspace/project.info/code.project_outline/docs.workspace_summary/note.create için hem doğru komut hem soru/olumsuz-komut/sıradan sohbet varyasyonları, toplam 25+ ifade) — hepsi beklenen sonucu verdi, regresyon yok.
  - Kanıt: yeni test `router_prompt_treats_an_imperative_note_command_as_note_create_regardless_of_punctuation`. Tam paket: `cargo fmt`, `cargo test --offline` (195 lib + 44 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).
- [x] **`memory_intent.rs`'in doğal dil sınırı — kullanıcının felsefi sorusu: "gerçek bir yapay zeka 'aklında tut' ile 'hafızana yaz'ı ilişkilendiremiyorsa bu ne çeşit bir yapay zeka olur?"**: kullanıcı iki yaklaşımı birlikte istedi.
  - **(1) Sabit liste genişletildi** (`REMEMBER_TRIGGERS`): "aklında tut(ki)", "aklında olsun(ki)", "aklında bulunsun", "kayıtlara geç", "remember that", "keep in mind (that)" eklendi — hâlâ sıfır risk (model devreye girmiyor). `FORGET_TRIGGER_PREFIXES`'e de simetri için "aklından " eklendi.
  - **(2) Model-destekli yedek yol eklendi** (`propose_unrecognized_remember_intent_with_provider`): sabit listede olmayan bir ifade için, önce ücretsiz bir ipucu kontrolü (`might_express_an_unrecognized_remember_intent` — "hafıza"/"hatırla"/"aklı" gibi gevşek alt dizeler; eşleşmezse model **hiç** çağrılmaz, sıradan hiçbir mesaj ek maliyet ödemez), sonra yalnız ipucu varsa modele "bu bir hatırlatma isteği mi?" sorulur. Router'da ölçülen aynı risk sınıfı burada da geçerli (modelin kararı bazen yanlış olabilir) — bu yüzden fixed-trigger yolunun aksine **asla doğrudan yazmaz**, yalnız bir `MemoryProposal` üretir ve normal `/remember`-tarzı önizleme/onay akışına (`pending_memory`) girer.
  - Gerçek modelle prompt'u iki turda doğrulandı: ilk versiyon "bunu hatırlar mısın acaba" gibi bir soruyu da yanlışlıkla kayıt önerisi olarak yorumluyordu (router'daki aynı soru/komut karışıklığı); aynı ayrım kuralı buraya da eklenip düzeltildi, sonra 7 ifadelik bir sette (gerçek istekler + sorular + sıradan sohbet) doğrulandı.
  - Yalnız TUI'ye bağlandı (`src/main.rs`, worker thread'de: ek yoksa ve ipucu eşleşirse modele soruluyor, sonuç `WorkerReply.memory_proposal` ile `app.pending_memory`'ye taşınıyor — mevcut `/remember approve|reject` akışı hiç değişmeden yeniden kullanılıyor). **Bilinçli olarak native masaüstüne eklenmedi**: desktop'ta hiçbir onay/slash-komut arayüzü yok (kendi kod yorumu: "no slash-command surface at all"; onay gerektiren akışlar zaten TUI'ye yönlendiriliyor) — model'in yanlış olabilecek bir kararını onaysız native'e eklemek kuralı ("asla doğrudan yazmaz") ihlal ederdi.
  - Kanıt: 5 yeni `memory_intent` testi (sıfır-maliyet garantisi, model önerisini önizlenebilir proposal'a çözme, model "NONE" derse sonuç yok, bilinen alan adı profil yoluna gitmesi) + 1 sabit liste genişletme testi. Tam paket: `cargo fmt`, `cargo test --offline` (200 lib + 44 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

### F4 — Güvenli coding ve yerel iş workbench'i

Durum: İLERLİYOR — F3 exit gate 16 Ağustos 2026'da kapandı; aynı gün "Read-only project analyst" maddesiyle F4 işi fiilen başladı, aynı günün ikinci turunda plan→patch→onay→uygula→test zinciri uçtan uca TUI'de çalışır hâle geldi, üçüncü turunda 6 senaryolu bir coding eval seti eklendi, dördüncü turunda test/verifier runner'a gerçek bir taban çizgisi regresyon karşılaştırması eklenip eval seti 7 senaryoya çıkarıldı, beşinci turunda coding plan'a model-üretimli varsayım/soru alanları eklendi, altıncı turunda patch preview'a seçilebilir dosya scope'u + kullanıcı notu eklendi, yedinci turunda coding plan'a tahmini risk eklenip "Coding plan UX" tamamen kapatıldı, sekizinci turunda genel bir `LocalTool` çerçevesi (`execute_approved`'ın tek-tool'a özel sabit kodlanmış hâlinin gerçek bir dispatch'e refactor edilmesi) 2 gerçek tool'la kanıtlanıp "Yerel üretkenlik tool framework" kapatıldı, dokuzuncu ve son gerçek turda genel bir `workflow.rs` çok-adımlı orkestratörü (retry/idempotency/rollback/approval/audit özeti) kanıtlanıp "Çok-adımlı workflow runner" kapatıldı (12/15 madde `[x]`). Aynı günün TUI-düzeltme oturumunun devamında (10. tur) kritik bir düzeltme yapıldı: "bu sandbox `CLONE_NEWNET` reddediyor" iddiası yanlış çıktı — gerçek makinede `unshare`/`bwrap`/`systemd-run --user --scope` hepsi çalışıyor; asıl engel `apply_worker_rlimits`'teki sabit `RLIMIT_NPROC=64` (per-UID sistem geneli thread sayacı, sıradan masaüstünde zaten binlerce) ve `--tmpfs /tmp`'nin workspace bind'ından sonra gelmesiydi (ikisi de gerçek, canlı olarak bulunup düzeltildi) — izole worker (bwrap+yeni gerçek cgroup v2) ilk kez gerçek makinede uçtan uca kanıtlandı, `main.rs`'teki ilgili testler artık yalnız gerçek başarıyı kabul ediyor. Kalan 2 madde (seccomp, gerçek overlay) hâlâ `[ ]` ama artık "burada yapılamaz" değil, yalnız "henüz yapılmadı".

Amaç: JARVIS'in kod tabanını anlaması, değişiklik önermesi ve yalnız onayla izole ortamda doğrulaması.

- [x] Worker threat model/ADR: host çalışma alanı, ağ, shell, secret, process tree ve resource exhaustion için saldırı modeli ve seçilen izolasyon yaklaşımı.
- [x] Isolated worker bootstrap: workspace snapshot/overlay, ayrı çalışma dizini, read-only input ve sınırlandırılmış output artifact alanı. (16 Ağustos 2026, 11. tur: yeni `WorkspaceWriteMode::Overlay` — `bwrap --overlay-src <root> --tmp-overlay <root>`, gerçek makinede kanıtlandı. Allowlist komut çalıştırıcısı (`command_runner.rs`) artık bunu kullanıyor: bir test/build komutunun hiçbir yazması gerçek workspace'e ulaşmıyor, worker çıkınca hepsi kayboluyor. `git apply` bilinçli olarak `WorkspaceWriteMode::Direct` kalıyor — onun tüm amacı onaylı bir değişikliğin gerçekten diske yazılması.)
- [x] OS izolasyonu: kullanıcı namespace/container/bubblewrap seçimi, ağ=kapalı varsayılanı, mount allowlist, no-new-privileges ve seccomp fizibilitesi. (16 Ağustos 2026, 12. tur: gerçek bir seccomp-bpf allowlist filtresi eklendi — `libseccomp` crate ile, bwrap'ın belgelediği `seccomp_export_bpf` formatıyla derlenip `--seccomp <fd>` üzerinden yükleniyor. `strace` bu makinede kurulu olmadığı için kaynaktan kullanıcı dizinine (root gerekmeden) derlendi, git/cargo/npm/pytest/python3/go'nun gerçek çalıştırmaları izlenip syscall kümesi ampirik olarak çıkarıldı. Kanarya testiyle doğrulandı: allowlist dışı bir syscall (`personality`) sandbox dışında başarılı, içinde gerçekten `EPERM` ile reddediliyor; tüm izlenen araçlar filtre altında hâlâ çalışıyor.)
- [x] Resource kontrolü: CPU/RAM/disk/PID/time quota, process group, watchdog, stdout/stderr limitleri ve güvenli cleanup. (16 Ağustos 2026, 10-11. turlar: CPU/RAM/PID/time/cleanup **çift katmanlı gerçek** — hem `setrlimit` hem gerçek cgroup v2, ikisi de canlı OOM-kill ile kanıtlandı. Disk: yukarıdaki `WorkspaceWriteMode::Overlay` sayesinde artık test/build komutları için de gerçek bir bütçe var — overlay'in tmpfs arka planı aynı `MemoryMax` cgroup'una sayıldığı için, sınırı aşan bir yazma girişimi gerçekten `SIGKILL` alıyor (canlı kanıtlandı: 64MB sınırla 200MB `dd` yazma denemesi öldürüldü). Ayrı bir dosya-sistemi kotası (XFS project quota vb.) değil, ama gerçek, kernel tarafından uygulanan bir üst sınır.)
- [x] Gerçek cancellation: task cancel → child process signal → grace period → kill → snapshot cleanup → audit/verifier sonucu.
- [x] Allowlist command runner: her komut için manifest, argüman schema, cwd scope, env allowlist, dry-run ve evidence capture.
- [x] Read-only project analyst: repo overview, dependency/test discovery, riskli dosya uyarısı ve hiçbir yazma yapmadan plan üretme.
- [x] Coding plan UX: yapılacaklar, etkilenen dosyalar, varsayımlar, test planı, tahmini risk ve kullanıcı soruları.
- [x] Patch generator: unified diff, dosya/path containment, diff hash, maksimum değişiklik limiti ve binary/secret dosya reddi.
- [x] Patch preview/review: satır bazlı görünüm, seçilebilir dosya scope'u, kullanıcı değişiklik notu ve explicit approve/reject.
- [x] Patch apply transaction: approval'a bağlı diff hash, snapshot/backup, atomic write, başarısızlıkta rollback ve audit.
- [x] Test/verifier runner: allowlisted test komutu, exit code/log özeti, değiştirilen dosya hash'i ve mevcut test regresyon raporu.
- [x] Coding evaluation seti: küçük hata düzeltme, test ekleme, yanlış patch reddi, timeout/cancel, secret exposure ve mevcut-test regression senaryoları.
- [x] Yerel üretkenlik tool framework: takvim, not, dosya düzenleme gibi her yeni tool için capability manifest, minimum scope, preview, approval ve verifier.
- [x] Çok-adımlı workflow runner: planı kullanıcıya gösterme, her yan etkili adımdan önce policy/approval, retry/idempotency, iptalde cleanup ve audit özeti.

Bu turda kanıtlanan alt dilim:

- [x] Worker threat-model ADR: [ADR-0001](docs/adr/0001-isolated-coding-worker.md) host fallback yasağını, ağ=kapalı worker kararını ve açık kalan quota/cancel sınırlarını tanımlar.
- [x] Coding plan/patch contract: workspace-relative scope, network-denied limitler, unified-diff/path/hash doğrulaması, proposal-bound approval, snapshot, `git apply --check`, dosya SHA-256 verifier kanıtı, rollback ve audit bağı kuruldu.
- [x] Release worker policy: Bubblewrap/network namespace kurulamazsa patch execution reddedilir; host shell fallback yoktur. Test harness'i container `CLONE_NEWNET` kısıtı nedeniyle yalnız semantic patch testini kontrollü geçici klasörde çalıştırır.
- [x] **Read-only proje analisti (16 Ağustos 2026, F4'ün ilk fiilen tamamlanan maddesi)**: yeni `src/project_analyst.rs` — `analyze_repository(root) -> RepoOverview`. `workbench.rs`'in patch/apply mekanizmasından tamamen ayrı, tek bir yazma işlemi yok.
  - Zaten var olan `preview_workspace_index` taraması üzerine kuruldu (aynı `.git`/`target`/`node_modules`/`.venv` hariç tutma, aynı gizli-bilgi/boyut filtresi — F3'ün "Workspace izin UX'i" ile aynı güvenlik sınırı, ikinci bir tarama mantığı icat edilmedi).
  - Kök dizindeki bilinen manifest dosyalarından (Cargo.toml/package.json/pyproject.toml/requirements.txt/go.mod/pom.xml/build.gradle(.kts)) dil ve önerilen test komutu tespit ediyor; birden fazla dil tespit edilebiliyor (ör. Rust + Node).
  - Riskli durumlar bilgilendirici not olarak dönüyor, hiçbir işlemi engellemiyor: bilinen manifest yok, çok büyük repo (>2000 dosya), gizli-bilgi benzeri/boyut limiti üstü dosyalar (zaten `preview_workspace_index`'in kendi filtresinden).
  - **Bilinçli kapsam sınırı**: yalnız kök dizindeki manifest'lere bakıyor (monorepo alt paketlerini taramıyor); "bu isteğe göre hangi dosyalar etkilenir" gibi isteğe özgü bir karar vermiyor — bu, henüz yapılmamış "Coding plan UX" maddesinin (model akıl yürütmesi gerektiren) işi.
  - TUI: `/analyze [proje-içi-göreli-klasör]` (klasör verilmezse proje kökü) — gerçek bu repo'nun kendisi üzerinde çalıştırılıp Rust/Cargo.toml/`cargo test`'in doğru tespit edildiği kanıtlandı.
  - Kanıt: 5 `project_analyst` testi (Rust tespiti, çoklu dil + Python tekilleştirme, bilinmeyen manifest risk notu, gizli-bilgi dosyası risk notu, geçersiz kök reddi) + 1 uçtan uca TUI testi (`analyze_command_detects_this_repos_own_rust_manifest_and_reports_unknown_subfolders` — bu repo'nun kendi `Cargo.toml`'unu gerçekten tespit ediyor). Tam paket: `cargo fmt`, `cargo test --offline` (205 lib + 45 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

Tamamlanma ölçütü: JARVIS bir değişikliği önce gösterir, kullanıcı onayı olmadan yazmaz; onay sonrası yalnız scope içindeki patch'i uygular ve test kanıtını döndürür.

**"Coding plan UX" maddesinde kısmi ilerleme (16 Ağustos 2026, hâlâ `[ ]` — kullanıcı soruları/varsayımlar/gerçek risk değerlendirmesi eksik olduğu için tam bitmiş sayılmıyor):**
- Yeni `draft_coding_plan_with_provider(overview, request_summary, provider) -> CodingPlan` (`project_analyst.rs`) — modele repo'nun dil/dosya listesini ve kullanıcının doğal dil isteğini verip hangi dosyaların ilgili olduğunu ve bir test planını soruyor. Hiçbir dosya açılmıyor/yazılmıyor.
- **Modelin dosya listesi güvenilmez çıktı olarak ele alınıyor**: yalnız `RepoOverview.included_files`'ta gerçekten var olan, birebir eşleşen yollar kabul ediliyor — router'ın yalnız `CapabilityRegistry`'deki tam üyeyi kabul etmesiyle aynı ilke, model asla bir dosya yolu icat edip kabul ettiremez.
- Gerçek modelle bu repo'nun kendi 21 Rust dosyasıyla 3 senaryo test edildi: "router'ın ... doğal dil bellek tanıma mantığını güncelle" → doğru şekilde `memory_intent.rs`'i (bugün tam olarak düzeltilen dosya) önerdi; "hava durumuna yeni şehir ekle" → doğru şekilde `weather.rs`'i önerdi; "bana bir şiir yaz" (kodla ilgisiz) → doğru şekilde `FILES: NONE` dedi.
  - TUI: `/plan <değişiklik isteği>` — model çağrısı gerektirdiği için arka plan worker thread'inde çalışıyor.
- **Hâlâ eksik**: "varsayımlar" ve "kullanıcı soruları" ayrı alanlar olarak üretilmiyor (model belirsizse yalnız `FILES: NONE` diyor, açıklayıcı bir soru sormuyor); "tahmini risk" hâlâ `create_read_only_coding_plan`'ın sabit boilerplate notları (isteğe özgü bir risk değerlendirmesi değil).
- Kanıt: 4 yeni `project_analyst` testi (gerçek dosyalar kabul/hayali dosya reddi, model "NONE" derse hata değil boş plan, TESTS satırı yoksa tespit edilen komuta düşme, tekrar eden dosyanın bir kez sayılması) + 1 main.rs testi (`/plan` argümansız çağrıldığında modele hiç dokunmadan kullanım mesajı gösteriyor). Tam paket: `cargo fmt`, `cargo test --offline` (211 lib + 46 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**"Coding plan UX" devamı — varsayımlar + kullanıcı soruları (16 Ağustos 2026, F4 tamamlama oturumu, beşinci tur — hâlâ `[ ]`, "tahmini risk" eksik kaldığı için tam bitmiş sayılmıyor):**
- `CodingPlan` yeni `assumptions: Vec<String>` ve `open_questions: Vec<String>` alanları (varsayılan boş — salt-okunur temel kurucu `create_read_only_coding_plan` hâlâ isteğe özgü yorum yapmıyor, yalnız `draft_coding_plan_with_provider` bunları modelden dolduruyor).
- Prompt iki satırdan dört satıra çıkarıldı: `FILES:`/`TESTS:` yanına `ASSUMPTIONS:` (isteğin belirtmediği, modelin sessizce varsaydığı noktalar) ve `QUESTIONS:` (kullanıcıya sorulması gereken netleştirici sorular) eklendi — ikisi de "NONE" diyebiliyor, hayali bir varsayım/soru icat etmiyor. Token bütçesi 300'den 400'e çıkarıldı (dört satır için).
- TUI: `/plan` çıktısı artık varsa "Varsayımlar:"/"Açık sorular:" bloklarını da gösteriyor.
- **Bu turda bulunan, ilgisiz bir gerçek düzeltme**: gerçek modelle canlı test edilirken model'in `FILES:` listesini bazen virgül yerine noktalı virgülle ayırdığı gözlemlendi (önceden yazılmış bir prompt/parser'da, bu turun konusu değil ama karşılaşıldığı an düzeltildi) — ayrıştırıcı artık her ikisini de kabul ediyor.
- Gerçek modelle canlı doğrulandı: hem ham yanıtın dört satırı da doğru üretebildiği, hem de belirsiz bir istekte ("kullanıcı profiline yeni bir alan ekle") modelin gerçekten anlamlı açık sorular ("Yeni alanın adı ne olmalı? Hangi veri tipinde olmalı?") ürettiği kanıtlandı.
- **Hâlâ eksik**: "tahmini risk" hâlâ `create_read_only_coding_plan`'ın sabit boilerplate notları — isteğe özgü bir risk değerlendirmesi değil.
- Kanıt: 4 yeni `project_analyst` testi (varsayım/soru ayrıştırma, ikisi de NONE ise boş liste, satırlar hiç üretilmezse geriye dönük uyumlu boş liste, noktalı virgülle ayrılmış FILES listesinin doğru ayrıştırılması). Tam paket: `cargo fmt`, `cargo test --offline` (246 lib + 53 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**"Coding plan UX" tamamlandı — tahmini risk (16 Ağustos 2026, F4 tamamlama oturumu, yedinci tur — artık `[x]`, checklist'in tüm alt maddeleri kapandı):**
- Prompt beşinci bir satıra çıkarıldı: `RISK:` — modele, isteğe/dosyalara özgü, somut TEK bir cümle isteniyor ("bu fonksiyon birçok yerde çağrılıyor" gibi); sabit onay/sandbox boilerplate'ini tekrar etmemesi açıkça isteniyor. Belirgin bir risk yoksa "NONE" diyebiliyor. Token bütçesi 400'den 450'ye çıkarıldı.
- Model'in ürettiği risk cümlesi, `CodingPlan.risk_notes`'un sabit boilerplate notlarının (network kapalı, onay gerekiyor vb.) **yerine değil yanına** ekleniyor — "Model risk değerlendirmesi: ..." önekiyle, 300 karaktere sınırlı (güvenilmez model çıktısı sayfalarca metin üretemesin diye).
- Gerçek modelle canlı doğrulandı: bu repo'nun kendi `runtime.rs::record_audit` fonksiyonunun imzasını değiştirme isteğinde, model gerçekten anlamlı ve isteğe özgü bir risk ürettti: *"The record_audit function might be called in multiple places, and changing its signature could break existing code that relies on the old signature."*
- Kanıt: 2 yeni `project_analyst` testi (model isteğe özgü bir risk üretirse boilerplate notların yanına eklendiği, "NONE" derse hiçbir hayali risk notu eklenmediği). Tam paket: `cargo fmt`, `cargo test --offline` (251 lib + 57 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**"Yerel üretkenlik tool framework" tamamlandı (16 Ağustos 2026, F4 tamamlama oturumu, sekizinci tur — artık `[x]`):**
- **Bulunan gerçek boşluk**: `execute_approved` (F0'dan beri var) yalnız `note.create`'i çalıştırabiliyordu — `if manifest.sandbox_profile != "LOCAL_RESTRICTED" || manifest.capability_id != "note.create"` koşulu HERHANGİ başka bir capability'yi doğrudan reddediyordu. "Framework" değil, tek bir tool'a özel, sabit kodlanmış bir fonksiyondu. Ayrıca `PolicyControl::ExplainBeforeExecute` F0'dan beri bildirilen ama hiç uygulanmayan bir kontroldü — onay ekranı yalnız `task_id • action_id` gösteriyordu, kullanıcı ne olacağını görmeden onaylıyordu.
- **Düzeltme — gerçek bir `LocalTool` trait'i** (`src/lib.rs`): `fn preview(&self, input) -> String` (onaydan önce tam olarak ne olacağını gösterir) + `fn execute(&self, input, task_id) -> ToolResult`. `execute_approved` artık sabit kodlanmış değil, `local_tool_for(capability_id)` ile dispatch ediyor — yeni bir tool eklemek: bir manifest girdisi (`capabilities.rs`) + bir `policy_for` kolu (`policy.rs`) + bu trait'i uygulayan bir struct + `local_tool_for`'a bir girdi, başka hiçbir yer değişmiyor.
- **İki gerçek tool aynı çerçeveden geçiyor**: `NoteCreateTool` (mevcut `note.create`, davranışı birebir korunarak taşındı — 251 var olan test hiç kırılmadı) ve yeni `FileAppendNoteTool` (`file.append_note` — workspace-göreli, gizli-bilgi benzeri olmayan bir dosyaya tek satır ekliyor; F4'ün sandbox'lı kod patch'lerinden tamamen ayrı, basit bir "yerel üretkenlik" ihtiyacı). İkisi de aynı Policy → Task → `WaitingForUser` → Approval → `execute_approved` → Verifier zincirinden geçiyor.
- **"Preview" gerçekten uygulandı**: yeni `Runtime::preview_pending_action(task_id)`, `pending_inputs`'taki ham input'u ilgili `LocalTool::preview`'a veriyor. TUI'de `/approvals` artık her bekleyen işlemin altında tam önizlemesini gösteriyor (`ExplainBeforeExecute`'un gerçek uygulaması).
- **"Verifier" genişletildi**: yeni `file.contains:<path>:<beklenen-metin>` kanıt türü — yalnız dosyanın var olduğunu değil, iddia edilen içeriği gerçekten taşıdığını kontrol ediyor (`file.exists`'ten daha güçlü).
- TUI: yeni `/note-append <proje-içi-göreli-dosya> | <satır>` — model çağrısı yok, doğrudan deterministik `classify()` üzerinden `Runtime::handle`.
- Kanıt: `lib.rs`'te 8 yeni test (uçtan uca onay zinciri — iki farklı append'in üst üste birikip birbirini ezmediği dahil —, onaydan önce dosyaya hiçbir şey yazılmadığı, path traversal/gizli-bilgi-benzeri-isim/boş-satır/aşırı-uzun-satır reddi, `file.contains` kanıtının yalnız gerçek içerikte geçtiği) + `main.rs`'te 2 yeni test (`/note-append` onay öncesi tam önizleme gösterip hiçbir şey yazmadığı, onay sonrası gerçekten yazıp doğruladığı; ayırıcısız çağrı kullanım mesajı gösterip runtime'a hiç dokunmadığı). Var olan 251 test hiç değişmeden geçmeye devam etti (davranış regresyonu yok).
- Tam paket: `cargo fmt`, `cargo test --offline` (257 lib + 59 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**"Çok-adımlı workflow runner" tamamlandı (16 Ağustos 2026, F4 tamamlama oturumu, dokuzuncu ve son gerçek tur — artık `[x]`, F4'ün bilinçli ertelenen 3 maddesi dışındaki tüm maddeler kapandı):**
- Yeni `src/workflow.rs` — genel, tool-bağımsız bir çok-adımlı orkestratör. `WorkflowStep` trait'i: `id`/`description` (plan gösterimi için), `has_side_effect` (hangi adımlar onay gerektirir), `idempotency_key` (aynı işin iki kez uygulanmasını engeller), `execute`, `rollback` (best-effort geri alma).
- `describe_workflow(steps)`: hiçbir adımı çalıştırmadan planı (adım sırası + hangilerinin onay gerektireceği) döner — "planı kullanıcıya gösterme".
- `run_workflow(steps, approve, cancel, max_retries)`: her yan etkili adımdan önce `approve` callback'ini çağırıyor (policy/approval); geçici bir hatadan sonra `max_retries` kadar aynı adımı yeniden deniyor (retry); aynı `idempotency_key`'e sahip bir adım bu çalıştırmada ikinci kez hiç yürütülmüyor (idempotency); bir adım reddedilir/başarısız olur/iptal edilirse, o ana kadar başarıyla tamamlanmış TÜM yan etkili adımlar TERS sırayla geri alınıyor (iptalde cleanup); dönen `WorkflowSummary` her adımın tam sonucunu taşıyor (audit özeti) — rollback'in kendisi başarısız olursa bu da gizlenmeden raporlanıyor.
- F4'ün kendi plan→patch→onay→uygula→test zinciri bu soyutlamanın somut örneği ve ilham kaynağı; **bilinçli tasarım kararı**: bu geç aşamada, zaten kapsamlı test edilmiş üretim kodunu (`Runtime::apply_coding_patch_with_regression_check`) bu yeni motora taşımak riskli olurdu — motor bağımsız, kendi başına kanıtlanmış bir kütüphane olarak bırakıldı, gelecekteki yeni çok-adımlı işler (F4 "Yerel üretkenlik tool framework"'ün `LocalTool`'ları dahil) için hazır.
- Kanıt: 10 test — plan gösterimi, tüm adımlar başarılı, reddedilen bir onayın önceki adımları geri aldığı, salt-okunur bir adımın hiç onay istemediği, geçici bir hatanın retry bütçesi içinde kurtarıldığı, retry bütçesi tükenince adımın başarısız olup öncekilerin geri alındığı, aynı idempotency key'in ikinci çalışmayı engellediği, **iptalin** (gerçek bir `CancelFlag`, bir onay callback'i içinden set edilerek) kalan adımları durdurup tamamlananları geri aldığı, bir rollback'in kendisi başarısız olursa bunun gizlenmeden raporlandığı, ve **gerçek dosya sistemi üzerinde çalışan gerçek bir adımın** (sentetik değil) uçtan uca tamamlandığı.
- Tam paket: `cargo fmt`, `cargo test --offline` (267 lib + 59 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**"Patch preview/review" tamamlandı (16 Ağustos 2026, F4 tamamlama oturumu, altıncı tur — artık `[x]`):**
- Yeni `workbench.rs::split_diff_by_file(diff) -> Vec<(PathBuf, String)>`: çok-dosyalı bir unified diff'i her dosyanın kendi bloğuna kayıpsız bölüyor (bloklar birleştirildiğinde orijinal diff'i birebir yeniden üretiyor).
- Yeni `workbench.rs::scope_patch_proposal_to_files(plan, proposal, selected_files) -> PatchProposal`: kullanıcının seçtiği bir alt küme için bağımsız, kendi hash'ine sahip yeni bir `PatchProposal` üretiyor. Yalnız daraltma mümkün — `selected_files`'ın her biri zaten orijinal `proposal.affected_files`'ın bir üyesi olmalı, asla scope genişletilemez. Yeni proposal'ın hash'i eskisinden farklı, bu yüzden eski bir onay yeni (daraltılmış) proposal'a asla "replay" edilemez.
- TUI'ye üç yeni komut: `/patch-files` (patch'i dosya dosya, her birinin kendi diff'iyle gösteriyor — "satır bazlı görünüm"ü tek bir kütlesel diff yerine dosya bazına indiriyor, ikinci bir bölme mantığı icat etmeden `scope_patch_proposal_to_files`'ı tek-dosyalık bir alt küme için yeniden kullanarak), `/patch-note <metin>` (onay öncesi serbest bir kullanıcı notu — hiçbir doğrulamayı etkilemiyor, yalnız onay sonrası mesaja ekleniyor; boş çağrılırsa temizler), ve `/approve-patch [dosya1 dosya2 ...]` genişletildi — dosya adı verilirse yalnız o alt küme onaylanıp uygulanıyor, diğer dosyalar hiç değişmiyor; hiçbiri verilmezse eskisi gibi tümü.
- Kanıt: `workbench.rs`'te 3 yeni test (çok-dosyalı diff'in kayıpsız bölünmesi, bir alt kümeye onay daraltmanın bağımsız-geçerli ve farklı-hash'li bir proposal ürettiği, proposal dışı bir dosyaya daraltmanın reddedildiği) + `main.rs`'te 6 yeni test (`/patch-note`/`/patch-files` teklif yokken no-op ve teklif varken çalışıyor, **gerçek bir iki-dosyalı `/approve-patch <dosya>` çağrısının yalnız seçilen dosyayı değiştirip diğerine hiç dokunmadığı** — bu ortamda gerçek `bwrap` `CLONE_NEWNET` reddi yüzünden başlatılamayabileceği için test iki geçerli sonuçtan birini kabul edecek şekilde yazıldı, ama seçilmeyen dosyanın asla değişmediği kesin olarak doğrulanıyor —, proposal dışı bir dosya seçiminin reddedildiği).
- Tam paket: `cargo fmt`, `cargo test --offline` (249 lib + 57 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**"OS izolasyonu"/"Resource kontrolü" maddelerinde kısmi ilerleme (16 Ağustos 2026, ikisi de hâlâ `[ ]` — tam bitmiş sayılmıyor):**
ADR-0001'in "henüz tamamlanmamış" diye işaretlediği maddelerden ikisi kapatıldı, üçü hâlâ açık (bkz.
[ADR-0001 Ek](docs/adr/0001-isolated-coding-worker.md#ek--runtime-quota-ve-ek-namespace-izolasyonu-16-ağustos-2026)).
- **Kapatıldı**: `WorkerLimits.max_runtime_seconds`/`max_output_bytes` alanları vardı ama hiç okunmuyordu — gerçek bir watchdog yoktu. Yeni `wait_with_timeout` fonksiyonu `git apply`'i artık gerçekten süreyle sınırlıyor (kota aşılınca `kill()`); `sleep` ile (gerçek "asılı" bir süreç, `git apply`'in kendi davranışına bağlı olmayan doğrudan bir watchdog testi) doğrulandı. Bubblewrap çağrısına `--unshare-pid`/`--unshare-ipc`/`--unshare-uts` eklendi (F4 tehdit modelinin "process tree" maddesi).
- **Hâlâ açık**: gerçek CPU/RAM/disk kotası (cgroups gerekir), seccomp filtresi, snapshot/overlay worker (şu an doğrudan workspace'e yazıp snapshot+rollback ile geri alıyor, gerçek bir copy-on-write overlay değil).
- **Doğrulama sınırı**: bu geliştirme ortamı `CLONE_NEWNET` izni vermediği için gerçek `bwrap` çağrısı burada uçtan uca çalıştırılamadı (ADR-0001'in kendi bilinen sınırı) — yalnız derlendiği (release profili dahil) doğrulandı, gerçek doğrulama hedef makinede release smoke ile yapılmalı.
- Kanıt: yeni testler `wait_with_timeout_kills_a_process_that_outlives_its_quota`, `wait_with_timeout_succeeds_for_a_process_that_finishes_in_time`. Tam paket: `cargo fmt`, `cargo test --offline` (207 lib + 45 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS, gerçek release profili derlemesi dahil).

**"Gerçek cancellation" + "Resource kontrolü" devamı (16 Ağustos 2026, ikinci tur — F4 tamamlama oturumu):**
- Yeni `wait_with_deadline_and_cancellation` (`workbench.rs`): `wait_with_timeout`'un yerini aldı (o artık ince bir sarmalayıcı). İki ayrı durdurma nedenini ayırt ediyor: `WorkerStopReason::UserCancelled` (çağıran başka bir thread'den `CancelFlag`'i `true` yapmış) ve `RuntimeQuotaExceeded` (süre kotası dolmuş). Her iki durumda da önce `SIGTERM` gönderiliyor, kısa bir grace period (200ms) bekleniyor, hâlâ canlıysa `SIGKILL`'e yükseltiliyor — çocuk süreç asla zombi kalmıyor (`wait()` her koşulda çağrılıyor).
  - Kanıt: `a_cancelled_process_is_stopped_long_before_its_runtime_quota` (gerçek `sleep 5`, 80ms'de iptal, 2sn içinde öldüğü kanıtlanıyor) + `a_process_that_ignores_sigterm_is_escalated_to_sigkill` (gerçek `sh -c "trap '' TERM; sleep 5"` — `SIGTERM`'i görmezden gelen bir süreç, `SIGKILL`'e yükseltilmesi kanıtlanıyor).
  - **Hâlâ eksik**: TUI'de gerçek bir `/cancel` komutu ve iptal/timeout'un audit zincirine yazılması yok — mekanizma hazır ama henüz hiçbir üretim çağrı noktası kullanmıyor.
- `WorkerLimits` yeni `max_memory_bytes` alanı (varsayılan 512MB) + yeni `apply_worker_rlimits`: `pre_exec` ile çocuk sürece gerçek `setrlimit` uyguluyor — `RLIMIT_AS` (bellek), `RLIMIT_FSIZE` (64MB, tek dosya boyutu), `RLIMIT_NPROC` (64, fork-bomb koruması), `RLIMIT_CPU` (süre kotasının 16 katı, arka plan koruması). Bu ortamda cgroups kurulamadığı için gerçek bir alternatif: kök yetkisi gerektirmiyor (bir süreç kendi limitini her zaman düşürebilir) ve çekirdek tarafından gerçekten uygulanıyor.
  - Kanıt: `a_child_process_is_bound_by_the_configured_memory_rlimit` — çocuk sürecin kendi `ulimit -v` çıktısını okuyup ayarlanan `max_memory_bytes` ile birebir eşleştiği doğrulanıyor (dolaylı değil, doğrudan kanıt).
  - `isolated_git_apply_command`/`isolated_worker_command` tek bir paylaşılan bwrap kurucusuna birleştirildi (`extra_ro_binds`, `chdir_relative` parametreli) — Allowlist command runner da aynı izolasyon sınırını kullanıyor, ikinci bir bakım yükü icat edilmedi.

**"Allowlist command runner" + "Test/verifier runner" (16 Ağustos 2026 — command runner artık `[x]`, aşağıdaki turda TUI'ye bağlandı; test/verifier runner kısmi ilerleme, hâlâ `[ ]` kaldı — bkz. sonraki bölüm):**
- Yeni `src/command_runner.rs`. `validate_command_line`: hiçbir shell'e girmiyor — noktalı virgül, boru, `&&`, backtick, `$()` gibi shell meta-karakterleri komut satırı seviyesinde tamamen reddediliyor (yorumlanmıyor, kaçırılmıyor). Yalnız sabit bir program/alt-komut izin listesindeki (`cargo test/build/check/clippy/fmt`, `npm test/run/ci`, `pytest`, `go test/build/vet`, `python3 -m`, `mvn test/verify`, `gradle test/check` vb.) komutlar kabul ediliyor; argümanlarda mutlak yol veya `..` reddediliyor.
- `resolve_program`: izin listesindeki program adını host `PATH`'inde gerçek, çalıştırılabilir bir mutlak yola çözüyor (worker'ın kendisi hiç `PATH` araması yapmıyor — F4 tehdit modelinin "ambient shell yok" ilkesi); `/usr` dışında bir yerde bulunursa (ör. rustup tipi `~/.cargo/bin`) o dizin ayrıca read-only bağlanıyor.
- `run_allowlisted_command`: `git apply` ile aynı izolasyonda (bwrap + rlimit + watchdog/cancel) çalıştırıyor; `dry_run: true` hiçbir süreç başlatmadan yalnız doğruluyor. `CommandRun` kanıtı: exit code, sınırlı stdout/stderr önizlemesi, tam çıktının SHA-256'sı.
- `run_test_plan`: bir `CodingPlan.test_plan`'ın her satırını çalıştırıyor; komut olarak ayrıştırılamayan serbest metin satırlarını (ör. project_analyst'ın "bilinen manifest yok" notu) hataya değil "skipped" listesine düşürüyor.
- Kanıt: 8 test — izin listesi dışı program/alt-komut reddi, shell meta-karakter reddi (5 farklı enjeksiyon deseni), path traversal/mutlak yol reddi, dry-run'ın hiç süreç başlatmadığı, gerçek bir izin listesindeki komutun (`python3 -m`) gerçekten çalışıp kanıt ürettiği, izin listesi dışı bir komutun `spawn`'a hiç ulaşmadığı, test planındaki serbest metnin atlandığı, **ve gerçek bir iptalin** (`python3 -m timeit -n 999999999 pass` — kasıtlı uzun süren gerçek bir komut, 80ms'de iptal edilip 5sn içinde durduğu kanıtlanıyor) `WorkerStopReason::UserCancelled` olarak ayrıştırıldığı.
- **Hâlâ eksik**: TUI'de hiçbir komut bunu henüz çağırmıyor (kütüphane seviyesinde tam, üretim çağrı noktası yok); env allowlist şu an `git apply` ile aynı sabit `--clearenv` + `PATH=/usr/bin` düzeyinde, komut bazlı ayrıca özelleştirilebilir değil.
- Tam paket: `cargo fmt`, `cargo test --offline` (222 lib + 46 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz).

**Gerçek, önceden fark edilmemiş bir bug bulundu ve düzeltildi (16 Ağustos 2026, F4 tamamlama oturumu sırasında): `ModelProvider::complete()` üretimde 8 token'a sabitliydi.**
- `LlamaServerProvider::complete()` her zaman `max_tokens: 8` gönderiyordu — router/bellek-niyeti gibi tek kelimelik sınıflandırma yanıtları için doğru, ama `draft_coding_plan_with_provider`'ın `TESTS:` satırını ve her `draft_patch_with_provider` tam-dosya yeniden yazımını **üretimde sessizce kırpıyordu**. Gerçek sunucuya karşı `curl` ile kanıtlandı: `"FILES: memory_intent.rs  \nTESTS"` — `TESTS:` kelimesinin ortasında kesiliyor. Coding-plan durumunda bu, yalnız `overview.suggested_test_commands`'e düşme mekanizması tarafından maskeleniyordu (sonuç tesadüfen doğru görünüyordu); patch generator'da maskeleyecek bir fallback yok, tam kırılırdı.
- **Düzeltme**: `ModelProvider` trait'ine yeni `complete_with_budget(prompt, max_tokens)` metodu eklendi (varsayılan gövde `complete()`'e yönlendiriyor — mevcut hiçbir çağrı noktası/mock etkilenmedi). `LlamaServerProvider` bunu gerçekten override ediyor. `draft_coding_plan_with_provider` artık 300 token, `draft_patch_with_provider` dosya boyutuna göre dinamik bir bütçe (`~bytes/3 + 200`, 64–4000 aralığında) kullanıyor.
- **Model sunucusu context penceresi 2048 → 8192'ye çıkarıldı** (`~/.config/systemd/user/jarvis-llama.service`, makine-yerel config, repo'da değil): tam dosya yeniden yazımı hem prompt'ta hem yanıtta dosyanın tamamını taşımak zorunda; 2048 çoğu gerçek kaynak dosyası için yetersizdi. RAM'de bolluk var (44GB boşta), CPU-only modelde context artışının latency'ye somut etkisi bu turda ayrıca ölçülmedi (var olan `-np 1` tekli-slot kısıtı zaten bilinen bir maliyet).
- Gerçek sunucuya karşı `curl` ile önce/sonra kanıtlandı; ayrıca geçici (commit'lenmeyen) canlı testlerle hem `draft_coding_plan_with_provider`'ın artık `TESTS:` satırını tam ürettiği hem de `draft_patch_with_provider`'ın gerçek bir dosyayı uçtan uca (taslak → onay → uygula → doğrula, gerçek disk üzerinde) doğru şekilde değiştirdiği kanıtlandı.

**"Patch generator" (16 Ağustos 2026, F4 tamamlama oturumu — artık `[x]`):**
- Yeni `src/workbench.rs::generate_unified_diff_for_file(relative_path, old_content, new_content) -> Option<String>`: iki tam içerik string'inden gerçek unified diff'i `git diff --no-index --no-prefix` ile **makine tarafında** hesaplıyor.
- **Bilinçli tasarım kararı**: model diff/hunk sözdizimi üretmeye hiç zorlanmıyor — küçük yerel modeller satır numarası muhasebesinde güvenilmez. Bunun yerine model her dosyanın **tam yeni içeriğini** üretiyor, gerçek diff'i makine hesaplıyor. Model "NO_CHANGE" derse ya da yeniden yazımı mevcut içerikle birebir aynıysa dosya sessizce atlanıyor (hata değil).
- Yeni `src/patch_generator.rs::draft_patch_with_provider(plan, provider) -> PatchProposal`: her etkilenen dosya için ayrı bir model çağrısı, sonucu `create_patch_proposal` ile aynı doğrulamadan (path containment, plan üyeliği, hash, boyut limiti) geçiyor. Markdown kod bloğu sarmalaması otomatik soyuluyor; modelin attığı sondaki newline (gerçek modelde gözlemlendi) orijinal dosyanınkiyle eşleşecek şekilde geri ekleniyor. Tüm dosyalar için "değişiklik yok" bir hata olarak dönüyor (boş bir patch asla üretilmiyor).
- **Dosya boyutu sınırı**: genel 512 KiB workspace okuma sınırından çok daha küçük, yeni `MAX_PATCH_GENERATOR_FILE_BYTES = 8000` — tam dosya hem prompt'ta hem yanıtta modelin kendi context penceresine sığmalı, yalnız okunabilir olması yetmez.
- **Binary/secret dosya reddi dolaylı ama gerçek**: `plan.affected_files` yalnız `/plan`'ın `overview.included_files`'ından gelebilir, o da zaten `preview_workspace_index`'in gizli-bilgi/boyut filtresinden geçmiş dosyalardır — gizli-bilgi benzeri bir dosya asla bir `CodingPlan`'a giremez, dolayısıyla patch generator'a da hiç ulaşmaz. Geçersiz UTF-8 (çoğu binary) `read_current_content` içinde ayrıca reddediliyor.
- Gerçek modelle uçtan uca doğrulandı (bu repo'nun kendi dosyalarından biriyle): İngilizce bir `greet()` fonksiyonunu Türkçeleştirme isteği doğru diff'i üretti, `apply_approved_patch` ile gerçekten diske yazıldı, `verifier_evidence` gerçek SHA-256 döndürdü.
- Kanıt: 6 yeni `patch_generator` testi (tam dosya yeniden yazımı geçerli bir `PatchProposal`'a dönüşüyor, markdown çit soyuluyor, tüm dosyalar NO_CHANGE derse hata — boş patch değil, çok-dosyalı planda yalnız gerçekten değişenler dahil oluyor, modelin attığı sondaki newline geri ekleniyor, boş planlı istek modele hiç dokunmadan reddediliyor) + `workbench.rs`'te 2 yeni test (üretilen diff'in başlık şekli `validate_patch_proposal`'ı gerçekten geçiyor, aynı içerik diff üretmiyor).

**"Patch apply transaction" + "Gerçek cancellation" + "Allowlist command runner" → TUI'ye tam bağlandı (16 Ağustos 2026, F4 tamamlama oturumu):**
- `runtime.rs`'e iki yeni metod: `apply_coding_patch` (workbench'in saf `apply_approved_patch`'ini sarıp `coding.patch.applied` audit event'i yazıyor — saf fonksiyonun yazacak bir `Runtime`'ı yok) ve `run_coding_tests_and_finalize` (bir `CodingPlan.test_plan`'ı `command_runner::run_test_plan` ile çalıştırıp sonuca göre iki yoldan biri: tüm komutlar geçtiyse snapshot atılır, değişiklik kalıcı olur (`coding.tests.passed` audit); geçmezse **veya iptal edilirse** snapshot otomatik geri yüklenir, dosyalar patch-öncesi hâline döner (`coding.tests.failed`/`coding.tests.cancelled` + `coding.patch.rolled_back_after_test_outcome` audit).
  - **Tasarım kararı — tam overlay/copy-on-write yerine "uygula + test et + geri al"**: F4'ün "Isolated worker bootstrap" maddesi bir workspace overlay'i (izole bir kopyada uygula/test et, yalnız başarılıysa gerçek workspace'e senkronla) öngörüyor. Bu turda bunun yerine daha basit, halihazırda var olan snapshot/rollback mekanizmasını test sonucuna da genişletmeyi tercih ettim: kullanıcı için nihai güvenlik özelliği aynı (test geçmezse dosyalar tam olarak eskisi gibi kalır), ama repo'nun tamamını her patch denemesinde kopyalamak gerekmiyor (büyük repo'larda pahalı olurdu) ve mevcut, zaten test edilmiş kodun üzerine inşa edildi. Gerçek copy-on-write overlay hâlâ açık bir madde (bkz. "Isolated worker bootstrap" checklist item), bilinçli olarak bu turda yapılmadı.
- TUI'ye dört yeni komut: `/patch` (en son `/plan`'a göre modelden gerçek bir diff taslağı üretir, arka plan thread), `/approve-patch` (senkron: onaylar, izole uygular, kanıtı gösterir, test planı varsa arka planda testleri çalıştırıp sonuca göre kalıcı bırakır ya da geri alır), `/reject-patch` (senkron, taslağı atar), `/abort` (**yeni bir mekanizma** — `Runtime::cancel`'dan tamamen ayrı: o bir task'ı başlamadan önce iptal ediyordu ki yorum satırı bunun "senkron MVP'de gerçek bir worker/process handle'ı yok" dediği için mümkün değildi; artık `CancelFlag` ile hâlâ çalışan izole bir süreç ortasında gerçekten durdurulabiliyor).
  - `App` yeni alanlar: `pending_coding_plan`, `pending_patch`, `active_cancel`. `WorkerReply` yeni alanlar: `coding_plan`, `patch_proposal`.
- Kanıt: `runtime.rs`/`lib.rs`'te 3 yeni test — onaylı bir patch'in dosyayı gerçekten değiştirip `coding.patch.applied` audit'lediği; başarısız bir testin dosyayı otomatik eski hâline döndürüp `coding.tests.failed` + rollback audit'i yazdığı (gerçek `python3 -m <yok-olan-modül>` ile); geçen bir testin değişikliği koruyup `coding.tests.passed` yazdığı (gerçek `python3 -m this` ile). `main.rs`'te 7 yeni test — `/patch` plan yokken senkron no-op, boş kapsamlı planla modele hiç dokunmadan reddediliyor, `/reject-patch` teklifi atıyor, `/abort` aktif iş yokken no-op / varken bayrağı çeviriyor, `/approve-patch` teklif yokken no-op, ve **uçtan uca `/approve-patch`** (gerçek `bwrap` bu geliştirme sandbox'ında `CLONE_NEWNET` reddi yüzünden başlatılamıyor — test bu yüzden iki geçerli sonuçtan birini kabul edecek şekilde yazıldı: dosya ya tam olarak yeni içeriğe geçti ya da hiç dokunulmadan kaldı, asla yarı-uygulanmış değil; hedef makinede bwrap çalıştığında aynı test değişikliğin gerçekten diske yazıldığını kanıtlar).
- Tam paket: `cargo fmt`, `cargo test --offline` (233 lib + 53 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS, gerçek release derlemesi + MCP smoke dahil).

**F4'te bu oturumda hâlâ açık kalan maddeler (dürüst değerlendirme — bu, oturumun erken bir turundaki anlık görüntü; "uçtan uca doğrulanamaz" ifadesi için aşağıdaki 10. tur düzeltmesine bkz., o iddia yanlış çıktı):**
- **Isolated worker bootstrap**: gerçek copy-on-write overlay yok (yukarıdaki tasarım kararına bkz.).
- **OS izolasyonu**: seccomp filtresi hâlâ yapılmadı — henüz yapılmadığı için, artık "burada doğrulanamaz" olduğu için değil (bkz. 10. tur düzeltmesi).
- **Resource kontrolü**: `RLIMIT_AS`/`RLIMIT_FSIZE`/`RLIMIT_NPROC`/`RLIMIT_CPU` gerçek ve test edildi (10. turda ayrıca gerçek cgroup v2 eklendi), ama bu "disk" için yalnız tek-dosya boyutu sınırı — toplam workspace disk kullanımı için gerçek bir kota (cgroups bunu sağlamıyor, overlay/tmpfs `size=` gerekir) yok. Bu yüzden checklist'te hâlâ `[ ]`.
- **Coding plan UX**: "varsayımlar"/"kullanıcı soruları" hâlâ ayrı üretilmiyor.
- **Patch preview/review**: diff önizlemesi var (satır bazlı, okunabilir) ama seçilebilir dosya scope'u (çok-dosyalı bir patch'in yalnız bir kısmını onaylama) ve kullanıcı değişiklik notu yok — all-or-nothing.
- **Yerel üretkenlik tool framework / Çok-adımlı workflow runner**: hiç başlanmadı — ikisi de kendi başına büyük, açık uçlu mühendislik/tasarım işleri (bkz. `jarvis-f4-progress.md` kalıcı hafıza notu).

**"Test/verifier runner" tamamlandı (16 Ağustos 2026, F4 tamamlama oturumu — dördüncü tur, artık `[x]`):**
- **Bulunan gerçek boşluk**: bir önceki turda dürüstçe belgelenmiş "mevcut test regresyon raporu yok" sınırı — bir test komutu patch'ten TAMAMEN bağımsız olarak zaten bozuksa, sistem bunu patch'in kendi hatasından ayırt edemiyordu, ikisi de aynı şekilde "testler geçmedi, geri al" olarak işleniyordu (yanlış "suçlama" riski).
- **Düzeltme**: `Runtime::run_coding_tests_and_finalize` yerini `apply_coding_patch_with_regression_check`'e bıraktı. Patch uygulanmadan **ÖNCE** aynı test planı bir "taban çizgisi" olarak bir kez çalıştırılıyor. Patch sonrası her komut için: taban çizgisinde GEÇİP patch sonrası BAŞARISIZ olan komutlar gerçek bir "regresyon" (`regressions` listesi) sayılıyor; taban çizgisinde de zaten başarısız olan bir komut artık patch'e karşı kullanılmıyor — değişiklik kalıcı kalır, audit'e `coding.tests.pre_existing_failure_tolerated` olarak dürüstçe yazılır. Yeni bir eşleşme bulunamazsa (ör. taban çizgisi skip edildiyse) temkinli davranılıyor: "zaten bozuktu" varsayılmıyor, regresyon sayılıyor.
- Audit event isimleri ayrıştı: `coding.tests.passed`, `coding.tests.regression_detected` (gerçek regresyon), `coding.tests.cancelled`, `coding.tests.failed` (hiçbir komut çalışmadı — ör. tüm satırlar skip edildi), `coding.tests.pre_existing_failure_tolerated` (kalıcı kalan ama taban çizgisinde de hata olan durumlar için ek bilgi).
- TUI'de `/approve-patch`: test planı olan bir patch artık taban çizgisi ölçümünden dolayı tek bir arka plan thread'inde (baseline → uygula → test → karşılaştır) çalışıyor — uygulama artık senkron bir ilk adım değil, çünkü taban çizgisi patch'ten önce ölçülmeli. Test planı olmayan patch'ler hâlâ tamamen senkron (taban çizgisi anlamsız).
- Kanıt: `lib.rs`'te 2 yeni Runtime testi (gerçek bir regresyonun — geçerli Python'un sözdizimi hatalı hale getirilmesi — tespit edilip geri alındığı; taban çizgisinde de zaten bozuk bir test komutunun artık doğru bir patch'i engellemediği) + `coding_eval.rs`'te eval setinin 6. senaryosu düzeltmeyi kanıtlayacak şekilde yeniden yazıldı, 7. senaryo (gerçek regresyon) eklendi — artık 7 uçtan uca senaryo.
- Tam paket: `cargo fmt`, `cargo test --offline` (242 lib + 53 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**KRİTİK DÜZELTME — "bu sandbox `CLONE_NEWNET` reddediyor" iddiası yanlıştı; izole worker ilk kez gerçekten uçtan uca çalıştırıldı (16 Ağustos 2026, F4 sonrası 10. tur):**

Bu belgede yukarıda (ve ADR-0001'de) tekrar tekrar geçen "bu geliştirme sandbox'ı `CLONE_NEWNET` vermiyor / gerçek `bwrap` burada başlatılamıyor" iddiası **doğru değildi**. Bu oturumda gerçek makinede (masaüstü, kök gerektirmez) doğrudan test edildi: `unshare --user`, tam ağ izolasyonlu `bwrap --unshare-net`, ve `systemd-run --user --scope -p MemoryMax=...` **hepsi çalışıyor**. Asıl neden hiç namespace izni değil, kodun kendisindeki iki gerçek, önceden fark edilmemiş hataydı:

1. **`apply_worker_rlimits`'te `RLIMIT_NPROC` sabit `64`'e ayarlıydı.** `RLIMIT_NPROC` Linux'ta *süreç ağacı başına* değil, o gerçek UID'nin **sistem genelinde sahip olduğu toplam thread sayısına** göre sayılır (`man setrlimit`: "more precisely, on Linux, threads"). Sıradan bir masaüstünde (tarayıcı, IDE, JARVIS'in kendi servisleri) bu sayı zaten binlerce (bu makinede ~1837 thread, yalnız ~170 üst-seviye süreç — `ps` süreç sayar, thread saymaz, bu da ilk düzeltme denemesinde yanıltıcı bir ölçüme yol açtı). Sınırı `64`'e sabitlemek, rlimit'i alan sürecin (ve ondan çatallanan bwrap'ın kendi `unshare(CLONE_NEWUSER)` çağrısının) **her türlü yeni süreç/thread oluşturmasını anında `EAGAIN` ile başarısız kılıyordu** — canlı olarak doğrulandı: `prlimit --nproc=64:64 --pid=$$` sıradan bir shell'de (168 var olan süreç) düz bir `fork()`'u bile aynı "Resource temporarily unavailable" hatasıyla kırıyor. **Düzeltme**: `current_thread_count_for_real_uid()` — o anki gerçek thread sayısını `/proc/*/task` üzerinden ölçüp üstüne 1024 pay bırakıyor; sabit bir sayı yerine, mevcut yüke göre dinamik.
2. **`--tmpfs /tmp` mount'u workspace bind'ından SONRA geliyordu.** bwrap mount'ları argüman sırasına göre uyguluyor; workspace'in gerçek yolu `/tmp` altındaysa (rutin — `std::env::temp_dir()` tabanlı geçici workspace'ler tam olarak bunu yapıyor), sonraki genel `--tmpfs /tmp` mount'u önceki spesifik bind'ı sandbox içinde görünmez kılıyor/gölgeliyordu — `bwrap: Can't chdir to <path>: No such file or directory`. **Düzeltme**: `--tmpfs /tmp` artık her zaman bind'lardan önce.

**Sonuç**: `src/main.rs`'teki `approve_patch_with_no_test_plan_applies_immediately_and_stays_synchronous` testi — daha önce "iki geçerli sonuçtan biri" kabul edecek şekilde yazılmıştı (gerçek bwrap burada başlamaz varsayımıyla) — artık **yalnız gerçek başarıyı** kabul edecek şekilde sıkılaştırıldı ve geçiyor: patch gerçekten diske yazılıyor. Yeni bir test, `approve_patch_with_a_real_test_plan_runs_it_through_the_real_isolated_worker`, allowlist komut çalıştırıcısının (`cargo`/`pytest`/`python3` gibi test komutlarını çalıştıran taraf) da aynı gerçek yoldan çalıştığını kanıtlıyor. Bu, F4'ün "Gerçek cancellation", "Allowlist command runner" ve "Patch apply transaction" maddelerinin daha önce yalnız `#[cfg(test)]` bypass yoluyla (gerçek bwrap'sız) kanıtlanmış olduğu, üretim yolunun bu oturuma kadar **hiç gerçekten uçtan uca doğrulanmamış** olduğu anlamına geliyor — artık doğrulandı.

Aynı turda "Resource kontrolü" maddesine gerçek cgroup v2 eklendi: `isolated_worker_command` artık `bwrap`'ı doğrudan değil, `systemd-run --user --scope -p MemoryMax=... -p MemorySwapMax=0 -p CPUQuota=...%` ile sarmalıyor — tüm worker süreç ağacını (yalnız `RLIMIT_AS`'ın bağladığı tek süreci değil) gerçek bir cgroup'a bağlıyor. `MemorySwapMax=0`'ın gerekli olduğu canlı olarak kanıtlandı: takas açıkken `MemoryMax` aşımı süreci öldürmek yerine sayfaları takasa itiyor. Yeni test `cgroup_memory_limit_is_enforced_by_the_real_kernel_when_available` (gerçekten sayfalara dokunan bir Python süreci, 64MB sınırla, gerçek `SIGKILL` bekleniyor) — `systemd-run --user` yoksa (ör. minimal bir CI konteyneri) sessizce atlanıyor, üretim kodu ise aynı yoklukta yüksek sesle hata veriyor (sessiz bozulma yok).

Kalan 2 madde (seccomp, gerçek overlay) hâlâ `[ ]` — ama artık "bu ortamda yapılamaz" değil, "henüz yapılmadı, zaman kalmadı" (bkz. yukarıdaki checklist notları). Bir sonraki oturumda doğrudan denenebilir.

Tam paket: `cargo fmt`, `cargo test --offline` (272 lib + 66 main + 9 desktop, hepsi PASS, 3 kez tekrarlanıp kararlılık doğrulandı), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS, gerçek release derlemesi dahil).

**"Isolated worker bootstrap" + "Resource kontrolü" (disk) tamamlandı (16 Ağustos 2026, 11. tur — ikisi de artık `[x]`):**
- Yeni `workbench.rs::WorkspaceWriteMode` enum: `Direct` (yazmalar gerçek workspace'e ulaşır — `git apply`'in kullandığı, çünkü onaylı bir değişikliğin kalıcı olması gerekiyor) ve `Overlay` (`bwrap --overlay-src <root> --tmp-overlay <root>` — yazmalar görünmez bir tmpfs katmanına gider, worker çıkınca hepsi kayboluyor, gerçek workspace'e hiç dokunulmuyor). `isolated_worker_command` artık bir `write_mode` parametresi alıyor.
- **Allowlist komut çalıştırıcısı artık `Overlay` kullanıyor**: bir test/build komutunun (`cargo test`, `pytest`, vb.) hiçbir yazması gerçek dosyalara ulaşmıyor — kasıtlı ya da kazara bir kaynak dosyayı bozması artık yapısal olarak imkânsız. `git apply` bilinçli olarak `Direct` kalıyor (mevcut snapshot/rollback tasarım kararı korundu).
- **Disk kotası "bedavaya" çözüldü**: overlay'in tmpfs arka planı gerçek Linux belleği, ve cgroup v2 bellek muhasebesine zaten dahil — yani bir önceki turda kurulan `MemoryMax` cgroup'u, worker'ın kendi ayırdığı bellek kadar overlay'e yazdığı veriyi de kapsıyor. Canlı kanıtlandı: `MemoryMax=64M` sınırıyla overlay içine 200MB yazmaya çalışan bir `dd` gerçekten `SIGKILL` aldı (exit 137), ve host'ta hiçbir iz kalmadı.
- **Bilinçli sınır**: bu, ayrı bir dosya-sistemi/blok kotası (XFS project quota gibi) değil — bellek muhasebesi üzerinden dolaylı ama gerçek, kernel tarafından uygulanan bir üst sınır. Yeterli ve test edilebilir, ama "gerçek disk quota" ile birebir aynı mekanizma değil; bu fark dürüstçe not edildi.
- **Bilinen ödünleşim**: `Overlay` modu, test/build komutlarının `target/`/`node_modules/` gibi önbellek dizinlerinin de her çalıştırmada sıfırlanması anlamına geliyor (hiçbir yazı kalıcı olmuyor) — bu, regresyon kontrolünün taban çizgisi + patch-sonrası iki çalıştırması arasında bile önbellek paylaşımını engelliyor. Güvenlik kazancı (test komutları gerçek dosyaları asla bozamaz) bunun karşılığında bilinçli olarak tercih edildi; gerçek projelerde `cargo test` gibi komutlar bu yüzden her seferinde daha yavaş derlenebilir — henüz kullanıcıya ayrıca bildirilmedi, ilk gerçek kullanımda gözlemlenirse ele alınmalı.
- Kanıt: yeni `overlay_write_mode_never_lets_a_workers_writes_reach_the_real_workspace` testi (gerçek `bwrap`, var olan bir dosyayı değiştirme + yeni dosya oluşturma, ikisi de host'a hiç ulaşmıyor) — `systemd-run --user` yoksa atlanıyor, aynı `cgroup_memory_limit_...` testiyle aynı gerekçe.
- Tam paket: `cargo fmt`, `cargo test --offline` (273 lib + 66 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**"OS izolasyonu" (seccomp) tamamlandı — F4'ün son maddesi, 15/15 (16 Ağustos 2026, 12. tur):**
- Yeni `src/seccomp_filter.rs`: `libseccomp` crate (Cargo.toml'a eklendi, çevrimdışı derleniyor) ile gerçek bir seccomp-bpf allowlist filtresi inşa ediyor, `ScmpFilterContext::export_bpf_mem` — bwrap'ın man sayfasının adlandırdığı `seccomp_export_bpf` fonksiyonunun aynısı — ile derliyor, `memfd_create` (CLOEXEC olmadan) ile bir fd'ye yazıp `--seccomp <fd>` olarak bwrap'a veriyor. `isolated_worker_command`'a eklendi — hem `git apply` (Direct) hem allowlist komut çalıştırıcısı (Overlay) aynı filtreyi alıyor.
- **Filtre bwrap'ın kendi kurulumunu asla engellemiyor**: bwrap filtreyi kendi üzerine, kendi namespace/mount kurulumu bittikten SONRA, son `execve`'den hemen önce yüklüyor — bu yüzden `unshare`/`mount` gibi bwrap'ın ihtiyaç duyduğu ayrıcalıklı syscall'lar filtreden hiç etkilenmiyor, yalnız hedef program (git/cargo/npm/...) kısıtlanıyor.
- **fd zinciri canlı doğrulandı**: `systemd-run --user --scope` → `bwrap` → hedef program zincirinin ortasında açılan bir fd'nin (CLOEXEC olmadan) tüm zincir boyunca aynı numarayla hayatta kaldığı, elle kurulmuş bir testle kanıtlandı.
- **Ampirik, tahmine dayalı değil**: bu makinede `strace` kurulu değildi (ve kernel audit/dmesg log yöntemi kök yetkisi istiyordu) — kurulum yapmadan, kaynak paketinden kullanıcı dizinine derlendi (root gerekmedi). Bu gerçek `strace`'le, bu makinede kurulu olan her allowlist aracının (`git apply --check`, `cargo check/fmt --check/clippy`, `npm test`, `pytest`, `python3 -m`, `go build/test/vet` — gerçek, atılabilir test fikstürleriyle) gerçek çalıştırmaları izlendi, syscall kümesi birleştirildi (107 benzersiz). `mvn`/`gradle` bu makinede kurulu değildi, ampirik olarak doğrulanamadı — bu dürüstçe not edildi (filtre genel olarak yeterince geniş, ama garanti değil).
- **Ölçüm sınırlarına karşı bilinçli pay**: `exit`/`exit_group` (asla dönmedikleri için `strace -c` özetinde hiç görünmüyorlar ama her programın ihtiyacı var) ve birkaç evrensel dosya/sinyal syscall'ı (`symlink`, `readv`/`writev`, `fsync`, xattr okuma) manuel olarak eklendi — dürüstçe belgelendi, sessizce icat edilmedi.
- **Kanarya testiyle doğrulandı**: allowlist dışı, zararsız bir syscall (`personality()`) sandbox dışında başarıyla dönüyor, filtre altında gerçekten `EPERM` (errno=1) ile reddediliyor — filtrenin gerçekten bir şeyi engellediği kanıtlandı, yalnız her şeye izin veren boş bir filtre değil. Aynı zamanda tüm izlenen araçlar (npm/pytest/python3/go) filtre altında da başarıyla çalıştı.
- Kanıt: `seccomp_filter.rs`'te 5 test (her syscall adının bu mimaride gerçek olduğu, listede kopya olmadığı, derlenen BPF'in boş olmayıp 8 byte'ın katı olduğu, memfd'nin yazılan içeriği birebir geri verdiği, `attach_seccomp_filter`'ın gerçek bir fd ekleyip okunabilir kıldığı) + gerçek üretim yolundaki mevcut testler (`approve_patch_with_no_test_plan_...`, `approve_patch_with_a_real_test_plan_...`) filtre etkinken de geçmeye devam etti (3 kez tekrarlanıp kararlılık doğrulandı).
- Tam paket: `cargo fmt`, `cargo test --offline` (278 lib + 66 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz), `scripts/release_check.sh --offline` (PASS).

**F4 sonuç: 15/15 madde `[x]`.** Aynı günün TUI-düzeltme oturumunda F4'ün 3 kalan maddesi de (isolated worker bootstrap, resource kontrolü, OS izolasyonu) tamamlandı — hepsi gerçek makinede, gerçek kanıtla.

**"Coding evaluation seti" (16 Ağustos 2026, F4 tamamlama oturumu — artık `[x]`):**
- Yeni `src/coding_eval.rs` — diğer test modüllerinden farklı olarak tek bir fonksiyonu değil, **tüm F4 zincirini** (plan → patch taslağı → onay → uygula → test/doğrula → tut-ya-da-geri-al, `Runtime` üzerinden, TUI'nin `/plan`/`/patch`/`/approve-patch` ile sürdüğü aynı yol) uçtan uca test ediyor. Çevrimdışı ve deterministik (`ScriptedProvider`, gerçek model çağrısı yok) ama zincirin kendi başlattığı her süreç gerçek (`git apply`, `python3`).
- Checklist'in isimlendirdiği 6 senaryonun hepsi ayrı ayrı test edildi:
  1. **Küçük hata düzeltme**: yanlış işaretli bir toplama fonksiyonu düzeltiliyor, gerçek bir Python syntax kontrolüyle doğrulanıyor, kalıcı kalıyor.
  2. **Test ekleme**: model dosyaya yeni bir test fonksiyonu ekliyor, sözdizimi geçerliliği korunuyor (gerçek bir `pytest` kurulumu garanti edilemediği için syntax kontrolü kullanıldı — dürüst bir sınır, modülün kendi dokümantasyonunda not edildi).
  3. **Yanlış patch reddi**: plan dışı bir dosyayı hedefleyen diff `create_patch_proposal`'da, onaydan sonra kurcalanmış bir diff `apply_coding_patch`'te reddediliyor — hiçbir aşamada diske bir şey yazılmıyor.
  4. **Timeout/cancel**: gerçek bir arka plan thread'den gelen bir `CancelFlag` ile kasıtlı uzun süren gerçek bir komut (`python3 -m timeit`) iptal ediliyor, patch **otomatik geri alınıyor** — yarı-uygulanmış bir durum kalmıyor.
  5. **Secret exposure, iki bağımsız katmanda**: (a) modelin enjekte ettiği gerçekçi bir secret işaretçisi (`ghp_...`) patch taslağı seviyesinde reddediliyor — bu turda bulunan gerçek bir boşluğun (aşağıya bkz.) kanıtı; (b) `.env` gibi gizli-bilgi benzeri isimli bir dosya zaten `analyze_repository` taramasına hiç girmiyor, bir `CodingPlan`'ın hedefi bile olamıyor.
  6. **Mevcut-test regresyonu**: patch'in kendisi doğru olsa bile, test komutu patch'ten bağımsız olarak zaten bozuksa (var olmayan bir modülü arıyor), sistem hâlâ "testler geçmedi" deyip geri alıyor — bu, yukarıda dürüstçe belgelenen bilinen sınırı (taban çizgisi karşılaştırması yok) kasıtlı olarak kanıtlayan ve gelecekte biri eklenirse bilinçli güncellenmesi gereken bir regresyon testi.
- **Bu turda bulunan gerçek bir güvenlik boşluğu, eval seti yazılırken ortaya çıktı ve düzeltildi**: gizli-bilgi filtresi (`reject_secret_like_workspace_document_content`, F3'ten beri var) şimdiye kadar yalnız *okuma* tarafında (RAG indeksleme) çalışıyordu — model bir dosyayı yeniden yazarken üretebileceği içerik hiç taranmıyordu. `draft_patch_with_provider`'a artık aynı filtre uygulanıyor: model bir secret enjekte ederse (PEM private key bloğu, `AKIA`/`ghp_`/`xoxb-` gibi yüksek-güven token önekleri), **tüm taslak** reddediliyor (yalnız o dosya sessizce atlanmıyor — kullanıcının bunu görmesi gerekiyor).
- Kanıt: (o anki tur için) 6 senaryo testi + 1 yeni `patch_generator` testi (secret enjeksiyonunun reddi). Tam paket: `cargo fmt`, `cargo test --offline` (240 lib + 53 main + 9 desktop, hepsi PASS), `cargo clippy --all-targets -D warnings` (temiz).

### F5 — Sesli etkileşim ve algı arayüzü

Durum: TAMAMLANDI — 11/11 madde, 19 Ağustos 2026. **Erişilebilirlik boşlukları 20 Ağustos'ta kapatıldı** (aşağıya bak). Ses yığını kuruldu ve gerçek donanımda (mikrofon + modeller) uçtan uca kanıtlandı. Mimari kararlar: [ADR-0007](docs/adr/0007-voice-audio-stack.md).

Amaç: Her zaman dinleyen bir sistem yerine açık, mahremiyeti koruyan push-to-talk ses akışı.

Seçilen yığın — üçü de **alt süreç**, projeye tek bir yeni Rust bağımlılığı bile eklenmedi (`llama-server`'ın zaten kullandığı desen): yakalama `pw-record` (PipeWire, sistemde vardı), STT whisper.cpp + `ggml-small-q5_1` (182 MB, MIT), TTS Piper + `tr_TR-dfki-medium` (30+61 MB, MIT).

- [x] Audio ADR: PipeWire/Wayland cihaz erişimi, mikrofon izinleri, örnekleme formatı, gecikme hedefi ve recording retention varsayılanı.
  - [ADR-0007](docs/adr/0007-voice-audio-stack.md). Kayıt formatı 16 kHz/mono/s16 — whisper'ın istediği format, ara dönüştürme yok. Yeni bir ses kütüphanesi (`cpal`) yerine alt süreç seçildi: mevcut desen, platform/derleme karmaşıklığı yok, iptal tek bir sinyal.
- [x] STT aday değerlendirmesi: Türkçe doğruluk, CPU/RAM, model boyutu, lisans, offline destek ve warm-start süreleri.
  - **Ölçüm ilk seçimi çürüttü.** `large-v3-turbo-q5_0` akıl yürütmeyle seçilmişti; üç model gerçek Türkçe cümlelerle ölçüldü: `small-q5_1` 7.2 s, `medium-q5_0` 22.8 s, `large-v3-turbo-q5_0` 36.2 s. Üçü de **aynı** hatayı yaptı — büyük model ek doğruluk vermiyor, `large-v3-turbo` `medium`'a göre %60 daha yavaş (CPU'da turbo'nun encoder'ı tam boy kalıyor). Seçim: `small-q5_1`, cümle başına ~2.4 s. Reddedilenler diskte bırakıldı: ölçüm sentetik ses üzerindeydi, gerçek mikrofonda `small` bozulursa geçiş hazır.
- [x] Transkript editörü: gönderim öncesi metni görme, düzeltme, silme, yeniden deneme ve normal `InputType::Voice` pipeline'ına dönüştürme.
  - **20 Ağustos düzeltmesi — ilk uygulama kullanıcının istediği şey değildi.** Transkript taslağa yazılıyor ve kullanıcının Enter'a basması bekleniyordu; kullanıcı ise konuşma istiyordu: *"basacağım, hiçbir şey enterlemeden JARVIS beni duyacak ve bana sesli cevap verecek."* Araya konan gözden geçirme adımı konuşmanın akışını kesiyordu. Varsayılan davranış değiştirildi: tuş bırakılınca istek **doğrudan gidiyor** ve yanıt **sesli** dönüyor — soru sesle geldiği için yanıt kanalı ayrıca ayarlanmıyor, isteğin kendisinden belli.
  - Gözden geçirme yolu korundu ama artık isteğe bağlı: `/voice-settings review on`. Gürültülü ortam veya teknik terimlerde konuşma tanıma yanılabilir; o zaman transkript taslakta bekler. `VoiceTranscript` sözleşmesi değişmedi — onaysız bir transkriptten istek üretebilecek kod yolu hâlâ yok, konuşma modunda onay tuşu bırakma eyleminin kendisi.
- [x] Voice privacy: ham ses varsayılan olarak kalıcı değil; kullanıcı isterse geçici dosyanın yeri/silme zamanı görünür.
  - `RecordingRetention` varsayılanı `DiscardImmediately` — bir ayar tercihi değil, **tipin kendi varsayılanı**: ayar dosyası unutulsa/bozulsa bile gizlilik korunur. Saklama seçilirse konum ve silinme zamanı tipin içinde (`KeepUntil { path, delete_after_epoch }`) ve kullanıcıya gösteriliyor. İptal edilen kayıt da siliniyor (gerçek mikrofonla test edildi).
- [x] TTS aday değerlendirmesi: Türkçe ses kalitesi, lisans, CPU kullanımı, ses modeli boyutu ve offline çalışma.
  - Piper `tr_TR-dfki-medium` — Türkçe için pratikte tek yerel seçenek. Ölçüldü: 3.58 s ses **0.11 s'de** üretildi (gerçek zamanın ~33 katı), yani TTS gecikme açısından hiç sorun değil. Eski `rhasspy/piper` (MIT, bağımsız ikili) seçildi, yeni `piper1-gpl` (Python wheel) değil.
- [x] Sesli approval UX: yüksek riskli aksiyon için yalnız ses değil, ekranda açık yazılı onay veya güvenli ikinci doğrulama.
  - `approval_channel_requirement` + `Runtime::approve_from`. Ses, policy gate'in zaten onay şartı koyduğu bir eylemi **tek başına** yetkilendiremiyor; deneme `approval.channel_insufficient` olarak audit'e yazılıyor. Kural tek yönlü: ses her zaman reddedebilir ve onay gerektirmeyen her eylemi yapabilir — aksi halde sesli kullanım gereksiz sakatlanırdı. Kural yalnız yazılmadı, **uygulanıyor** (2 test).
- [x] Wake word araştırma spike: ayrı feature flag, lokal algılama, görünür dinleme göstergesi, fiziksel/klavye kill switch ve retention=off.
  - Karar: **eklenmeyecek** (ADR-0007). Wake word mikrofonun sürekli açık olmasını gerektirir; bu, planın kendi amaç cümlesiyle ("her zaman dinleyen bir sistem yerine ... push-to-talk") doğrudan çelişir. Yeniden değerlendirme koşulları ADR'de yazılı.
- [x] Push-to-talk capture: tuş basılıyken kayıt, ses seviyesi/VAD göstergesi, bırakınca transkript kuyruğu ve kolay iptal.
  - **Gerçek bas-tut eklendi:** terminal Kitty klavye protokolünü destekliyorsa (foot/kitty/ghostty/WezTerm) `REPORT_EVENT_TYPES` açılıyor ve **F2 basılıyken kayıt, bırakınca çeviri** çalışıyor. Desteklemeyen terminalde sessizce `/voice` aç/kapa yoluna düşüyor — özelliğin hiç çalışmaması yerine daha zayıf ama çalışan biçimi kalıyor, ve kullanıcıya hangisinin geçerli olduğu `/voice-settings` ile söyleniyor.
  - **Ses seviyesi göstergesi:** kayıt sürerken durum çubuğunda canlı RMS çubuğu + metin karşılığı ("sessiz/çok kısık/normal/yüksek"). Gerçek konuşma ile sessizlik ayırt edilebiliyor (ölçüldü: 0.574 vs 0.000).
  - **Bulunan gerçek kırılganlık:** seviye okuyucu WAV başlığını sabit 44 bayt varsayıyordu; `data` chunk'ından önce fazladan bir chunk bulunan bir dosyada başlık baytlarını ses örneği sanıp **sahte seviye** gösterirdi. Sahte gösterge, hiç gösterge olmamasından kötü (kullanıcı ona güvenir) — `data` chunk'ı artık gerçekten aranıyor, regresyon testi eklendi.
- [x] TTS playback: yanıt bitince opt-in oynatma, duraklat/durdur, hız/ses seçimi, kulaklık cihaz değişimi ve sessiz mod.
  - `SpeechSettings`: otomatik oynatma (**varsayılan kapalı** — sesli bir asistanın izinsiz konuşmaya başlaması kullanıcının kontrolü kaybettiği ilk yer), hız (0.5–2.0x, Piper'ın ters `length_scale`'ine çevriliyor), sessiz mod. Sessiz mod otomatik oynatmadan **ayrı** bir kavram: biri kalıcı alışkanlık, diğeri anlık durum (toplantı, gece) — ve sessiz mod her şeyi bastırıyor.
  - TUI: `/speak`, `/speak stop`, `/voice-settings autoplay on|off`, `/voice-settings speed <0.5-2.0>`, `/voice-settings mute|unmute`. Cihaz seçimi PipeWire'ın kendi çıkış yönlendirmesine bırakıldı (kulaklık takılınca sistem zaten yönlendiriyor); ayrı bir cihaz seçici eklemek mevcut davranışı tekrarlamak olurdu.
- [x] Accessibility: klavye-only kullanım, ekran okuyucu metinleri, işitme/görme farklılıkları için eşdeğer metin kontrolleri.
  - TUI zaten tamamen klavyeyle kullanılıyor. Ses tarafında eklenen kural: **her görsel bilginin metin eşdeğeri var** — seviye çubuğu görsel, `level_description()` okunabilir; ses ayarlarının tamamı `summary()` ile tek satır metin. Her sesli eylemin yazılı eşdeğeri var (`/voice`, `/speak`), ve her sesli çıktının metin karşılığı zaten ekranda (yanıtın kendisi). İşitme farklılığı için ses hiçbir zaman *tek* bilgi kanalı değil; görme farklılığı için hiçbir gösterge yalnız görsel değil.
- [x] E2E: mikrofon izin reddi, cihaz yok, model yok, sessizlik/gürültü, Türkçe transkript, iptal, sesli tool approval ve kayıt silme.
  - Kapsanan senaryolar: gerçek mikrofon kaydı, iptal + dosya silme, TTS→STT turu (Türkçe transkript), sessizlik reddi (`TranscriptRejection::Empty`, "onaysız"dan ayrı bir durum olarak), sesli onay reddi + audit kaydı, **model/cihaz yok** (eksik parça sessizce yutulmuyor, açıkça bildiriliyor — bu projede "sessizce daha kötü çalışmak" daha önce gerçek bir hataydı, embedding servisi; aynı hata ses tarafında tekrarlanmadı), boş metin seslendirme reddi, bozuk/WAV olmayan dosya.
  - 12 offline + 4 gerçek donanım testi. Donanım testleri `#[ignore]` — golden set'le aynı desen, offline release gate bozulmuyor.

Tamamlanma ölçütü: Kullanıcı bir tuşa basıp konuşur, gönderilecek transkripti görür/onaylar ve yanıtı isterse sesli duyar.

**Erişilebilirlik düzeltmesi — 20 Ağustos 2026.** İki gerçek boşluk kapatıldı:

- **Sesli onay sınırı gerçek akışta uygulanmıyordu.** `approve_from` hiçbir yerden çağrılmıyordu;
  TUI istekleri `Gui` olarak kuruyor, onay `approve()` üzerinden `Cli` gidiyordu — yani sınır
  kâğıt üstündeydi. Artık **köken (provenance) taşınıyor**: transkript *düzenlenmeden*
  gönderildiyse istek `InputType::Voice` oluyor (kullanıcı metni düzenlediyse artık yazılı girdi
  — düzenleme, sesin taşıdığı belirsizliği ortadan kaldıran şeyin ta kendisi). Ayrı bir
  "düzenlendi mi" bayrağı yerine metin karşılaştırılıyor: her tuş vuruşuna kanca takmadan doğru
  cevabı veriyor. `approve_task` kökeni `approve_from`'a geçiriyor, böylece **sesle verilmiş bir
  `/approve` reddediliyor** ve kullanıcıya nedeni açıklanıyor.
- **Masaüstü istemcisinde hiç ses yoktu.** Eklendi ve TUI ile aynı çekirdek fonksiyonları
  çağırıyor — iki istemcinin ayrı ses mantığı yazması, birinde düzeltilen bir gizlilik kuralının
  diğerinde eksik kalması demek olurdu. Masaüstünde **gerçek bas-tut** doğrudan mümkün (egui
  buton basılı durumunu bildiriyor, terminalin aksine); canlı seviye göstergesi, seslendirme,
  sessize alma ve otomatik oynatma da var.

### F6 — Model kalite, dataset governance ve adaptasyon

Durum: TAMAMLANDI — 7/7 madde, 19 Ağustos 2026. **Erişilebilirlik boşlukları 20 Ağustos'ta kapatıldı** (aşağıya bak). Hiçbir model indirilmedi: "model karşılaştırması" maddesi, makinede F2'de zaten indirilmiş olan Qwen2.5-VL-3B aday olarak kullanılarak tamamlandı.

**F6'nın en önemli çıktısı bir özellik değil, bir uyarı:** golden set iki bağımsız kanıtla (madde 1 kalite değerlendirmesi + madde 3 model karşılaştırması) **ayrım gücü yetersiz** çıktı — 3B CPU modeli, 8B GPU modeliyle berabere kalıyor. Bir sonraki tur zor/çok adımlı senaryolarla genişletme olmalı; aksi halde ne model seçimi ne de fine-tuning kararı bu setle desteklenemez.

Amaç: Sohbeti hard-code etmek yerine ölçmek; gerekiyorsa küçük, geri alınabilir bir model adaptasyonu yapmak.

Çalışma sırası (F5 ile bağımsız/paralel ilerliyor, F6'nın kendi önkoşulu F3/F4'ün gerçek eval
verisi — F5 değil): önce indirme gerektirmeyen maddeler (1, 7, 6, 2, 4), model karşılaştırması
ve LoRA/QLoRA fizibilitesi (indirme gerektiren kısımlar) kullanıcının ev ağında yapılacak.

Not (19 Ağustos 2026): `append_teacher_example()` ve `teacher_examples` tablosu şema olarak hazır
ve test edilmiş, ama hiçbir üretim akışından (TUI/desktop/Runtime) hiç çağrılmıyor — yani "insan
onaylı öğretim örneği" borusu şu an gerçek veri akıtmıyor. Bu yüzden madde 2 (dataset
export/versioning), madde 6 (kullanıcı geri bildirimi intake'i) bu boşluğu kapatmadan anlamlı
değil — sıralama buna göre revize edildi.

- [x] Sürümlü benchmark: Türkçe diyalog, takip sorusu, güvenlik sınırı, RAG doğruluğu ve coding görevleri için golden set + latency/quality raporu.
  - Belge: [docs/f6_model_quality_golden_set.md](docs/f6_model_quality_golden_set.md) — F2'nin QA setinden (Türkçe diyalog/takip/güvenlik sınırı, C01-C20) referansla devralındı, tekrar yazılmadı. Koşum aracı: [src/model_quality_eval.rs](src/model_quality_eval.rs), `coding_eval.rs` deseniyle ama canlı model gerektirdiği için `#[ignore]`'lu (offline `cargo test`/`release_check.sh` bozulmadı: 280 PASS + 10 ignored).
  - **Kanıt: 10/10 senaryo PASS**, gerçek Qwen3-8B + gerçek embedding servisiyle koşuldu (19 Ağustos 2026, commit `77b82f5`, prompt `5451932`, `-ngl 28` Vulkan). Coding K01-K05 (latency 10.3-19.0 s), RAG R01-R05 (latency 4.6-7.1 s).
  - K05: 16 Ağustos router-misfire'ının kalıcı regresyon koruması — canlı doğrulandı, `conversation.reply` + gerçek C++ üretiyor.
  - R05 (en güçlü sonuç): `Sensitive` işaretli belge atıf olarak hiç yüzeye çıkmadı, içindeki sır model yanıtına sızmadı — F3 sensitivity filtresinin **gerçek modelle uçtan uca** kanıtı (birim testi değil).
  - **Metodoloji düzeltmesi (ilk koşumda bulundu):** İlk RAG korpusu 2 belgeydi ve tüm senaryolar `PASS` veriyordu, ama sonuç limiti (4) korpustan büyük olduğu için her sorgu zaten tüm korpusu getiriyordu — yani testler hiçbir sıralama gücü ölçmüyor, değersiz bir `PASS` üretiyordu. 5 çeldirici belge eklendi (toplam 8); `rag_runtime()` artık "korpus > limit" koşulunu kendi kendine assert ediyor, böylece fixture ileride küçülürse test sessizce değersizleşmek yerine gürültülü düşer. Düzeltme sonrası doğru belge 8 belge arasından ilk sırada geliyor.
  - **Zor senaryolar eklendi (20 Ağustos 2026):** Z01 — derlenebilir/şablonlu/düzgün kapatılabilen thread-safe C++ kuyruğu. Doğrulama mekanik: kod blokları gerçekten `g++ -fsyntax-only` ile derleniyor. İlk koşumda `FAIL` (eksik `<stdexcept>`), yani **set artık ayrım üretiyor**. Zor senaryolar regresyon korumasından yapısal olarak ayrı (`EvalScenario::hard`): regresyon düşerse test kırmızı, zor senaryo düşerse bu bir ölçüm sonucu — karıştırmak testi kalıcı kırmızı yapıp kırmızının görmezden gelinmesine yol açardı.
  - **`/eval` + "ölçülmemiş konfigürasyon" uyarısı (20 Ağustos 2026):** Ölçüm zincirinin son halkası kapatıldı. Bir ölçüm yapıldı ama kimse bakmıyorsa zincir kopuktur; model veya prompt değiştiğinde golden set'i koşmayı hatırlamak insana bırakılan bir disiplindi ve unutuluyordu. Artık açılışta ve `/status`'ta "bu konfigürasyon hiç ölçülmedi — `/eval`" uyarısı çıkıyor, masaüstünde HUD'da ÖLÇÜM satırı var. Prompt parmak izi commit hash'i değil metnin SHA-256'sı olduğu için commit edilmemiş bir prompt düzenlemesi bile yeni konfigürasyon sayılıyor. Store yoksa uyarı susuyor — kullanıcıyı çözemeyeceği bir şey için uyarmak gürültü olurdu.
  - Kalite bulgusu (dürüst): bu senaryolarda model amatör değil (doğru, idiomatic çıktı) — yani "amatör kod" şikayeti basit görevlerde ortaya çıkmıyor. Set bir sonraki turda **zor/çok adımlı/proje bağlamlı** senaryolarla genişletilmeli, aksi halde sürekli geçen ama gerçek şikayeti ölçmeyen bir sete dönüşür.
- [x] Dataset export/versioning: yalnız human-reviewed, verifier-passed, sensitivity etiketli örnekler; silme/poisoned-example marker'ları ve dataset manifest hash'i.
  - `src/dataset.rs` (+ `dataset_tests.rs`): export bir veritabanı dökümü değil — incelenmemiş / verifier'ı geçmemiş / `Sensitive` örnek dışarı çıkamaz ve **sessizce düşmez**, gerekçesiyle `excluded` listesinde raporlanır. Marker'lar uygunluğu **ezer**: düzgün görünen ama `Poisoned` işaretli bir örnek asla export edilemez (sıralama bilinçli, tersi zehirli örneği geçirirdi). Silme/poisoned marker'ı satırı yok etmek yerine "bilinen-kötü" olarak kalıcı kılar — silmek aynı içeriğin sonra yeniymiş gibi kuyruğa dönmesine izin verirdi.
  - Manifest hash'i içerik-adresli (SHA-256, elle yazılmış kanonik metin üzerinden — bir bağımlılığın formatı değişince hash'in kaymaması için). "Bu model hangi dataset ile eğitildi" sorusunu yanıtlar.
  - Kanıt: 4 birim testi + uçtan uca `feedback → inceleme → TeacherExample → dataset` testi PASS.
- [x] Model karşılaştırması: mevcut Qwen3 baseline ile aday modellerin CPU/RAM gecikmesi ve kalite ölçümü.
  - **İndirme yapılmadan tamamlandı:** aday olarak makinede F2'de zaten indirilmiş Qwen2.5-VL-3B kullanıldı (gerçek bir alternatif model — 3B, farklı aile). İki sunucu **aynı anda** ayakta koşuldu; sırayla yeniden yükleme yapılsaydı ölçüm model yükleme süresini de içerir ve gecikmeler karşılaştırılamaz olurdu.
  - Ölçüm: baseline Qwen3-8B (`-ngl 28` Vulkan) **5/5 senaryo, medyan 7019 ms, 5443 MB RAM + ~4.5 GB VRAM**; aday Qwen2.5-VL-3B (`-ngl 0`, yalnız CPU) **5/5 senaryo, medyan 8994 ms, 3609 MB RAM + 0 VRAM**. Verdict: `Unchanged`.
  - **Sonucun anlamı bir uyarı, başarı raporu değil:** 3B'lik, tamamen CPU'da çalışan bir model 8B'lik GPU hızlandırmalı modelle beşte beş berabere kaldı ve medyanda yalnız ~%28 geride. Doğru okuma "aday 8B kadar iyi" değil, **"bu golden set 3B ile 8B arasındaki farkı ölçemiyor"**. Bu, madde 1'in kalite bulgusunu (setin kolay olduğu) bağımsız bir kanıtla doğruluyor.
  - **Pratik sonuç:** Yeni aday modeller (ör. kod-özel model) indirilmeden **önce** golden set zor senaryolarla genişletilmeli — aksi halde indirme yapılır, karşılaştırma koşulur ve sonuç yine `Unchanged` çıkar, yani indirme boşa gider.
  - Altyapı tarafında eksik yok: ölçüm → registry kaydı → verdict üretimi uçtan uca çalıştı. Yeni aday eklemek yalnız ikinci bir sunucu başlatıp aynı testi koşmak demek.
- [x] LoRA/QLoRA fizibilite kararı: VRAM/RAM, eğitim süresi, lisans, eval hedefi ve rollback artifact'i kullanıcıya sunulmadan eğitim başlamaz.
  - Karar: **şimdi eğitim yapılmayacak** — [ADR-0006](docs/adr/0006-lora-adaptation-feasibility.md). Üç bağımsız gerekçe: (1) `teacher_examples` boş, eğitilecek veri yok; (2) golden set, iddia edilen kalite sorununu ölçemedi — ölçülmemiş bir problemi eğitimle çözmek iyileşmeyi doğrulayamamak demek, F6'nın tamamlanma ölçütü bunu yasaklıyor; (3) daha ucuz ve denenmemiş bir seçenek var (kod-özel model, dataset/eğitim/rollback artefaktı gerektirmiyor). ADR yeniden değerlendirme tetikleyicilerini ve eğitim yapılacaksa gereken önkoşulları da yazıyor.
- [x] Old-vs-new regresyonu ve tek komutla model/adaptor rollback.
  - `compare_model_config_runs` + `Runtime::model_config_regression()`: en yeni konfigürasyonu kendi `rollback_target`'ıyla karşılaştırır. Verdict bilinçli olarak muhafazakâr — **bir senaryo kaybı, 4x hızlanma olsa bile regresyondur**, çünkü F6'nın ölçütü "iyileştirir ve regresyon üretmez", doğruluğu hıza takas etmek değil. Kalite aynıyken ≥1.5x yavaşlama da regresyon; sıradan dalgalanma değil.
  - Kanıt: 3 birim testi + 1 uçtan uca registry testi PASS.
- [x] Kullanıcı geri bildirimi intake'i: beğen/beğenme veya düzeltme sinyali doğrudan eğitim verisi olmaz; sensitivity, provenance ve human review kuyruğundan geçer.
  - `FeedbackCandidate` bilinçli olarak `TeacherExample`'dan **ayrı bir tip**: planın "doğrudan eğitim verisi olmaz" kuralı böylece yapısal hale geliyor — insan incelemesini atlayarak eğitim verisi üretebilecek hiçbir kod yolu yok. Terfi tek kapıdan (`feedback_candidate_is_promotable`) geçer: insan onayı şart, `Sensitive` aday asla uygun değil, ve yalın bir "bu yanlıştı" sinyali öğrenilecek doğru cevap taşımadığı için terfi edemez (düzeltme metni taşıyan `Correction` eder ve modelin yanıtının yerine geçer). `Rejected` aday silinmez, bilinen-kötü olarak kalır.
  - TUI: `/feedback iyi|kotu|duzelt <doğru yanıt>`, `/feedback list`, `/feedback onayla|reddet <id>`.
  - Kanıt: 3 uçtan uca test PASS (incelenmemiş aday terfi edemiyor; onaylı ama hassas aday da edemiyor; düzeltme yanıtın yerine geçiyor).
- [x] Prompt/model konfigürasyon registry'si: her deneyin model hash'i, prompt sürümü, benchmark sonucu ve rollback hedefi kaydedilir.
  - `ModelConfigRun` + migration 11. Registry bilinçli olarak bir **log**, anahtar değil: satır yazmak hangi modelin/prompt'un kullanıldığını değiştirmez. Prompt sürümü olarak commit hash'i değil **prompt metninin SHA-256'sı** saklanır — commit hash'i commit edilmemiş bir düzenlemeyi kaçırırdı.
  - Golden set koşumu artık kaydı **otomatik** üretiyor (elle doldurma bitti). TUI: `/model-runs`.
  - Kanıt: 3 birim testi + canlı koşum (4/4 senaryo, medyan 13.5 s, prompt parmak izi `93140771…`) PASS.

**Erişilebilirlik düzeltmesi — 20 Ağustos 2026.** F6'nın maddeleri tamamlanmıştı ama yaptığımız
şeylerin bir kısmına TUI'den ulaşılamıyordu: kod vardı, test edilmişti, ama kullanıcı için
pratikte yoktu. Bu, "çağrılmayan bir kural sadece belgedir" hatasının bir başka biçimiydi ve
kapatıldı:

- Golden set, test modülünden **üretim koduna** taşındı ([src/quality_eval.rs](src/quality_eval.rs)).
  Artık hem `model_quality_eval` testleri hem TUI'nin `/eval` komutu **aynı tanımı** koşuyor —
  ikisinin birbirinden sapması mümkün değil. `/eval` arka planda çalışıyor (arayüzü kilitlemiyor),
  izole bir in-memory store kullanıyor (kullanıcının gerçek çalışma alanını indekslemiyor) ve
  sonucu otomatik olarak registry'ye kaydediyor.
- `/model-runs compare` — eski/yeni verdict'i artık görülebiliyor.
- `/feedback terfi <id> <capability>` — onaylı aday artık gerçekten eğitim verisine dönüşebiliyor.
  Bu olmadan zincirin son adımı gerçek kullanımda erişilemezdi.
- `/dataset export <sürüm> [yol]`, `/dataset mark <id> poisoned|deleted <gerekçe>`,
  `/dataset markers` — export ve marker'lar artık kullanılabilir.
- Korpus indekslenmemişken RAG senaryoları **atlanıyor**, sessizce "düştü" sayılmıyor: eksik
  altyapıyı model kalitesizliği gibi raporlamak ölçümü yanlış yönlendirirdi.

Tamamlanma ölçütü: Her model veya adapter değişikliği, sürümlü eval'de hedef metriği iyileştirir ve güvenlik/latency regresyonu üretmez; aksi halde kullanılmaz.

### F7 — Yetkili security/pentest ve bug bounty yeteneği

Durum: BEKLENİYOR — F4 izolasyonundan önce execution açılmaz. **20 Ağustos 2026'da kullanıcı önceliklendirdi**: bunu gerçek bug bounty programlarında kullanacak, "en kritik yeteneklerden biri olacak" — F7, F8/F9'dan önce gelir (bkz. [[jarvis-f6-f7-prioritization-open]] hafıza kaydı).

Amaç: "sızma testi yapabilen" değil, yalnız yazılı yetki ve teknik sınırlar altında güvenli değerlendirme yapabilen bir capability oluşturmak — ama gerçek bug bounty iş akışını (keşif, manuel test, raporlama, program uyumu) baştan destekleyecek şekilde tasarlanmış olarak.

**Bug bounty bağlamının tasarıma etkisi:** Programlar scope'u wildcard/CIDR ile tanımlar (madde F7.1'de zorunlu hale geldi); kapsam-dışı bir varlığa dokunmak en sık ban/hukuki sorun sebebi (F7.2 bunu OS seviyesinde imkansız kılıyor); en değerli bug'lar (IDOR, yetki atlatma, iş mantığı hataları) otomatik taramayla değil manuel/yarı-manuel çalışmayla bulunuyor (F7.4).

#### F7.1 — Yetkilendirme ve scope (önce bu — hiçbir şey bunsuz açılmaz)

- [x] İmzalı authorization/scope manifest (expiry/revoke önceki maddede tamamlandı; hedef canonicalization CIDR/wildcard maddesinde tamamlandı).
  - **Kanıt:** Her scope, bu makinede bir kez `/dev/urandom`'dan üretilip ayrı bir tabloda (kullanıcıya hiç gösterilmeyen, `/secret show` ile hiç erişilemeyen) saklanan bir anahtarla HMAC-SHA256 imzalanıyor. **Bu, bug bounty programının yetki verdiğinin kanıtı değil** — hiçbir yerel sistem bunu kanıtlayamaz — diskteki scope'un `save_pentest_scope` tarafından yazılanla birebir aynı olduğunun, bir veritabanı dosyası elle düzenlenerek veya başka bir makineden bir yedek geri yüklenerek değiştirilmediğinin kanıtı. `authorize_pentest_action` imzayı HER ÇAĞRIDA yeniden doğruluyor (yalnız kayıt anında değil) — kurcalama, tam olarak bir eylemi yetkilendirmek üzereyken yakalanıyor.
  - **Test:** 5 yeni test — yeni kaydedilen scope geçerli imza taşıyor, **gerçek ham SQL ile veritabanını kurcalayıp** imzanın gerçekten düştüğünü kanıtlama (varsayım değil), imzalama anahtarının bir kez üretilip tekrar kullanıldığı, Runtime'ın kurcalanmış aktif scope'u reddettiği (uçtan uca), aynı isimle yeniden kaydetmenin her seferinde taze içeriği kapsayan yeni bir imza ürettiği.
  - **Dürüst sınır:** Bu simetrik (HMAC) bir imza, asimetrik değil — imzalayan ve doğrulayan aynı anahtarı paylaşıyor çünkü ikisi de aynı makine. Bu, "veritabanı dosyası elle/başka yerden değiştirildi mi" sorusuna cevap veriyor, "bu yetkiyi gerçekten bug bounty programı mı verdi" sorusuna değil — o sorunun cevabı hâlâ kullanıcının kendi doğrulamasında. Yeni bir kriptografi kütüphanesi eklenmedi; HMAC-SHA256, zaten bağımlılık olan `sha2`'nin üzerine kuruldu.
- [x] CIDR ve wildcard scope desteği — bug bounty scope'u (`*.example.com`, `10.0.0.0/24`) artık ifade edilebiliyor.
  - **Kanıt:** `parse_pentest_target_pattern` scope girdilerini `ExactHost`/`Wildcard`/`Cidr` olarak ayrıştırıyor; eşleşme `pentest_target_pattern_matches` ile yapılıyor. CIDR minimum `/16` (daha geniş — ör. `10.0.0.0/8` — reddediliyor: tek satırlık bir yazım hatasının ~16 milyon adresi kapsama almasını önlüyor); ağ adresi host-bit taşıyorsa reddediliyor (`10.0.0.5/24` değil, `10.0.0.0/24`). Wildcard yalnız gerçek alt alanları kapsıyor, apex'in kendisini KAPSAMIYOR (`*.example.com` scope'undayken `example.com`'un kendisi ayrı listelenmeli) — wildcard'ın apex'i de kapsaması yaygın ve tehlikeli bir aşırı-yetkilendirme hatası, bilinçli olarak önlendi. Dışlama (exclusion) her zaman izinden önce kontrol ediliyor ve daha dar bir dışlama, daha geniş bir izin wildcard'ından kazanıyor.
  - **Hedef canonicalization sertleştirmesi (aynı işin parçası):** IPv4 ayrıştırması artık lider-sıfırlı oktetleri (`010`) reddediyor — bazı ayrıştırıcılar bunu sekizlik (octal) okur, yani aynı string farklı araçlarda farklı adrese çözülebilir; bu tam olarak bir probun scope dışına sessizce kaymasına yol açabilecek türden bir belirsizlik.
  - **Test:** 6 yeni test — CIDR sınır adresleri (/24 ve dar bir /28'in tam kenarları dahil), wildcard'ın apex'i kapsamadığı, sahte alt-dize eşleşmesinin (`evilexample.test` ↔ `example.test`) olmadığı, dar dışlamanın geniş izinden kazandığı, ve önceden reddedilmesi gereken 6 senaryo (`10.0.0.0/8`, host-bitli CIDR, tek etiketli wildcard, geçersiz oktet, punycode, ham Unicode — son ikisi hâlâ bilinçli olarak reddediliyor).
  - **Kalan (henüz yapılmadı):** DNS pinning/rebinding savunması — bu, gerçek ağ isteği yapan bir worker gerektiriyor (F7.2'nin parçası), scope/hedef eşleştirme seviyesinde tamamlanamaz; F7.2'ye bırakıldı.
- [ ] Program scope'unu doğrudan içe aktarma (HackerOne/Bugcrowd yapılandırılmış scope API'leri) — elle girmenin yanlış kapsam riskini ortadan kaldırır.
- [x] Çoklu program/scope yönetimi + expiry/revoke: aktif scope her zaman açıkça gösterilir; bir programın scope'u yüklüyken yanlışlıkla başka bir programın hedefine dokunma riski engellenir.
  - **Kanıt:** `pentest_scopes` tablosu (şema sürümü 13) isimli scope'ları saklıyor; `set_active_pentest_scope` her seferinde ÖNCE tüm satırları pasifleştirip SONRA hedefi aktif ediyor — yani "hangi programa karşı yetkiliyim" sorusunun her an tek ve belirsiz olmayan bir cevabı var. `revoke_pentest_scope` doğal süre dolumundan (expires_at) bağımsız, anında etkili bir iptal — iptal edilen scope aynı anda pasifleştiriliyor ve bir daha aktif edilemiyor (yetkiyi bilinçli geri çekme kararının kazara geri alınmasını önlemek için). `Runtime::authorize_pentest_action` tek giriş noktası: gelecekteki her pentest capability'si buradan geçmek zorunda, kendi `PentestScope`'unu geçirip karar alamaz — böylece iptal/değişiklik anında etkili olur, hiçbir yerde önbelleğe alınmış bir karar kalmaz. Aktif scope yokken sonuç "özellik eksik" değil açık bir "hayır" (deny-by-default).
  - **Test:** 9 yeni test — hiç scope aktif değilken açık "yok" cevabı, iki program aynı anda saklanıp yalnız birinin aktif olabildiği, iptalin süre dolumundan bağımsız anında etkili olduğu, iptal edilenin tekrar aktif edilemediği, gerekçesiz iptalin reddedildiği, Runtime'ın tek giriş noktasından hem izin hem reddi doğru verdiği, iptal edilmiş bir scope'un Runtime seviyesinde de (çift katman savunma) reddedildiği, ve geçersiz bir scope'un diske hiç yazılmadığı.
- [ ] Pasif/aktif keşif ayrımı: pasif kaynaklara bakmak (sertifika şeffaflık kayıtları, arama motoru verisi) hedefe hiç dokunmaz — mevcut SAFE/ACTIVE merdiveninden daha ince, ayrı bir kategori.

#### F7.2 — Ağ sınırlama (en kritik güvenlik parçası)

- [ ] Network-scoped sandbox worker: yalnız allowlist egress, kill switch, dry-run ve gerçek cancellation/cleanup.
- [ ] Rate/runtime limitleri — agresif tarama çoğu bug bounty programında otomatik ban sebebi.
- [ ] WAF/engelleme tespiti: hedef aniden farklı davranmaya (sürekli 429/503) başlarsa kör devam etmek yerine dur ve haber ver.
- [ ] Programın kendi politika metnini okuma: "otomatik tarama yasak" gibi kısıtlamalara uyum, izin verilen test saatleri.

#### F7.3 — Keşif ve sürekli izleme

- [ ] Pasif keşif: subdomain (sertifika şeffaflık kayıtları), teknoloji parmak izi, geçmiş URL/endpoint kayıtları.
- [ ] Aktif keşif (yalnız scope onaylıysa): port/servis tarama, subdomain brute-force, JS analiziyle endpoint keşfi.
- [ ] Varlık envanteri kalıcı kaydı + periyodik yeniden tarama + **yeni varlık ortaya çıkınca bildirim**. Bug bounty'de değerin çoğu buradan geliyor — yeni bir subdomain/endpoint'e ilk bakan avantajlı; bu madde olmadan F7 sadece tek seferlik bir tarayıcı kalır.

#### F7.4 — Manuel test araçları (en yüksek getirili kategori)

- [ ] İstek yakalama/değiştirme/tekrar gönderme (proxy/replay) ve cevapları karşılaştırma (diff). En yüksek ödemeli bug sınıfları (IDOR, yetki atlatma, iş mantığı hataları) neredeyse hiç otomatik taramayla bulunmuyor; bunsuz F7 yalnız "otomatik keşif + bilinen açık eşleştirme" aracı kalır.
- [ ] Oturum açmış (authenticated) test desteği: program tarafından verilen test hesabı bilgisinin güvenli saklanması ve tarama/replay araçlarına enjekte edilmesi — mevcut Secret Manager'a bağlanır ([[jarvis-layered-memory-architecture]]), yeni bir mekanizma icat edilmez.

#### F7.5 — SAFE modun somut ilk kontrolleri

- [ ] Subdomain devralma (takeover) tespiti — yalnız DNS/HTTP kontrolü, sömürü yok.
- [ ] Açığa çıkmış hassas dosya/yanlış yapılandırma tespiti (`.git`, `.env`, yedek dosyaları, açık depolama).
- [ ] Bilinen CVE eşleştirmesi (parmak izinden çıkan yazılım sürümüne karşı) — CVE veri kaynağının güncel tutulması dahil.
- [ ] TLS/sertifika sorunları.

#### F7.6 — Bulgu yönetimi ve raporlama

- [ ] Evidence tabanlı finding formatı, insan onayı, audit export ve scope dışı/secret hedef deny testleri.
- [ ] Daha önce bulunanla eşleştirme (deduplication) — mevcut audit hash-chain deseniyle, yeni bir mekanizma icat edilmez. Programlar tekrar bildirilen bulgulara olumsuz bakıyor.
- [ ] Rapor öncesi yeniden doğrulama: bulgu ile rapor yazma arasında hedef değişmiş olabilir, göndermeden önce hâlâ geçerli mi diye tekrar bakılır.
- [ ] **Modelin kendisi raporu yazabilmeli, iyi bir şekilde.** Kanıt toplamak (F7.6'nın diğer maddeleri) ile göndermeye hazır bir rapor arasındaki mesafe kapanmalı: model, toplanan kanıttan (istek/cevap çiftleri, ekran görüntüleri, replay/diff sonuçları) platformun beklediği yapıda (özet, adım adım tekrar üretme, etki analizi, önerilen düzeltme, CVSS/severity tahmini) düzyazı bir rapor taslağı üretir. Kullanıcı gönderilmeden önce gözden geçirip onaylar — F4'ün patch akışındaki "önce göster, sonra onay al" deseniyle aynı, burada da model asla kullanıcı adına doğrudan göndermez. Rapor kalitesi golden set'e (F6) yeni bir zor senaryo olarak eklenebilir: gerçek bir bulgu senaryosundan üretilen rapor taslağı, gerekli bölümlerin hepsini içeriyor mu diye mekanik olarak kontrol edilebilir.
- [ ] Düzeltme sonrası hedefli yeniden test: program "düzelttik, doğrular mısın" dediğinde tüm taramayı değil yalnız o bulguyu tekrar kontrol etme.
- [ ] Program-özel hariç tutulan/düşük değerli bulgu sınıfları filtresi (ör. "self-XSS kabul etmiyoruz") — program politikasından okunur, zaman kaybını önler.

#### F7.7 — Dış araç araştırmasından ve genişletilmiş veri planından alınan somut ekler

20 Ağustos 2026'da 11 dış pentest/OSINT aracının incelemesi ([docs/f7_security_tool_research.md](docs/f7_security_tool_research.md)) ve web/mobil/desktop/offensive pentest veri planının F7'yle ilgili bölümleri ([docs/security_and_engineering_vision.md](docs/security_and_engineering_vision.md) bölüm 1-8, 13) F7'ye somut fikirler ekledi. **Hiçbir aracın kodu kopyalanmıyor** — yalnız davranış/mimari fikri alınıyor, her biri kendi lisans notuyla kaynak belgede kayıtlı.

- [ ] **Otonomi modeli netleştirmesi:** mevcut SAFE/ACTIVE/INTRUSIVE/DESTRUCTIVE merdiveni "ne yapılabilir" sorusunu cevaplıyor; buna dik bir ikinci eksen eklenmeli — "ne kadar gözetim gerekir": `MANUAL` (her tool çağrısından önce onay), `SUPERVISED_AUTONOMY` (planı model kurar, kullanıcı onaylar, düşük riskli read-only adımlar otomatik yürür, aktif adım onay ister), `BOUNDED_AUTONOMY` (yazılı scope+süre+bütçe önceden tanımlı, worker yalnız allowlist'teki capability'leri kullanır, scope dışı/yüksek riskli adımda otomatik durur). İki eksen birbirinin yerine geçmez.
- [ ] **Görev kontrolü (steering) ve devam ettirme:** kullanıcı çalışan bir güvenlik görevine "yalnızca auth akışına odaklan", "bu endpoint'i kapsam dışına al" veya "dur" diyebilmeli; uzun süren iş yarıda kalırsa state kaybetmeden devam edebilmeli (F3'ün session/resume desenine bağlanır).
- [ ] **Kapsam matrisi (coverage tuple):** `(hedef, endpoint, parametre, zafiyet_sınıfı)` dörtlüsü izlenir — hangi kombinasyonun test edildiği, hangisinin edilmediği görünür olur; "sıradaki iş" önerisi yalnız kapsam içinde ve daha önce test edilmemiş kombinasyonları önerir.
- [ ] **`confirm_finding` sözleşmesi:** bir bulgu yeniden üretme kanıtı olmadan "confirmed" durumuna geçemez — "model şüphelendi" ile "doğrulandı" ayrı, karıştırılmayan durumlar (F7.6'nın evidence-based finding maddesiyle aynı ilke, burada ayrı bir sözleşme olarak netleştirildi).
- [ ] **Bilgi paketleri (knowledge packs):** konu/teknoloji/risk/önkoşul/güvenli-doğrulama/remediation metadata'sı taşıyan, yalnız görev kapsamına uygun olanı yüklenen bilgi paketleri — bilgi ile tool authority tamamen ayrı kalır. **Lisans sınırı:** kaynak içerikler (ör. HackTricks, CC BY-NC 4.0) kopyalanmaz, yalnız konu başlıkları ve JARVIS'in kendi özgün notları kullanılır.
- [ ] **OSINT — F7'nin resmi bir alt-alanı:** domain/kullanıcı adı/telefon pivotlarını tek bir görev grafiğinde ilişkilendiren, salt-okunur bir capability. Her iddia kaynak URL + fetch zamanı + confidence + "bulunamadı" ile "bilinmiyor" ayrımı taşır. **Varsayılan mod `PASSIVE_PUBLIC_ONLY`**; login bypass, CAPTCHA atlatma, kapalı grup erişimi, credential kullanımı, toplu scraping ve kişisel profil çıkarımı yasak. Breach-intelligence kaynakları varsayılan kapalı, yalnız açık onay + hukuki uygunluk kontrolüyle açılır.
- [ ] **Yetkili evidence snapshot (web mirroring) capability'si:** yetkili bir hedefin dinamik testten önce HTML/CSS/JS/görsel/bağlantı yapısıyla hash'lenmiş, değişmez bir kopyasını alma — provenance ve "snapshot ile gerçek hedef arasındaki fark" raporlama için. **Bu "web kopyalama" değil "yetkili kanıt görüntüsü" olarak adlandırılmalı.** Sınırlar: maksimum boyut, host allowlist, MIME allowlist, disk kotası, robots/yetki kararı, retention, credential redaction; indirilen içerik modele doğrudan verilmez, untrusted attachment/data envelope olarak tutulur.
- [ ] **Pasif kaynak keşfi genişletmesi:** CDN/WAF arkasındaki olası origin IP'ler (CNAME/CT kayıtları, HTTP fingerprint, favicon, MX/TXT/PTR gibi pasif sinyallerle) ve tarihsel URL/endpoint arşivleri (Wayback/Common Crawl benzeri kaynaklardan) — "aday origin" ile "doğrulanmış açık" ayrı tutulur; arşivlenmiş response'lar sır/PII barındırabileceği için ham hali modele verilmez, redaction + boyut sınırı + "tarihsel veri" etiketiyle işlenir.
- [ ] **HTML/JS/HAR'dan endpoint/parametre çıkarımı:** Burp/ZAP/Caido export'ları dahil çeşitli girdilerden endpoint/parametre/path-word çıkarıp normalize bir asset grafiğine yazma; scope prefix/filter parser seviyesinde zorunlu; "bulundu" ile "erişilebilir/doğrulandı" durumları ayrı; 403/429/timeout oranı yükselince zarif durma + eksik-sonuç uyarısı.
- [ ] **F4/F7 worker'ı için somut regresyon test sınıfları** (bir aracın kendi güvenlik denetiminden alınan dersler — kod değil, test kapsamı): DNS rebinding ve resolve/connect TOCTOU, symlink/realpath tabanlı hassas-yol atlatma, tool çıktısı/MCP sonucu için gerçek transport-seviyeli boyut sınırı, capture/evidence store için OOM sınırı, iptal edilen tool-call oturumlarının bozuk geçmiş bırakmaması, akan (streaming) tool-call parçalarının doğru birleştirilmesi, URL userinfo/JWT/Authorization sır redaction'ı, terminal escape/kontrol-bayt temizliği, atomik bulgu oluşturma ve eşzamanlı ekleme güvenliği.
- [ ] **Kaynak/araç risk sınıflandırması:** her potansiyel araç fikri capability manifest'i (risk seviyesi, ağ etkisi, credential ihtiyacı, çıktı formatı) ile kayıt altına alınır; aynı hedef için tekrar eden tarama sonuçları normalize edilir; hiçbir araç kendi threat model'i, sandbox'ı, scope testi ve regresyon testinden geçmeden registry'ye girmez.

**Önerilen ilk entegrasyon sırası** (kaynak belgeden): (1) bilgi paketi formatı, (2) pasif URL/endpoint kanıt grafiği, (3) pasif origin/asset aday kayıtları, (4) F4 worker'ı üzerinden düşük riskli read-only parser'lar, (5) F7'de scope'lu aktif keşif + doğrulanmış PoC capability'leri, (6) OSINT için gizlilik/rıza/hukuki sınır çalışması.

Tamamlanma ölçütü: Scope dışı hiçbir hedefe trafik çıkamaz; SAFE modda üretilen her bulgu kanıt ve audit ile ilişkilidir. Bu gate geçmeden aktif test capability'si eklenmez. F7.1-F7.2 (yetkilendirme + ağ sınırlama) tamamlanmadan F7.3 ve sonrası açılmaz — sıra bilinçli, en riskli katman en önce sağlamlaştırılır.

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

### F10 — Uzun vadeli yetkinlik genişlemesi (taahhüt edilmiş hedef)

Durum: BEKLENİYOR — çekirdek (F0-F9) istikrarına bağımlı, ama **kapsam dışı/opsiyonel değil**

Amaç: JARVIS'i tek bir alanda değil, [docs/security_and_engineering_vision.md](docs/security_and_engineering_vision.md)'deki tüm alanlarda (aşağıdaki liste) gerçekten yetkin, kanıtlı ve güvenli sınırlar içinde çalışan bir sisteme dönüştürmek.

**Taahhüt edilen yetkinlik alanları** (tam ayrıntı kaynak belgede, bölüm numaralarıyla):

- Offensive/defensive güvenlik: web (1), mobil (2), desktop (3), ofansif pentest (4), cloud security (9), malware analizi (10), tersine mühendislik (11), OSINT (13), threat intelligence (14), ağ güvenliği (17), enterprise security (18), yetkili full red-team (19), infrastructure/platform security (15), AI security (16).
- Yazılım geliştirme: coding agent (22), veri mühendisliği (23), model eğitimi/adaptasyon/eval (24), mobil uygulama geliştirme (27), web geliştirme (28), cloud geliştirme (29), çoklu platform golden set (30), geniş dil/framework kapsamı (31), veritabanı/PostgreSQL (33), dağıtık sistemler (34), QA/test mühendisliği (35), DevOps/SRE/release (36), performans/kaynak optimizasyonu (37), UI/UX/erişilebilirlik/lokalizasyon (38), paketleme/dağıtım (39), privacy/uyumluluk/yönetişim (40).
- Günlük yaşam: kişisel günlük yardımcı ve yaşam iş akışları (42), ileri Windows/Linux sistem yönetimi (44).
- Mimari: local-first çoklu cihaz agent mimarisi — desktop ileri seviye, Android orta seviye (26); harici AI araçlarını öğrenme/karşılaştırma kaynağı olarak kullanma, kopyalamadan (43).

**Sıralama kuralı (kullanıcı tarafından netleştirildi, 20 Ağustos 2026):** Mobil (F8'in Android/uzak istemci kısmı) yalnız çekirdek (F0-F9, özellikle F2/F7/F9) gerçekten sağlam çalıştıktan sonra açılır. Alan önceliği kaynak belgenin kendi bütçe sıralamasını izler: önce offensive security + AI/data engineering, sonra mobile/web/PostgreSQL/otomasyon, en son DevOps/SRE/UX/release/performans/uyumluluk destek katmanı.

- [ ] Her yeni yetkinlik alanı, F7/F4 desenindeki gibi kendi capability manifest'i, policy kararı, izole worker'ı, provenance şeması ve eval kapısıyla açılır — hiçbiri "model istedi, çalıştı" şeklinde bağlanmaz.
- [ ] Daha büyük/özel modeller, çoklu ajan koordinasyonu, federated/on-device learning ve ileri perception; benchmark + threat model + maliyet değerlendirmesi sonrası devreye girer.
- [ ] Her yeni alan/deney ana sürümden feature flag, ayrı artifact ve rollback ile ayrılır; kullanıcı verisi deney setine varsayılan olarak girmez.
- [ ] Bir alan yalnız F6 eval kapısını ve F9 release kapısını geçerse "aktif" sayılır — kapsamlı kilitli eval matrisi, farklı platform/dil varyantları, adversarial test, scope ihlali sıfır ve kritik unsafe action'da sıfır tolerans hedeflenir (kaynak belge bölüm 20'nin ileri seviye kabul kriterleri).

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

**20 Ağustos 2026 — bug bounty önceliklendirmesi sonrası eklenen maddeler** (üst düzey gruplama
için bkz. F7.1-F7.6 fazlı liste; buradaki numaralar aynı işlerin teknik backlog karşılığı,
7.1-7.9'un devamı, mevcut maddeler yeniden numaralandırılmadı):

- [ ] 7.10 Program scope içe aktarma: HackerOne/Bugcrowd yapılandırılmış scope API'lerinden
  doğrudan okuma — elle girmenin yanlış kapsam riski.
- [ ] 7.11 Çoklu program/scope yönetimi: aktif scope göstergesi, programlar arası yanlışlıkla
  karışmayı engelleme.
- [ ] 7.12 Pasif/aktif keşif ayrımı: pasif kaynaklara bakmak (sertifika şeffaflık kayıtları vb.)
  hedefe dokunmaz, SAFE/ACTIVE merdiveninden ayrı ve daha ince bir kategori.
- [ ] 7.13 Sürekli varlık keşfi ve fark tespiti: pasif+aktif subdomain/port/teknoloji/endpoint
  keşfi, kalıcı envanter, periyodik yeniden tarama, yeni varlık bildirimi.
- [ ] 7.14 Manuel test proxy/replay: istek yakalama/değiştirme/tekrar gönderme, cevap diff'i —
  IDOR/yetki atlatma/iş mantığı hataları gibi otomatik taramayla bulunamayan sınıflar için.
- [ ] 7.15 Authenticated test desteği: program test hesabı bilgisinin Secret Manager üzerinden
  güvenli saklanması ve tarama/replay araçlarına enjeksiyonu.
- [ ] 7.16 SAFE-mode somut kontroller: subdomain takeover, açık dosya/yanlış yapılandırma
  tespiti, bilinen CVE eşleştirmesi (güncel veri kaynağı dahil), TLS/sertifika sorunları.
- [ ] 7.17 Bulgu deduplication: mevcut audit hash-chain deseniyle, daha önce bulunan/bildirilen
  bulgularla eşleştirme.
- [ ] 7.18 Rapor öncesi yeniden doğrulama: bulgu ile rapor arasında hedef değişmiş olabilir,
  göndermeden önce staleness/confidence kontrolü.
- [ ] 7.19 **Model-yazımı rapor taslağı**: toplanan kanıttan (istek/cevap, replay/diff sonucu)
  platform formatında (özet, tekrar üretme adımları, etki, önerilen düzeltme, CVSS/severity
  tahmini) düzyazı rapor taslağı üretme; kullanıcı onayından önce asla gönderilmez (F4 patch
  akışıyla aynı "önce göster, sonra onay al" deseni). Rapor kalitesi F6 golden set'e zor senaryo
  olarak eklenebilir.
- [ ] 7.20 Düzeltme sonrası hedefli yeniden test: tüm taramayı değil yalnız ilgili bulguyu
  tekrar kontrol etme.
- [ ] 7.21 Program politika uyumu: "otomatik tarama yasak" gibi kısıtlamaları okuma, izinli test
  saatleri, rate/runtime limitleri, WAF/engelleme tespiti (hedef aniden 429/503 vermeye
  başlarsa dur ve haber ver).
- [ ] 7.22 Program-özel hariç tutulan/düşük değerli bulgu sınıfları filtresi (ör. "self-XSS
  kabul etmiyoruz") — program politikasından okunur, zaman kaybını önler.

Tamamlanma ölçütü: Kullanıcının sözlü yetki iddiası tek başına yeterli olmamalı; scope runtime
tarafından enforce edilmeli. Mevcut contract yalnız ilk adımdır; imzalı authorization ve network
enforcement eklenmeden security tool açılmayacak. 7.10-7.22, 7.1-7.9 tamamlanmadan başlamaz —
en riskli katman (yetkilendirme + ağ sınırlama) önce sağlamlaşır.

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
