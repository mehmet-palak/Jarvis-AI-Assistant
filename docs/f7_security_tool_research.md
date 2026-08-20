# JARVIS Geliştirme Planı — Araç İnceleme ve Entegrasyon Notları

Tarih: 20 Ağustos 2026
Durum: Canlı plan / araştırma notu

> Bu belge, projenin kök dizininde `geliştirme_planı.txt` olarak duran bir araştırma
> notundan buraya taşındı (20 Ağustos 2026) — kullanıcı tüm içeriğin F7 planına
> eklenmesini istedi. F7'nin üst düzey [DEVELOPMENT_PLAN.md](../DEVELOPMENT_PLAN.md)
> bölümüne buradan somut, F7-kapsamlı fikirler (F7.7) çekildi; bu belgenin tamamı
> ayrıntılı referans olarak burada duruyor. İçerik değiştirilmedi, yalnız başlık
> biçimlendirildi.

Bu dosya, JARVIS’in ana mimarisini bozmadan dış araçlardan alınabilecek fikirleri,
özellikleri ve ileride yapılacak entegrasyonları kaydetmek için tutulur.

Temel ilke
----------

JARVIS hibrit çalışacak:

- Kullanıcı isterse kontrollü otonom görev yürütülebilecek.
- Kullanıcı istediği anda görevi durdurabilecek, yönlendirebilecek veya kapsamı değiştirebilecek.
- Model hiçbir zaman tek başına tool/policy yetkisi kazanmayacak.
- Her yan etkili işlem mevcut Request -> Policy -> Task -> Tool -> Verifier -> Audit zincirinden geçecek.
- Pentest yalnız yazılı olarak yetkilendirilmiş hedeflerde ve izole worker içinde çalışacak.

FAZ DURUMU NOTU
---------------

F0-F2 ürün temeli tamamlandı. F6’nın önceki yol haritasındaki anlamı model benchmark,
dataset governance ve model adaptasyonudur; bu dosyada “F6 tamamlandı” ifadesi kullanıcının
bugünkü plan kararı olarak kabul edilir. Uygulama planı ve eski faz kayıtları arasında fark
oluşursa bu dosyada karar ayrıca belirtilir ve DEVELOPMENT_PLAN.md ile eşitlenir.

================================================================
ARAÇ 1 — STRIX
================================================================

Kaynak klasör:
/home/mehmet/Masaüstü/toollar/strix

Genel tanım
-----------

Strix, LLM destekli ve araç kullanabilen bir pentest ajanı sistemidir. ChatGPT, Claude veya
başka bir LLM’i tek başına ürün olarak sunmaz; modeli sandbox, agent orchestration, güvenlik
araçları, doğrulama ve raporlama katmanlarıyla birleştirir.

Strix’in öne çıkan özellikleri
------------------------------

[ ] Reconnaissance ve attack-surface discovery
[ ] White-box kaynak kod taraması
[ ] Black-box web/API testleri
[ ] Browser ve HTTP proxy tabanlı testler
[ ] Shell/command execution ve Python PoC runtime
[ ] Root agent + uzman alt ajanlardan oluşan agent graph
[ ] Ajanların ortak bulgu ve görev durumu paylaşması
[ ] Gerçek PoC ile bulgu doğrulama
[ ] CVSS/CWE/evidence/remediation alanları olan structured findings
[ ] Markdown, JSON, CSV ve SARIF raporları
[ ] Dependency/CVE bulgularını dinamik bulgulardan ayrı raporlama
[ ] Docker sandbox, bind mount ve workspace izolasyonu
[ ] CPU/RAM/PID/disk/log/resource limitleri
[ ] Network ayarı ve raw-socket yetkilerinin açıkça tanımlanması
[ ] Bütçe, maksimum turn ve context compaction kontrolleri
[ ] Retry, timeout, idle recovery ve scan resume
[ ] Local run history ve tarayıcı tabanlı viewer
[ ] CI/CD ve pull-request güvenlik taraması
[ ] Hazır framework/cloud/API/LLM güvenlik skill’leri

JARVIS’e doğrudan alınmayacaklar
-------------------------------

- Strix’in Python/OpenAI Agents SDK merkezli runtime’ı.
- Cloud hesabı, dış telemetry veya kullanıcı verisi aktarımı.
- Host üzerinde sınırsız shell veya browser yetkisi.
- Prompt içeriğinin policy yerine geçirilmesi.
- Modelin kendi bulgusunu doğrulama olmadan doğru kabul etmesi.
- Yetkisiz veya scope dışı hedeflerde tarama.

JARVIS’e uyarlanabilecek fikirler
---------------------------------

