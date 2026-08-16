//! F4 "Read-only proje analisti": JARVIS'in kod tabanını **hiçbir dosyaya dokunmadan** okuyup
//! anlaması. `workbench.rs`'in patch/apply mekanizmasından tamamen ayrı — burada tek bir yazma
//! işlemi bile yok, yalnız zaten var olan `preview_workspace_index` taramasının (aynı `.git`/
//! `target`/`node_modules`/`.venv` hariç tutması, aynı gizli-bilgi/boyut filtresi — F3'ün
//! "Workspace izin UX'i" maddesiyle aynı güvenlik sınırı) üzerine dil/bağımlılık/test komutu
//! tespiti ekliyor.
//!
//! Kapsam sınırı (bilinçli): bu modül yalnız **genel repo yapısını** anlıyor — "bu isteğe göre
//! hangi dosyalar etkilenir" gibi bir isteğe özgü, anlamsal bir karar vermiyor (bu, F4'ün henüz
//! yapılmamış "Coding plan UX" maddesinin işi, model akıl yürütmesi gerektirir). Yalnız kök
//! dizindeki bilinen manifest dosyalarına bakıyor — alt dizinlerdeki monorepo paketlerini
//! ayrıca taramıyor (v1 sınırı, gerekirse genişletilir).

use std::path::{Path, PathBuf};

use crate::workspace::preview_workspace_index;
use crate::{create_read_only_coding_plan, CodingPlan, ModelProvider};

/// Kök dizindeki bilinen bir manifest dosyasının, tespit edilen dili ve önerilen test komutunu
/// eşlediği sabit tablo. Sırayla kontrol edilir; `pyproject.toml` varsa `requirements.txt` ayrıca
/// eklenmez (aynı dil için tekrar bir giriş olmasın diye).
const MANIFEST_SIGNATURES: &[(&str, &str, &str)] = &[
    ("Cargo.toml", "Rust", "cargo test"),
    ("package.json", "JavaScript/TypeScript (Node)", "npm test"),
    ("pyproject.toml", "Python", "pytest"),
    ("requirements.txt", "Python", "pytest"),
    ("go.mod", "Go", "go test ./..."),
    ("pom.xml", "Java (Maven)", "mvn test"),
    ("build.gradle", "Java/Kotlin (Gradle)", "./gradlew test"),
    ("build.gradle.kts", "Java/Kotlin (Gradle)", "./gradlew test"),
];

/// Bir repo'nun çok büyük sayılıp sayılmayacağının eşiği — yalnız kullanıcıya bilgilendirici bir
/// risk notu düşürmek için, hiçbir davranışı değiştirmiyor (tarama yine tamamlanır).
const LARGE_REPOSITORY_FILE_COUNT: usize = 2_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoOverview {
    pub root: PathBuf,
    /// Kök dizinde tespit edilen diller — birden fazla olabilir (ör. Rust + bir Node aracı).
    pub detected_languages: Vec<String>,
    /// Tespitin dayandığı manifest dosyalarının kök-göreli yolları.
    pub dependency_manifests: Vec<PathBuf>,
    /// Tespit edilen her dil için önerilen test komutu — kesin doğru olduğu garanti edilmez,
    /// yalnız bir başlangıç noktası; gerçek komut kullanıcı/mevcut CI yapılandırmasıyla teyit
    /// edilmeli.
    pub suggested_test_commands: Vec<String>,
    /// `preview_workspace_index`'in bulduğu, taramaya dahil edilebilir dosya sayısı.
    pub file_count: usize,
    /// Aynı dosyaların toplam boyutu (tahmini, F3'teki gibi).
    pub total_bytes: u64,
    /// Kullanıcıya gösterilecek, hiçbir işlemi engellemeyen bilgilendirici notlar (büyük repo,
    /// hiç bilinen manifest bulunamadı, gizli-bilgi/boyut yüzünden atlanan dosya sayısı).
    pub risk_notes: Vec<String>,
    /// Kök-göreli, taramaya dahil edilebilir gerçek dosya yolları (`preview_workspace_index`'in
    /// `included` listesi, aynen). "Coding plan UX" (`draft_coding_plan_with_provider`) bunu
    /// modelin önerdiği dosyaların **gerçekten var olup olmadığını** doğrulamak için kullanıyor —
    /// model asla bir dosya yolu icat edip kabul ettiremez.
    pub included_files: Vec<PathBuf>,
}

