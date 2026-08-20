# JARVIS Data Plan — Web / Mobil / Desktop / Ofansif Pentest / Uzun Vadeli Vizyon

Tarih: 20 Ağustos 2026
Durum: Planlama ve veri yönetişimi aşaması — **F10 uzun vadeli araştırma vizyonu**

> Bu belge, projenin kök dizininde `data_planı.txt` olarak duran bir araştırma
> notundan buraya taşındı (20 Ağustos 2026) — kullanıcı tüm içeriğin plana eklenmesini
> istedi. İçerik 44 bölüm ve F7'nin (yetkili pentest/bug bounty) çok ötesinde bir kapsam
> taşıyor — malware analizi, tersine mühendislik, tehdit istihbaratı, kurumsal güvenlik,
> tam red-team, VE ayrıca tamamen bağımsız olarak çok-dilli coding agent, veritabanı
> mühendisliği, DevOps/SRE ve kişisel günlük asistan vizyonu.
>
> [DEVELOPMENT_PLAN.md](../DEVELOPMENT_PLAN.md)'deki yerleşimi: Bölüm 1-8 ve 13'ün
> (web/mobil/desktop/offensive pentest + OSINT veri planı) F7'yle doğrudan ilgili somut
> parçaları F7.7'ye çekildi. Geri kalan 30+ bölüm (malware, RE, threat intel, enterprise
> security, red-team, ve tüm genel yazılım mühendisliği vizyonu) **F10 — Kontrollü
> araştırma ve uzun vadeli evrim** kapsamına yerleştirildi: planın kendi tanımıyla "ürünün
> zorunlu teslim kriteri değil, yalnız stabil sürümden sonra kontrollü deneyler için."
> İçerik değiştirilmedi, yalnız başlık biçimlendirildi.

Bu dosya JARVIS’in güvenlik yetenekleri için kaliteli, izinli, kaynaklı ve test edilebilir
veri toplama planıdır. Amaç modele rastgele internet metni yığmak değil; RAG, skill,
benchmark, regression ve ileride kontrollü model adaptasyonu için güvenilir örnekler üretmektir.

KAPSAM VE TEMEL İLKELER
-----------------------

- Veri yalnızca açık lisanslı, kullanıcıya ait veya kasıtlı olarak zafiyetli laboratuvarlardan gelir.
- Gerçek sistemlerde test yalnızca yazılı yetki ve açık scope ile yapılır.
- Gerçek credential, cookie, token, kişisel veri ve müşteri verisi dataset’e girmez.
- Ham saldırı çıktısı ile doğrulanmış güvenlik bulgusu ayrı tutulur.
- Modelin tahmini “finding” değildir; verifier kanıtı olmadan confirmed etiketi verilmez.
- Her kayıt kaynak, zaman, lisans, scope, model/prompt sürümü ve doğrulama kanıtı taşır.
- İlk kullanım RAG ve eval içindir; fine-tuning ancak veri yönetişimi tamamlandıktan sonra değerlendirilir.
- Aktif test verisi yalnız izole lablarda veya yazılı izinli hedeflerde üretilir.
- Her veri kümesinin silme, düzeltme, yeniden üretme ve rollback yolu bulunur.

VERİ KULLANIM KATMANLARI
------------------------

1. Knowledge/RAG
   Standartlar, metodoloji, remediation ve güvenlik kavramları.

2. Skill/playbook
   Bir test sınıfının ne zaman seçileceğini, hangi önkoşullara sahip olduğunu ve
   hangi yetkilerle çalışabileceğini anlatan insan incelemeli prosedür.

3. Eval/benchmark
   Modelin doğru plan, doğru tool seçimi, güvenli red ve doğru rapor üretip üretmediğini ölçer.

4. Regression
   Daha önce düzeltilmiş JARVIS hatalarının tekrar oluşup oluşmadığını kontrol eder.

5. Model adaptation
   Yalnız human-reviewed, verifier-passed, lisans/provenance bilgisi eksiksiz örnekler.

VERİ KAYIT ŞEMASI
-----------------

Her örnek mümkün olduğunca aşağıdaki alanları taşıyacak:

- record_id: değişmez UUID.
- domain: web | mobile | desktop | offensive_pentest.
- subdomain: api | auth | storage | browser | android | ios | linux | windows | macos vb.
- task_type: recon, enumeration, validation, remediation, report, refusal, tool_selection vb.
- source_type: standard, advisory, lab, own_run, human_review, synthetic.
- source_uri ve source_commit/hash.
- license ve commercial_use durumu.
- collected_at ve reviewed_at.
- authorization/scope_ref; gerçek hedefte kullanıldıysa yazılı yetki kaydı.
- target_kind: lab, local_fixture, owned_asset, public_document.
- input: kullanıcı isteği veya güvenli görev tanımı.
- context: yalnız gerekli, redakte edilmiş bağlam.
- expected_behavior.
- model_proposal: modelin planı/intent’i.
- allowed_tools ve denied_tools.
- observed_actions.
- evidence_refs: log, request/response, screenshot, trace, test result.
- verifier_result: pass, fail, inconclusive.
- finding_status: observation, candidate, confirmed, rejected.
- severity, confidence, CWE/CVE/OWASP mapping.
- remediation.
- data_sensitivity ve retention.
- human_reviewer ve review_notes.
- deletion_marker ve dataset_version.

ORTAK KALİTE ETİKETLERİ
-----------------------

- authoritative: resmi standart veya üretici advisory.
- reproducible: aynı lab koşumunda tekrar üretildi.
- human_reviewed: insan güvenlik incelemesinden geçti.
- verifier_passed: teknik doğrulama kanıtı var.
- context_complete: scope ve önkoşullar açık.
- safe_to_train: lisans, gizlilik ve kalite kapıları geçti.
- rag_only: eğitim için değil, kaynaklı retrieval için kullanılabilir.
- eval_only: model ağırlıklarına girmeden benchmark’ta kullanılacak.
- rejected: yanlış, eksik, lisanssız veya doğrulanmamış.

================================================================
1. WEB GÜVENLİK VERİSİ
================================================================

1.1 Web bilgi kaynakları
------------------------

- OWASP Top 10.
- OWASP API Security Top 10.
- OWASP ASVS.
- OWASP Cheat Sheet Series.
- OWASP WSTG.
- CWE ve CAPEC.
- MITRE ATT&CK web/application ile ilişkili teknikler.
- RFC’ler: HTTP, TLS, OAuth, JWT, cookies, CORS, DNS, URI.
- Üretici security advisories.
- NVD, OSV ve CISA KEV.
- Framework security dokümantasyonu: Django, Rails, Spring, Express, FastAPI,
  Laravel, Next.js, ASP.NET, GraphQL, Kubernetes ingress ve API gateway’ler.

Bu katmanda exploit payload koleksiyonu yerine; zafiyetin kök nedeni, önkoşulu,
güvenli doğrulama yöntemi, etkisi ve remediation bilgisi önceliklidir.

1.2 Web güvenlik konu matrisi
-----------------------------

Kimlik ve oturum
- Authentication bypass.
- Session fixation ve session lifecycle.
- JWT doğrulama ve key/config hataları.
- OAuth/OIDC redirect, state, nonce ve scope hataları.
- MFA ve recovery akışları.
- Cookie flags, SameSite ve secure transport.

Yetkilendirme ve iş mantığı
- IDOR/BOLA.
- Broken Function Level Authorization.
- Tenant isolation.
- Privilege escalation.
- Mass assignment/property-level authorization.
- Workflow bypass.
- Race condition ve double-spend sınıfı laboratuvarlar.

Input ve server-side zafiyetleri
- SQL/NoSQL/LDAP/OS command injection.
- SSTI.
- SSRF ve egress policy.
- XXE.
- Unsafe deserialization.
- Path traversal ve file inclusion.
- Upload validation ve parser/decompression riskleri.
- Prototype pollution.

Client-side ve browser
- Reflected/stored/DOM XSS.
- CSRF.
- Clickjacking ve frame policy.
- CORS/COOP/COEP/CSP hataları.
- Open redirect.
- PostMessage origin doğrulaması.
- WebSocket authorization.

API ve veri katmanı
- OpenAPI/Swagger contract mismatch.
- GraphQL introspection/authorization/depth.
- Rate limit ve resource exhaustion.
- Pagination/filter/sort authorization.
- Webhook signature ve replay.
- API version/inventory drift.
- Sensitive data exposure.

Cloud ve deployment
- Object storage exposure.
- Misconfigured reverse proxy/CDN/WAF.
- Secret in build artifact/source map.
- Container/Kubernetes ingress mistakes.
- Debug endpoints, health endpoints ve admin panels.
- Dependency/CVE ve supply-chain riskleri.

1.3 Web lab kaynakları
----------------------

- OWASP Juice Shop: modern web, REST API ve OWASP kategorileri.
- OWASP WebGoat: öğretici görev, açıklama, exploit ve mitigation döngüsü.
- OWASP crAPI: API authorization ve business logic testleri.
- DVWA, Mutillidae ve Web Security Academy benzeri yalnız lisans/şartları kontrol edilmiş lablar.
- Kendi minimal fixture’larımız: her zafiyet için güvenli ve deterministik test uygulaması.
- JARVIS test harness: olumlu, olumsuz, edge-case ve güvenli-red senaryoları.

1.4 Web veri üretim senaryoları
------------------------------

Her senaryo en az şu varyantlara sahip olacak:

- anonymous / authenticated / low-privilege / admin.
- browser / API / mobile-like client.
- JSON / form / multipart / GraphQL.
- normal / malformed / boundary-size input.
- success / denied / timeout / rate-limited response.
- vulnerability present / absent / ambiguous.

Her senaryoda modelden beklenen:

- scope’u tekrar etmesi.
- düşük riskli keşfi planlaması.
- yüksek riskli adımda durması.
- kanıt olmadan kesin bulgu yazmaması.
- zafiyet yoksa “bulunmadı” demesi.
- remediation ve tekrar test adımı vermesi.

================================================================
2. MOBİL GÜVENLİK VERİSİ
================================================================

2.1 Mobil bilgi kaynakları
--------------------------

- OWASP Mobile Application Security Verification Standard (MASVS).
- OWASP Mobile Security Testing Guide (MSTG).
- OWASP Mobile Top 10.
- Android Developers security documentation.
- Apple Platform Security ve developer security documentation.
- NIST mobile/application security yayınları.
- CWE/CAPEC ve üretici advisory’leri.
- CVE/NVD/OSV kayıtları ve resmi framework release notları.

2.2 Android veri alanları
-------------------------

- Manifest permission ve exported component analizi.
- Activity/service/receiver/provider exposure.
- Intent validation ve deep-link handling.
- Network Security Config ve cleartext traffic.
- TLS/certificate pinning davranışı.
- Keystore/key handling ve secret storage.
- WebView bridge, JavaScript ve navigation policy.
- Local storage, SQLite, preferences, logs ve backup.
- IPC, Binder ve content provider authorization.
- File provider/path traversal.
- Root/debuggable/test build flags.
- Release signing ve update channel.
- Third-party SDK ve dependency riskleri.
- API auth, token lifecycle ve device binding.

2.3 iOS veri alanları
---------------------

- Entitlements ve URL schemes.
- ATS/network policy.
- Keychain ve secure storage.
- App Transport/ATS istisnaları.
- WebView/WKWebView bridge.
- Pasteboard, screenshots ve background data.
- Universal links/deep links.
- Jailbreak/debugger/resign riskleri.
- IPC, extensions ve app groups.
- Certificate validation ve pinning.
- Privacy manifests, permission prompts ve data minimization.
- Third-party SDK, symbol ve secret exposure.

2.4 Mobil lab kaynakları
------------------------

- OWASP MASVS/MSTG referans uygulamaları.
- Android/iOS kasıtlı zafiyetli eğitim uygulamaları.
- Kendi Android ve iOS fixture uygulamalarımız.
- Emulator/simulator üzerinde network intercept ve test harness.
- Offline APK/IPA metadata fixture’ları.
- Güvenli demo backend’leri ve mock API’ler.

Mobil veri toplama kuralları:

- Gerçek kullanıcı cihazı veya kişisel uygulama verisi kullanılmayacak.
- APK/IPA yalnız lisansı ve kaynağı belli örneklerden alınacak.
- Sertifika, token, keystore ve provisioning secret’ları dataset’e girmeyecek.
- Dynamic instrumentation sonuçları yalnız lab cihazında ve redakte edilmiş şekilde saklanacak.
- Her test cihazı/emulator sürümü kaydedilecek.

================================================================
3. DESKTOP UYGULAMA GÜVENLİĞİ VERİSİ
================================================================

3.1 Platform kapsamı
--------------------

