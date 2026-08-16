# ADR-0001: Kod patch worker'ı host fallback kullanmaz

Durum: Kabul edildi — 14 Ağustos 2026 (16 Ağustos 2026'da runtime quota/ek namespace izolasyonuyla genişletildi, bkz. "Ek" bölümü)

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

## Ek — Runtime quota ve ek namespace izolasyonu (16 Ağustos 2026)

F4'e devam edilirken bulunan gerçek bir boşluk: `WorkerLimits.max_runtime_seconds`/
`max_output_bytes` alanları vardı ama hiçbir yerde okunmuyordu — asılı/patolojik bir `git apply`
süresiz bloke olabilirdi, gerçek bir watchdog yoktu.

**Kapatıldı:**
- `run_git_apply` artık `Child::wait_with_output`'a kör kör bloke olmuyor; `wait_with_timeout`
  (yeni, ayrı test edilebilir fonksiyon) `try_wait()` ile yoklayıp kota aşılınca süreci gerçekten
  öldürüyor (`kill()` + reap). Gerçek bir "asılı" süreçle (`sleep`) test edildi — `git apply`'in
  kendi davranışına bağlı olmayan, doğrudan watchdog testi.
- `max_output_bytes` artık hatada gösterilen stderr önizlemesini sınırlıyor (görüntü-seviyeli bir
  sınır — süreç çalışırken canlı bir "taşınca öldür" değil; gerekçe: `git apply`'in stderr'i zaten
  `max_diff_bytes` (256 KiB) ile dolaylı sınırlı, canlı bir akış sınırı ancak henüz var olmayan bir
  keyfi komut çalıştırıcıda (F4 "Allowlist command runner") anlamlı olur).
- Bubblewrap çağrısına üç ek, blob/derleme gerektirmeyen namespace bayrağı eklendi:
  `--unshare-pid`, `--unshare-ipc`, `--unshare-uts` — F4 tehdit modelinin "process tree" maddesini
  doğrudan hedefliyor (worker artık host'taki başka süreçleri göremez/sinyalleyemez).

**Hâlâ açık (F4'ün ileri maddeleri):**
- Gerçek CPU/RAM/disk kotası (cgroups gerektirir — henüz yok).
- Seccomp filtresi — bir BPF programı üretmek/doğrulamak, bu ADR'nin kapsamı dışında ayrı bir
  mühendislik işi; bilinçli olarak ertelendi.
- Snapshot/overlay worker: şu an gerçek workspace'e doğrudan read-write bağlanıp (snapshot+rollback
  ile) yazılıyor, gerçek bir copy-on-write overlay dosya sistemi değil — daha basit ama farklı bir
  tasarım tercihi, henüz overlay'e geçilmedi.
- Gerçek cancellation (kullanıcı bir işlemi ortasında iptal edebilsin) — henüz yok.
- **Doğrulama sınırı**: bu geliştirme ortamının container'ı `CLONE_NEWNET` izni vermediği için (ADR-0001'in orijinal metninde de not edildi), gerçek `bwrap` çağrısı bu ortamda uçtan uca çalıştırılıp doğrulanamadı — yalnız derlendiği (release profili dahil) doğrulandı. Gerçek doğrulama, ADR'nin öngördüğü gibi hedef makinede release smoke ile yapılmalı.

## Ek 2 — Gerçek cancellation, resource rlimit'leri, patch generator ve uçtan uca TUI zinciri (16 Ağustos 2026, F4 tamamlama oturumu)

Yukarıdaki "Hâlâ açık" listesinden ikisi kapatıldı, ikisi kısmen kapatıldı, ikisi hâlâ açık kalıyor.

**Kapatıldı:**
- **Gerçek cancellation**: `wait_with_deadline_and_cancellation` — bir `CancelFlag` (`Arc<AtomicBool>`)
  dışarıdan `true` yapıldığında ya da süre kotası dolduğunda, her ikisinde de önce `SIGTERM`, 200ms
  grace period, hâlâ canlıysa `SIGKILL`. TUI'de `/abort` komutu bunu tetikliyor — `Runtime::cancel`'dan
  (bir task başlamadan önce iptali) tamamen ayrı bir mekanizma, hâlâ çalışan izole bir süreci
  ortasında durduruyor. Gerçek `sleep`/`SIGTERM`'i trap'leyen bir `sh` süreciyle test edildi.
- **Allowlist command runner**: `src/command_runner.rs` — sabit program/alt-komut izin listesi, hiçbir
  shell'e girmiyor, `git apply` ile aynı bwrap+rlimit+watchdog izolasyonunu paylaşıyor. Artık TUI'de
  `/approve-patch`'in test/verifier adımı olarak üretimde kullanılıyor (kütüphanede durmuyor).

**Kısmen kapatıldı:**
- **Resource kontrolü**: gerçek `setrlimit` (`RLIMIT_AS`/`RLIMIT_FSIZE`/`RLIMIT_NPROC`/`RLIMIT_CPU`)
  artık her izole worker çağrısına uygulanıyor — cgroups değil ama kök yetkisi gerektirmeyen, çekirdek
  tarafından gerçekten uygulanan bir alternatif. Toplam workspace disk kullanımı için hâlâ gerçek bir
  kota yok (yalnız tek-dosya `RLIMIT_FSIZE`).
- **Snapshot/overlay worker**: tam bir copy-on-write overlay hâlâ yok, ama artık snapshot/rollback
  mekanizması yalnız "apply başarısız oldu" durumuna değil, "testler geçmedi ya da iptal edildi"
  durumuna da genişletildi (`Runtime::run_coding_tests_and_finalize`) — kullanıcı için nihai sonuç
  aynı (test geçmezse dosyalar tam olarak eskisi gibi kalır), yalnız mekanizma overlay değil.
  Bilinçli bir tasarım kararı: her patch denemesinde tüm workspace'i kopyalamak yerine mevcut,
  zaten test edilmiş snapshot altyapısını genişletmek tercih edildi.

**Hâlâ açık:** gerçek CPU/RAM/disk cgroups kotası, seccomp filtresi, gerçek copy-on-write overlay.

**Yeni bu turda:** `src/patch_generator.rs` — modelin diff sözdizimi üretmesi hiç istenmiyor (küçük
yerel modeller güvenilmez), bunun yerine tam dosya içeriği isteniyor, gerçek diff `git diff --no-index`
ile makine tarafında hesaplanıyor. `/plan → /patch → /approve-patch` zinciri artık TUI'de uçtan uca
çalışıyor; onay sonrası izole uygulama + izole test çalıştırma + başarısız/iptal edilen testte otomatik
rollback + her adımın audit zincirine yazılması dahil. Ayrıca gerçek, önceden fark edilmemiş bir bug
bulunup düzeltildi: `ModelProvider::complete()` üretimde 8 token'a sabitliydi, uzun yanıtları (test
planı satırı, tam dosya yeniden yazımı) sessizce kırpıyordu — `complete_with_budget` eklendi, model
sunucusunun context penceresi 2048'den 8192'ye çıkarıldı. Detaylı kanıt: `DEVELOPMENT_PLAN.md`, F4
bölümü.
