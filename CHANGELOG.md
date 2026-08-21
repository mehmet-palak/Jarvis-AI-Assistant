# Değişiklik Kaydı (Changelog)

Bu proje [Semantic Versioning](https://semver.org/lang/tr/) kullanır. Sürüm numarası
`Cargo.toml`'dan gelir ve `jarvis_core::JARVIS_VERSION` ile programdan okunabilir (`/status`).

## [0.1.0] — geliştirme (yayınlanmadı)

İlk geliştirme sürümü. Fazlar F0-F7 tamamlandı, F9 operasyonel olgunluk sürüyor.

### Eklendi
- **F0-F3:** güvenli çekirdek, local-first masaüstü, multimodal ekler, kullanıcı profili +
  kontrollü bellek + gerçek RAG (hybrid FTS + embedding).
- **F4:** güvenli coding workbench — plan→patch→onay→uygula→test zinciri, gerçek izole worker
  (bwrap + cgroup v2 + seccomp-bpf), süreç-grubu temizliği.
- **F5:** sesli etkileşim (bas-tut, TTS/STT), erişilebilirlik.
- **F6:** model kalite ölçümü (golden set, `/eval`), dataset governance.
- **F7:** yetkili pentest/bug bounty — imzalı scope, ağ kapısı, keşif (CT/DNS/port/JS),
  manuel test (replay/diff), SAFE kontroller, bulgu yönetimi + rapor taslağı; doğal-dil arayüzü
  (scope + keşif, onaylı).
- **F9 (sürüyor):** şema göç bütünlük kontrolü, kendi-kendini-doğrulayan yedek + saklama,
  sürüm uyumluluk güvencesi, gizlilik-güvenli metrikler, bağımlılık güvenlik denetimi
  (cargo audit), timeout/cancellation worker'a süreç-grubu öldürme, audit export (witness).

### Güvenlik
- Bağımlılık güvenlik denetimi kuruldu (RustSec). İlk denetimdeki uyarılar
  [docs/security_dependency_audit.md](docs/security_dependency_audit.md)'de gerekçeleriyle
  belgelendi.