- Linux: ELF, systemd user service, Wayland/X11, DBus, polkit, desktop portals.
- Windows: PE, services, registry, ACL, UAC, named pipes, COM, PowerShell boundaries.
- macOS: Mach-O, launchd, entitlements, sandbox, Keychain, XPC, notarization.
- Cross-platform: Electron, Tauri, Qt, .NET, Java, Python desktop ve auto-updater.

3.2 Desktop veri alanları
-------------------------

- Installer/update güvenliği.
- Binary signature ve integrity.
- Local privilege boundary.
- IPC authentication/authorization.
- File/path/symlink/TOCTOU.
- Secret/config/cache/log storage.
- Auto-start ve persistence behavior.
- Plugin/extension loading.
- Browser/desktop bridge.
- Clipboard, screenshot, notification ve global shortcut erişimi.
- Network proxy/TLS/certificate validation.
- Crash report ve telemetry privacy.
- Sandboxing ve capability isolation.
- Uninstall/cleanup/rollback.

3.3 Desktop lab kaynakları
--------------------------

- Kendi JARVIS test binary’leri.
- Açık kaynak, izinli ve güvenli örnek uygulamalar.
- Electron/Tauri/Qt/GTK minimal fixture’ları.
- Linux user-service ve DBus mock laboratuvarı.
- Windows/macOS test VM/snapshot’ları.
- Intentionally vulnerable desktop training apps; lisans ve kullanım şartı kontrolü.

Her desktop örneği için:

- OS ve sürüm.
- Architecture.
- Build type ve signing durumu.
- Install/update adımları.
- Required privileges.
- IPC endpoints.
- Network destinations.
- Sensitive local paths.
- Reproducible test command.
- Cleanup ve snapshot rollback adımı.

================================================================
4. OFANSİF PENTEST VERİSİ
================================================================

Bu bölüm “izinsiz saldırı verisi” toplama planı değildir. Yalnızca yetkili lab, kendi
varlıklarımız ve açıkça izinli değerlendirmelerden güvenli kanıt üretme planıdır.

4.1 Pentest yaşam döngüsü veri sınıfları
----------------------------------------

Scope
- Hedef, kapsam dışı hedef, port/protokol, zaman aralığı, yetki sahibi.
- Allowed methods ve forbidden methods.
- Credential scope ve kullanıcı rolleri.
- Stop condition ve emergency contact.

Recon
- Asset, domain, subdomain, service, version ve source provenance.
- Passive/active ayrımı.
- Observation timestamp ve confidence.

Enumeration
- Endpoint, parameter, role, API operation, technology ve auth state.
- Coverage tuple ve test durumu.

Validation
- Candidate finding.
- Reproduction request/response.
- Preconditions.
- Expected vs actual behavior.
- Negative/control test.

Evidence
- Redakte raw request/response.
- Hash, screenshot, trace, log ve verifier result.
- No raw credential or unnecessary personal data.

Reporting
- Title, impact, severity, confidence, CWE/CVSS.
- PoC summary, remediation, affected asset and retest status.

Learning
- Başarılı workflow.
- Failed assumption.
- Coverage gap.
- Tool configuration lesson.
- Human correction.

4.2 Offensive veri üretim modları
----------------------------------

SAFE_READ_ONLY
- Passive discovery, metadata, code review, dependency analysis.
- Varsayılan mod.

SUPERVISED_ACTIVE
- Kullanıcı planı onaylar.
- Düşük etkili aktif kontrol yapılır.
- Her riskli adımda tekrar approval.

BOUNDED_AUTONOMY
- İmzalı scope, zaman, hız, network ve resource kotası önceden tanımlıdır.
- Scope dışı veya yüksek riskli adımda otomatik durur.

DESTRUCTIVE
- JARVIS MVP ve ilk F7 kapsamı dışında.
- Ayrı threat model, özel laboratuvar ve açık insan onayı olmadan açılmaz.

4.3 Veri kaynakları
-------------------

- OWASP güvenlik standartları ve kasıtlı zafiyetli eğitim uygulamaları.
- NVD, OSV, CISA KEV ve resmi vendor advisory’leri.
- Kendi test uygulamalarımız ve sandbox worker çıktıları.
- Yazılı izinli staging/test ortamları.
- Human-reviewed JARVIS pentest oturumları.
- Yetkili CI security scan sonuçları.
- Synthetic mutation ve negative-case üretimi.

Gerçek internet hedeflerinden alınan veri dataset’e ancak:

- Yetki belgesi mevcutsa.
- Kapsam ve retention açıkça tanımlıysa.
- Kişisel/gizli bilgiler redakte edilmişse.
- Lisans/kullanım şartı uygunsa.
- İnsan incelemesi ve silme mekanizması varsa.

================================================================
5. VERİ TOPLAMA VE İŞLEME PIPELINE’I
================================================================

Adım 1 — Source registration
----------------------------

Kaynağın URI, lisansı, sahibi, kullanım amacı, toplama tarihi, güncelleme sıklığı ve
commercial-use durumu kaydedilir.

Adım 2 — Acquisition
--------------------

Veri yalnız izinli kanaldan alınır. Download hash’i, archive version’ı ve acquisition
log’u tutulur. Ağ erişimi olmayan ortamda kullanıcı onayı olmadan indirme yapılmaz.

Adım 3 — Normalization
----------------------

HTML/PDF/Markdown/JSON/HTTP trace/Android metadata farklı normalizer’lardan geçer.
Encoding, duplicate, malformed record ve binary payload ayrıştırılır.

Adım 4 — Secret/privacy scrub
-----------------------------

Bearer, cookie, JWT, password, API key, private key, email, phone, IP ve kişisel içerik
redakte edilir. Redaction işlemi kayda hangi alanların değiştiğini yazar.

Adım 5 — Provenance binding
---------------------------

Her chunk veya örnek source_id, document hash, section, line/time range ve version taşır.

Adım 6 — Human review
---------------------

Reviewer doğruluk, lisans, bağlam, güvenli kullanım ve remediation kontrolü yapar.

Adım 7 — Lab verification
-------------------------

Mümkünse örnek izole fixture’da yeniden üretilir. PoC yalnız test ortamına yönelir.

Adım 8 — Dataset split
---------------------

- train/adaptation: yalnız izinli ve reviewed örnek.
- validation: model/prompt seçimi.
- test: kilitli, eğitim sürecine görünmeyen benchmark.
- regression: geçmiş bug’lar ve güvenlik sınırları.

Adım 9 — Version/release
-----------------------

Manifest hash’i, örnek sayısı, lisans özeti, redaction sürümü ve kabul/reject oranı yayınlanır.

================================================================
6. MODEL EĞİTİMİNDEN ÖNCE RAG VE EVAL
================================================================

İlk hedef model ağırlıklarını değiştirmek değil:

- Kaynaklı security RAG.
- Skill/playbook retrieval.
- Tool seçimi benchmark’ı.
- Güvenli-red benchmark’ı.
- Finding confirmation benchmark’ı.
- Türkçe/İngilizce security terminology seti.
- Latency ve resource ölçümü.

Her model için aynı kilitli test seti çalıştırılacak:

- Doğru kapsamı seçme.
- Scope dışı isteği reddetme.
- Passive ve active adımı ayırma.
- Approval isteme.
- Tool sonucunu yanlış yorumlamama.
- Evidence yoksa finding üretmeme.
- Doğru remediation verme.
- Türkçe/İngilizce cevap kalitesi.
- Context değişiminde eski hedefi taşımama.

Fine-tuning/LoRA kararı ancak:

- En az iki bağımsız review.
- Lisans manifesti.
- Poisoning kontrolü.
- Train/validation/test ayrımı.
- Old-vs-new regression.
- Rollback artifact’i.
- Kullanıcı verisi deletion marker’ı.

tamamlandıktan sonra alınabilir.

================================================================
7. KALİTE VE REDDETME KRİTERLERİ
================================================================

Aşağıdaki kayıtlar dataset’e alınmaz:

- Kaynağı belirsiz veya lisansı belirsiz içerik.
- Yetkisiz gerçek hedef çıktısı.
- Gerçek credential/token/cookie/private key.
- Sadece model iddiası olup kanıtı olmayan finding.
- Bağlamı ve scope’u olmayan exploit örneği.
- Kopya/duplicate veya çelişkili çözülmemiş kayıt.
- Güncelliği bilinmeyen advisory.
- Zararlı veya çalıştırılabilir payload’ın gereksiz ham hali.
- İnsan incelemesi olmadan sentetik örnek.
- Redaction sonrası anlamı bozulmuş kayıt.

Kalite metrikleri:

- Provenance completeness.
- License completeness.
- Human-review pass rate.
- Verifier reproducibility rate.
- False-positive rate.
- Scope violation rate.
- Safe-refusal accuracy.
- Evidence completeness.
- Duplicate rate.
- Staleness/last-reviewed age.
- Model success and latency by task class.

================================================================
8. İLK UYGULAMA ROTASI
================================================================

1. Resmi OWASP/NIST/MITRE/CISA kaynaklarının manifestini oluştur.
2. OWASP Juice Shop, WebGoat ve crAPI için izin/lisans kaydını yap.
3. Kendi minimal web/API fixture’larımızı yaz.
4. Web read-only recon ve evidence schema’sını oluştur.
5. Android/iOS ve desktop için küçük fixture uygulamalarını tanımla.
6. JARVIS’in mevcut testlerinden güvenli-red ve tool-selection eval seti çıkar.
7. İlk RAG indexini yalnız kaynaklı ve redakte içerikle kur.
8. Kilitli test seti oluşturmadan model eğitimine başlama.
9. F4 isolated worker tamamlanmadan active pentest data collection açma.
10. F7’de yalnız SAFE/read-only ve supervised active modlarla başla.

KARAR
-----

Kaliteli veri planının merkezi “çok veri” değil, “kanıtlanabilir ve yeniden üretilebilir veri”dir.
JARVIS önce resmi kaynak + kontrollü lab + kendi human-reviewed trace’leriyle gelişecek.
Model eğitimi sonradan, yalnız dataset manifesti ve regression gate hazır olduğunda ele alınacak.


======================================================================
9. OFFENSIVE CLOUD SECURITY VERİ PLANI
======================================================================

Amaç: JARVIS’in bulut ortamlarında yetkili güvenlik değerlendirmesi,
konfigürasyon denetimi, kanıt toplama ve düzeltme önerisi yapabilmesi.
Gerçek hesaplarda izinsiz tarama, kimlik bilgisi toplama, kalıcı erişim,
veri çıkarma veya yıkıcı işlem verisi toplanmayacaktır.

9.1 Öncelikli standart ve kaynak sınıfları

- CIS Benchmarks ve CIS Controls: yapılandırma kontrolü ve kanıt formatı.
- NIST SP 800-53, 800-115, 800-190 ve ilgili cloud güvenlik rehberleri.
- CSA Cloud Controls Matrix ve Cloud Security Alliance rehberleri.
- MITRE ATT&CK Enterprise/Cloud ve ilgili teknik açıklamalar.
- OWASP Cloud-Native Application Security Top 10.
- AWS, Azure ve Google Cloud resmi güvenlik dokümantasyonu.
- Kubernetes, Docker, Helm ve Terraform resmi güvenlik belgeleri.
- OPA/Rego, Kyverno, Checkov, Trivy ve benzeri açık lisanslı policy örnekleri.
- CISA, vendor advisory, CVE/NVD ve OSV kayıtları.

9.2 Veri konu matrisi

- Kimlik ve erişim: IAM, RBAC, federasyon, MFA, servis hesapları,
  kısa ömürlü token, least privilege ve privilege boundary.
- Ağ ve sınır: VPC/VNet, security group, firewall, ingress/egress,
  private endpoint, DNS, load balancer ve segmentasyon.
- Depolama ve veri: bucket/blob ACL, şifreleme, anahtar yönetimi,
  yedekler, snapshot, public exposure ve veri yaşam döngüsü.
- Compute ve container: VM metadata, instance profile, image provenance,
  container capability, rootless çalışma, seccomp/AppArmor ve escape riskleri.
- Kubernetes: API server, admission, RBAC, secrets, network policy,
  pod security, etcd, service account ve multi-tenant izolasyon.
- IaC ve CI/CD: Terraform/CloudFormation drift, secret sızıntısı,
  state dosyası, pipeline yetkileri, artifact imzası ve dependency pinning.
- Serverless ve managed servisler: event policy, function role,
  trigger doğrulaması, queue/topic erişimi ve log/trace güvenliği.
- Gözlemlenebilirlik: audit log, data event log, retention, alert,
  zaman senkronizasyonu, kanıt bütünlüğü ve olay korelasyonu.
- Tedarik zinciri: image/package provenance, SBOM, imza, SLSA ve build izolasyonu.

9.3 Toplanacak örnekler

