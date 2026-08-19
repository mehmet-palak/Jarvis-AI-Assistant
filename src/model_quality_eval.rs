//! F6 "Model kalitesi golden set" koşum aracı — [`docs/f6_model_quality_golden_set.md`] belgesinin
//! çalıştırılabilir karşılığı.
//!
//! `coding_eval` (F4) ile aynı "eval seti kendi dosyasında" desenini izler, ama iki temel farkı
//! vardır ve bu farklar bilinçlidir:
//!
//! 1. **Canlı model gerektirir.** `coding_eval` deterministik/offline'dır (`ScriptedProvider`);
//!    burada ölçülen şey pipeline'ın kendisi değil, *gerçek modelin çıktı kalitesidir* — bu yüzden
//!    gerçek `LlamaServerProvider` kullanılır. Bu nedenle her test `#[ignore]`'dur: `cargo test` ve
//!    `scripts/release_check.sh` offline çalışmaya devam eder, bu set yalnız açıkça istendiğinde
//!    (`cargo test --lib model_quality -- --ignored --nocapture`) koşar.
//! 2. **Her şeyi otomatik yargılamaz.** Kod kalitesi ("amatör mü, idiomatic mi") mekanik olarak
//!    ölçülemez; bu set yalnız *mekanik olarak doğrulanabilir olanı* assert eder (yanlış capability'ye
//!    yönlendirme, boş/kod içermeyen yanıt, uydurma) ve geri kalanını insan değerlendirmesi için
//!    gecikmeyle birlikte yazdırır. Golden set belgesindeki `PASS`/`FAIL` sütunu bu koşumun
//!    çıktısıyla, insan tarafından doldurulur — bu modül o kararı kendisi vermez.

#[cfg(test)]
mod tests {
    use crate::{
        InputType, LlamaServerProvider, ModelProvider, ModelRuntimeState, Request, Runtime,
        SqliteStore,
    };
    use std::time::Instant;

    /// Bir golden-set senaryosunun tek koşumu. `capability`, routing'in nereye gittiğini gösterir
    /// (K05 gibi routing senaryolarının mekanik assert'i buna dayanır); `latency_ms` F6'nın
    /// istediği "latency/quality raporu"nun latency yarısıdır.
    struct ScenarioRun {
        id: &'static str,
        capability: String,
        latency_ms: u128,
        output: String,
    }

    impl ScenarioRun {
        /// İnsan değerlendirmesi için okunabilir rapor. `--nocapture` ile koşulduğunda golden set
        /// belgesine doldurulacak veriyi doğrudan verir.
        fn report(&self) {
            println!("\n=== {} ===", self.id);
            println!("capability : {}", self.capability);
            println!("latency    : {} ms", self.latency_ms);
            println!("çıktı      :\n{}\n", self.output);
        }
    }

    fn eval_runtime() -> Runtime {
        Runtime::with_store(SqliteStore::in_memory().expect("sqlite schema"))
    }

    fn live_provider() -> LlamaServerProvider {
        let provider = LlamaServerProvider::local_default();
        assert_eq!(
            provider.runtime_state(),
            ModelRuntimeState::Ready,
            "F6 golden set canlı model sunucusu gerektirir; `systemctl --user start jarvis-llama.service` \
             ile başlatıp tekrar deneyin (bu testler bilinçli olarak #[ignore]'dur)."
        );
        provider
    }

