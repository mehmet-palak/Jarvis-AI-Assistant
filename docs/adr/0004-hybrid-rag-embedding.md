# ADR-0004: Hibrit RAG — FTS'in yanına, üzerine değil, embedding tabanlı anlamsal arama

Durum: Kabul edildi — 15 Ağustos 2026

## Karar

JARVIS'in workspace RAG'ı artık **hibrit**: mevcut SQLite FTS5 (anahtar kelime araması, F3 madde
9-12'de kuruldu) hiç değiştirilmeden kalır; üzerine, isteğe bağlı bir embedding tabanlı anlamsal
arama katmanı eklendi. İkisi **Reciprocal Rank Fusion (RRF)** ile birleştiriliyor — biri "asıl"
diğeri "yedek" değil, her arama her iki sinyali de kullanıyor.

Model: **Qwen3-Embedding-0.6B (Q8_0 GGUF, 639 MB)**, kullanıcı onayıyla 15 Ağustos 2026'da indirildi.

## Model seçimi gerekçesi

Karşılaştırılan alternatifler: `multilingual-e5-small` (126 MB, kod-farkındalığı yok),
`granite-embedding-311m-multilingual-r2` (253 MB, kod-farkında), `EmbeddingGemma-300m` (Google,
kilitli/gated HuggingFace erişimi, kısıtlı Gemma lisansı), `jina-embeddings-v3` (CC BY-NC 4.0,
ticari/kısıtlı kullanım). Qwen3-Embedding-0.6B seçildi çünkü: resmi Qwen yayını (JARVIS'in sohbet
modeliyle aynı aile), 100+ dil + kod retrieval için özel eğitilmiş (JARVIS'in RAG kapsamı "repo,
API docs, framework docs, kişisel notlar" — PDF mimari belgesi §10), Apache 2.0 (tam serbest),
kilitsiz doğrudan indirme. Kaynak tüketimi kullanıcı tarafından öncelik dışı bırakıldı ("işinde en
iyisi olsun, boyut önemli değil") ama yine de RAM'de ~300 MB, zaten kabul edilen vision modelinden
(3B parametre) çok daha hafif.

## Mimari ilkeler (uygulanan)

- **FTS asla bozulmadı.** `search_workspace` hiç değişmedi. `hybrid_search_workspace`,
  embedding sağlayıcısı yoksa/erişilemezse aynı düz FTS sırasına döner.
- **Adapter arkasında değiştirilebilir.** `EmbeddingProvider` trait; bugün `LlamaEmbeddingProvider`
  onu uyguluyor. Model değişirse yalnız yeni bir implementasyon yazılır.
- **Türetilmiş cache, veri kaynağı değil.** `workspace_chunk_embeddings`, bir chunk silinince
  onunla birlikte temizlenir; içerikten her an yeniden hesaplanabilir.
- **Model-versiyonlu cache anahtarı.** Vektör yeniden kullanımı hem `content_sha256` hem
  `embedding_model_id`'ye göre eşleşir — farklı bir modele geçilirse eski vektör asla yanlışlıkla
  yeniden kullanılmaz (kullanıcının ChatGPT'den aldığı ve ilettiği geri bildirimle bulunan gerçek
  bir tasarım açığıydı, düzeltildi).
- **Geriye dönük doldurma.** Bir belge daha önce FTS-only indekslenmişse (embedding sağlayıcısı
  henüz yokken), embedding sağlayıcısı sonradan bağlanınca ve aynı belge tekrar
  indekslendiğinde (`/index-folder` gibi), metin değişmemiş olsa bile eksik embedding'ler otomatik
  tamamlanır — kullanıcının fark edip zorla yeniden indekslemesi gerekmez.
- **İçerik-hash ile vektör paylaşımı.** Aynı içerik (aynı model için) birden fazla dosyada/chunk'ta
  geçiyorsa, model yalnız bir kez çağrılır.
- **Provenance dedup olmaz.** Yalnız ham vektör paylaşılır; her chunk kendi `chunk_id`/
  `document_id`/`canonical_path`/`chunk_ordinal` bilgisini `workspace_chunks`/`workspace_documents`
  join'i üzerinden korur.
- **Hata toleranslı.** Embedding servisi kapalı/hatalıysa, o chunk yalnız FTS ile kalır; hiçbir
  indeksleme veya arama işlemi bu yüzden başarısız olmaz.
- **Görünür, sessiz değil.** TUI `/status`, aktif modun "FTS-only" mi "hybrid (FTS +
  <model>)" mi olduğunu açıkça gösterir.

## Servis

`jarvis-embedding.service` (systemd, port 8090, `--embedding --pooling last`, loopback-only,
CORS kısıtlı) — text/vision servisleriyle aynı desende. **Text/vision servislerinden farklı olarak
otomatik başlatılmaz** — yalnız zaten erişilebilirse `Runtime`'a bağlanır. Gerekçe: hibrit arama,
zaten çalışan FTS'in üzerine bir iyileştirme, kullanılmayan bir oturum için RAM harcamaya değmez.

## Gerçek doğrulama

Gerçek çalışan servis + gerçek metin modeliyle uçtan uca test edildi: paraphrase edilmiş bir soru
("belgelerimde uçan kırmızı bir şey var mı") — dokümanla ortak uzun bir ifade paylaşmadan — doğru
belgeyi (`balon.md`) buldu, alakasız bir belgeyi (Rust programlama notu) hiç karıştırmadı. Ayrıca
9 birim testi: içerik-hash yeniden kullanımı, model-izolasyonu, geriye dönük doldurma, RRF'nin
gerçekten sıralamayı değiştirdiği, `Runtime` seviyesinde uçtan uca sohbet turu.

## Bilinçli olarak ertelenen iyileştirmeler (kullanıcı onayıyla, F3 sonrası değerlendirilecek)

ChatGPT'den gelen bir gözden geçirmeyle bulunan, gerçek ama bugünün kapsamı dışında bırakılan
maddeler:

1. **Retrieval öncesi permission/sensitivity filtresi** — şu an her indekslenen belge zaten aynı
   tek onay seviyesinden geçiyor, ayrı bir hassasiyet katmanı yok; workspace belgelerine
   `MemoryRecord` gibi bir sensitivity alanı eklenirse yeniden değerlendirilmeli.
2. **Semantic-aware chunking** — Markdown başlık/bölüm, kod fonksiyon/sınıf, PDF paragraf bazlı
   bölme + örtüşme; şu an kör ~1200 karakter bölme kullanılıyor.
3. ~~**Batch embedding**~~ — 16 Ağustos 2026'da uygulandı: `EmbeddingProvider::embed_batch`
   (varsayılan: `embed`'i döngüyle çağırır; `LlamaEmbeddingProvider` gerçek toplu HTTP isteğiyle
   override eder), `SqliteStore::embed_and_store_chunks_batch` — bir belgenin tüm chunk'ları tek
   bir model çağrısında embed ediliyor (içerik-hash tekilleştirmesi batch içinde de geçerli).
4. **Gözlemlenebilirlik metrikleri** (FTS/semantic hit sayısı, gecikme, cache hit oranı) — şu an
   yalnız audit log sinyali var; kişisel araç için orantısız görüldü.
5. **Açık `rag status`/`rag rebuild`/`rag verify` komutları** — cache zaten kavramsal olarak
   yeniden inşa edilebilir (silinip yeniden indekslenebilir) ama tek bir komut yok.
6. **Configurable RRF sabitleri** — şu an sabit (`RRF_K=60`, aday havuzu `limit*4`); gerçek bir
   değerlendirme seti (F3 madde 18) olmadan ayarlamak tahmin olurdu.
7. **Opsiyonel reranker aşaması** — yalnız bir kıyaslama "RRF yetmiyor" derse eklenecek, şimdiden
   ikinci bir model çalıştırmanın gerekçesi yok.
