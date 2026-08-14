# F2 local vision smoke — 14 Ağustos 2026

Bu kayıt bir kullanıcı görseli veya sohbet içeriği tutmaz. Amaç, indirilen vision modelinin ayrı
loopback sunucusunda gerçekten çalıştığını ve uygulama sınırını doğrulamaktır.

## Çalışma profili

- Model: `ggml-org/Qwen2.5-VL-3B-Instruct-GGUF`, `Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf`
- Projector: `mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf`
- Lisans: Apache-2.0 (model deposu)
- Endpoint: yalnız `127.0.0.1:8089`
- Runtime: CPU-only `-ngl 0`, 6 CPU thread, VRAM `0`
- Context: 2048; dinamik görsel çözünürlüğü için `--image-min-tokens 1024`
- Browser sınırı: CORS yalnız `localhost`, credential kapalı
- Servis bellek gözlemi: yaklaşık `2.8–3.0 GB` RAM

## Kanıtlanan smoke

| Kontrol | Sonuç | Kanıt |
| --- | --- | --- |
| Dosyalar | PASS | Model `1,929,901,056` bayt; projector `844,757,728` bayt indirildi. |
| Servis health | PASS | `GET /health` → `{"status":"ok"}`. |
| Gerçek PNG/JPEG analizi | PASS | Yalnız sistemdeki örnek Arch Linux logosu (PNG) ve genel Qt arka planı (JPEG) ile `/v1/chat/completions`; ikisi de model tarafından işlendi. |
| İlk görsel gecikmesi | PASS / ölçüm | Yaklaşık 33.8 sn: 1069 prompt token + 60 üretim token. |
| Aynı görsel sıcak tekrar | PASS / ölçüm | Vision KV cache ile yaklaşık 2–4 sn; 32 token için servis üretimi ~1.8 sn. |
| Uygulama contractı | PASS | Rust testleri, ek baytlarının yalnız vision isteğine girdiğini; yerel path'in metin modele veya vision prompt gövdesine girmediğini; vision açıklamasının escaped untrusted data olduğunu doğrular. |

## Sınırlar

- İlk farklı veya büyük görsel CPU üzerinde yavaştır; bu nedenle vision açıklaması 96 token ile
  sınırlıdır. Son kullanıcı yanıtını normal text modeli üretir.
- JARVIS görsel seçildiğinde ayrı vision servisini talep anında başlatır. Metin-only turlarda bu
  servis gerekli değildir.
- Ek, vision sunucusuna gitmeden önce decode edilip en çok 2 MP'lik JPEG olarak yeniden kodlanır.
  Böylece EXIF ve PNG text chunk gibi dosya metadata'sı taşınmaz; transport 8 MiB ile sınırlıdır.
- `exit` veya native **Modeli RAM'den çıkar** hem text hem vision servislerini kapatır. Pencereyi
  kapatmak ya da `/quit` yalnız arayüzü kapatır.
- PNG/JPEG dışındaki görseller, bozuk/stale dosyalar ve erişilemeyen vision servisi güvenli hata
  verir; hiçbirinde JARVIS görseli gördüğünü iddia etmez.
