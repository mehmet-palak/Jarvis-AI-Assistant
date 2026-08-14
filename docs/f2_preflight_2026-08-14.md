# F2 indirimsiz ön koşum — 14 Ağustos 2026

Bu kayıt F2 exit review değildir. Amaç, mevcut CPU-only yerel modelin ve yeni sohbet
contract'ının erken davranışını görünür kılmaktır; normal sohbeti şablon/if-else yanıtlarla
değiştirmek değildir.

## Koşum profili

- Model: `Qwen3-8B-Q4_K_M.gguf`
- Sunucu: loopback-only `llama-server`, `-ngl 0`, VRAM `0`
- Ağ/indirme: yok
- Uygulama: F2 çalışma ağacı; commit öncesi doğrulama

## Kanıtlanan davranışlar

| Alan | Sonuç | Not |
| --- | --- | --- |
| Türkçe kısa sohbet | Kısmi PASS | Yanıt Türkçe ve tam cümleydi; gereksiz hava durumu iddiası içerdi. |
| English short chat | PASS | Yanıt İngilizce, kısa ve doğal kaldı. |
| Sohbet içindeki `saat` sözcüğü | PASS | Şiir bağlamında model `UNKNOWN` seçti; tool çağrısı oluşmadı. |
| Tek çağrılı intent üretimi | PASS | Güncel yerel zaman isteğinde model yalnız dar `system.time` intent envelope'u üretti. |
| Envelope güvenlik sınırı | PASS | 71 core, 15 TUI, 4 native test; yalnız tam allowlist kimliği kabul ediliyor, etiket kullanıcıya render edilmiyor. |

## Açık kalite bulguları

- **F2-QA-001 — Türkçe doğruluk/doğallık:** Qwen3-8B-Q4 bazen bağlamsız somut ayrıntı uydurabiliyor veya zayıf Türkçe üretebiliyor. Bu bir model/prompt kalite bulgusudur; kullanıcı mesajına karşı sabit cevap yazılarak kapatılmayacak.
- **F2-QA-002 — Uzun bağlam:** Önceki etkileşimlerde modelin takip sorusunda uydurma öneriler vermesi ve uzun cevaplarda kalite düşmesi görüldü. Sürümlü C02–C06 koşumu ile ölçülüp prompt/model benchmark'ına taşınacak.

## Sonuç

Dil contract'ı ve güvenli tek-çağrılı model intent hattı çalışıyor; F2.0 sohbet kalite kapısı
henüz kapalı değildir. `docs/f2_conversation_qa.md` içindeki 20 senaryonun tamamı gerçek local
koşum ve insan değerlendirmesiyle PASS olmadan exit review işaretlenmez.
