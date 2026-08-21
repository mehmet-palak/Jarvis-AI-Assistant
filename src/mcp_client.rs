//! F8 MCP **egress** ("istemci yönü") + eklenti (plugin/skill) güven çekirdeği — [ADR-0008].
//!
//! Bu modül, JARVIS'in DIŞ, güvenmediği bir MCP sunucusunu/eklentisini kullanmadan önce
//! uygulanan **deny-by-default** güven kararının tipli çekirdeğidir. ADR-0008'in çatı kuralı burada
//! yaşıyor: *dış bir MCP aracı, izole edilmiş güvenilmez bir veri kaynağından ibarettir* — bu
//! yüzden JARVIS ona bağlanmadan önce her şey kanıtlanmalıdır.
//!
//! **Eklenti = dış araç:** ADR-0008'de netleştiği gibi bir eklenti, teknik olarak izole, imzalı bir
//! dış MCP aracıdır. Bu yüzden F8'in "eklenti/skill ekosistemi" maddesi ayrı bir sistem değil,
//! aynı `McpServerManifest`/güven çekirdeğinin bir `McpServerKind::Plugin` etiketiyle
//! kullanılmasıdır — iki mekanizma değil, tek mekanizma.
//!
//! **Bu dosyanın kapsamı (Faz 1 — iskelet):** tipli manifest + imzalama (F7'nin HMAC-imzalı scope
//! deseniyle aynı ilkeller) + doğrulama + tedarik-zinciri (rug-pull) için artefakt-hash sabitleme +
//! egress protokol sürüm kontrolü + tek deny-by-default bağlanma kapısı. Gerçek süreç başlatma
//! (F4 sandbox'ında), rıza ekranı, veri-akış filtreleri ve sampling/resources/prompts ele alımı
//! sonraki fazların (ADR-0008 sıra 2-6) işi.

use crate::persistence::{constant_time_eq, encode_hex, hmac_sha256};
use crate::DataSensitivity;

/// Bu build'in anladığı MCP **egress** tel-protokol sürümü. Bir dış sunucunun bildirdiği
/// `protocolVersion` bundan farklıysa bağlanma reddedilir — ingress'teki
/// `validate_mcp_protocol_version`'ın simetriği (ADR-0008 Katman 0). Anlaşılmayan bir protokolde bir
/// aracın yanıtını yorumlamak, sessizce yanlış-yorumlanmış veriyi güvenmek olurdu.
pub const CURRENT_MCP_CLIENT_PROTOCOL_VERSION: u16 = 1;

/// Bir kayıt defteri girdisinin türü — güven mekanizması aynı olsa da kullanıcıya "bu bir MCP aracı
/// mı yoksa bir eklenti mi" ayrımını göstermek için (ADR-0008: eklenti = dış araç).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerKind {
    ExternalTool,
    Plugin,
}

impl McpServerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExternalTool => "external_tool",
            Self::Plugin => "plugin",
        }
    }
}

/// Bir kayıtlı sunucunun durumu. Deny-by-default: yalnız `Active` bir sunucuya bağlanılır;
/// `Quarantined` (kural ihlali sonrası otomatik) ve `Revoked` (kullanıcı iptali) bağlanmayı
/// reddeder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerStatus {
    Active,
    Quarantined,
    Revoked,
}

impl McpServerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Quarantined => "quarantined",
            Self::Revoked => "revoked",
        }
    }
}

/// Dış sunucunun nasıl başlatılacağı. Şu an yalnız yerel alt-süreç (stdio); ağ taşımaları (HTTP/SSE)
/// bilinçli olarak F10'a erteli ("sunucu/internet ertelendi" kararıyla tutarlı).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    Stdio { command: String, args: Vec<String> },
}

impl McpTransport {
    fn canonical_string(&self) -> String {
        match self {
            Self::Stdio { command, args } => {
                format!("stdio\u{1f}{command}\u{1f}{}", args.join("\u{1e}"))
            }
        }
    }
}

/// Bir dış MCP sunucusunun/eklentisinin bildirimi. Bu, kullanıcının onayladığı güven sözleşmesidir;
/// `artifact_hash` dışında hiçbir alan çalışma anında değişmez (değişirse imza tutmaz).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerManifest {
    pub schema_version: u16,
    /// Kayıt defterindeki kararlı kimlik (kullanıcıya ait, otomatik keşif yok).
    pub id: String,
    pub display_name: String,
    pub kind: McpServerKind,
    pub transport: McpTransport,
    /// Sunucunun sunduğunu bildirdiği araç kimlikleri (kullanıcıya onay ekranında aynen gösterilir —
    /// ADR-0008 "tool poisoning"e karşı).
    pub declared_tools: Vec<String>,
    /// Bu sunucunun eşlenebileceği JARVIS yetenekleri — beyaz-liste, deny-by-default.
    pub capability_allowlist: Vec<String>,
    /// Bu sunucuya gönderilebilecek azami veri hassasiyeti (tavan).
    pub sensitivity_ceiling: DataSensitivity,
    /// Ağ erişimi — deny-by-default; yalnız manifest bildirir ve kullanıcı onaylarsa `true`.
    pub network_allowed: bool,
    /// Sunucuyu başlatan artefaktın (binary/script) SHA-256'sı — **tedarik zinciri / rug-pull**
    /// sabitlemesi. `npx-latest` gibi kod değişse hash değişir, bağlanma reddedilir (ADR-0008).
    pub artifact_hash: String,
}

