# F2 TUI görsel smoke — gerçek terminal — 15 Ağustos 2026

Ratatui `TestBackend` regresyonuna ek olarak, gerçek release binary'si gerçek Wayland/Hyprland
terminalinde (`foot`, 2560x1440 monitör, `scale: 1.25`) üç farklı geometri/font ayarıyla çalıştırıldı
ve ekran görüntüsüyle doğrulandı.

## Koşumlar

1. **Küçük pencere** (986x523 px, tiled üst yarı) — sistem mesajı, composer ve durum çubuğu net,
   kırpılma yok.
2. **Aynı geometri, odaksız/odaklı çift pencere** — üst (odaksız) pencere içi boş anahat cursor,
   alt (odaklı) pencere dolu cursor gösterdi. Odak/cursor davranışı beklendiği gibi ayrışıyor.
3. **Büyük font** (`size=22`, 986x1074 px) — gövde metni ve composer tam okunaklı kaldı.

## Bulgu

Büyük font koşumunda üst durum çubuğu (`Sohbet` satırı) taşıyor: `MODEL HAZIR` rozeti kırpılıyor
(`MODEL HAZI`) ve `VRAM: 0 · EK: 0` alanı tamamen görünüm dışına çıkıyor. Sohbet/mesaj/composer
işlevselliği etkilenmiyor; yalnız bu bir bilgi rozetinin görsel taşmasıdır.

- Etki: kozmetik, P0/P1 değil (bkz. [f2_bug_report_template.md](f2_bug_report_template.md) formatı).
- Öneri: durum çubuğu genişliğe göre rozetleri gizleme/kısaltma önceliği alsın (`VRAM`/`EK` önce
  düşsün, `MODEL HAZIR` en son).
- Takip: UX backlog'una eklendi; F2.0 exit review'u bloklamıyor (bkz. [f2_conversation_qa.md](f2_conversation_qa.md)
  içindeki benzer kozmetik backlog bulguları — emoji-cursor sütunu, ek kapatma kısayolu vb. — aynı
  şekilde işlem görüyor).

## Sonuç

Küçük/büyük pencere, odak/cursor davranışı ve okunabilir kontrast gerçek terminalde PASS. Büyük
fontta durum çubuğu taşması tek kozmetik bulgu olarak backlog'a not edildi.
