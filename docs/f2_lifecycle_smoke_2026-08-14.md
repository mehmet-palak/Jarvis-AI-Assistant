# F2 model lifecycle smoke — 14 Ağustos 2026

Bu koşum, gerçek release TUI'nin pseudo-terminal içindeki açık kapanış yollarını test eder.
Kullanıcı mesajı veya sohbet geçmişi üretilmedi.

1. `jarvis-llama.service` ve `jarvis-vision.service` aktifken `target/release/jarvis` açıldı.
2. TUI'ye `exit` gönderildi.
3. İki servis de `inactive` durumuna geçti; sonra test öncesi hazır halini korumak için ikisi de
   yeniden başlatıldı.
4. Son health kontrolü: `127.0.0.1:8088` ve `127.0.0.1:8089` için `{"status":"ok"}`.

Bu, `exit`in RAM serbest bırakma yolunu kanıtlar. `/quit`, `Ctrl+C`, pencere kapatma,
resize/minimize/focus ve bildirim tıklaması için kullanıcı kabul koşumu F2.0 exit review içinde
ayrı kalır.

## RAM'i koruyan kapanış yolları

Model servisleri aktifken release TUI ayrı pseudo-terminal koşumlarında açıldı. Arayüz raw-mode'a
geçtikten sonra iki saniye beklenip aşağıdaki girdiler gönderildi:

| Girdi | Text servis önce/sonra | Vision servis önce/sonra | Sonuç |
| --- | --- | --- | --- |
| `/quit` + Enter | `active` → `active` | `active` → `active` | PASS |
| `Ctrl+C` | `active` → `active` | `active` → `active` | PASS |

Bu iki koşum yalnız release TUI sürecini bitirdi; hiçbir model servisini başlatmadı veya
durdurmadı. Terminal pencere yöneticisiyle kapanış ve native pencerenin kullanıcı etkileşimli
kapanışı halen kullanıcı kabul koşumudur.
