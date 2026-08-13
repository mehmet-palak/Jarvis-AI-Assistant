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

## MVP tamamlanma kaydı ve Phase 2 çalışma sırası

Durum: PLANLANDI — 14 Ağustos 2026

MVP tamamlandı: local CPU sohbeti, kalıcı model yaşam döngüsü, terminal sohbet ekranı, onay/policy zinciri, SQLite audit/recovery, ilk MCP ve güvenli read-only capability'ler çalışıyor. Bu, “JARVIS fikrinin ispatı”dır; henüz tam masaüstü ürünü veya eğitilmiş özel model değildir.

Bu noktadan sonra mimariyi yeniden tasarlamak yerine aşağıdaki dikey dilimler sırayla teslim edilecek. Her dilim, kendi kullanıcı akışı, güvenlik sınırı, otomatik testleri ve gerçek smoke kanıtı olmadan `[x]` olmayacak.

### Phase 2 çalışma ilkeleri

- Mevcut Rust core (`Request → Policy → Task → Tool → Verifier → Audit`) korunur; yeni arayüz veya model adapterı bu zinciri bypass edemez.
- Bir model, embedding modeli veya sistem paketi indirilmeden önce boyut, RAM/VRAM etkisi, lisans ve ne için gerektiği kullanıcıya açıkça söylenir; indirme kullanıcı onayıyla başlar.
- Önce günlük kullanım değeri ve veri güvenliği, sonra otonomi gelir. Fine-tuning ve aktif pentest, ölçüm/scope/sandbox katmanlarından önce başlatılmaz.
- Her yeni yetenek için en az bir başarı, bir reddetme/edge-case ve bir gerçek local smoke testi gerekir.
- TUI MVP olarak korunur; native masaüstü kabuğu aynı core'a ikinci bir istemci olarak eklenir. Core yeniden yazılmaz.

### P2.0 — MVP stabilizasyonu ve ürün kalite kapısı

Durum: BEKLENİYOR

Amaç: Yeni büyük yetenek eklemeden önce günlük kullanım regresyonlarını görünür ve tekrar üretilebilir hale getirmek.

- [ ] Gerçek terminal etkileşim matrisi: paste, `Ctrl+V`, `Ctrl+Backspace`, `Ctrl+W`, `Ctrl+U`, mouse wheel, uzun taslak, uzun geçmiş, bildirim, `/quit` ve `exit` için otomasyon + interaktif smoke.
- [ ] Sohbet kalite değerlendirme seti: Türkçe selamlaşma, kısa takip sorusu, uzun bağlam, konu değiştirme, “bilmiyorum” ve tool-iddiası senaryolarından oluşan sürümlü küçük benchmark.
- [ ] TUI hata/backlog kaydı: kullanıcı raporlarını test senaryosuna ve çözüm kanıtına bağlayan hata şablonu.
- [ ] Release komutu: format, test, clippy, release build, servis health ve kritik E2E smoke'u tek raporda toplar.

Tamamlanma ölçütü: Günlük metin giriş/çıkış davranışı en az 20 senaryoda tekrar üretilebilir; yeni dilimler bu regression setini geçmeden birleşmez.

### P2.1 — Native desktop kabuğu ve gerçek görsel ekler

Durum: BEKLENİYOR — İlk ürün önceliği

Amaç: Terminal MVP'yi terk etmeden, fotoğraf ve dosya eklemeye uygun gerçek masaüstü deneyimini kurmak.

- [ ] Rust-native UI spike: `egui/eframe` ile ayrı pencere, sohbet listesi, salt-okunur mesaj kartları, resize ve bildirim odak davranışı için küçük prototip.
- [ ] Attachment contract: `AttachmentRef` (ID, canonical local path, MIME, byte size, SHA-256, oluşturulma zamanı, provenance, sensitivity) ve task/audit ilişkisi.
- [ ] Güvenli yerel dosya seçimi: `Ctrl+O`/ataç düğmesi; yalnız allowlist MIME, canonical path, boyut/piksel limiti, hash ve kullanıcı görünür önizlemesi. Dosya yolu veya EXIF içeriği model talimatı olmaz.
- [ ] Vision adapter: mevcut metin modelinden ayrı, CPU uyumlu multimodal GGUF + eşleşen `mmproj` ile loopback-only local endpoint. **Bu noktada model indirmeden önce seçenekler, disk/RAM maliyeti ve lisans kullanıcıya sunulacak.**
- [ ] Görsel cevap policy: model yalnız görüntü açıklaması/analizi üretir; görselden gelen hiçbir metin tool yetkisi kazanmaz. Hassas veya desteklenmeyen görsel için açık hata ve yerel silme kontrolü sağlanır.
- [ ] E2E: JPEG/PNG başarı, bozuk dosya/MIME reddi, boyut limiti, provenance, model kapalıyken hata, görsel prompt-injection izolasyonu ve TUI fallback testleri.

Tamamlanma ölçütü: Kullanıcı masaüstü penceresinden tek bir fotoğraf seçip ne gördüğünü sorabilir; ek hem UI'da görünür hem de core policy/audit zincirinde güvenli data olarak kalır.