/// Manifest + imzası. İmza, manifestin kanonik baytları üzerinden HMAC-SHA256'dır (F7 scope
/// imzalama deseni) — kurcalama tespiti içindir, gerçek bir yetki doğrulaması değil (ADR-0008'in
/// dürüst sınırı).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedMcpManifest {
    pub manifest: McpServerManifest,
    pub signature: String,
}

/// `DataSensitivity` için tavan-karşılaştırması sıralaması (Public < Internal < Sensitive). Enum'un
/// kendisinde sıralama yok; bu tavan kontrolü MCP'ye özel.
fn sensitivity_rank(sensitivity: DataSensitivity) -> u8 {
    match sensitivity {
        DataSensitivity::Public => 0,
        DataSensitivity::Internal => 1,
        DataSensitivity::Sensitive => 2,
    }
}

/// Bir veri hassasiyeti, sunucunun tavanının altında/eşit mi? (Faz 4 veri-akış kontrolü bunu
/// kullanacak; çekirdek karar burada tipli olarak yaşasın diye şimdi tanımlı.)
pub fn sensitivity_within_ceiling(data: DataSensitivity, ceiling: DataSensitivity) -> bool {
    sensitivity_rank(data) <= sensitivity_rank(ceiling)
}

/// Her güvenlik-ilgili alanı uzunluk-önekleyerek kanonik baytlar üretir (F7'nin
/// `canonical_pentest_scope_bytes`'ı ile aynı gerekçe: uzunluk öneki olmadan bitişik alanlar sınır
/// belirsizliği yaratır). Manifestte imzayı etkilemesi gereken HER alan buraya girmeli — biri
/// eksik kalırsa o alan sessizce kurcalanabilir.
fn canonical_mcp_manifest_bytes(manifest: &McpServerManifest) -> Vec<u8> {
    let fields = [
        manifest.schema_version.to_string(),
        manifest.id.clone(),
        manifest.display_name.clone(),
        manifest.kind.as_str().to_string(),
        manifest.transport.canonical_string(),
        manifest.declared_tools.join("\n"),
        manifest.capability_allowlist.join("\n"),
        manifest.sensitivity_ceiling.as_str().to_string(),
        manifest.network_allowed.to_string(),
        manifest.artifact_hash.clone(),
    ];
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes
}

/// Manifesti imzalar (hex HMAC-SHA256). Anahtar yönetimi çağırana ait (store, pentest scope'larla
/// aynı özel imza-anahtarı tablosunu kullanır) — bu fonksiyon saf ve test edilebilir kalsın diye
/// anahtarı parametre alır.
pub fn sign_mcp_manifest(key: &[u8; 32], manifest: &McpServerManifest) -> String {
    encode_hex(&hmac_sha256(key, &canonical_mcp_manifest_bytes(manifest)))
}

/// Manifestin imzasını **sabit zamanlı** doğrular. Kurcalanmış herhangi bir alan (tool listesi,
/// yetenek beyaz-listesi, hassasiyet tavanı, artefakt hash'i, ...) imzayı bozar.
pub fn verify_mcp_manifest(key: &[u8; 32], manifest: &McpServerManifest, signature: &str) -> bool {
    let expected = sign_mcp_manifest(key, manifest);
    constant_time_eq(expected.as_bytes(), signature.as_bytes())
}

/// Bir dış sunucunun bildirdiği egress protokol sürümünü doğrular (ingress'in simetriği).
pub fn validate_external_mcp_protocol_version(version: u16) -> Result<(), String> {
    if version == CURRENT_MCP_CLIENT_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(format!(
            "desteklenmeyen dış MCP protokol sürümü {version} — bu build yalnız sürüm {CURRENT_MCP_CLIENT_PROTOCOL_VERSION}'i anlıyor; bağlanma reddedildi"
        ))
    }
}

