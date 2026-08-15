//! Kullanıcı profili şeması.
//!
//! Bir profil alanı, genel bellek sisteminin (`MemoryNamespace::UserProfile`) üzerine oturan
//! sabit isimli bir anahtar kümesidir. Depolama, onay, denetim (audit) ve silme zaten
//! `propose_memory`/`commit_memory_proposal`/`delete_memory` tarafından sağlanıyor; bu modül
//! **ikinci bir depolama yolu açmaz**, yalnız "hangi anahtarlar profil sayılır" ve "değerleri
//! nasıl doğrularız" sorularını cevaplar. Bkz. ADR-0003.
//!
//! Sohbetten otomatik kalıcı yazma yasağı burada da geçerli: bu modül yalnız açık kullanıcı
//! komutlarının (örn. `/remember`) çağıracağı bir yardımcıdır, kendi başına hiçbir depolama
//! tetiklemez ve model tarafından doğrudan çağrılamaz.

use crate::{
    propose_memory, turkish_case_fold, DataSensitivity, MemoryNamespace, MemoryProposal,
    MemoryRecord,
};
use serde_json::json;

/// Bilinen profil alanları. Kullanıcı bunların dışında serbest anahtarlar da tutabilir (genel
/// bellek sistemi zaten bunu destekliyor); bu liste yalnız profil CRUD arayüzünün göstereceği
/// sabit alan kümesidir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileField {
    DisplayName,
    PreferredAddress,
    Language,
    RolePreference,
}

impl ProfileField {
    pub const ALL: [ProfileField; 4] = [
        ProfileField::DisplayName,
        ProfileField::PreferredAddress,
        ProfileField::Language,
        ProfileField::RolePreference,
    ];

    /// Bellekte saklanan sabit anahtar. Bu string değişirse eski kayıtlar artık "bilinen" profil
    /// alanı olarak tanınmaz; bu yüzden kasıtlı olarak sabit tutulur, ileride değişirse ADR'de
    /// bir migration notu gerekir.
    pub fn memory_key(self) -> &'static str {
        match self {
            Self::DisplayName => "display_name",
            Self::PreferredAddress => "preferred_address",
            Self::Language => "language",
            Self::RolePreference => "role_preference",
        }
    }

    /// Kullanıcıya gösterilecek Türkçe etiket.
    pub fn label(self) -> &'static str {
        match self {
            Self::DisplayName => "Ad",
            Self::PreferredAddress => "Hitap biçimi",
            Self::Language => "Dil",
            Self::RolePreference => "Rol / tercih",
        }
    }

    pub fn from_memory_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|field| field.memory_key() == key)
    }

    /// Kısa Türkçe takma ad (`ad`, `hitap`, `dil`, `rol`). Komut satırında `memory_key`'in tam
    /// İngilizce hâlini yazmak zorunda kalmasın diye.
    fn short_alias(self) -> &'static str {
        match self {
            Self::DisplayName => "ad",
            Self::PreferredAddress => "hitap",
            Self::Language => "dil",
            Self::RolePreference => "rol",
        }
    }

    /// Kullanıcının komut satırında yazdığı serbest metni bilinen bir alana çözer; hem tam
    /// bellek anahtarını (`display_name`) hem kısa takma adı (`ad`) kabul eder, büyük/küçük harf
    /// duyarsız.
    pub fn from_user_input(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        // English memory keys (e.g. "DISPLAY_NAME") and Turkish aliases (e.g. "DİL") need
        // different case-folding rules for a bare 'I'/'İ' — trying plain ASCII-lowercase first
        // and falling back to the Turkish-aware fold covers both without guessing the language.
        // See `parse_data_sensitivity` in lib.rs for the same issue and a longer explanation.
        for normalized in [trimmed.to_lowercase(), turkish_case_fold(trimmed)] {
            if let Some(field) = Self::ALL
                .into_iter()
                .find(|field| field.memory_key() == normalized || field.short_alias() == normalized)
            {
                return Some(field);
            }
        }
        None
    }
}

