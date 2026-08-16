# JARVIS Desktop MVP

Linux-first, local-first JARVIS core başlangıcı. `jarvis`, satır satır çalışan eski kabuk yerine terminal içinde ayrı mesaj geçmişi ve metin uzadıkça büyüyen bir yazma alanı olan bir sohbet ekranı açar. Yanıtlar değiştirilemez geçmişte kalır; yazı yalnız alttaki **Mesaj** kutusuna gider.

## Çalıştırma

```bash
jarvis
```

Geliştirme sırasında:

```bash
cd /home/mehmet/jarvis/jarvis
cargo run --offline
```

Komut hangi klasörden çalıştırılırsa çalıştırılsın bundled model ve runtime kurulum kökünden bulunur. İlk açılışta model sunucusu aktif değilse JARVIS onu otomatik başlatır; model RAM'e yüklenirken ekran açık kalır ve mesaj kutusu korunur.

## Sohbet ekranı

- Mesaj gönder: `Enter`
- Metin kısayolları: `Ctrl+V` normal panoyu, mouse orta tuşu Wayland primary selection'ı yapıştırır; `Ctrl+Backspace` veya `Ctrl+W` önceki kelimeyi siler; `Ctrl+U` ve `Esc` taslağı temizler.
- Uzun taslak: giriş kutusu tek satırdan başlayıp yazdıkça yukarı doğru büyür; ekran yüksekliğine göre sınıra ulaşınca en yeni bölüm görünür kalır.
- Geçmişte gezin: `↑` / `↓`, `PageUp` / `PageDown` veya mouse tekerleği; geçmiş taşınca sağda ayrı bir scrollbar ve `↑↓ kaydır` başlığı görünür. Uzun mesajlar geçmişte tam olarak tutulur; görünmeyen bölümler bu yolla okunur.
- Yanıt bildirimi: JARVIS bir yanıtı tamamladığında Hyprland bildirim alanında kısa bir önizleme gösterir.
- Kısayollar: `/help`, `/status`, `/clear`, `/approvals`
- Tek bir işlem onay bekliyorsa `/approve` veya `/cancel`; birden fazla varsa `/approve <task-id>` / `/cancel <task-id>`
- `/quit` veya `Ctrl+C`: yalnız sohbet ekranını kapatır; model arka planda RAM'de kalır.
- `exit`: sohbet ekranını kapatır; açık olan text ve vision model sunucularını durdurur, RAM boşalır.

Hyprland'de terminali `Super + Q` ile kapatmak da sadece arayüzü sonlandırır; model sunucusu çalışmaya devam eder. Sonraki `jarvis` açılışı model hâlâ RAM'deyse doğrudan kullanır, değilse otomatik başlatır.

## Native masaüstü penceresi (F2 önizleme)

Terminal arayüzü günlük kullanım için korunur. Aynı governed core'a bağlı, salt-okunur mesaj kartları ve ayrı composer kullanan native Wayland penceresi ise şu an F2 doğrulama aşamasındadır:

```bash
cd /home/mehmet/jarvis/jarvis
cargo run --offline --bin jarvis-desktop
```

Release derlemesi güncelse aynı pencereyi doğrudan şununla da açabilirsin:

```bash
jarvis --desktop
```

- Arayüz, eski JARVIS tasarımındaki teal/siyah HUD dilini native olarak uygular: merkezde durum orb'u, solda sistem/ayar/onay paneli, sağda salt-okunur sohbet konsolu ve bağımsız composer bulunur. Pencereyi `Super + Q` ile kapatmak model servisini durdurmaz; soldaki **Modeli RAM'den çıkar** düğmesi bunu açıkça yapar.
- Birden fazla native pencere açılmaz; kapanmamış/eski kilit güvenle temizlenir ve mevcut pencere kullanılmaya devam edilir.
- Mesaj kartları düzenlenemez. Ortadaki arama alanı mevcut oturumda Türkçe büyük/küçük harf farkını gözetmeden arar; **Sen / JARVIS / Sistem** filtresiyle daraltılabilir.
- Onay isteyen işler soldaki panelde task ID ile görünür; **Onayla** yalnız o task'ı core approval zincirinden geçirir, **Reddet** yan etkiyi çalıştırmadan iptal eder.
- Tema, yazı ölçeği ve bildirim tercihi yalnız `~/.config/jarvis/desktop.json` içinde tutulur; ekrandan varsayılanlara döndürülebilir veya kullanıcı seçtiği konuma dışa aktarılabilir. Bildirim tercihi açıksa yanıt, onay bekleme ve işlem hatası bildirilir. Action destekleyen notification daemon + Hyprland v0.55+ ortamında **JARVIS'i aç** seçeneği yalnız kendi native penceresine focus ister; destek yoksa bildirim best-effort kalır. Sohbet içeriği, ek dosya yolu veya kimlik bilgisi bu dosyaya yazılmaz.
- `Ctrl+O` veya **Dosya ekle** ile PNG/JPEG/TXT/Markdown/PDF seçilir; önizleme ve kaldırma işlemi dosyayı silmez. PNG/JPEG gönderildiğinde piksel baytları yalnız ayrı local vision sunucusuna gider; normal chat modeline yerel yol veya ham piksel verilmez. Vision betimlemesi normal modele escaped, güvenilmeyen veri olarak iletilir. TXT/Markdown/PDF içeriği ise ayrı RAG onayı verilene kadar yalnız doğrulanmış metadata olarak kalır.
- Gönderimden sonra **Oturum ek makbuzları** bölümünde yalnız dosya adı, MIME, boyut/dimension, SHA-256 ve attachment ID görünür. Bu liste 50 kayıtla sınırlı ve geçicidir; tekli/tümü temizlenebilir veya kullanıcı seçtiği JSON konumuna metadata olarak dışa aktarılabilir. Yerel yol, ham dosya, prompt ve model yanıtı export'a girmez. TUI eşdeğerleri: `/attachment-history`, `/attachment-history remove <id>|clear`, `/attachment-export <dosya-yolu>`.