- Güvenli ve hatalı cloud policy çiftleri; beklenen bulgu ve düzeltme.
- Yetki grafikleri: principal → role → resource → action.
- Redacted audit log ve olay zaman çizelgesi.
- IaC diff, policy evaluation sonucu ve doğrulama kanıtı.
- Kubernetes manifesti, admission sonucu ve güvenli alternatif.
- Saldırı zinciri iddiası yerine “önkoşul → gözlem → risk → düzeltme” kaydı.
- Yanlış pozitif, istisna ve compensating control örnekleri.

9.4 Laboratuvar ve güvenlik sınırları

- Yerel LocalStack/MinIO/Kubernetes/kind veya k3d tabanlı fixture’lar.
- Ayrı hesap/project, sahte kimlikler, kısa ömürlü erişim ve otomatik teardown.
- Her denemede scope manifesti, izin kaydı, zaman sınırı ve kaynak etiketi.
- Üretim sırları, gerçek müşteri verisi ve dış hedefler dataset’e giremez.
- Aktif testler önce read-only policy audit; yazma işlemleri supervised moda bağlıdır.


======================================================================
10. MALWARE ANALYSIS VERİ PLANI
======================================================================

Amaç: JARVIS’in zararlı yazılımı güvenli biçimde sınıflandırması,
özellik çıkarması, davranış kanıtını yorumlaması ve raporlamasıdır.
Bu bölüm örnek üretmeyi, yaymayı veya gerçek hedefte çalıştırmayı kapsamaz.

10.1 Kaynak ve lisans politikası

- Yalnızca açık lisanslı, eğitim/araştırma kullanımına izin veren,
  provenance’ı doğrulanmış örnekler.
- Malware sample paylaşım sitelerinde lisans, kullanım şartı ve erişim
  yetkisi ayrı kaydedilir; belirsiz kaynak dataset’e alınmaz.
- IOC’ler mümkünse hash, domain, path ve string düzeyinde redacted tutulur.
- Canlı payload, çalıştırılabilir dosya ve silahlandırılmış talimatlar
  model eğitimine ham biçimde konmaz; güvenli özet/özellik katmanı kullanılır.
- Zararlı örnekleri yalnızca şifreli, erişim kontrollü, internetsiz veya
  egress’i tamamen kapatılmış analiz ortamında saklanır.

10.2 Analiz katmanları

- Statik temel: SHA-256/ssdeep, PE/ELF/Mach-O metadata, section,
  import/export, string, resource, signature ve packer göstergeleri.
- Statik ileri: control-flow özeti, syscall/API referansı,
  config extraction, certificate, entropy ve embedded object.
- Dinamik güvenli gözlem: process tree, file/registry/config değişimi,
  network attempt, mutex, service/task, memory region ve zaman çizelgesi.
- Tespit: YARA/Sigma/Snort/Suricata kuralları, ATT&CK teknik eşleşmesi,
  IOC confidence ve false-positive notu.
- Raporlama: aile iddiası, kanıt, belirsizlik, etki, containment ve
  temizleme/iyileştirme önerisi.

10.3 Veri şeması ekleri

- sample_id, artifact_hash, file_type, architecture, first_seen, source,
  license, sandbox_profile, snapshot_id, static_features, dynamic_events,
  iocs, ATT&CK_mapping, detection_rules, analyst_verdict, confidence.
- Her davranış kaydı: timestamp, parent_process, action, target,
  observed_evidence, normalized_event, confidence.
- “Gözlenmedi” ile “örnekte yok” ayrımı zorunludur.
- Benign/dual-use araçlar ayrı etiketlenir; otomatik olarak malware kabul edilmez.

10.4 Güvenli laboratuvar

- Disposable VM snapshot, host-only ağ, sahte DNS/HTTP sinkhole,
  kontrollü fake services ve ağ çıkışı olmayan varsayılan profil.
- Analiz worker’ı JARVIS’ten ayrı yetki ve dosya sistemiyle çalışır.
- İnsan onayı olmadan örnek çalıştırma, unpacking, network release veya
  kalıcı değişiklik yapılmaz.
- Her çalıştırma başlamadan önce sample hash, izin, süre ve kill-switch doğrulanır.
- Ham örnekler RAG’e verilmez; yalnızca rapor, normalize olay ve IOC özeti verilir.

10.5 İlk eğitim/eval görevleri

- “Bu dosya nedir?” yerine kanıt temelli sınıflandırma.
- Statik özelliklerden olası davranış ve confidence çıkarma.
- Dinamik olayları ATT&CK tekniğine eşleme.
- IOC listesini önem ve güven seviyesine göre ayırma.
- Benign yazılım ile malware arasındaki belirsizliği doğru ifade etme.
- Raporun containment ve remediation kısmını güvenli biçimde üretme.


======================================================================
11. REVERSE ENGINEERING VERİ PLANI
======================================================================

Amaç: JARVIS’in yetkili yazılım incelemesi, uyumluluk, hata ayıklama,
interoperability ve güvenlik araştırması için binary davranışını açıklaması.
Korsanlık, lisans korumasını aşma, DRM kırma veya yetkisiz yazılım değişikliği
veri hedefi değildir.

11.1 Kaynak türleri

- Kendi yazdığımız ve lisans sahibi olduğumuz binary’ler.
- Açık kaynak projelerin release binary/source eşleşmeleri.
- Eğitim amaçlı crackme, CTF ve tersine mühendislik laboratuvarları.
- Üretici izinli SDK, plugin ve interoperability örnekleri.
- Reproducible build çıktıları ve sembol/map dosyaları.
- Resmi ABI/API, file format ve protocol dokümantasyonu.

11.2 İnceleme alanları

- Binary format: PE/ELF/Mach-O header, section, symbol, relocation,
  debug info, signing ve dependency metadata.
- Program akışı: function boundary, call graph, control-flow özeti,
  data-flow ilişkisi ve kritik path açıklaması.
- API/ABI: exported/imported API, syscall, library, IPC ve protocol.
- Memory ve güvenlik: stack/heap kullanım modeli, permission,
  input boundary, serialization ve güvenlik kontrolü.
- Mobil/desktop paketleri: manifest, resource, config, update ve plugin.
- Diffing: sürüm farkı, patch etkisi, regression ve davranış değişimi.
- Dinamik gözlem: debugger trace, syscall trace, file/network event;
  ham secret ve kişisel veri redaction sonrası kaydedilir.

11.3 Dataset örneği

- artifact_hash, platform, architecture, compiler/toolchain,
  source_available, symbols_available, build_flags, license.
- question: “Bu fonksiyonun amacı nedir?”
- evidence: disassembly/pseudocode/source correlation özeti.
- answer: yalnız kanıtla desteklenen açıklama.
- uncertainty: function boundary veya decompiler hatası.
- verifier: unit test, source match, dynamic trace veya human review.
- forbidden_claim: gözlenmemiş davranış, kesin exploit veya üretim etkisi.

11.4 Araç ve çıktı politikası

- Ghidra, Binary Ninja/IDA (lisans izin veriyorsa), radare2, objdump,
  readelf, nm, strings, Frida/LLDB/GDB gibi araçların yalnız izinli
  fixture ve laboratuvar çıktıları kullanılabilir.
- Araç çıktısı tek başına gerçek kabul edilmez; kaynak, disassembly ve
  dinamik kanıtla çapraz doğrulanır.
- Decompiler metni “kesin kaynak kod” olarak etiketlenmez.
- Exploit geliştirme yerine kök neden, etki ve güvenli düzeltme hedeflenir.

11.5 Reverse engineering laboratuvarı

- Aynı kaynak koddan farklı compiler/optimization/architecture build’leri.
- Bilinen fonksiyonları içeren küçük fixture binary’ler.
- Hatalı ve güvenli parser sürümlerinin karşılaştırılması.
- Sembolü kaldırılmış binary ile sembollü referans eşleştirmesi.
- C/C++/Rust/Go/.NET/Java/Swift örnekleri ve platform varyantları.
- Her fixture için beklenen function map, API map ve davranış testi.


======================================================================
12. ORTAK KALİTE, GÜVENLİK VE VERİ BİRLEŞTİRME KURALLARI
======================================================================

12.1 Ortak provenance

Her kayıtta kaynak URI, erişim tarihi, içerik hash’i, lisans,
yetki/scope referansı, araç sürümü, ortam snapshot’ı, analist ve doğrulama
kanıtı bulunur. Provenance eksikse kayıt yalnız “untrusted research” olarak
tutulur; eğitim veya üretim RAG’ine alınmaz.

12.2 Ortak veri katmanları

- L0 raw/quarantine: erişim kontrollü, şifreli, modelden uzak.
- L1 normalized: secret ve PII temizlenmiş, hash/provenance eklenmiş.
- L2 reviewed: insan tarafından incelenmiş ve güvenlik etiketi verilmiş.
- L3 eval: kilitli test ve regression senaryoları.
- L4 RAG/skill: yalnız güvenilir özet, prosedür ve remediation bilgisi.

12.3 Zorunlu kalite kapıları

- Lisans ve kullanım hakkı doğrulanmadan ingest yok.
- Secret, token, kişisel veri ve gerçek müşteri bilgisi redaction olmadan yok.
- En az bir bağımsız kanıt veya insan incelemesi olmadan “confirmed” yok.
- Train/test sızıntısı, duplicate ve aynı olayın farklı yazımları kontrol edilir.
- Prompt injection, kötü amaçlı talimat ve poisoned document ayrı etiketlenir.
- Zararlı veya aktif payload yalnız quarantine’de; model girdisi normalize özettir.
- Her silme/geri çekme işlemi manifest ve dataset version’a işlenir.

12.4 Başlangıç veri dağılımı

- %35 resmi standart ve güvenlik rehberi.
- %25 kontrollü web/mobile/desktop/cloud fixture ve lab trace’i.
- %15 malware/reverse engineering statik ve dinamik özetleri.
- %15 insan doğrulamalı rapor, remediation ve açıklama çiftleri.
- %10 adversarial, belirsiz, benign ve false-positive eval kayıtları.

Bu oranlar sabit kota değildir; kalite, lisans ve temsil dengesi bozulursa
yeniden ayarlanır.

12.5 İlk uygulama sırası

1. Cloud, malware ve reverse engineering için ayrı source registry oluştur.
2. Her alan için lisans/scope/provenance manifest şablonunu kullan.
3. Önce kendi küçük fixture ve eval setlerini üret; dış örnekleri sonra ekle.
4. Malware için güvenli sandbox ve quarantine erişimini tamamla.
5. Cloud için LocalStack/MinIO/kind tabanlı policy fixture’larını kur.
6. Reverse engineering için kaynak-binary eşleşmeli küçük program seti hazırla.
7. Her alanda benign, belirsiz, false-positive ve refusal senaryoları ekle.
8. Dataset’i önce RAG/eval olarak kullan; fine-tuning kararını metriklerden sonra ver.
9. Zehirlenme, veri sızıntısı ve yetkisiz eylem testleri geçmeden üretime alma.
10. Her yeni kaynakta manifest, insan incelemesi ve rollback noktası oluştur.

Son karar: Bu üç alan JARVIS’in yeteneklerini ciddi biçimde genişletebilir;
ancak değer, saldırı örneği sayısından değil güvenli izolasyon, kanıt,
lisans, yeniden üretilebilirlik ve doğru belirsizlik yönetiminden gelir.


======================================================================
13. OSINT VERİ PLANI
======================================================================

Amaç: JARVIS’in açık, yasal ve doğrulanabilir kaynaklardan araştırma
yapması; varlık, teknoloji, ilişki ve risk bilgisini kanıtlarıyla
birleştirmesidir. OSINT, kişileri izlemek veya rastgele interneti taramak
değil; yetkili kuruluş, proje veya araştırma sorusunu cevaplamaktır.

13.1 OSINT katmanları

- Kurumsal kimlik: resmi alan adları, ASN, sertifika, repo, duyuru ve
  güvenilir şirket kayıtları.
- Alan adı/DNS: passive DNS, certificate transparency, WHOIS/RDAP,
  DNS kayıtları, nameserver ve domain ilişki grafiği.
- Varlık keşfi: public IP/URL, cloud bucket iddiası, CDN, teknoloji
  parmak izi ve yayınlanmış servis metadata’sı.
- Kod ve tedarik zinciri: açık repo, commit, issue, package, SBOM,
  release, dependency ve secret-sızıntısı sinyalleri.
- Web içeriği: robots/sitemap, public API dokümanı, JS bundle metadata,
  değişiklik geçmişi, arşiv ve güvenlik başlıkları.
- Sertifika ve altyapı ilişkileri: SAN, issuer, hosting, ASN ve zaman
  içindeki değişim; yalnız kamuya açık kanıtla.
- Medya ve belge: rapor, PDF, görsel metadata ve yayımlanmış teknik sunum.
- İnsan/organizasyon: yalnız resmi görev, public contact ve kurumsal
  ilişki; özel kişi profilleme, hassas veri toplama ve doxxing yok.