/// Bir profil alanı için bellek teklifi oluşturur. Doğrulama burada yapılır; namespace her zaman
/// `UserProfile`, anahtar her zaman `field.memory_key()`'dir — çağıran taraf (TUI/native) yanlış
/// namespace veya anahtar seçemez. Bu, `propose_memory`'nin üzerine ince bir sarmalayıcıdır;
/// depolamayı yine `commit_memory_proposal` yapar, teklif burada henüz kalıcı değildir.
pub fn propose_profile_field(
    field: ProfileField,
    value: &str,
    source: impl Into<String>,
    include_in_model_context: bool,
) -> Result<MemoryProposal, String> {
    validate_profile_value(field, value)?;
    propose_memory(
        MemoryNamespace::UserProfile,
        field.memory_key(),
        value.trim(),
        DataSensitivity::Internal,
        source,
        include_in_model_context,
        None,
    )
}

const PROFILE_VALUE_MAX_LEN: usize = 200;

/// Bir profil alanı değerini kaydetmeden önce doğrular. Boş, yalnız boşluk, çok uzun veya kontrol
/// karakteri içeren değerler reddedilir; dil/isim gibi alanlar için sabit bir kelime listesi
/// dayatılmaz (kullanıcı kendi dilinde/biçiminde yazabilsin diye — bu da bilinçli bir tercih,
/// bkz. ADR-0003).
pub fn validate_profile_value(field: ProfileField, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{} boş olamaz.", field.label()));
    }
    if trimmed.chars().count() > PROFILE_VALUE_MAX_LEN {
        return Err(format!(
            "{} en fazla {PROFILE_VALUE_MAX_LEN} karakter olabilir.",
            field.label()
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(format!("{} kontrol karakteri içeremez.", field.label()));
    }
    Ok(())
}

/// Genel bellek kayıtları arasından bilinen profil alanlarının **en güncel** değerini seçer.
/// Aynı alan birden çok kez `/remember` ile yazılmışsa (bellek sistemi bunu engellemiyor — her
/// onay yeni bir kayıt oluşturur), en son güncellenen kazanır; daha eskiler yalnız `/memory`
/// üzerinden görünür ve elle silinebilir.
#[derive(Debug, Clone, Default)]
pub struct ProfileSnapshot {
    pub display_name: Option<MemoryRecord>,
    pub preferred_address: Option<MemoryRecord>,
    pub language: Option<MemoryRecord>,
    pub role_preference: Option<MemoryRecord>,
}

impl ProfileSnapshot {
    /// Belirli bir alanın şu anki (varsa) kaydını döner. UI, kullanıcıya "sil" seçeneği
    /// sunarken ham `memory_id`'yi bilmesine gerek kalmadan bunu kullanabilir.
    pub fn record_for(&self, field: ProfileField) -> Option<&MemoryRecord> {
        match field {
            ProfileField::DisplayName => self.display_name.as_ref(),
            ProfileField::PreferredAddress => self.preferred_address.as_ref(),
            ProfileField::Language => self.language.as_ref(),
            ProfileField::RolePreference => self.role_preference.as_ref(),
        }
    }

    pub fn from_records(records: &[MemoryRecord]) -> Self {
        let mut snapshot = Self::default();
        for record in records {
            if record.namespace != MemoryNamespace::UserProfile {
                continue;
            }
            let Some(field) = ProfileField::from_memory_key(&record.key) else {
                continue;
            };
            let slot = match field {
                ProfileField::DisplayName => &mut snapshot.display_name,
                ProfileField::PreferredAddress => &mut snapshot.preferred_address,
                ProfileField::Language => &mut snapshot.language,
                ProfileField::RolePreference => &mut snapshot.role_preference,
            };
            let is_newer = slot
                .as_ref()
                .is_none_or(|current| record.updated_at >= current.updated_at);
            if is_newer {
                *slot = Some(record.clone());
            }
        }
        snapshot
    }

    /// Şu an değeri olan bilinen alanların listesi. `/profile reset` gibi "hepsini temizle"
    /// akışlarının hangi kayıtları sileceğini bulmasında kullanılır.
    pub fn populated_fields(&self) -> Vec<ProfileField> {
        ProfileField::ALL
            .into_iter()
            .filter(|field| self.record_for(*field).is_some())
            .collect()
    }
}

/// Profili, ek makbuzu dışa aktarımıyla aynı üslupta bir JSON belgesine çevirir: yalnız bilinen
/// dört alanın anahtarı/değeri/güncellenme zamanı/model-context durumu içerir. Ham `memory_id`,
/// `source` veya profil dışı serbest bellek kayıtları hiç dahil edilmez.
pub fn profile_manifest(snapshot: &ProfileSnapshot) -> Result<String, String> {
    let fields = ProfileField::ALL
        .into_iter()
        .filter_map(|field| {
            snapshot.record_for(field).map(|record| {
                json!({
                    "field": field.memory_key(),
                    "label": field.label(),
                    "value": record.value,
                    "include_in_model_context": record.include_in_model_context,
                    "updated_at": record.updated_at,
                })
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({
        "schema_version": 1,
        "kind": "jarvis-user-profile",
        "fields": fields,
    }))
    .map(|serialized| format!("{serialized}\n"))
    .map_err(|error| format!("profile manifest serialization failed: {error}"))
}

#[cfg(test)]
mod manifest_and_reset_tests {
    use super::*;
    use crate::DataSensitivity;

    fn record(key: &str, value: &str, namespace: MemoryNamespace, updated_at: u64) -> MemoryRecord {
        MemoryRecord {
            schema_version: 1,
            memory_id: format!("memory-{key}-{updated_at}"),
            namespace,
            key: key.into(),
            value: value.into(),
            sensitivity: DataSensitivity::Internal,
            source: "test".into(),
            include_in_model_context: true,
            created_at: updated_at,
            updated_at,
            expires_at: None,
        }
    }

    #[test]
    fn populated_fields_lists_only_fields_with_a_current_record() {
        let records = vec![
            record("display_name", "Mehmet", MemoryNamespace::UserProfile, 1),
            record("language", "tr", MemoryNamespace::UserProfile, 2),
        ];
        let snapshot = ProfileSnapshot::from_records(&records);
        assert_eq!(
            snapshot.populated_fields(),
            vec![ProfileField::DisplayName, ProfileField::Language]
        );
    }

    #[test]
    fn manifest_contains_only_known_fields_never_raw_memory_id_or_source() {
        let records = vec![record(
            "display_name",
            "Mehmet",
            MemoryNamespace::UserProfile,
            1,
        )];
        let snapshot = ProfileSnapshot::from_records(&records);
        let manifest = profile_manifest(&snapshot).expect("manifest serializes");
        assert!(manifest.contains("\"value\": \"Mehmet\""));
        assert!(manifest.contains("jarvis-user-profile"));
        assert!(!manifest.contains("memory_id"));
        assert!(!manifest.contains("\"source\""));
    }

    #[test]
    fn empty_snapshot_produces_an_empty_field_list_not_an_error() {
        let manifest = profile_manifest(&ProfileSnapshot::default()).expect("still serializes");
        assert!(manifest.contains("\"fields\": []"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataSensitivity;

    fn record(key: &str, value: &str, namespace: MemoryNamespace, updated_at: u64) -> MemoryRecord {
        MemoryRecord {
            schema_version: 1,
            memory_id: format!("memory-{key}-{updated_at}"),
            namespace,
            key: key.into(),
            value: value.into(),
            sensitivity: DataSensitivity::Internal,
            source: "test".into(),
            include_in_model_context: true,
            created_at: updated_at,
            updated_at,
            expires_at: None,
        }
    }

    #[test]
    fn every_known_field_round_trips_through_its_memory_key() {
        for field in ProfileField::ALL {
            assert_eq!(
                ProfileField::from_memory_key(field.memory_key()),
                Some(field)
            );
        }
        assert_eq!(ProfileField::from_memory_key("not-a-profile-key"), None);
    }

    #[test]
    fn value_validation_rejects_empty_oversized_and_control_characters() {
        assert!(validate_profile_value(ProfileField::DisplayName, "Mehmet").is_ok());
        assert!(validate_profile_value(ProfileField::DisplayName, "   ").is_err());
        assert!(validate_profile_value(ProfileField::DisplayName, &"a".repeat(201)).is_err());
        assert!(validate_profile_value(ProfileField::DisplayName, "Meh\u{0007}met").is_err());
    }

    #[test]
    fn user_input_accepts_both_the_memory_key_and_the_short_turkish_alias() {
        assert_eq!(
            ProfileField::from_user_input("display_name"),
            Some(ProfileField::DisplayName)
        );
        assert_eq!(
            ProfileField::from_user_input("Ad"),
            Some(ProfileField::DisplayName)
        );
        // "İ" (noktalı büyük I) standart to_lowercase() ile 'i' + birleşik nokta işaretine döner;
        // turkish_case_fold bunu doğru şekilde düz 'i'ye çevirir.
        assert_eq!(
            ProfileField::from_user_input("  DİL  "),
            Some(ProfileField::Language)
        );
        // Regresyon: "DISPLAY_NAME" (İngilizce, büyük harf) turkish_case_fold ile TEK BAŞINA
        // denenseydi 'I' Türkçe kuralına göre 'ı'ya döner ve "dısplay_name" hiç eşleşmezdi.
        assert_eq!(
            ProfileField::from_user_input("DISPLAY_NAME"),
            Some(ProfileField::DisplayName)
        );
        assert_eq!(ProfileField::from_user_input("bilinmeyen"), None);
    }

    #[test]
    fn propose_profile_field_validates_before_building_a_proposal() {
        let proposal =
            propose_profile_field(ProfileField::DisplayName, "Mehmet", "test-source", true)
                .expect("valid value proposes cleanly");
        assert_eq!(proposal.record.key, "display_name");
        assert_eq!(proposal.record.value, "Mehmet");
        assert_eq!(proposal.record.namespace, MemoryNamespace::UserProfile);
        assert!(
            propose_profile_field(ProfileField::DisplayName, "   ", "test-source", true).is_err()
        );
    }

    #[test]
    fn record_for_exposes_the_current_record_without_a_raw_memory_id() {
        let records = vec![record(
            "preferred_address",
            "Mehmet Bey",
            MemoryNamespace::UserProfile,
            1,
        )];
        let snapshot = ProfileSnapshot::from_records(&records);
        assert_eq!(
            snapshot
                .record_for(ProfileField::PreferredAddress)
                .unwrap()
                .value,
            "Mehmet Bey"
        );
        assert!(snapshot.record_for(ProfileField::DisplayName).is_none());
    }

    #[test]
    fn snapshot_keeps_only_the_newest_record_per_field_and_ignores_other_namespaces() {
        let records = vec![
            record("display_name", "Eski", MemoryNamespace::UserProfile, 10),
            record("display_name", "Yeni", MemoryNamespace::UserProfile, 20),
            record("language", "tr", MemoryNamespace::UserProfile, 5),
            // Aynı anahtar başka bir namespace'te ise profil sayılmaz.
            record("language", "proje-dili-en", MemoryNamespace::Project, 999),
            // Bilinmeyen bir anahtar sessizce görmezden gelinir; genel /memory listesinde durur.
            record("favori_renk", "teal", MemoryNamespace::UserProfile, 30),
        ];
        let snapshot = ProfileSnapshot::from_records(&records);
        assert_eq!(snapshot.display_name.unwrap().value, "Yeni");
        assert_eq!(snapshot.language.unwrap().value, "tr");
        assert!(snapshot.preferred_address.is_none());
        assert!(snapshot.role_preference.is_none());
    }
}