1. İzole security worker
   F7 için Docker/bubblewrap/namespace tabanlı, network allowlist’li worker.
   Host shell fallback varsayılan olarak yasak kalacak.

2. Hibrit agent orchestration
   Kullanıcı “otomatik devam et” diyebilecek; her kritik adımda durdurma,
   yönlendirme ve approval mümkün olacak. Root agent yalnızca planlayacak;
   yürütme typed capability üzerinden yapılacak.

3. Scope manifest
   Hedef, port, protokol, izin seviyesi, süre, rate limit, kapsam dışı hedefler,
   revoke ve kill switch alanları olan imzalı/denetlenebilir scope kaydı.

4. Evidence-first finding
   Bir güvenlik bulgusu yalnızca kanıt, tekrar adımı ve mümkünse güvenli PoC ile
   rapora girecek. “Model öyle düşünüyor” tek başına bulgu sayılmayacak.

5. Ayrı finding türleri
   Dinamik doğrulanmış açık, dependency CVE’si, statik risk ve gözlem ayrı sınıflar
   olarak tutulacak. Severity ve confidence birbirine karıştırılmayacak.

6. Security report schema
   Finding ID, hedef, kapsam, kanıt, PoC özeti, CVSS, CWE, etkilenen dosya/endpoint,
   remediation, varsayımlar, verifier sonucu ve audit correlation ID.

7. Bütçe ve kaynak kontrolü
   Görev başına model turn limiti, zaman limiti, token bütçesi, CPU/RAM/disk/PID
   kotası ve çıktı boyutu sınırı.

8. Resume ve steering
   Uzun pentest işi yarıda kaldığında state kaybetmeden devam edecek. Kullanıcı
   çalışan göreve “yalnızca auth akışına odaklan”, “bu endpoint’i kapsam dışına al”
   veya “dur” komutu verebilecek.

9. Kaynak farkındalığı
   Framework, API spec, dependency lockfile, cloud ve auth bağlamı ayrı skill/data
   paketleri olarak yüklenebilecek; bunlar tool authority olmayacak.

10. SARIF ve CI entegrasyonu
    İleride GitHub/GitLab pipeline’larına bağlanabilecek; fakat merge/block kararı
    yalnız verifier ve kullanıcı/CI policy sonucuyla verilecek.

JARVIS entegrasyon eşlemesi
----------------------------

Strix fikri                  JARVIS katmanı             Hedef faz
---------------------------  -------------------------  ----------
Sandbox/resource limits      Isolated worker            F4
Scope manifest               Security policy            F7
Agent graph                  Governed task DAG          F4/F7
Recon/browser/proxy          Allowlisted capabilities  F7
PoC/evidence verifier        Finding verifier           F7
CVSS/CWE/SARIF               Security report schema     F7/F9
Budget/retry/compaction      Model runtime              F6/F9
Run history/resume           Persistence/audit          F9
CI/CD integration            MCP/remote/release        F8/F9

Önerilen hibrit modlar
----------------------

MANUAL:
- Her tool çağrısından önce kullanıcı kararı.
- En güvenli mod; geliştirme ve belirsiz hedefler için.

SUPERVISED_AUTONOMY:
- Planı model oluşturur.
- Kullanıcı planı onaylar.
- Düşük riskli read-only adımlar otomatik yürür.
- Aktif test, PoC, dosya değişikliği veya ağ erişimi approval ister.
- Kullanıcı her an durdurabilir veya scope’u daraltabilir.

BOUNDED_AUTONOMY:
- Yazılı scope ve süre/bütçe önceden tanımlıdır.
- Worker yalnız allowlist içindeki capability’leri kullanır.
- Her adım audit edilir.
- Kill switch, timeout ve resource quota zorunludur.
- Scope dışı veya yüksek riskli adımda otomatik durur.

Önceliklendirme
---------------

[1] Strix’ten kavramsal olarak scope, evidence ve sandbox modelini al.
[2] JARVIS F4 coding worker’ında resource/cancellation sınırlarını uygula.
[3] F7 pentest worker’ında yalnız SAFE/read-only mod ile başla.
[4] PoC ve active testleri approval + isolated worker arkasına koy.
[5] F7 sonunda SARIF/CVSS/CWE raporlamasını ekle.
[6] F9’da CI/CD, resume, metrics ve operasyonel dashboard’a bağla.

Güvenlik kararı
---------------

Strix’in “autonomous hacker” yaklaşımı JARVIS’te sınırsız yetki olarak uygulanmayacak.
JARVIS’in hedefi “authorized, bounded, interruptible autonomous security worker”dır:

- Yetki kullanıcıdan veya imzalı scope’tan gelir.
- Model yalnızca öneri/plan üretir.
- Policy ve verifier yürütme kararını bağımsız verir.
- Tool çıktıları untrusted data olarak kalır.
- Tüm yan etkiler audit edilir.
- Kullanıcı müdahalesi ve durdurma her zaman mümkündür.

Sonraki araç incelemeleri
-------------------------

[ ] Diğer pentest/agent araçlarını aynı formatta incele.
[ ] Her araç için lisans ve bağımlılık kontrolü yap.
[ ] Alınacak fikri mevcut JARVIS fazına eşleştir.
[ ] Entegrasyondan önce threat model ve rollback planı yaz.
[ ] Kullanıcı onayı olmadan büyük model, dataset veya sistem paketi indirme.

================================================================
ARAÇ 2 — MR.HOLMES
================================================================

Genel amaç
----------

Domain, kullanıcı adı ve telefon numarası üzerinden public-source OSINT yapan; arama/dork,
site araştırması, WHOIS ve bazı port/transfer yardımcıları içeren Python tabanlı araç.

JARVIS’e alınabilecek fikirler
------------------------------

- OSINT görevini ayrı bir read-only capability olarak modellemek.
- Sonuçları kaynak, zaman, güven seviyesi ve tekrar doğrulama zamanı ile saklamak.
- Domain/username/phone pivotlarını tek bir görev grafiğinde ilişkilendirmek.
- Kullanıcıya her dış kaynağın ve olası veri doğruluğu sınırının gösterilmesi.

JARVIS’e doğrudan alınmaması gerekenler
---------------------------------------

- Proxy/anonymity özelliğinin güvenlik veya yetki yerine konması.
- Telefon/kişisel bilgi aramasının varsayılan açık olması.
- Dork ve dış aramaların scope/approval olmadan çalışması.

Lisans notu: Depodaki LICENSE GPLv3’tür. Kodun JARVIS içine gömülmesi copyleft etkisi
doğurabilir; şimdilik yalnızca davranış fikri alınmalı, kod kopyalanmamalıdır.

Hedef faz: F7 scope’lu OSINT; F9 provenance/reporting.

================================================================
ARAÇ 3 — GOCLONE
================================================================

Genel amaç
----------

Bir web sayfasını HTML, CSS, JavaScript, görsel ve bağlantı yapısıyla yerel klasöre mirror
eden Go aracı. İsteğe bağlı proxy, cookie, user-agent ve yerel serve seçenekleri vardır.

JARVIS’e alınabilecek fikirler
------------------------------

- Yetkili web uygulaması için immutable evidence snapshot alma.
- Dinamik test öncesi sayfa/asset setini hash’leyerek provenance oluşturma.
- Snapshot ile gerçek hedef arasındaki farkı raporlama.
- İndirilen içeriği doğrudan modele vermek yerine untrusted attachment/data envelope olarak tutma.

Güvenlik sınırı
---------------

Mirroring geniş veri ve kişisel içerik indirebilir. Maksimum boyut, host allowlist, MIME
allowlist, disk kotası, robots/authorization kararı, retention ve credential redaction zorunlu.
JARVIS bunu “web kopyalama” değil “yetkili evidence snapshot” capability’si olarak adlandırmalı.

Lisans notu: MIT. Kod kopyalanacaksa copyright ve lisans bildirimi korunmalıdır.
Hedef faz: F7 evidence intake; F9 retention/export.

================================================================
ARAÇ 4 — NULLCLOUD
================================================================

Genel amaç
----------

CDN/WAF arkasındaki olası origin IP’lerini ve subdomain ilişkilerini araştıran Python aracı.
Pasif kaynaklar, CNAME/CT kayıtları, HTTP header/fingerprint, favicon, MX/TXT/PTR, AXFR
denemeleri ve bazı aktif ASN/cloud/body modülleri içerir. API anahtarlı FOFA, Shodan,
Censys, Netlas ve benzeri sağlayıcılar desteklenir.

JARVIS’e alınabilecek fikirler
------------------------------

- Passive discovery ile active probing’i kesin biçimde ayırmak.
- Provider, kaynak, zaman ve confidence alanları olan origin-candidate kaydı.
- “Aday origin” ile “doğrulanmış açık” kavramlarını ayırmak.
- JSON/YAML/CSV/Markdown çıktısını ortak Finding/Evidence şemasına dönüştürmek.
- Rate limit, thread, timeout, sample ve memory threshold değerlerini policy’ye bağlamak.

Güvenlik sınırı
---------------

Origin probing ve AXFR/ASN sweep aktif ağ işlemleridir. F7’de yalnız yazılı scope, hedef
allowlist, egress allowlist, rate limit, kill switch ve kullanıcı onayıyla çalışabilir.
API anahtarları prompt, audit veya model context’ine girmemelidir.

