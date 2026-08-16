//! F4 "Coding evaluation seti". Unlike the other test modules in this crate — which each prove
//! one function's contract in isolation — this module strings the *whole* F4 pipeline together
//! (plan → patch draft → approve → apply → test/verify → keep-or-rollback, through `Runtime`,
//! exactly as `/plan`/`/patch`/`/approve-patch` drive it in the TUI) for each of the scenario
//! categories the F4 checklist names by name: "küçük hata düzeltme, test ekleme, yanlış patch
//! reddi, timeout/cancel, secret exposure ve mevcut-test regression senaryoları" — the last
//! category gets two scenarios (a pre-existing failure correctly tolerated, and a genuine
//! regression correctly caught), since `apply_coding_patch_with_regression_check`'s pre-patch
//! baseline is what tells the two apart.
//!
//! Deterministic and offline throughout — no live model calls (a `ScriptedProvider`, mirroring
//! `patch_generator`'s own test mock, stands in for the model), but every process the pipeline
//! itself spawns (`git apply`, `python3`) is real. This is a *regression gate* for the coding
//! feature specifically, not a duplicate of the unit tests already covering each function; it is
//! meant to keep failing (loudly, by design) if a future change breaks the pipeline's end-to-end
//! behavior even when every individual function's own unit tests still pass in isolation.

