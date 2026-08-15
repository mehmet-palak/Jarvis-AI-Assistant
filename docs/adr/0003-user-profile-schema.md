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
- **En güncel kayıt kazanır — `ProfileSnapshot` düzeyinde bir güvenlik ağı, birincil mekanizma
  değil.** `memory_id` artık (16 Ağustos 2026'dan sonra, bkz. aşağıdaki ek) `(namespace, key)`'den
  türetiliyor, yani aynı alanı iki kez `/remember` ile yazmak normalde tek satırı günceller, ikinci
  bir satır oluşturmaz. `ProfileSnapshot`'ın "en güncel `updated_at` kazanır" mantığı yine de kalır
  — yalnız eski (şema öncesi) içe aktarılmış bir yedek gibi istisnai durumlar için bir yedek/emniyet
  katmanı olarak.

## Sohbetten otomatik kalıcı yazma

Kod tabanında `propose_memory`/`commit_memory_proposal`'a giden yollar iki tane (16 Ağustos
2026'dan beri, bkz. aşağıdaki ek): TUI'deki açık `/remember` komutu (`src/main.rs`) ve doğal dil
tetikleyicileri (`src/memory_intent.rs`, örn. "hafızana yaz: ..."). İkisi de aynı kısıtı paylaşır:
model, normal sohbet yanıtı üretirken bu fonksiyonlara hiçbir zaman erişemez —
`handle_with_provider*` zinciri onlara dokunmaz; karar her zaman kullanıcının kendi yazdığı ham
metne (slash komutu veya doğal dil tetikleyicisi) bakılarak veriliyor, model hiç danışılmıyor.
`src/profile.rs` da aynı kısıtı miras alır: kendi başına hiçbir depolama tetiklemez, yalnız açık
kullanıcı komutlarının çağıracağı bir yardımcıdır.

**Native masaüstü:** artık (16 Ağustos 2026) doğal dil tetikleyicileri üzerinden aynı yeteneğe
sahip (`handle_natural_language_memory_command`, `src/bin/jarvis_desktop.rs`) — bir `/remember`
sözdizimi eşdeğeri hâlâ yok, ama bu artık gerçek bir eksiklik değil: doğal dil yolu zaten hem
TUI'de hem native'de aynı tek adımlı, onay-gerektirmeyen davranışı sağlıyor.

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
bildiği en yüksek sürümden (`persistence.rs`'teki `CURRENT_SCHEMA_VERSION`, sık sık artan bir
sayı — burada sabit bir değer olarak anılmıyor) düşükse, `migrate()` dokunmadan **önce** `VACUUM INTO` ile
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

## Ek — Güncelleme düzeltmesi, doğal dil komutları ve kalıcı sohbet geçmişi (16 Ağustos 2026)

**Gerçek bug düzeltildi:** yukarıdaki "Gerekçe" bölümünde "aynı alanı iki kez `/remember` ile
yazmak iki ayrı satır oluşturuyor — bilinçli bir tasarım" diye anlatılan davranış aslında bir
hataydı, bilinçli bir tasarım değildi. `memory_id`, değer/kaynak/nanosaniye nonce'undan
türetildiği için aynı `(namespace, key)` bile her onayda "yeni" sayılıyordu — gerçek bir şişme
riski, üstelik eski ve yeni değer ikisi de geçerliyse ikisi de birden modele gidebiliyordu.
Düzeltme: `memory_id` artık yalnız `(namespace, key)`'den türetiliyor, bu da zaten var olan
`ON CONFLICT(memory_id) DO UPDATE` yolunu (`src/persistence.rs`, hiç değişmedi) gerçek bir
güncelleme için tetikliyor. `created_at` korunuyor, `updated_at` ilerliyor. Kanıt:
`remembering_the_same_key_again_updates_the_existing_record_instead_of_duplicating_it`
(`src/lib.rs`).

**Doğal dil komutları:** kullanıcı `/remember anahtar = değer` sözdizimini hatırlamak zorunda
kalmadan, normal bir cümleyle ("hafızana yaz: adım Ali", "hafızandan isim bilgimi sil") tek
adımda (ikinci bir onay adımı olmadan) bellek yazabilsin/silebilsin istedi. Yeni modül
`src/memory_intent.rs` (`parse_memory_intent`) sabit bir tetikleyici cümle listesine karşı
kullanıcının ham girdisini eşleştiriyor — "sohbetten otomatik kalıcı yazma yasağı" burada da
geçerli: karar modelin kendisine değil, kullanıcının yazdığı ham metne bakılarak veriliyor,
slash komutlarının çalışma şekliyle birebir aynı ilke. Hem TUI hem native desktop'ta çalışıyor
(bkz. yukarıdaki "Sohbetten otomatik kalıcı yazma" bölümünün güncellenmiş hâli).

**Kalıcı sohbet geçmişi (yeni, bu ADR'nin doğrudan kapsamı dışında ama ilişkili):** `Runtime.
chat_history` artık `chat_messages` tablosunda (schema sürüm 8) diske de yazılıyor — önceden
yalnız RAM'de tutulan, JARVIS kapanınca kaybolan bir tasarımdı (bilinçli bir gizlilik tercihiydi),
kullanıcı isteğiyle tersine çevrildi. `/clear` artık hem görünen listeyi hem `chat_history`'i hem
diskteki kaydı siler — geçmiş kalıcı olduğu için "temizle" gerçek bir sıfırlama, yalnız kozmetik
değil. Bu, bellek kayıtlarının (`MemoryRecord`) tabi olduğu sensitivity/export/delete
makinesinden **ayrı** bir mekanizma — sohbet geçmişi kendi tablosunda, kendi basit kapsam/silme
kuralıyla (`Runtime::clear_chat_history`) yönetiliyor, profil/bellek şemasının bir parçası değil.

## Ek — Katmanlı bellek mimarisi: kullanıcının 5-katmanlı tasarımıyla karşılaştırma ve gerçek boşlukların kapatılması (16 Ağustos 2026)

Kullanıcı, F3 sırasında bu bellek sisteminin kendisiyle önceden birlikte tasarladığı katmanlı bir
mimariye (active/temporary context, session, task, project, long-term user memory + 6 kural:
her şeyi kaydetmeme, secret manager referansı, provenance/trust/scope/sensitivity metadata, task
izolasyonu, SQLite-first, opsiyonel semantic retrieval) dayandığını hatırlattı. Kodla karşılaştırma
üç gerçek boşluk buldu, üçü de aynı gün kapatıldı:

1. **`TrustLevel` yoktu.** `source` (provenance) ve `sensitivity` zaten vardı, "trust level" eksikti
   — çünkü bugüne kadar tek bir güven seviyesi vardı (açık kullanıcı komutu). `MemoryRecord`'a
   `trust_level: TrustLevel` (`UserAsserted`/`Imported`) eklendi. `propose_memory`'nin imzası
   **değişmedi** — yeni `propose_memory_with_trust_and_scope` bunu taşıyor, `propose_memory` ona
   `(UserAsserted, None)` ile yönleniyor, 24 çağrı noktasının hiçbiri etkilenmedi.
2. **Task-scoped izolasyon yoktu.** `MemoryRecord.scope_id: Option<String>` eklendi (yalnız `Task`
   namespace için anlamlı, `validate_memory_record` artık zorunlu kılıyor — `Session`/
   `EphemeralToolOutput`'un `expires_at` zorunluluğuyla aynı yapısal desen). `memory_id` artık
   `scope_id`'yi de içeriyor, iki task'ın aynı anahtarı asla birbirini ezmiyor.
   `SqliteStore::retrieve_memory` artık `task_scope: Option<&str>` alıyor — sıradan sohbet turu
   (`task_scope=None`) `Task` namespace'ini **tamamen hariç tutuyor** (önceden tüm task'ların tüm
   kayıtları her sohbet turuna karışıyordu); yeni `Runtime::task_scoped_memory_context(task_id)`
   yalnız o task'ın kayıtlarını döner.
3. **Session/Task/Project'e gerçek bir yazma yolu yoktu.** `/remember` her zaman `UserProfile`'a
   yazıyordu; diğer dördü şema olarak vardı ama hiçbir üretim yolu onlara yazmıyordu (yalnız
   `/memory import`, dolaylı bir yol). `/remember [profil|proje|görev <task-id>|oturum|geçici]
   anahtar = değer` eklendi — geriye dönük uyumluluk için, namespace kelimesini soyduktan sonra
   gerçek bir "anahtar = değer" kalmıyorsa (örn. kullanıcının anahtarı gerçekten "proje" ise) eski
   davranışa (UserProfile, orijinal metin) düşülüyor.

Ayrıca **Secret Manager** eklendi (kuralın "secret'ları doğrudan hafızaya yazmıyoruz, sadece
referans tutuyoruz" kısmı — daha önce hiç yoktu, ADR-0003'ün "Şifreleme kararı" bölümündeki genel
şifreleme-eklenmeme kararından **ayrı ve ek** bir mekanizma):
- Yeni `secrets` tablosu (`memories`'ten tamamen ayrı, schema sürüm 10) — gerçek değer yalnız
  burada. `Runtime::remember_secret` bunu yazar ve `memories`'e yalnız bir **yer tutucu** satır
  ekler (`sensitivity=Sensitive`, `include_in_model_context=false` — sıradan sohbet bağlamı bunu
  hiç görmez). Gerçek değer yalnız `Runtime::reveal_secret`'ın açık, kullanıcı-tetiklemeli
  çağrısıyla (`/secret show <anahtar>`) ortaya çıkar.
- Doğal dil: ayrı, öncelikli bir tetikleyici kümesi (`hafızana gizli kaydet: ...` vb.,
  `src/memory_intent.rs`) — sıradan `REMEMBER_TRIGGERS`'tan bilinçli olarak ayrı, bir kimlik
  bilgisinin yanlışlıkla sıradan yola gitmemesi için.
- TUI: `/secret anahtar = değer`, `/secret show <anahtar>`, `/secret forget <anahtar>`, `/secrets`
  (yalnız anahtarları listeler).
- Audit: yalnız anahtar adı, gerçek değer asla (F3 "filtre loglanır ama sır saklanmaz" ilkesiyle
  aynı desen).

Kanıt: 5 yeni `lib.rs` testi (`trust_level_distinguishes_direct_writes_from_imports`,
`task_scoped_memory_isolates_concurrent_tasks_from_each_other_and_from_ordinary_context`,
`remembering_a_secret_never_stores_the_real_value_in_ordinary_memory`,
`a_remembered_secret_never_reaches_a_real_conversation_turn` — gerçek bir sohbet turu üzerinden
uçtan uca, `forgetting_a_secret_removes_both_the_real_value_and_its_placeholder`) + doğal dil ve
TUI/desktop seviyesinde uçtan uca testler. Tam paket (`cargo fmt`/`test`/`clippy -D
warnings`/`release_check.sh`) her aşamada PASS. Ayrıntılar için `DEVELOPMENT_PLAN.md`'nin "F3
sonrası düzeltmeler" bölümüne bakın.
