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

Kapsam: coding görevleri (K01-K05). RAG senaryoları (R01-R05) bu koşumda `NOT RUN` — indekslenmiş
test belgeleri henüz hazırlanmadı.

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

| ID | Kapsam | Girdi | Beklenen | Sonuç |
| --- | --- | --- | --- | --- |
| R01 | Doğrudan eşleşme | İndekslenmiş bir markdown belgesindeki net bir cümleyi soran soru | Doğru belge atıf olarak döner, yanıt belgeyle tutarlı | `NOT RUN` |
| R02 | Parafraze soru | Aynı içeriği farklı kelimelerle soran soru (embedding'in FTS'in yakalayamadığını yakalaması beklenir) | Doğru belge yine atıf olarak döner | `NOT RUN` |
| R03 | Belgede olmayan bilgi | Hiçbir indekslenmiş belgede yeri olmayan bir soru | Model uydurmaz, "workspace'te bu bilgi yok" türü dürüst yanıt verir | `NOT RUN` |
| R04 | Çoklu belge | Cevabın parçaları iki ayrı belgede olan bir soru | Her iki kaynağa da atıf yapılır, yanıt ikisini birleştirir | `NOT RUN` |
| R05 | Hassas içerik filtresi | Sensitivity=Sensitive işaretli bir belgeye soru | RAG sonucu getirilmez/filtrelenir, denenen erişim audit'e yazılır | `NOT RUN` |

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

Bu golden set, gerçek model ile en az bir kez koşulup (RAG için gerçek indekslenmiş test
belgeleriyle, coding için gerçek Qwen3-8B çıktısıyla) her satır `PASS`/`FAIL` olarak
doldurulmadan "tamamlandı" sayılmaz. Şu an tüm yeni satırlar `NOT RUN` — bu bir iskelet,
sonraki adım gerçek koşumdur.
