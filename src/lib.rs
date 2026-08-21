//! JARVIS implementation baseline: a small, typed, policy-gated vertical slice.

pub mod attachments;
pub mod calendar;
mod capabilities;
#[cfg(test)]
mod coding_eval;
pub mod command_runner;
pub mod dataset;
pub mod desktop_config;
pub mod embedding;
pub mod mcp_client;
pub mod mcp_egress;
mod memory_intent;
mod model;
mod model_quality_eval;
pub mod patch_generator;
pub mod pentest_evidence;
pub mod pentest_intent;
pub mod pentest_knowledge;
pub mod pentest_network_gate;
pub mod pentest_osint;
pub mod pentest_recon;
pub mod pentest_replay;
pub mod pentest_reporting;
pub mod pentest_safe_checks;
mod persistence;
mod policy;
mod profile;
pub mod profile_files;
pub mod project_analyst;
pub mod quality_eval;
mod runtime;
mod seccomp_filter;
pub mod vision;
pub mod voice;
pub mod weather;
pub mod workbench;
pub mod workflow;
mod workspace;

pub use attachments::{
    attachment_receipt_manifest, inspect_local_attachment, inspect_local_document,
    inspect_local_image, revalidate_local_attachment, validate_attachment, AttachmentKind,
    AttachmentReceipt, AttachmentRef,
};
pub use calendar::{
    default_calendar_path, CalendarEvent, CalendarProvider, EventDate, LocalIcsCalendarProvider,
};
pub use capabilities::{capability_manifest, CapabilityRegistry};
pub use command_runner::{
    run_allowlisted_command, run_test_plan, validate_command_line, CommandRun, TestRunReport,
};
pub use dataset::{
    build_dataset_export, compare_model_config_runs, DatasetExclusion, DatasetExport,
    DatasetMarker, DatasetMarkerKind, ModelConfigComparison, ModelConfigVerdict,
};
pub use desktop_config::{
    default_desktop_preferences_path, load_desktop_preferences, save_desktop_preferences,
    DesktopPreferences, ThemePreference,
};
pub use embedding::{
    cosine_similarity, deserialize_embedding, serialize_embedding, EmbeddingProvider,
    LlamaEmbeddingProvider,
};
pub use mcp_client::{
    authorize_mcp_connect, hash_artifact, sign_mcp_manifest,
    validate_external_mcp_protocol_version, validate_mcp_manifest, verify_mcp_manifest,
    McpConnectRejection, McpServerKind, McpServerManifest, McpServerStatus, McpTransport,
    RegisteredMcpServer, SignedMcpManifest, CURRENT_MCP_CLIENT_PROTOCOL_VERSION,
};
pub use mcp_egress::{McpEgressSession, McpRpcTransport, SandboxedStdioTransport};
pub use memory_intent::{
    parse_memory_intent, propose_unrecognized_remember_intent_with_provider, MemoryIntent,
};
pub use model::{
    normalize_llama_cli_output, route_with_provider, DeterministicModelProvider, IntentResolution,
    LlamaCliProvider, LlamaServerProvider, ModelProvider, ModelResponse, ModelRuntimeState,
    RouteSource,
};
pub use patch_generator::draft_patch_with_provider;
pub use persistence::SqliteStore;
pub use policy::{
    approval_channel_requirement, authorize_pentest_target, classify,
    feedback_candidate_is_promotable, policy_for, validate_feedback_candidate,
    validate_model_config_run, validate_pentest_scope, validate_request, validate_teacher_example,
    voice_approval_is_sufficient, ApprovalChannelRequirement,
};
pub use profile::{
    profile_manifest, propose_profile_field, validate_profile_value, ProfileField, ProfileSnapshot,
};
pub use profile_files::{
    default_profile_files_dir, ensure_profile_files_exist, isolate_profile_file_as_data,
    read_profile_file, ABOUT_JARVIS_FILE_NAME, ABOUT_USER_FILE_NAME, MAX_PROFILE_FILE_CHARS,
};
pub use project_analyst::{analyze_repository, draft_coding_plan_with_provider, RepoOverview};
pub use runtime::{RegressionCheckedPatch, Runtime};
pub use vision::{LlamaVisionServerProvider, VisionAnalysis, VisionProvider};
pub use voice::{
    level_description, level_meter, recent_audio_level, speakable_summary, synthesize_speech,
    synthesize_speech_with, transcribe_recording, transcript_into_request, RecordingRetention,
    SpeechSettings, TranscriptRejection, VoiceRecording, VoiceStackAvailability, VoiceStackPaths,
    VoiceTranscript,
};
pub use weather::{OpenMeteoWeatherProvider, WeatherProvider, WeatherSnapshot};
pub use workbench::{
    apply_approved_patch, approve_patch, create_patch_proposal, create_read_only_coding_plan,
    discard_patch_snapshot, generate_unified_diff_for_file, new_cancel_flag,
    restore_patch_snapshot, scope_patch_proposal_to_files, ApprovedPatch, CancelFlag, CodingPlan,
    PatchApplication, PatchProposal, PatchSnapshot, WorkerLimits, WorkerNetwork, WorkerStopReason,
    WorkspaceWriteMode,
};
pub use workflow::{
    describe_workflow, run_workflow, StepOutcomeKind, WorkflowStep, WorkflowStepOutcome,
    WorkflowSummary,
};
pub(crate) use workspace::{
    chunk_workspace_text, configured_retrieval_candidate_multiplier, configured_rrf_k,
    configured_workspace_retrieval_result_limit, extract_pdf_text, fts_query,
    is_secret_like_rejection, reject_oversized_workspace_document,
    reject_secret_like_workspace_document_content, reject_secret_like_workspace_document_name,
    validate_workspace_document_content, validate_workspace_document_path,
};
pub use workspace::{
    preview_workspace_index, RagStatus, RagVerifyReport, WorkspaceCitation,
    WorkspaceFolderIndexReport, WorkspaceIndexPreview, WorkspaceIngestionReport,
    CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION, MAX_WORKSPACE_CHUNK_CHARS,
    MAX_WORKSPACE_DOCUMENT_BYTES, MIN_RELEVANT_SIMILARITY, WORKSPACE_CONTEXT_CHAR_BUDGET,
    WORKSPACE_RETRIEVAL_RESULT_LIMIT,
};

use std::ffi::CString;
use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

// Internal (non-public) items from `model`/`persistence`: these back Runtime's own conversation
// handling and audit-chain tests, and are not part of the crate's public API surface.
#[cfg(test)]
use model::JARVIS_SYSTEM_PROMPT;
use model::{model_capability_intent, UNTRUSTED_MODEL_INTENT_SUPPRESSED};
use persistence::audit_hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    Cli,
    Voice,
    Gui,
    Mobile,
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub schema_version: u16,
    pub request_id: String,
    pub input_type: InputType,
    pub content: String,
    /// Local attachments are immutable references with verified metadata. They are not injected
    /// into text prompts and must be explicitly supported by a future vision/document provider.
    pub attachments: Vec<AttachmentRef>,
}

/// Typed, local MCP ingress contract. Transport/JSON-RPC are deliberately outside this core;
/// this adapter maps only known MCP tool IDs into the same request-policy-task pipeline as CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpIngressRequest {
    pub schema_version: u16,
    pub request_id: String,
    pub tool_id: String,
    pub argument: String,
}

/// F8 "MCP production hardening: protocol sürümleme". Bu build'in anladığı MCP tel-protokol
/// sürümü. Bir MCP isteği bundan farklı bir `schema_version` taşıyorsa, işlenmeden ÖNCE
/// reddedilir (F9'daki veritabanı sürüm güvencesinin dış-protokol karşılığı) — anlaşılmayan bir
/// protokolde bir aracı çalıştırmak, sessizce yanlış yorumlanmış bir isteği yürütmek demek olurdu.
pub const CURRENT_MCP_PROTOCOL_VERSION: u16 = 1;

/// Bir MCP isteğinin protokol sürümünü doğrular. Şu an tek desteklenen sürüm var; ileride birden
/// çok sürüm desteklenirse burası bir aralık/küme kontrolüne dönüşür. `0` (belirtilmemiş) ve
/// gelecekteki sürümler açıkça reddedilir — sessizce varsayılana düşmek yerine.
pub fn validate_mcp_protocol_version(version: u16) -> Result<(), String> {
    if version == CURRENT_MCP_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(format!(
            "desteklenmeyen MCP protokol sürümü {version} — bu build yalnız sürüm {CURRENT_MCP_PROTOCOL_VERSION}'i anlıyor; istek işlenmeden reddedildi"
        ))
    }
}

/// F8 "MCP production hardening: credential/raw-secret response filtresi". Bir MCP aracının
/// yanıtı, DIŞ bir kanala (MCP istemcisine) çıkmadan ÖNCE sır/kimlik-bilgisi benzeri içeriğe karşı
/// taranır. Bu, F3'ün workspace ingestion'da zaten kullandığı aynı yüksek-güven, düşük-yanlış-
/// pozitif imza kümesi (PEM özel-anahtar başlıkları, bilinen token önekleri). Sır benzeri içerik
/// bulunursa yanıt redakte edilir — `Some(redakte_metin)`; temizse `None` (yanıt olduğu gibi
/// gider). Bir sırrı JARVIS'in kendi içinde tutmakla dış bir araca sızdırmak ayrı şeylerdir;
/// bu, ikincisine karşı bir savunma katmanı.
pub fn redact_secret_like_mcp_response(output: &str) -> Option<String> {
    if crate::workspace::reject_secret_like_workspace_document_content(output).is_err() {
        Some(
            "[MCP yanıtı redakte edildi: sır/kimlik-bilgisi benzeri içerik dış kanala aktarılmadı]"
                .to_string(),
        )
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Low,
    Medium,
    High,
    Critical,
}

impl Risk {
    pub fn as_str(&self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
            Risk::Critical => "critical",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "low" => Some(Risk::Low),
            "medium" => Some(Risk::Medium),
            "high" => Some(Risk::High),
            "critical" => Some(Risk::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PentestMode {
    Safe,
    Active,
    Intrusive,
    Destructive,
}

impl PentestMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PentestMode::Safe => "safe",
            PentestMode::Active => "active",
            PentestMode::Intrusive => "intrusive",
            PentestMode::Destructive => "destructive",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "safe" => Some(PentestMode::Safe),
            "active" => Some(PentestMode::Active),
            "intrusive" => Some(PentestMode::Intrusive),
            "destructive" => Some(PentestMode::Destructive),
            _ => None,
        }
    }
}

/// F7.7 "Otonomi modeli netleştirmesi": `PentestMode` "NE yapılabilir" sorusunu cevaplıyor
/// (safe/active/...); bu, ona DİK ikinci eksen — "NE KADAR gözetim gerekir". İkisi birbirinin
/// yerine geçmez: bir eylem hem `Active` (ne) hem `Manual` (her adımda onay iste) olabilir.
/// `Ord` bilinçli: daha yüksek değer = daha az insan gözetimi = daha fazla otomatik yürütme
/// yetkisi, tıpkı `PentestMode`'un daha yüksek = daha invaziv olması gibi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PentestAutonomy {
    /// Her tool çağrısından önce açık insan onayı. En güvenli, en yavaş — varsayılan.
    Manual,
    /// Planı model kurar, kullanıcı onaylar; düşük riskli read-only adımlar (SAFE) otomatik yürür,
    /// hedefe gerçekten dokunan (ACTIVE+) her adım yine onay ister.
    SupervisedAutonomy,
    /// Yazılı scope + süre + bütçe önceden tanımlı; worker yalnız allowlist'teki capability'leri
    /// kullanır, scope dışı/yüksek riskli (INTRUSIVE+) bir adımda otomatik durur. En fazla
    /// otomasyon, yalnız önceden sıkıca sınırlanmış bir kutu içinde.
    BoundedAutonomy,
}