Lisans notu: MIT. Hedef faz: F7 passive/active recon; F9 secret ve telemetry yönetimi.

================================================================
ARAÇ 5 — HACKTRICKS
================================================================

Genel amaç
----------

Bir komut çalıştırma aracı değil; web, cloud, Linux, Windows, mobile, AD, API, LLM ve
pentest tekniklerini içeren geniş bir bilgi tabanı ve mdBook arayüzüdür. Yerel Docker/serve,
arama indexi, çoklu dil ve skill benzeri konu başlıkları bulunur.

JARVIS’e alınabilecek fikirler
------------------------------

- F7 için security knowledge pack yapısı.
- Konu, teknoloji, risk, önkoşul, güvenli doğrulama ve remediation metadata’sı.
- Modelin yalnız görev kapsamına uygun bilgi paketini yüklemesi.
- Bilgi ile tool authority’nin tamamen ayrılması.
- Kaynak URL, sürüm, son doğrulama zamanı ve güven notu ile provenance.

Lisans/entegrasyon notu
-----------------------

İçerik CC BY-NC 4.0’tür; ayrıca kitapta dış kaynaklardan alınan bölümler bulunabilir. JARVIS
GitHub’da veya ileride ticari bağlamda dağıtılacaksa HackTricks metnini kopyalamak yerine
yalnızca konu başlıklarını ve kendi özgün notlarımızı kullanmalıyız. Kaynak içerik gerekiyorsa
lisans, attribution ve non-commercial sınırı ayrıca incelenmelidir.

Hedef faz: F3 provenance/RAG; F6 reviewed security dataset; F7 security knowledge packs.

================================================================
ARAÇ 6 — TUNTOOLS
================================================================

Genel amaç
----------

Tek bir araç değil, 22 araçlık karışık bir snapshot’tır. İçinde DNS/privacy, subdomain ve
network discovery, OSINT, WordPress, CSRF/clickjacking, SSRF/LFI/RFI, SQLi/XSS, hash/password,
metadata temizleme ve sqlmap gibi farklı risk seviyelerinde araçlar bulunur.

Önemli alt araç grupları
------------------------

- Recon: `fuckdomain`, `ohsert`, `tunfinder`, `tunmap`, `dnsw`.
- Web checks: `huhwp`, `infbusted`, `tuncsrfjacking`, `tunxss`, `whereissql`.
- Injection/test: `tunsql`, bundled `sqlmap`, `aaslfri`.
- OSINT: `starintel`, `idkmap` ve ilgili yardımcılar.
- Local/privacy utilities: `locktor`, `mdrjohn`, `qwcron`, `hate64`, `passfly`.

JARVIS’e alınabilecek fikirler
------------------------------

- Araç koleksiyonunu capability manifest’leri ile sınıflandırmak.
- Her aracın risk seviyesi, ağ etkisi, credential ihtiyacı ve çıktı formatını tanımlamak.
- Aynı hedef için duplicate scan sonuçlarını normalize etmek.
- Pause/resume, worker pool, timeout ve JSON evidence formatını ortaklaştırmak.

Kesin sınır
-----------

TunTools içindeki SQLi, XSS, SSRF, CSRF, port tarama, DNS değiştirme, Tor ve credential
yardımcıları otomatik olarak JARVIS’e eklenmeyecek. Her araç ayrı threat model, sandbox,
scope ve regression testinden geçmeden registry’ye giremez.

Lisans notu: Koleksiyonun kökü ve alt araçları aynı lisansa sahip değildir. sqlmap GPLv2+
ve bazı alt araçlar ayrı MIT/GPL veya belirsiz/örnek proje lisanslarına sahiptir. Kod
birleştirmeden önce araç bazında license manifest hazırlanmalıdır.

Hedef faz: F4 tool manifest/worker; F7 yetkili security capabilities.

================================================================
ARAÇ 7 — USER-SCANNER
================================================================

Genel amaç
----------

Email ve username OSINT’i için yüzlerce platforma paralel sorgu yapan Python aracı. Metadata
çıkarımı, cross-scan/pivot, alias permutation, proxy validation, JSON/CSV/PDF export ve
harici breach-intelligence sağlayıcısı entegrasyonları sunar.

JARVIS’e alınabilecek fikirler
------------------------------

- İlk kimlikten türeyen pivotları açıkça gösteren investigation graph.
- Her iddianın kaynak URL, fetch zamanı, confidence ve “not found/unknown” ayrımı.
- Paralel sorgular için global rate/concurrency bütçesi.
- Sonuç deduplication ve contradiction handling.
- Kullanıcıya ait veriler için export/delete ve hassasiyet sınıfları.

