# ADR-0003: Kullanıcı profili, genel bellek sisteminin üzerine sabit alan kümesi olarak kurulur

Durum: Kabul edildi — 15 Ağustos 2026

## Karar

JARVIS'te "profil" için ikinci bir depolama tablosu/yolu açılmaz. Profil, F3 öncesinde zaten var
olan genel bellek sisteminin (`MemoryNamespace::UserProfile`, `propose_memory` →
`commit_memory_proposal` → audit zinciri) üzerine oturan, **sabit isimli dört anahtardan** oluşur:

| Alan | Bellek anahtarı |
| --- | --- |
| Ad | `display_name` |
| Hitap biçimi | `preferred_address` |
| Dil | `language` |
| Rol / tercih | `role_preference` |

`src/profile.rs`, bu dört anahtarı tanımlar (`ProfileField`), değerlerini doğrular
(`validate_profile_value`) ve genel bellek kayıtları arasından bilinen alanların en güncel
değerini seçer (`ProfileSnapshot`). Depolama, onay, silme ve denetim tamamen mevcut
`propose_memory`/`commit_memory_proposal`/`delete_memory` üzerinden geçer; bu modül yalnız bir
doğrulama/okuma katmanıdır.

## Gerekçe

- **İkinci bir sistem kurmak riskli.** Genel bellek sistemi zaten namespace ayrımı, TTL,
  sensitivity, `include_in_model_context` opt-in'i, onay öncesi preview ve audit zincirini
  sağlıyor ve test kapsamında. Profili ayrı bir tablo/yol yapmak bu garantileri iki kez
  bakımlı tutmayı gerektirirdi.
- **Sabit anahtarlar, serbest anahtarları kısıtlamaz.** Kullanıcı hâlâ `/remember favori_renk =
  teal` gibi bilinmeyen anahtarlar yazabilir; bunlar `/memory` listesinde görünür ve elle
  silinebilir, sadece profil CRUD arayüzünün (F3 madde 2) göstereceği "bilinen alan" formunun bir
  parçası olmazlar.
- **Değer doğrulaması kasıtlı olarak gevşek.** Dil veya isim alanına sabit bir kelime listesi
  (örn. yalnız "tr"/"en") dayatmadık — bu hem gereksiz kısıtlayıcı olurdu hem de kullanıcı kendi
  yazdığı biçimde (örn. "Türkçe" veya "tr") saklayabilsin istiyoruz. Doğrulama yalnız boş/aşırı
  uzun/kontrol karakteri içeren girdileri reddeder.
- **En güncel kayıt kazanır.** Bellek sistemi `memory_id`'yi her onayda yeniden üretiyor (nonce
  içeriyor), yani aynı alanı iki kez `/remember` ile yazmak iki ayrı satır oluşturuyor — bilinçli
  bir tasarım (geçmiş değerler tamamen kaybolmuyor, `/memory`'de görünür kalıyor). `ProfileSnapshot`
  bu satırlar arasından yalnız `updated_at`'i en yeni olanı profil değeri sayar.

## Sohbetten otomatik kalıcı yazma

Kod tabanında `propose_memory`/`commit_memory_proposal`'a giden tek yol, TUI'deki açık `/remember`
komutudur (`src/main.rs`); model, normal sohbet yanıtı üretirken bu fonksiyonları hiçbir zaman
çağırmaz — `handle_with_provider*` zinciri bunlara erişmez. `src/profile.rs` da aynı kısıtı miras
alır: kendi başına hiçbir depolama tetiklemez, yalnız açık kullanıcı komutlarının çağıracağı bir
yardımcıdır.

**Bilinen açık nokta:** native masaüstü (`jarvis_desktop.rs`) şu an `/remember`'ın bir eşdeğerine
sahip değil — bellek/profil yalnız TUI'den yazılabiliyor. Bu, F3'ün "Profile CRUD UX" maddesinde
(madde 2) native tarafa da taşınacak; bu ADR yalnız şemayı kapsıyor.

## Sonuç

Profil, genel bellek sisteminin üzerine ince, doğrulayıcı bir katmandır — ayrı bir depolama yolu
değil. Bu, tek bir onay/audit/silme zincirini korur ve mimarinin "ikinci bir Policy/Persistence
yolu açma" ilkesiyle tutarlıdır.

## Ek — Namespace fiziksel ayrımı (15 Ağustos 2026, F3 madde 4)

`MemoryNamespace`'e iki yeni üye eklendi: `Session` (oturuma özel kısa ömürlü not) ve
`EphemeralToolOutput` (bir aracın kısa süreli önbelleğe alınmış çıktısı). Üç kalıcı namespace'ten
(`UserProfile`/`Project`/`Task`) **fiziksel** olarak ayrılmaları şu şekilde sağlanıyor:
`validate_memory_record`, bu iki namespace için `expires_at` alanı boşsa kaydı doğrudan reddediyor
— yani süresiz kalıcı bir Session/EphemeralToolOutput kaydı **oluşturulamıyor**, bu bir isimlendirme
kuralı değil, yapısal bir kısıt. `retrieve_memory` zaten süresi geçmiş kayıtları otomatik filtrelediği
için (`expires_at IS NULL OR expires_at > now`) bu iki namespace modele hiçbir zaman bayat veri
sızdırmıyor.

**Bilinen açık nokta:** Şu an hiçbir üretim kod yolu (`/remember`, `/profile set`) bu iki yeni
namespace'e yazmıyor — ikisi de yalnız `UserProfile`'a sabitlenmiş durumda. Bu bilinçli: namespace'ler
şema olarak hazır, ama onları dolduracak somut özellik (örn. görev-bazlı otomatik not, RAG sonucu
önbelleği) henüz F3'ün ilerideki maddelerinde/F4'te gelecek.

## Ek — Silme: tombstone yok, gerçek `DELETE` (15 Ağustos 2026, F3 madde 7)

Tek kayıt (`/forget <id>`), namespace (`/forget namespace <...>`) ve "her şeyi unut" (`/forget all`)
silme yolları hepsi gerçek SQL `DELETE` çalıştırır — **tombstone/soft-delete satırı bırakmaz**.
Bilinçli tercih: ADR-0002'nin ekler için kurduğu "kaldırmak gerçekten kaldırmaktır, gizli bir kopya
tutmayız" felsefesiyle tutarlı. JARVIS tek-kullanıcı, tek-cihaz, senkronizasyonsuz yerel bir SQLite
veritabanı kullanıyor; tombstone'un asıl değeri (dağıtık sistemlerde "silindi" bilgisini diğer
replikalara yaymak) burada karşılığı yok.

