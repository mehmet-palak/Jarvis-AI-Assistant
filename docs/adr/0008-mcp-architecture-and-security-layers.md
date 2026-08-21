# ADR-0008: MCP mimarisi ve güvenlik katmanları

Durum: Önerildi / tasarlandı — 21 Ağustos 2026.
Kısmen uygulandı (yalnız **ingress**: protokol sürümleme + sır/kimlik-bilgisi yanıt redaksiyonu,
commit 62dfe0a). **Egress** (JARVIS'in dış araçları kullanması) ve aşağıdaki kalan sertleştirme
katmanlarının tamamı **henüz uygulanmadı**; bu belge tasarım kararını sabitler ve **F10'da**
(pentest + kodlama derinleşmesiyle birlikte) gözden geçirilip uygulanacaktır.

## Bağlam

MCP (Model Context Protocol) yayınlanmış bir standarttır — protokolü biz icat etmiyoruz, ona
uyuyoruz. JARVIS'te MCP **iki yönlü** olur ve iki yönün güven duruşu **zıttır**; bütün tasarımın
kalbi bu asimetridir:

- **Sunucu yönü (ingress) — "JARVIS bir MCP aracı olur".** Dışarıdaki bir istemci (ör. Claude
  Desktop) JARVIS'in yeteneklerini araç olarak çağırır. *İstek* dışarıdan gelir (güvenilmez), ama
  *çalıştıran* JARVIS'in kendi güvenli kodudur. Risk: dışarıdaki biri JARVIS'i kandırıp bir şey
  yaptırmak ya da sır sızdırmak ister.
- **İstemci yönü (egress) — "JARVIS dış araçları kullanır".** JARVIS dışarıdaki bir MCP sunucusunu
  (hava durumu, dosya, GitHub sunucusu vb.) çağırır. *İstek* JARVIS'in kendisindendir (güvenilir),
  ama *çalıştıran dış, güvenmediğimiz koddur* ve *dönen yanıt* modele geri akacak güvenilmez
  veridir. Risk: kötü niyetli/ele geçirilmiş sunucu (a) yanıtına prompt-injection koyup JARVIS'i
  yönlendirir, (b) gönderdiğimiz veriyi kaçırır, (c) verdiğimiz izinle zarar verir.

Ingress'i mevcut istek-policy-task hattı zaten büyük ölçüde çözüyor. Yeni tasarımın neredeyse
tamamı **egress**'tir — dış, güvenilmez kodun ilk kez JARVIS'e girdiği yer.

## Çatı kural (invariant)

> **MCP bir taşıma katmanıdır, asla bir yetki değil.**

- **Ingress:** bir MCP isteği, sıradan bir yerel CLI isteğinin yapamayacağı hiçbir şeyi yapamaz —
  aynı policy kapısı, aynı onaylar, aynı approval-gated capability'ler. MCP ingress'in normal istek
  hattı üzerinde **hiçbir ayrıcalığı yoktur.**
- **Egress:** dış bir MCP aracı, izole edilmiş güvenilmez bir **veri kaynağından** ibarettir —
  çıktısı **veridir, talimat değil** (`ContentProvenance::ToolOutput`); kod worker'ımızla **aynı F4
  hapsinde** koşar; yalnız kullanıcının açıkça izin verdiğine dokunabilir.

Bu kural sayesinde MCP, "sıfırdan yeni bir güvenlik dünyası" değil, "mevcut sağlam evin üstüne
onaylı, izlenen bir kapı"dır. Aşağıdaki her katman, zaten kurulmuş bir parçanın yeniden
kullanımıdır.

## Karar — katmanlı tasarım (savunma derinliği)

Parantez içinde her katmanın **yeniden kullandığı mevcut parça** belirtilmiştir.

### Katman 0 — Taşıma & Protokol *(standarttan)*
- Birincil taşıma **stdio** (yerel alt-süreç). Ağ taşımaları (HTTP/SSE) bilinçli olarak **erteli** —
  "sunucu/internet ertelendi" kararıyla tutarlı.
- JSON-RPC 2.0. Protokol sürümü hem ingress'te (yapıldı: `validate_mcp_protocol_version`) hem
  egress'te doğrulanır: **dış sunucunun bildirdiği** `protocolVersion` de kontrol edilir, bilinmeyen
  reddedilir (ingress'in simetriği).

### Katman 1 — Kimlik & Manifest *(F7 HMAC-imzalı scope deseni)*
- JARVIS bağlandığı her dış sunucuyu **yerel, kullanıcıya ait bir kayıt defterinde** (`mcp_servers`
  tablosu) açıkça tanır — otomatik keşif **yok**, deny-by-default.
- Her sunucunun bir **manifesti** vardır: id, başlatma komutu/argümanları, bildirdiği araç listesi,
  izin kapsamı, ve manifestin **içerik hash'i / imzası**. Bağlanırken hash yeniden hesaplanır;
  manifest (ya da işaret ettiği binary) kullanıcının onayladığından beri değiştiyse JARVIS
  **reddeder ve yeniden onay ister.**

### Katman 2 — Rıza & İzin *(F5 yazılı-onay disiplini; bizim katmanımız, standartta yok)*
- Her sunucu bir **yetenek beyaz-listesine** ve bir **veri-hassasiyeti tavanına** eşlenir. Örn. hava
  durumu sunucusu: `Public` üstü hiçbir şey alamaz, yalnız metin döndürür, durum değiştiren hiçbir
  yeteneği tetikleyemez.
- İlk kullanımda (veya manifest değişince) kullanıcıya bir **izin ekranı** çıkar; sunucunun kendini
  nasıl tanıttığı (araç açıklamaları dahil, bkz. Katman 6) **aynen gösterilir**. Yazılı onay audit'e
  yazılır.
- **Onay-hatırlama granülerliği** (onay yorgunluğu ↔ fazla-geniş onay dengesi): onay
  `(sunucu, araç, hassasiyet, argüman-şekli)` düzeyinde hatırlanır. Bir araç bir argüman için
  onaylandı diye, kötü niyetli farklı bir argümanla otomatik onaylanmış **sayılmaz**.

### Katman 3 — Sandbox *(F4 hapishanesi, ADR-0001)*
- Dış MCP sunucu süreçleri **mevcut F4 sandbox'ında** koşar: bwrap + cgroup v2 (bellek/CPU) +
  seccomp-bpf + overlay dosya sistemi + varsayılan-ağsız namespace. Dış bir MCP aracı, teknik olarak
  **kod worker'ımızın ta kendisidir** — farklı yük, aynı hapis.
- Ağı yalnız manifesti bildirip kullanıcı onayladıysa alır (deny-by-default net namespace). Yerel
  dosya sunucusu hiç ağ almaz.
- **TOCTOU:** hash-kontrol ile exec arası binary takası riskine karşı, mümkün olduğunca sabitlenmiş/
  kopyalanmış bir artefakttan çalıştırılır.

### Katman 4 — Veri akış kontrolü (iki yön) *(mevcut sır filtresi + `ContentProvenance`)*
- **Dışarı (JARVIS → araç):** argümanlar gönderilmeden önce sır/kimlik-bilgisi filtresinden geçer
  (bugünkü yanıt redaksiyonunu **dışarı yönde** de kullanırız). Hassasiyet tavanı zorlanır.
- **İçeri (araç → JARVIS):** dönen her bayt `ContentProvenance::ToolOutput` (güvenilmez) etiketlenir;
  modele "talimat değil, veri" olarak sarmalanır (attachment/workspace için kullandığımız aynı
  savunma) — prompt-injection'ı **yapısal olarak** etkisizleştirir. Ayrıca boyut sınırı + içeri gelen
  sırra karşı tarama.
- **Bağlam minimizasyonu:** sınırı yalnız **açıkça scope'lanmış argüman** geçer. Konuşma geçmişi,
  bellek, profil **asla** bir MCP aracına sızmaz.

### Katman 5 — Audit & İptal *(mevcut hash-zincirli audit + F9 süreç-grubu öldürme)*
- Her MCP etkileşimi (bağlanma, çağrı, hassasiyet kararı, redaksiyon, yanıt provenance'ı) zincire
  yazılır; `/audit-export` ile dışa aktarılır.
- `/mcp revoke <sunucu>` sunucuyu anında kapatır, hapisteki süreç **grubunu** öldürür (F9). Kural
  ihlali yapan sunucu (injection döndürür, kotayı aşar, izinsiz ağ dener, izinsiz sampling ister)
  otomatik **karantinaya** alınır.
- **Global kapatma anahtarı:** `/mcp off` tüm egress'i anında durdurur.

## Karar — MCP yüzeyinin tamamı (yalnız `tools/call` değil)

MCP dört primitif taşır; her birinin ayrı güven sonucu vardır. Erken bir tasarım hatası bunu yalnız
"araç çağırma" sanmaktı:

- **Tools (araçlar):** yukarıdaki katmanların ana konusu.
- **Resources (kaynaklar):** sunucunun sunduğu dosya/veri — güvenilmez içerik, araç çıktısı gibi
  provenance etiketlenir.
- **Prompts (şablonlar):** sunucunun verdiği hazır prompt şablonları — **doğrudan modele giren
  metin**, yani injection yüzeyi. Güvenilmez sarmalanır ya da reddedilir.
- **Sampling:** MCP'de bir dış sunucu, *istemciden* (JARVIS'ten) kendi adına bir **LLM tamamlaması
  çalıştırmasını** isteyebilir — yani dış, güvenmediğimiz bir sunucu, yerel modeli saldırganın
  seçtiği bir prompt'la koşturabilir. Bu düpedüz bir **yetki-yükseltme kanalıdır**. Karar:
  **sampling deny-by-default**; ancak çok özel, açıkça onaylı, izole ve provenance-etiketli bir
  yolla açılabilir. Çatı kuralın ("MCP yetki değildir") en çok zorlandığı yer burasıdır.

## Karar — injection ve bileşim yüzeyleri

- **"Tool poisoning" (çağrıdan önce injection):** sunucunun `tools/list` ile verdiği araç
  **açıklamaları** güvenilmez metindir ve model *ne zaman hangi aracı çağıracağına* karar verirken
  bunları okur. Kötü niyetli bir açıklama, hiç araç çağrılmadan modeli zehirler. Karar: araç
  açıklamaları da güvenilmez provenance'la işaretlenir ve **onay ekranında kullanıcıya aynen
  gösterilir**.
- **Ajans döngüsü bileşim riski (confused deputy):** model, A aracının güvenilmez çıktısını okuyup
  B aracını çağırmaya karar verebilir; saldırgan A'nın çıktısıyla JARVIS'i B'yi kendi seçtiği
  argümanlarla çağırmaya yönlendirir. Karar: **döngünün ortasında oluşan her yeni araç çağrısı da
  policy kapısından + hassasiyet tavanından yeniden geçer** — "kullanıcı en başta onayladı"
  sayılmaz.
- **Sunucular-arası bilgi-akış kontrolü (kaçırma zinciri):** tek tek güvenli iki sunucu birlikte
  tehlikeli olur — Sunucu A veriye erişir (ağsız), Sunucu B ağa erişir (verisiz); A'nın çıktısı B'ye
  argüman verilirse veri dışarı kaçar. Karar: veri **etiketi araç grafiği boyunca akar**;
  yüksek-hassasiyetli bir kaynaktan gelen veri, ağ-yetkili düşük-güvenli bir hedefe
  **yönlendirilemez** (klasik information-flow control). Tek-sunucu tavanı bunu yakalamaz.

## Karar — tedarik zinciri, kimlik-bilgisi, kanallar, kalıntı, eşzamanlılık

- **Tedarik zinciri / "rug pull":** çoğu MCP sunucusu `npx`/`pip` ile çekilir ve `npx` sık sık **her
  çalıştırmada en son sürümü** indirir — bu, manifest hash kilidini baypas eder (bugün iyi huylu,
  yarın kötü niyetli). Karar: sunucunun **tam sürümü/binary hash'i sabitlenir** (manifest metni
  değil, çalışan kodun kendisi); bir sunucu **eklemek/güncellemek başlı başına yazılı onay gerektiren
  bir güven olayıdır.**
- **Sunucuya kimlik-bilgisi akışı:** bazı sunucular meşru olarak token ister (GitHub → PAT). Token,
  sandbox'a **env var olarak** enjekte edilir, **asla loglanmaz**, yalnız o sunucuya scope'lanır,
  iptal edilebilir. Secret Manager'daki bir sır, o sunucu için açıkça onaylı bir kimlik-bilgisi
  değilse asla bir MCP argümanı olamaz.
- **Hata & stderr kanalları:** redaksiyon yalnız başarı yanıtına değil, **JSON-RPC hata nesnesine**
  ve sunucunun **stderr'ine** de uygulanır — ikisi de ayrı sızıntı/injection yüzeyidir.
- **İptal sonrası veri kalıntısı (F9 veri-egemenliği):** bir sunucu iptal edilince audit girdileri
  **kalır** (tanık), ama verilmiş **kimlik-bilgileri silinir**; belleğe/RAG'a düşmüş çıktıları
  `/forget` disipliniyle temizlenebilir.
- **Eşzamanlılık:** eşzamanlı MCP çağrıları altında audit **hash zincirinin doğrusal ve tutarlı**
  kaldığı; runtime'ın thread-güvenliği — gerçek testle kanıtlanacak bir madde.

## Karar — çekirdek değişmezinin korunması

MCP **tümüyle opsiyonel, opt-in bir katmandır.** F9'da kanıtlanan "hiçbir çekirdek yetenek ağ
gerektirmez" değişmezi korunur: bir MCP sunucusu **asla** bir çekirdek işlevin ön-koşulu olamaz;
JARVIS MCP olmadan da tam çalışır.

## Yapım sırası (her faz gerçek-kanıt testiyle biter — projenin disiplini)

1. **Egress iskeleti:** `mcp_servers` kayıt defteri + manifest + deny-by-default bağlanma + dış
   sunucu protokol kontrolü. Henüz çalıştırma yok.
2. **Sandbox bağlama:** dış sunucuyu F4 hapsinde başlat; ağ/fork/aşırı-boyut/sampling deneyen
   **düşmanca bir test sunucusuyla** gerçekten kapatıldığını kanıtla.
3. **Rıza + izin ekranı:** TUI onay akışı + yetenek beyaz-listesi + hassasiyet tavanı + granüler
   onay-hatırlama.
4. **Veri akış filtreleri:** dışarı sır/hassasiyet koruması + içeri provenance etiketi + boyut sınırı
   + bağlam minimizasyonu + sunucular-arası akış kontrolü.
5. **Audit + iptal + karantina + global kapatma + eşzamanlılık testi.**
6. **MCP yüzeyinin tamamı:** resources/prompts güvenilmez ele alma, **sampling deny-by-default**;
   ingress bitirişi (güven sınırının belgelenmesi, "hattın üstünde yetki yok" değişmezinin testle
   sabitlenmesi).

## Tehditler ve sınırlar (açıkça korumadığımız şeyler)

- Verilen iznin **içinde kalarak** zarar veren bir sunucuya karşı koruyamayız (kullanıcı bir şeye
  açıkça izin verdiyse, o şey yapılır) — bu yüzden izin ekranı ve granülerlik kritik.
- Sunucunun **iç doğruluğunu** doğrulamayız; onu bir kara-kutu güvenilmez veri kaynağı sayarız.
- Sandbox, F4'ün aynı **çekirdek-özelliği uyarılarına** tabidir (cgroup v2 / user namespace / seccomp
  hedef makinede mevcut olmalı; yoksa özellik kapalı kalır, host fallback yoktur).