Güvenlik/gizlilik sınırı
------------------------

Email, username, avatar ve breach verisi kişisel veri olabilir. Infostealer/breach kaynağı
varsayılan kapalı, açık kullanıcı onaylı ve hukuki uygunluk kontrolünden geçmiş olmalı.
Proxy kullanımı yetki vermez; platform rate limitleri ve terms of service korunmalıdır.

Lisans notu: MIT. Harici servislerin kendi kullanım şartları ve API lisansları ayrıca geçerlidir.
Hedef faz: F3 privacy/provenance; F7 authorized OSINT; F9 retention/deletion.

================================================================
ARAÇ 8 — WAYMORE
================================================================

Genel amaç
----------

Wayback Machine, Common Crawl, AlienVault OTX, URLScan, VirusTotal, GhostArchive ve varsa
Intelligence X gibi arşiv/kaynaklardan URL toplar; ayırt edici özelliği arşivlenmiş response’ları
indirip içlerinden ek URL, parametre, yorum ve endpoint çıkarmasıdır.

JARVIS’e alınabilecek fikirler
------------------------------

- Passive historical URL discovery capability.
- URL ile archived response arasındaki provenance bağı.
- URL-only ve response-download modlarını ayrı risk seviyelerine ayırmak.
- Kaynakların rate-limit ve incomplete-result durumunu açıkça raporlamak.
- Tarih, MIME, status, response size, dedup hash ve retention alanları.
- Waymore çıktısını xnLinkFinder benzeri ikinci aşama parser’a güvenli data olarak vermek.

Güvenlik sınırı
---------------

Arşiv response’ları secret, token, kişisel veri veya zararlı içerik barındırabilir. Varsayılan
olarak ham response modele verilmemeli; secret redaction, size limit, MIME allowlist, local
retention ve kullanıcı onayı uygulanmalı. Dış kaynak sorguları yalnız yetkili scope için
çalışmalı ve sonuç “tarihsel veri” olarak etiketlenmelidir.

Lisans notu: MIT. Dış arşivlerin kullanım şartları ve verinin yeniden dağıtım koşulları ayrıca
kontrol edilmelidir. Hedef faz: F7 passive recon; F3 provenance/RAG; F9 retention.

================================================================
ARAÇ 9 — XNLINKFINDER
================================================================

Genel amaç
----------

HTML, JavaScript, HTTP response, directory, Burp/ZAP/Caido export ve HAR girdilerinden URL,
endpoint, parametre, path-word ve potansiyel link çıkaran parser/crawler aracıdır. Scope prefix,
scope filter, cookie/header, user-agent, depth, proxy, response size ve memory threshold seçenekleri
vardır.

JARVIS’e alınabilecek fikirler
------------------------------

- Girdi adaptörleri: raw URL, dosya, HAR, Burp, ZAP ve Caido export.
- Endpoint/parameter çıkarımını ayrı normalize edilmiş asset graph’a yazmak.
- Scope prefix/filter’i parser seviyesinde zorunlu yapmak.
- “Bulundu” ile “erişilebilir/doğrulandı” durumlarını ayırmak.
- Link origin, response hash, content type, depth ve confidence provenance’ı.
- 403/429/timeout oranı yükselince graceful stop ve kullanıcıya incomplete uyarısı.

JARVIS’e doğrudan kopyalanmayacak davranışlar
----------------------------------------------

- TLS doğrulamasını kapatan insecure seçenek varsayılan olamaz.
- Cookie/Authorization header’ları model prompt’una veya rapora sızamaz.
- Sınırsız crawl depth, response boyutu veya memory kullanımı kabul edilmez.
- Scope dışı linkler yalnız gözlem olarak kalmalı; istek atılmamalı.

Lisans notu: MIT. Hedef faz: F7 passive recon/parser worker; F9 evidence graph.

================================================================
ORTAK ENTEGRASYON TASARIMI
================================================================

Bu araçlardan alınan hiçbir özellik doğrudan “model istedi, çalıştı” şeklinde bağlanmayacak.
Önerilen akış:

User request
  -> intent proposal
  -> target/scope resolver
  -> policy + risk classification
  -> approval (gerekliyse)
  -> isolated worker + resource quota
  -> normalized evidence
  -> verifier / confidence
  -> finding or observation
  -> audit + report

İlk entegrasyon sırası
----------------------

1. HackTricks’ten özgün ve lisans uyumlu security knowledge-pack formatı.
2. xnLinkFinder/waymore fikirleriyle passive URL/endpoint evidence graph.
3. NullCloud ve ohsert benzeri passive asset/origin candidate kayıtları.
4. F4 worker tamamlandıktan sonra TunTools içinden düşük riskli read-only parser’lar.
5. F7’de scope’lu active recon ve doğrulanmış PoC capability’leri.
6. User-scanner/Mr.Holmes için privacy, consent ve legal boundary çalışması.