impl PentestAutonomy {
    pub fn as_str(&self) -> &'static str {
        match self {
            PentestAutonomy::Manual => "manual",
            PentestAutonomy::SupervisedAutonomy => "supervised_autonomy",
            PentestAutonomy::BoundedAutonomy => "bounded_autonomy",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "manual" => Some(PentestAutonomy::Manual),
            "supervised_autonomy" => Some(PentestAutonomy::SupervisedAutonomy),
            "bounded_autonomy" => Some(PentestAutonomy::BoundedAutonomy),
            _ => None,
        }
    }

    /// İki eksenin birleşimi: verilen otonomi seviyesinde, verilen `PentestMode`'daki bir adım
    /// insan onayı OLMADAN otomatik yürüyebilir mi? Bu, F7.7'nin "iki eksen birbirinin yerine
    /// geçmez" ilkesinin somut kuralı — otonomi ne kadar yüksek olursa olsun, yeterince invaziv
    /// bir adım her zaman onay ister.
    pub fn allows_unattended(&self, action_mode: PentestMode) -> bool {
        match self {
            // Manual: hiçbir şey otomatik değil, en zararsız read-only adım bile onay ister.
            PentestAutonomy::Manual => false,
            // Supervised: yalnız gerçekten dokunmayan (SAFE) adımlar otomatik; ACTIVE+ onay ister.
            PentestAutonomy::SupervisedAutonomy => action_mode <= PentestMode::Safe,
            // Bounded: önceden sınırlanmış kutu içinde ACTIVE'e kadar otomatik; INTRUSIVE+ durur.
            PentestAutonomy::BoundedAutonomy => action_mode <= PentestMode::Active,
        }
    }
}

/// F7.7 "Görev kontrolü (steering) ve devam ettirme." Kullanıcı çalışan bir güvenlik görevine
/// "yalnızca auth akışına odaklan", "bu endpoint'i kapsam dışına al" veya "dur" diyebilmeli. Bu,
/// o yönlendirmelerin tipli, kalıcılaştırılabilir (dolayısıyla devam ettirilebilir — F3 session/
/// resume desenine bağlanır) hâli. **Karar mantığı burada gerçek ve test edilebilir; onu canlı
/// bir çalışan görev döngüsüne bağlamak (worker'ın her adımdan önce buna danışması) gerçek bir
/// orkestrasyon katmanı geldiğinde yapılacak.** Bir bulgu/kanıt değil, bir görev yönetim durumu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PentestSteering {
    /// `true` ise kullanıcı "dur" dedi — hiçbir yeni adım yürütülmemeli.
    pub stopped: bool,
    /// Boş değilse kullanıcı odağı daralttı ("yalnızca şunlara odaklan") — yalnız bu ön eklerle
    /// başlayan endpoint'ler yürütülebilir. Boşsa odak sınırı yok.
    pub focus_endpoint_prefixes: Vec<String>,
    /// Kullanıcının açıkça kapsam dışına aldığı endpoint ön ekleri — her zaman kazanır (odakta
    /// olsa bile).
    pub excluded_endpoint_prefixes: Vec<String>,
}

impl PentestSteering {
    /// Verilen bir endpoint üzerinde, mevcut yönlendirme altında bir adım yürütülebilir mi?
    /// Sıra önemli: önce "dur", sonra dışlama (her zaman kazanır), sonra odak.
    pub fn permits_endpoint(&self, endpoint: &str) -> bool {
        if self.stopped {
            return false;
        }
        if self
            .excluded_endpoint_prefixes
            .iter()
            .any(|prefix| endpoint.starts_with(prefix.as_str()))
        {
            return false;
        }
        if self.focus_endpoint_prefixes.is_empty() {
            return true;
        }
        self.focus_endpoint_prefixes
            .iter()
            .any(|prefix| endpoint.starts_with(prefix.as_str()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestScope {
    pub schema_version: u16,
    pub authorization_ref: String,
    pub targets: Vec<String>,
    pub excluded_targets: Vec<String>,
    pub expires_at: u64,
    pub maximum_mode: PentestMode,
    pub max_runtime_seconds: u32,
}

/// F7.1 "Çoklu program/scope yönetimi: aktif scope her zaman açıkça gösterilir; bir programın
/// scope'u yüklüyken yanlışlıkla başka bir programın hedefine dokunma riski engellenir."
///
/// A named, persisted scope plus its lifecycle state. `PentestScope` itself stays the immutable
/// authorization contract exactly as it was recorded; revocation is tracked here as a separate
/// event rather than by mutating the scope, the same way the rest of JARVIS treats history as
/// append-only (the audit hash-chain never edits a past entry either).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPentestScope {
    /// The program/engagement label the user chose (e.g. "hackerone-acme-corp"); not part of
    /// the authorization itself, only how it is referred to locally.
    pub name: String,
    pub scope: PentestScope,
    /// At most one stored scope is active at a time — this is the guard against accidentally
    /// probing program B's assets while program A's scope is the one you meant to be using.
    pub is_active: bool,
    pub revoked_at: Option<u64>,
    pub revoked_reason: Option<String>,
    /// F7.1 "İmzalı authorization/scope manifest". An HMAC-SHA256 over the scope's canonical
    /// bytes, keyed by a signing key this machine generated and never exposes (see
    /// `SqliteStore::pentest_signing_key`). This is not proof the bug bounty program authorized
    /// anything — no local system can attest to that — it is proof the scope on disk is exactly
    /// what was written by `save_pentest_scope` on *this* machine and has not been edited since
    /// (by a direct database edit, a restored backup from elsewhere, or any other path that
    /// bypasses the typed contract). `authorize_pentest_action` refuses to authorize anything
    /// against a scope whose signature does not verify.
    pub signature: String,
}

impl StoredPentestScope {
    /// A scope is usable right now only if it is not revoked. Natural expiry is still checked
    /// separately by `validate_pentest_scope`/`authorize_pentest_target` — this only covers the
    /// *explicit* revoke path, which can happen before expiry (a program pulls out, a mistake is
    /// found in the authorization, the user changes their mind).
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// F7.3 "Varlık envanteri kalıcı kaydı" — bir scope altında kaydedilmiş tek bir keşfedilmiş
/// varlık (şimdilik yalnız alt alan adları; F7.3'ün diğer maddeleri, ör. port/servis keşfi,
/// gelecekte aynı tabloya farklı bir `source` değeriyle yazacak, yeni bir tablo icat edilmeyecek).
/// F7.7 "Kapsam matrisi (coverage tuple)": `(hedef, endpoint, parametre, zafiyet_sınıfı)`
/// dörtlüsünün tek bir satırı — hangi kombinasyonun test EDİLDİĞİNİ kaydediyor. Bu, "sıradaki
/// iş" önerisinin temeli: neyin test edilmediği, neyin edildiğini bilmeden görünmez. Bir bulgu
/// (`PentestFinding`) "bir şey BULUNDU" der; bu ise "bir şey KONTROL EDİLDİ (bulunsun ya da
/// bulunmasın)" der — ikisi ayrı, çünkü test edilip temiz çıkan bir kombinasyon da değerli
/// bilgidir (aynı işi iki kez yapmamak için).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestCoverageEntry {
    pub scope_name: String,
    pub target: String,
    pub endpoint: String,
    pub parameter: String,
    pub vulnerability_class: String,
    pub tested_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPentestAsset {
    pub scope_name: String,
    pub asset: String,
    /// Bu varlığın hangi keşif kaynağından geldiği (ör. `"certificate_transparency"`) — birden
    /// çok kaynak aynı varlığı bulursa `source`, en son yazanınkiyle güncellenir (bilinçli:
    /// "hangi kaynaktan geldiği" bilgisi kesin bir tarihçe değil, en güncel gözlemin etiketi).
    pub source: String,
    pub first_seen: u64,
    pub last_seen: u64,
}

/// F7.3 "Aktif keşif: port/servis tarama" bir taramanın sonucu —
/// `Runtime::scan_pentest_ports`. Yalnız hangi portların açık olduğunu, kaç tanesinin
/// gerçekten denendiğini raporluyor — servis parmak izi/banner alma bu maddede yok (o, ayrı bir
/// F7.3 alt maddesi, "teknoloji parmak izi").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestPortScanResult {
    pub target: String,
    pub open_ports: Vec<u16>,
    /// Gerçekten denenmiş port sayısı — `stopped_early_due_to_runtime_budget` doğruysa bu,
    /// çağıranın istediği port listesinin tamamından küçük olabilir.
    pub scanned_port_count: usize,
    /// `true` ise scope'un `max_runtime_seconds` bütçesi tükendiği için tarama, istenen tüm
    /// portlara ulaşmadan durdu — bu, sessizce yetkilendirilen sürenin ötesine geçmek yerine
    /// dürüst bir "buraya kadar yapabildim" raporu.
    pub stopped_early_due_to_runtime_budget: bool,
}

/// F7.3 "Aktif keşif: JS analiziyle endpoint keşfi" bir sorgunun sonucu —
/// `Runtime::discover_pentest_endpoints_via_javascript`. Bulunanlar hostname envanterine
/// (`pentest_assets`) YAZILMIYOR — bir endpoint yolu bir hostname değil, F7.1'in host tabanlı
/// scope eşleştirmesine uymuyor; bu yalnız çağırana dönen, kalıcı olmayan bir sonuç.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestEndpointDiscoveryResult {
    pub target: String,
    /// JS kaynağının indirildiği yol (ör. `/assets/app.js`).
    pub source_path: String,
    /// Bulunan, gerçekçi görünen endpoint yolları — "bulundu" anlamında, "erişilebilir/doğrulandı"
    /// anlamında değil (F7.7'nin kendi ayrımı, burada da geçerli).
    pub endpoints: Vec<String>,
}

/// F7.6 "Bulgu yönetimi": bir bulgunun yaşam döngüsü durumu. F7.7'nin `confirm_finding`
/// sözleşmesiyle aynı ilke — "model şüphelendi" ile "yeniden üretme kanıtıyla doğrulandı" ayrı,
/// karıştırılmayan durumlar olmalı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PentestFindingStatus {
    /// Bir SAFE/ACTIVE kontrolü bunu otomatik olarak gözlemledi — henüz insan onayı veya yeniden
    /// üretme kanıtı YOK. Bir bulgunun varsayılan, ilk durumu.
    Suspected,
    /// `confirm_pentest_finding` ile, hem taze bir yeniden üretme kanıtı hem açık bir insan
    /// onayıyla buraya geçti.
    Confirmed,
    /// İnsan bunun gerçek bir bulgu olmadığına (yanlış pozitif) karar verdi — silinmedi, tarihçe
    /// olarak kalıyor (append-only felsefesi, audit chain'le aynı).
    Rejected,
}

impl PentestFindingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PentestFindingStatus::Suspected => "suspected",
            PentestFindingStatus::Confirmed => "confirmed",
            PentestFindingStatus::Rejected => "rejected",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "suspected" => Some(PentestFindingStatus::Suspected),
            "confirmed" => Some(PentestFindingStatus::Confirmed),
            "rejected" => Some(PentestFindingStatus::Rejected),
            _ => None,
        }
    }
}