    /// Bir senaryoyu gerçek pipeline üzerinden koşar. Ham `/completion` çağrısı değil
    /// `handle_with_provider` kullanılır — çünkü ölçmek istediğimiz şey kullanıcının gerçekte
    /// gördüğü davranış (routing dahil), modelin izole çıktısı değil.
    fn run_scenario(
        id: &'static str,
        history: &[(&'static str, &str)],
        prompt: &str,
        provider: &dyn ModelProvider,
    ) -> ScenarioRun {
        let mut runtime = eval_runtime();
        for (role, content) in history {
            runtime.chat_history.push(crate::ConversationMessage {
                role,
                content: (*content).to_string(),
            });
        }
        let request = Request {
            schema_version: 1,
            request_id: format!("f6-{id}"),
            input_type: InputType::Cli,
            content: prompt.to_string(),
            attachments: Vec::new(),
        };

        let started = Instant::now();
        let (task, result, _verify) = runtime.handle_with_provider(request, provider);
        let latency_ms = started.elapsed().as_millis();

        ScenarioRun {
            id,
            capability: task.capability,
            latency_ms,
            output: result.output,
        }
    }

    /// Bir yanıtın gerçekten kod içerip içermediğinin mekanik göstergesi. Kalite yargısı değildir —
    /// yalnız "hiç kod üretmedi / yerine dosya listesi verdi" başarısızlık modunu yakalar.
    fn looks_like_code(output: &str, markers: &[&str]) -> bool {
        output.contains("```") || markers.iter().any(|marker| output.contains(marker))
    }

    /// K01 — basit fonksiyon (Rust). Mekanik: yanıt kod içermeli ve routing sohbette kalmalı.
    /// Kalite (idiomatic mi, isimlendirme) insan değerlendirmesine bırakılır.
    #[test]
    #[ignore = "canlı model sunucusu gerektirir"]
    fn k01_simple_rust_function() {
        let provider = live_provider();
        let run = run_scenario(
            "K01",
            &[],
            "Rust'ta bir string slice'ın sesli harf sayısını döndüren kısa bir fonksiyon yaz.",
            &provider,
        );
        run.report();
        assert_eq!(
            run.capability, "conversation.reply",
            "kod yazma isteği sohbette kalmalı, bir capability'ye yönlendirilmemeli"
        );
        assert!(
            looks_like_code(&run.output, &["fn "]),
            "K01 yanıtı hiç kod içermiyor"
        );
    }

    /// K02 — aynı görev, Python. Dil değişiminin routing'i veya kod üretimini bozmadığını gösterir.
    #[test]
    #[ignore = "canlı model sunucusu gerektirir"]
    fn k02_simple_python_function() {
        let provider = live_provider();
        let run = run_scenario(
            "K02",
            &[],
            "Python'da bir metindeki sesli harf sayısını döndüren kısa bir fonksiyon yaz.",
            &provider,
        );
        run.report();
        assert_eq!(run.capability, "conversation.reply");
        assert!(
            looks_like_code(&run.output, &["def "]),
            "K02 yanıtı hiç kod içermiyor"
        );
    }

    /// K03 — hata ayıklama. Mekanik olarak yalnız "boş/kaçamak yanıt değil" doğrulanır; doğru
    /// teşhis edip etmediği insan değerlendirmesidir.
    #[test]
    #[ignore = "canlı model sunucusu gerektirir"]
    fn k03_debugging_a_broken_snippet() {
        let provider = live_provider();
        let run = run_scenario(
            "K03",
            &[],
            "Bu Python fonksiyonu neden her zaman 0 döndürüyor?\n\ndef topla(sayilar):\n    toplam = 0\n    for s in sayilar:\n        toplam = s\n    return toplam - toplam",
            &provider,
        );
        run.report();
        assert_eq!(run.capability, "conversation.reply");
        assert!(
            run.output.trim().len() > 40,
            "K03 yanıtı anlamlı bir açıklama içermiyor"
        );
    }

    /// K04 — kod açıklama. Var olan kodu açıklama isteğinin de sohbette kalması gerekir; bu istek
    /// `code.project_outline`'a benzer göründüğü için routing açısından gerçek bir sınır testidir.
    #[test]
    #[ignore = "canlı model sunucusu gerektirir"]
    fn k04_explaining_existing_code() {
        let provider = live_provider();
        let run = run_scenario(
            "K04",
            &[],
            "Şu Rust satırı ne yapıyor açıkla: `let total: usize = items.iter().filter(|i| i.active).count();`",
            &provider,
        );
        run.report();
        assert_eq!(
            run.capability, "conversation.reply",
            "kod açıklama isteği workspace-okuma capability'lerine yönlendirilmemeli"
        );
        assert!(run.output.trim().len() > 40);
    }

    /// K05 — router sınırı, kalıcı regresyon koruması. Bu senaryo 16 Ağustos 2026'da gerçek
    /// kullanımda bulunan hatanın (kod yazma isteği `note.create`/`code.project_outline`'a
    /// yönlendiriliyordu) tekrar etmediğini garanti eder. Hatanın yalnız *konuşma geçmişiyle*
    /// tekrarlandığı canlı olarak kanıtlanmıştı — bu yüzden geçmiş burada bilinçli olarak
    /// tohumlanıyor; geçmişsiz bir koşum bu regresyonu yakalayamaz.
    #[test]
    #[ignore = "canlı model sunucusu gerektirir"]
    fn k05_code_request_with_history_does_not_misroute() {
        let provider = live_provider();
        let run = run_scenario(
            "K05",
            &[
                ("user", "jarvis bana bir c++ kodu yaz"),
                (
                    "assistant",
                    "Elbette, ne tür bir C++ kodu istediğinizi belirtir misiniz?",
                ),
            ],
            "direkt buraya yaz orta düzey bir script olsun",
            &provider,
        );
        run.report();
        assert_eq!(
            run.capability, "conversation.reply",
            "REGRESYON: kod yazma isteği tekrar yanlış capability'ye yönlendirildi"
        );
        assert!(
            looks_like_code(&run.output, &["#include", "int main"]),
            "K05 yanıtı gerçek kod yerine başka bir şey döndürdü"
        );
    }
}