Kapanış kriteri
---------------

Her araç entegrasyonu için:

[ ] Lisans ve bağımlılık manifesti.
[ ] Threat model ve abuse-case listesi.
[ ] Capability manifest ve risk seviyesi.
[ ] Scope/approval/cancellation testi.
[ ] Resource ve network limit testi.
[ ] Untrusted output/provenance testi.
[ ] Başarı, ret ve timeout smoke testleri.
[ ] Rollback ve disable planı.
[ ] Kullanıcıya görünür rapor ve audit kanıtı.

================================================================
ARAÇ 10 — DIKTE
================================================================

Genel amaç
----------

Dikte, Wayland/KDE Plasma merkezli yerel-first sesli yazma ve sesli komut aracıdır.
Global kısayolla kayıt başlatır/durdurur, whisper.cpp ile konuşmayı yazıya çevirir,
llama.cpp veya seçilen sağlayıcıyla transkripti temizler ve sonucu panoya/odaklanan
pencereye yapıştırır. Toplantı kaydı, dosya transkripsiyonu, altyazı, geçmiş ve ayar
arayüzü de bulunur.

Öne çıkan özellikler
--------------------

- Ctrl+Space ile kayıt başlatma/durdurma.
- Yerel whisper.cpp ve llama.cpp sunucuları.
- Model/program indirmesinde SHA-256 doğrulaması.
- Sessizlik ve gürültü tabanı algılama; boş kaydı modele göndermeme.
- Ham transkript başarısız cleanup durumunda kaybolmuyor.
- Türkçe/İngilizce arayüz.
- Sesli komutu Claude Code, Codex veya OpenRouter’a yönlendirme.
- Toplantıda mikrofon ve hoparlör kanallarını ayrı transkribe etme.
- Karar, aksiyon ve açık sorulardan toplantı tutanağı çıkarma.
- Audio/video dosyasından TXT veya SRT üretme.
- Yerel IPC, CLI ve JSON çıktısı.
- Wayland clipboard/paste, overlay ve tray UI.

JARVIS’e alınabilecek fikirler
------------------------------

1. Push-to-talk / dikte UX’i
   F5 için Ctrl+Space veya kullanıcı tanımlı tuşla kayıt başlatma; gönderimden önce
   transkripti düzenleme, iptal etme ve yeniden deneme.

2. Ayrı ses pipeline’ı
   Capture -> VAD -> STT -> transcript review -> normal JARVIS InputType::Voice
   akışı. Ham ses varsayılan olarak kalıcı tutulmamalı.

3. Yerel-first model lifecycle
   Whisper ve cleanup modeli RAM’de warm tutma, ilk yükleme ile sonraki gecikmeyi
   ayırma, model hash/lisans/disk/RAM etkisini gösterme.

4. Sessizlik/gürültü koruması
   Fan sesi veya sessiz kayıtta modelin hayali cümle üretmesini engellemek için
   recording-specific noise floor ve minimum speech threshold.

5. Transcript provenance
   Her ses girdisi için session/task ID, dil, model hash, zaman, confidence ve
   kullanıcı düzeltmesi tutulabilir. Ham ses ile düzenlenmiş metin ayrı saklanmalı.

6. Toplantı/çoklu kanal modeli
   İleride microphone/system-audio kanallarını izinli şekilde ayırıp timestamp’li
   transcript ve minutes üretebiliriz. Bu F5 kapsamıdır; kalıcı recording opt-in olmalı.

7. Sesli agent komutu
   Dikte yalnızca normal kullanıcı mesajı üretir; tool/agent komutu yine JARVIS’in
   model tabanlı routing, policy, approval ve verifier zincirinden geçer.

JARVIS’e doğrudan alınmayacaklar
-------------------------------

- Claude/Codex CLI’ye doğrudan yetki verip JARVIS policy’sini bypass etmek.
- `/dev/input`, ydotool veya clipboard erişimini approval’sız genişletmek.
- Ham mikrofon/toplantı kayıtlarını varsayılan olarak kalıcı tutmak.
- SHA-256’sız model veya binary indirmek.
- Sesli komutu otomatik olarak yüksek riskli tool çağrısına çevirmek.
- Overlay/XWayland davranışını masaüstü güvenlik sınırlarının yerine koymak.

Lisans ve bağımlılık notu
------------------------

