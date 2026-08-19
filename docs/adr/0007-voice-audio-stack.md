# ADR-0007 — F5 ses yığını: yakalama, konuşma tanıma ve seslendirme

Tarih: 19 Ağustos 2026
Durum: Kabul edildi
Kapsam: F5 madde 1 (Audio ADR), madde 2 (STT aday değerlendirmesi), madde 6 (TTS aday
değerlendirmesi), madde 9 (wake word spike)

## Karar özeti

| Katman | Seçim | Boyut | Lisans |
| --- | --- | --- | --- |
| Ses yakalama | `pw-record` (PipeWire, sistemde zaten var) | — | — |
| Konuşma tanıma | whisper.cpp + `ggml-small-q5_1.bin` | 182 MB | MIT |
| Seslendirme | Piper (rhasspy 2023.11.14-2) + `tr_TR-dfki-medium` | 30 MB + 61 MB | MIT |

Üçü de **alt süreç olarak** çağrılıyor — `llama-server`'ın zaten kullandığı desen. Bu, projeye
tek bir yeni Rust bağımlılığı bile eklemiyor.

## Ses yakalama — neden yeni bir kütüphane değil

Sistemde PipeWire 1.6.8 çalışıyor ve `pw-record` kurulu. Ölçüldü: `pw-record --rate 16000
--channels 1 --format s16` doğrudan whisper'ın istediği formatı üretiyor — ara dönüştürme
adımı yok.

`cpal` gibi bir Rust ses kütüphanesi eklemek yerine alt süreç kullanmanın gerekçesi:

- Projede zaten üç alt süreç var (`llama-server`, `bwrap`, `git`); yeni bir kalıp icat edilmiyor.
- Ses kütüphaneleri platform bağımlılığı ve derleme karmaşıklığı getirir; alt süreç getirmiyor.
- İptal etmek tek bir sinyal göndermek demek (`SIGINT`), yarı yazılmış bir tampon temizlemek değil.
- Kullanıcının sistemi Linux/PipeWire; taşınabilirlik bugün gerçek bir gereksinim değil
  (bkz. "Bilinen sınırlar").

Kayıt formatı: 16 kHz, mono, `s16`. Whisper'ın beklediği format bu; başka bir şey seçmek her
transkripsiyonda gereksiz bir yeniden örnekleme demek olurdu.

## Konuşma tanıma — ölçülerek seçildi, akıl yürütmeyle değil

İlk seçim `large-v3-turbo-q5_0` idi ("turbo hız için optimize, Türkçe'de en doğru"). **Ölçüm bunu
çürüttü.** Piper ile üretilmiş üç Türkçe cümle, üç modele karşı koşuldu (8 CPU thread):

| Model | Boyut | Tam eşleşme | Toplam süre |
| --- | --- | --- | --- |
| `small-q5_1` | 182 MB | 1/3 | **7.2 s** |
| `medium-q5_0` | 515 MB | 2/3 | 22.8 s |
| `large-v3-turbo-q5_0` | 548 MB | 2/3 | 36.2 s |

Belirleyici gözlemler:

1. **Üç model de aynı hatayı yaptı** ("alanımdaki" → "alanındaki"). Büyük model bu hatayı
   düzeltmiyor — yani ek maliyet karşılığında ek doğruluk gelmiyor.
2. `large-v3-turbo`, `medium`'a göre **hiçbir doğruluk kazancı vermeden %60 daha yavaş.** CPU'da
   "turbo"nun avantajı kayboluyor: turbo yalnız decoder katmanlarını azaltıyor, encoder tam boy
   kalıyor ve CPU'da süreyi encoder belirliyor.
3. `small`'ın fazladan hataları küçük: cümle başı büyük harf ve aynı tek kelime. Kullanıcı zaten
   transkripti göndermeden önce görüp düzeltiyor (madde 4), bu yüzden bu sınıf hata maliyetsiz.

**Seçim: `small-q5_1`.** Kısa bir cümle ~2.4 saniyede çevriliyor; bas-konuş için kullanılabilir
tek seçenek bu. `medium` (7.6 s/cümle) ve `large-v3-turbo` (12 s/cümle) etkileşimli kullanım için
çok yavaş.

`medium-q5_0` ve `large-v3-turbo-q5_0` diskte bırakıldı: bu ölçüm **TTS ile üretilmiş temiz ses**
üzerinde yapıldı. Gerçek mikrofon kaydı (arka plan gürültüsü, ağız uzaklığı, aksan) daha zor bir
girdi; `small` orada belirgin şekilde bozulursa daha büyük modele geçiş yeniden değerlendirilir.
Bu, F6'nın kurduğu ölçüm disiplininin ses tarafındaki karşılığı.

