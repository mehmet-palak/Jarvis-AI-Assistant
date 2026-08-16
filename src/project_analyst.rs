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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
}