Dikte GPLv3 lisanslıdır. Kodunu JARVIS’e gömmek copyleft ve dağıtım yükümlülükleri
doğurabilir; bu nedenle ilk yaklaşım davranış/contract esinlenmesi ve ayrı process
veya adapter sınırı olmalıdır. PipeWire, wl-clipboard, ydotool, ffmpeg, PyQt6,
whisper.cpp ve llama.cpp gibi bileşenlerin lisansları ayrıca manifestlenmelidir.

Hedef faz: F5 push-to-talk ve çoklu algı; F6 model benchmark/voice dataset; F9
model lifecycle, privacy, retention ve release operasyonu.

Önerilen JARVIS ses akışı
-------------------------

Kullanıcı tuşa basar
  -> local audio capture
  -> VAD / silence gate
  -> local STT
  -> düzenlenebilir transcript
  -> kullanıcı gönderir veya iptal eder
  -> InputType::Voice
  -> model conversation/routing
  -> policy + approval + tool/verifier
  -> metin ve isteğe bağlı TTS çıktısı

Dikte’den alınacak en önemli fikir: ses girişini doğrudan eyleme bağlamamak;
önce transkript, sonra kullanıcı kontrolü, sonra JARVIS’in governed pipeline’ı.

================================================================
ARAÇ 11 — PENTESTERFLOW
================================================================

Kaynak klasör:
/home/mehmet/Masaüstü/pentesterflow/agent

Genel amaç
----------

PentesterFlow, pentest uzmanını süreç boyunca kontrol sahibi tutan human-in-the-loop
agentic AI CLI’dır. Scope -> recon -> enumeration -> validation -> coverage -> reporting
-> learning akışını tek terminal arayüzünde toplar. Yerel veya hosted LLM, shell/HTTP/file,
browser capture, Burp, MCP ve Markdown skill’leriyle çalışır.

Öne çıkan mimari
----------------

- Agent loop TUI’dan ayrıdır; Ink/React yalnız event/permission bridge olarak çalışır.
- LLM provider factory: Ollama, LM Studio, OpenAI-compatible, Kimi, Groq, Gemini,
  OpenRouter ve DeepSeek gibi backend’ler.
- Tool Registry: shell, HTTP, file, glob/grep, browser, MCP, coverage ve finding tool’ları.
- Permission bridge: allow-once, allow-session, deny ve açık kullanıcı onayı.
- Scope/target state; hedef, yetki notu ve kapsam bilgisi agent context’inden ayrılmaz.
- Skill registry: Markdown playbook’ları yükler, tool izinlerini skill’e göre sınırlar.
- Session persistence, compaction, snapshot, resume ve startup recap.
- Project/personal intelligence store; lessons, successful workflows, failed assumptions
  ve coverage gaps kaydedilir.
- Coverage matrix: endpoint + parameter + vulnerability class tuple’larını izler.
- `confirm_finding`: yalnız reproduction evidence ile confirmed finding yazılması beklenir.
- Burp ingest bridge, browser capture store ve MCP server.
- JSON-lines audit/log, redaction, deterministic local artifact paths.
- Model/tool budget, context limit, retry ve abort signal yönetimi.

JARVIS’e alınabilecek fikirler
------------------------------

1. Human-in-the-loop pentest state machine
   JARVIS’in plan-act-observe-verify-report-learn döngüsünü açık task state’leriyle
   göstermek. Kullanıcı çalışan plana müdahale edebilmeli; pause, resume, narrow-scope,
   cancel ve re-plan birinci sınıf işlemler olmalı.

2. Coverage tracking
   F7’de `(target, endpoint, parameter, vulnerability_class)` matrisi tutmak. `/next`
   benzeri öneri yalnız kapsam içinde ve daha önce test edilmemiş işleri önermeli.

3. Evidence-bound finding
   Finding oluşturma capability’si verifier evidence olmadan çalışmamalı. Request,
   response, reproduction steps, confidence, impact, remediation ve audit ID birlikte
   tutulmalı. “Model şüphelendi” ile “doğrulandı” ayrı durumlar olmalı.

4. Session resume ve context snapshots
   Uzun güvenlik görevlerinde session, scope, findings, coverage ve model/prompt sürümü
   kaydedilmeli. Snapshot’lar redacted, bounded ve kullanıcıya görünür olmalı.

5. Project/personal intelligence ayrımı
   F3 memory tasarımında proje hafızası ile kullanıcının genel öğrenilmiş tercihleri
   ayrılmalı. Duplicate, TTL, sensitivity, provenance ve forget işlemleri zorunlu.
   JARVIS modeli retrain etmeden “operational learning” yapabilir.