## Seslendirme — Piper

Türkçe için pratikte tek gerçek yerel seçenek: `rhasspy/piper-voices` deposunda Türkçe yalnız
`tr_TR-dfki-medium` (61 MB) olarak var.

Ölçüldü: 3.58 saniyelik ses **0.11 saniyede** üretildi — gerçek zamanın ~33 katı hız. Bu, TTS'in
gecikme açısından hiç sorun olmayacağı anlamına geliyor.

Çalıştırılabilir sürüm olarak **eski `rhasspy/piper` 2023.11.14-2** seçildi (MIT), yeni
`OHF-Voice/piper1-gpl` değil. Gerekçe: eskisi bağımsız bir ikili dosya (mevcut alt süreç
desenine uyuyor), yenisi bir Python wheel'i — Python ortamı yönetmek gerekirdi. Lisans farkı da
ikincil bir gerekçe.

## Gizlilik — ham ses varsayılan olarak saklanmaz

`RecordingRetention` tipinin varsayılanı `DiscardImmediately`: transkript çıkarıldığı anda WAV
dosyası siliniyor. Bu bir yapılandırma tercihi değil, **tipin kendi varsayılanı** — bir ayar
dosyası unutulsa veya bozulsa bile davranış gizliliği koruyor.

Kullanıcı saklamayı seçerse konum ve silinme zamanı tipin içinde taşınıyor
(`KeepUntil { path, delete_after_epoch }`) ve `user_visible_summary()` ile gösteriliyor. "Belki
bir yerde duruyor" durumu kabul edilmiyor.

Ses hiçbir zaman ağa çıkmıyor: whisper ve Piper tamamen yerel, indirme dışında internet
kullanmıyorlar.

## Sesli onay sınırı

Ses, yazılı onaydan **daha zayıf** bir yetkilendirme kanalı: yanlış duyulabilir, odadaki başka
biri söyleyebilir, bir kayıttan tekrar oynatılabilir. Bu yüzden `approval_channel_requirement`,
policy gate'in zaten onay şartı koyduğu bir eylemin ses ile onaylanmasını reddediyor ve deneme
`approval.channel_insufficient` olarak audit'e yazılıyor.

Kural tek yönlü: ses her zaman **reddedebilir** ve onay gerektirmeyen her eylemi yapabilir.
Yalnız "zaten onay gerektiren bir eylemin tek yetkilendirmesi olmak" yasak. Aksi halde sesli
kullanım gereksiz yere sakatlanır ve kullanıcı sesi hiç kullanmaz.

## Wake word ("Hey JARVIS") — ŞİMDİLİK YAPILMAYACAK

F5 planı bunu zaten "araştırma spike'ı" olarak, ayrı bir feature flag arkasında istiyordu.
Karar: **eklenmeyecek.**

Gerekçe: wake word, mikrofonun **sürekli açık** olmasını gerektirir. Bu, F5'in ve projenin geri
kalanının tüm gizlilik duruşuyla çelişir — "her zaman dinleyen bir sistem yerine açık,
mahremiyeti koruyan push-to-talk" planın kendi amaç cümlesi. Bas-konuş, kullanıcının her kaydı
bilerek başlattığı bir modeldir; wake word bunu terk eder.

Yeniden değerlendirme koşulu: kullanıcı açıkça isterse, ve ancak (a) ayrı bir feature flag,
(b) dinleme sırasında görünür bir gösterge, (c) klavye/fiziksel kill switch, (d) `retention=off`
zorunlu — dördü birden olmadan açılmaz.

## Bilinen sınırlar (dürüst kayıt)

- **Yalnız Linux/PipeWire.** `pw-record` bir platform bağımlılığı. Windows/macOS desteği
  gerekirse yakalama katmanı değişir; STT/TTS katmanları değişmez.
- **Terminalde gerçek "bas-tut" yok.** Terminal, tuş bırakma olayını güvenilir şekilde
  bildirmiyor. Bu yüzden TUI'de kayıt **aç/kapa** şeklinde: bir kez başlat, bir kez durdur.
  Native masaüstü istemcisinde gerçek bas-tut mümkün, ayrı ele alınacak.
- **STT ölçümü sentetik ses üzerinde yapıldı.** Gerçek mikrofon kaydıyla doğrulama, gerçek
  kullanım sırasında yapılacak; `small` yetersiz kalırsa `medium` diskte hazır bekliyor.
