# ADR-0005: Açılış karşılaması, elle düzenlenen profil dosyaları ve JARVIS'in ilk gerçek internet erişimi (hava durumu)

Durum: Kabul edildi — 16 Ağustos 2026

## Bağlam

Kullanıcı iki ilişkili istek iletti: (1) JARVIS her açıldığında yazılı (ileride sesli — bkz. aşağıdaki
"Sesli/TTS" bölümü) bir karşılama istiyor: isim + hava durumu + dünden kalan notlar; (2) kendisi ve
JARVIS hakkında, her başlangıçta yeniden okunacak, elle düzenlenen profil dosyaları oluşturmayı
düşünüyor ("böylece hata olmaz").

Bu, projede ilk kez **gerçek internet erişimi** (hava durumu) gerektiren bir karar noktası açtı —
F0'dan beri JARVIS'in tüm capability'leri (`CapabilityRegistry`) ya tamamen local ya da loopback-only
(local model servisleri) idi; F3'ün `no_baseline_capability_requires_network_access` testi bunu
doğruluyor.

## Karar 1 — Profil dosyaları: ikinci, ayrı bir mekanizma

Kullanıcı iki tasarım arasında seçim yaptı: "veritabanının bir görünümü/export'u" ya da "benim
(Claude'un) kendi hafıza dosyalarım gibi, elle düzenlenen talimat dosyaları". Kullanıcı **ikincisini**
seçti.

- `~/.config/jarvis/profile/about_user.md` ve `about_jarvis.md` — düz Markdown, kullanıcının kendi
  editörüyle açıp düzenlediği dosyalar.
- **JARVIS bu dosyalara asla yazmaz**, yalnız okur (`ensure_profile_files_exist` yalnız dosya yoksa
  bir şablonla oluşturur, var olan içeriği asla ezmez — test: `ensure_profile_files_exist_creates_
  templates_but_never_overwrites_real_content`).
- Kasıtlı olarak `MemoryRecord`/`propose_memory` sisteminden **ayrı**: bellek kayıtları JARVIS'in
  kendi komutlarıyla (onay adımından geçerek) yazılır; bu dosyalar tamamen kullanıcının elinde,
  hiçbir onay akışından geçmez çünkü zaten JARVIS'in kendisi hiç yazmıyor.
- Her turda taze okunur (önbelleklenmez) — kullanıcı JARVIS çalışırken dosyayı değiştirirse bir
  sonraki turda hemen yansır.
- `MAX_PROFILE_FILE_CHARS = 8_000` — bağlam bütçesini tek başına tüketmesin diye üst sınır.
- **Aynı "veri, talimat değil" ilkesi geçerli** (`isolate_profile_file_as_data`,
  `<profile-file label="...">...</profile-file>` zarfı) — [ADR-0003](0003-user-profile-schema.md)'ün
  bellek kayıtları için kurduğu ilkenin birebir aynısı: kullanıcı bu dosyaları kendi yazmış olsa bile,
  model buradan hiçbir zaman tool/policy yetkisi kazanmaz.

## Karar 2 — Hava durumu: Open-Meteo, sabit İstanbul/Ümraniye konumu, governed pipeline dışında

**Değerlendirilen alternatifler:**
1. Genel bir web araması — reddedildi: JARVIS'in hiçbir genel web-arama capability'si yok, bunu bu
   özellik için icat etmek kapsam dışı bir iş olurdu; hava durumu gibi yapılandırılmış bir veri için
   doğrudan bir hava durumu API'si daha basit ve güvenilir.
2. AccuWeather — kullanıcının ilk önerisi; araştırıldıktan sonra geliştirici API'sinin ücretli/kayıt
   gerektirdiği görülünce kullanıcı kendisi vazgeçti.
3. **Open-Meteo (open-meteo.com)** — **seçildi**. Ücretsiz, API anahtarı/kayıt gerektirmiyor, basit
   REST/JSON arayüzü.

**Konum:** Kullanıcının açık talimatıyla sabit — İstanbul, Ümraniye (enlem/boylam
`OpenMeteoWeatherProvider::istanbul_umraniye()` içinde sabit kodlanmış). Konum değişirse bu
constructor güncellenir ya da `OpenMeteoWeatherProvider { .. }` doğrudan farklı koordinatlarla
kurulur.

**Mimari sınır — bu bir capability DEĞİL:** `WeatherProvider`/`OpenMeteoWeatherProvider`
`CapabilityRegistry`'ye hiç kaydedilmedi ve governed pipeline'ın (intent→policy→task→tool→verifier→
audit) hiçbir adımından geçmiyor. Yalnız `Runtime::startup_briefing()` — kullanıcı JARVIS'i her
açtığında bir kez — bu sağlayıcıyı okuyor. **Model bunu hiçbir zaman "çağıramaz."** Bu, F3'ün
`no_baseline_capability_requires_network_access` testinin hâlâ doğru kalmasını sağlıyor — o test
yalnız `CapabilityRegistry` kayıtlarını kapsıyor, hava durumu hiçbir zaman bir kayıt olmadı.

**Hata toleransı:** `current_weather()` başarısız olursa (ağ yok, servis kapalı, timeout — 5 sn)
`startup_briefing()` o satırı sessizce atlar; JARVIS'in açılışı bu yüzden hiçbir zaman engellenmez
ya da hataya düşmez.

**Yeni bağımlılık:** `ureq` (`tls`+`json` özellikleriyle, rustls tabanlı — OpenSSL yok) — projenin
ilk HTTP client bağımlılığı. Diğer her şey (text/vision/embedding local servisleri) daha önce elle
yazılmış ham TCP/HTTP kullanıyordu; hava durumu için harici, gerçek bir HTTPS istemcisi gerekiyordu.

**Gerçek doğrulama:** `curl` ile gerçek Open-Meteo uç noktası (`41.0166,29.1173`) canlı olarak
sorgulandı, gerçek güncel hava verisi (sıcaklık + WMO kodu) döndüğü doğrulandı. JSON ayrıştırma
mantığı (`parse_open_meteo_response`) ayrıca ağdan bağımsız, saf fonksiyon olarak birim test
edildi (`cargo test --offline` bu yüzden asla ağ gerektirmiyor).

## Karar 3 — Açılış karşılaması bileşimi

`Runtime::startup_briefing() -> String`, tamamen zaten yerelde var olan verilerle çalışır (hava
durumu dışında hiçbir yeni veri kaynağı gerektirmez, o da isteğe bağlıdır):

1. Selamlama — profildeki `preferred_address`/`display_name` varsa kullanılır, yoksa genel "Hoş
   geldiniz."
2. Hava durumu — sağlayıcı bağlıysa ve fetch başarılıysa (yoksa satır hiç görünmez).
3. Bekleyen onay sayısı.
4. Son güncellenen en fazla 3 not (yalnız `Project`/`UserProfile` namespace'i, secret placeholder'ları
   (`source == "secret-manager"`) hariç — gerçek bir sır asla karşılamada görünmez).

TUI'de `App`'in açılış mesajına ikinci bir sistem mesajı olarak eklenir (`run_tui`); native
masaüstünde `JarvisDesktop::new`'in ikinci mesajı olarak eklenir.

## Sesli/TTS — bilinçli olarak yapılmadı

Kullanıcı karşılamanın "sesli ve yazılı" olmasını istediğini belirtti, ama açıkça şunu da ekledi:
"sesli yapmadık farkındayım ama şimdiden söyleyeyim" — yani bu, F5'in kapsamına giren bir **gelecek
niyeti**, bugünün işine dahil bir istek değil. Bu ADR ve bugünkü uygulama yalnız **yazılı** karşılamayı
kapsıyor. TTS/STT [F5 — Sesli etkileşim ve algı arayüzü](../../DEVELOPMENT_PLAN.md)'te ayrıca ele
alınacak.

## Sonuçlar

- JARVIS artık her açılışta kişiselleştirilmiş, bilgilendirici bir karşılama gösteriyor.
- Profil dosyaları, bellek sisteminin CRUD/onay disipliniyle karışmadan, kullanıcının serbestçe
  düzenleyebileceği ek bir bağlam kanalı sağlıyor.
- Hava durumu, JARVIS'in "governed pipeline dışı ama zararsız" bir istisna sınıfını (yalnız
  başlangıçta, yalnız okuma, model asla çağıramaz) ilk kez somutlaştırdı — ileride benzer
  "bir kerelik, model-invoke edilemeyen, salt-okunur" ek özellikler için bir emsal oluşturuyor.
- `ureq` bağımlılığı yalnız bu dar kullanım için var; genel bir HTTP client altyapısı **değil** —
  gelecekte model'in doğrudan web erişimi kazanması ayrı, kendi ADR'sini gerektiren bir karar olur.