- Olay ve zafiyet: vendor advisory, CVE, CISA KEV, CERT duyurusu ve
  resmi olay raporları.
- Zaman serisi: ilk görülme, son görülme, değişiklik, güven puanı ve
  çelişen kaynakların kaydı.

13.2 Yüzlerce katmana ölçekleme modeli

Her katman ayrı worker değil, ortak bir “source → normalize → correlate →
verify → report” sözleşmesine uyar. Katmanlar şu gruplarda çoğaltılır:

- Kaynak ailesi (DNS, repo, sertifika, arşiv, advisory, package vb.).
- Varlık türü (domain, IP, URL, org, repo, package, certificate,
  cloud resource, identity, technology).
- Zaman perspektifi (anlık, geçmiş, değişim, trend).
- Güven seviyesi (kaynak itibarı, tazelik, bağımsız doğrulama).
- İlişki tipi (owns, hosts, resolves, signs, depends-on, references).
- Çıktı tipi (lead, confirmed fact, risk, change, hypothesis, refusal).

Bu yapı yüzlerce kombinasyonu destekler; her kombinasyonun ayrı ve sınırsız
tarama yapması gerekmez. Önceliklendirme, maliyet, rate limit ve scope
kontrolüyle belirlenir.

13.3 OSINT veri şeması ekleri

- query_id, investigation_id, scope, source_uri, retrieval_time,
  content_hash, source_type, publisher, license, jurisdiction.
- subject_type, subject_id, observed_value, relation, first_seen,
  last_seen, confidence, corroboration_count, contradiction_status.
- collection_method (passive/manual/authorized), robots_or_terms_status,
  rate_limit, analyst, retention, deletion_marker.
- Evidence snippet, canonical URL ve ekran görüntüsü/metadata referansı.

13.4 OSINT güvenlik kuralları

- Varsayılan mod PASSIVE_PUBLIC_ONLY.
- Scope dışı domain/IP/hesap için sorgu yapılmaz.
- Login bypass, CAPTCHA atlatma, private group erişimi, credential use,
  mass scraping ve kişisel profil çıkarımı yasaktır.
- Kaynakların kullanım şartları, robots sinyali ve rate limitleri kaydedilir.
- Ham web içeriği prompt injection içerebilir; model talimatı olarak değil,
  untrusted evidence olarak izole edilir.


======================================================================
14. ÇOK KATMANLI THREAT INTELLIGENCE MİMARİSİ
======================================================================

Amaç: Tek bir IOC listesi değil; stratejik, operasyonel, taktik ve teknik
seviyeleri birleştiren, zamanla değişen ve kanıt temelli bir istihbarat
katmanı kurmaktır.

14.1 İstihbarat seviyeleri

- Stratejik: sektör, aktör eğilimleri, iş etkisi, jeopolitik ve yatırım
  kararlarını destekleyen uzun dönemli özetler.
- Operasyonel: kampanya, olay zinciri, hedefleme modeli, altyapı ve
  savunma önceliği.
- Taktik: ATT&CK teknikleri, detection gap, playbook ve kontrol önerisi.
- Teknik: IP/domain/hash/URL, YARA/Sigma/Suricata, packet veya log sinyali.
- Context: asset criticality, owner, exposure, business process ve risk.
- Temporal: IOC lifecycle, decay, re-use, campaign transition ve confidence.

14.2 Threat intelligence katmanları

1. Source ingestion: TAXII/STIX, CERT, vendor, advisory, OSINT ve kendi
   telemetry kaynaklarının güvenli alınması.
2. Schema normalization: STIX 2.1 benzeri ortak varlık/ilişki modeli.
3. Enrichment: DNS, ASN, certificate, malware family, CVE, ATT&CK,
   package ve cloud context.
4. Deduplication: hash, canonicalization ve ilişki tabanlı tekrar temizliği.
5. Correlation graph: entity, relation, time, confidence ve provenance.
6. Scoring: kaynak itibarı, freshness, independent corroboration,
   asset relevance ve false-positive geçmişi.
7. Detection translation: SIEM query, Sigma/YARA/Snort önerisi,
   cloud policy ve endpoint control.
8. Validation: lab replay, benign karşılaştırma, human review ve rollback.
9. Dissemination: role-based brief, alert, API, RAG ve case timeline.
10. Feedback: analist kararı, incident sonucu, detection precision ve
    intelligence gap geri bildirimi.

14.3 Tehdit aktörü ve kampanya verisi

- Aktör adı bir gerçek değil, kaynaklar arası normalize edilmiş iddiadır.
- Attribution kesinliği: unknown, possible, probable, high-confidence.
- TTP, altyapı, hedef sektör ve zaman penceresi ayrı kanıtlanır.
- Aktif kişi kimliği, özel veri ve spekülatif doxxing tutulmaz.
- Çelişen iddialar silinmez; contradiction graph içinde görünür kalır.

14.4 Threat intel kalite metrikleri

- IOC precision/recall ve decay sonrası geçerlilik.
- Detection true/false positive, analyst acceptance ve remediation rate.
- Source freshness, corroboration, provenance completeness.
- Time-to-enrich, time-to-triage, time-to-detect ve time-to-report.
- Modelin belirsizliği doğru belirtme ve unsupported attribution oranı.


======================================================================
15. INFRASTRUCTURE VE PLATFORM SECURITY VERİ PLANI
======================================================================

Kapsam: Linux, Windows, macOS, container, Kubernetes, cloud, storage,
CI/CD, identity, logging ve backup altyapılarının güvenli işletimi.

- Asset inventory, ownership, criticality, dependency ve lifecycle.
- OS hardening, patch, package, service, kernel, driver ve boot güvenliği.
- Network segmentation, firewall, bastion, VPN, DNS ve time sync.
- Secrets, certificate, key rotation, vault, backup ve disaster recovery.
- Container image, runtime, registry, SBOM, signature ve admission.
- Kubernetes control/data plane, multi-tenancy ve workload identity.
- IaC review, drift, policy-as-code ve deployment approval.
- Monitoring, EDR, audit, alert routing, retention ve tamper evidence.
- BCP/DR: RTO/RPO, restore test, failover ve veri bütünlüğü.
- İç servisler arası trust boundary, API gateway ve service mesh.

Her kayıt asset owner, environment (lab/stage/prod), change window ve
geri alma prosedürünü içermelidir. Üretimde aktif değişiklik varsayılan
olarak kapalıdır.


======================================================================
16. AI SECURITY VE MODEL OPERASYONLARI VERİ PLANI
======================================================================

Amaç: JARVIS’in yalnız model kullanması değil, AI sistemlerini güvenli
tasarlaması, değerlendirmesi ve işletmesidir.

- Prompt injection, indirect injection ve untrusted document ayrıştırma.
- Tool permission, approval gate, least privilege ve sandbox escape testi.
- Data poisoning, backdoor, retrieval poisoning ve provenance kontrolü.
- Model extraction, sensitive data leakage ve membership/inference riski.
- Jailbreak, unsafe completion, refusal quality ve dual-use sınırları.
- RAG grounding, citation, chunk quality, stale document ve deletion.
- Model registry, version pinning, rollback, canary ve audit trail.
- Eval: doğruluk, groundedness, tool correctness, latency, maliyet,
  Türkçe/İngilizce kalite, güvenlik ve belirsizlik kalibrasyonu.
- Human-in-the-loop, escalation, reviewer disagreement ve appeal.
- Agent trace: plan, tool call, input/output, approval, evidence ve result.

AI verisi üçe ayrılır: model bilgisi, araç/skill sözleşmesi ve test/eval.
Kullanıcı sohbeti otomatik olarak eğitim verisi yapılmaz; açık onay,
redaction, retention ve silme mekanizması gerekir.


======================================================================
17. AĞ GÜVENLİĞİ VERİ PLANI
======================================================================

- TCP/IP, DNS, HTTP(S), TLS, SSH, VPN, routing ve segmentation temelleri.
- Firewall/IDS/IPS, proxy, WAF, NAC, zero trust ve egress kontrolü.
- Packet metadata, flow, DNS log, HTTP log, authentication ve endpoint
  olaylarının zaman korelasyonu.
- Secure protocol configuration, certificate lifecycle ve key exchange.
- Wireless, Bluetooth ve IoT yalnız izinli lab ortamında.
- DDoS/abuse verileri yalnız simülasyon ve kapasite planlama bağlamında.
- Detection engineering: normal baseline, anomaly, signature ve triage.
- PCAP örnekleri anonimleştirilmiş, küçük ve etiketli tutulur.
- Aktif network testleri SAFE_READ_ONLY veya izole staging scope’u ister.

Beklenen model çıktısı saldırı komutu değil; gözlem, hipotez, doğrulama
adımı, risk, containment ve savunma kuralıdır.


======================================================================
18. ENTERPRISE SECURITY VERİ PLANI
======================================================================

- Asset/business process/owner/criticality ilişkisi.
- IAM lifecycle, joiner-mover-leaver, privileged access ve SoD.
- GRC: policy, control, risk acceptance, exception ve audit evidence.
- Vulnerability management: discovery, prioritization, remediation,
  SLA, exposure ve compensating control.
- SOC: alert triage, case management, escalation, evidence ve lessons learned.
- Incident response: prepare, identify, contain, eradicate, recover,
  post-incident ve iletişim planı.
- Third-party/vendor risk, supply chain, contract ve breach notification.
- Data classification, DLP, privacy, retention ve legal hold.
- Business continuity, tabletop exercise ve crisis decision log.
- Executive/technical rapor ayrımı; aynı kanıtın farklı hedefe sunulması.

Enterprise verisi müşteri sırrı içeriyorsa varsayılan olarak yalnız şema,
sentetik örnek ve redacted özet tutulur. Gerçek vaka verisi açık yazılı
izin ve erişim kontrolü olmadan JARVIS’e alınmaz.


======================================================================
19. YETKİLİ FULL RED-TEAM VERİ PLANI
======================================================================

Full red-team yalnız önceden yazılı izin, net kapsam, zaman penceresi,
acil durdurma kişisi ve geri dönüş planıyla ele alınır. Amaç “en çok hasar”
değil, savunmanın tespit ve müdahale kapasitesini ölçmektir.

19.1 Aşamalar

- Rules of engagement, scope, exclusions, success criteria ve safety plan.
- Asset discovery ve threat model.
- Passive reconnaissance ve güvenli doğrulama.
- Exposure validation: düşük etkili, rate-limited ve reversible kontroller.
- Initial access simülasyonu: yalnız lab/staging veya açıkça yetkili hedef.
- Privilege boundary ve lateral movement simülasyonu; gerçek veri erişimi yok.
- Objective validation: marker/canary ile, veri çıkarma yerine kanıt.
- Detection/response ölçümü: alarm, containment, timeline ve coverage.
- Cleanup, restore, evidence package ve kapanış raporu.

19.2 Otonomi kademeleri

- R0: plan önerisi, insan onayı zorunlu.
- R1: passive collection ve read-only checks.
- R2: staging’de supervised active validation.
- R3: bounded autonomy; her adım scope, budget, rate-limit ve kill-switch ile.
- R4: production destructive action kesinlikle MVP kapsamı dışında.

JARVIS’in “otonom” davranışı araç çağırma özgürlüğü değil; önceden tanımlı
politika içinde planlama, kanıt toplama ve gerektiğinde durma yeteneğidir.

19.3 Full red-team dataset’i

- ROE/scope manifest, plan, action, approval, tool trace, evidence,
  detection event, analyst decision, cleanup proof ve final report.
- Başarısızlık, güvenli durma, yanlış pozitif ve kullanıcı iptali de
  başarıyla tamamlanan senaryolar kadar önemlidir.
- Gerçek exploit payload yerine capability sınıfı, önkoşul ve kanıt özeti.
- Her senaryo benign kontrol ve rollback testiyle eşleştirilir.


======================================================================
20. İLERİ SEVİYE HEDEF VE KADEMELİ TESLİM MODELİ
======================================================================

Uzun vadeli hedef JARVIS’i web, mobil, desktop, cloud, malware analysis,
reverse engineering, OSINT, threat intelligence, ağ, enterprise ve yetkili
red-team alanlarında ileri seviyeye taşımaktır. Bu, bütün alanların aynı anda
uzmanlaşacağı anlamına gelmez; her alan ayrı capability, worker, policy,
provenance ve eval kapılarıyla ilerler.

İlk teslimlerde aşağıdaki temel davranışlar ölçülür:

- Temel kavramı doğru açıklama ve uygun kaynağa yönlendirme.
- Scope dışı isteği reddetme ve yetki isteme.
- Basit/orta seviye bulguyu kanıtla doğrulama.
- Belirsizliği, false positive’i ve eksik veriyi açıkça belirtme.
- Tool/worker seçme, sonucu normalize etme ve rapor üretme.
- Türkçe ve İngilizce teknik iletişim.
- Web, mobil, desktop, cloud, malware, RE, OSINT ve network arasında
  ilişkileri kurma; ancak ileri uzmanlık iddiasında bulunmama.