## Local model çalışma profili

Model, loopback üzerindeki kalıcı `llama-server` servisinde çalışır:

```text
Qwen3-8B-Q4_K_M.gguf
CPU/RAM only: -ngl 0
VRAM layer: 0
Context window: 2048 tokens
Normal chat response budget: up to 256 tokens, with one bounded automatic continuation if the local server reports a generation limit
```

Bu servis yalnız `127.0.0.1:8088` üzerinde dinler. İlk mesajdaki model yükleme maliyeti ortadan kalkar; normal yanıtta yalnız token üretim süresi kalır. Sunucuyu elle kontrol etmek gerekirse:

```bash
systemctl --user status jarvis-llama.service
systemctl --user stop jarvis-llama.service
systemctl --user start jarvis-llama.service
```

## Local vision çalışma profili

Görsel ekler için text modelinden ayrı bir Qwen2.5-VL 3B GGUF sunucusu kullanılır. Servis yalnız
ilk PNG/JPEG isteğinde başlatılır; text-only sohbetlerde RAM tüketmez.

```text
Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf + eşleşen mmproj
CPU/RAM only: -ngl 0
VRAM layer: 0
Endpoint: 127.0.0.1:8089 (loopback only)
Vision observation budget: 96 tokens
```

Kurulum deposundaki unit ile yapılır ve uygulama ilk görsel isteğinde otomatik başlatır:

```bash
bash scripts/install_vision_service.sh
systemctl --user status jarvis-vision.service
```

Vision çıktısı nihai yanıt değildir; ayrı modelin kısa, güvenilmeyen gözlemidir. JARVIS normal
sohbet modeli bunu bağlam verisi olarak kullanır. İlk farklı görsel CPU'da daha yavaş olabilir;
ayrıntılı smoke ve ölçüm kaydı [f2_vision_smoke_2026-08-14.md](docs/f2_vision_smoke_2026-08-14.md)
içindedir.

Modelin tool veya policy yetkisi yoktur. Doğal sohbet modelden gelir; geçmiş, modele gerçek `user`/`assistant` rolleriyle ve tam konuşma çiftleri halinde iletilir (artık diske de yazılır, bkz. aşağıdaki "Bellek, profil ve RAG" bölümü). Son kullanıcı mesajı önceliklidir; kısa takip soruları yakın turlardan bağlam alır fakat önceki yanıtı gereksizce tekrar etmez. Kişisel bilgi veya tercih uygulama koduna gömülmez; bunun için kalıcı, kullanıcı onaylı bir profil/bellek katmanı kurulu. İzin gerektiren dosya değişiklikleri Policy Gate ve task-bound approval akışından geçer. Başka bir çalışma alanını okumak için `JARVIS_WORKSPACE_ROOT=/path/to/workspace` ayarlanabilir.

## Bellek, profil ve RAG

JARVIS kalıcı, denetlenebilir bir hafızaya ve belge tabanlı bilgiye (RAG) sahip. Hiçbir kayıt model
kendi kendine karar vererek yazılmaz — yalnız açık kullanıcı komutuyla (slash komutu veya doğal dil
tetikleyicisiyle); model normal sohbet üretirken bu yollara hiç erişemez.

**Bellek (kalıcı, `jarvis.db` içinde):**
- `/remember anahtar = değer` → önizleme → `/remember approve` (ya da `reject`)
- Doğal dille tek adımda: "hafızana yaz: adım Ali", "hafızandan isim bilgimi sil" — ikinci bir onay
  adımı gerekmez, cümlenin kendisi zaten açık komuttur. Aynı anahtarı tekrar yazmak günceller,
  ikinci bir kayıt oluşturmaz.