/// F7.6 "Evidence tabanlı finding formatı" — bir güvenlik bulgusunun kalıcı, tipli kaydı.
/// `finding_id`, `(scope_name, target, category, title)` dörtlüsünün içerik hash'i — `memory_id`
/// ile aynı desen (`propose_memory_with_trust_and_scope`): AYNI bulgu tekrar kaydedilirse yeni
/// bir satır DEĞİL, aynı satır güncellenir. Bu, F7.6'nın ayrı bir madde olarak istediği
/// "eşleştirme (deduplication)"yi ayrı bir mekanizma icat etmeden, kimliğin kendisinden
/// sağlıyor — gerçekten aynı bulgu yapısal olarak asla iki satıra bölünemez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestFinding {
    pub finding_id: String,
    pub scope_name: String,
    pub target: String,
    /// Serbest biçimli kısa bir kategori etiketi (ör. `"subdomain_takeover"`,
    /// `"exposed_sensitive_file"`, `"idor"`) — F7.5/F7.4'ün kontrol adlarıyla hizalı olması
    /// önerilir ama zorunlu bir enum değil, yeni bulgu türleri kod değişikliği gerektirmesin diye.
    pub category: String,
    /// Yalnız bazı kategoriler için dolu — `category` + `target` tek başına yeterli olmadığında
    /// (ör. `exposed_sensitive_file`'da HANGİ yolun bulunduğu, `/.env` gibi) hassas bir yeniden
    /// doğrulama için gereken somut parametre. F7.6'nın "rapor öncesi yeniden doğrulama"
    /// maddesinin dayanağı — bu olmadan yalnız "bu kategoriden BİR ŞEY hâlâ var mı" gibi
    /// belirsiz bir kontrol yapılabilirdi.
    pub check_parameter: Option<String>,
    pub title: String,
    /// Ham kanıt metni — istek/cevap çiftleri, eşleşen imza, vb. `record_pentest_finding`
    /// bunu kaydetmeden önce bariz sır benzeri içeriğe karşı tarıyor (aynı kontrol, workspace RAG
    /// ingestion'ın zaten kullandığı `reject_secret_like_workspace_document_content`) — bir
    /// bulgunun kanıtı olarak ham bir API anahtarını/token'ı JARVIS'in kendi veritabanına
    /// yazmamak için.
    pub evidence: String,
    pub severity_estimate: Risk,
    pub status: PentestFindingStatus,
    pub recorded_at: u64,
    pub confirmed_at: Option<u64>,
    /// Yalnız `status == Confirmed` ise dolu — doğrulama sırasında toplanan taze kanıt.
    pub confirmation_evidence: Option<String>,
}

/// F7.6 "Rapor öncesi yeniden doğrulama" + "Düzeltme sonrası hedefli yeniden test" —
/// `Runtime::revalidate_pentest_finding`'in döndüğü şey. İki ayrı plan maddesi AYNI mekanizmayı
/// paylaşıyor: bulgu ile rapor yazma arasında hedef değişmiş olabilir (rapor öncesi) VEYA program
/// "düzelttik, doğrular mısın" demiş olabilir (düzeltme sonrası) — her iki durumda da soru aynı:
/// "bu bulgu hâlâ gerçekten orada mı".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PentestFindingRevalidation {
    /// Kontrol tekrar çalıştırıldı ve bulgu hâlâ gözlemleniyor.
    StillPresent,
    /// Kontrol tekrar çalıştırıldı ve bulgu artık gözlemlenmiyor — düzeltilmiş olabilir (ya da
    /// hedef geçici olarak erişilemez olabilir; bu ayrımı yapmak insanın işi, bu yalnız bir
    /// sinyal).
    NoLongerPresent,
    /// Bu kategori için otomatik bir yeniden doğrulama kontrolü henüz yok — elle kontrol gerekir.
    CheckNotSupported,
}

/// F7.6 "Modelin kendisi raporu yazabilmeli, iyi bir şekilde." — bir bulgunun gönderilmeye hazır
/// rapor taslağı. Düzyazı bölümlerin KENDİSİ burada üretilmiyor (bu, model/sohbet zamanında
/// olması gereken bir şey — sabit bir Rust şablonuna hardcode edilmiş metin, gerçek bir rapor
/// yazma yeteneği DEĞİLDİR); bu yapı yalnız modelin doldurması gereken sözleşmeyi tanımlıyor ve
/// `validate_pentest_report_draft_completeness` bunun mekanik olarak kontrol edilebilmesini
/// sağlıyor (F7.6'nın kendi notu: "golden set'e yeni bir zor senaryo olarak eklenebilir").
/// Asla kalıcı olarak saklanmıyor/gönderilmiyor — F4'ün patch akışındaki "önce göster, sonra
/// onay al" deseniyle aynı, kullanıcı gözden geçirmeden JARVIS hiçbir yere göndermez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestFindingReportDraft {
    pub finding_id: String,
    pub summary: String,
    pub reproduction_steps: String,
    pub impact_analysis: String,
    pub suggested_fix: String,
    pub severity_estimate: Risk,
}

/// F7.4 "Manuel test araçları": gönderilecek isteğin tam tanımı — bir kullanıcının/modelin "bu
/// isteği değiştirip tekrar gönder" dediğinde elinde olması gereken her şey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestHttpRequest {
    /// `GET`, `POST`, `PUT`, `DELETE`, ... — büyük/küçük harf duyarlı değil, gönderilirken büyük
    /// harfe çevriliyor.
    pub method: String,
    /// `/` ile başlamalı — tam bir URL değil, yalnız yol + varsa sorgu dizesi.
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// `false` ise `http://`, `true` ise `https://` kullanılır. Gerçek dünyada her pentest
    /// hedefi TLS arkasında olmuyor; bu alan olmadan yalnız HTTPS hedefleri desteklenirdi.
    pub use_tls: bool,
    /// `None` ise şemanın varsayılan portu (`use_tls`'e göre 443/80). Scope yetkilendirmesi
    /// (`authorize_pentest_action`) her zaman ÇIPLAK hostname'e karşı kontrol edilir — port
    /// hedef kimliğinin bir parçası değil (port taramasıyla aynı model: `scan_pentest_ports`
    /// port'u da ayrı bir parametre olarak alıyor) — bu yüzden aynı yetkili host'un standart
    /// olmayan bir portundaki bir servisi test etmek scope'u yeniden yazmayı gerektirmiyor.
    pub port: Option<u16>,
}

/// F7.4: gerçekten alınan cevap — durum kodu, başlıklar, gövde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestHttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// F7.4 "cevapları karşılaştırma" — iki cevap arasındaki fark. Tam bir satır-satır diff motoru
/// DEĞİL (o, ayrı ve daha büyük bir iş); yalnız "ne değişti" sorusuna hızlı, doğru bir ilk cevap:
/// durum kodu değişti mi, hangi başlıklar eklendi/kaldırıldı/değişti, gövde birebir aynı mı ve ne
/// kadar büyüklük farkı var.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestHttpResponseDiff {
    pub status_changed: bool,
    pub old_status: u16,
    pub new_status: u16,
    pub headers_added: Vec<(String, String)>,
    pub headers_removed: Vec<(String, String)>,
    /// `(başlık adı, eski değer, yeni değer)`.
    pub headers_changed: Vec<(String, String, String)>,
    pub body_identical: bool,
    pub old_body_len: usize,
    pub new_body_len: usize,
}