**Yedek etkisi (dokümante edilmesi istenen nokta):** Bir silme işlemi yalnız **canlı** `jarvis.db`
dosyasını etkiler. Kullanıcının daha önce aldığı herhangi bir dosya-seviyeli yedek (örn. bu projede
zaten bir örneği olan `jarvis.db.audit-race-backup-*.db` gibi) silinen veriyi **hâlâ içerir** —
silme işlemi geçmişe dönük yedekleri temizlemez.

## Ek — Rollback, export/import ve şifreleme kararı (15 Ağustos 2026, F3 madde 8)

**Rollback:** `SqliteStore::open`, açtığı dosyanın üzerindeki `schema_migrations` sürümü bu build'in
bildiği en yüksek sürümden (şu an 5) düşükse, `migrate()` dokunmadan **önce** `VACUUM INTO` ile
(`backup_to` — zaten var, test'liydi, kullanılmayan bir fonksiyondu) dosyayı `<yol>.pre-migration-
backup-<epoch>.db` olarak yedekler. Zaten güncel veya yepyeni bir veritabanı hiç yedeklenmez — her
normal açılışta gereksiz yedek birikmesin diye. "Rollback" burada programatik bir geri alma değil,
**bu dosyayı geri yükleme** prosedürüdür — tek kullanıcılı, senkronizasyonsuz yerel bir uygulama için
programatik down-migration makinesi orantısız olurdu.

**Export/Import:** `memory_export`/`memory_import` (TUI: `/memory export <dosya-yolu>`, `/memory
import <dosya-yolu>`) tüm namespace'leri kapsayan, taşınabilir bir JSON yedeği sağlar —
`backup_to`'nun aksine (ki o tüm veritabanını, task/audit dahil, ham SQLite dosyası olarak yedekler)
bu yalnız bellek kayıtlarını, insan-okunur biçimde taşır. `memory_id`/`source` dışa aktarılmaz;
içe aktarma her zaman `propose_memory` üzerinden **yeni** bir teklif üretir, hiçbir zaman doğrudan
yazmaz — model gibi, içe aktarma da yalnız açık `/memory import` komutuyla erişilebilir, kalıcı
yazma onay adımından (aynı `commit_memory_proposal`) geçer.

**Şifreleme kararı:** Hassas (`Sensitive`) olarak işaretlenmiş kayıtlar için ayrı bir şifreli
depolama **eklenmedi**. Gerekçe: JARVIS tek kullanıcılı, tek cihazlı, ağa kapalı yerel bir uygulama;
gerçek güvenlik sınırı işletim sisteminin dosya izinleridir — bu, ADR-0002'nin ekler için zaten kabul
ettiği aynı sınırdır. `sensitivity` alanı bir sınıflandırma/organizasyon etiketi olarak kalır, şifreleme
anahtarı yönetimi gibi ek karmaşıklık getirmez. **Bu karar, çok kullanıcılı, senkronize/bulut yedekli
bir gelecek senaryosunda yeniden gözden geçirilmelidir** — o zaman gerçek bir tehdit modeli değişir.