6. Skill tabanlı çalışma paketleri
   Recon, webvuln, SSRF, JWT, GraphQL, race ve deserialization gibi metodolojiler
   ayrı skill paketleri olabilir. Skill yalnız bilgi ve izin bağlamı sağlar; tool
   authority yine JARVIS policy/registry’den gelir.

7. Burp/browser/MCP capture entegrasyonu
   Seçili HTTP request’leri import edip endpoint/parameter evidence graph’ına bağlamak.
   Browser capture varsayılan kapalı, oturumluk opt-in, bounded ve token’lı olmalı.

8. Provider abstraction
   Model adapter’ını core’dan ayırma fikri bizim model değiştirilebilir mimarimizi
   destekliyor. Qwen’den ileride başka modele geçerken policy, memory, audit ve tool
   zinciri sabit kalabilir.

9. Reproducible command UX
   Tool çağrıları görünür, tekrar çalıştırılabilir ve finding içine kanıt olarak
   eklenebilir. Ancak komutlar çalışmadan önce risk sınıfı ve approval görünmeli.

10. Redaction ve bounded persistence
    Secret, Bearer, cookie, JWT, URL userinfo, credential ve hassas target verisi
    compaction/snapshot/learning’e gitmeden redakte edilmeli. Log, capture, findings,
    memory ve MCP çıktıları boyut/retention limitlerine sahip olmalı.

JARVIS’e doğrudan alınmayacaklar
-------------------------------

- `--yolo` veya `--dangerously-skip-permissions` benzeri sınırsız mod.
- Prompt/skill metninin gerçek policy olarak kabul edilmesi.
- Shell, HTTP veya MCP tool’larının host üzerinde approval’sız çalışması.
- Browser/Burp capture’ın varsayılan kalıcı veya sınırsız olması.
- “Confirmed finding” kararını yalnızca model çıktısına bırakmak.
- Target verisiyle kullanıcının kendi local secret’larını aynı trust alanında saklamak.
- Birinci aşamada tüm dış LLM provider’larını indirmek veya bağlamak.

PentesterFlow denetiminden alınan dersler
-----------------------------------------

Projenin AUDIT.md dosyası, security agent altyapısında güvenlik özelliklerinin kendisinin
de test edilmesi gerektiğini gösteriyor. Özellikle şu sınıflar JARVIS için regression konusu
olmalı:

- DNS rebinding ve resolve/connect TOCTOU.
- Symlink ve realpath tabanlı sensitive-path bypass.
- Tool output/MCP sonucu için gerçek transport-level size limit.
- Capture store, endpoint/parameter map ve intelligence için OOM sınırı.
- Aborted tool-call session’larının bozuk history bırakmaması.
- Streaming tool-call parçalarının doğru birleştirilmesi.
- URL userinfo, JWT ve Authorization secret redaction.
- Terminal escape/control-byte temizliği.
- Atomic finding creation ve concurrent append güvenliği.

Not: Bu maddeler PentesterFlow’dan kod kopyalama amacıyla değil, JARVIS’in F4/F7 worker,
MCP, memory ve audit test setini genişletmek amacıyla kaydedildi.

Lisans notu
----------

PentesterFlow Apache-2.0 lisanslıdır. Kaynak koddan fikir almak ve uygun lisans notices’larını
koruyarak kod kullanmak mümkün olabilir; yine de bağımlılıkların lisansları ayrıca kontrol
edilmelidir. İlk yaklaşım kod kopyalamak değil, JARVIS’in Rust core contract’larına uygun
özgün yeniden uygulamadır.

JARVIS faz eşlemesi
-------------------

PentesterFlow fikri             JARVIS hedefi                  Faz
-----------------------------  ------------------------------  --------
Permission bridge               Approval/policy UX              F0-F2
Session/resume/snapshot         Persistence + audit              F3/F9
Intelligence store              Controlled memory/RAG             F3
Coverage matrix                 Security task state              F7
Skill registry                  Knowledge/provenance packs        F3/F7
Browser/Burp/MCP capture        Evidence ingress                 F7/F8
Confirmed finding               Evidence verifier/reporting       F7
Provider factory                Model adapter/benchmark           F6
Redaction/bounded stores        Operational security              F9

Entegrasyon sırası
------------------

[ ] F3’te project memory ve personal memory ayrımını bu modelle netleştir.
[ ] F4’te pause/resume/cancel, bounded worker ve snapshot contract’larını uygula.
[ ] F6’da provider/model benchmark ve prompt/model version kaydını ekle.
[ ] F7’de coverage matrix, scope-aware task DAG ve evidence-bound finding geliştir.
[ ] F8’de opt-in Burp/browser/MCP capture adapter’ı ekle.
[ ] F9’da redaction, retention, atomic persistence ve audit stress testlerini tamamla.