/// F7.3 "Pasif keşif" bir sorgunun sonucu — `Runtime::discover_pentest_assets_via_certificate_transparency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PentestReconResult {
    /// Sorgulanan kök alan adı, tam olarak çağrıldığı gibi (normalize edilmemiş).
    pub queried_domain: String,
    /// Servisin bildirdiği, aktif scope'un `targets`/`excluded_targets`'ına göre süzülmüş,
    /// scope İÇİNDE olan isimler — bunlar kalıcı varlık envanterine kaydedildi.
    pub in_scope_assets: Vec<String>,
    /// `in_scope_assets`'in bir alt kümesi: bu scope için DAHA ÖNCE hiç görülmemiş, bu sorguda
    /// ilk kez ortaya çıkan isimler. Bug bounty'de değerin çoğu buradan gelir (F7.3'ün kendi
    /// gerekçesi) — kullanıcıya "yeni bir şey bulundu" bildirimi tam olarak bu listeden çıkar.
    pub new_assets: Vec<String>,
    /// Servisin bildirdiği ama aktif scope'un dışında kalan isim sayısı — hangi isimler olduğu
    /// KAYDEDİLMİYOR (kapsam dışı bir hedefi envantere yazıp yanlışlıkla test edilebilir gibi
    /// göstermemek için), yalnız "bu kadarı elendi" şeffaflığı için sayı tutuluyor.
    pub out_of_scope_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny,
    AskUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyControl {
    UserApproval,
    ExplainBeforeExecute,
    VerifierRequired,
    AuditRequired,
    ReadOnlyFilesystem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyResult {
    pub decision: PolicyDecision,
    pub risk: Risk,
    pub reason: String,
    pub approval_required: bool,
    pub required_controls: Vec<PolicyControl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    WaitingForUser,
    Cancelled,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub task_id: String,
    pub request_id: String,
    pub state: TaskState,
    pub capability: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Success,
    PartialSuccess,
    Failure,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub status: ToolStatus,
    pub output: String,
    pub error: Option<String>,
    pub state_changed: bool,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStatus {
    Pass,
    Fail,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifierResult {
    pub status: VerifyStatus,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub task_id: String,
    pub event: String,
    pub sequence: u64,
    pub previous_hash: String,
    pub event_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredLogEvent {
    pub timestamp: u64,
    pub level: LogLevel,
    pub correlation_id: String,
    pub task_id: String,
    pub event: String,
}

/// F9 "Metrikler ... kişisel içerik toplamadan yerel telemetry". Bir oturumun operasyonel özeti —
/// YALNIZ sayımlar ve capability/olay adları; hiçbir kullanıcı metni, hedef, sır ya da içerik YOK.
/// `Runtime::metrics_summary` bunu bellekteki görev/audit verisinden türetiyor; hiçbir yere
/// gönderilmiyor, yalnız kullanıcının kendi sistemini gözlemlemesi için.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeMetricsSummary {
    pub total_tasks: usize,
    /// Capability adı → o capability'yle kaç görev. Yalnız capability KİMLİKLERİ (ör.
    /// `system.health`), kullanıcı girdisi değil.
    pub tasks_by_capability: std::collections::BTreeMap<String, usize>,
    pub verification_pass: usize,
    pub verification_fail: usize,
    pub policy_allow: usize,
    pub policy_deny: usize,
    pub policy_ask_user: usize,
    /// `LogLevel::Warn` seviyesindeki olay sayısı (başarısız/geçersiz olaylar).
    pub warning_events: usize,
    /// Toplam kayıtlı structured-log olayı — "ne kadar aktivite oldu"nun kaba ölçüsü.
    pub total_events: usize,
    /// F9 "Metrikler: latency" — işlenmiş bir isteğin ortalama süresi (ms). Görev yoksa 0.
    /// Yalnız bir süre ölçüsü; hiçbir içerik taşımaz.
    pub average_task_latency_millis: u64,
}

/// A dialogue turn passed to a model as data. Roles describe attribution only; they grant no
/// policy, tool, or system authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentProvenance {
    TrustedUser,
    UntrustedProjectFile,
    UntrustedWeb,
    ToolOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentRef {
    pub source: String,
    pub provenance: ContentProvenance,
    pub content: String,
}

/// Renders retrieved content as data, never as an instruction. Model/policy prompts can include
/// this representation without granting any authority to the source document.
pub fn isolate_untrusted_content(content: &ContentRef) -> String {
    format!(
        "<untrusted-content source=\"{}\" provenance=\"{:?}\">\n{}\n</untrusted-content>",
        content.source, content.provenance, content.content
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeacherEscalationDecision {
    LocalOnly,
    ApprovalRequired,
}

pub fn assess_teacher_escalation(private_context: bool) -> TeacherEscalationDecision {
    if private_context {
        TeacherEscalationDecision::ApprovalRequired
    } else {
        TeacherEscalationDecision::LocalOnly
    }
}

impl AuditEvent {
    fn pending(task_id: impl Into<String>, event: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            event: event.into(),
            sequence: 0,
            previous_hash: String::new(),
            event_hash: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub approval_id: String,
    pub task_id: String,
    pub action_id: String,
    pub approved: bool,
    pub expires_at: u64,
    pub scope_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataSensitivity {
    Public,
    Internal,
    Sensitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryNamespace {
    /// Durable, user-approved profile facts (name, address form, language, role preference).
    UserProfile,
    /// Durable facts scoped to a project the user is working on.
    Project,
    /// Durable facts scoped to a specific task.
    Task,
    /// Short-lived notes scoped to the current conversation session. Physically distinct from
    /// the three durable namespaces above: `validate_memory_record` refuses to store a `Session`
    /// record without an expiry, so it cannot silently become permanent.
    Session,
    /// Short-lived cache of a tool's own output (for example, a computed result worth reusing
    /// for a few follow-up turns). Same expiry requirement as `Session`, for the same reason.
    EphemeralToolOutput,
}

impl MemoryNamespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserProfile => "USER_PROFILE",
            Self::Project => "PROJECT",
            Self::Task => "TASK",
            Self::Session => "SESSION",
            Self::EphemeralToolOutput => "EPHEMERAL_TOOL_OUTPUT",
        }
    }

    /// True for the two namespaces that must never persist indefinitely. `propose_memory` and
    /// `validate_memory_record` both enforce this — it is a physical constraint on the record
    /// itself (an expiry is mandatory), not just a naming convention.
    pub fn requires_expiry(self) -> bool {
        matches!(self, Self::Session | Self::EphemeralToolOutput)
    }

    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "USER_PROFILE" => Ok(Self::UserProfile),
            "PROJECT" => Ok(Self::Project),
            "TASK" => Ok(Self::Task),
            "SESSION" => Ok(Self::Session),
            "EPHEMERAL_TOOL_OUTPUT" => Ok(Self::EphemeralToolOutput),
            _ => Err(format!("unknown memory namespace: {value}")),
        }
    }
}

/// Parses a user-typed namespace word (English or Turkish), case-insensitive, for commands like
/// `/forget namespace <...>`. Same dual ASCII/Turkish-fold approach as `parse_data_sensitivity`
/// and `ProfileField::from_user_input`, for the same "INTERNAL" vs "ınternal" reason.
pub fn parse_memory_namespace(input: &str) -> Option<MemoryNamespace> {
    let trimmed = input.trim();
    for candidate in [trimmed.to_lowercase(), turkish_case_fold(trimmed)] {
        let namespace = match candidate.as_str() {
            "user_profile" | "profile" | "profil" => Some(MemoryNamespace::UserProfile),
            "project" | "proje" => Some(MemoryNamespace::Project),
            "task" | "görev" | "gorev" => Some(MemoryNamespace::Task),
            "session" | "oturum" => Some(MemoryNamespace::Session),
            "ephemeral_tool_output" | "ephemeral" | "geçici" | "gecici" => {
                Some(MemoryNamespace::EphemeralToolOutput)
            }
            _ => None,
        };
        if namespace.is_some() {
            return namespace;
        }
    }
    None
}

/// Kullanıcının katmanlı bellek tasarımının "her kayıtta mümkünse provenance/trust level/scope/
/// sensitivity metadata'sı olsun" kuralının karşılığı. `source` zaten provenance'ı taşıyordu;
/// `trust_level` bu kuralın eksik parçasıydı — bugüne kadar tek bir güven seviyesi vardı (açık
/// kullanıcı komutu), bu yüzden görünmüyordu. Şimdilik yalnız gerçekten ayırt edilebilir iki
/// seviye var; yeni bir seviye (örn. bir belgeden çıkarılmış ama henüz onaylanmamış bir öneri)
/// gerçek bir üretici olmadan eklenmedi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Kullanıcı bu değeri doğrudan bu oturumda yazdı/onayladı (`/remember`, doğal dil bellek
    /// komutları, `/profile set`). Bu alan eklenmeden önce var olan **tek** seviyeydi; her iki
    /// doğrudan yazma yolu da hâlâ bunu üretiyor.
    UserAsserted,
    /// `/memory import` ile geldi — yine yalnız açık bir kullanıcı komutuyla erişilebilir (import
    /// asla otomatik değildir), ama kaydın nihai kökeni (kullanıcının sağladığı bir JSON dosyası)
    /// "kullanıcı bu değeri az önce JARVIS'e yazdı"dan bir adım uzakta.
    Imported,
}

impl TrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserAsserted => "USER_ASSERTED",
            Self::Imported => "IMPORTED",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "IMPORTED" => Self::Imported,
            // Bilinmeyen/eski (bu alan eklenmeden önce yazılmış) satırlar orijinal (tek) seviyeye
            // düşer — bu, o satırların gerçek kökenini yanlış yansıtmaz, çünkü hepsi zaten
            // `UserAsserted` idi.
            _ => Self::UserAsserted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    pub schema_version: u16,
    pub memory_id: String,
    pub namespace: MemoryNamespace,
    pub key: String,
    pub value: String,
    pub sensitivity: DataSensitivity,
    pub source: String,
    pub include_in_model_context: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub expires_at: Option<u64>,
    pub trust_level: TrustLevel,
    /// Kullanıcının "concurrent task'lar birbirinin context'ini kirletmesin" kuralının karşılığı.
    /// Yalnız `MemoryNamespace::Task` için anlamlıdır — o zaman bu, kaydın ait olduğu tam task_id.
    /// `None` (diğer tüm namespace'ler için, ya da eski satırlar için) "bu kayıt belirli bir
    /// task'a taahhüt edilmemiş" demektir. Bkz. `Runtime::task_scoped_memory_context` — normal
    /// sohbet bağlamı artık `Task` namespace'ini hiç çekmiyor, yalnız bu fonksiyon, açıkça bir
    /// `task_id` verilerek, yalnız o task'ın kayıtlarını döner.
    pub scope_id: Option<String>,
}

/// A pending memory write. Creating one has no persistent side effect; it exists so the UI can
/// show exactly what will be saved before the user accepts or rejects it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProposal {
    pub proposal_id: String,
    pub record: MemoryRecord,
}

pub fn propose_memory(
    namespace: MemoryNamespace,
    key: impl Into<String>,
    value: impl Into<String>,
    sensitivity: DataSensitivity,
    source: impl Into<String>,
    include_in_model_context: bool,
    expires_at: Option<u64>,
) -> Result<MemoryProposal, String> {
    propose_memory_with_trust_and_scope(
        namespace,
        key,
        value,
        sensitivity,
        source,
        include_in_model_context,
        expires_at,
        TrustLevel::UserAsserted,
        None,
    )
}

