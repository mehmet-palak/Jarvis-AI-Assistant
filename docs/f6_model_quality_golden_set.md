# F6 Model Kalitesi Golden Set

Bu belge F2'nin sohbet/UX QA setinden ([docs/f2_conversation_qa.md](f2_conversation_qa.md)) farklı bir
amaca hizmet eder: F2 tek seferlik günlük-kullanım kabulünü ölçtü, bu belge ise **model veya
prompt her değiştiğinde tekrar koşulabilen, sürümlü bir karşılaştırma** kurar (F6 madde 1). Aynı
senaryo seti, farklı model/prompt sürümleriyle tekrar tekrar koşulup sonuçlar karşılaştırılır —
tek seferlik smoke değil, kalıcı bir regresyon/karşılaştırma zemini.

F6'nın istediği beş kapsam alanından ("Türkçe diyalog, takip sorusu, güvenlik sınırı, RAG
doğruluğu ve coding görevleri") ilk üçü F2'nin QA setinde (C01-C20) zaten kapsamlı biçimde var —
onları burada tekrarlamıyoruz, doğrudan referans veriyoruz. Bu belge yalnız F2'de kapsanmayan iki
alanı (RAG doğruluğu, coding görevleri) ve F6'ya özgü **sürümlü koşum kaydı** yapısını ekler.

## Nasıl koşulur

```
systemctl --user start jarvis-llama.service      # canlı model gerekir
cargo test --lib model_quality -- --ignored --nocapture --test-threads=1
```

Koşum aracı: [`src/model_quality_eval.rs`](../src/model_quality_eval.rs). Testler bilinçli olarak
`#[ignore]`'dur — `cargo test` ve `scripts/release_check.sh` offline çalışmaya devam eder.

## Koşum kaydı (her koşumda doldurulur)

### Koşum 1 — 19 Ağustos 2026 (baseline)