#[cfg(test)]
mod tests {
    use crate::{
        approve_patch, create_read_only_coding_plan, draft_patch_with_provider, new_cancel_flag,
        ModelProvider, ModelResponse, Runtime, SqliteStore,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct ScriptedProvider {
        replies: Mutex<Vec<&'static str>>,
    }

    impl ScriptedProvider {
        fn new(replies: Vec<&'static str>) -> Self {
            Self {
                replies: Mutex::new(replies),
            }
        }
    }

    impl ModelProvider for ScriptedProvider {
        fn provider_id(&self) -> &str {
            "eval"
        }
        fn model_id(&self) -> &str {
            "scripted"
        }
        fn complete(&self, _prompt: &str) -> Result<ModelResponse, String> {
            let mut replies = self.replies.lock().expect("lock");
            if replies.is_empty() {
                return Err("scripted eval provider ran out of replies".into());
            }
            let text = replies.remove(0);
            Ok(ModelResponse {
                provider_id: self.provider_id().into(),
                model_id: self.model_id().into(),
                text: text.into(),
                structured_json: None,
                finish_reason: "stop".into(),
            })
        }
    }

    fn eval_fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "jarvis-coding-eval-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("fixture root");
        for (path, content) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, content).unwrap();
        }
        root
    }

    fn eval_runtime() -> Runtime {
        Runtime::with_store(SqliteStore::in_memory().expect("sqlite schema"))
    }

    /// Senaryo 1/7 — küçük hata düzeltme: bir fonksiyon yanlış değer döndürüyor, model doğru
    /// değeri üretiyor, gerçek bir "test" komutu (Python syntax kontrolü, F4'ün allowlist command
    /// runner'ı üzerinden) değişikliği doğruluyor, sonuç kalıcı kalıyor.
    #[test]
    fn scenario_small_bug_fix_is_applied_verified_and_kept() {
        let root = eval_fixture(
            "bugfix",
            &[("calc.py", "def add(a, b):\n    return a - b\n")], // bug: - olmalı +
        );
        let plan = create_read_only_coding_plan(
            &root,
            "add fonksiyonundaki toplama hatasını düzelt",
            vec![PathBuf::from("calc.py")],
            vec!["python3 -m py_compile calc.py".to_string()],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec!["def add(a, b):\n    return a + b\n"]);
        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch drafts");
        let approval = approve_patch(&proposal, true).expect("approved");

        let mut runtime = eval_runtime();
        let (outcome, finalize) = runtime
            .apply_coding_patch_with_regression_check(&plan, &proposal, &approval, None)
            .expect("regression check runs");
        assert_eq!(
            fs::read_to_string(root.join("calc.py")).unwrap(),
            "def add(a, b):\n    return a + b\n"
        );
        assert!(
            outcome.kept,
            "syntax check must pass on both sides: {outcome:?}"
        );
        assert!(finalize.is_ok());
        assert!(runtime
            .audit
            .iter()
            .any(|event| event.event == "coding.tests.passed"));

        fs::remove_dir_all(&root).ok();
    }

    /// Senaryo 2/7 — test ekleme: model dosyaya yeni bir test fonksiyonu ekliyor; doğrulama
    /// yalnız sözdiziminin hâlâ geçerli olduğunu kontrol ediyor (gerçek bir `pytest` kurulumu
    /// garanti edilemediği için — dürüst bir sınır, bkz. modül dokümantasyonu).
    #[test]
    fn scenario_test_addition_keeps_the_file_syntactically_valid() {
        let root = eval_fixture(
            "addtest",
            &[("calc.py", "def add(a, b):\n    return a + b\n")],
        );
        let plan = create_read_only_coding_plan(
            &root,
            "add fonksiyonu için bir test ekle",
            vec![PathBuf::from("calc.py")],
            vec!["python3 -m py_compile calc.py".to_string()],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec![
            "def add(a, b):\n    return a + b\n\n\ndef test_add():\n    assert add(2, 3) == 5\n",
        ]);
        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch drafts");
        assert!(proposal.unified_diff.contains("test_add"));
        let approval = approve_patch(&proposal, true).expect("approved");

        let mut runtime = eval_runtime();
        let (outcome, finalize) = runtime
            .apply_coding_patch_with_regression_check(&plan, &proposal, &approval, None)
            .expect("regression check runs");
        assert!(outcome.kept);
        assert!(finalize.is_ok());

        fs::remove_dir_all(&root).ok();
    }

    /// Senaryo 3/7 — yanlış patch reddi: plan dışı bir dosyayı değiştirmeye çalışan bir diff,
    /// hem `create_patch_proposal` seviyesinde hem de kurcalanmış bir hash `validate_patch_
    /// proposal` seviyesinde reddedilmeli — hiçbir aşamada diske hiçbir şey yazılmamalı.
    #[test]
    fn scenario_a_wrong_patch_is_rejected_before_it_can_touch_disk() {
        let root = eval_fixture(
            "wrongpatch",
            &[("in_scope.py", "x = 1\n"), ("out_of_scope.py", "y = 2\n")],
        );
        let plan = create_read_only_coding_plan(
            &root,
            "in_scope.py içeriğini değiştir",
            vec![PathBuf::from("in_scope.py")],
            vec![],
        )
        .expect("valid plan");

        // (a) Plan dışı bir dosyayı hedefleyen bir diff reddedilmeli.
        let out_of_scope_diff = "diff --git a/out_of_scope.py b/out_of_scope.py\n--- a/out_of_scope.py\n+++ b/out_of_scope.py\n@@ -1 +1 @@\n-y = 2\n+y = 999\n";
        assert!(crate::create_patch_proposal(
            &plan,
            out_of_scope_diff,
            vec![PathBuf::from("out_of_scope.py")]
        )
        .is_err());

        // (b) Geçerli bir proposal üretilip sonra kurcalanırsa (hash artık eşleşmiyor) apply
        // asla tetiklenmemeli — `validate_patch_proposal` bunu `apply_approved_patch`'in daha ilk
        // adımında yakalıyor.
        let valid_diff = "diff --git a/in_scope.py b/in_scope.py\n--- a/in_scope.py\n+++ b/in_scope.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n";
        let mut proposal =
            crate::create_patch_proposal(&plan, valid_diff, vec![PathBuf::from("in_scope.py")])
                .expect("valid proposal");
        let approval = approve_patch(&proposal, true).expect("approved before tampering");
        proposal
            .unified_diff
            .push_str("# tampered after approval\n");

        let mut runtime = eval_runtime();
        let result = runtime.apply_coding_patch(&plan, &proposal, &approval);
        assert!(result.is_err(), "a tampered proposal must never apply");
        assert_eq!(
            fs::read_to_string(root.join("in_scope.py")).unwrap(),
            "x = 1\n"
        );

        fs::remove_dir_all(&root).ok();
    }

    /// Senaryo 4/7 — timeout/cancel: kullanıcı bir test çalışırken `/abort` eşdeğeri bir
    /// `CancelFlag` ile iptal ederse, patch **otomatik geri alınır** — hiçbir yarı-uygulanmış
    /// durum kalmaz. Gerçek bir uzun süren komut (`python3 -m timeit`) ve gerçek bir arka plan
    /// thread'den gelen iptal kullanılıyor (mock değil).
    #[test]
    fn scenario_cancelling_a_running_test_rolls_back_the_patch() {
        let root = eval_fixture(
            "cancel",
            &[("calc.py", "def add(a, b):\n    return a - b\n")],
        );
        let plan = create_read_only_coding_plan(
            &root,
            "add fonksiyonunu düzelt",
            vec![PathBuf::from("calc.py")],
            vec!["python3 -m timeit -n 999999999 pass".to_string()],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec!["def add(a, b):\n    return a + b\n"]);
        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch drafts");
        let approval = approve_patch(&proposal, true).expect("approved");

        let mut runtime = eval_runtime();
        let cancel = new_cancel_flag();
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            cancel_for_thread.store(true, Ordering::SeqCst);
        });
        let (outcome, finalize) = runtime
            .apply_coding_patch_with_regression_check(&plan, &proposal, &approval, Some(&cancel))
            .expect("regression check runs even when cancelled mid-way");
        assert!(!outcome.kept, "a cancelled run must never be kept");
        assert!(finalize.is_ok());
        assert_eq!(
            fs::read_to_string(root.join("calc.py")).unwrap(),
            "def add(a, b):\n    return a - b\n",
            "a cancelled test run must roll the file back to its pre-patch content"
        );
        assert!(runtime
            .audit
            .iter()
            .any(|event| event.event == "coding.tests.cancelled"));

        fs::remove_dir_all(&root).ok();
    }

    /// Senaryo 5/7 — secret exposure, iki savunma katmanı ayrı ayrı: (a) modelin bir dosyayı
    /// yeniden yazarken enjekte ettiği bir secret patch taslağı seviyesinde reddediliyor
    /// (`patch_generator`), (b) zaten gizli-bilgi benzeri isimli bir dosya `analyze_repository`
    /// taramasına hiç girmiyor, dolayısıyla bir `CodingPlan`'ın hedefi bile olamıyor.
    #[test]
    fn scenario_secret_exposure_is_blocked_at_two_independent_layers() {
        // (a) Patch draft katmanı.
        let root = eval_fixture("secret-patch", &[("config.py", "TOKEN = None\n")]);
        let plan = create_read_only_coding_plan(
            &root,
            "config.py'a örnek bir token ekle",
            vec![PathBuf::from("config.py")],
            vec![],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec![
            "TOKEN = \"ghp_abcdefghijklmnopqrstuvwxyz0123456789\"\n",
        ]);
        let result = draft_patch_with_provider(&plan, &provider);
        assert!(
            result.is_err(),
            "a secret-like rewrite must never become an appliable patch"
        );
        assert!(!root.join("config.py").metadata().unwrap().is_dir()); // fixture sanity
        assert_eq!(
            fs::read_to_string(root.join("config.py")).unwrap(),
            "TOKEN = None\n"
        );
        fs::remove_dir_all(&root).ok();

        // (b) Tarama katmanı: gizli-bilgi benzeri isimli bir dosya hiç dahil edilmiyor.
        let root = eval_fixture("secret-scan", &[(".env", "API_KEY=super-secret\n")]);
        let overview = crate::analyze_repository(&root).expect("scan succeeds");
        assert!(
            !overview.included_files.contains(&PathBuf::from(".env")),
            "a secret-like named file must never be scannable, let alone plannable"
        );
        assert!(overview
            .risk_notes
            .iter()
            .any(|note| note.contains("gizli-bilgi benzeri")));
        fs::remove_dir_all(&root).ok();
    }

    /// Senaryo 6/7 — mevcut-test regresyonu, doğru şekilde tolere ediliyor: bir test komutu
    /// patch'ten TAMAMEN bağımsız olarak zaten bozuksa (ör. var olmayan bir modülü arıyor), doğru
    /// bir patch artık bu yüzden geri alınmıyor. **Bu, önceki turda dürüstçe belgelenen bilinen
    /// bir sınırın gerçek düzeltmesini kanıtlıyor**: `apply_coding_patch_with_regression_check`
    /// artık patch'ten önce aynı test planını bir "taban çizgisi" olarak çalıştırıyor; taban
    /// çizgisinde de başarısız olan bir komut artık patch'e karşı kullanılmıyor.
    #[test]
    fn scenario_a_pre_existing_broken_test_no_longer_blocks_an_otherwise_correct_patch() {
        let root = eval_fixture(
            "pre-existing",
            &[("calc.py", "def add(a, b):\n    return a - b\n")],
        );
        let plan = create_read_only_coding_plan(
            &root,
            "add fonksiyonunu düzelt",
            vec![PathBuf::from("calc.py")],
            // Bilerek var olmayan bir modülü arıyor — patch'ten TAMAMEN bağımsız, hem patch
            // öncesi hem sonrası aynı şekilde başarısız olacak bir "test".
            vec!["python3 -m jarvis_eval_module_that_never_exists".to_string()],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec!["def add(a, b):\n    return a + b\n"]);
        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch drafts");
        let approval = approve_patch(&proposal, true).expect("approved");

        let mut runtime = eval_runtime();
        let (outcome, finalize) = runtime
            .apply_coding_patch_with_regression_check(&plan, &proposal, &approval, None)
            .expect("regression check runs");
        assert!(
            outcome.kept,
            "a failure present both before and after the patch must not be blamed on it"
        );
        assert!(outcome.regressions.is_empty());
        assert!(finalize.is_ok());
        assert_eq!(
            fs::read_to_string(root.join("calc.py")).unwrap(),
            "def add(a, b):\n    return a + b\n",
            "the correct fix must be kept even though the configured test was already broken"
        );
        assert!(runtime
            .audit
            .iter()
            .any(|event| event.event == "coding.tests.pre_existing_failure_tolerated"));

        fs::remove_dir_all(&root).ok();
    }

    /// Senaryo 7/7 — gerçek regresyon: patch, patch-öncesi çalışan bir test komutunu bozarsa
    /// (yeni sözdizimi hatası), bu artık genuine bir regresyon olarak ayırt ediliyor ve geri
    /// alınıyor — bir önceki senaryonun tam tersi, aynı taban çizgisi mekanizmasının doğru
    /// tarafı da yakaladığının kanıtı.
    #[test]
    fn scenario_a_genuine_regression_introduced_by_the_patch_is_rolled_back() {
        let root = eval_fixture("genuine-regression", &[("calc.py", "x = 1\n")]);
        let plan = create_read_only_coding_plan(
            &root,
            "calc.py'ı değiştir",
            vec![PathBuf::from("calc.py")],
            vec!["python3 -m py_compile calc.py".to_string()],
        )
        .expect("valid plan");
        // Model geçerli Python'u kasıtlı olarak sözdizimi hatalı bir şeye "düzeltiyor" — gerçek
        // dünyada bir modelin üretebileceği türden bir regresyon.
        let provider = ScriptedProvider::new(vec!["x = (\n"]);
        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch drafts");
        let approval = approve_patch(&proposal, true).expect("approved");

        let mut runtime = eval_runtime();
        let (outcome, finalize) = runtime
            .apply_coding_patch_with_regression_check(&plan, &proposal, &approval, None)
            .expect("regression check runs");
        assert!(!outcome.kept, "a genuine regression must not be kept");
        assert_eq!(outcome.regressions.len(), 1);
        assert!(finalize.is_ok());
        assert_eq!(
            fs::read_to_string(root.join("calc.py")).unwrap(),
            "x = 1\n",
            "a genuine regression must restore the pre-patch content"
        );
        assert!(runtime
            .audit
            .iter()
            .any(|event| event.event == "coding.tests.regression_detected"));

        fs::remove_dir_all(&root).ok();
    }
}