/// Manifestin kendi içinde tutarlı ve deny-by-default kurallarına uygun olup olmadığını denetler.
/// Bu, tek policy-gate doğrulayıcısıdır — hiçbir çağıran bu kontrolü atlayarak geçersiz bir manifest
/// kaydedemesin diye (F7'nin `validate_pentest_scope`'u ile aynı disiplin).
pub fn validate_mcp_manifest(manifest: &McpServerManifest) -> Result<(), String> {
    if manifest.schema_version != CURRENT_MCP_CLIENT_PROTOCOL_VERSION {
        return Err(format!(
            "manifest schema_version {} desteklenmiyor (beklenen {CURRENT_MCP_CLIENT_PROTOCOL_VERSION})",
            manifest.schema_version
        ));
    }
    if manifest.id.trim().is_empty() {
        return Err("manifest id boş olamaz".into());
    }
    if manifest.display_name.trim().is_empty() {
        return Err("manifest display_name boş olamaz".into());
    }
    match &manifest.transport {
        McpTransport::Stdio { command, .. } => {
            if command.trim().is_empty() {
                return Err("stdio transport için komut boş olamaz".into());
            }
        }
    }
    if manifest.artifact_hash.len() != 64
        || !manifest
            .artifact_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("manifest artifact_hash 64 karakterlik bir SHA-256 hex olmalı (tedarik-zinciri sabitlemesi)".into());
    }
    // Deny-by-default: bir sunucu hiçbir yeteneğe eşlenemiyorsa onu bağlamanın anlamı yok; boş
    // beyaz-liste sessizce "her şey" değil, "hiçbir şey"tir — ve o zaman kayıt bir hatadır.
    if manifest.capability_allowlist.is_empty() {
        return Err("manifest capability_allowlist boş olamaz (deny-by-default: en az bir açık yetenek gerekli)".into());
    }
    if manifest
        .capability_allowlist
        .iter()
        .any(|cap| cap.trim().is_empty())
    {
        return Err("capability_allowlist boş bir yetenek adı içeremez".into());
    }
    Ok(())
}

/// Bir bağlanma denemesinin reddedilme nedeni — tipli, böylece audit/UI net bir gerekçe gösterir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpConnectRejection {
    NotActive(McpServerStatus),
    InvalidManifest(String),
    SignatureMismatch,
    ArtifactChanged,
    UnsupportedProtocol(String),
}

impl McpConnectRejection {
    pub fn reason(&self) -> String {
        match self {
            Self::NotActive(status) => {
                format!("sunucu aktif değil (durum: {}) — bağlanma reddedildi", status.as_str())
            }
            Self::InvalidManifest(why) => format!("geçersiz manifest: {why}"),
            Self::SignatureMismatch => {
                "manifest imzası doğrulanamadı (kurcalanmış olabilir) — bağlanma reddedildi".into()
            }
            Self::ArtifactChanged => {
                "sunucu artefaktı onaylandığından beri DEĞİŞTİ (olası rug-pull) — yeniden onay gerekir, bağlanma reddedildi".into()
            }
            Self::UnsupportedProtocol(why) => why.clone(),
        }
    }
}

/// Bir dış MCP sunucusuna/eklentisine bağlanmadan önce uygulanan **tek deny-by-default kapı**
/// (F7'nin `authorize_pentest_target`'ı ile aynı "tek giriş noktası" deseni). Sıra önemlidir:
/// önce kayıt durumu, sonra manifest bütünlüğü/imzası, sonra tedarik-zinciri (artefakt hash'i), en
/// son protokol. Her kontrol ayrı geçmeli; herhangi biri düşerse tipli bir ret döner.
///
/// - `signing_key`: store'un özel imza anahtarı.
/// - `approved_artifact_hash`: kullanıcının onayladığı andaki artefakt hash'i (kayıt defterinde).
/// - `live_artifact_hash`: şu an diskteki artefaktın yeniden hesaplanmış hash'i.
/// - `advertised_protocol_version`: sunucunun `initialize` yanıtında bildirdiği sürüm.
#[allow(clippy::too_many_arguments)]
pub fn authorize_mcp_connect(
    signed: &SignedMcpManifest,
    signing_key: &[u8; 32],
    status: McpServerStatus,
    approved_artifact_hash: &str,
    live_artifact_hash: &str,
    advertised_protocol_version: u16,
) -> Result<(), McpConnectRejection> {
    if status != McpServerStatus::Active {
        return Err(McpConnectRejection::NotActive(status));
    }
    validate_mcp_manifest(&signed.manifest).map_err(McpConnectRejection::InvalidManifest)?;
    if !verify_mcp_manifest(signing_key, &signed.manifest, &signed.signature) {
        return Err(McpConnectRejection::SignatureMismatch);
    }
    // Tedarik zinciri: onaylanan artefakt ile şu an diskteki artefakt AYNI olmalı. Sabit-zamanlı
    // karşılaştırma — bu bir hash eşleşmesi, ama yine de imza-benzeri bir eşitlik kontrolü.
    if !constant_time_eq(
        approved_artifact_hash.as_bytes(),
        live_artifact_hash.as_bytes(),
    ) {
        return Err(McpConnectRejection::ArtifactChanged);
    }
    validate_external_mcp_protocol_version(advertised_protocol_version)
        .map_err(McpConnectRejection::UnsupportedProtocol)?;
    Ok(())
}

/// Bir artefakt dosyasının (sunucu binary'si/script'i) SHA-256 hex'ini hesaplar — hem onay anında
/// (`approved_artifact_hash`'i üretmek) hem bağlanma anında (`live_artifact_hash`) kullanılır.
/// Dosya `MAX_ARTIFACT_BYTES` ile sınırlı okunur.
pub fn hash_artifact(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("artefakt açılamadı ({}): {error}", path.display()))?;
    let mut reader = file.take(MAX_ARTIFACT_BYTES);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("artefakt okunamadı: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(encode_hex(&hasher.finalize()))
}

#[cfg(test)]
#[path = "mcp_client_tests.rs"]
mod tests;