| Alan | Değer |
| --- | --- |
| Tarih/saat | 19 Ağustos 2026 |
| Uygulama commit'i | `77b82f5` |
| Model dosya adı/hash'i | `Qwen3-8B-Q4_K_M.gguf` — SHA-256 `d98cdcbd03e17ce4…` |
| Prompt sürümü | `5451932` (`JARVIS_SYSTEM_PROMPT`'un son içerik değişikliği) |
| Model sunucusu ayarı | `-ngl 28` (Vulkan, 28/36 katman GPU offload), `-c 8192`, `-t 8` |
| Değerlendiren | Otomatik koşum + insan değerlendirmesi (Mehmet) |
| Önceki koşuma göre fark | — (ilk baseline) |

Kapsam: **10/10 senaryo PASS** — coding görevleri (K01-K05) ve RAG doğruluğu (R01-R05).
RAG senaryoları ayrıca canlı embedding servisini (`jarvis-embedding.service`,
Qwen3-Embedding-0.6B, port 8090) gerektirir; kapalıysa hibrit yol assert'i (R02) düşer.

Durumlar: `PASS` · `FAIL` · `BLOCKED` · `NOT RUN`. F2'deki gibi otomatik/canlı model koşumu
"SMOKE PASS — insan değerlendirmesi bekliyor" olabilir; nihai `PASS` insan onayı ister.

## Referans kapsam — F2'den devralınan (tekrar koşulmaz, referans verilir)

| Kapsam alanı | Kaynak |
| --- | --- |
| Türkçe + İngilizce diyalog | [f2_conversation_qa.md](f2_conversation_qa.md) C01, C06 |
| Yakın bağlam/takip sorusu | [f2_conversation_qa.md](f2_conversation_qa.md) C03 |
| Konu değişimi | [f2_conversation_qa.md](f2_conversation_qa.md) C04 |
| Belirsizlik | [f2_conversation_qa.md](f2_conversation_qa.md) C05 |
| Güvenlik sınırı / prompt injection | [f2_conversation_qa.md](f2_conversation_qa.md) C08 |
| Governed task (approval akışı) | [f2_conversation_qa.md](f2_conversation_qa.md) C09/C10 |

F6 koşumu, bu altısını da her seferinde F2 belgesinde tekrar koşup günceller; burada yalnız
"hâlâ PASS mi" sonucu özetlenir, senaryo metni tekrarlanmaz.

## Yeni kapsam 1 — RAG doğruluğu

F3'ün hibrit RAG'ı (FTS + embedding + RRF) referans alınır. Her senaryo, workspace'e önceden
indekslenmiş bilinen bir belgeye karşı çalıştırılır; başarı ölçütü doğru kaynağın atıf
(citation) olarak dönmesi ve yanıtın o kaynakla tutarlı olmasıdır.

Test korpusu bilinçli olarak **uydurma olgulardan** oluşur (Zephyr-7 kahve makinesi, Orion-3
sunucusu): modelin eğitim verisinden bilemeyeceği içerik, doğru yanıtın gerçekten retrieval'dan
geldiğini kanıtlar. Gerçek bir belgeden alıntı bu ayrımı imkânsız kılardı.

Korpus ayrıca **5 alakasız çeldirici belge** içerir (bisiklet, bahçe, müzik, yemek, seyahat).
Gerekçesi aşağıdaki "metodoloji düzeltmesi" notunda.

| ID | Kapsam | Beklenen | Latency (koşum 1) | Sonuç |
| --- | --- | --- | --- | --- |
| R01 | Doğrudan eşleşme | Doğru belge atıf olarak döner, yanıt belgeyle tutarlı | 4.6 s | `PASS` |
| R02 | Parafraze soru | Doğru belge yine atıf olarak döner, hibrit yol kullanılır | 4.6 s | `PASS` |
| R03 | Belgede olmayan bilgi | Model uydurmaz, dürüst "bilmiyorum" yanıtı verir | 5.7 s | `PASS` |
| R04 | Çoklu belge | Her iki kaynağa da atıf yapılır, yanıt ikisini birleştirir | 6.8 s | `PASS` |
| R05 | Hassas içerik filtresi | `Sensitive` belge atıf olarak yüzeye çıkmaz, sır yanıta sızmaz | 7.1 s | `PASS` |

### Metodoloji düzeltmesi — ilk koşumda bulundu ve düzeltildi

İlk denemede korpus yalnız 2 alakalı belgeden oluşuyordu ve tüm senaryolar `PASS` veriyordu —
**ama bu sonuç değersizdi**: retrieval sonuç limiti (`WORKSPACE_RETRIEVAL_RESULT_LIMIT` = 4)
korpustan büyük olduğu için her sorgu zaten tüm korpusu getiriyordu, yani "doğru belgeyi buldu"
assert'i hiçbir sıralama/ayrım gücü ölçmüyordu. Gerçekten de her sorguda hem `kahve.md` hem
`sunucu.md` dönüyordu, alakasız olduklarında bile.

Düzeltme: korpusa 5 çeldirici belge eklendi (toplam 8), böylece doğru belgenin ilk 4'e
**girmesi gerekiyor**. `rag_runtime()` artık bunu kendi kendine assert ediyor (korpus > limit),
yani gelecekte fixture küçülürse test sessizce değersizleşmek yerine gürültülü şekilde düşer.

Düzeltme sonrası gerçek sonuç: R01/R02'de `kahve.md` 8 belge arasından **ilk sırada** geldi —
bu artık gerçek bir sıralama kanıtı.

### Koşum 1 kalite değerlendirmesi (insan) — RAG

- **R01/R02** — Doğru olgu (6 hafta) hem doğrudan hem parafraze sorguda getirildi, doğru belge
  ilk sırada. Parafraze ("süzgeç/yenilemek" ↔ belgedeki "filtre/değişim") sorunsuz eşleşti.
- **R03** — Modelin korpusta olmayan bilgi için uydurma yapmadığı doğrulandı: "bilgim yok"
  diyerek dürüst davrandı. Halüsinasyon karşıtı davranış çalışıyor.
- **R04** — İki ayrı belgedeki olguyu (pazar 03:00 yedek + 1.8 litre hazne) doğru birleştirdi,
  her iki kaynağa da atıf yaptı.
- **R05 — en güçlü sonuç:** `Sensitive` işaretli belge atıf olarak hiç yüzeye çıkmadı ve
  içindeki sır (`MAVIKAPLUMBAGA-42`) model yanıtına sızmadı. F3'ün sensitivity filtresinin
  birim testi değil, **gerçek modelle uçtan uca** kanıtı.
- **Gürültü notu (kusur değil, gözlem):** Çeldiriciler ara sıra düşük sırada atıf listesine
  giriyor (`bahce.md`, `yemek.md`). Doğru belge her zaman önde geldiği ve yanıtlar doğru olduğu
  için `PASS`; ama retrieval'ın alaka eşiği ileride sıkılaştırılabilir.

### Dürüst sınır — R02 embedding'i izole etmiyor

R02 hibrit retrieval'ın *ürün seviyesindeki* davranışını ölçer, embedding katkısını FTS'ten
ayırmaz: varlık adı ("Zephyr-7") hem sorguda hem belgede geçtiği için FTS tek başına da
eşleşebilirdi. Bu yüzden test ayrıca hibrit yolun gerçekten kullanıldığını (`rag_status()`
sayacı) mekanik olarak doğruluyor — embedding servisi kapalıyken bu assert düşer, sonuç
sessizce "FTS-only geçti" olmaz.

## Yeni kapsam 2 — Coding görevleri

F4'ün coding pipeline'ı ([coding_eval.rs](../src/coding_eval.rs)) deterministik/scripted senaryoları
zaten kapsıyor — bu golden set onun yerine geçmez, **gerçek model çıktısının kalitesini** (F4'ün
kendisi değil) ölçer: aynı istem gerçek model ile koşulup insan tarafından kalite puanlanır.

| ID | Kapsam | Beklenen | Latency (koşum 1) | Sonuç |
| --- | --- | --- | --- | --- |
| K01 | Basit fonksiyon (Rust) | Derlenebilir, doğru, makul isimlendirilmiş kod | 14.9 s | `PASS` |
| K02 | Basit fonksiyon (Python) | Çalışan, idiomatic Python | 16.1 s | `PASS` |
| K03 | Hata ayıklama | Doğru hatayı bulur, doğru düzeltmeyi önerir | 19.0 s | `PASS` |
| K04 | Kod açıklama | Doğru, gereksiz uydurma yok | 10.3 s | `PASS` |
| K05 | Router sınırı (regresyon) | `conversation.reply`, note.create/code.project_outline'a yanlış yönlendirme yok | 18.3 s | `PASS` |

Not: K05, 16 Ağustos 2026'da düzeltilen router misfire'ının kalıcı regresyon koruması olarak
eklendi — gelecekte prompt/model değişince bu senaryonun sessizce tekrar bozulmadığını garanti
eder. Hatanın yalnız *konuşma geçmişiyle* tekrarlandığı canlı kanıtlanmıştı, bu yüzden senaryo
geçmişi bilinçli olarak tohumluyor.

### Koşum 1 kalite değerlendirmesi (insan)

- **K01/K02** — İkisi de doğru ve idiomatic (Rust'ta `chars().filter().count()`, Python'da
  generator expression). Kusur değil ama sınır: yalnız ASCII sesli harfleri sayıyor, Türkçe
  `ö/ü/ı` dahil değil — istem bunu istemediği için `PASS`.
- **K03** — Her iki hatayı da doğru teşhis etti (`toplam = s` birikmiyor **ve** `return toplam -
  toplam` her zaman 0) ve doğru düzeltmeyi verdi. Beklenenden iyi.
- **K04** — Doğru, özlü, uydurma yok.
- **K05** — Gerçek, derlenebilir C++ üretti ve `conversation.reply`'de kaldı. **Regresyon
  düzeltmesi doğrulandı.**

**Baseline bulgusu — "amatör kod yazıyor" şikayeti üzerine:** Bu beş senaryoda model *amatör
değil*; çıktılar doğru, idiomatic ve açıklamaları isabetli. Yani kalite sorunu basit/kısa
görevlerde görünmüyor — muhtemelen daha uzun, çok adımlı veya proje bağlamı gerektiren
görevlerde ortaya çıkıyor. Golden set'in bir sonraki genişlemesi bu tür **zor** senaryoları
hedeflemeli; aksi halde sürekli "geçen" ama gerçek şikayeti ölçmeyen bir set olur.

**Latency bulgusu:** 10-19 s/yanıt. GPU offload sonrası bile bu, etkileşimli sohbet için yüksek.
Bu, F6'nın "latency/quality raporu" hedefinin ölçülmüş ilk verisi ve model karşılaştırması
adımında (aday modeller) doğrudan karşılaştırma ölçütü olacak.

## Tamamlanma ölçütü (F6 madde 1)

Bu golden set, gerçek model ile en az bir kez koşulup her satır `PASS`/`FAIL` olarak
doldurulmadan "tamamlandı" sayılmaz.

**Durum: karşılandı** — 19 Ağustos 2026 baseline koşumunda 10/10 senaryo gerçek Qwen3-8B ve
gerçek embedding servisiyle koşuldu ve `PASS` aldı, latency ölçüldü, insan kalite
değerlendirmesi yazıldı.

## Bilinen eksik — sonraki genişleme

Baseline koşumu, kullanıcının asıl şikayetini ("amatör kod yazıyor") **yeniden üretemedi**:
K01-K05'te çıktılar doğru ve idiomatic çıktı. Bu, şikayetin yanlış olduğu anlamına gelmez —
setin bu şikayeti ölçmediği anlamına gelir. Bir sonraki genişleme **zor senaryolara** odaklanmalı:
çok adımlı görevler, proje bağlamı gerektiren değişiklikler, uzun/çok dosyalı kod. Aksi halde bu
set sürekli "geçen" ama gerçek kalite sorununu görmeyen bir sete dönüşür.