İleri seviye kabul kriterleri zamanla yükseltilir: her alan için kapsamlı
kilitli eval matrisi, farklı platform ve dil varyantları, adversarial test,
tool trace doğrulaması, grounded/verified cevap, scope ihlali sıfır, kritik
unsafe action seçiminde sıfır tolerans ve insanın kabul ettiği rapor formatı.

Uygulama sırası:

1. OSINT source registry ve passive collector sözleşmesi.
2. Threat intel STIX-benzeri entity/relation/time modeli.
3. Cloud/infra ve network fixture’ları.
4. AI security eval ve tool-policy testleri.
5. Enterprise incident/evidence şeması.
6. Red-team yalnız staging/lab R0–R2 senaryoları.
7. Her alanda RAG + skill + eval; fine-tuning ancak veri sızıntısı,
   lisans ve regression gate’leri geçildikten sonra.

Bu genişleme projeyi büyütür; alanları tek seferde “tam uzman” yapmak yerine
ileri seviye nihai hedefi koruyup ortak provenance, policy, evidence ve
evaluation altyapısını önce kurmak geliştirme süresini yönetilebilir tutar.


======================================================================
21. ZARARLI VE ETİK SINIRLARI AŞINDIRAN VERİLERE KARŞI KORUMA
======================================================================

Bu bölüm, kötü niyetli aktörlerin kullandığı içerik türlerini yalnızca risk
modellemek ve JARVIS’e sokmamak için tanımlar. Bu içerikleri arama, indirme,
derleme, yeniden üretme veya erişim yollarını belgeleme planımız yoktur.

21.1 Riskli veri sınıfları

- Kaynağı, lisansı veya izin durumu belirsiz sızıntı veri setleri.
- Zararlı eylemi adım adım kolaylaştıran payload, exploit veya kaçınma
  talimatları.
- Modelin güvenlik politikasını devre dışı bırakmaya odaklı jailbreak ve
  prompt-injection koleksiyonları.
- Çalıntı kimlik bilgileri, token, private key, kişisel veri ve gerçek
  müşteri kayıtları.
- Malware’in çalıştırılabilir ham örnekleri ve yayılım/kalıcılık tarifleri.
- Yetkisiz erişim, dolandırıcılık, kimlik taklidi veya fiziksel zarar
  süreçlerini optimize eden diyaloglar.
- Güvenilir görünümlü fakat talimat içeren poisoned document ve sahte
  güvenlik raporları.
- Kaynağı doğrulanmamış forum/arşiv içerikleri ve otomatik üretilmiş spam.

21.2 Bu içerikler neden risklidir?

Kötü niyetli kişiler genellikle açık web, anonim paylaşım alanları,
çalıntı/sızdırılmış arşivler, kötüye kullanılabilir açık kaynak depoları,
otomatik sentetik içerik ve güvenilir belge görünümündeki prompt injection
metinlerinden yararlanmaya çalışır. Buradaki temel risk “verinin kötü
kalitesi” değil, modelin talimat hiyerarşisini, yetki sınırını veya güvenlik
politikasını aşındırmasıdır.

Bu açıklama kaynak bulma rehberi değildir; yalnızca veri kabul sistemimizin
hangi tehditleri varsayacağını belirtir.

21.3 JARVIS veri kabul savunması

1. Provenance zorunluluğu: kaynak, sahiplik, lisans, erişim tarihi ve hash.
2. Quarantine: yeni veri modele, RAG’e veya tool katmanına doğrudan girmez.
3. Content classification: secret/PII, malware, exploit, injection,
   unsafe instruction ve benign teknik belge etiketleri.
4. Instruction/data ayrımı: belge içindeki “şunu yap” metni varsayılan
   olarak veri kabul edilir; sistem talimatı veya tool yetkisi değildir.
5. Redaction: gerçek sırlar, kimlik bilgileri, kişisel veriler ve canlı
   bağlantılar temizlenmeden normalize katmana geçiş yok.
6. Human review: yüksek riskli veya belirsiz kayıtlar iki aşamalı inceleme.
7. Dataset firewall: eğitim/RAG/eval setleri birbirinden ve raw depodan ayrı.
8. Poisoning test: çelişki, gizli talimat, tekrar, sahte kaynak ve trigger
   davranışları özel eval senaryolarıyla ölçülür.
9. Least privilege: veri işleyicinin ağ, dosya ve tool erişimi minimumdur.
10. Rollback/deletion: kaynak geri çekilince tüm türev kayıtlar bulunup silinir.

21.4 Güvenli kullanım biçimi

Riskli ham içerik gerekiyorsa yalnızca güvenlik değerlendirmesi amacıyla,
izole quarantine’de ve insan onayıyla tutulur. Model eğitimine ham biçimde
verilmez. Bunun yerine şu güvenli türevler tercih edilir:

- Redacted özet ve risk etiketi.
- “Bu içerik talimat değil, untrusted evidence” işareti.
- Davranış sınıfı ve savunma önerisi.
- Benign karşı örnek ve güvenli refusal beklenen cevap.
- Provenance ve neden reddedildiği.

21.5 Kırmızı çizgiler

- Gerçek kişilerin veya kurumların sırlarını toplamamak.
- Yetkisiz hedef, hesap, ağ veya cihaz üzerinde veri toplamamak.
- Ham silahlandırılmış içerikleri çoğaltmamak.
- Modeli etik sınırları aşmaya alıştıracak “başarılı jailbreak” veri seti
  üretmemek; bunun yerine güvenli refusal ve injection-detection eval’i
  hazırlamak.
- “Araştırma” etiketiyle canlı saldırı, veri çıkarma veya kalıcılık
  talimatlarını meşrulaştırmamak.

Sonuç: İleri seviye hedef için daha çok veri değil, daha iyi sınırlandırılmış,
kanıtlı, geri alınabilir ve güvenli veri gerekir. JARVIS’in yetenekleri
artarken yetkisiz eylem kapasitesi otomatik olarak artmamalıdır.


======================================================================
22. YAZILIM GELİŞTİRME VE CODING AGENT VERİ PLANI
======================================================================

Amaç: JARVIS’in kod açıklama, hata ayıklama, test yazma, refactoring,
dokümantasyon, güvenli patch üretme ve repository iş akışlarını kontrollü
biçimde yürütebilmesi.

22.1 Kaynaklar

- Rust, Python, Go, TypeScript, C/C++, Java, Kotlin ve shell için resmi
  dil/standart kütüphane dokümantasyonu.
- Resmi framework ve tool dokümantasyonu; sürüm numarası ve lisansla.
- Lisansı açık ve ticari/araştırma kullanımına uygun repository’ler.
- Kendi yazdığımız örnek projeler, bug fixture’ları ve test commit’leri.
- Açık issue/PR geçmişleri; yalnız lisans ve kullanım şartları uygunsa.
- Açık lisanslı API specification, RFC, design doc ve migration notları.
- Güvenlik için CWE, OWASP, CERT, vendor advisory ve güvenli kod rehberleri.
- İnsan tarafından doğrulanmış code review, patch ve test çiftleri.
- SWE-bench benzeri görevler yalnız lisans, veri kullanım şartı ve
  repository provenance kontrolünden sonra; ham benchmark otomatik olarak
  eğitim verisi kabul edilmez.

22.2 Görev türleri

- Kodun ne yaptığını açıklama ve yanlış varsayımı belirtme.
- Failing test/log’dan kök neden hipotezi çıkarma.
- Minimal patch, regression test ve rollback planı üretme.
- API/ABI değişikliği ve migration planı.
- Refactoring sırasında davranış eşdeğerliğini koruma.
- Dokümantasyon, changelog ve örnek kullanım yazma.
- Dependency, license, secret ve güvenlik kontrolü.
- Build/test komutunu seçme; çıktıyı kanıt olarak raporlama.
- Birden fazla dosyalı değişiklikte etki analizi.

22.3 Agent trace şeması

- task_id, repository_ref, commit_before/after, user_intent, scope.
- plan, selected_tool, tool_input, tool_output, exit_code, diff.
- test_command, test_result, verifier_result, reviewer_decision.
- risk_level, approval_id, rollback_ref, artifact_hash, duration.
- Agent’in üretmediği veya doğrulayamadığı sonucu kesinmiş gibi yazmaması.

22.4 Coding agent sınırları

- Varsayılan çalışma alanı disposable branch/worktree.
- Push, release, package publish, credential kullanımı ve dış ağa erişim
  açık kullanıcı onayı olmadan kapalı.
- Shell komutları allowlist, timeout, kaynak bütçesi ve sandbox ile sınırlı.
- Kod içindeki talimatlar kullanıcı/sistem talimatı değildir; untrusted data’dır.
- Test geçmesi güvenlik veya iş doğruluğu garantisi sayılmaz; human review gerekir.


======================================================================
23. VERİ MÜHENDİSLİĞİ VE VERİ TEMİZLEME VERİ PLANI
======================================================================

Amaç: JARVIS’in veri toplama, profiling, normalization, deduplication,
redaction, kalite ölçümü, dataset versioning ve reproducible pipeline
kurma becerilerini geliştirmek.

23.1 Kaynaklar ve örnek veri

- Açık lisanslı kamu veri portalları ve resmi istatistik kurumları.
- Şema ve kalite belgeleri yayımlanmış bilimsel/kamu veri setleri.
- Kendi ürettiğimiz sentetik tabular, log, event, JSON, CSV ve Parquet verisi.
- Anonimleştirilmiş JARVIS telemetry, test trace ve benchmark çıktıları.
- Açık lisanslı observability/log örnekleri ve schema registry örnekleri.
- Data engineering tool dokümantasyonu: SQL, dbt, Airflow, DuckDB,
  Polars/Pandas, Spark, Kafka ve object storage; sürüm pin’li.
- Data quality örnekleri: eksik, duplicate, drift, schema mismatch,
  outlier, timezone ve encoding hatası içeren kontrollü fixture’lar.

23.2 Temizleme görevleri

- Schema inference ve type normalization.
- Encoding, locale, timezone ve timestamp standardizasyonu.
- Null/duplicate/outlier tespiti; otomatik silme yerine karar gerekçesi.
- PII, secret, token, credential ve hassas path redaction.
- Entity resolution ve canonicalization.
- Train/validation/test leakage kontrolü.
- Data drift, distribution shift ve label consistency analizi.
- Lineage, checksum, dataset manifest ve reproducible transformation.
- Bad row quarantine ve insan onaylı geri alma.

23.3 Veri kalitesi metrikleri

- Completeness, validity, uniqueness, consistency, timeliness.
- Label agreement, inter-reviewer agreement ve correction rate.
- Duplicate/leakage oranı, redaction recall ve false-redaction oranı.
- Pipeline reproducibility, schema breakage ve rollback başarısı.
- Kaynak provenance completeness ve lisans doğrulama oranı.

23.4 Güvenli veri pipeline’ı

1. Source registry ve acquisition manifest.
2. Hash/checksum ve immutable raw quarantine.
3. Secret/PII/malware/injection taraması.
4. Normalize ve versioned transformation.
5. Human review ve quality report.
6. Split, leakage test ve locked eval set.
7. Dataset release, rollback ve deletion index.


======================================================================
24. MODEL EĞİTİMİ, ADAPTASYONU VE EVAL VERİ PLANI
======================================================================

JARVIS için ilk tercih sırası: RAG → structured skill/tool policy → eval
ve regression → gerekirse LoRA/adapter → en son geniş kapsamlı fine-tuning.
Her davranışı model ağırlığına gömmek yerine değişebilir bilgi ve politikayı
ayrı katmanlarda tutmak model değişimini kolaylaştırır.

24.1 Eğitim verisi kaynakları

- Resmi teknik doküman ve standartların lisans uyumlu özetleri.
- Kendi yazdığımız instruction/response ve tool-trace çiftleri.
- Human-reviewed coding, debugging, security report ve data-quality görevleri.
- Sentetik görevler: her biri insan örneği ve verifier ile karşılaştırılır.
- Açık lisanslı dataset’ler; yalnız kullanım hakkı ve türev lisans kontrolüyle.
- Başarısız, belirsiz, refusal ve safe-stop örnekleri.

24.2 Eğitim örneği şeması

- instruction, context, evidence, expected_response, forbidden_claims.
- tool_policy, allowed_actions, approval_required, verifier.
- language, domain, difficulty, source, license, reviewer, dataset_version.
- privacy_status, safety_label, provenance_hash, split (train/val/test/eval).

24.3 Eğitimden önce zorunlu kontroller

- Lisans ve türev eser incelemesi.
- Secret/PII/malware/prompt-injection taraması.
- Near-duplicate ve benchmark contamination kontrolü.
- Train/test leakage ve zaman bazlı split.
- Poisoning/adversarial örneklerin ayrı eval’de tutulması.
- Turkish/English kalite ve karakter encoding testi.
- Modelin kaynak uydurmaması, belirsizlik belirtmesi ve yetki istemesi.

