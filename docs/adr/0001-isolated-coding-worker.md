# ADR-0001: Kod patch worker'ı host fallback kullanmaz

Durum: Kabul edildi — 14 Ağustos 2026

## Bağlam

JARVIS'in bir patch'i önermesi ile kullanıcının çalışma alanına yazması farklı yetkilerdir. Model
çıktısı, diff görünümünden geçmiş olsa bile shell ya da ağ yetkisi kazanmamalıdır. Bir worker'ın
izolasyonu başlatılamadığında host üzerinde çalışmaya devam etmesi, bu sınırı anlamsızlaştırır.

## Karar

- Patch yalnız `CodingPlan` içindeki workspace-relative dosyalara yönelik unified git diff olur.
- Diff SHA-256 ile bağlanır. `ApprovedPatch`, yalnız o proposal ID ve hash'i için geçerlidir.
- Uygulamadan önce `git apply --check` çalışır ve her değişecek dosyanın geçici dizinde snapshot'ı
  alınır.
- Release build'de `git apply`, `/usr/bin/bwrap` içinde başlatılır: network namespace ayrılır,
  environment temizlenir, sadece runtime kütüphaneleri read-only bağlanır, workspace kontrollü
  biçimde bağlanır ve `/tmp` özeldir.
- `bwrap`, `git`, gerekli runtime bağları veya network namespace kurulamazsa işlem **reddedilir**.
  Host shell fallback yoktur.
- Uygulama başarısız olursa snapshot otomatik geri yüklenir. Başarılı uygulamanın rollback'i ayrı
  bir kullanıcı kararıdır; snapshot kullanıcı tarafından atılana kadar saklanır.
- Test derlemelerinde CI/container `CLONE_NEWNET` izni vermeyebildiğinden semantic patch testleri
  kontrollü geçici klasörde çalışır. Bu test yolu release binary'de bulunmaz.

## Tehditler ve sınırlar

Bu dilim path traversal, diff hash değiştirme, plan dışı dosya, binary/rename/new/deleted-file
diff'leri ve ağ erişimini hedefler. Henüz CPU/RAM/PID quota, uzun iş cancellation, seccomp ve
snapshot overlay worker tamamlanmamıştır; bunlar F4'ün açık maddeleri olarak kalır. `bwrap`
network namespace desteği hedef makinede release smoke ile ayrıca doğrulanır.

## Sonuç

JARVIS kullanıcı onayı olmadan yazamaz; onay yanlış diff'e genişleyemez. İzolasyon kanıtı olmayan
makinede kod değiştirme özelliği kapalı kalır.