/// Same as `propose_memory`, with an explicit `trust_level` and (for `MemoryNamespace::Task`
/// records) a `scope_id` — the task_id this record is committed to. `propose_memory` itself stays
/// untouched (still every existing call site's exact signature) and simply calls this with
/// `(TrustLevel::UserAsserted, None)`, its own original, only-ever-existing behavior.
#[allow(clippy::too_many_arguments)]
pub fn propose_memory_with_trust_and_scope(
    namespace: MemoryNamespace,
    key: impl Into<String>,
    value: impl Into<String>,
    sensitivity: DataSensitivity,
    source: impl Into<String>,
    include_in_model_context: bool,
    expires_at: Option<u64>,
    trust_level: TrustLevel,
    scope_id: Option<String>,
) -> Result<MemoryProposal, String> {
    let key = key.into();
    let value = value.into();
    let source = source.into();
    let created_at = now_epoch();
    // `memory_id` is the record's stable identity: `(namespace, key)` normally — never the
    // value, source or a timestamp. A `scope_id` (Task-namespace records) is folded in too, so
    // two different tasks writing the same key never collide into one row — this is the whole
    // point of task-scoped memory; without it, task B's "karar" would silently overwrite task
    // A's. When `scope_id` is `None` the formula is byte-for-byte the original one, so every
    // record written before this field existed keeps the exact same `memory_id` it always had.
    let memory_identity = match &scope_id {
        Some(scope) => format!("memory-v1|{}|{}|{}", namespace.as_str(), scope, key),
        None => format!("memory-v1|{}|{}", namespace.as_str(), key),
    };
    let memory_id = format!("memory-{}", &sha256_hex(&memory_identity)[..16]);
    // `proposal_id` identifies *this specific proposed write* (so a UI's "is my pending proposal
    // still the one I showed the user" check never collides across two different proposals for
    // the same key) — it still varies by value/source/time, unlike `memory_id` above.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let proposal_identity = format!("memory-proposal-v1|{memory_id}|{value}|{source}|{nonce}");
    let proposal_id = format!("memory-proposal-{}", &sha256_hex(&proposal_identity)[..16]);
    let record = MemoryRecord {
        schema_version: 1,
        memory_id,
        namespace,
        key,
        value,
        sensitivity,
        source,
        include_in_model_context,
        created_at,
        updated_at: created_at,
        expires_at,
        trust_level,
        scope_id,
    };
    validate_memory_record(&record)?;
    Ok(MemoryProposal {
        proposal_id,
        record,
    })
}

pub fn validate_memory_record(record: &MemoryRecord) -> Result<(), String> {
    if record.schema_version != 1 {
        return Err(format!(
            "unsupported memory schema version: {}",
            record.schema_version
        ));
    }
    if record.memory_id.trim().is_empty()
        || record.key.trim().is_empty()
        || record.value.trim().is_empty()
        || record.source.trim().is_empty()
    {
        return Err("memory requires id, key, value and source".into());
    }
    if record.key.chars().count() > 120 || record.value.chars().count() > 4_000 {
        return Err("memory key or value exceeds its safety limit".into());
    }
    match record.expires_at {
        Some(expires_at) if expires_at <= record.created_at => {
            return Err("memory expiry must be after its creation time".into());
        }
        None if record.namespace.requires_expiry() => {
            return Err(format!(
                "{} memory requires an expiry; it must never persist indefinitely",
                record.namespace.as_str()
            ));
        }
        _ => {}
    }
    match &record.scope_id {
        Some(scope_id) => {
            if scope_id.trim().is_empty() || scope_id.chars().count() > 120 {
                return Err("memory scope_id must be non-empty and within its safety limit".into());
            }
            if record.namespace != MemoryNamespace::Task {
                return Err(format!(
                    "scope_id is only meaningful for {} memory, not {}",
                    MemoryNamespace::Task.as_str(),
                    record.namespace.as_str()
                ));
            }
        }
        // Mirrors `requires_expiry()`'s structural constraint (Session/EphemeralToolOutput):
        // a `Task` record with no `scope_id` would be an orphan — nothing to isolate it from
        // other tasks, defeating the entire point of the namespace. Not just a convention, this
        // is enforced the same way an unbounded Session record is refused.
        None if record.namespace == MemoryNamespace::Task => {
            return Err("Task memory requires a scope_id (the task_id it belongs to)".into());
        }
        None => {}
    }
    Ok(())
}

/// Renders approved memory as data only. It deliberately never turns user profile fields into
/// system instructions or capability grants.
pub fn isolate_memory_as_data(record: &MemoryRecord) -> String {
    format!(
        "<memory-data id=\"{}\" namespace=\"{}\" key=\"{}\" sensitivity=\"{}\" source=\"{}\">\n{}\n</memory-data>",
        record.memory_id,
        record.namespace.as_str(),
        record.key,
        record.sensitivity.as_str(),
        record.source,
        record.value
    )
}

/// TUI usability fix (2026-08-16): builds the "Kaynaklar" text appended after a reply, shared by
/// both the TUI (`main.rs`) and the desktop client (`bin/jarvis_desktop.rs`) so the two never
/// drift apart. Workspace-citation/vision lines (genuinely query-matched, i.e. actually relevant
/// to what was asked) are still listed in full. Memory-attribution lines (`"• Kayıtlı bilgi
/// kullanıldı: ..."`, one per record in `Runtime::approved_memory_context`'s always-on personal-
/// ization context — retrieved on *every* turn regardless of topic, by design, so the model always
/// knows the user's name/preferences even for small talk) are collapsed into one short count
/// instead of listed individually: a trivial reply like "evet, uyanığım" was showing a multi-line
/// source dump of unrelated profile/project records under it — this keeps the same transparency
/// (the count is still visible, nothing is hidden) without the clutter. Returns `None` when there
/// is nothing to show at all.
pub fn format_sources_block(sources: &[String]) -> Option<String> {
    const MEMORY_PREFIX: &str = "• Kayıtlı bilgi kullanıldı:";
    let (memory_lines, other_lines): (Vec<&String>, Vec<&String>) = sources
        .iter()
        .partition(|line| line.starts_with(MEMORY_PREFIX));
    let mut block = String::new();
    if !other_lines.is_empty() {
        block.push_str("\n\nKaynaklar:\n");
        block.push_str(
            &other_lines
                .iter()
                .map(|line| line.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !memory_lines.is_empty() {
        block.push_str(&format!(
            "\n\n(bu yanıtta {} kayıtlı bilgi bağlam olarak kullanıldı)",
            memory_lines.len()
        ));
    }
    (!block.is_empty()).then_some(block)
}

/// Portable JSON backup of every memory record (any namespace), for the F3 "Memory
/// migration/backup ... export/import" requirement. Excludes `memory_id` and `source`: import
/// always mints a fresh id (matching how `propose_memory` already works — nothing round-trips a
/// stable external id) and re-attributes `source` to the import action itself, so the origin of a
/// re-imported record is never confused with wherever it first came from.
pub fn memory_export(records: &[MemoryRecord]) -> Result<String, String> {
    let entries = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "namespace": record.namespace.as_str(),
                "key": record.key,
                "value": record.value,
                "sensitivity": record.sensitivity.as_str(),
                "include_in_model_context": record.include_in_model_context,
                "expires_at": record.expires_at,
                "scope_id": record.scope_id,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": 1,
        "kind": "jarvis-memory-export",
        "entries": entries,
    }))
    .map(|serialized| format!("{serialized}\n"))
    .map_err(|error| format!("memory export serialization failed: {error}"))
}

/// Parses a `memory_export` document back into proposals — never commits anything directly.
/// Import is only ever reachable through an explicit user command (`/memory import <path>`); the
/// caller must still run each proposal through the same approval path as any other memory write
/// (this project has exactly one way to persist memory, and import does not get a second one).
/// A malformed entry is skipped with its reason collected rather than aborting the whole import,
/// so one bad row in a hand-edited export file doesn't block the rest.
pub fn memory_import(
    source: impl Into<String>,
    json: &str,
) -> Result<(Vec<MemoryProposal>, Vec<String>), String> {
    let source = source.into();
    let document: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| format!("invalid memory export JSON: {error}"))?;
    let entries = document
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "memory export JSON has no \"entries\" array".to_string())?;
    let mut proposals = Vec::new();
    let mut skipped = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let parsed = (|| -> Result<MemoryProposal, String> {
            let namespace_word = entry
                .get("namespace")
                .and_then(serde_json::Value::as_str)
                .ok_or("missing namespace")?;
            let namespace = MemoryNamespace::from_str(namespace_word)?;
            let key = entry
                .get("key")
                .and_then(serde_json::Value::as_str)
                .ok_or("missing key")?;
            let value = entry
                .get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or("missing value")?;
            let sensitivity_word = entry
                .get("sensitivity")
                .and_then(serde_json::Value::as_str)
                .ok_or("missing sensitivity")?;
            let sensitivity = DataSensitivity::from_str(sensitivity_word)?;
            let include_in_model_context = entry
                .get("include_in_model_context")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let expires_at = entry.get("expires_at").and_then(serde_json::Value::as_u64);
            // `Task` alanı için isteğe bağlı `scope_id`: önceden dışa aktarılmış bir Task kaydı
            // geri içe aktarılırken hangi task'a ait olduğunu korur. Eksikse ve namespace `Task`
            // ise `validate_memory_record` zaten reddedecek (task_id'siz bir Task kaydı yapısal
            // olarak geçersiz) — bu, o girdiyi sessizce yok saymak yerine `skipped` listesine
            // düşürür.
            let scope_id = entry
                .get("scope_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            // Kullanıcının "her kayıtta mümkünse trust level" kuralı: import her zaman
            // `Imported` — kaydın nihai kökeni kullanıcının sağladığı bir JSON dosyası, "kullanıcı
            // bunu az önce JARVIS'e yazdı"dan bir adım uzakta.
            propose_memory_with_trust_and_scope(
                namespace,
                key,
                value,
                sensitivity,
                source.clone(),
                include_in_model_context,
                expires_at,
                TrustLevel::Imported,
                scope_id,
            )
        })();
        match parsed {
            Ok(proposal) => proposals.push(proposal),
            Err(error) => skipped.push(format!("entries[{index}]: {error}")),
        }
    }
    Ok((proposals, skipped))
}

impl DataSensitivity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "PUBLIC",
            Self::Internal => "INTERNAL",
            Self::Sensitive => "SENSITIVE",
        }
    }

    pub(crate) fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "PUBLIC" => Ok(Self::Public),
            "INTERNAL" => Ok(Self::Internal),
            "SENSITIVE" => Ok(Self::Sensitive),
            _ => Err(format!("unknown data sensitivity: {value}")),
        }
    }
}

/// Parses a user-typed sensitivity word (English or the Turkish word a `/remember`-style command
/// accepts), case-insensitive. Used by the memory write UX so the user actually chooses
/// sensitivity instead of every record silently defaulting to one fixed value.
///
/// This mixes English and Turkish vocabulary in one input, so a single case-folding rule cannot
/// be correct for both: `turkish_case_fold("INTERNAL")` folds the bare `I` to Turkish dotless
/// `ı` (correct for genuine Turkish text), which then fails to match the English word "internal".
/// Trying plain ASCII-lowercase first (correct for English) and falling back to
/// `turkish_case_fold` (correct for Turkish) covers both without guessing the input's language.
pub fn parse_data_sensitivity(input: &str) -> Option<DataSensitivity> {
    let trimmed = input.trim();
    for candidate in [trimmed.to_lowercase(), turkish_case_fold(trimmed)] {
        let sensitivity = match candidate.as_str() {
            "public" | "genel" | "herkese açık" => Some(DataSensitivity::Public),
            "internal" | "dahili" => Some(DataSensitivity::Internal),
            "sensitive" | "hassas" => Some(DataSensitivity::Sensitive),
            _ => None,
        };
        if sensitivity.is_some() {
            return sensitivity;
        }
    }
    None
}