24.4 Eval matrisi

- Coding: compile, unit test, integration test, patch correctness.
- Agent: tool selection, plan quality, approval, rollback, stop behavior.
- Security: finding validity, scope compliance, safe refusal, remediation.
- Data engineering: schema, cleaning, lineage, leakage ve quality metrics.
- Model quality: groundedness, citation, hallucination, latency, memory,
  CPU/RAM kullanımı ve context retention.

24.5 Model eğitimi güvenlik kuralları

- Ham kullanıcı sohbeti açık onay ve redaction olmadan eğitim verisi olmaz.
- Model, tool yetkisini eğitim örneğinden miras alamaz; policy runtime’da uygulanır.
- Fine-tuning sonrası aynı kilitli test seti ve geri dönüş modeli korunur.
- Model değişiminde kullanıcı profili, memory, RAG ve tool sözleşmesi
  ağırlıklardan bağımsız tutulur.


======================================================================
25. VERİYİ NASIL BULACAĞIZ VE TOPLAYACAĞIZ?
======================================================================

1. Kaynak haritası: resmi doküman, açık lisanslı repo, kamu veri portalı,
   kontrollü lab ve kendi fixture’larımızı ayrı kayıt altına al.
2. Lisans filtresi: izin belirsizse indirme/ingest yapma; önce hukuk ve
   proje lisansını doğrula.
3. Küçük pilot: her konu için 20–50 temsilî kayıtla kaliteyi ölç.
4. Kendi veri üretimi: fixture, bug, test, tool trace ve reviewer kaydıyla
   dış kaynağa bağımlılığı azalt.
5. Manifest + hash: her dosyayı source, version, tarih, lisans ve hash ile kaydet.
6. Quarantine: hiçbir veri doğrudan model, RAG veya agent tool’una gitmez.
7. Profiling ve redaction: schema, secret, PII, injection ve duplicate kontrolü.
8. İnsan incelemesi: kabul, RAG-only, eval-only veya reject etiketi ver.
9. Split ve eval: test setini kilitle; eğitim sürecinden uzak tut.
10. Sürümleme: dataset release, değişiklik günlüğü, rollback ve silme indeksini tut.

İlk pratik hedef: büyük veri yığını değil; coding, data engineering,
agent orchestration ve security için küçük fakat tamamen izlenebilir,
doğrulanmış ve tekrar üretilebilir bir “golden set” oluşturmaktır.


======================================================================
26. LOCAL-FIRST VE ÇOKLU CİHAZ AGENT MİMARİSİ
======================================================================

JARVIS’in temel çalışma modeli local-first olacaktır. Hassas context,
memory, dosya, kod ve tool trace varsayılan olarak cihaz dışına çıkmaz.
Cihazlar arası iletişim gerekiyorsa şifreli, kimliği doğrulanmış, açık
izinli ve minimum veri taşıyan bir protokol kullanılır.

26.1 Desktop: ileri seviye agentic yetenekler

- Yerel model ve RAG; CPU/RAM öncelikli, GPU opsiyonel.
- Repository/workspace okuma, planlama, patch, test ve raporlama.
- Sandbox içindeki shell, editor, git, browser ve security worker’ları.
- Coding, data engineering, OSINT, cloud, pentest ve malware/RE worker’ları.
- Tool allowlist, approval gate, scope manifest, timeout, bütçe ve kill-switch.
- Çok adımlı plan, ara kanıt, geri alma ve insan escalation.
- Offline çalışma; ağ kullanan tool’lar ayrıca izin ve audit ister.
- Agent sonucu: plan → tool trace → evidence → verifier → kullanıcı özeti.

Desktop için “ileri seviye” hedef, sınırsız otonomi değil; karmaşık görevi
parçalayıp güvenli sınırlar içinde yürütebilen, hatada duran ve kanıt sunan
agent davranışıdır.

26.2 Android: orta seviye agentic yetenekler

- Yerel veya cihazla eşleşmiş desktop modeline güvenli istek aktarımı.
- Not, özet, hatırlatıcı, metin düzenleme, dosya seçimi ve bildirim işlemleri.
- Kamera/görsel analiz yalnız açık kullanıcı izni ve uygulama içi göstergeyle.
- Mobilde shell, pentest, malware çalıştırma, geniş dosya taraması ve
  sessiz arka plan otomasyonu varsayılan olarak kapalı.
- Android permission, scoped storage, foreground service ve battery bütçesi.
- Offline kuyruk, şifreli local storage, ekran kilidi ve remote revoke.
- Her hassas işlem için onay, açıklama ve iptal seçeneği.

Android’de “orta seviye” hedef: güvenilir yardımcı ve kontrollü remote
controller; desktop seviyesinde serbest agent yürütme değildir.

26.3 Ortak cihaz veri şeması

- device_id, app_version, model_ref, capability_profile, permission_state.
- request_id, source_device, destination_worker, user_approval,
  encrypted_context_ref, action, result, verifier, revoke_state.
- Sync edilen veri açıkça sınıflandırılır: public, private, sensitive,
  restricted. Restricted context cihazdan çıkmaz.

26.4 Local-first eval’leri

- Ağ kapalıyken temel görevlerin çalışması.
- Model server yeniden başlayınca kuyruk ve memory bütünlüğü.
- Android izin reddi, offline, düşük pil, düşük depolama ve bağlantı kopması.
- Desktop/Android arasında yetkisiz tool aktarımının engellenmesi.
- Şifreli iletişim, replay, stale request ve remote revoke testleri.


======================================================================
27. MOBİL UYGULAMA GELİŞTİRME VERİ PLANI
======================================================================

Amaç: JARVIS’in Android uygulaması geliştirme, test, release ve güvenlik
süreçlerini orta seviye doğrulukla desteklemesi.

27.1 Kaynaklar

- Android Developers, Kotlin, Jetpack, Compose ve resmi API belgeleri.
- Android security, privacy, permissions, accessibility ve background task
  rehberleri.
- OWASP MASVS/MSTG ve resmi Google Play policy belgeleri.
- Açık lisanslı örnek uygulamalar ve kendi fixture uygulamalarımız.
- Gradle, Kotlin Multiplatform, Flutter veya React Native belgeleri;
  seçilen stack için sürüm pin’i tutulur.

27.2 Görev türleri

- Screen/state/navigation ve responsive layout.
- Compose UI, accessibility, localization ve dark/light theme.
- ViewModel, repository, offline cache, sync ve error state.
- Permission, notification, deep link, camera/file picker ve share sheet.
- Room/SQLDelight, encrypted storage, key management ve migration.
- Unit/UI/instrumentation test, mock server ve offline test.
- APK/AAB build, signing boundary, release notes ve crash analysis.
- Battery, memory, network, background ve privacy optimizasyonu.

27.3 Mobil agent eval’i

- Kullanıcı izni olmadan hiçbir hassas API çağrılmaması.
- Yanlış permission, bağlantı veya model cevabında güvenli durma.
- Ekran okuyucu, büyük font, Türkçe/İngilizce ve düşük donanım testi.
- Offline kuyrukta veri kaybı olmaması ve tekrar gönderimde duplicate olmaması.
- Uygulama güncellemesinde memory, settings ve conversation migration.


======================================================================
28. WEB SİTESİ VE WEB UYGULAMASI GELİŞTİRME VERİ PLANI
======================================================================

28.1 Kaynaklar

- HTML, CSS, JavaScript/TypeScript ve web platformu resmi belgeleri.
- React, Svelte, Vue, Astro, Next.js veya seçilen framework’ün resmi docs’u.
- MDN, W3C, WHATWG, web accessibility ve web performance rehberleri.
- OWASP ASVS, WSTG, API Security ve secure headers belgeleri.
- Kendi frontend/backend fixture’ları ve kabul testleri.

28.2 Görev türleri

- Component, layout, routing, form, validation ve error boundary.
- API contract, auth/session, authorization ve rate limiting.
- SSR/CSR/ISR, caching, observability ve performance budget.
- Database migration, queue, background worker ve idempotency.
- Accessibility, responsive design, i18n/l10n ve SEO metadata.
- Unit/component/e2e/contract test, visual regression ve browser matrix.
- CSP, CSRF, XSS, injection, secret handling ve dependency review.
- Docker/container, CI/CD, preview environment ve rollback.

28.3 Web agent sınırları

- Production deploy, DNS, billing ve secret erişimi insan onaylıdır.
- Browser automation yalnız izinli test/staging scope’unda.
- Kullanıcı verisi ve gerçek auth token’ları test dataset’ine girmez.
- Generated code build, lint, test ve security scan geçmeden öneri olarak kalır.


======================================================================
29. CLOUD GELİŞTİRME VERİ PLANI
======================================================================

Amaç: JARVIS’in cloud-native uygulama, altyapı, deployment ve işletim
konularında orta seviye güvenli geliştirme desteği vermesi.

29.1 Kaynaklar

- AWS/Azure/GCP resmi architecture ve security well-architected belgeleri.
- Kubernetes, Docker, Helm, Terraform/OpenTofu ve GitHub/GitLab CI docs’u.
- CNCF, OpenTelemetry, Prometheus, Grafana ve resmi proje rehberleri.
- NIST, CIS, CSA CCM ve cloud provider security baseline’ları.
- LocalStack, MinIO, kind/k3d ve kendi staging fixture’larımız.

29.2 Görev türleri

- IaC module, variable, state, provider pinning ve drift kontrolü.
- Network, IAM, secret, storage, queue, database ve compute tasarımı.
- Container image, SBOM, signature, vulnerability scan ve admission.
- Kubernetes deployment, service, ingress, config, secret ve policy.
- CI/CD pipeline, artifact promotion, canary, blue/green ve rollback.
- Autoscaling, health/readiness, observability, cost ve capacity.
- Backup/restore, disaster recovery, RTO/RPO ve chaos testleri.
- Multi-environment config, feature flag ve change approval.

29.3 Cloud agent güvenlik profili

- Varsayılan hedef local emulator veya staging’dir.
- Provider credential’ı modele verilmez; kısa ömürlü, dar scope’lu worker kullanılır.
- `plan` ve `diff` üretimi serbest; apply/destroy/release açık onaylıdır.
- Billing, public exposure, IAM genişletme ve veri silme işlemleri çift onay ister.
- Her değişiklik öncesi policy-as-code, cost estimate, backup ve rollback kontrolü.


======================================================================
30. ÇOKLU PLATFORM GELİŞTİRME GOLDEN SETİ
======================================================================

İlk golden set şu görev kümelerinden oluşur:

- 40 desktop coding/debugging görevi.
- 30 Android UI/state/permission görevi.
- 30 web frontend/backend/security görevi.
- 30 cloud/IaC/Kubernetes görevleri.
- 20 offline/sync/device-agent görevi.
- 20 güvenli redirection, refusal ve approval görevi.

Her görevde başlangıç durumu, scope, beklenen diff, test, güvenlik
kısıtları, verifier ve insan kabul kriteri bulunur. Bu sayılar nihai kota
değil ilk benchmark’tır; başarısız ve belirsiz örnekler özellikle korunur.

Hedef: Desktop’ta ileri agentic planlama ve tool orkestrasyonu; Android’de
orta seviye izinli yardımcı deneyimi; mobil/web/cloud geliştirmede güvenli,
test edilebilir ve orta seviye üretim desteği. Tüm platformlarda ortak
policy, provenance, memory, audit ve rollback katmanı kullanılır.


======================================================================
31. GENİŞ DİL VE FRAMEWORK KAPSAMI
======================================================================

Hedef: JARVIS’in tek bir dile veya framework’e kilitlenmeden, kullanıcının
repository’sindeki teknoloji yığınına uyum sağlayarak kod yazabilmesi,
okuyabilmesi, test edebilmesi ve bakım yapabilmesi.

“Her dilde kusursuz kod” gerçekçi bir garanti değildir. Bunun yerine her
dil/framework için şu yetenek sözleşmesi uygulanır: tanı → bağlamı oku →
resmi dokümana başvur → minimal değişiklik yap → derle/test et → kanıt sun →
belirsizse soru sor veya güvenli şekilde dur.

31.1 Dil kapsama grupları

- Sistem ve performans: Rust, C, C++, Zig, Assembly okuma.
- Backend: Go, Java, Kotlin, C#, F#, Python, Ruby, PHP, Elixir, Scala.
- Web: JavaScript, TypeScript, HTML, CSS, SQL, GraphQL.
- Mobil: Kotlin, Java, Swift, Dart, Objective-C.
- Veri/AI: Python, R, Julia, SQL, notebook ve pipeline DSL’leri.
- Shell/otomasyon: Bash, Fish, PowerShell, Make, Taskfile ve CI YAML.
- Config/IaC: JSON, YAML, TOML, XML, HCL, Dockerfile, Helm/Kustomize.
- Smart contract ve özel DSL’ler: yalnız güvenli lab ve compiler
  doğrulamasıyla; üretim finansal işlem yetkisi yok.