- `/memory` (listele), `/forget <id>|all`, `/forget namespace <profil|proje|görev|oturum|geçici>`
- `/memory export <dosya-yolu>` / `/memory import <dosya-yolu>`
- Profil (ad/hitap/dil/rol): `/profile`, `/profile set <alan> = <değer>`, `/profile delete <alan>`,
  `/profile reset`, `/profile export <dosya-yolu>`

**RAG (belge indeksleme ve kaynaklı cevap):**
- `/index <proje-içi-göreli-dosya> [public|internal|sensitive]`
- `/index-preview <klasör> [hariç-desen ...]` / `/index-folder <klasör> [hariç-desen ...] [public|internal|sensitive]`
- `/rag status` (belge/chunk/embedding sayısı, hibrit mi FTS-only mi), `/rag rebuild`, `/rag verify`
- Bir yanıt belge kaynaklıysa altında `[n] dosya#chunk — "kısa alıntı"` görünür; `/source <n>` tam
  metni açar. `sensitive` işaretli belgeler indekslenir ama otomatik alıntı olarak asla çıkmaz.
- Arama, kelime eşleşmesini (FTS) isteğe bağlı yerel embedding modelinin (bağlıysa) anlam
  eşleşmesiyle birleştirir (Reciprocal Rank Fusion) — embedding servisi kapalıyken bile FTS çalışır.

**Sohbet geçmişi:** diske de yazılır (yalnız RAM değil) — JARVIS yeniden başlatılınca kaldığı yerden
devam eder. `/clear` hem görünen listeyi hem modele giden bağlamı hem diskteki kaydı siler.

**Sır (Secret Manager):** `/secret anahtar = değer`, `/secret show <anahtar>`, `/secret forget <anahtar>`,
`/secrets` (yalnız anahtarları listeler). Gerçek değer sıradan bellekten tamamen ayrı bir tabloda
tutulur; normal sohbet bağlamına asla girmez, yalnız `/secret show` ile açıkça istenince görünür.

**Açılış karşılaması:** JARVIS her açıldığında (TUI ve native masaüstü) isim (varsa) + güncel hava
durumu (İstanbul/Ümraniye, isteğe bağlı) + bekleyen onay sayısı + son notlarla kişiselleştirilmiş,
yazılı bir karşılama gösterir (`Runtime::startup_briefing`). Sesli karşılama henüz yok, F5'in kapsamında.

**Profil dosyaları (elle düzenlenen):** `~/.config/jarvis/profile/about_user.md` ve `about_jarvis.md`
— kullanıcının kendi editörüyle doğrudan düzenlediği, JARVIS'in yalnız okuduğu (asla yazmadığı) serbest
metin dosyaları; her turda taze okunur.

Detaylı karar/gerekçe: [ADR-0003](docs/adr/0003-user-profile-schema.md) (bellek/profil şeması),
[ADR-0004](docs/adr/0004-hybrid-rag-embedding.md) (hibrit RAG/embedding),
[ADR-0005](docs/adr/0005-startup-briefing-profile-files-and-weather.md) (açılış karşılaması, profil
dosyaları, hava durumu).

## İlk desteklenen governed istekler

```text
sistem durumu nedir
saat kaç
dosya oku: Cargo.toml
proje bilgisi
kod projesi özeti
doküman özeti
not oluştur: yarın markete git
```

Kalıcı not işlemleri kullanıcı onayı bekler. Notlar `notes/` altında oluşturulur. Task ve audit kayıtları `jarvis.db` içinde tutulur. `dosya oku` yalnız çalışma dizini içindeki en fazla 64 KiB normal UTF-8 dosyaları okur; path traversal reddedilir.

## Doğrulama

```bash
bash scripts/release_check.sh
# Model servisinin yalnız loopback health kontrolünü de eklemek için:
bash scripts/release_check.sh --with-service
# Text ve kurulu vision servislerinin ikisini de kontrol etmek için:
bash scripts/release_check.sh --with-vision
cargo run --offline --bin router_benchmark
```

`router_benchmark`, local modelin dar desktop routing görevlerindeki ilk baseline ölçümüdür; genel model kalitesi veya security/coding benchmarkı değildir.

`release_check.sh`, normal koşumda geçici bir SQLite dosyasıyla MCP `system.health` PASS ve bilinmeyen tool DENY smoke'unu da doğrular; kalıcı `jarvis.db` veya model servisini değiştirmez.

## MCP stdio

İlk MCP transportu stdio üzerinden JSON-RPC mesajları alır; tüm tool çağrıları aynı registry ve Policy Gate'ten geçer.

```bash
cargo run --bin mcp_stdio
```

Desteklenen methodlar: `initialize`, `tools/list`, `tools/call`. Registry dışı tool adları execution'a dönüşmez.