/// `root`'u salt-okunur olarak analiz eder — hiçbir dosyanın içeriğini açmaz, hiçbir şey yazmaz.
/// Aynı containment/exclude/gizli-bilgi/boyut kurallarını kullanan `preview_workspace_index`
/// üzerine kurulu; bu fonksiyon kendi başına yeni bir tarama mantığı icat etmiyor.
pub fn analyze_repository(root: &Path) -> Result<RepoOverview, String> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| format!("workspace root unavailable: {error}"))?;
    if !canonical_root.is_dir() {
        return Err("workspace root must be a directory".into());
    }
    let preview = preview_workspace_index(&canonical_root, &[])?;

    let mut detected_languages = Vec::new();
    let mut dependency_manifests = Vec::new();
    let mut suggested_test_commands = Vec::new();
    for (file_name, language, test_command) in MANIFEST_SIGNATURES {
        let manifest_path = canonical_root.join(file_name);
        if !manifest_path.is_file() {
            continue;
        }
        // pyproject.toml zaten Python'u tespit ettiyse requirements.txt'i tekrar aynı dil için
        // eklemesin diye — tabloda bilerek pyproject.toml, requirements.txt'ten önce geliyor.
        if *language == "Python" && detected_languages.iter().any(|found| found == "Python") {
            continue;
        }
        detected_languages.push((*language).to_string());
        dependency_manifests.push(PathBuf::from(file_name));
        suggested_test_commands.push((*test_command).to_string());
    }

    let mut risk_notes = Vec::new();
    if detected_languages.is_empty() {
        risk_notes.push(
            "Kök dizinde bilinen bir bağımlılık manifesti (Cargo.toml/package.json/... ) bulunamadı; dil/test komutu otomatik tespit edilemedi.".to_string(),
        );
    }
    if preview.included.len() > LARGE_REPOSITORY_FILE_COUNT {
        risk_notes.push(format!(
            "Büyük repo: {} dosya taranabilir durumda — tam okuma pahalı olabilir, kapsamı daraltmayı düşün.",
            preview.included.len()
        ));
    }
    if !preview.excluded_secret_like.is_empty() {
        risk_notes.push(format!(
            "{} dosya gizli-bilgi benzeri isim/desen yüzünden analiz dışı bırakıldı.",
            preview.excluded_secret_like.len()
        ));
    }
    if !preview.excluded_oversized.is_empty() {
        risk_notes.push(format!(
            "{} dosya boyut sınırını aştığı için analiz dışı bırakıldı.",
            preview.excluded_oversized.len()
        ));
    }

    Ok(RepoOverview {
        root: canonical_root,
        detected_languages,
        dependency_manifests,
        suggested_test_commands,
        file_count: preview.included.len(),
        total_bytes: preview.estimated_total_bytes,
        risk_notes,
        included_files: preview.included,
    })
}

/// Bir istekte modele önerilen dosya listesinin en fazla kaç tanesini gösterebileceğimiz —
/// prompt boyutunu (ve gerçek latency'yi, 16 Ağustos 2026'da ölçüldüğü gibi CPU-only modelde
/// prefill maliyeti gerçek) sınırlı tutmak için. Bundan fazla dosya varsa yalnız bir not
/// düşülüyor, tüm liste zorlanmıyor.
const MAX_CANDIDATE_FILES_IN_PROMPT: usize = 150;