### P2.2 — Kullanıcı profili, kontrollü bellek ve gerçek RAG

Durum: BEKLENİYOR

Amaç: JARVIS'in kişisel bilgiyi hard-code etmeden, izinli ve açıklanabilir biçimde hatırlaması; belgelerden kaynaklı cevap vermesi.

- [ ] Ayrı profile store: ad, tercih ve rol gibi bilgiler için açık kullanıcı düzenleme/silme ekranı; sohbetten otomatik kalıcı yazma yok.
- [ ] Memory türleri: session, user-profile, project ve task memory fiziksel olarak ayrılır; her kayıtta provenance, sensitivity, TTL ve silme durumu olur.
- [ ] Workspace ingestion: çoklu belge indeksleme, SQLite metadata/FTS ile retrieval, dosya hash'i ve değişiklik algılama.
- [ ] Secret/hassas dosya filtresi: `.env`, anahtarlar, credential ve kullanıcı tanımlı path'ler indeks dışı; sonuçlarda kaynak ve alıntı sınırı gösterilir.
- [ ] Context budgeter: en alakalı, provenance'ı korunan küçük parçaları modele verir; retrieval içeriği data envelope dışına çıkmaz.
- [ ] E2E: belgeden doğru cevap/kaynak, injection reddi, secret exclusion, bellek silme ve konu değişiminde eski profilin yanlış kullanılmaması.

Tamamlanma ölçütü: Kullanıcı bir klasörü izinle indeksleyip kaynak gösteren cevap alabilir ve saklanan tüm kişisel veriyi görüntüleyip silebilir.

### P2.3 — Güvenli coding workbench

Durum: BEKLENİYOR

Amaç: JARVIS'in kod tabanını anlaması, değişiklik önermesi ve yalnız onayla izole ortamda doğrulaması.

- [ ] Read-only plan/diff akışı: görev planı, etkilenen dosyalar, patch preview ve test planı.
- [ ] Isolated worker: workspace snapshot, allowlist komutları, CPU/RAM/süre limiti, ağ kapalı çalışma ve iptal/cleanup handle'ları.
- [ ] Patch uygulama onayı: dosya bazlı scope, diff hash, explicit approval, rollback/snapshot ve verifier evidence.
- [ ] Coding evaluation seti: küçük hatalar, test ekleme, yanlış patch reddi ve existing-test regression senaryoları.

Tamamlanma ölçütü: JARVIS bir değişikliği önce gösterir, kullanıcı onayı olmadan yazmaz; onay sonrası yalnız scope içindeki patch'i uygular ve test kanıtını döndürür.

### P2.4 — Sesli etkileşim (push-to-talk ile başlar)

Durum: BEKLENİYOR

Amaç: Her zaman dinleyen bir sistem yerine açık, mahremiyeti koruyan push-to-talk ses akışı.

- [ ] Yerel STT seçimi ve indirme kararı: model boyutu/kaynak tüketimi kullanıcıya sunulur; kayıt varsayılan olarak kalıcı tutulmaz.
- [ ] Push-to-talk, ses seviyesi göstergesi, transkript doğrulama/düzenleme ve normal `InputType::Voice` pipeline'ı.
- [ ] Yerel TTS seçeneği, ses seçimi ve açık kapatma anahtarı.
- [ ] E2E: mikrofon izin reddi, model yok, sessizlik, Türkçe transkript, sesli tool approval ve kayıt silme testleri.

Tamamlanma ölçütü: Kullanıcı bir tuşa basıp konuşur, gönderilecek transkripti görür/onaylar ve yanıtı isterse sesli duyar.

### P2.5 — Model kalite programı ve yalnız kanıt sonrası adaptasyon

Durum: BEKLENİYOR

Amaç: Sohbeti hard-code etmek yerine ölçmek; gerekiyorsa küçük, geri alınabilir bir model adaptasyonu yapmak.

- [ ] Sürümlü benchmark: Türkçe diyalog, takip sorusu, güvenlik sınırı, RAG doğruluğu ve coding görevleri için golden set + latency/quality raporu.
- [ ] Dataset export/versioning: yalnız human-reviewed, verifier-passed, sensitivity etiketli örnekler; silme/poisoned-example marker'ları ve dataset manifest hash'i.
- [ ] Model karşılaştırması: mevcut Qwen3 baseline ile aday modellerin CPU/RAM gecikmesi ve kalite ölçümü.
- [ ] LoRA/QLoRA fizibilite kararı: VRAM/RAM, eğitim süresi, lisans, eval hedefi ve rollback artifact'i kullanıcıya sunulmadan eğitim başlamaz.
- [ ] Old-vs-new regresyonu ve tek komutla model/adaptor rollback.

Tamamlanma ölçütü: Her model veya adapter değişikliği, sürümlü eval'de hedef metriği iyileştirir ve güvenlik/latency regresyonu üretmez; aksi halde kullanılmaz.

### P2.6 — Yetkili security/pentest hazırlığı

Durum: BEKLENİYOR — P2.3 izolasyonundan önce execution açılmaz