/// A candidate training example. It is intentionally a data-governance record, not a model
/// instruction: only a human-reviewed, verifier-passing example for a registered capability can
/// be persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeacherExample {
    pub schema_version: u16,
    pub example_id: String,
    pub prompt: String,
    pub expected_capability: String,
    pub response: String,
    pub evidence: Vec<String>,
    pub verifier_status: VerifyStatus,
    pub provenance: String,
    pub human_reviewed: bool,
    pub sensitivity: DataSensitivity,
}

/// F6 "Kullanıcı geri bildirimi intake'i". A raw signal from the user about one conversation
/// turn. Deliberately a *separate* type from `TeacherExample`: the plan's rule is that feedback
/// "doğrudan eğitim verisi olmaz" — it must pass sensitivity, provenance and human review first.
/// Modelling it as its own pending record is what makes that rule structural instead of a
/// convention someone can forget: there is no code path that turns a signal into training data
/// without an explicit review step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackCandidate {
    pub schema_version: u16,
    pub candidate_id: String,
    pub recorded_at: u64,
    pub prompt: String,
    pub response: String,
    pub signal: FeedbackSignal,
    /// For `Correction`, what the user says the answer should have been. Empty otherwise.
    pub correction: String,
    pub sensitivity: DataSensitivity,
    pub provenance: String,
    /// Set once a human has decided; an unreviewed candidate can never be exported or promoted.
    pub review: FeedbackReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackSignal {
    Positive,
    Negative,
    Correction,
}

impl FeedbackSignal {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackSignal::Positive => "positive",
            FeedbackSignal::Negative => "negative",
            FeedbackSignal::Correction => "correction",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "positive" => Some(FeedbackSignal::Positive),
            "negative" => Some(FeedbackSignal::Negative),
            "correction" => Some(FeedbackSignal::Correction),
            _ => None,
        }
    }
}

/// Where a candidate stands in the human review queue. `Rejected` is kept rather than deleted so
/// a bad or poisoned example stays *known-bad* — deleting it would let the same content be
/// re-submitted later and silently re-enter the queue as if it were new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackReview {
    Pending,
    Approved,
    Rejected,
}

impl FeedbackReview {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackReview::Pending => "pending",
            FeedbackReview::Approved => "approved",
            FeedbackReview::Rejected => "rejected",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(FeedbackReview::Pending),
            "approved" => Some(FeedbackReview::Approved),
            "rejected" => Some(FeedbackReview::Rejected),
            _ => None,
        }
    }
}

/// F6 "Prompt/model konfigürasyon registry'si": one recorded experiment — which model and which
/// system prompt produced which benchmark result, and what to roll back to if it turns out worse.
///
/// This is deliberately a *record*, not a switch: writing a row never changes which model or
/// prompt the running system uses. It exists so a model or prompt change is never an unmeasured,
/// unattributable, irreversible event — F6's completion criterion is exactly that every model
/// change either improves the versioned eval or is not adopted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfigRun {
    pub schema_version: u16,
    pub run_id: String,
    pub recorded_at: u64,
    pub provider_id: String,
    pub model_id: String,
    /// Identifies the exact weights: SHA-256 of the GGUF when known, otherwise a provider-
    /// reported identifier. Two runs with different fingerprints are never comparable as
    /// "same model".
    pub model_fingerprint: String,
    /// SHA-256 of the exact system prompt text used, so a prompt edit is detectable even when
    /// no commit or model changed.
    pub prompt_fingerprint: String,
    /// Free-form runtime settings that materially affect the result (`-ngl 28`, context size).
    pub server_settings: String,
    pub scenarios_passed: u32,
    pub scenarios_failed: u32,
    pub median_latency_ms: u64,
    pub notes: String,
    /// `run_id` of the configuration to return to if this one regresses. `None` for a baseline
    /// that has nothing earlier to fall back to.
    pub rollback_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityManifest {
    pub capability_id: String,
    pub version: String,
    pub risk: Risk,
    pub effect_scope: String,
    pub requires_network: bool,
    pub sandbox_profile: String,
    pub verifier_profile: String,
}

// Chat-history trimming threshold for Runtime's conversation window. Model provider
// adapters live in `model`; this constant is a Runtime-only concern and stays here.
// Four completed exchanges preserve short follow-ups while preventing an old topic from
// dominating a new request or adding avoidable CPU prompt-processing latency.
const MAX_COMPLETED_CHAT_HISTORY_TURNS: usize = 8;

/// Current unix epoch seconds. Public because clients (TUI/desktop) construct timestamped
/// records — F6 feedback candidates, for one — and must use the same clock the core does.
pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// F9 "Sürüm/migration yönetimi: semantic version". Tek kaynak: `Cargo.toml`'un `version`'ı
/// (derleme zamanında gömülür) — elle senkron tutulacak ikinci bir sabit yok. CHANGELOG.md bu
/// sürümün insan-okunur değişiklik kaydını tutuyor.
pub const JARVIS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// F9 "Metrikler": bir metrik özetini kullanıcıya gösterilecek okunabilir Türkçe metne çevirir.
/// Yalnız sayımlar ve capability/olay adları — özetin kendisi zaten gizlilik-güvenli.
pub fn format_metrics_summary(metrics: &RuntimeMetricsSummary) -> String {
    let mut lines = vec![
        format!("Toplam görev: {}", metrics.total_tasks),
        format!(
            "Doğrulama: {} geçti / {} kaldı",
            metrics.verification_pass, metrics.verification_fail
        ),
        format!(
            "Policy: {} izin / {} ret / {} onay-bekle",
            metrics.policy_allow, metrics.policy_deny, metrics.policy_ask_user
        ),
        format!("Uyarı olayı: {}", metrics.warning_events),
        format!("Toplam olay: {}", metrics.total_events),
        format!(
            "Ortalama işlem süresi: {} ms",
            metrics.average_task_latency_millis
        ),
    ];
    if !metrics.tasks_by_capability.is_empty() {
        lines.push("Capability dağılımı:".to_string());
        for (capability, count) in &metrics.tasks_by_capability {
            lines.push(format!("  • {capability}: {count}"));
        }
    }
    lines.join("\n")
}

fn approval_scope_hash(task_id: &str, action_id: &str, input: &str) -> String {
    // Versioned deterministic scope binding. Cryptographic canonical hashing is a later contract upgrade.
    let mut hash: u64 = 14695981039346656037;
    for byte in format!("scope-v1|{}|{}|{}", task_id, action_id, input).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("scope-v1-fnv1a-{hash:016x}")
}

pub(crate) fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Türkçe-doğru küçük harfe çevirme. Rust'ın standart `str::to_lowercase()`'i Unicode
/// case-folding kurallarını kullanır: 'İ' (noktalı büyük I) 'i' değil, 'i' + birleşik nokta
/// işaretine ("i̇") döner ve 'I' (noktasız büyük I) 'ı' yerine düz 'i'ye döner. Bu, kullanıcının
/// yazdığı Türkçe metni karşılaştırırken sessizce yanlış eşleşmeye (ya da profil alanı gibi
/// durumlarda hiç eşleşmemeye) yol açar. Bu fonksiyon 'I'/'İ'yi elle doğru harfe eşler, geri kalan
/// her karakter için standart `to_lowercase()`'e güvenir. Native masaüstündeki mesaj arama ve
/// `profile` modülündeki alan adı çözümleme aynı bu fonksiyonu kullanır — iki ayrı yerde iki farklı
/// (ve tutarsız) Türkçe katlama mantığı olmasın diye.
pub fn turkish_case_fold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            'I' => folded.push('ı'),
            'İ' => folded.push('i'),
            _ => folded.extend(character.to_lowercase()),
        }
    }
    folded
}

impl TaskState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::WaitingForUser => "WAITING_FOR_USER",
            Self::Cancelled => "CANCELLED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Interrupted => "INTERRUPTED",
        }
    }
}

fn sandbox_violation(reason: &str) -> ToolResult {
    ToolResult {
        status: ToolStatus::Failure,
        output: String::new(),
        error: Some(format!("sandbox profile violation: {reason}")),
        state_changed: false,
        evidence: vec![],
    }
}

/// Enforces the baseline's read-only runtime profile before dispatch. This is a capability
/// boundary in the in-process MVP; an OS-isolated worker is a separate, future layer.
fn execute_read_only(manifest: &CapabilityManifest, input: &str) -> ToolResult {
    if manifest.sandbox_profile != "NO_EXEC_READ_ONLY" {
        return sandbox_violation("read-only capability requires NO_EXEC_READ_ONLY profile");
    }
    match manifest.capability_id.as_str() {
        "conversation.reply" => ToolResult {
            status: ToolStatus::Success,
            output: "Local sohbet modeli şu anda yanıt veremedi.".into(),
            error: None,
            state_changed: false,
            evidence: vec!["conversation.reply:local".into()],
        },
        "system.health" => ToolResult {
            status: ToolStatus::Success,
            output: system_health_snapshot(),
            error: None,
            state_changed: false,
            evidence: vec!["health-check:ok".into()],
        },
        "system.time" => ToolResult {
            status: ToolStatus::Success,
            output: now_epoch().to_string(),
            error: None,
            state_changed: false,
            evidence: vec!["timestamp:present".into()],
        },
        "file.read_workspace" => read_workspace_file(input),
        "project.info" => project_info(),
        "code.project_outline" => project_outline(),
        "docs.workspace_summary" => read_workspace_file("dosya oku: README.md"),
        _ => sandbox_violation("capability is not executable in read-only profile"),
    }
}

/// Collects a bounded, read-only local health snapshot. It intentionally avoids shell commands,
/// network access outside loopback and any user-file reads; capability policy still treats the
/// result as a low-risk governed observation.
fn system_health_snapshot() -> String {
    let cpu_threads = std::thread::available_parallelism()
        .map(|value| value.get().to_string())
        .unwrap_or_else(|_| "bilinmiyor".into());
    let cpu_usage = cpu_usage_percent()
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "bilinmiyor".into());
    let cpu_temperature = thermal_temperature()
        .map(|value| format!("{value:.1} °C"))
        .unwrap_or_else(|| "bilinmiyor".into());
    let load = fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|value| value.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "bilinmiyor".into());
    let (memory_total, memory_available) = proc_memory_snapshot();
    let memory_used = memory_total
        .and_then(|total| memory_available.map(|available| total.saturating_sub(available)));
    let (disk_used, disk_total) = disk_snapshot(".");
    let network = network_snapshot();
    let gpu = gpu_snapshot();
    let fans = fan_snapshot();
    let text_model = loopback_service_status(8088);
    let vision_model = loopback_service_status(8089);
    format!(
        "JARVIS core sağlıklı\nCPU kullanım: {cpu_usage} • sıcaklık: {cpu_temperature} • iş parçacığı: {cpu_threads}\nSistem yükü (1 dk): {load}\nRAM: {} kullanılan / {} toplam\nDisk (.): {} kullanılan / {} toplam\nGPU: {gpu}\nFan/RPM: {fans}\nAğ (loopback hariç, sayaç): alınan {} • gönderilen {}\nText model loopback: {text_model}\nVision model loopback: {vision_model}",
        memory_used.map(format_bytes).unwrap_or_else(|| "bilinmiyor".into()),
        memory_total.map(format_bytes).unwrap_or_else(|| "bilinmiyor".into()),
        disk_used.map(format_bytes).unwrap_or_else(|| "bilinmiyor".into()),
        disk_total.map(format_bytes).unwrap_or_else(|| "bilinmiyor".into()),
        network.0,
        network.1,
    )
}