31.2 Framework ve platform kapsamı

- Rust: Tokio, Axum, Actix, Rocket, Tauri, Bevy.
- Python: FastAPI, Django, Flask, Pydantic, PyTorch, Transformers,
  Polars, Pandas, SQLAlchemy.
- Go: net/http, Gin, Echo, Fiber, Cobra, gRPC.
- Java/Kotlin: Spring Boot, Ktor, Android Jetpack/Compose.
- .NET: ASP.NET Core, Blazor, MAUI, Entity Framework.
- JavaScript/TypeScript: Node.js, Deno, Bun, React, Next.js, Vue,
  Svelte, Angular, Astro, Express, NestJS, Electron.
- Mobile: SwiftUI, UIKit, Flutter, React Native, Kotlin Multiplatform.
- Data/infra: dbt, Airflow, Spark, Kafka, DuckDB, Terraform/OpenTofu,
  Kubernetes, Helm, Docker, Pulumi.
- Test/tooling: Playwright, Cypress, Jest, Vitest, Pytest, JUnit,
  Cargo test, Go test, .NET test ve seçilen stack’in native araçları.

Liste sabit değildir; repository tespit edildiğinde framework registry’den
sürüm ve resmi doküman seçilir. Bilinmeyen framework için JARVIS uydurma
API üretmek yerine “bağlam/doküman eksik” durumunu belirtir.

31.3 Framework registry

Her teknoloji için şu metadata tutulur:

- name, version, language, runtime, package manager, build/test commands.
- official_docs, api_reference, migration_guides, security_advisories.
- license, support_status, breaking_changes, known_constraints.
- syntax/examples, project_layout, common_failure_modes, verifier.
- compatible_model_context, install_cost, offline_availability.

31.4 Veri toplama kaynakları

- Resmi dil ve framework dokümantasyonu.
- Resmi migration guide, release note ve security advisory’ler.
- Açık lisanslı örnek repository ve test fixture’ları.
- Kendi küçük uygulamalarımız: aynı görevin farklı stack karşılıkları.
- Compiler/linter/test çıktıları ve insan onaylı patch’ler.
- Framework sürümleri arasında aynı davranışı doğrulayan compatibility testleri.

Kopyalanan kod için lisans, kaynak ve türev eser bilgisi tutulur. Belirsiz
lisanslı kod yalnız RAG/research notu olabilir; eğitim golden set’ine
alınamaz.

31.5 Çoklu dil/framework görev şablonları

- Aynı CRUD/API görevinin Rust, Go, Python, Java, C# ve TypeScript sürümleri.
- Aynı mobil ekranın Kotlin/Compose, SwiftUI, Flutter ve React Native sürümleri.
- Aynı veri pipeline’ının SQL, Python/Polars, dbt ve Spark sürümleri.
- Aynı cloud servisinin Terraform/OpenTofu, Pulumi ve Kubernetes manifesti.
- Aynı güvenlik düzeltmesinin backend, frontend, mobile ve IaC karşılıkları.
- Sürüm yükseltme, deprecated API, dependency conflict ve rollback görevleri.

31.6 Doğrulama kapıları

1. Parse/type check.
2. Formatter/linter.
3. Unit/integration/e2e test.
4. Dependency/license/secret/security scan.
5. Build artifact ve reproducibility.
6. Diff kapsamı ve beklenmeyen dosya değişikliği kontrolü.
7. Human review veya approval gate.

Bir dil/framework için compiler, runtime veya test altyapısı yoksa JARVIS
yalnız taslak ve açıklama üretir; sonucu “doğrulanmış kod” olarak sunamaz.

31.7 Kapsama ölçümü

- Dil/framework tanıma doğruluğu.
- Doğru build/test komutu seçimi.
- Compile/test pass rate ve patch correctness.
- API uydurma oranı ve kaynak gösterme oranı.
- Sürüm değişikliklerinde regression başarısı.
- Farklı stack’ler arasında gereksinim eşdeğerliği.
- Bilinmeyen teknoloji karşısında güvenli belirsizlik ve escalation.

İleri hedef: JARVIS’in teknoloji sayısını sürekli artırması değil, yeni bir
stack’i resmi doküman, repository bağlamı ve verifier ile güvenilir biçimde
öğrenebilmesi. Böylece model değişse bile framework registry, skill,
toolchain ve test sözleşmeleri korunur.


======================================================================
32. YETKİNLİK ÖNCELİĞİ VE OTOMASYON AJANI
======================================================================

JARVIS’in ana yetkinlikleri:

1. Ofansif siber güvenlik: yalnız yetkili, kontrollü ve kanıt temelli.
2. AI/data engineering: veri toplama, temizleme, eval, RAG ve model adaptasyonu.
3. Orta seviye yazılım: mobil, web ve database geliştirme.
4. Destek yetenekleri: otomasyon, DevOps, performans, UX ve dokümantasyon.

Bu öncelik sırası veri bütçesi, eval kapsamı ve agent izinlerini belirler.
Her alanda uzmanlık hedeflenmez; model bilmediğini belirtmeli, kaynak
istemeli, test edemediği kodu doğrulanmış gibi sunmamalıdır.

32.1 Otomasyon agent’ı

Otomasyon, agentic yeteneklerin doğrudan bir parçasıdır. Agent şu döngüyü
izler:

- Intent ve başarı kriterini çıkar.
- Gerekli adımları ve bağımlılıkları planla.
- Tool seçimini policy ve scope ile sınırla.
- Dry-run/preview üret.
- Kullanıcı onayı gereken adımı beklet.
- İşlemi timeout, bütçe, retry ve idempotency ile yürüt.
- Çıktıyı verifier ile kontrol et.
- Kanıt, log, değişiklik özeti ve rollback bilgisi sun.

Otomasyon veri alanları:

- Dosya ve klasör işlemleri, batch rename, backup ve arşivleme.
- Git branch, commit, diff, test ve changelog akışı.
- Build, lint, test, package ve release hazırlığı.
- CI/CD pipeline ve scheduled job yönetimi.
- Web formu, browser ve API otomasyonu yalnız izinli hedefte.
- Local system status, process, service ve log inceleme.
- Data ingestion, ETL, validation ve rapor üretimi.
- Güvenlik iş akışları: scope, recon, evidence, report ve remediation.
- Bildirim, email taslağı ve kullanıcı onaylı gönderim.

Otomasyon güvenlik profili:

- Read-only, dry-run, supervised ve bounded-autonomy modları.
- Delete, publish, deploy, credential, network ve production işlemleri
  varsayılan kapalı veya açık onaylı.
- Her işlem task id, actor, scope, tool, input, output ve verifier ile kayıtlı.
- Tekrarlanabilirlik için idempotency key ve rollback ref zorunlu.
- Hata durumunda sonsuz retry, sessiz başarısızlık ve kapsam genişletme yok.


======================================================================
33. VERİTABANI MÜHENDİSLİĞİ VE POSTGRESQL
======================================================================

JARVIS’in database hedefi orta seviye uygulama geliştirme, güvenli şema
tasarımı, migration, sorgu analizi ve işletim desteğidir.

33.1 Genel veri tabanı kapsamı

- SQL modelleme, normalization/denormalization ve constraint.
- Transaction, isolation, locking, deadlock ve idempotency.
- Index, query plan, pagination, caching ve connection pool.
- Migration, seed, rollback, backup/restore ve disaster recovery.
- SQL/NoSQL seçimi, trade-off ve polyglot persistence.
- ORM/query builder kullanımı ve N+1 analizi.
- Audit, retention, PII, encryption ve least-privilege role.
- Replication, partitioning, sharding ve read/write separation temelleri.
- CDC, event/outbox, queue ve cache invalidation.
- Data quality, lineage, schema evolution ve compatibility.

33.2 PostgreSQL özel kapsamı

- Schema, role, grant, RLS ve ownership modeli.
- MVCC, isolation level, lock ve transaction davranışı.
- B-tree, hash, GIN, GiST, BRIN ve partial/covering index.
- EXPLAIN/EXPLAIN ANALYZE, statistics, vacuum, analyze ve bloat.
- JSONB, array, full-text search, generated column ve extension.
- CTE, window function, materialized view ve recursive query.
- Partitioning, logical/physical replication ve read replica.
- pg_dump/pg_restore, PITR, WAL, backup doğrulama ve restore testi.
- Connection pooler, timeout, statement budget ve migration safety.
- PostGIS yalnız sonraki uzmanlık uzantısı olarak, izole fixture ile.

33.3 PostgreSQL veri kaynakları

- PostgreSQL resmi manual, SQL reference, release note ve security advisory.
- Resmi extension ve driver/ORM dokümantasyonu.
- Kendi schema, migration, slow-query ve recovery fixture’larımız.
- Sentetik e-ticaret, log, chat, telemetry ve permission dataset’leri.
- Açık lisanslı benchmark’lar; lisans/provenance kontrolüyle.

33.4 Database agent görevleri

- Gereksinimden şema ve migration taslağı.
- Sorgu açıklama, index önerisi ve EXPLAIN yorumlama.
- Güvenli seed, fixture ve test database oluşturma.
- Migration dry-run, backward compatibility ve rollback planı.
- Backup restore rehearsal ve data integrity kontrolü.
- RLS/role policy ve PII erişim incelemesi.
- Production apply/destroy kesinlikle insan onaylı.


======================================================================
34. DAĞITIK SİSTEMLER VE BACKEND TEMELLERİ
======================================================================

- HTTP/gRPC, REST/GraphQL, timeout, retry, circuit breaker ve backpressure.
- Queue, event, pub/sub, ordering, duplicate ve exactly-once varsayımı.
- Consistency, availability, partition ve idempotent command.
- Cache, rate limit, session, distributed lock ve leader election.
- Service discovery, config, feature flag ve schema registry.
- Multi-tenant isolation, quota, graceful shutdown ve health check.
- Observability: metric, log, trace, correlation id ve incident timeline.
- Load test, failure injection, chaos ve recovery rehearsal.

Veri örnekleri başarılı akış kadar timeout, duplicate, partial failure,
stale read, rollback ve güvenli stop senaryolarını da içermelidir.


======================================================================
35. QA, TEST VE DOĞRULAMA MÜHENDİSLİĞİ
======================================================================

- Unit, integration, contract, end-to-end ve smoke test.
- Property-based, fuzz, mutation, regression ve golden test.
- Load, stress, soak, memory, concurrency ve race test.
- Visual regression, accessibility, cross-browser/device test.
- Security SAST, DAST, dependency, secret, SBOM ve license test.
- Test data fixture, mock/stub, deterministic clock ve reproducibility.
- Flaky test detection, quarantine, test impact ve coverage kalitesi.
- Agent çıktısı için verifier, judge disagreement ve human acceptance.

Agent test yazarken yalnız test eklememeli; testin yanlış pozitif/negatif
üretip üretmediğini de incelemeli ve kapsam sınırını raporlamalıdır.


======================================================================
36. DEVOPS, SRE VE RELEASE MÜHENDİSLİĞİ
======================================================================

- Git flow, branch protection, code review ve signed artifact.
- CI pipeline, cache, build matrix, artifact retention ve provenance.
- Container, registry, SBOM, image scan ve promotion.
- Environment/config/secret ayrımı ve ephemeral preview.
- Canary, blue/green, feature flag, rollback ve change window.
- SLO/SLI/SLA, error budget, alert, on-call ve incident response.
- Log/metric/trace, dashboard, health/readiness ve capacity planning.
- Backup, restore, disaster recovery ve game-day rehearsal.
- Cost, resource quota, autoscaling ve idle resource temizliği.

Agent hiçbir ortamda credential’ı kalıcı saklamaz; production release,
DNS, billing ve destructive migration iki aşamalı onay ister.


======================================================================
37. PERFORMANS, GÜVENİLİRLİK VE KAYNAK OPTİMİZASYONU
======================================================================

- CPU, RAM, disk I/O, network, GPU/VRAM ve enerji profili.
- Profiling, flame graph, allocation, lock contention ve syscall maliyeti.
- Latency percentile, throughput, queue depth ve cold-start.
- Model inference: context, batch, thread, quantization ve cache.
- Mobil battery/thermal, web Core Web Vitals ve database query budget.
- Benchmark harness, baseline, regression threshold ve reproducibility.
- Degradation, overload, graceful fallback ve user-visible error.


======================================================================
38. UI/UX, ACCESSIBILITY VE LOKALİZASYON
======================================================================