- Ağ taşımaları (uzak MCP) kapsam dışıdır — yalnız yerel stdio tasarlandı.

## Açık sorular (F10'da netleşecek)

- Onay-hatırlamanın "argüman şekli" granülerliği pratikte nasıl tanımlanır (şema mı, örüntü mü)?
- Sunucular-arası bilgi-akış etiketleri hangi ayrıntıda taşınır — kaba (kaynak-başına) mı, ince
  (alan-başına) mı?
- Sampling'i açtığımız "çok özel onaylı yol" gerçekten gerekli bir kullanım mı, yoksa tümden
  kapalı mı kalmalı?
- Düşmanca test sunucusu hangi saldırı repertuvarını kapsamalı (asgari kanıt kümesi)?

## Sonuç

MCP güvenliği "yeni bir ev" değil, mevcut omurganın (policy kapısı, F4 hapis, F5 onay, F7 imzalı
scope, F9 audit/süreç-öldürme/veri-egemenliği, mevcut provenance/secret filtreleri) yeniden
kullanımıyla kurulan **onaylı, izlenen bir kapıdır.** Ingress ~%80 hazır (protokol + redaksiyon
uygulandı); esas iş egress ve bu belgedeki genişletilmiş sertleştirme katmanlarıdır. Uygulama
**F10'a** ertelendi; bu ADR o güne kadar tasarım kararını ve açık soruları sabitler.