fn proc_memory_snapshot() -> (Option<u64>, Option<u64>) {
    let mut total_kib: Option<u64> = None;
    let mut available_kib: Option<u64> = None;
    if let Ok(contents) = fs::read_to_string("/proc/meminfo") {
        for line in contents.lines() {
            let mut fields = line.split_whitespace();
            match fields.next() {
                Some("MemTotal:") => total_kib = fields.next().and_then(|value| value.parse().ok()),
                Some("MemAvailable:") => {
                    available_kib = fields.next().and_then(|value| value.parse().ok())
                }
                _ => {}
            }
        }
    }
    (
        total_kib.map(|value| value * 1024),
        available_kib.map(|value| value * 1024),
    )
}

fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut scaled = value as f64;
    let mut unit = 0;
    while scaled >= 1024.0 && unit < UNITS.len() - 1 {
        scaled /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value, UNITS[unit])
    } else {
        format!("{scaled:.1} {}", UNITS[unit])
    }
}

fn cpu_totals() -> Option<(u64, u64)> {
    let contents = fs::read_to_string("/proc/stat").ok()?;
    let line = contents.lines().find(|line| line.starts_with("cpu "))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(|value| value.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let total = values.iter().sum();
    let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

fn cpu_usage_percent() -> Option<f64> {
    let before = cpu_totals()?;
    std::thread::sleep(Duration::from_millis(100));
    let after = cpu_totals()?;
    let total_delta = after.0.saturating_sub(before.0);
    let idle_delta = after.1.saturating_sub(before.1);
    (total_delta > 0)
        .then(|| (total_delta.saturating_sub(idle_delta)) as f64 * 100.0 / total_delta as f64)
}

fn thermal_temperature() -> Option<f64> {
    let mut maximum = None;
    for entry in fs::read_dir("/sys/class/thermal").ok()?.flatten() {
        let path = entry.path().join("temp");
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(value) = raw.trim().parse::<f64>() else {
            continue;
        };
        if (0.0..=150_000.0).contains(&value) {
            maximum = Some(maximum.map_or(value, |current: f64| current.max(value)));
        }
    }
    maximum.map(|value| value / 1000.0)
}

fn fan_snapshot() -> String {
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else {
        return "ölçüm yok".into();
    };
    let mut fans = Vec::new();
    for entry in entries.flatten() {
        let directory = entry.path();
        let Ok(files) = fs::read_dir(&directory) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if !name.starts_with("fan") || !name.ends_with("_input") {
                continue;
            }
            if let Ok(value) = fs::read_to_string(file.path()) {
                fans.push(format!(
                    "{}: {} RPM",
                    name.trim_end_matches("_input"),
                    value.trim()
                ));
            }
        }
    }
    if fans.is_empty() {
        "ölçüm yok".into()
    } else {
        fans.join(" • ")
    }
}

fn disk_snapshot(path: &str) -> (Option<u64>, Option<u64>) {
    let Ok(path) = CString::new(path) else {
        return (None, None);
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: statvfs writes the initialized structure for the valid C path.
    let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return (None, None);
    }
    // SAFETY: statvfs returned success, so the structure is initialized.
    let stats = unsafe { stats.assume_init() };
    let block_size = stats.f_frsize;
    let total = stats.f_blocks.saturating_mul(block_size);
    let available = stats.f_bavail.saturating_mul(block_size);
    (Some(total.saturating_sub(available)), Some(total))
}

fn network_snapshot() -> (String, String) {
    let mut received = 0_u64;
    let mut sent = 0_u64;
    if let Ok(contents) = fs::read_to_string("/proc/net/dev") {
        for line in contents.lines().skip(2) {
            let Some((interface, values)) = line.split_once(':') else {
                continue;
            };
            if interface.trim() == "lo" {
                continue;
            }
            let fields = values.split_whitespace().collect::<Vec<_>>();
            if let (Some(rx), Some(tx)) = (fields.first(), fields.get(8)) {
                received = received.saturating_add(rx.parse().unwrap_or(0));
                sent = sent.saturating_add(tx.parse().unwrap_or(0));
            }
        }
    }
    (format_bytes(received), format_bytes(sent))
}

fn gpu_snapshot() -> String {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return "ölçüm yok".into();
    };
    let mut cards = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let device = entry.path().join("device");
        let usage = ["gpu_busy_percent", "gt_busy_percent"]
            .iter()
            .find_map(|file| fs::read_to_string(device.join(file)).ok())
            .map(|value| format!(" kullanım {}%", value.trim()))
            .unwrap_or_else(|| " kullanım ölçüm yok".into());
        let temperature = fs::read_dir(device.join("hwmon"))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .find_map(|dir| fs::read_to_string(dir.path().join("temp1_input")).ok())
            .and_then(|value| value.trim().parse::<f64>().ok())
            .map(|value| format!(" sıcaklık {:.1}°C", value / 1000.0))
            .unwrap_or_else(|| " sıcaklık ölçüm yok".into());
        cards.push(format!("{name}:{usage},{temperature}"));
    }
    if cards.is_empty() {
        "ölçüm yok".into()
    } else {
        cards.join(" • ")
    }
}

fn loopback_service_status(port: u16) -> &'static str {
    let address = ("127.0.0.1", port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next());
    address
        .and_then(|address| TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok())
        .map(|_| "erişilebilir")
        .unwrap_or("kapalı")
}

fn workspace_root() -> Result<PathBuf, String> {
    let configured = std::env::var_os("JARVIS_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    fs::canonicalize(&configured).map_err(|error| format!("workspace root is unavailable: {error}"))
}

fn read_workspace_file(input: &str) -> ToolResult {
    const MAX_READ_BYTES: u64 = 64 * 1024;
    let requested = input
        .split_once(':')
        .map(|(_, path)| path.trim())
        .filter(|path| !path.is_empty());
    let Some(requested) = requested else {
        return ToolResult {
            status: ToolStatus::Failure,
            output: String::new(),
            error: Some("file.read requires 'dosya oku: relative/path'".into()),
            state_changed: false,
            evidence: vec![],
        };
    };
    let Ok(root) = workspace_root() else {
        return ToolResult {
            status: ToolStatus::Failure,
            output: String::new(),
            error: Some("workspace root is unavailable".into()),
            state_changed: false,
            evidence: vec![],
        };
    };
    let requested_path = PathBuf::from(requested);
    if requested_path.is_absolute()
        || requested_path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return ToolResult {
            status: ToolStatus::Failure,
            output: String::new(),
            error: Some("workspace path must be a contained relative path".into()),
            state_changed: false,
            evidence: vec![],
        };
    }
    let path = root.join(requested_path);
    let canonical = match fs::canonicalize(&path) {
        Ok(path) if path.starts_with(&root) => path,
        Ok(_) => {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some("requested file escapes workspace".into()),
                state_changed: false,
                evidence: vec![],
            }
        }
        Err(error) => {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(format!("file read failed: {error}")),
                state_changed: false,
                evidence: vec![],
            }
        }
    };
    let metadata = match fs::metadata(&canonical) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_READ_BYTES => metadata,
        Ok(metadata) if metadata.len() > MAX_READ_BYTES => {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(format!("file exceeds read limit of {MAX_READ_BYTES} bytes")),
                state_changed: false,
                evidence: vec![],
            }
        }
        Ok(_) => {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some("requested path is not a regular readable file".into()),
                state_changed: false,
                evidence: vec![],
            }
        }
        Err(error) => {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(format!("file metadata failed: {error}")),
                state_changed: false,
                evidence: vec![],
            }
        }
    };
    match fs::read_to_string(&canonical) {
        Ok(content) => ToolResult {
            status: ToolStatus::Success,
            output: content,
            error: None,
            state_changed: false,
            evidence: vec![format!(
                "file.read:{}:{}",
                canonical.display(),
                metadata.len()
            )],
        },
        Err(error) => ToolResult {
            status: ToolStatus::Failure,
            output: String::new(),
            error: Some(format!("file must be valid UTF-8 text: {error}")),
            state_changed: false,
            evidence: vec![],
        },
    }
}

pub fn read_workspace_content_ref(relative_path: &str) -> Result<ContentRef, String> {
    let result = read_workspace_file(&format!("dosya oku: {relative_path}"));
    if result.status != ToolStatus::Success {
        return Err(result
            .error
            .unwrap_or_else(|| "workspace content retrieval failed".into()));
    }
    let source = result
        .evidence
        .first()
        .and_then(|evidence| evidence.strip_prefix("file.read:"))
        .and_then(|value| value.rsplit_once(':').map(|(path, _)| path.to_owned()))
        .ok_or_else(|| "workspace content retrieval produced invalid evidence".to_string())?;
    Ok(ContentRef {
        source,
        provenance: ContentProvenance::UntrustedProjectFile,
        content: result.output,
    })
}

fn project_info() -> ToolResult {
    let root = match workspace_root() {
        Ok(root) => root,
        Err(error) => {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(error),
                state_changed: false,
                evidence: vec![],
            }
        }
    };
    let cargo_manifest = root.join("Cargo.toml");
    let readme = root.join("README.md");
    let output = format!(
        "workspace={}\ncargo_manifest={}\nreadme={}",
        root.display(),
        cargo_manifest.is_file(),
        readme.is_file()
    );
    ToolResult {
        status: ToolStatus::Success,
        output,
        error: None,
        state_changed: false,
        evidence: vec![format!("project.root:{}", root.display())],
    }
}

fn project_outline() -> ToolResult {
    let root = match workspace_root() {
        Ok(root) => root,
        Err(error) => return sandbox_violation(&error),
    };
    let source_dir = root.join("src");
    let mut source_files = match fs::read_dir(&source_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".rs"))
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    source_files.sort();
    ToolResult {
        status: ToolStatus::Success,
        output: format!(
            "workspace={}\nsource_files={}",
            root.display(),
            source_files.join(",")
        ),
        error: None,
        state_changed: false,
        evidence: vec![format!("project.root:{}", root.display())],
    }
}

