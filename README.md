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
- Metin kısayolları: `Ctrl+V` yapıştır; `Ctrl+Backspace` veya `Ctrl+W` önceki kelimeyi sil; `Ctrl+U` ve `Esc` taslağı temizler.
- Uzun taslak: giriş kutusu tek satırdan başlayıp yazdıkça yukarı doğru büyür; ekran yüksekliğine göre sınıra ulaşınca en yeni bölüm görünür kalır.
- Geçmişte gezin: `↑` / `↓`, `PageUp` / `PageDown` veya mouse tekerleği; geçmiş taşınca sağda ayrı bir scrollbar ve `↑↓ kaydır` başlığı görünür. Uzun mesajlar geçmişte tam olarak tutulur; görünmeyen bölümler bu yolla okunur.
- Yanıt bildirimi: JARVIS bir yanıtı tamamladığında Hyprland bildirim alanında kısa bir önizleme gösterir.
- Kısayollar: `/help`, `/status`, `/clear`, `/approvals`
- Tek bir işlem onay bekliyorsa `/approve` veya `/cancel`; birden fazla varsa `/approve <task-id>` / `/cancel <task-id>`
- `/quit` veya `Ctrl+C`: yalnız sohbet ekranını kapatır; model arka planda RAM'de kalır.
- `exit`: sohbet ekranını kapatır ve yerel model sunucusunu durdurur; RAM boşalır.

Hyprland'de terminali `Super + Q` ile kapatmak da sadece arayüzü sonlandırır; model sunucusu çalışmaya devam eder. Sonraki `jarvis` açılışı model hâlâ RAM'deyse doğrudan kullanır, değilse otomatik başlatır.

## Native masaüstü penceresi (F2 önizleme)

Terminal arayüzü günlük kullanım için korunur. Aynı governed core'a bağlı, salt-okunur mesaj kartları ve ayrı composer kullanan native Wayland penceresi ise şu an F2 doğrulama aşamasındadır:

```bash
cd /home/mehmet/jarvis/jarvis
cargo run --offline --bin jarvis-desktop
```

- Pencereyi `Super + Q` ile kapatmak model servisini durdurmaz; soldaki **Modeli RAM'den çıkar** düğmesi bunu açıkça yapar.
- Birden fazla native pencere açılmaz; kapanmamış/eski kilit güvenle temizlenir ve mevcut pencere kullanılmaya devam edilir.
- Mesaj kartları düzenlenemez. Ortadaki arama alanı mevcut oturumda Türkçe büyük/küçük harf farkını gözetmeden arar; **Sen / JARVIS / Sistem** filtresiyle daraltılabilir.
- Tema, yazı ölçeği ve bildirim tercihi yalnız `~/.config/jarvis/desktop.json` içinde tutulur; ekrandan varsayılanlara döndürülebilir veya kullanıcı seçtiği konuma dışa aktarılabilir. Sohbet içeriği, ek dosya yolu veya kimlik bilgisi bu dosyaya yazılmaz.
- `Ctrl+O` veya **Görsel ekle** ile PNG/JPEG seçilir; önizleme ve kaldırma işlemi dosyayı silmez. Mevcut text-only model görsel piksellerini henüz analiz etmez; vision GGUF ve `mmproj` indirilene kadar ek sadece doğrulanmış metadata olarak taşınır.

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

Modelin tool veya policy yetkisi yoktur. Doğal sohbet modelden gelir; geçmiş, modele gerçek `user`/`assistant` rolleriyle ve tam konuşma çiftleri halinde iletilir. Son kullanıcı mesajı önceliklidir; kısa takip soruları yakın turlardan bağlam alır fakat önceki yanıtı gereksizce tekrar etmez. Kişisel bilgi veya tercih uygulama koduna gömülmez; bunun için sonraki dilimde kullanıcı profili/bellek katmanı eklenecektir. İzin gerektiren dosya değişiklikleri Policy Gate ve task-bound approval akışından geçer. Başka bir çalışma alanını okumak için `JARVIS_WORKSPACE_ROOT=/path/to/workspace` ayarlanabilir.

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
cargo run --offline --bin router_benchmark
```

`router_benchmark`, local modelin dar desktop routing görevlerindeki ilk baseline ölçümüdür; genel model kalitesi veya security/coding benchmarkı değildir.

## MCP stdio

İlk MCP transportu stdio üzerinden JSON-RPC mesajları alır; tüm tool çağrıları aynı registry ve Policy Gate'ten geçer.

```bash
cargo run --bin mcp_stdio
```

Desteklenen methodlar: `initialize`, `tools/list`, `tools/call`. Registry dışı tool adları execution'a dönüşmez.
