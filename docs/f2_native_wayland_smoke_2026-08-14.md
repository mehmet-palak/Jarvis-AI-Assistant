# F2 native Wayland smoke — 14 Ağustos 2026

Bu koşum yalnız release derlemesinin pencere açılışını doğrulamak içindir; sohbet, kullanıcı
dosyası veya kişisel ayar içeriği kayda alınmadı.

## Sonuç

- `target/release/jarvis-desktop`, mevcut Hyprland/Wayland oturumunda açıldı.
- Ekran doğrulamasında HUD başlığı, CPU-only/VRAM 0 model göstergesi, sol güvenlik paneli,
  merkez orb, salt-okunur sistem mesaj kartı ve ayrı composer görünür durumdaydı.
- 14 Ağustos 2026 release ölçümünde aynı binary, Hyprland v0.56.2 client listesine **222 ms**
  içinde kaydoldu; o anda RSS **181,904 KiB** (yaklaşık 178 MiB) idi.
- Hyprland v0.55+ Lua dispatcher sözleşmesiyle `hl.dsp.focus({ window = "pid:…" })` çağrısı
  gerçek release pencere PID'si için PASS verdi. Native bildirim action'ı destekleyen daemonlarda
  “JARVIS'i aç” seçeneği yalnız bu PID'ye bu focus isteğini yollar. `notify-send`, daemon veya
  compositor bunu desteklemezse task/UI sonucu değişmez.
- Aynı ölçüm penceresi `hl.dsp.window.close({ window = "pid:…" })` ile zarifçe kapandı;
  iki model servisi çalışmaya devam etti.
- Release binary, native GUI bileşeninin core'dan ayrı ikinci bir runtime yaratmadığını doğrulayan
  unit setiyle birlikte çalıştı.
- Smoke sırasında oluşturulan test penceresi kapandıktan sonra yalnız o koşuma ait bayat
  single-instance lock kontrol edilerek temizlendi; kullanıcının penceresi veya model servisi
  durdurulmadı.

## Henüz kullanıcı kabulü gerektirenler

- Gerçek kullanıcı etkileşimiyle resize/minimize, dosya seçici, bildirim action tıklaması ve
  erişilebilirlik tercihlerinin uzun oturum davranışı.
- Bu maddeler F2.0 exit review altında açık kalır; otomatik smoke bunların yerine geçmez.
