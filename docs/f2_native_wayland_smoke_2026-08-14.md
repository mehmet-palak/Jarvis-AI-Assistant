# F2 native Wayland smoke — 14 Ağustos 2026

Bu koşum yalnız release derlemesinin pencere açılışını doğrulamak içindir; sohbet, kullanıcı
dosyası veya kişisel ayar içeriği kayda alınmadı.

## Sonuç

- `target/release/jarvis-desktop`, mevcut Hyprland/Wayland oturumunda açıldı.
- Ekran doğrulamasında HUD başlığı, CPU-only/VRAM 0 model göstergesi, sol güvenlik paneli,
  merkez orb, salt-okunur sistem mesaj kartı ve ayrı composer görünür durumdaydı.
- Release binary, native GUI bileşeninin core'dan ayrı ikinci bir runtime yaratmadığını doğrulayan
  unit setiyle birlikte çalıştı.
- Smoke sırasında oluşturulan test penceresi kapandıktan sonra yalnız o koşuma ait bayat
  single-instance lock kontrol edilerek temizlendi; kullanıcının penceresi veya model servisi
  durdurulmadı.

## Henüz kullanıcı kabulü gerektirenler

- Gerçek kullanıcı etkileşimiyle resize/minimize/focus, dosya seçici, bildirim tıklaması ve
  erişilebilirlik tercihlerinin uzun oturum davranışı.
- Bu maddeler F2.0 exit review altında açık kalır; otomatik smoke bunların yerine geçmez.
