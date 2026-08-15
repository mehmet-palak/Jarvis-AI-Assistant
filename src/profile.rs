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

use crate::{MemoryNamespace, MemoryRecord};

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