Amaç: “sızma testi yapabilen” değil, yalnız yazılı yetki ve teknik sınırlar altında güvenli değerlendirme yapabilen bir capability oluşturmak.

- [ ] İmzalı authorization/scope manifest, hedef canonicalization, CIDR semantiği, DNS pinning/rebinding savunması ve expiry/revoke.
- [ ] Network-scoped sandbox worker: yalnız allowlist egress, rate/runtime limiti, kill switch, dry-run ve gerçek cancellation/cleanup.
- [ ] Önce SAFE/read-only envanter ve raporlama; ACTIVE/INTRUSIVE/DESTRUCTIVE modları varsayılan olarak kapalı kalır.
- [ ] Evidence tabanlı finding formatı, insan onayı, audit export ve scope dışı/secret hedef deny testleri.

Tamamlanma ölçütü: Scope dışı hiçbir hedefe trafik çıkamaz; SAFE modda üretilen her bulgu kanıt ve audit ile ilişkilidir. Bu gate geçmeden aktif test capability'si eklenmez.

### P2.7 — Operasyonel olgunluk ve isteğe bağlı remote

Durum: BEKLENİYOR

- [ ] Metrikler: latency, model yükleme, token üretimi, başarı/verification oranı, iptal ve kaynak kullanımı.
- [ ] Backup/retention komutları, config/model/dataset rollback ve audit export/witness stratejisi.
- [ ] Remote/mobile yalnız explicit device pairing, public key, nonce/replay koruması, revoke ve server-side kill switch'ten sonra ele alınır.

Tamamlanma ölçütü: Yerel desktop sürümü güvenilir olmadan hiçbir remote device yetki veya kişisel bellek erişimi almaz.

### Önerilen uygulama sırası

1. **P2.0 stabilizasyonu** — önce eldeki kullanım hatalarını test edilebilir hale getiririz.
2. **P2.1 native desktop + vision** — fotoğraf/dosya ihtiyacını ve terminal sınırlarını doğrudan çözer.
3. **P2.2 memory + RAG** — kişiselleşme ve dokümanlarla gerçek çalışma bu katmandan gelir.
4. **P2.3 coding workbench** — sadece izolasyon ve onay temeli üstünde.
5. **P2.4 voice** — arayüz temelinin üzerine eklenir.
6. **P2.5 quality/LoRA** — hangi eğitimin gerçekten gerekli olduğunu benchmark gösterdikten sonra.
7. **P2.6 security** ve **P2.7 remote** — en son, çünkü hata maliyetleri daha yüksektir.

İlk somut Phase 2 işi önerisi: P2.0'ın küçük regresyon paketiyle birlikte P2.1'in native desktop/attachment contract spike'ı. Vision modeli indirme noktasına geldiğimizde burada durup kullanıcıdan açık onay alınır.

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
- Vision attachment dilimi MVP dışındadır; ayrıntılı uygulama sırası ve güvenlik gate'i **P2.1 — Native desktop kabuğu ve gerçek görsel ekler** altında planlandı.
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
  - Kanıt: SQLite startup recovery testleri; RECOVERING worker semantics Phase 2’dedir.
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
  - Sınır: External MCP server registry/discovery ve untrusted response provenance Phase 2’dedir.
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
eğitim/fine-tuning, advanced pentest ve mobile/remote Phase 2+ kapsamındadır.

Son doğrulama komutları:

```text
cargo fmt
cargo test
printf 'system health\ndosya oku: Cargo.toml\nnot oluştur\nunknown\nexit\n' | cargo run --quiet
```

Sonuç: 51 test geçti; tool intentleri deterministic/policy-gated, serbest doğal sohbet ise local modelin bounded session-history taşıyan, native user/assistant rollü data-only conversation path’inden yürür. Qwen3-8B CPU-only çalışır; sohbet çıktısı tool veya policy authority kazanmaz ve reasoning kapalıdır. SQLite migration v3, restart recovery, overwrite-safe snapshot, SHA-256 audit-chain, correlation log, zero-trust workspace content, teacher privacy gate, MCP stdio transportu, coding/docs ve HUD/voice basics kanıtlandı. İlk platform Linux-first desktop terminal UI’dır.

### MVP sonrası rota — Phase 2’ye kontrollü geçiş

1. **RAG’i gerçek retrieval’a taşı:** ContentRef provenance, hassas dosya exclusion, SQLite metadata index ve context budget.
2. **Coding agent güvenliği:** isolated worker içinde test/check/diff üretme; patch preview + explicit approval + verifier.
3. **Training governance:** dataset export/versioning, review kuyruğu, benchmark harness; ardından küçük LoRA/QLoRA deneyi ve rollback.
4. **Pentest readiness:** imzalı scope manifest, CIDR/DNS/egress enforcement ve network-scoped worker; ancak sonra SAFE capability’ler.
5. **Operasyonel sertleştirme:** gerçek timeout/cancel worker’ı, backup command/retention, metric dashboard ve audit witness/export.

Phase 2’ye geçmeden önce MVP’nin güvenlik/kalite gate’i yeniden çalıştırılır; fine-tuning ve advanced pentest aynı anda başlatılmaz.