/// F4 "Coding plan UX": kullanıcının doğal dille yazdığı bir değişiklik isteğini ve zaten
/// hesaplanmış bir `RepoOverview`'i alıp modele "bu istekle hangi dosyalar ilgili, test planı ne
/// olmalı" sorusunu sorar — hiçbir dosya açılmaz/yazılmaz, yalnız bir `CodingPlan` üretilir.
///
/// Modelin önerdiği dosya listesi **güvenilmez çıktıdır**: yalnız `overview.included_files`'ta
/// gerçekten var olan, birebir eşleşen yollar kabul edilir — aynı router'ın yalnız
/// `CapabilityRegistry`'deki tam bir üyeyi kabul etmesi ilkesiyle (model asla bir dosya yolu icat
/// edip kabul ettiremez). Model hiçbir dosya öneremezse ya da hiçbiri gerçek çıkmazsa, boş bir
/// `affected_files` ile dönen bir plan üretilir — bu bir hata değil, "kullanıcıya açıklayıcı soru
/// sorulmalı" durumunun kendisi (henüz ayrı bir soru-sorma UX'i yok, bu bilinçli bir sonraki adım).
pub fn draft_coding_plan_with_provider(
    overview: &RepoOverview,
    request_summary: &str,
    provider: &dyn ModelProvider,
) -> Result<CodingPlan, String> {
    let truncated = overview.included_files.len() > MAX_CANDIDATE_FILES_IN_PROMPT;
    let file_list = overview
        .included_files
        .iter()
        .take(MAX_CANDIDATE_FILES_IN_PROMPT)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let truncated_note = if truncated {
        format!(
            "\n... ve {} dosya daha listede yok.",
            overview.included_files.len() - MAX_CANDIDATE_FILES_IN_PROMPT
        )
    } else {
        String::new()
    };
    let languages = if overview.detected_languages.is_empty() {
        "bilinmiyor".to_string()
    } else {
        overview.detected_languages.join(", ")
    };
    let prompt = format!(
        "/no_think You are a read-only coding planning assistant for a local repository. You never write files or run commands — you only propose a plan. \
Given the repository's file list and a change request, output exactly two lines. \
First line: FILES: followed by a comma-separated list of ONLY the files from the list below that are actually relevant to the request, copied exactly as they appear in the list — never invent a path that is not in the list. If none of the listed files seem relevant, or you are not confident, write exactly: FILES: NONE. \
Second line: TESTS: a short, concrete description of how to verify the change (a command if you can infer one, or a short description otherwise). \
Repository languages: {languages}. \
Repository files:\n{file_list}{truncated_note}\n\n\
Change request: {request_summary}"
    );
    let response = provider
        .complete(&prompt)
        .map_err(|error| format!("coding plan draft failed: {error}"))?;

    let mut affected_files: Vec<PathBuf> = Vec::new();
    let mut test_plan: Vec<String> = Vec::new();
    for line in response.text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("FILES:") {
            let rest = rest.trim();
            if rest != "NONE" && !rest.is_empty() {
                for candidate in rest.split(',') {
                    let candidate_path = PathBuf::from(candidate.trim());
                    if overview.included_files.contains(&candidate_path)
                        && !affected_files.contains(&candidate_path)
                    {
                        affected_files.push(candidate_path);
                    }
                    if affected_files.len() >= 12 {
                        break; // `WorkerLimits::default().max_changed_files` ile aynı üst sınır
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("TESTS:") {
            let rest = rest.trim();
            if !rest.is_empty() {
                test_plan.push(rest.to_string());
            }
        }
    }
    if test_plan.is_empty() {
        test_plan = overview.suggested_test_commands.clone();
    }

    create_read_only_coding_plan(&overview.root, request_summary, affected_files, test_plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModelResponse;
    use std::fs;

    #[derive(Debug)]
    struct FixedReplyProvider(&'static str);

    impl ModelProvider for FixedReplyProvider {
        fn provider_id(&self) -> &str {
            "test"
        }
        fn model_id(&self) -> &str {
            "fixed-reply"
        }
        fn complete(&self, _prompt: &str) -> Result<ModelResponse, String> {
            Ok(ModelResponse {
                provider_id: self.provider_id().into(),
                model_id: self.model_id().into(),
                text: self.0.into(),
                structured_json: None,
                finish_reason: "stop".into(),
            })
        }
    }

    fn temporary_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "jarvis-project-analyst-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("test fixture root");
        root
    }

    #[test]
    fn detects_a_rust_project_from_its_cargo_manifest_and_suggests_cargo_test() {
        let root = temporary_repo("rust");
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

        let overview = analyze_repository(&root).expect("analysis succeeds");
        assert_eq!(overview.detected_languages, vec!["Rust".to_string()]);
        assert_eq!(
            overview.dependency_manifests,
            vec![PathBuf::from("Cargo.toml")]
        );
        assert_eq!(
            overview.suggested_test_commands,
            vec!["cargo test".to_string()]
        );
        assert_eq!(overview.file_count, 2);
        assert!(overview.risk_notes.is_empty());

        fs::remove_dir_all(&root).expect("test cleanup");
    }

    #[test]
    fn detects_multiple_manifests_as_multiple_languages_without_duplicating_python() {
        let root = temporary_repo("multi");
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(root.join("pyproject.toml"), "[project]\nname = \"demo\"\n").unwrap();
        fs::write(root.join("requirements.txt"), "requests\n").unwrap();

        let overview = analyze_repository(&root).expect("analysis succeeds");
        assert_eq!(overview.detected_languages, vec!["Rust", "Python"]);
        // requirements.txt kendisi hâlâ bir manifest dosyası olarak sayılıyor (dosya sistemine
        // gerçekten var), ama "Python" dili yalnız bir kez listelendi.
        assert_eq!(
            overview
                .detected_languages
                .iter()
                .filter(|language| *language == "Python")
                .count(),
            1
        );

        fs::remove_dir_all(&root).expect("test cleanup");
    }

    #[test]
    fn no_known_manifest_is_reported_as_a_risk_note_not_an_error() {
        let root = temporary_repo("unknown");
        fs::write(root.join("notes.md"), "sadece notlar\n").unwrap();

        let overview = analyze_repository(&root).expect("analysis still succeeds");
        assert!(overview.detected_languages.is_empty());
        assert!(overview
            .risk_notes
            .iter()
            .any(|note| note.contains("bilinen bir bağımlılık manifesti")));

        fs::remove_dir_all(&root).expect("test cleanup");
    }

    #[test]
    fn secret_like_files_are_never_read_only_counted_as_a_risk_note() {
        let root = temporary_repo("secret");
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(root.join(".env"), "API_KEY=super-secret\n").unwrap();

        let overview = analyze_repository(&root).expect("analysis succeeds");
        assert!(overview
            .risk_notes
            .iter()
            .any(|note| note.contains("gizli-bilgi benzeri")));

        fs::remove_dir_all(&root).expect("test cleanup");
    }

    #[test]
    fn a_missing_or_non_directory_root_is_rejected() {
        let missing = std::env::temp_dir().join("jarvis-project-analyst-does-not-exist-at-all");
        assert!(analyze_repository(&missing).is_err());
    }

    fn fixture_overview(name: &str, files: &[&str]) -> (PathBuf, RepoOverview) {
        let root = temporary_repo(name);
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        for file in files {
            let path = root.join(file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, "// içerik\n").unwrap();
        }
        let overview = analyze_repository(&root).expect("analysis succeeds");
        (root, overview)
    }

    /// Model'in önerdiği dosya listesi güvenilmez çıktıdır — yalnız gerçekten var olan, listeyle
    /// birebir eşleşen yollar kabul edilir. Burada model iki gerçek dosya + bir hayali dosya
    /// öneriyor; yalnız gerçek ikisi plana giriyor.
    #[test]
    fn only_files_that_actually_exist_in_the_scan_are_accepted_hallucinated_paths_are_dropped() {
        let (root, overview) = fixture_overview("plan-real", &["src/lib.rs", "src/model.rs"]);
        let provider = FixedReplyProvider(
            "FILES: src/lib.rs, src/model.rs, src/hayali_dosya_yok.rs\nTESTS: cargo test route",
        );

        let plan = draft_coding_plan_with_provider(&overview, "router'ı düzelt", &provider)
            .expect("plan is produced");
        assert_eq!(
            plan.affected_files,
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/model.rs")]
        );
        assert_eq!(plan.test_plan, vec!["cargo test route".to_string()]);

        fs::remove_dir_all(&root).expect("test cleanup");
    }

    /// Model "FILES: NONE" derse (isteği hangi dosyalarla ilgili olduğunu çıkaramazsa), hata değil
    /// — boş bir `affected_files` ile geçerli bir plan üretilir.
    #[test]
    fn the_model_saying_none_produces_an_empty_but_valid_plan_not_an_error() {
        let (root, overview) = fixture_overview("plan-none", &["src/lib.rs"]);
        let provider = FixedReplyProvider("FILES: NONE\nTESTS: belirsiz, önce netleştirilmeli");

        let plan = draft_coding_plan_with_provider(&overview, "bir şey yap", &provider)
            .expect("an empty-scope plan is still a valid plan, not an error");
        assert!(plan.affected_files.is_empty());

        fs::remove_dir_all(&root).expect("test cleanup");
    }

    /// Model bir TESTS satırı vermezse, `RepoOverview`'in zaten tespit ettiği önerilen test
    /// komutuna (ör. `cargo test`) düşülüyor — plan asla boş bir test planıyla kalmıyor.
    #[test]
    fn a_missing_tests_line_falls_back_to_the_detected_test_command() {
        let (root, overview) = fixture_overview("plan-fallback", &["src/lib.rs"]);
        let provider = FixedReplyProvider("FILES: src/lib.rs");

        let plan = draft_coding_plan_with_provider(&overview, "bir şey düzelt", &provider)
            .expect("plan is produced");
        assert_eq!(plan.test_plan, vec!["cargo test".to_string()]);

        fs::remove_dir_all(&root).expect("test cleanup");
    }

    /// Model aynı dosyayı iki kez önerirse (ya da yanıtı fazladan boşluk/çoklu virgül içerirse),
    /// plana yalnız bir kez giriyor.
    #[test]
    fn a_duplicated_file_in_the_model_reply_is_only_counted_once() {
        let (root, overview) = fixture_overview("plan-dup", &["src/lib.rs"]);
        let provider = FixedReplyProvider("FILES: src/lib.rs, src/lib.rs\nTESTS: cargo test");

        let plan = draft_coding_plan_with_provider(&overview, "bir şey düzelt", &provider)
            .expect("plan is produced");
        assert_eq!(plan.affected_files, vec![PathBuf::from("src/lib.rs")]);

        fs::remove_dir_all(&root).expect("test cleanup");
    }
}
