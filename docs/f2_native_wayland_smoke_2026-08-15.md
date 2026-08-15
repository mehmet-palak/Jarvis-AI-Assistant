# F2 native Wayland smoke — resize/minimize/Ctrl+O/mesaj/bildirim — 15 Ağustos 2026

[14 Ağustos kaydı](f2_native_wayland_smoke_2026-08-14.md), pencere açılışı ve compositor
focus/close temelini kapatmıştı. Bu koşum, o kayıtta "kullanıcı kabulü gerekir" diye açık bırakılan
resize/minimize, `Ctrl+O` picker, mesaj gönderme ve bildirim action maddelerini gerçek release
binary'siyle, `ydotool`/`grim`/`hyprctl` ile sınadı. Gerçek kullanıcı `jarvis.db`'sine bu koşumda
elle dokunulmadı; yalnız uygulamanın normal audit/lifecycle yazımı oluştu.

## Doğrulanan

- **Pencere açılışı**: `target/release/jarvis-desktop`, gerçek Hyprland tiling düzeninde iki farklı
  geometride (2048x1152 tam panel, 986x1074 paylaşılan tile) hatasız, kırpılmadan render etti.
- **`Ctrl+O` dosya seçici**: gerçek `xdg-desktop-portal-gtk` "Dosya Aç" diyaloğu açıldı, kullanıcının
  gerçek Masaüstü klasörünü doğru gösterdi. Diyalog test sırasında **hiçbir dosya seçilmeden**
  iptal edildi (kullanıcının kişisel dosyalarına dokunulmadı).

## Bulgular — backlog'a eklenmesi gereken

1. **Sessiz kapanış**: `Ctrl+O` ile açılan dosya seçici `Escape` ile iptal edildikten kısa süre
   sonra, `jarvis-desktop` penceresi hiçbir hata/panic kaydı bırakmadan (boş stderr, coredump yok,
   dmesg'de segfault yok) tamamen kapandı. Bu koşumda compositor focus dispatch'i (`hl.dsp.focus`)
   diyalog adresinde "window not found" uyarısı verdi; kapanışın picker-iptal akışıyla mı yoksa
   focus kaybıyla mı tetiklendiği kesin ayrıştırılamadı. **Tekrar üretilebilirlik teyit edilmeli.**
2. **Tab-sırası riski**: Pencere açılışında `Tab` tuşu odağı doğrudan **"MODELİ RAM'DEN ÇIKAR"**
   butonuna taşıyor (composer'a değil). Bu düğme fokuslanmışken yazılan bir mesajın içindeki boşluk
   karakteri düğmeyi tetikleyip modeli gerçekten durdurdu (`jarvis-llama.service` bu testte kazayla
   `inactive` oldu; test sonunda `systemctl --user start jarvis-llama.service` ile eski `active`
   duruma geri getirildi ve health `{"status":"ok"}` ile doğrulandı). Klavye-öncelikli kullanımda
   yıkıcı bir eylemin ilk Tab durağı olması güvenli bir varsayılan değil.
   - Öneri: ilk `Tab` durağı composer olmalı; RAM'den çıkar gibi durum-değiştiren eylemler tab
     sırasında sona alınmalı veya ayrı bir onay adımı taşımalı.

## Test edilemeyen (gerçek kullanıcı elle sınamalı)

- Fare ile sürükleyerek **resize/minimize**: bu sistemde `hyprctl dispatch` klasik dispatcher
  string'lerini kabul etmiyor (özel Lua `hl.dsp.*` API'si var) ve pencere kenarını sürüklemek
  sentetik girdiyle güvenilir şekilde taklit edilemedi.
- Fare ile composer'a tıklayıp **gerçek mesaj yazıp gönderme**: `ydotool` mutlak fare koordinatları
  bu çoklu-monitör/ölçekli (1.25x) kurulumda güvenle kalibre edilemediği için denenmedi — yanlış
  koordinata tıklamak kullanıcının diğer pencerelerine (VSCode, Brave) müdahale riski taşıyordu.
- **Bildirim action tıklaması** ("JARVIS'i aç"): gerçek bir masaüstü bildirim tıklamasını tetiklemek
  için kullanıcı etkileşimi gerekiyor.

## Bulgu 2 düzeltmesi — aynı koşum içinde

`Tab`-sırası bulgusu için iki kod düzeltmesi yapıldı ve gerçek release binary'siyle yeniden
doğrulandı:

1. Composer artık pencere açılışında varsayılan klavye odağını alıyor
   (`composer_focus_claimed`); artık `Tab`'a gerek kalmadan doğrudan yazmaya başlamak metni
   composer'a yazıyor.
2. "MODELİ RAM'DEN ÇIKAR" düğmesi artık tek tıklamayla değil, `STOP_MODEL_CONFIRM_WINDOW` (4
   saniye) içinde **ikinci bir tıklama/aktivasyon** ile çalışıyor; ilk aktivasyon yalnız düğmeyi
   "EMİN MİSİN? TEKRAR TIKLA" olarak işaretliyor.

Doğrulama: aynı senaryo (`Tab` + boşluklu çok kelimeli metin) gerçek release binary'sinde tekrar
denendi — **ilk düzeltme denemesinde bile** (yalnız onay penceresi eklenmişken, composer
autofocus'u olmadan) art arda iki boşluk aynı frame'de düğmeyi hem kolluyor hem onaylıyor ve model
yine durdu; bu, `jarvis-llama.service` yeniden başlatılarak giderildi. Composer autofocus'u
eklendikten sonra aynı senaryo (`Tab` **atlanarak**, doğrudan çok kelimeli gerçek bir cümle
yazılarak) tekrar edildi: metin composer'a gitti, `MODEL HAZIR` yeşil kaldı, servis `active`
kaldı. Kök neden gerçekten composer'ın varsayılan odağı almaması olduğu, yalnız onay penceresinin
tek başına yeterli olmadığı doğrulandı.

**Çözülemeyen ek bulgu**: Composer'a doğru yazılan metni `ydotool` ile sentetik `Enter`
tuşuyla göndermeyi iki kez denedim, ikisinde de mesaj gönderilmedi (composer içeriği ve sohbet
geçmişi değişmedi). `Ctrl+O` kısayolu aynı oturumda güvenilir çalıştığından, bu ya `ydotool`'un bu
özel tuş olayını egui'nin beklediği şekilde iletmemesi ya da gerçek bir Enter-gönderim hatası
olabilir — sentetik girdiyle ayrıştırılamadı. **Kullanıcının gerçek klavyeyle Enter'a basıp/`GÖNDER`
düğmesine tıklayıp mesajın gerçekten gittiğini doğrulaması gerekiyor.**

## Kapanış durumu

- Model servisleri test sonunda test-öncesi durumuna getirildi: `jarvis-llama.service active`
  (health PASS), `jarvis-vision.service inactive` (testten önce de inactive'ti, değişmedi).
- Açılan tüm test pencereleri kapatıldı; kullanıcının gerçek dosyalarına yazılmadı/silinmedi.

Bu koşum resize/minimize/mesaj gönderme/bildirim action'ını **kapatmıyor** — bunlar gerçek kullanıcı
elinde birkaç dakikalık elle kabul olarak kalıyor. Buna karşılık iki somut, tekrar araştırılması
gereken bulgu (sessiz kapanış, Tab-sırası) backlog'a eklendi.
