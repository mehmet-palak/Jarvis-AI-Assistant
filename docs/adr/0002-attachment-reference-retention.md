# ADR-0002: Ekler varsayılan olarak yerinde referanslanır

Durum: Kabul edildi — 14 Ağustos 2026

## Karar

JARVIS v1 ekin ham kopyasını uygulama kasasına taşımaz. `AttachmentRef` yalnız kullanıcının
seçtiği canonical yerel yolu, doğrulanmış MIME/dimension/byte-size bilgisini ve SHA-256 hash'ini
tutar. Gönderimden hemen önce dosya yeniden açılır; path, hash veya boyut değişmişse istek
reddedilir ve kullanıcı dosyayı yeniden seçer.

## Gerekçe

- Aynı fotoğrafın gereksiz gizli kopyasını oluşturmaz.
- Kullanıcının dosya yöneticisinden sildiği ek JARVIS tarafından saklanmış kalmaz.
- Disk kullanımını ve data-retention yüzeyini küçültür.
- Stale reference doğrulaması, seçimden sonra dosyanın değiştirilmesi riskini görünür kılar.

## Mahremiyet ve silme semantiği

- UI'daki “kaldır” yalnız gönderim kuyruğundaki referansı temizler; kullanıcının orijinal dosyasını
  asla silmez.
- Başarılı/başarısız task audit'i attachment ID'yi içerir; canonical path veya görüntü byte'ı audit
  eventine yazılmaz.
- Native ve TUI, gönderim tamamlandığında en fazla 50 adet **oturumluk ek makbuzu** gösterebilir.
  Makbuz yalnız kullanıcı görünür dosya adı, MIME, boyut/dimension, SHA-256 ve attachment ID
  içerir. Yerel yol, ham byte, prompt, model yanıtı veya task/audit içermez; pencere/TUI kapanınca
  kaybolur. Kullanıcı tekli/tümü kaldırabilir veya yalnız bu metadata'yı açıkça seçtiği JSON
  dosyasına dışa aktarabilir.
- Gelecekte vision modelinin kendi cache'i olursa ayrı, süreli retention ve tekli/tüm cache silme
  UX'i ile bu ADR güncellenecektir.

## Sonuç

Ek geçmişi uygulama kopyası değil, kullanıcının kontrol ettiği dosyaya kısa ömürlü bir referanstır.
