# ADR-0006 — LoRA/QLoRA adaptasyon fizibilitesi: şimdilik ERTELENDİ

Tarih: 19 Ağustos 2026
Durum: Kabul edildi (karar: şimdi eğitim yapılmayacak)
Kapsam: F6 madde 4 — "LoRA/QLoRA fizibilite kararı: VRAM/RAM, eğitim süresi, lisans, eval
hedefi ve rollback artifact'i kullanıcıya sunulmadan eğitim başlamaz."

## Bağlam

F6'nın planı, model adaptasyonunu bir hedef değil, *ölçüme bağlı bir seçenek* olarak koyar:
"Her model veya adapter değişikliği, sürümlü eval'de hedef metriği iyileştirir ve güvenlik/
latency regresyonu üretmez; aksi halde kullanılmaz." Bu ADR, o kararın şu anki cevabıdır.

Kullanıcının somut şikayeti "amatör kod yazıyor" idi ve bu, fine-tuning'i akla getirdi.

## Ölçülen gerçekler (varsayım değil)

- **Golden set baseline (19 Ağustos 2026, 10/10 PASS)**: K01-K05 coding senaryolarında model
  *amatör değil* — doğru, idiomatic çıktı üretti, K03'te iki ayrı hatayı doğru teşhis etti.
  Yani şikayeti tetikleyen davranış bu senaryolarda **yeniden üretilemedi**.
- **Donanım**: 8 GB VRAM (4.47 GB'ı mevcut modelin), 62 GB sistem RAM.
- **Dataset**: `teacher_examples` tablosu şu an **boş**. F6 madde 6'nın geri bildirim intake'i
  bu turda yeni kuruldu; henüz hiç insan-onaylı örnek birikmedi.

## Karar

**Şimdi LoRA/QLoRA eğitimi yapılmayacak.** Üç bağımsız gerekçe, herhangi biri tek başına yeterli:

1. **Eğitilecek veri yok.** Sıfır onaylı örnekle adaptasyon anlamsızdır. F6'nın kendi tasarımı
   veriyi organik biriktirmeyi öngörür; bu, atlanabilecek bir adım değil.
2. **Çözülecek ölçülmüş bir problem yok.** Golden set, iddia edilen kalite sorununu ölçemedi.
   Ölçülmemiş bir problemi eğitimle "çözmek", iyileşmeyi doğrulayamamak demektir — F6'nın
   tamamlanma ölçütü tam da bunu yasaklar.
3. **Daha ucuz, denenmemiş bir seçenek var.** Kod kalitesi gerçekten sorun çıkarırsa, ilk adım
   eğitim değil **kod-özel bir model** kullanmaktır (ör. Qwen2.5-Coder ailesi, F4'ün coding
   pipeline'ında). Dataset, eğitim süresi veya rollback artefaktı gerektirmez; yalnız hangi
   görevde hangi modelin çağrıldığını değiştirir.

## Yeniden değerlendirme tetikleyicileri

Bu karar kalıcı değil. Aşağıdakilerden biri gerçekleşirse ADR yeniden açılır:

- Golden set **zor senaryolarla** genişletilir ve kod kalitesi ölçülebilir şekilde `FAIL` verir.
- Kod-özel model denenir ve yetersiz kalır (yani sorun model seçimiyle çözülmüyor).
- `teacher_examples` içinde anlamlı sayıda (kabaca birkaç yüz) insan-onaylı, hassas olmayan
  örnek birikir.

## Eğitim yapılacaksa önkoşullar (bu ADR'ın asıl kalıcı değeri)

Eğitim başlamadan önce şunlar kullanıcıya sunulmalıdır — planın açık şartı:

- **VRAM/RAM bütçesi**: 8 GB VRAM'de 8B bir modelin QLoRA (4-bit) eğitimi sınırdadır; gradient
  checkpointing ve küçük batch/sequence gerekir. Ölçülmeden başlanmamalı.
- **Lisans**: temel modelin lisansı türev ağırlıklara izin veriyor mu.
- **Eval hedefi**: hangi golden-set satırının, hangi ölçüde iyileşmesi başarı sayılacak
  (önceden yazılmalı, sonradan değil).
- **Rollback artefaktı**: adapter'ı devre dışı bırakıp önceki konfigürasyona dönmenin tek
  komutluk yolu. Registry (`ModelConfigRun.rollback_target`) ve
  `Runtime::model_config_regression()` bu kararın ölçüm tarafını zaten sağlıyor.

## Sonuçlar

- F6 madde 4 kapandı: karar verildi ve gerekçelendirildi, eğitim yapılmadı.
- Hiçbir model indirilmedi, hiçbir eğitim çalıştırılmadı, mevcut davranış değişmedi.