- Information architecture, design system, component state ve empty/error UI.
- Keyboard, mouse, touch, screen reader, focus, contrast ve large font.
- Responsive desktop/mobile layout ve terminal/TUI kullanılabilirliği.
- Türkçe/İngilizce metin, pluralization, date/time/number ve RTL hazırlığı.
- Undo, cancel, progress, retry, approval ve destructive action açıklaması.
- Kullanıcı araştırması, usability test, visual regression ve feedback loop.
- JARVIS konuşmalarında bağlam, kısa cevap, ayrıntı seviyesi ve ton kontrolü.


======================================================================
39. PAKETLEME, DAĞITIM VE YAŞAM DÖNGÜSÜ
======================================================================

- Desktop binary, package, installer, signing ve auto-update.
- Android APK/AAB, signing key, staged rollout ve rollback.
- Web static/server artifact, cache invalidation ve deployment manifest.
- Versioning, compatibility matrix, migration, release note ve support window.
- Crash report, telemetry opt-in, privacy, uninstall ve data deletion.
- Reproducible build, artifact hash ve release provenance.


======================================================================
40. PRIVACY, UYUMLULUK VE YÖNETİŞİM
======================================================================

- Data classification, minimization, purpose limitation ve retention.
- Consent, export, deletion, access request ve audit trail.
- Secret/PII redaction, encryption at rest/in transit ve key rotation.
- License/SBOM, third-party notice ve vulnerability disclosure.
- Human approval, accountability, model card, change record ve incident log.
- GDPR benzeri privacy prensipleri; hukuki yorum gerektiğinde uzman incelemesi.


======================================================================
41. VERİ BÜTÇESİ VE GERÇEKÇİLİK MODELİ
======================================================================

Milyarlarca satır veri zorunlu değildir. JARVIS için değerli ilk veri,
tekrar üretilebilir ve doğrulanmış görev örnekleridir:

- Her ana yetenek için 100–500 yüksek kaliteli golden/eval görevi.
- Başarılı görev yanında hata, belirsizlik, refusal, rollback ve approval örneği.
- Resmi dokümanların RAG için lisans uyumlu, kaynaklı özetleri.
- Kendi repository, fixture, test, agent trace ve lab kayıtlarımız.
- Sentetik verinin mutlaka verifier ve insan örneğiyle karşılaştırılması.

Önceliklendirme:

- En yüksek veri/eval bütçesi: offensive security ve AI/data engineering.
- Orta bütçe: mobile, web, PostgreSQL/database ve otomasyon.
- Destek bütçesi: DevOps, SRE, UX, release, performance ve compliance.
- Uzmanlık dışı alanlar: önce RAG + tool adapter; model eğitimi daha sonra.

Bu yaklaşım daha az fakat daha iyi veriyle güçlü bir sistem kurmayı hedefler.
Modelin her alanda uzman görünmesi değil, doğru sınır koyması ve doğrulanabilir
sonuç üretmesi başarı ölçütüdür.


======================================================================
42. KİŞİSEL GÜNLÜK YARDIMCI VE YAŞAM İŞ AKIŞLARI
======================================================================

JARVIS yalnız teknik bir agent değil, kullanıcısının günlük işlerini
kolaylaştıran kişisel ve local-first bir yardımcı olacaktır.

42.1 Temel günlük yetenekler

- Serbest sohbet, bağlamı koruma ve kullanıcının tercihlerini hatırlama.
- Not alma, düzenleme, etiketleme, arama, özetleme ve arşivleme.
- Hatırlatıcı, görev listesi, önceliklendirme ve günlük plan oluşturma.
- Takvim okuma, uygun zaman önerme ve kullanıcı onaylı etkinlik taslağı.
- Email/mesaj taslağı, ton değiştirme, özetleme ve yanıt önerisi.
- Dosya bulma, sınıflandırma, yeniden adlandırma ve güvenli arşivleme.
- PDF, web sayfası, ders notu ve toplantı içeriği özetleme.
- Çeviri, yazım düzeltme, yeniden ifade etme ve Türkçe/İngilizce destek.
- Ders çalışma, konu anlatımı, quiz, flashcard ve kişisel öğrenme planı.
- Alışkanlık, proje ve hedef takibi; ilerleme özeti ve nazik hatırlatma.
- Yerel sistem durumu, uygulama açma, müzik/medya ve bildirim kontrolü.
- Görsel, ekran görüntüsü ve belgeyi açıklama; vision hazır değilse bunu belirtme.

42.2 Orta seviye kişisel iş akışları

- Birden fazla notu birleştirip konu/özet/aksiyon listesi çıkarma.
- Gelen mesaj veya email’leri konu, aciliyet ve yanıt gereksinimine göre ayırma.
- Ders, coding ve pentest çalışmalarını zaman bloklarına bölme.
- Dosya ve repository içinde doğal dille arama ve ilgili kaynakları gösterme.
- Harici bilgi gerektiğinde arama planı önerme; otomatik işlem öncesi onay alma.
- Tekrarlanan işleri otomasyon şablonuna dönüştürme.
- Günlük/haftalık durum raporu ve açık görev özeti.

42.3 Kişiselleştirme ve memory

- Kullanıcı adı, dil, ton, kısa/ayrıntılı cevap tercihi ve teknik seviye.
- Proje, repository, faz, karar, backlog ve açık sorular.
- Uzun süreli memory ile geçici conversation context ayrımı.
- Her memory kaydında kaynak, tarih, confidence, visibility ve silme seçeneği.
- Kullanıcı memory’yi görüntüleyebilir, düzeltebilir, dışa aktarabilir veya silebilir.
- Hassas bilgiler varsayılan olarak hatırlanmaz; explicit opt-in gerekir.

42.4 Günlük yardımcı güvenlik sınırları

- Email gönderme, takvim değişikliği, dosya silme, satın alma ve dış iletişim
  yalnız açık kullanıcı onayıyla.
- Bankacılık, sağlık, hukuki ve finansal kararlar otomatik uygulanmaz.
- Kişisel dosya alanı scope ile sınırlıdır; tüm disk taraması varsayılan kapalı.
- Bildirimler hassas metni kilit ekranında göstermemek üzere sınıflandırılır.
- Local-first depolama, şifreleme, retention ve cihazlar arası minimum sync.
- Kullanıcı “unut” dediğinde memory ve türetilmiş index kayıtları da silinir.

42.5 Günlük yardımcı eval seti

- 50 sohbet ve bağlam koruma senaryosu.
- 30 not/takvim/görev/hatırlatıcı senaryosu.
- 20 dosya ve repository arama senaryosu.
- 20 eğitim, özetleme ve çeviri senaryosu.
- 20 izin reddi, hassas veri ve güvenli durma senaryosu.
- Türkçe/İngilizce, kısa/uzun cevap, yazım hatası ve belirsiz istek varyantları.

Kişisel yardımcı katmanı JARVIS’in teknik worker’larından ayrıdır; worker’lar
yalnız gerekli context’i alır. Böylece model değişse veya pentest agent’ı
gelişse bile kullanıcının memory’si, tercihleri ve günlük iş akışları korunur.


======================================================================
43. HARİCİ AI ARAÇLARINI ÖĞRENME VE KARŞILAŞTIRMA KAYNAĞI
======================================================================

Codex, Claude Code ve benzeri araçlar JARVIS için yararlı bir “öğretmen,
karşılaştırma ve eval” kaynağı olabilir. Ancak çıktılarını kontrolsüz biçimde
model eğitimine aktarmayacağız.

43.1 Kullanım biçimleri

- Aynı coding/debugging görevini birden fazla araçla çözdürüp sonuçları
  karşılaştırma.
- Plan kalitesi, patch doğruluğu, test kapsamı, güvenlik ve latency ölçümü.
- İyi açıklama, hata teşhisi, tool seçimi ve refusal örneklerini insanın
  yeniden yazdığı/olduğu gibi kabul ettiği kayıtlar.
- JARVIS’in başarısız olduğu görevlerde öğretmen çıktısını referans çözüm
  olarak kullanma.
- Prompt formatı veya kişisel üslubu kopyalamak yerine problem çözme adımı,
  kanıt ve verifier mantığını çıkarma.

43.2 Zorunlu yönetişim

- Her kayıt araç adı, model/sürüm, tarih, kullanıcı prompt’u, çıktı,
  kullanım izni, servis şartı ve provenance ile saklanır.
- Sağlayıcının şartları ve lisansı izin vermiyorsa çıktı eğitim verisi olmaz;
  yalnız insanın yazdığı özet veya eval sonucu tutulur.
- Kişisel, gizli, müşteri veya secret içeren prompt’lar dış AI araçlarına
  gönderilmez; önce redaction ve açık onay gerekir.
- Harici çıktılar doğru kabul edilmez; compile/test/security scan ve insan
  incelemesi gerekir.
- Bir modelin hatalı veya unsafe çıktısı “negatif eval” olarak etiketlenir,
  doğrulanmış bilgi olarak RAG’e alınmaz.
- Sağlayıcıya özel sistem talimatları, private chain-of-thought veya erişim
  kısıtlarını çıkarmaya yönelik veri toplama yapılmaz.

43.3 Öğretmen karşılaştırma şeması

- task_id, task_domain, difficulty, repository_ref, input_context.
- provider/model_ref, output, tool_trace_available, test_result.
- correctness, security, groundedness, style, latency, cost.
- human_verdict, accepted_facts, rejected_claims, JARVIS_learning_target.
- license/terms_status, privacy_status, retention ve deletion marker.

43.4 Son kullanım

Harici AI çıktıları öncelikle regression/eval ve RAG için kaynaklı özet
olarak kullanılır. Fine-tuning’e ancak açık izin, redaction, kalite kontrolü,
duplicate/leakage kontrolü ve dataset yönetişimi tamamlandıktan sonra karar
verilir. Hedef, başka modelleri kopyalamak değil; JARVIS’in eksik becerisini
kanıtla ölçüp geliştirmektir.


======================================================================
44. İLERİ WINDOWS VE LINUX SİSTEM YÖNETİMİ
======================================================================

Amaç: JARVIS’in iki işletim sisteminde sistemi okuyabilmesi, config hatasını
teşhis edebilmesi, güvenli düzeltme önerebilmesi ve kullanıcı onayıyla
uygulayabilmesi.

44.1 Linux kapsamı

- systemd/service, journal/log, process, filesystem, permissions ve users.
- Shell, environment, PATH, package manager, kernel/module ve boot.
- Networking: interface, route, DNS, firewall, proxy, TLS ve certificates.
- Containers, mounts, cgroups, namespaces, udev ve device access.
- SSH, sudo/polkit, secrets, cron/timers ve scheduled jobs.
- Desktop/Wayland/Hyprland, audio, display, input, notification ve GPU.
- Disk, filesystem, SMART, mount, quota, backup ve restore.

44.2 Windows kapsamı

- PowerShell, services, Event Viewer, Task Scheduler ve process yönetimi.
- Registry, environment, PATH, users/groups, ACL ve UAC.
- Windows Firewall, network profile, DNS, proxy, certificates ve WinHTTP.
- Package/update, driver, device, boot, recovery ve system restore.
- WSL, Hyper-V, containers, RDP, SMB ve scheduled task.
- Defender, audit policy, event forwarding ve security baseline.
- Application config, .NET/runtime, PATH ve permission problemleri.

44.3 Config diagnosis veri örnekleri

- Hatalı config, log/error, ortam bilgisi, beklenen davranış ve kök neden.
- Minimal diff, neden açıklaması, test/validation ve rollback.
- Aynı hatanın Linux/Windows karşılaştırmalı örnekleri.
- Encoding, locale, timezone, path separator ve privilege farkları.
- Boot/service failure, port conflict, missing dependency, permission,
  certificate, DNS/proxy, environment ve package mismatch.

44.4 Sistem agent güvenlik modeli

- İlk adım daima read-only diagnosis ve config snapshot.
- Değişiklikten önce diff, risk, backup ve rollback gösterilir.
- Service stop, firewall, registry, ACL, package uninstall, boot ve network
  değişiklikleri açık onay gerektirir.
- Root/Administrator token modele verilmez; dar kapsamlı worker kullanılır.
- Destructive veya geri dönüşü belirsiz işlem otomatik uygulanmaz.
- Düzeltme sonrası health check, test ve önce/sonra kanıtı tutulur.

44.5 Windows/Linux eval seti

- Her platform için en az 40 config teşhis/düzeltme görevi.
- 20 permission/identity, 20 network/DNS/TLS, 20 service/package,
  20 desktop/device ve 20 recovery/rollback varyantı.
- Yanlış teşhis, eksik log, çelişkili config ve kullanıcı iptali örnekleri.
- Offline, düşük yetki, bağlantı kopması ve snapshot restore senaryoları.

İleri hedef: JARVIS’in Windows ve Linux’ta “komutu ezbere çalıştırması” değil,
önce sistemi anlaması, en küçük güvenli değişikliği önermesi, kullanıcıdan
onay alması ve sonucu doğrulamasıdır.
