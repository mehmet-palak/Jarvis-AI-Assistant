//! Kullanıcının elle düzenlediği, serbest metin profil dosyaları — "bana dair" ve "JARVIS'e
//! dair" — kalıcı bellek sisteminden (`MemoryRecord`/`propose_memory`) kasıtlı olarak ayrı, ikinci
//! bir mekanizma. Fark şu: bellek kayıtları JARVIS'in kendi komutlarıyla (onay adımından geçerek)
//! yazılır; bu dosyalar kullanıcının kendi metin editörüyle doğrudan düzenlediği, JARVIS'in hiç
//! yazmadığı (yalnız okuduğu) dosyalardır — Claude'un kendi `CLAUDE.md`/hafıza dosyalarıyla aynı
//! desen.
//!
//! Model asla bunlardan tool/policy yetkisi kazanmaz — bellek kayıtları için geçerli olan "data,
//! instruction değil" ilkesi burada da aynen geçerli (`isolate_profile_file_as_data`), kullanıcı
//! bunları kendi yazmış olsa bile.

use std::fs;
use std::path::{Path, PathBuf};

pub const ABOUT_USER_FILE_NAME: &str = "about_user.md";
pub const ABOUT_JARVIS_FILE_NAME: &str = "about_jarvis.md";

/// Tek bir dosyanın modele taşınacak içerik üst sınırı — bir kullanıcı bu dosyaya yanlışlıkla koca
/// bir belge yapıştırırsa (asıl belge indeksleme işi zaten `/index`'in), bağlam bütçesini tek
/// başına tüketmesin diye. `MAX_WORKSPACE_CHUNK_CHARS`'ın birkaç katı — bu, kısa bir RAG parçasından
/// çok daha uzun, gerçek bir "profil belgesi" olabilir ama yine de sınırsız değil.
pub const MAX_PROFILE_FILE_CHARS: usize = 8_000;

const ABOUT_USER_TEMPLATE: &str = "\
# Bana dair

JARVIS'in her açılışta okuyacağı, senin elle düzenlediğin bir dosya. Buraya kendinle ilgili,
JARVIS'in bilmesini istediğin her şeyi serbest metin olarak yazabilirsin — tercihlerin, çalışma
biçimin, projelerin, ne tür cevaplar istediğin gibi.

Bu dosya bellek sisteminden (`/remember`) ayrıdır: JARVIS buraya hiçbir zaman kendi kendine yazmaz,
yalnız okur. Değiştirmek için bu dosyayı doğrudan bir metin editörüyle aç ve düzenle.
";

const ABOUT_JARVIS_TEMPLATE: &str = "\
# JARVIS'e dair

JARVIS'in her açılışta okuyacağı, senin elle düzenlediğin bir dosya. Buraya JARVIS'in nasıl
davranmasını istediğini, hangi kuralları hep hatırlamasını istediğini serbest metin olarak
yazabilirsin.

Bu dosya bellek sisteminden ayrıdır: JARVIS buraya hiçbir zaman kendi kendine yazmaz, yalnız okur.
Değiştirmek için bu dosyayı doğrudan bir metin editörüyle aç ve düzenle.
";

/// `~/.config/jarvis/profile/` (`XDG_CONFIG_HOME` varsa onu, yoksa `~/.config`'i temel alır) —
/// `desktop_config.rs`'in `default_desktop_preferences_path`'iyle aynı arama mantığı, ayrı bir alt
/// klasörde (desktop tercihleriyle karışmasın diye).
pub fn default_profile_files_dir() -> Option<PathBuf> {
    let config_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_root.join("jarvis").join("profile"))
}

/// Klasör ve iki dosya yoksa örnek şablonla oluşturur; zaten varsa hiçbir şeye dokunmaz
/// (kullanıcının yazdığı gerçek içeriği asla ezmez). En iyi çaba: yazma başarısız olursa (izin
/// sorunu vb.) sessizce devam eder — bu, JARVIS'in açılışını asla engellememesi gereken, isteğe
/// bağlı bir özellik.
pub fn ensure_profile_files_exist(dir: &Path) {
    let Ok(()) = fs::create_dir_all(dir) else {
        return;
    };
    for (name, template) in [
        (ABOUT_USER_FILE_NAME, ABOUT_USER_TEMPLATE),
        (ABOUT_JARVIS_FILE_NAME, ABOUT_JARVIS_TEMPLATE),
    ] {
        let path = dir.join(name);
        if !path.exists() {
            let _ = fs::write(path, template);
        }
    }
}

/// Bir profil dosyasını okur; yoksa, boşsa veya okunamıyorsa `None` — hepsi "bu dosya devre dışı"
/// anlamına gelir, hiçbiri hata değildir (bu tamamen isteğe bağlı bir özellik). İçerik
/// `MAX_PROFILE_FILE_CHARS`'a kırpılır.
pub fn read_profile_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_PROFILE_FILE_CHARS).collect())
}

/// Bellek kayıtları için kullanılan "data, instruction değil" zarfıyla aynı ilke
/// (`isolate_memory_as_data`) — kullanıcı bu dosyayı kendi yazmış olsa bile, model buradan hiçbir
/// zaman tool/policy yetkisi kazanmaz, yalnız bağlam verisi olarak görür.
pub fn isolate_profile_file_as_data(label: &str, content: &str) -> String {
    format!("<profile-file label=\"{label}\">\n{content}\n</profile-file>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_profile_files_exist_creates_templates_but_never_overwrites_real_content() {
        let dir = std::env::temp_dir().join(format!(
            "jarvis-profile-files-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        ensure_profile_files_exist(&dir);
        assert!(read_profile_file(&dir.join(ABOUT_USER_FILE_NAME)).is_some());
        assert!(read_profile_file(&dir.join(ABOUT_JARVIS_FILE_NAME)).is_some());

        fs::write(dir.join(ABOUT_USER_FILE_NAME), "gerçek kullanıcı içeriği").unwrap();
        ensure_profile_files_exist(&dir); // ikinci çağrı var olanı ezmemeli
        assert_eq!(
            read_profile_file(&dir.join(ABOUT_USER_FILE_NAME)),
            Some("gerçek kullanıcı içeriği".to_string())
        );

        fs::remove_dir_all(&dir).expect("test cleanup");
    }

    #[test]
    fn read_profile_file_treats_missing_or_empty_as_none_not_an_error() {
        let dir = std::env::temp_dir().join(format!(
            "jarvis-profile-files-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        assert_eq!(read_profile_file(&dir.join("yok.md")), None);

        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("bos.md"), "   \n  ").unwrap();
        assert_eq!(read_profile_file(&dir.join("bos.md")), None);

        fs::remove_dir_all(&dir).expect("test cleanup");
    }

    #[test]
    fn read_profile_file_truncates_to_the_char_limit() {
        let dir = std::env::temp_dir().join(format!(
            "jarvis-profile-files-huge-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("huge.md"),
            "a".repeat(MAX_PROFILE_FILE_CHARS + 500),
        )
        .unwrap();
        let content = read_profile_file(&dir.join("huge.md")).expect("content present");
        assert_eq!(content.chars().count(), MAX_PROFILE_FILE_CHARS);

        fs::remove_dir_all(&dir).expect("test cleanup");
    }
}