/// F4 "Yerel üretkenlik tool framework": the one shared contract every approval-gated local tool
/// implements. `CapabilityManifest`/`capability_manifest` already carry the risk/scope/verifier
/// metadata Policy needs; this trait is what makes a *new* tool pluggable into `execute_approved`
/// (dispatch) and the approval flow's preview (what the user sees before they approve) without
/// either of those growing a hardcoded per-tool branch. Adding a tool means: one manifest entry
/// (`capabilities.rs`), one `policy_for` arm, one struct implementing this trait, one entry in
/// `local_tool_for`'s dispatch table — nothing else has to change.
trait LocalTool: Send + Sync {
    /// Read-only, no side effect: exactly what would happen if this action is approved. Shown to
    /// the user before they approve (`Runtime::preview_pending_action`) — the same principle F4's
    /// patch preview already established for coding, generalized to every approval-gated tool.
    /// Previously, `PolicyControl::ExplainBeforeExecute` was a declared-but-unenforced control —
    /// nothing actually showed an explanation before approval; this closes that real gap.
    fn preview(&self, input: &str) -> String;
    /// The actual side-effecting action. Only ever reached through `Runtime::approve` — Task
    /// state and Policy already gate this call, this function trusts that gate and does not
    /// re-check it.
    fn execute(&self, input: &str, task_id: &str) -> ToolResult;
}

/// Whether `input` actually carries real note content after a colon. Shared by `note_body` (what
/// to save) and `Runtime`'s router guard (whether a `note.create` classification was even
/// plausible) so the two can never disagree about what counts as "real content" — see the bug
/// this closed, documented on the router guard in `runtime.rs`.
pub(crate) fn note_body_is_present(input: &str) -> bool {
    input
        .split_once(':')
        .is_some_and(|(_, text)| !text.trim().is_empty())
}

fn note_body(input: &str) -> &str {
    input
        .split_once(':')
        .map(|(_, text)| text.trim())
        .filter(|text| !text.is_empty())
        .unwrap_or("JARVIS note")
}

struct NoteCreateTool;

impl LocalTool for NoteCreateTool {
    fn preview(&self, input: &str) -> String {
        format!(
            "Yeni bir not dosyası oluşturulacak. İçerik:\n{}",
            note_body(input)
        )
    }

    fn execute(&self, input: &str, task_id: &str) -> ToolResult {
        let directory = std::env::var_os("JARVIS_NOTE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("notes"));
        if let Err(error) = fs::create_dir_all(&directory) {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(error.to_string()),
                state_changed: false,
                evidence: vec![],
            };
        }
        let root = match fs::canonicalize(&directory) {
            Ok(root) => root,
            Err(error) => {
                return ToolResult {
                    status: ToolStatus::Failure,
                    output: String::new(),
                    error: Some(error.to_string()),
                    state_changed: false,
                    evidence: vec![],
                }
            }
        };
        let path = root.join(format!(
            "{}.md",
            task_id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_")
        ));
        if path.parent() != Some(root.as_path()) || !path.starts_with(&root) {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some("note path escapes allowed root".into()),
                state_changed: false,
                evidence: vec![],
            };
        }
        let body = note_body(input);
        match fs::write(&path, format!("# JARVIS Note\n\n{}\n", body)) {
            Ok(()) => ToolResult {
                status: ToolStatus::Success,
                output: path.display().to_string(),
                error: None,
                state_changed: true,
                evidence: vec![format!("file.exists:{}", path.display())],
            },
            Err(error) => ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(error.to_string()),
                state_changed: false,
                evidence: vec![],
            },
        }
    }
}

/// F4 "Yerel üretkenlik tool framework"'ün ikinci gerçek tool'u — çerçevenin coding'e/note'a özgü
/// olmadığını kanıtlıyor. Bilerek dar: yalnız zaten var olan, workspace-göreli bir metin
/// dosyasına TEK BİR satır ekliyor (F4'ün sandbox'lı kod patch'lerinden tamamen ayrı, çok daha
/// basit bir yerel üretkenlik ihtiyacı — ör. kişisel bir todo/log dosyası).
struct FileAppendNoteTool;

/// `JARVIS_APPEND_DIR` (yoksa proje kökü altında `append-notes/`) — yalnız bu kök altındaki,
/// gizli-bilgi benzeri olmayan dosyalara ekleme yapılabilir. `note.create`'in kendi
/// `JARVIS_NOTE_DIR` deseniyle aynı; ikinci bir izin modeli icat edilmedi.
fn file_append_note_root() -> Result<PathBuf, String> {
    let directory = std::env::var_os("JARVIS_APPEND_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("append-notes"));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("append target directory unavailable: {error}"))?;
    fs::canonicalize(&directory)
        .map_err(|error| format!("append target directory unavailable: {error}"))
}

/// `input`'in `"<capability>: <relative-path>|<line>"` biçimini ortak olarak çözüyor —
/// `preview`/`execute` aynı ayrıştırmayı iki kez yazmasın diye.
fn parse_append_note_input(input: &str) -> Result<(PathBuf, String), String> {
    let body = input
        .split_once(':')
        .map(|(_, text)| text.trim())
        .unwrap_or("");
    let (path_part, line) = body
        .split_once('|')
        .ok_or_else(|| "expected format: <relative-path>|<line>".to_string())?;
    let relative_path = PathBuf::from(path_part.trim());
    workbench::validate_workspace_relative_path(&relative_path)?;
    workspace::reject_secret_like_workspace_document_name(&relative_path)?;
    let line = line.trim();
    if line.is_empty() {
        return Err("append line must not be empty".into());
    }
    if line.len() > 2_000 {
        return Err("append line exceeds the 2000-byte limit".into());
    }
    Ok((relative_path, line.to_string()))
}

impl LocalTool for FileAppendNoteTool {
    fn preview(&self, input: &str) -> String {
        match parse_append_note_input(input) {
            Ok((relative_path, line)) => format!(
                "{} dosyasına şu satır eklenecek:\n{line}",
                relative_path.display()
            ),
            Err(error) => format!("Geçersiz istek, onaylanamaz: {error}"),
        }
    }

    fn execute(&self, input: &str, _task_id: &str) -> ToolResult {
        let (relative_path, line) = match parse_append_note_input(input) {
            Ok(parsed) => parsed,
            Err(error) => {
                return ToolResult {
                    status: ToolStatus::Failure,
                    output: String::new(),
                    error: Some(error),
                    state_changed: false,
                    evidence: vec![],
                }
            }
        };
        let root = match file_append_note_root() {
            Ok(root) => root,
            Err(error) => {
                return ToolResult {
                    status: ToolStatus::Failure,
                    output: String::new(),
                    error: Some(error),
                    state_changed: false,
                    evidence: vec![],
                }
            }
        };
        let path = root.join(&relative_path);
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if existing.len() + line.len() + 1 > MAX_WORKSPACE_DOCUMENT_BYTES as usize {
            return ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some("append target would exceed the workspace document size limit".into()),
                state_changed: false,
                evidence: vec![],
            };
        }
        let mut new_content = existing;
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(&line);
        new_content.push('\n');
        match fs::write(&path, &new_content) {
            Ok(()) => ToolResult {
                status: ToolStatus::Success,
                output: path.display().to_string(),
                error: None,
                state_changed: true,
                evidence: vec![
                    format!("file.exists:{}", path.display()),
                    format!("file.contains:{}:{line}", path.display()),
                ],
            },
            Err(error) => ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(error.to_string()),
                state_changed: false,
                evidence: vec![],
            },
        }
    }
}

fn local_tool_for(capability_id: &str) -> Option<&'static dyn LocalTool> {
    match capability_id {
        "note.create" => Some(&NoteCreateTool),
        "file.append_note" => Some(&FileAppendNoteTool),
        _ => None,
    }
}

fn execute_approved(manifest: &CapabilityManifest, input: &str, task_id: &str) -> ToolResult {
    if manifest.sandbox_profile != "LOCAL_RESTRICTED" {
        return sandbox_violation(
            "this capability's sandbox profile does not allow local execution",
        );
    }
    match local_tool_for(&manifest.capability_id) {
        Some(tool) => tool.execute(input, task_id),
        None => sandbox_violation("no local tool is registered for this capability"),
    }
}

fn verify(result: &ToolResult) -> VerifierResult {
    if result.status == ToolStatus::Success && !result.evidence.is_empty() {
        for evidence in &result.evidence {
            if let Some(path) = evidence.strip_prefix("file.exists:") {
                if !std::path::Path::new(path).is_file() {
                    return VerifierResult {
                        status: VerifyStatus::Fail,
                        reason: "file evidence does not exist".into(),
                        evidence: result.evidence.clone(),
                    };
                }
            }
            if let Some(rest) = evidence.strip_prefix("file.read:") {
                let Some((path, _)) = rest.rsplit_once(':') else {
                    return VerifierResult {
                        status: VerifyStatus::Fail,
                        reason: "malformed file read evidence".into(),
                        evidence: result.evidence.clone(),
                    };
                };
                if !std::path::Path::new(path).is_file() {
                    return VerifierResult {
                        status: VerifyStatus::Fail,
                        reason: "read evidence file does not exist".into(),
                        evidence: result.evidence.clone(),
                    };
                }
            }
            if let Some(path) = evidence.strip_prefix("project.root:") {
                if !std::path::Path::new(path).is_dir() {
                    return VerifierResult {
                        status: VerifyStatus::Fail,
                        reason: "project root evidence does not exist".into(),
                        evidence: result.evidence.clone(),
                    };
                }
            }
            // F4 "Yerel üretkenlik tool framework": `file.exists`'ten daha güçlü bir doğrulama —
            // dosyanın yalnız var olmasını değil, iddia edilen içeriği gerçekten taşıdığını
            // kontrol ediyor. `FileAppendNoteTool` bunu kullanıyor.
            if let Some(rest) = evidence.strip_prefix("file.contains:") {
                let Some((path, expected_substring)) = rest.split_once(':') else {
                    return VerifierResult {
                        status: VerifyStatus::Fail,
                        reason: "malformed file contains evidence".into(),
                        evidence: result.evidence.clone(),
                    };
                };
                let contains = fs::read_to_string(path)
                    .map(|content| content.contains(expected_substring))
                    .unwrap_or(false);
                if !contains {
                    return VerifierResult {
                        status: VerifyStatus::Fail,
                        reason: "expected content was not found in the file".into(),
                        evidence: result.evidence.clone(),
                    };
                }
            }
        }
        VerifierResult {
            status: VerifyStatus::Pass,
            reason: "tool evidence is present".into(),
            evidence: result.evidence.clone(),
        }
    } else {
        VerifierResult {
            status: VerifyStatus::Fail,
            reason: result
                .error
                .clone()
                .unwrap_or_else(|| "missing evidence".into()),
            evidence: result.evidence.clone(),
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
