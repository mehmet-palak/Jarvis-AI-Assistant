//! The Decision Core / Orchestrator: `Runtime` is the single place a `Request` or
//! `McpIngressRequest` becomes a `Task` and runs the
//! `Intent -> Policy -> Task -> Tool -> Verifier -> Audit` chain. No client, model output, or MCP
//! tool ID reaches a capability's execution without going through `policy_for` and the registered
//! `CapabilityRegistry` here first.
//!
//! This module intentionally imports the crate root with a glob: `Runtime` is the orchestrator
//! that ties together nearly every public and crate-internal contract (policy, persistence,
//! model routing, memory, workspace RAG, workbench, vision, audit) rather than owning a narrow
//! concern of its own, so an explicit per-symbol import list would just restate the crate's
//! surface with no added clarity.
use crate::*;
use pentest_safe_checks::{
    PentestExposedFileFinding, PentestTechnologyFingerprint, PentestTlsCheckResult,
    TakeoverSignatureMatch,
};

use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// F7.3 "Aktif keşif" port taramasının kendi kendini sınırlaması: bir çağrının tek başına
/// birinin sunucusuna yönelik sınırsız bir tarama aracına dönüşemeyeceği tavan. Ayrı ayrı çok
/// sayıda çağrı yapılabilir (o zaman devreye scope'un `max_runtime_seconds`'ı girer), ama TEK
/// bir çağrı asla binlerce portu art arda deneyemez.
const MAX_PENTEST_PORTS_PER_SCAN: usize = 200;

/// Tek bir port denemesi için bağlanma zaman aşımı — ne çok kısa (açık bir portu "kapalı" gibi
/// yanlış raporlamak) ne çok uzun (yavaş/filtrelenmiş bir port taramayı gereksiz uzatmak).
const PENTEST_PORT_CONNECT_TIMEOUT: Duration = Duration::from_millis(800);

/// F7.3 "Aktif keşif: subdomain brute-force" — aynı kendi-kendini-sınırlama disiplini port
/// taramasıyla aynı: tek bir çağrı binlerce DNS sorgusu üretemez.
const MAX_PENTEST_DNS_BRUTEFORCE_WORDS: usize = 2000;

#[derive(Debug)]
pub struct Runtime {
    pub tasks: HashMap<String, Task>,
    pub audit: Vec<AuditEvent>,
    pub approvals: HashMap<String, Approval>,
    pub registry: CapabilityRegistry,
    pending_inputs: HashMap<String, String>,
    pub(crate) store: Option<SqliteStore>,
    audit_sequence: u64,
    audit_hash: String,
    structured_logs: Vec<StructuredLogEvent>,
    pub(crate) chat_history: Vec<ConversationMessage>,
    /// Optional hybrid-retrieval embedding adapter (F3 madde 13, ADR-0004). `None` by default —
    /// every workspace indexing/search path already degrades to FTS-only when this is unset,
    /// so attaching or removing it is never a breaking change for an existing `Runtime`.
    embedding_provider: Option<Box<dyn EmbeddingProvider>>,
    /// F3 "Citation UX": the exact citations used to ground the most recent conversational
    /// reply, full chunk content included — not just the compact `evidence` strings, which only
    /// carry a path and chunk ordinal. This is what a caller (TUI `/source <n>`) reads to show
    /// "kaynağı aç": the complete chunk text, not only its short excerpt. Overwritten every turn
    /// (empty when that turn used none); never persisted, this is in-memory display state only.
    last_workspace_citations: Vec<WorkspaceCitation>,
    /// F3 post-close "gözlemlenebilirlik" (GPT önerisi 4/7), surfaced via `rag_status`. Session-
    /// only counters (never persisted) — how many retrieval calls this session actually used the
    /// embedding signal vs. degraded to plain FTS.
    hybrid_queries_this_session: usize,
    fts_only_queries_this_session: usize,
    /// Kullanıcının elle düzenlediği "bana dair"/"JARVIS'e dair" dosyalarının bulunduğu klasör
    /// (`src/profile_files.rs`). `None` — çağıran (TUI/native) hiç ayarlamadıysa — bu özellik
    /// tamamen devre dışıdır, hiçbir davranış değişmez (embedding_provider ile aynı "isteğe bağlı
    /// ek, asla zorunlu bağımlılık değil" deseni).
    profile_files_dir: Option<PathBuf>,
    /// İsteğe bağlı hava durumu sağlayıcısı — JARVIS'in tek gerçek internet erişimi gerektiren
    /// yeteneği (kullanıcı onayıyla, 16 Ağustos 2026). Yalnız açılış karşılamasında kullanılır;
    /// hiçbir governed capability/task/policy yoluna hiç girmez, model ona hiç "çağrı" yapamaz —
    /// yalnız Runtime'ın kendisi, başlangıç metnini oluştururken bir kez okur.
    weather_provider: Option<Box<dyn WeatherProvider>>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            tasks: HashMap::new(),
            audit: Vec::new(),
            approvals: HashMap::new(),
            pending_inputs: HashMap::new(),
            store: None,
            registry: CapabilityRegistry::baseline(),
            audit_sequence: 0,
            audit_hash: "GENESIS".into(),
            structured_logs: Vec::new(),
            chat_history: Vec::new(),
            embedding_provider: None,
            last_workspace_citations: Vec::new(),
            hybrid_queries_this_session: 0,
            fts_only_queries_this_session: 0,
            profile_files_dir: None,
            weather_provider: None,
        }
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            registry: CapabilityRegistry::baseline(),
            ..Self::default()
        }
    }

    pub fn with_store(store: SqliteStore) -> Self {
        store
            .recover_interrupted_tasks()
            .expect("startup task recovery must succeed");
        assert!(
            store
                .audit_chain_is_valid()
                .expect("audit integrity verification must succeed"),
            "audit integrity verification failed"
        );
        let (audit_sequence, audit_hash) =
            store.audit_tail().expect("audit tail query must succeed");
        // User-requested (2026-08-16): a new session picks up conversation history from the last
        // one instead of starting empty — `Runtime::new()` (no store) still starts empty, since
        // there is nowhere to have persisted anything.
        let chat_history = store
            .recent_chat_messages(MAX_COMPLETED_CHAT_HISTORY_TURNS)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(role, content)| {
                // `ConversationMessage.role` is `&'static str`, not an owned `String` — map the
                // persisted role back onto the same two literals `append_chat_turn` ever writes.
                // An unrecognized role (foreign/corrupted data) is dropped, never guessed at.
                let static_role = match role.as_str() {
                    "user" => "user",
                    "assistant" => "assistant",
                    _ => return None,
                };
                Some(ConversationMessage {
                    role: static_role,
                    content,
                })
            })
            .collect();
        Self {
            store: Some(store),
            registry: CapabilityRegistry::baseline(),
            audit_sequence,
            audit_hash,
            chat_history,
            ..Self::default()
        }
    }

    fn record_audit(&mut self, mut event: AuditEvent) {
        if let Some(store) = self.store.as_mut() {
            event = store
                .append_audit_chain(&event.task_id, &event.event)
                .expect("audit persistence must succeed");
        } else {
            event.sequence = self.audit_sequence + 1;
            event.previous_hash = self.audit_hash.clone();
            event.event_hash = audit_hash(
                event.sequence,
                &event.previous_hash,
                &event.task_id,
                &event.event,
            );
        }
        self.audit_sequence = event.sequence;
        self.audit_hash = event.event_hash.clone();
        self.structured_logs.push(StructuredLogEvent {
            timestamp: now_epoch(),
            level: if event.event.contains("failed") || event.event.contains("invalid") {
                LogLevel::Warn
            } else {
                LogLevel::Info
            },
            correlation_id: event.task_id.clone(),
            task_id: event.task_id.clone(),
            event: event.event.clone(),
        });
        self.audit.push(event);
    }

    fn save_task(&self, task: &Task) {
        if let Some(store) = &self.store {
            store
                .save_task(task)
                .expect("task persistence must succeed");
        }
    }

    pub fn task_summaries(&self) -> Vec<&Task> {
        let mut tasks: Vec<_> = self.tasks.values().collect();
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        tasks
    }

    pub fn structured_logs(&self) -> &[StructuredLogEvent] {
        &self.structured_logs
    }

    fn append_chat_turn(&mut self, role: &'static str, content: String) {
        // Keep only whole, most-recent exchanges. Before a new user turn we remove the oldest
        // user/assistant pair, so the model never receives an orphaned assistant reply.
        if role == "user" {
            while self.chat_history.len() >= MAX_COMPLETED_CHAT_HISTORY_TURNS {
                self.chat_history.drain(0..2);
            }
        }
        // Best-effort persistence: a write/prune failure here must never fail the actual
        // conversation turn — the in-memory copy (below) is still authoritative for this turn
        // either way. Pruned to the same cap as the in-memory history on every append, so the
        // on-disk table can never grow past what a session would ever load back anyway.
        if let Some(store) = self.store.as_ref() {
            let _ = store.append_chat_message(role, &content);
            let _ = store.prune_chat_messages_to(MAX_COMPLETED_CHAT_HISTORY_TURNS);
        }
        self.chat_history
            .push(ConversationMessage { role, content });
        if role == "assistant" {
            while self.chat_history.len() > MAX_COMPLETED_CHAT_HISTORY_TURNS {
                self.chat_history.drain(0..2);
            }
        }
    }

    /// User-requested (2026-08-16): clears conversation history for real — both the in-memory
    /// copy the model sees and, if a store is attached, the persisted copy on disk. Before chat
    /// persistence existed, `/clear` only ever reset the TUI's visible message list while the
    /// model's own context quietly lived on; now that history survives a restart, "clear" has to
    /// actually mean clear. Returns how many persisted rows were removed (`0` with no store).
    pub fn clear_chat_history(&mut self) -> Result<usize, String> {
        self.chat_history.clear();
        match self.store.as_ref() {
            Some(store) => store
                .clear_chat_messages()
                .map_err(|error| error.to_string()),
            None => Ok(0),
        }
    }

    pub(crate) fn conversation_context(&self) -> String {
        let turns = self
            .chat_history
            .iter()
            .map(|turn| format!("[{}]\n{}", turn.role, turn.content))
            .collect::<Vec<_>>()
            .join("\n\n");
        format!("<conversation-history-data>\n{turns}\n</conversation-history-data>")
    }

    fn approved_memory_context(&self) -> Vec<MemoryRecord> {
        self.store
            .as_ref()
            .and_then(|store| {
                // All five namespaces are eligible; `retrieve_memory` already excludes expired
                // rows (`expires_at IS NULL OR expires_at > now`), so an expired Session or
                // EphemeralToolOutput record never reaches the model as if it were still current.
                // `task_scope: None` — an ordinary conversational turn is not "about" any one
                // task, so `Task` namespace records are excluded entirely here (kullanıcının
                // "concurrent task'lar birbirinin context'ini kirletmesin" kuralı): a specific
                // task's memory only ever reaches the model through
                // `Runtime::task_scoped_memory_context`, scoped to exactly that task_id.
                store
                    .retrieve_memory(
                        &[
                            MemoryNamespace::UserProfile,
                            MemoryNamespace::Project,
                            MemoryNamespace::Task,
                            MemoryNamespace::Session,
                            MemoryNamespace::EphemeralToolOutput,
                        ],
                        None,
                        8,
                    )
                    .ok()
            })
            .unwrap_or_default()
    }

    /// Kullanıcının "concurrent task'lar birbirinin context'ini kirletmesin" kuralının doğrudan
    /// uygulama noktası: yalnız `task_id`'ye taahhüt edilmiş `Task` namespace kayıtlarını döner —
    /// başka hiçbir task'ın kaydı asla karışmaz. Sıradan sohbet bağlamı (`approved_memory_context`)
    /// bunu hiç çağırmaz; bu yalnız belirli bir task'ın gerçekten aktif olarak takip edildiği bir
    /// akışta kullanılmak üzere var (henüz hiçbir üretim çağrı noktası yok — F3 sonrası şema/API
    /// hazırlığı, bkz. `MemoryRecord::scope_id`).
    pub fn task_scoped_memory_context(&self, task_id: &str) -> Vec<MemoryRecord> {
        self.store
            .as_ref()
            .and_then(|store| {
                store
                    .retrieve_memory(&[MemoryNamespace::Task], Some(task_id), 8)
                    .ok()
            })
            .unwrap_or_default()
    }

    /// Saves a proposal only after a UI/CLI has shown it to the user and obtained approval.
    /// Conversation responses cannot call this implicitly.
    pub fn commit_memory_proposal(
        &mut self,
        proposal: &MemoryProposal,
        user_approved: bool,
    ) -> Result<MemoryRecord, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "persistent memory requires an attached local store".to_string())?;
        let record = store.commit_memory_proposal(proposal, user_approved)?;
        self.record_audit(AuditEvent::pending(
            format!("memory-{}", record.memory_id),
            "memory.write.user_approved",
        ));
        Ok(record)
    }

    pub fn delete_memory(&mut self, memory_id: &str) -> Result<bool, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "persistent memory requires an attached local store".to_string())?;
        let deleted = store.delete_memory(memory_id)?;
        if deleted {
            self.record_audit(AuditEvent::pending(
                format!("memory-{memory_id}"),
                "memory.delete.user_requested",
            ));
        }
        Ok(deleted)
    }

    /// Deletes every record in one namespace only (e.g. all `Project` memory), leaving the other
    /// four namespaces untouched. A real, hard `DELETE` — see ADR-0003's "Silme: tombstone yok"
    /// addendum for why this project does not keep a soft-deleted/tombstoned copy.
    pub fn delete_memory_namespace(&mut self, namespace: MemoryNamespace) -> Result<usize, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "persistent memory requires an attached local store".to_string())?;
        let deleted = store.delete_memory_namespace(namespace)?;
        if deleted > 0 {
            self.record_audit(AuditEvent::pending(
                format!("memory-namespace-{}", namespace.as_str()),
                "memory.delete.namespace_user_requested",
            ));
        }
        Ok(deleted)
    }

    /// Deletes every record, in any namespace, whose key matches `key` (Turkish-fold-insensitive
    /// — see `turkish_case_fold`). Convenience for natural-language "belleğimden X bilgisini
    /// sil" (`MemoryIntent::ForgetKey`): the user names a concept, not an opaque `memory_id` or
    /// which namespace it happened to be saved under. Built only from already-public primitives
    /// (`list_memory`/`delete_memory`) — no new persistence-layer query.
    pub fn delete_memory_by_key(&mut self, key: &str) -> Result<usize, String> {
        let folded_key = turkish_case_fold(key.trim());
        let matching_ids: Vec<String> = self
            .list_memory()?
            .into_iter()
            .filter(|record| turkish_case_fold(&record.key) == folded_key)
            .map(|record| record.memory_id)
            .collect();
        let mut deleted = 0;
        for memory_id in matching_ids {
            if self.delete_memory(&memory_id)? {
                deleted += 1;
            }
        }
        Ok(deleted)
    }

    /// Kullanıcının "secret'ları doğrudan hafızaya yazmıyoruz, Secret Manager referansı
    /// tutuyoruz" kuralı. Gerçek değer yalnız ayrı `secrets` tablosunda (`SqliteStore::store_secret`);
    /// `memories`'e yalnız bir yer tutucu satır ekleniyor — `Sensitive`, `include_in_model_context=false`
    /// — böylece `/memory` bu anahtarın var olduğunu gösterir ama modele giden sıradan sohbet
    /// bağlamı (`approved_memory_context`, `include_in_model_context=1` filtresi) buna hiç
    /// dokunmaz; gerçek değer yalnız `reveal_secret`'ın açık, kullanıcı-tetiklemeli çağrısıyla
    /// ortaya çıkar. Audit yalnız anahtar adını taşır, gerçek değeri asla (F3 "filtre loglanır ama
    /// sır saklanmaz" ilkesiyle aynı desen).
    pub fn remember_secret(&mut self, key: &str, value: &str) -> Result<(), String> {
        {
            let store = self
                .store
                .as_ref()
                .ok_or_else(|| "secrets require an attached local store".to_string())?;
            store.store_secret(key, value, "user-command")?;
            let placeholder = propose_memory_with_trust_and_scope(
                MemoryNamespace::UserProfile,
                key,
                "[gizli değer — /secret show ile görüntülenir]",
                DataSensitivity::Sensitive,
                "secret-manager",
                false,
                None,
                TrustLevel::UserAsserted,
                None,
            )?;
            store.commit_memory_proposal(&placeholder, true)?;
        }
        self.record_audit(AuditEvent::pending(
            format!("secret:{key}"),
            "secret.remembered",
        ));
        Ok(())
    }

    /// Gerçek sır değerini döner — yalnız kullanıcının kendi açık talebiyle (`/secret show
    /// <anahtar>`) çağrılmalı. Hiçbir sohbet/model bağlamı derleme yolu bunu hiç çağırmaz.
    pub fn reveal_secret(&mut self, key: &str) -> Result<Option<String>, String> {
        let value = {
            let store = self
                .store
                .as_ref()
                .ok_or_else(|| "secrets require an attached local store".to_string())?;
            store.resolve_secret(key)?
        };
        self.record_audit(AuditEvent::pending(
            format!("secret:{key}"),
            "secret.revealed",
        ));
        Ok(value)
    }

    /// Hem gerçek sır değerini (`secrets` tablosu) hem `memories`'teki yer tutucu satırı siler —
    /// ikisi de kalırsa `/memory` listesinde sahipsiz bir yer tutucu görünmeye devam ederdi.
    pub fn forget_secret(&mut self, key: &str) -> Result<bool, String> {
        let deleted = {
            let store = self
                .store
                .as_ref()
                .ok_or_else(|| "secrets require an attached local store".to_string())?;
            store.delete_secret(key)?
        };
        if deleted {
            let _ = self.delete_memory_by_key(key);
            self.record_audit(AuditEvent::pending(
                format!("secret:{key}"),
                "secret.forgotten",
            ));
        }
        Ok(deleted)
    }

    /// Yalnız anahtar adları — hiçbir zaman değer içermez.
    pub fn list_secret_keys(&self) -> Result<Vec<String>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| "secrets require an attached local store".to_string())?
            .list_secret_keys()
    }

    /// Deletes a known profile field's current record, if any (`Ok(false)` when the field was
    /// never set). Shared by `/profile delete <field>` and natural-language "hafızandan X
    /// bilgisini sil" (`MemoryIntent::ForgetProfileField`) — exactly one place resolves a
    /// `ProfileField` to its current record and removes it.
    pub fn delete_profile_field(&mut self, field: ProfileField) -> Result<bool, String> {
        let snapshot = self.profile_snapshot()?;
        match snapshot.record_for(field) {
            Some(record) => self.delete_memory(&record.memory_id),
            None => Ok(false),
        }
    }

    pub fn list_memory(&self) -> Result<Vec<MemoryRecord>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| "persistent memory requires an attached local store".to_string())?
            .list_memory()
    }

    /// Bilinen profil alanlarının (ad/hitap/dil/rol) güncel değerlerini döner. TUI ve native UI
    /// aynı anlığı görsün diye tek yerde toplanır; `ProfileSnapshot::from_records` mantığını iki
    /// arayüzde de ayrı ayrı yazmamak için.
    pub fn profile_snapshot(&self) -> Result<ProfileSnapshot, String> {
        Ok(ProfileSnapshot::from_records(&self.list_memory()?))
    }

    pub fn forget_all_memory(&mut self) -> Result<usize, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "persistent memory requires an attached local store".to_string())?;
        let deleted = store.forget_all_memory()?;
        if deleted > 0 {
            self.record_audit(AuditEvent::pending(
                "memory-all",
                "memory.delete_all.user_requested",
            ));
        }
        Ok(deleted)
    }

    /// Applies a separately reviewed and hash-bound coding patch, then records a correlation
    /// event. This API deliberately accepts an `ApprovedPatch` receipt rather than a boolean.
    pub fn apply_approved_coding_patch(
        &mut self,
        plan: &CodingPlan,
        proposal: &PatchProposal,
        approval: &ApprovedPatch,
    ) -> Result<PatchApplication, String> {
        let application = apply_approved_patch(plan, proposal, approval)?;
        self.record_audit(AuditEvent::pending(
            format!("coding-{}", application.proposal_id),
            format!("coding.patch.applied:{}", application.diff_sha256),
        ));
        Ok(application)
    }

    /// Indexing is a separate, visible action. The caller must pass the folder and file that the
    /// user approved; JARVIS never scans a home directory or project automatically.
    pub fn index_workspace_document(
        &mut self,
        approved_root: &Path,
        relative_path: &Path,
        user_approved: bool,
    ) -> Result<WorkspaceIngestionReport, String> {
        self.index_workspace_document_with_sensitivity(
            approved_root,
            relative_path,
            DataSensitivity::Internal,
            user_approved,
        )
    }

    /// Same as `index_workspace_document`, with an explicit sensitivity level (F3 post-close
    /// "retrieval öncesi permission/sensitivity filtresi", GPT önerisi 1/7). `Sensitive`
    /// documents are excluded from ordinary conversational retrieval — see
    /// `SqliteStore::search_workspace`'s doc comment for exactly where that filter lives.
    pub fn index_workspace_document_with_sensitivity(
        &mut self,
        approved_root: &Path,
        relative_path: &Path,
        sensitivity: DataSensitivity,
        user_approved: bool,
    ) -> Result<WorkspaceIngestionReport, String> {
        if !user_approved {
            return Err("workspace indexing requires explicit user approval".into());
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "workspace indexing requires an attached local store".to_string())?;
        let outcome = store.index_workspace_document_with_embedding_and_sensitivity(
            approved_root,
            relative_path,
            self.embedding_provider.as_deref(),
            sensitivity,
        );
        // F3 "Secret/hassas filtre: ... filtre loglanır ama sır saklanmaz." A rejection is always
        // audited — the relative path and a fixed reason category, never the file's bytes/text —
        // so a secret-like exclusion is visible in the audit trail without the audit trail itself
        // ever becoming a place a credential could leak to. The secret-like reason gets its own
        // event name so it is distinguishable from an unrelated rejection (oversized, binary,
        // path-escape, ...).
        let report = match outcome {
            Ok(report) => report,
            Err(error) => {
                let event_name = if is_secret_like_rejection(&error) {
                    "workspace.index.rejected_secret_like"
                } else {
                    "workspace.index.rejected"
                };
                self.record_audit(AuditEvent::pending(
                    format!("workspace-path:{}", relative_path.display()),
                    event_name,
                ));
                return Err(error);
            }
        };
        self.record_audit(AuditEvent::pending(
            format!("workspace-{}", report.document_id),
            "workspace.index.user_approved",
        ));
        Ok(report)
    }

    /// Attaches (or, with `None`, detaches) the hybrid-retrieval embedding provider. Every
    /// indexing/search path already tolerates `None` — this exists so a caller (TUI/native
    /// startup) can opt in only when the local embedding service is actually reachable.
    pub fn set_embedding_provider(&mut self, provider: Option<Box<dyn EmbeddingProvider>>) {
        self.embedding_provider = provider;
    }

    /// Kullanıcının elle düzenlediği profil dosyalarının okunacağı klasörü ayarlar (bkz.
    /// `profile_files.rs`). Çağıran taraf (TUI/native) klasör yoksa şablon dosyalarla oluşturmak
    /// için `ensure_profile_files_exist`'i kendisi çağırmalı — bu yalnız *nereden okunacağını*
    /// belirler, dosya oluşturmaz.
    pub fn set_profile_files_dir(&mut self, dir: Option<PathBuf>) {
        self.profile_files_dir = dir;
    }

    /// "Bana dair"/"JARVIS'e dair" dosyalarını (varsa) taze okur ve model verisi zarfına sarar —
    /// bellek kayıtları için kullanılan aynı "data, instruction değil" ilkesiyle
    /// (`isolate_memory_as_data`), kullanıcı bu dosyaları kendi yazmış olsa bile model buradan
    /// hiçbir zaman tool/policy yetkisi kazanmaz. Her turda taze okunur (önbelleklenmez) — dosya
    /// küçük bir boyuta zaten kırpılıyor (`MAX_PROFILE_FILE_CHARS`), kullanıcı JARVIS çalışırken
    /// dosyayı değiştirirse bir sonraki turda hemen yansısın diye.
    fn profile_file_context(&self) -> Vec<ConversationMessage> {
        let Some(dir) = self.profile_files_dir.as_ref() else {
            return Vec::new();
        };
        [
            ("Kullanıcı hakkında", ABOUT_USER_FILE_NAME),
            ("JARVIS hakkında", ABOUT_JARVIS_FILE_NAME),
        ]
        .into_iter()
        .filter_map(|(label, file_name)| {
            let content = read_profile_file(&dir.join(file_name))?;
            Some(ConversationMessage {
                role: "user",
                content: isolate_profile_file_as_data(label, &content),
            })
        })
        .collect()
    }

    /// İsteğe bağlı hava durumu sağlayıcısını bağlar (ya da `None` ile ayırır). JARVIS'in
    /// gerçek internete çıkan tek yeteneği — yalnız açılış karşılamasında kullanılır, hiçbir
    /// governed capability/task/policy yoluna hiç girmez.
    pub fn set_weather_provider(&mut self, provider: Option<Box<dyn WeatherProvider>>) {
        self.weather_provider = provider;
    }

    /// JARVIS her açıldığında gösterilecek karşılama metni: isim (varsa), bekleyen onaylar, son
    /// güncellenen (görev/oturum/geçici olmayan — yani gerçekten "not" sayılabilecek) bellek
    /// kayıtları ve (sağlayıcı bağlıysa) güncel hava durumu. Yalnız yerelde zaten elde olan
    /// verilerle çalışır — hava durumu dışında hiçbir yeni veri kaynağı gerektirmez, ve o da
    /// isteğe bağlıdır (sağlayıcı bağlı değilse o satır hiç görünmez, hata değil).
    pub fn startup_briefing(&self) -> String {
        let mut lines = Vec::new();
        let greeting = self
            .profile_snapshot()
            .ok()
            .and_then(|snapshot| {
                snapshot
                    .record_for(ProfileField::PreferredAddress)
                    .or_else(|| snapshot.record_for(ProfileField::DisplayName))
                    .map(|record| record.value.clone())
            })
            .map(|name| format!("Hoş geldiniz, {name}."))
            .unwrap_or_else(|| "Hoş geldiniz.".to_string());
        lines.push(greeting);

        if let Some(provider) = self.weather_provider.as_ref() {
            if let Ok(weather) = provider.current_weather() {
                lines.push(format!(
                    "{}: {}°C, {}.",
                    weather.location, weather.temperature_celsius, weather.description
                ));
            }
        }

        let pending = self.pending_approvals().len();
        if pending > 0 {
            lines.push(format!("{pending} bekleyen onayınız var."));
        }

        if let Ok(records) = self.list_memory() {
            let mut notes: Vec<_> = records
                .iter()
                .filter(|record| {
                    matches!(
                        record.namespace,
                        MemoryNamespace::Project | MemoryNamespace::UserProfile
                    ) && record.source != "secret-manager"
                })
                .collect();
            notes.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
            if !notes.is_empty() {
                let preview = notes
                    .iter()
                    .take(3)
                    .map(|record| format!("{} = {}", record.key, record.value))
                    .collect::<Vec<_>>()
                    .join("; ");
                lines.push(format!("Son notlarınız: {preview}."));
            }
        }

        lines.join(" ")
    }

    /// Whether workspace retrieval is currently hybrid (FTS + embedding) or FTS-only. Surfaced to
    /// the user (`/status`) so hybrid mode is never a silent, invisible behavior change.
    pub fn embedding_status(&self) -> Option<&str> {
        self.embedding_provider
            .as_ref()
            .map(|provider| provider.embedding_model_id())
    }

    /// F6 "Kullanıcı geri bildirimi intake'i". Records a raw signal about one turn. This can
    /// never create training data by itself — that requires `promote_feedback_candidate`, which
    /// goes through the policy gate.
    pub fn record_feedback(&self, candidate: &FeedbackCandidate) -> Result<(), String> {
        self.store
            .as_ref()
            .ok_or_else(|| "feedback intake requires an attached local store".to_string())?
            .record_feedback_candidate(candidate)
    }

    pub fn feedback_candidates(
        &self,
        review: Option<FeedbackReview>,
        limit: usize,
    ) -> Result<Vec<FeedbackCandidate>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| "feedback intake requires an attached local store".to_string())?
            .feedback_candidates(review, limit)
    }

    pub fn review_feedback(
        &self,
        candidate_id: &str,
        review: FeedbackReview,
    ) -> Result<(), String> {
        self.store
            .as_ref()
            .ok_or_else(|| "feedback intake requires an attached local store".to_string())?
            .set_feedback_review(candidate_id, review)
    }

    /// The one path from user feedback to training data. Every rule the F6 plan states about
    /// this transition is enforced by `feedback_candidate_is_promotable` (human review,
    /// sensitivity, and that a bare "this was wrong" carries nothing to learn from) before the
    /// example is even constructed, and `append_teacher_example` then applies the pre-existing
    /// `TeacherExample` contract on top. There is deliberately no way to skip either check.
    pub fn promote_feedback_candidate(
        &self,
        candidate_id: &str,
        expected_capability: &str,
    ) -> Result<TeacherExample, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "feedback intake requires an attached local store".to_string())?;
        let candidate = store
            .feedback_candidates(None, 1_000)?
            .into_iter()
            .find(|item| item.candidate_id == candidate_id)
            .ok_or_else(|| format!("feedback candidate not found: {candidate_id}"))?;
        feedback_candidate_is_promotable(&candidate)?;

        // A correction supersedes what the model actually said; a positive signal endorses it.
        let response = match candidate.signal {
            FeedbackSignal::Correction => candidate.correction.clone(),
            _ => candidate.response.clone(),
        };
        let example = TeacherExample {
            schema_version: 1,
            example_id: format!("from-feedback-{}", candidate.candidate_id),
            prompt: candidate.prompt.clone(),
            expected_capability: expected_capability.to_string(),
            response,
            evidence: vec![format!(
                "human-approved user feedback ({})",
                candidate.signal.as_str()
            )],
            verifier_status: VerifyStatus::Pass,
            provenance: candidate.provenance.clone(),
            human_reviewed: true,
            sensitivity: candidate.sensitivity,
        };
        store.append_teacher_example(&example, &self.registry)?;
        Ok(example)
    }

    /// F6 "Dataset export/versioning". Builds a versioned dataset artifact from the stored
    /// teacher examples. Eligibility and marker rules live in `dataset`; this only supplies the
    /// store's contents so there is a single implementation of "what may be exported".
    pub fn export_dataset(
        &self,
        dataset_version: u32,
        markers: &[DatasetMarker],
    ) -> Result<DatasetExport, String> {
        let examples = self
            .store
            .as_ref()
            .ok_or_else(|| "dataset export requires an attached local store".to_string())?
            .teacher_examples()?;
        Ok(build_dataset_export(dataset_version, &examples, markers))
    }

    /// F6 "Prompt/model konfigürasyon registry'si". Records one measured experiment. Recording
    /// never changes which model or prompt is in use — see `ModelConfigRun`'s doc comment for
    /// why the registry is deliberately a log, not a switch.
    pub fn record_model_config_run(&self, run: &ModelConfigRun) -> Result<(), String> {
        self.store
            .as_ref()
            .ok_or_else(|| "model config registry requires an attached local store".to_string())?
            .record_model_config_run(run)
    }

    /// Newest-first recorded configurations. The first row is the current configuration; its
    /// `rollback_target` names the run to return to if it regressed.
    pub fn model_config_runs(&self, limit: usize) -> Result<Vec<ModelConfigRun>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| "model config registry requires an attached local store".to_string())?
            .model_config_runs(limit)
    }

    /// F6 "Old-vs-new regresyonu ve tek komutla model/adaptor rollback".
    ///
    /// Compares the newest recorded configuration against the one it names as its rollback
    /// target and says whether adopting it was justified. The verdict is deliberately
    /// conservative: a configuration that loses *any* scenario is a regression even if it is
    /// faster, because F6's completion criterion is that a change must improve the eval without
    /// producing a safety or latency regression — not that it may trade correctness for speed.
    pub fn model_config_regression(&self) -> Result<Option<ModelConfigComparison>, String> {
        let runs = self.model_config_runs(50)?;
        let Some(current) = runs.first() else {
            return Ok(None);
        };
        let Some(target_id) = current.rollback_target.as_ref() else {
            return Ok(None);
        };
        let Some(previous) = runs.iter().find(|run| &run.run_id == target_id) else {
            return Err(format!(
                "rollback target {target_id} is not in the registry; cannot compare"
            ));
        };
        Ok(Some(compare_model_config_runs(previous, current)))
    }

    /// F7.1 — the single entry point any future pentest/security capability must call before
    /// touching a target. There is deliberately no other way in: a caller cannot pass its own
    /// `PentestScope` and get a decision, because that would let a stale or hand-typed scope
    /// override whatever the user actually has active right now. Authorization always comes from
    /// the stored *active* scope, exactly one of them, looked up fresh on every call — so
    /// revoking or switching scopes takes effect immediately, with no cached decision anywhere.
    ///
    /// "No active scope" is a hard deny, not a missing-feature error: it is the correct, safe
    /// answer when the user has not explicitly authorized anything yet.
    pub fn authorize_pentest_action(
        &self,
        target: &str,
        requested_mode: PentestMode,
    ) -> Result<StoredPentestScope, String> {
        let active = self.active_verified_pentest_scope()?;
        authorize_pentest_target(&active.scope, target, requested_mode)?;
        Ok(active)
    }

    /// The active scope, checked exactly the way `authorize_pentest_action` checks it (exists,
    /// not revoked, signature still valid) but *without* also authorizing a specific target/mode.
    /// Factored out for F7.3 recon: a passive lookup (e.g. certificate transparency) does not
    /// touch any target and so has no single `(target, mode)` pair to check yet — it needs the
    /// scope itself, verified, so it can classify a whole batch of candidate names against it.
    /// `authorize_pentest_action` stays the only entry point for anything that *acts* on a
    /// target; this is not a second way to get that authorization, only a shared prerequisite.
    fn active_verified_pentest_scope(&self) -> Result<StoredPentestScope, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "pentest authorization requires an attached local store".to_string())?;
        let Some(active) = store.active_pentest_scope()? else {
            return Err(
                "no active pentest scope — authorize one with /pentest activate <isim> before testing anything"
                    .to_string(),
            );
        };
        // Defense in depth: `set_active_pentest_scope`/`revoke_pentest_scope` already keep these
        // two facts from coexisting, but a security gate should never trust "it can't happen" —
        // it should still refuse if it somehow did.
        if active.is_revoked() {
            return Err(format!(
                "the active pentest scope '{}' has been revoked and cannot authorize anything",
                active.name
            ));
        }
        // F7.1 "İmzalı authorization/scope manifest": verified on every call, not only at save
        // time, so a scope edited on disk after being saved (a direct database edit, a restored
        // backup from a different machine) is caught the moment it would matter — right before
        // it authorizes anything — not merely when it happened to be re-saved.
        if !store.pentest_scope_signature_is_valid(&active.name)? {
            return Err(format!(
                "the active pentest scope '{}' failed signature verification — it was modified \
                 outside JARVIS and cannot be trusted; re-save it to reauthorize",
                active.name
            ));
        }
        Ok(active)
    }

    /// F7.3 pasif keşif: sertifika şeffaflık (Certificate Transparency) kayıtlarından bir kök
    /// alan adının bilinen alt alan adlarını sorgular — hedefin kendisine TEK BİR paket bile
    /// gitmez, yalnız halka açık crt.sh servisine sorgu atılır (bu, F7.1'in not ettiği "pasif
    /// keşif hedefe hiç dokunmaz" ayrımının kendisi). Aktif bir scope zorunlu (rastgele bir alan
    /// adı için sorgu atmayı serbest bırakmamak için — recon da F7.1'in yetkilendirme kapısından
    /// geçer), ama tek tek sonuç hedefe göre SAFE/ACTIVE mod tavanı kontrolü YAPILMAZ çünkü henüz
    /// hiçbir eyleme dair mod seçilmedi; yalnız "bu isim scope'un beyan ettiği sınırlar içinde mi"
    /// sorusu soruluyor (bu yüzden sınıflandırma `PentestMode::Safe` ile yapılıyor — her geçerli
    /// scope'un maximum_mode'u en az bunu kapsar, yani bu yalnızca hedef eşleşmesini sınıyor).
    pub fn discover_pentest_assets_via_certificate_transparency(
        &self,
        apex_domain: &str,
    ) -> Result<PentestReconResult, String> {
        let candidates = crate::pentest_recon::query_certificate_transparency(apex_domain)?;
        self.record_pentest_recon_candidates(apex_domain, "certificate_transparency", candidates)
    }

    /// F7.3 "Aktif keşif: subdomain brute-force". Plan metninin kendi sınıflandırmasıyla bu,
    /// yukarıdaki sertifika şeffaflık sorgusundan farklı bir tavan gerektiriyor: hedefin kendi
    /// sunucusuna hiçbir paket gitmese de (yalnız DNS altyapısına sorgu gidiyor), scope'un
    /// `maximum_mode`'unun en az `Active` olması zorunlu — yalnız hedef eşleşmesi yeterli değil.
    /// Bu kontrol burada, tüm brute-force işlemi için BİR KEZ yapılıyor (her aday için ayrı ayrı
    /// değil) çünkü "bu tür bir keşfe izin var mı" sorusunun cevabı işlemin tamamı için aynı.
    pub fn discover_pentest_assets_via_dns_bruteforce(
        &self,
        apex_domain: &str,
        subdomain_wordlist: &[String],
    ) -> Result<PentestReconResult, String> {
        if subdomain_wordlist.is_empty() {
            return Err("alt alan adı kelime listesi boş olamaz".into());
        }
        if subdomain_wordlist.len() > MAX_PENTEST_DNS_BRUTEFORCE_WORDS {
            return Err(format!(
                "bir seferde en fazla {MAX_PENTEST_DNS_BRUTEFORCE_WORDS} kelime denenebilir (istenen: {})",
                subdomain_wordlist.len()
            ));
        }
        let active = self.active_verified_pentest_scope()?;
        if active.scope.maximum_mode < PentestMode::Active {
            return Err(format!(
                "subdomain brute-force ACTIVE yetki gerektirir — scope '{}' yalnız '{}' seviyesine kadar izin veriyor",
                active.name,
                active.scope.maximum_mode.as_str()
            ));
        }
        let candidates =
            crate::pentest_recon::build_dns_bruteforce_candidates(apex_domain, subdomain_wordlist);
        let resolved: Vec<String> = candidates
            .into_iter()
            .filter(|candidate| crate::pentest_recon::dns_name_resolves(candidate))
            .collect();
        self.record_pentest_recon_candidates(apex_domain, "dns_bruteforce", resolved)
    }

    /// F7.3 "Aktif keşif: JS analiziyle endpoint keşfi". Port taramasıyla aynı tavan gerekçesi:
    /// hedeften gerçek bir dosya indiriliyor, bu yüzden `PentestMode::Active` talep ediliyor.
    /// Bulunan endpoint YOLLARI (host değil) kalıcı hostname envanterine yazılmıyor — F7.1'in
    /// scope eşleştirmesi host tabanlı, bir yolu ona zorla uydurmak yanlış bir soyutlama olurdu;
    /// bu yalnız çağırana dönen bir sonuç (bkz. `PentestEndpointDiscoveryResult`'ın kendi notu).
    pub fn discover_pentest_endpoints_via_javascript(
        &self,
        target: &str,
        js_path: &str,
    ) -> Result<PentestEndpointDiscoveryResult, String> {
        self.authorize_pentest_action(target, PentestMode::Active)?;
        let source = crate::pentest_recon::fetch_javascript_source(target, js_path)?;
        let endpoints = crate::pentest_recon::extract_endpoint_paths_from_javascript(&source);
        Ok(PentestEndpointDiscoveryResult {
            target: target.to_string(),
            source_path: js_path.to_string(),
            endpoints,
        })
    }

    /// F7.4 "Manuel test araçları: istek yakalama/değiştirme/tekrar gönderme". Aynı tavan
    /// gerekçesi: gerçek bir istek hedefe gönderiliyor, `PentestMode::Active` gerektiriyor.
    /// Kullanıcı/model bunu farklı istek gövdesi/başlıklarla iki kez çağırıp sonuçları
    /// `diff_pentest_http_responses`'a vererek "ne değişti" sorusunu cevaplayabilir — bu, F7.4'ün
    /// en yüksek değerli iş akışı (IDOR/yetki atlatma gibi bulgular genelde tam olarak burada
    /// bulunuyor, otomatik taramayla değil).
    pub fn replay_pentest_http_request(
        &self,
        target: &str,
        request: &PentestHttpRequest,
    ) -> Result<PentestHttpResponse, String> {
        self.authorize_pentest_action(target, PentestMode::Active)?;
        crate::pentest_replay::send_http_request(target, request)
    }

    /// F7.5 için paylaşılan yardımcı: `PentestMode::Safe` seviyesinde hedefe basit bir GET
    /// isteği gönderir. Üç F7.5 kontrolünün (devralma tespiti, teknoloji parmak izi, TLS
    /// bağlantısı) üçü de tek bir isteğe dayanıyor, bu yüzden "yetkilendir + gönder" mantığı
    /// burada bir kez var — F7.4'ün diğer kapsamlı isteklerinden (replay) farklı olarak burada
    /// yalnız `GET /` gönderiliyor, kullanıcı tanımlı bir istek değil. `use_tls`/`port` alanları,
    /// gerçek dünyada her hedefin standart 443/HTTPS'te olmaması VE testlerin gerçek internete
    /// çıkmadan yerel bir sunucuya yönelebilmesi için var — `PentestHttpRequest`'in kendi
    /// tasarımıyla aynı gerekçe.
    fn send_pentest_safe_get(
        &self,
        target: &str,
        path: &str,
        use_tls: bool,
        port: Option<u16>,
    ) -> Result<PentestHttpResponse, String> {
        self.authorize_pentest_action(target, PentestMode::Safe)?;
        let request = PentestHttpRequest {
            method: "GET".into(),
            path: path.to_string(),
            headers: vec![],
            body: vec![],
            use_tls,
            port,
        };
        crate::pentest_replay::send_http_request(target, &request)
    }

    /// F7.5 "Subdomain devralma (takeover) tespiti — yalnız DNS/HTTP kontrolü, sömürü yok."
    /// Hedefin kök sayfasını çekip bilinen bir "terk edilmiş servis" imzasına karşı kontrol
    /// ediyor — hiçbir devralma denemesi yapmıyor, yalnız zafiyetin var olma OLASILIĞINI
    /// bildiriyor (F7.7'nin "bulundu" ile "doğrulandı" ayrımı burada da geçerli).
    pub fn check_pentest_subdomain_takeover(
        &self,
        target: &str,
        use_tls: bool,
        port: Option<u16>,
    ) -> Result<Option<TakeoverSignatureMatch>, String> {
        let response = self.send_pentest_safe_get(target, "/", use_tls, port)?;
        Ok(crate::pentest_safe_checks::detect_takeover_signature(
            &response.body,
        ))
    }

    /// F7.5 "Açığa çıkmış hassas dosya/yanlış yapılandırma tespiti." Bilinen bir yol listesini
    /// dener; tek bir yolun ağ hatası (zaman aşımı vb.) TÜM taramayı iptal etmiyor — o yol
    /// atlanıp devam ediliyor. Yetkilendirme yalnız BİR KEZ, döngünün başında kontrol ediliyor
    /// (scope dışı/iptal edilmiş bir hedefte altı kez aynı reddi denemek yerine).
    pub fn scan_pentest_exposed_files(
        &self,
        target: &str,
        use_tls: bool,
        port: Option<u16>,
    ) -> Result<Vec<PentestExposedFileFinding>, String> {
        self.authorize_pentest_action(target, PentestMode::Safe)?;
        let mut findings = Vec::new();
        for (path, description, content_signature) in
            crate::pentest_safe_checks::sensitive_file_checks()
        {
            if Self::exposed_file_path_is_still_present(
                target,
                path,
                *content_signature,
                use_tls,
                port,
            ) {
                findings.push(PentestExposedFileFinding { path, description });
            }
        }
        Ok(findings)
    }

    /// Bir tek yolu kontrol eder — `scan_pentest_exposed_files`'ın döngüsü ve
    /// `revalidate_pentest_finding`'in hassas yeniden doğrulaması BU tek fonksiyonu paylaşıyor,
    /// aynı mantık iki kez yazılmadı. Ağ hatası (zaman aşımı vb.) sessizce "bulunamadı" sayılır —
    /// çağıranların ikisi de bunu farklı ele alıyor (tarama atlar, yeniden doğrulama da "artık
    /// yok" der; bu, hedefe geçici olarak ulaşılamamasını "düzeltildi" ile karıştırma riski taşır,
    /// ama F7.6'nın kendi notu gereği amaç zaten yalnız "hâlâ var mı" sorusuna hızlı bir cevap).
    fn exposed_file_path_is_still_present(
        target: &str,
        path: &str,
        content_signature: fn(&[u8]) -> bool,
        use_tls: bool,
        port: Option<u16>,
    ) -> bool {
        let request = PentestHttpRequest {
            method: "GET".into(),
            path: path.to_string(),
            headers: vec![],
            body: vec![],
            use_tls,
            port,
        };
        let Ok(response) = crate::pentest_replay::send_http_request(target, &request) else {
            return false;
        };
        response.status == 200 && content_signature(&response.body)
    }

    /// F7.3'ün kalan "teknoloji parmak izi" alt maddesi + F7.5'in CVE eşleştirmesinin ön koşulu.
    /// Yalnız hedefin kendi beyan ettiği başlıkları (`Server`, `X-Powered-By`, vb.) çıkarıyor —
    /// bu bir kanıt değil bir ipucu (bir sunucu başlığını yanlış/eksik beyan edebilir).
    pub fn fingerprint_pentest_technology(
        &self,
        target: &str,
        use_tls: bool,
        port: Option<u16>,
    ) -> Result<PentestTechnologyFingerprint, String> {
        let response = self.send_pentest_safe_get(target, "/", use_tls, port)?;
        Ok(crate::pentest_safe_checks::extract_technology_fingerprint(
            &response.headers,
        ))
    }

    /// F7.5 "TLS/sertifika sorunları." **Dürüst sınır:** yalnız "HTTPS bağlantısı (sertifika
    /// doğrulaması dahil) başarılı oldu mu" sorusuna cevap veriyor — tam olarak hangi sertifika
    /// sorunuyla karşılaşıldığını (süre dolumu/kendinden imzalı/hostname uyuşmazlığı) ayırt
    /// edemiyor; bunun için ham TLS handshake'ine inen yeni bir bağımlılık gerekir, bilinçli
    /// olarak eklenmedi (bkz. `pentest_safe_checks` modül notu). Bu kontrolün amacı gereği
    /// `use_tls` her zaman `true` — `port` yine de özelleştirilebilir (standart olmayan bir
    /// HTTPS portu).
    pub fn check_pentest_tls_connectivity(
        &self,
        target: &str,
        port: Option<u16>,
    ) -> Result<PentestTlsCheckResult, String> {
        self.authorize_pentest_action(target, PentestMode::Safe)?;
        let request = PentestHttpRequest {
            method: "GET".into(),
            path: "/".into(),
            headers: vec![],
            body: vec![],
            use_tls: true,
            port,
        };
        match crate::pentest_replay::send_http_request(target, &request) {
            Ok(_) => Ok(PentestTlsCheckResult {
                tls_connection_succeeded: true,
                failure_detail: None,
            }),
            Err(error) => Ok(PentestTlsCheckResult {
                tls_connection_succeeded: false,
                failure_detail: Some(error),
            }),
        }
    }

    /// F7.6 "Evidence tabanlı finding formatı" + "scope dışı ... hedef deny testleri". Bir
    /// bulguyu kaydetmeden önce hedefin hâlâ aktif scope'un yetkili sınırları içinde olduğu
    /// kontrol ediliyor — JARVIS'in kendi veritabanına yazmak hedefe hiçbir paket göndermese de,
    /// yanlış/hayali bir hedef hakkında bir bulgunun raporlara sızmasını önlemek bu kontrolün işi.
    /// Kaydın kendisi (sır benzeri kanıt reddi, deduplication) `SqliteStore::record_pentest_finding`'de.
    pub fn record_pentest_finding(
        &self,
        target: &str,
        category: &str,
        title: &str,
        evidence: &str,
        severity_estimate: Risk,
        check_parameter: Option<&str>,
    ) -> Result<PentestFinding, String> {
        let active = self.authorize_pentest_action(target, PentestMode::Safe)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "pentest findings require an attached local store".to_string())?;
        store.record_pentest_finding(
            &active.name,
            target,
            category,
            title,
            evidence,
            severity_estimate,
            check_parameter,
        )
    }

    /// F7.6 "İnsan onayı" + F7.7'nin `confirm_finding` sözleşmesi. Hedefin hâlâ scope içinde
    /// olduğunu (kayıt sırasından bu yana scope değişmiş/iptal edilmiş olabilir — defense in
    /// depth, `authorize_pentest_action`'ın her yerde uyguladığı ilkeyle aynı) yeniden kontrol
    /// ediyor, sonra `human_approved: bool` + taze kanıt zorunluluğunu `SqliteStore`'a devrediyor.
    pub fn confirm_pentest_finding(
        &self,
        finding_id: &str,
        confirmation_evidence: &str,
        human_approved: bool,
    ) -> Result<PentestFinding, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "pentest findings require an attached local store".to_string())?;
        let finding = store
            .pentest_finding(finding_id)?
            .ok_or_else(|| format!("'{finding_id}' adında bir bulgu yok"))?;
        self.authorize_pentest_action(&finding.target, PentestMode::Safe)?;
        store.confirm_pentest_finding(finding_id, confirmation_evidence, human_approved)
    }

    /// F7.6: bir bulgunun yanlış pozitif olduğuna insan karar verdiğinde — hiçbir yeniden
    /// yetkilendirme gerekmiyor, silmek/iptal etmek her zaman daha güvenli yöndeki karar.
    pub fn reject_pentest_finding(&self, finding_id: &str) -> Result<(), String> {
        self.store
            .as_ref()
            .ok_or_else(|| "pentest findings require an attached local store".to_string())?
            .reject_pentest_finding(finding_id)
    }

    /// Bir scope'un tüm bulguları — en son kaydedilen önce.
    pub fn pentest_findings(&self, scope_name: &str) -> Result<Vec<PentestFinding>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| "pentest findings require an attached local store".to_string())?
            .pentest_findings(scope_name)
    }

    /// F7.6 "Rapor öncesi yeniden doğrulama" + "Düzeltme sonrası hedefli yeniden test" — aynı
    /// mekanizma, iki farklı zamanlama için (bkz. `PentestFindingRevalidation`'ın kendi notu).
    /// Bulgunun `category`'sine göre F7.5'in İLGİLİ tek kontrolünü tekrar çalıştırıyor —
    /// `scan_pentest_ports`/`scan_pentest_exposed_files` gibi TÜM taramayı değil, yalnız o bulguyu.
    /// Hedefin hâlâ scope içinde olduğu (defense in depth, her yerde uygulanan ilke) yine
    /// kontrol ediliyor.
    pub fn revalidate_pentest_finding(
        &self,
        finding_id: &str,
        use_tls: bool,
        port: Option<u16>,
    ) -> Result<PentestFindingRevalidation, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "pentest findings require an attached local store".to_string())?;
        let finding = store
            .pentest_finding(finding_id)?
            .ok_or_else(|| format!("'{finding_id}' adında bir bulgu yok"))?;
        self.authorize_pentest_action(&finding.target, PentestMode::Safe)?;

        match finding.category.as_str() {
            pentest_safe_checks::FINDING_CATEGORY_SUBDOMAIN_TAKEOVER => {
                let result =
                    self.check_pentest_subdomain_takeover(&finding.target, use_tls, port)?;
                Ok(if result.is_some() {
                    PentestFindingRevalidation::StillPresent
                } else {
                    PentestFindingRevalidation::NoLongerPresent
                })
            }
            pentest_safe_checks::FINDING_CATEGORY_TLS_CONNECTIVITY => {
                let result = self.check_pentest_tls_connectivity(&finding.target, port)?;
                Ok(if result.tls_connection_succeeded {
                    PentestFindingRevalidation::NoLongerPresent
                } else {
                    PentestFindingRevalidation::StillPresent
                })
            }
            pentest_safe_checks::FINDING_CATEGORY_EXPOSED_SENSITIVE_FILE => {
                let Some(path) = finding.check_parameter.as_deref() else {
                    return Err(
                        "bu bulgu için hangi yolun kontrol edileceği kaydedilmemiş (check_parameter boş)"
                            .into(),
                    );
                };
                let Some((_, _, content_signature)) = pentest_safe_checks::sensitive_file_checks()
                    .iter()
                    .find(|(candidate_path, ..)| *candidate_path == path)
                else {
                    return Err(format!(
                        "'{path}' F7.5'in bilinen hassas dosya listesinde değil, yeniden doğrulanamıyor"
                    ));
                };
                let still_present = Self::exposed_file_path_is_still_present(
                    &finding.target,
                    path,
                    *content_signature,
                    use_tls,
                    port,
                );
                Ok(if still_present {
                    PentestFindingRevalidation::StillPresent
                } else {
                    PentestFindingRevalidation::NoLongerPresent
                })
            }
            _ => Ok(PentestFindingRevalidation::CheckNotSupported),
        }
    }

    /// F7.6 "Modelin kendisi raporu yazabilmeli, iyi bir şekilde." Düzyazı bölümlerin kendisi
    /// çağırandan (model) geliyor — bu yalnız iki koruma ekliyor: (1) yalnız `Confirmed`
    /// durumundaki bir bulgu için taslak üretilebilir (bir `Suspected` şüphe için "göndermeye
    /// hazır" bir rapor yazmak, F7.7'nin `confirm_finding` sözleşmesini atlamak olurdu), (2)
    /// döndürülen taslak `validate_pentest_report_draft_completeness`'ten geçmek ZORUNDA — eksik
    /// bir bölümle üretilen bir taslak asla döndürülmüyor.
    pub fn draft_pentest_finding_report(
        &self,
        finding_id: &str,
        summary: &str,
        reproduction_steps: &str,
        impact_analysis: &str,
        suggested_fix: &str,
    ) -> Result<PentestFindingReportDraft, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "pentest findings require an attached local store".to_string())?;
        let finding = store
            .pentest_finding(finding_id)?
            .ok_or_else(|| format!("'{finding_id}' adında bir bulgu yok"))?;
        if finding.status != PentestFindingStatus::Confirmed {
            return Err(format!(
                "yalnız doğrulanmış (confirmed) bulgular için rapor taslağı üretilebilir — '{finding_id}' şu an '{}' durumunda",
                finding.status.as_str()
            ));
        }
        let draft = PentestFindingReportDraft {
            finding_id: finding_id.to_string(),
            summary: summary.to_string(),
            reproduction_steps: reproduction_steps.to_string(),
            impact_analysis: impact_analysis.to_string(),
            suggested_fix: suggested_fix.to_string(),
            severity_estimate: finding.severity_estimate,
        };
        crate::pentest_reporting::validate_pentest_report_draft_completeness(&draft)?;
        Ok(draft)
    }

    /// F7.6 "Program-özel hariç tutulan/düşük değerli bulgu sınıfları filtresi." Bir scope'un tüm
    /// bulgularından, programın kabul etmediği kategorileri (ör. `"self_xss"`) çıkararak "rapora
    /// girecekler" listesini döner. Bulguları SİLMİYOR — envanter olduğu gibi kalıyor, yalnız bu
    /// görünüm süzülüyor (bir programın politikası değişebilir).
    pub fn pentest_findings_for_report(
        &self,
        scope_name: &str,
        excluded_categories: &[String],
    ) -> Result<Vec<PentestFinding>, String> {
        let all = self.pentest_findings(scope_name)?;
        Ok(
            crate::pentest_reporting::filter_findings_by_excluded_categories(
                &all,
                excluded_categories,
            ),
        )
    }

    /// The scope-filtering + persistence half of recon, factored out from the real-network call
    /// above specifically so it is testable without a live HTTP request — mirrors `weather.rs`'s
    /// own split (a thin real-network wrapper around a pure, offline-tested function). Every F7.3
    /// discovery source (certificate transparency, DNS brute-force, and any future one —
    /// technology fingerprinting, historical URL archives) funnels its raw candidate names
    /// through this same method with its own `source` label, so the "in scope vs. out of scope,
    /// new vs. already known" logic exists exactly once regardless of where the names came from
    /// or whether that source itself needed a higher mode ceiling to run.
    pub(crate) fn record_pentest_recon_candidates(
        &self,
        queried_domain: &str,
        source: &str,
        candidates: Vec<String>,
    ) -> Result<PentestReconResult, String> {
        let active = self.active_verified_pentest_scope()?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "pentest recon requires an attached local store".to_string())?;

        let mut in_scope = Vec::new();
        let mut out_of_scope_count = 0usize;
        for candidate in candidates {
            match authorize_pentest_target(&active.scope, &candidate, PentestMode::Safe) {
                Ok(()) => in_scope.push(candidate),
                Err(_) => out_of_scope_count += 1,
            }
        }
        in_scope.sort();
        in_scope.dedup();

        let new_assets = store.record_pentest_assets(&active.name, source, &in_scope)?;

        Ok(PentestReconResult {
            queried_domain: queried_domain.to_string(),
            in_scope_assets: in_scope,
            new_assets,
            out_of_scope_count,
        })
    }

    /// F7.3 "Aktif keşif: port/servis tarama". Unlike the certificate-transparency lookup above,
    /// this genuinely touches the target — a real TCP connect attempt per port — so it goes
    /// through the *same* single authorization entry point every other action-on-a-target uses
    /// (`authorize_pentest_action`), requested at `PentestMode::Active` (not `Safe`): a scope
    /// whose `maximum_mode` is only `Safe` must refuse this, exactly like it refuses any other
    /// active action. No sandboxed worker exists yet to route this through F7.2's SOCKS5 gate
    /// (that wiring is still the deferred item noted in F7.2/F7.3's own plan text) — this method
    /// connects directly, from JARVIS's own process, the same honest scope the certificate-
    /// transparency lookup above already has.
    ///
    /// Bounded on two independent axes so a single call can never turn into an unbounded scan of
    /// someone else's host: `MAX_PENTEST_PORTS_PER_SCAN` caps how many ports one call may probe at
    /// all, and the scope's own `max_runtime_seconds` is enforced as a wall-clock deadline over
    /// the whole loop (checked before every connect attempt) — a scan that is still running when
    /// the authorized time budget runs out stops there, reporting how far it got rather than
    /// silently continuing past what was authorized.
    pub fn scan_pentest_ports(
        &self,
        target: &str,
        ports: &[u16],
    ) -> Result<PentestPortScanResult, String> {
        if ports.is_empty() {
            return Err("taranacak port listesi boş olamaz".into());
        }
        if ports.len() > MAX_PENTEST_PORTS_PER_SCAN {
            return Err(format!(
                "bir seferde en fazla {MAX_PENTEST_PORTS_PER_SCAN} port taranabilir (istenen: {})",
                ports.len()
            ));
        }
        let active = self.authorize_pentest_action(target, PentestMode::Active)?;
        let deadline =
            Instant::now() + Duration::from_secs(active.scope.max_runtime_seconds as u64);
        Ok(Self::scan_pentest_ports_until(target, ports, deadline))
    }

    /// The timed loop itself, factored out from validation/authorization above specifically so
    /// the "stop once the deadline has passed" behavior is directly testable — a caller can pass
    /// an already-expired `Instant` (exactly the way `pentest_network_gate`'s own tests pass an
    /// already-expired deadline into `accept_one`) instead of needing a real port that is
    /// genuinely slow to fail, which no test could make deterministic without depending on real
    /// network timing.
    pub(crate) fn scan_pentest_ports_until(
        target: &str,
        ports: &[u16],
        deadline: Instant,
    ) -> PentestPortScanResult {
        let mut open_ports = Vec::new();
        let mut scanned_port_count = 0usize;
        let mut stopped_early_due_to_runtime_budget = false;
        for &port in ports {
            if Instant::now() >= deadline {
                stopped_early_due_to_runtime_budget = true;
                break;
            }
            scanned_port_count += 1;
            if pentest_tcp_port_is_open(target, port, PENTEST_PORT_CONNECT_TIMEOUT) {
                open_ports.push(port);
            }
        }
        PentestPortScanResult {
            target: target.to_string(),
            open_ports,
            scanned_port_count,
            stopped_early_due_to_runtime_budget,
        }
    }

    /// SHA-256 of the exact system prompt this build sends. This is the "prompt version" the
    /// registry stores: a commit hash would not notice an uncommitted edit, and a version number
    /// would have to be remembered by hand.
    /// Şu anki model + prompt bileşiminin hiç ölçülüp ölçülmediği.
    ///
    /// F6'nın kurduğu ölçüm zincirinin son halkası: bir ölçüm yapıldı ama kimse ona bakmıyorsa
    /// zincir kopuk demektir. Bir model veya prompt değiştiğinde kullanıcının bunu *hatırlaması*
    /// gerekiyordu; hatırlamak insana bırakılan bir disiplin ve kaçınılmaz olarak unutuluyor.
    ///
    /// Prompt parmak izi commit hash'i değil metnin kendisinin SHA-256'sı olduğu için,
    /// commit edilmemiş bir prompt düzenlemesi bile "yeni konfigürasyon" olarak görünüyor —
    /// prompt'u kurcalayıp sonucu ölçmek tam da yapılacak şey.
    pub fn configuration_is_measured(&self, model_id: &str) -> Result<bool, String> {
        let prompt = Self::active_prompt_fingerprint();
        Ok(self.model_config_runs(200)?.iter().any(|run| {
            run.prompt_fingerprint == prompt
                && (run.model_id == model_id || run.model_fingerprint == model_id)
        }))
    }

    /// Ölçülmemiş bir konfigürasyon için kullanıcıya gösterilecek uyarı; ölçülmüşse `None`.
    ///
    /// Uyarı bir *hata* değil, bir bilgi: JARVIS çalışmaya devam ediyor. Ama "bu ayarın nasıl
    /// davrandığını hiç ölçmedik" bilgisi, bir kalite şikayetini değerlendirirken kritik.
    /// Store bağlı değilse sessiz kalıyor — registry olmadan ölçüm zaten mümkün değil ve
    /// kullanıcıyı çözemeyeceği bir şey için uyarmak gürültüdür.
    pub fn unmeasured_configuration_notice(&self, model_id: &str) -> Option<String> {
        match self.configuration_is_measured(model_id) {
            Ok(true) | Err(_) => None,
            Ok(false) => Some(format!(
                "Bu konfigürasyon ({model_id} + güncel sistem prompt'u) hiç ölçülmedi — `/eval` ile golden set'i koşabilirsin."
            )),
        }
    }

    pub fn active_prompt_fingerprint() -> String {
        crate::sha256_hex(crate::model::JARVIS_SYSTEM_PROMPT)
    }

    /// F3 post-close "`/rag status`" (GPT önerisi 4+5/7). A snapshot, not a subscription — cheap
    /// enough to compute on every call (a handful of `COUNT(*)` queries), so there is no separate
    /// metrics store to keep in sync or go stale.
    pub fn rag_status(&self) -> Result<RagStatus, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "RAG status requires an attached local store".to_string())?;
        let embedding_model = self.embedding_status().map(str::to_owned);
        let embedded_chunk_count = match &embedding_model {
            Some(model_id) => store
                .workspace_chunk_embedding_count_for_model(model_id)
                .map_err(|error| error.to_string())?,
            None => 0,
        };
        Ok(RagStatus {
            document_count: store
                .workspace_document_count()
                .map_err(|error| error.to_string())?,
            chunk_count: store
                .workspace_chunk_count()
                .map_err(|error| error.to_string())?,
            embedded_chunk_count,
            embedding_model,
            hybrid_queries_this_session: self.hybrid_queries_this_session,
            fts_only_queries_this_session: self.fts_only_queries_this_session,
        })
    }

    /// F3 post-close "`/rag rebuild`" (GPT önerisi 5/7): recomputes every stored embedding for
    /// the currently-attached model from scratch. Requires an embedding provider — there is
    /// nothing to rebuild in FTS-only mode (`search_workspace` has no derived cache at all).
    pub fn rebuild_rag_index(&mut self) -> Result<usize, String> {
        let provider = self
            .embedding_provider
            .as_deref()
            .ok_or_else(|| "rebuild requires an attached embedding provider".to_string())?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "RAG rebuild requires an attached local store".to_string())?;
        let rebuilt = store.rebuild_embeddings_for_model(provider)?;
        self.record_audit(AuditEvent::pending(
            "workspace-rag",
            format!("workspace.rag.rebuilt:{rebuilt}"),
        ));
        Ok(rebuilt)
    }

    /// F3 post-close "`/rag verify`" (GPT önerisi 5/7): an integrity check the user can run
    /// on demand, and the concrete thing `/rag rebuild` fixes if this comes back unhealthy.
    pub fn verify_rag_index(&self) -> Result<RagVerifyReport, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "RAG verify requires an attached local store".to_string())?;
        let chunk_count = store
            .workspace_chunk_count()
            .map_err(|error| error.to_string())?;
        let embedding_model = self.embedding_status();
        let embedded_chunk_count = match embedding_model {
            Some(model_id) => store
                .workspace_chunk_embedding_count_for_model(model_id)
                .map_err(|error| error.to_string())?,
            None => 0,
        };
        Ok(RagVerifyReport {
            document_count: store
                .workspace_document_count()
                .map_err(|error| error.to_string())?,
            chunk_count,
            embedded_chunk_count,
            orphaned_embedding_count: store
                .workspace_orphaned_embedding_count()
                .map_err(|error| error.to_string())?,
            chunks_missing_embedding: embedding_model.map(|_| chunk_count - embedded_chunk_count),
        })
    }

    /// F3 "Citation UX: ... kaynağı aç davranışı". The citations that grounded the most recent
    /// conversational reply, in ranked order — a caller reads this to let the user open/expand a
    /// specific one (by 1-based position) without needing to re-run retrieval or touch the store.
    pub fn last_workspace_citations(&self) -> &[WorkspaceCitation] {
        &self.last_workspace_citations
    }

    /// Indexes every file `preview_workspace_index` reports as `included` under `approved_root`.
    /// Reuses `index_workspace_document` per file — no second, divergent ingestion path — so
    /// every existing per-file guarantee (secret-name/size/binary/UTF-8 rejection, path
    /// containment, chunking, audit) still applies exactly once, file by file.
    pub fn index_workspace_folder(
        &mut self,
        approved_root: &Path,
        exclude_patterns: &[String],
        user_approved: bool,
    ) -> Result<WorkspaceFolderIndexReport, String> {
        self.index_workspace_folder_with_sensitivity(
            approved_root,
            exclude_patterns,
            DataSensitivity::Internal,
            user_approved,
        )
    }

    /// Same as `index_workspace_folder`, with every file in the folder indexed at the given
    /// sensitivity level (F3 post-close "retrieval öncesi permission/sensitivity filtresi", GPT
    /// önerisi 1/7) — a whole folder ("finans klasörüm") marked `Sensitive` in one command.
    pub fn index_workspace_folder_with_sensitivity(
        &mut self,
        approved_root: &Path,
        exclude_patterns: &[String],
        sensitivity: DataSensitivity,
        user_approved: bool,
    ) -> Result<WorkspaceFolderIndexReport, String> {
        if !user_approved {
            return Err("workspace folder indexing requires explicit user approval".into());
        }
        let preview = preview_workspace_index(approved_root, exclude_patterns)?;
        let mut report = WorkspaceFolderIndexReport::default();
        for relative_path in preview.included {
            match self.index_workspace_document_with_sensitivity(
                approved_root,
                &relative_path,
                sensitivity,
                true,
            ) {
                Ok(ingestion) => report.indexed.push(ingestion),
                Err(error) => report.failed.push((relative_path, error)),
            }
        }
        Ok(report)
    }

    fn approved_workspace_context(&mut self, query: &str) -> Vec<WorkspaceCitation> {
        let Some(store) = self.store.as_ref() else {
            return Vec::new();
        };
        // Every real conversation retrieval goes through the hybrid path; it degrades to plain
        // FTS by itself whenever no embedding provider is attached or the embedding call fails —
        // this is never a hard dependency, so failure here just falls back silently.
        let query_embedding = self.embedding_provider.as_ref().and_then(|provider| {
            provider
                .embed(query)
                .ok()
                .map(|vector| (provider.embedding_model_id().to_owned(), vector))
        });
        // F3 post-close "gözlemlenebilirlik" (GPT önerisi 4/7): the one signal worth tracking
        // cheaply — did this turn actually use the semantic signal, or fall back to plain FTS.
        if query_embedding.is_some() {
            self.hybrid_queries_this_session += 1;
        } else {
            self.fts_only_queries_this_session += 1;
        }
        let query_embedding_ref = query_embedding
            .as_ref()
            .map(|(model_id, vector)| (model_id.as_str(), vector.as_slice()));
        let citations = store
            .hybrid_search_workspace(
                query,
                query_embedding_ref,
                configured_workspace_retrieval_result_limit(),
            )
            .unwrap_or_default();
        // F3 "Retrieval policy: token/context budget". `citations` already comes back best-first
        // and duplicate-suppressed; this is the last, caller-side backstop that keeps the total
        // untrusted text handed to the model in one turn under a fixed ceiling regardless of how
        // many results the result-count limit or a future chunk-size change would otherwise allow.
        let mut budget_remaining = WORKSPACE_CONTEXT_CHAR_BUDGET;
        citations
            .into_iter()
            .take_while(|citation| {
                let cost = citation.content.chars().count();
                if cost > budget_remaining {
                    false
                } else {
                    budget_remaining -= cost;
                    true
                }
            })
            .collect()
    }

    pub fn pending_approvals(&self) -> Vec<&Approval> {
        let mut approvals: Vec<_> = self
            .approvals
            .values()
            .filter(|approval| {
                !approval.approved
                    && approval.expires_at > now_epoch()
                    && self
                        .tasks
                        .get(&approval.task_id)
                        .is_some_and(|task| task.state == TaskState::WaitingForUser)
            })
            .collect();
        approvals.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        approvals
    }

    /// F4 "Yerel üretkenlik tool framework": tam olarak neyin onaylanacağını, onaylamadan önce
    /// gösteriyor — `PolicyControl::ExplainBeforeExecute` şimdiye kadar bildirilen ama hiçbir
    /// zaman uygulanmayan bir kontroldü (yalnız task_id/action_id gösteriliyordu). `None` döner:
    /// task/input bulunamazsa, ya da bu capability için kayıtlı bir `LocalTool` yoksa (henüz
    /// approval-gated her capability bir `LocalTool` değil, ör. F4 coding patch'leri kendi
    /// önizlemesine sahip).
    pub fn preview_pending_action(&self, task_id: &str) -> Option<String> {
        let task = self.tasks.get(task_id)?;
        let input = self.pending_inputs.get(task_id)?;
        let manifest = self.registry.get(&task.capability)?;
        if let Some(tool) = local_tool_for(&manifest.capability_id) {
            return Some(tool.preview(input));
        }
        // These four predate the `LocalTool` refactor (`execute_approved`'s own hardcoded match
        // in `lib.rs`, not the trait dispatch `local_tool_for` covers) — real bug found live
        // (2026-08-16): they had *no* preview at all, so `ExplainBeforeExecute` (their own
        // declared policy control) was silently unenforced for exactly this set, the same gap
        // `preview_pending_action` was built to close for `note.create`/`file.append_note`. A
        // user asking JARVIS to *write* new code, misrouted here by the local model (a real,
        // separately documented router-accuracy issue), had no way to see — before approving —
        // that this would only ever *list* existing files, never write anything new.
        legacy_workspace_read_preview(&manifest.capability_id, input)
    }

    pub fn handle(&mut self, request: Request) -> (Task, ToolResult, VerifierResult) {
        let capability = classify(&request.content).to_owned();
        self.handle_with_resolution(
            request,
            IntentResolution {
                capability: capability.clone(),
                source: if capability == "unknown" {
                    RouteSource::Unknown
                } else {
                    RouteSource::Deterministic
                },
            },
        )
    }

    pub fn handle_with_provider(
        &mut self,
        request: Request,
        provider: &dyn ModelProvider,
    ) -> (Task, ToolResult, VerifierResult) {
        self.handle_with_provider_and_vision(request, provider, None)
    }

    /// Runs optional image analysis before the ordinary text turn. Image bytes are exposed only
    /// to `vision`; its output is subsequently escaped as untrusted data for the text model.
    pub fn handle_with_provider_and_vision(
        &mut self,
        request: Request,
        provider: &dyn ModelProvider,
        vision: Option<&dyn VisionProvider>,
    ) -> (Task, ToolResult, VerifierResult) {
        let analyses = if let Some(vision) = vision {
            let mut analyses = Vec::new();
            for attachment in request
                .attachments
                .iter()
                .filter(|attachment| attachment.kind.is_image())
            {
                match vision.analyze(attachment, &request.content) {
                    Ok(analysis) => analyses.push(analysis),
                    Err(error) => return self.vision_failure(request, &error),
                }
            }
            analyses
        } else {
            vec![]
        };
        self.handle_with_provider_and_analyses(request, provider, &analyses)
    }

    pub(crate) fn vision_failure(
        &mut self,
        request: Request,
        source_error: &str,
    ) -> (Task, ToolResult, VerifierResult) {
        let (mut task, _, _) = self.handle_with_resolution(
            request,
            IntentResolution {
                capability: "conversation.reply".into(),
                source: RouteSource::LocalModel,
            },
        );
        task.state = TaskState::Failed;
        self.tasks.insert(task.task_id.clone(), task.clone());
        self.save_task(&task);
        self.record_audit(AuditEvent::pending(task.task_id.clone(), "vision.failed"));
        let stale_attachment = source_error.contains("queued attachment")
            || source_error.contains("attachment path cannot be resolved")
            || source_error.contains("attachment metadata cannot be read");
        let result = ToolResult {
            status: ToolStatus::Failure,
            output: String::new(),
            error: Some(if stale_attachment {
                "Ek dosya artık erişilebilir değil veya seçildikten sonra değişti. Güvenlik için analiz edilmedi; dosyayı yeniden seçip tekrar gönder."
            } else {
                "Local vision modeli şu an hazır değil veya görsel işlenemedi. Görsel analiz edilmedi; model hazır olduğunda tekrar deneyebilirsin."
            }
            .into()),
            state_changed: false,
            evidence: vec![if stale_attachment {
                "vision.analysis:stale_attachment"
            } else {
                "vision.analysis:unavailable"
            }
            .into()],
        };
        let verification = verify(&result);
        (task, result, verification)
    }

    pub(crate) fn handle_with_provider_and_analyses(
        &mut self,
        request: Request,
        provider: &dyn ModelProvider,
        vision_analyses: &[VisionAnalysis],
    ) -> (Task, ToolResult, VerifierResult) {
        // Free-form user text is never mapped with reply templates or keyword rules. One local
        // model turn produces either natural chat or a narrow intent envelope, so ordinary chat
        // does not pay for a second routing generation. An envelope is still only a proposal:
        // registry, policy and verifier decide whether it can become a governed task.
        self.append_chat_turn("user", request.content.clone());
        let conversation = self.conversation_context();
        let memories = self.approved_memory_context();
        let profile_files = self.profile_file_context();
        let citations = self.approved_workspace_context(&request.content);
        let attachments = request.attachments.clone();
        // RAG chunks, attachment descriptors and vision output are inputs the user may want us
        // to discuss, but they are never allowed to become an authority for a capability. The
        // model is prompted accordingly above; this independent gate keeps that invariant true
        // even when a small local model follows an injected instruction.
        let has_untrusted_model_context =
            !citations.is_empty() || !attachments.is_empty() || !vision_analyses.is_empty();
        let mut model_messages = memories
            .iter()
            .map(|record| ConversationMessage {
                // The provider receives this as a user-data envelope; it is deliberately not
                // a system message and has no authority over tools or policy.
                role: "user",
                content: isolate_memory_as_data(record),
            })
            .collect::<Vec<_>>();
        model_messages.extend(profile_files);
        model_messages.extend(citations.iter().map(|citation| ConversationMessage {
            role: "user",
            content: isolate_untrusted_content(&citation.as_untrusted_content()),
        }));
        model_messages.extend(attachments.iter().map(|attachment| ConversationMessage {
            role: "user",
            content: attachment.untrusted_descriptor(),
        }));
        model_messages.extend(vision_analyses.iter().map(|analysis| ConversationMessage {
            role: "user",
            content: analysis.untrusted_descriptor(),
        }));
        model_messages.extend(self.chat_history.iter().cloned());
        // A short model-based routing pass handles governed requests before the conversational
        // turn. This is learned routing—not a phrase/answer table—and policy/verifier authority
        // remains in the same pipeline. Untrusted attachment/RAG context never enters this pass.
        let (routed_resolution, response) = if !has_untrusted_model_context {
            // Route first, then only generate a conversational reply if it's actually going to be
            // used. Real latency finding (16 Ağustos 2026, gerçek `llama-server`'a karşı ölçüldü):
            // `jarvis-llama.service` runs with `-np 1` (a single decode slot), so two client-side
            // "concurrent" calls to it were never actually overlapping at the model — the server
            // just queued the second one behind the first, and both full passes were paid for on
            // every turn regardless of outcome. A routed capability's conversational reply is
            // discarded below anyway (`task.capability != "conversation.reply"`), so skipping its
            // generation here saves one full extra model pass (measured: ~3.5s of prompt prefill
            // alone for the router call on this CPU-only 8B model) whenever a capability is
            // actually routed, with no change in behavior for ordinary chat (still needs both the
            // route check and the reply).
            // `chat_history` already includes the current turn (appended at the top of this
            // function) — exclude it here and pass only what came *before*, kept to the last
            // couple of messages so the router's per-turn cost stays bounded.
            let history_end = self.chat_history.len().saturating_sub(1);
            let history_start = history_end.saturating_sub(2);
            let recent_history = &self.chat_history[history_start..history_end];
            let routed = Some(route_with_provider(
                &request.content,
                recent_history,
                &self.registry,
                provider,
            ))
            .filter(|resolution| resolution.capability != "unknown")
            // Real bug found live (2026-08-16): a casual continuation like "hadi yaz bekliyorum"
            // (referring to a script the user was promised in conversation, not a note) was
            // getting classified as `note.create` — the router prompt *does* say coding
            // discussion should stay conversational, but an 8B local model is not perfectly
            // reliable at that distinction. `note.create`'s own content extraction (`note_body`)
            // requires a colon-delimited payload after the trigger phrase; ordinary conversation
            // essentially never has one, so a genuine `note.create` command ("not al: X") always
            // has real content and a misfire almost never does — treating a content-less
            // `note.create` exactly like `unknown` here (before the conversational reply is
            // skipped for latency, see the comment above) makes a misfire fall through to a real
            // conversational answer instead of silently creating an empty placeholder note.
            .filter(|resolution| {
                resolution.capability != "note.create" || note_body_is_present(&request.content)
            });
            let response = if routed.is_none() {
                provider
                    .converse_messages(&model_messages)
                    .or_else(|_| provider.converse(&conversation))
                    .ok()
            } else {
                None
            };
            (routed, response)
        } else {
            (
                None,
                provider
                    .converse_messages(&model_messages)
                    .or_else(|_| provider.converse(&conversation))
                    .ok(),
            )
        };
        let proposed_capability = routed_resolution
            .as_ref()
            .map(|resolution| resolution.capability.clone())
            .or_else(|| {
                response
                    .as_ref()
                    .and_then(|response| model_capability_intent(&response.text, &self.registry))
            })
            // Same guard as the `routed_resolution` filter above, applied here too since this is
            // a second, independent place a `note.create` classification can come from (an
            // embedded `<jarvis-intent>` tag inside an otherwise-normal conversational reply).
            .filter(|capability| {
                capability != "note.create" || note_body_is_present(&request.content)
            });
        let proposed_source = routed_resolution
            .as_ref()
            .map(|resolution| resolution.source)
            .unwrap_or(RouteSource::LocalModel);
        let suppress_untrusted_model_intent =
            has_untrusted_model_context && proposed_capability.is_some();
        let resolution = proposed_capability
            .filter(|_| !suppress_untrusted_model_intent)
            .map(|capability| IntentResolution {
                capability,
                source: proposed_source,
            })
            .unwrap_or_else(|| IntentResolution {
                capability: "conversation.reply".into(),
                source: RouteSource::LocalModel,
            });
        let (task, mut result, verification) = self.handle_with_resolution(request, resolution);
        if task.capability == "conversation.reply" {
            if suppress_untrusted_model_intent {
                result.output = UNTRUSTED_MODEL_INTENT_SUPPRESSED.into();
                result.evidence = vec!["conversation.reply:untrusted-intent-suppressed".into()];
                self.append_chat_turn("assistant", result.output.clone());
                self.record_audit(AuditEvent::pending(
                    task.task_id.clone(),
                    "model_intent.suppressed_untrusted_context",
                ));
            } else if let Some(response) =
                response.filter(|response| !response.text.trim().is_empty())
            {
                result.output = response.text;
                result.evidence = vec!["conversation.reply:local-model".into()];
                self.append_chat_turn("assistant", result.output.clone());
            }
        } else {
            self.append_chat_turn("assistant", result.output.clone());
        }
        for memory in memories {
            // Workspace citations already surface as visible evidence ("why was this used");
            // memory context only ever reached the audit log, invisible during normal use. This
            // gives it the same visible attribution — the key/namespace, not the value, so the
            // source line never duplicates a long or sensitive value inline.
            result.evidence.push(format!(
                "memory.used:{}:{}",
                memory.namespace.as_str(),
                memory.key
            ));
            self.record_audit(AuditEvent::pending(
                task.task_id.clone(),
                format!("memory.retrieved:{}", memory.memory_id),
            ));
        }
        for citation in &citations {
            result.evidence.push(format!(
                "workspace.citation:{}#chunk-{}",
                citation.canonical_path.display(),
                citation.chunk_ordinal
            ));
            self.record_audit(AuditEvent::pending(
                task.task_id.clone(),
                format!("workspace.retrieved:{}", citation.chunk_id),
            ));
        }
        // F3 "Citation UX": kept separately from `evidence` (which is a generic, string-only
        // trail shared by every capability) so a caller can show the full "kaynağı aç" content
        // for this exact reply without re-querying the store or losing the chunk text.
        self.last_workspace_citations = citations;
        for attachment in attachments {
            self.record_audit(AuditEvent::pending(
                task.task_id.clone(),
                format!("attachment.retrieved:{}", attachment.attachment_id),
            ));
        }
        for analysis in vision_analyses {
            result
                .evidence
                .push(format!("vision.analysis:{}", analysis.attachment_id));
            self.record_audit(AuditEvent::pending(
                task.task_id.clone(),
                format!("vision.analyzed:{}", analysis.attachment_id),
            ));
        }
        (task, result, verification)
    }

    /// Maps a narrowly typed MCP tool call to a registered desktop capability. The adapter never
    /// accepts a free-form capability name and therefore cannot bypass the registry or policy.
    pub fn handle_mcp(&mut self, ingress: McpIngressRequest) -> (Task, ToolResult, VerifierResult) {
        let content = match ingress.tool_id.as_str() {
            "jarvis.system.health" => "system health".into(),
            "jarvis.system.time" => "system time".into(),
            "jarvis.file.read_workspace" => format!("dosya oku: {}", ingress.argument),
            "jarvis.project.info" => "proje bilgisi".into(),
            "jarvis.code.project_outline" => "code.project_outline".into(),
            "jarvis.docs.workspace_summary" => "docs.workspace_summary".into(),
            "jarvis.note.create" => format!("not oluştur: {}", ingress.argument),
            _ => format!("unknown mcp tool: {}", ingress.tool_id),
        };
        self.handle(Request {
            schema_version: ingress.schema_version,
            request_id: ingress.request_id,
            input_type: InputType::Mcp,
            content,
            attachments: vec![],
        })
    }

    fn handle_with_resolution(
        &mut self,
        request: Request,
        resolution: IntentResolution,
    ) -> (Task, ToolResult, VerifierResult) {
        let capability = resolution.capability.as_str();
        let task_id = format!("task-{}", request.request_id);
        let mut task = Task {
            task_id: task_id.clone(),
            request_id: request.request_id.clone(),
            state: TaskState::Queued,
            capability: capability.to_owned(),
        };
        if let Err(reason) = validate_request(&request) {
            task.state = TaskState::Failed;
            let result = ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(reason.clone()),
                state_changed: false,
                evidence: vec![],
            };
            let verification = VerifierResult {
                status: VerifyStatus::Fail,
                reason,
                evidence: vec![],
            };
            self.record_audit(AuditEvent::pending(task_id.clone(), "request.invalid"));
            self.tasks.insert(task_id, task.clone());
            self.save_task(&task);
            return (task, result, verification);
        }
        self.record_audit(AuditEvent::pending(task_id.clone(), "task.queued"));
        self.record_audit(AuditEvent::pending(
            task_id.clone(),
            format!("intent.{:?}", resolution.source),
        ));
        let policy = if self.registry.contains(capability) {
            policy_for(capability, &request.content)
        } else {
            PolicyResult {
                decision: PolicyDecision::Deny,
                risk: Risk::High,
                reason: "capability is not registered".into(),
                approval_required: false,
                required_controls: vec![PolicyControl::AuditRequired],
            }
        };
        self.record_audit(AuditEvent::pending(
            task_id.clone(),
            format!("policy.{:?}", policy.decision),
        ));
        if policy.decision != PolicyDecision::Allow {
            task.state = if policy.decision == PolicyDecision::AskUser {
                TaskState::WaitingForUser
            } else {
                TaskState::Failed
            };
            let result = ToolResult {
                status: ToolStatus::Failure,
                output: String::new(),
                error: Some(policy.reason.clone()),
                state_changed: false,
                evidence: vec![],
            };
            let verification = VerifierResult {
                status: VerifyStatus::Fail,
                reason: policy.reason,
                evidence: vec![],
            };
            self.record_audit(AuditEvent::pending(task_id.clone(), "task.blocked"));
            if policy.decision == PolicyDecision::AskUser {
                let approval = Approval {
                    approval_id: format!("approval-{}", task_id),
                    task_id: task_id.clone(),
                    action_id: capability.to_owned(),
                    approved: false,
                    expires_at: now_epoch() + 900,
                    scope_hash: approval_scope_hash(&task_id, capability, &request.content),
                };
                self.pending_inputs
                    .insert(task_id.clone(), request.content.clone());
                if let Some(store) = &self.store {
                    store
                        .save_approval(&approval)
                        .expect("approval persistence must succeed");
                }
                self.approvals.insert(task_id.clone(), approval);
            }
            self.tasks.insert(task_id, task.clone());
            self.save_task(&task);
            return (task, result, verification);
        }
        task.state = TaskState::Running;
        let result = self
            .registry
            .get(capability)
            .map(|manifest| execute_read_only(manifest, &request.content))
            .unwrap_or_else(|| sandbox_violation("registered capability manifest is missing"));
        let verification = verify(&result);
        task.state = if verification.status == VerifyStatus::Pass {
            TaskState::Completed
        } else {
            TaskState::Failed
        };
        self.record_audit(AuditEvent::pending(task_id.clone(), "tool.executed"));
        self.record_audit(AuditEvent::pending(
            task_id.clone(),
            format!("verify.{:?}", verification.status),
        ));
        self.tasks.insert(task_id, task.clone());
        self.save_task(&task);
        (task, result, verification)
    }

    /// Approves and resumes exactly one waiting task. Approval never grants a broader scope.
    pub fn approve(&mut self, task_id: &str) -> Option<(Task, ToolResult, VerifierResult)> {
        // A typed approval is the strongest channel available, so the origin gate below is a
        // no-op for it. Voice callers must use `approve_from` and accept its refusal.
        self.approve_from(task_id, InputType::Cli)
    }

    /// F5 "Sesli approval UX". Same approval path, but aware of *how* the confirmation arrived.
    ///
    /// This exists because an unenforced rule is only documentation: `approval_channel_requirement`
    /// states that speech alone may not authorize an action the policy gate already gated, and
    /// this is the single place that refuses to proceed when it does not. A refusal is audited —
    /// an attempt to approve a restricted action by voice is exactly the event a later review
    /// would want to see, whether it came from the user, someone else in the room, or a replayed
    /// recording.
    pub fn approve_from(
        &mut self,
        task_id: &str,
        origin: InputType,
    ) -> Option<(Task, ToolResult, VerifierResult)> {
        {
            let task = self.tasks.get(task_id)?;
            let input = self.pending_inputs.get(task_id)?;
            if approval_channel_requirement(&task.capability, input, origin)
                == ApprovalChannelRequirement::WrittenConfirmationRequired
            {
                self.record_audit(AuditEvent::pending(
                    task_id,
                    "approval.channel_insufficient",
                ));
                return None;
            }
        }
        self.approve_checked(task_id)
    }

    fn approve_checked(&mut self, task_id: &str) -> Option<(Task, ToolResult, VerifierResult)> {
        let task = self.tasks.get(task_id)?.clone();
        if task.state != TaskState::WaitingForUser {
            return None;
        }
        let approval = self.approvals.get_mut(task_id)?;
        if approval.approved {
            return None;
        }
        if approval.expires_at <= now_epoch() {
            self.record_audit(AuditEvent::pending(task_id, "approval.expired"));
            return None;
        }
        let input = self.pending_inputs.get(task_id)?.clone();
        if approval.scope_hash != approval_scope_hash(task_id, &approval.action_id, &input) {
            self.record_audit(AuditEvent::pending(task_id, "approval.scope_mismatch"));
            return None;
        }
        approval.approved = true;
        if let Some(store) = &self.store {
            store
                .save_approval(approval)
                .expect("approval persistence must succeed");
        }
        self.record_audit(AuditEvent::pending(task_id, "approval.granted"));
        let input = self.pending_inputs.remove(task_id)?;
        let result = self
            .registry
            .get(&task.capability)
            .map(|manifest| {
                // An approval may authorize either a restricted persistent action or a
                // privacy-sensitive read. The manifest still selects the only execution path;
                // approval never expands the capability or its sandbox profile.
                if manifest.sandbox_profile == "NO_EXEC_READ_ONLY" {
                    execute_read_only(manifest, &input)
                } else {
                    execute_approved(manifest, &input, task_id)
                }
            })
            .unwrap_or_else(|| sandbox_violation("registered capability manifest is missing"));
        let verification = verify(&result);
        let mut resumed = task;
        resumed.state = if verification.status == VerifyStatus::Pass {
            TaskState::Completed
        } else {
            TaskState::Failed
        };
        self.record_audit(AuditEvent::pending(task_id, "tool.executed"));
        self.record_audit(AuditEvent::pending(
            task_id,
            format!("verify.{:?}", verification.status),
        ));
        self.tasks.insert(task_id.into(), resumed.clone());
        self.save_task(&resumed);
        Some((resumed, result, verification))
    }

    /// Cancels a task before an approved side effect starts. Running tools are intentionally not
    /// cancelled here: this synchronous MVP has no worker/process handle to terminate safely.
    pub fn cancel(&mut self, task_id: &str) -> Option<Task> {
        let mut task = self.tasks.get(task_id)?.clone();
        if !matches!(task.state, TaskState::Queued | TaskState::WaitingForUser) {
            return None;
        }
        task.state = TaskState::Cancelled;
        self.pending_inputs.remove(task_id);
        self.record_audit(AuditEvent::pending(task_id, "task.cancelled"));
        self.tasks.insert(task_id.into(), task.clone());
        self.save_task(&task);
        Some(task)
    }

    /// F4 "Patch apply transaction": applies one approved patch (workbench does the actual
    /// snapshot/apply/rollback-on-apply-failure work — see `apply_approved_patch`) and records the
    /// one audit event that pure library function has no way to write itself (it has no `Runtime`
    /// to write to). This is the only place in the crate that turns a reviewed diff into a real
    /// change on disk.
    pub fn apply_coding_patch(
        &mut self,
        plan: &CodingPlan,
        proposal: &PatchProposal,
        approval: &ApprovedPatch,
    ) -> Result<PatchApplication, String> {
        let application = apply_approved_patch(plan, proposal, approval)?;
        self.record_audit(AuditEvent::pending(
            format!("patch-{}", proposal.proposal_id),
            "coding.patch.applied",
        ));
        Ok(application)
    }

    /// F4 "Test/verifier runner", `application`/`baseline`/`post_patch`/`regressions`/`kept`
    /// display fields kept together for the caller (originally `PatchApplication` itself, but its
    /// `snapshot` is consumed internally by finalize — see below).
    pub fn apply_coding_patch_with_regression_check(
        &mut self,
        plan: &CodingPlan,
        proposal: &PatchProposal,
        approval: &ApprovedPatch,
        cancel: Option<&CancelFlag>,
    ) -> Result<(RegressionCheckedPatch, Result<(), String>), String> {
        // F4 "Test/verifier runner"'ın belgelenmiş bilinen sınırı (16 Ağustos 2026 ilk turda): bir
        // test komutu patch'ten TAMAMEN bağımsız olarak zaten bozuksa, sistem bunu patch'in kendi
        // hatasıyla ayırt edemiyordu — her ikisi de "testler geçmedi, geri al" olarak işleniyordu.
        // Düzeltme: patch uygulanmadan ÖNCE aynı test planı bir kez "taban çizgisi" olarak
        // çalıştırılıyor; yalnız taban çizgisinde GEÇEN ama patch sonrası BAŞARISIZ olan komutlar
        // gerçek bir "regresyon" sayılıyor. Taban çizgisinde zaten başarısız olan bir komut patch
        // sonrası da başarısızsa, bu artık patch'e karşı kullanılmıyor — değişiklik kalıcı kalır
        // (audit'e "önceden var olan hata tolere edildi" olarak dürüstçe yazılır).
        let baseline = run_test_plan(&plan.workspace_root, &plan.test_plan, &plan.limits, cancel);
        let application = self.apply_coding_patch(plan, proposal, approval)?;
        let post_patch = run_test_plan(&plan.workspace_root, &plan.test_plan, &plan.limits, cancel);

        let mut regressions = Vec::new();
        for post_run in &post_patch.ran {
            if post_run.succeeded() {
                continue;
            }
            let was_passing_before = baseline
                .ran
                .iter()
                .find(|baseline_run| baseline_run.command_line() == post_run.command_line())
                .map(CommandRun::succeeded)
                // Taban çizgisinde hiç eşleşen bir komut yoksa (ör. skip edildiyse) temkinli
                // davranılıyor: "zaten bozuktu" varsayılmıyor, patch sonrası başarısızlık
                // regresyon sayılıyor — güvenlik tarafı hataya düşsün, sessizce tolere etmesin.
                .unwrap_or(true);
            if was_passing_before {
                regressions.push(post_run.command_line());
            }
        }
        let cancelled = post_patch
            .ran
            .iter()
            .any(|run| run.stopped == Some(WorkerStopReason::UserCancelled));
        // `TestRunReport::all_ran_passed`'ın kendi kuralıyla aynı: hiçbir komut gerçekten
        // çalışmadıysa (`ran` boş — hepsi skip edildiyse ya da test planı boşsa) "doğrulandı"
        // sayılmıyor, temkinli varsayılan geri almaktır.
        let anything_ran = !post_patch.ran.is_empty();
        let kept = anything_ran && regressions.is_empty() && !cancelled;

        let proposal_id = application.proposal_id.clone();
        let changed_files = application.changed_files.clone();
        let verifier_evidence = application.verifier_evidence.clone();
        let baseline_had_failures = baseline.ran.iter().any(|run| !run.succeeded());
        let finalize = if kept {
            self.record_audit(AuditEvent::pending(
                format!("patch-{proposal_id}"),
                "coding.tests.passed",
            ));
            if baseline_had_failures {
                self.record_audit(AuditEvent::pending(
                    format!("patch-{proposal_id}"),
                    "coding.tests.pre_existing_failure_tolerated",
                ));
            }
            discard_patch_snapshot(application.snapshot)
        } else {
            let event_name = if cancelled {
                "coding.tests.cancelled"
            } else if !regressions.is_empty() {
                "coding.tests.regression_detected"
            } else {
                "coding.tests.failed"
            };
            self.record_audit(AuditEvent::pending(
                format!("patch-{proposal_id}"),
                event_name,
            ));
            let restore = restore_patch_snapshot(&application.snapshot);
            let cleanup = discard_patch_snapshot(application.snapshot);
            self.record_audit(AuditEvent::pending(
                format!("patch-{proposal_id}"),
                "coding.patch.rolled_back_after_test_outcome",
            ));
            match (restore, cleanup) {
                (Err(restore_error), _) => Err(format!(
                    "tests did not pass and automatic rollback also failed: {restore_error}"
                )),
                (_, Err(cleanup_error)) => Err(format!(
                    "tests did not pass; workspace was restored but snapshot cleanup failed: {cleanup_error}"
                )),
                _ => Ok(()),
            }
        };
        Ok((
            RegressionCheckedPatch {
                proposal_id,
                changed_files,
                verifier_evidence,
                baseline,
                post_patch,
                regressions,
                kept,
            },
            finalize,
        ))
    }
}

/// `Runtime::preview_pending_action`'s fallback for the four read-only workspace capabilities
/// that predate the `LocalTool` refactor. Each description is deliberately honest about scope —
/// `code.project_outline` in particular spells out "lists existing files, never writes new code",
/// directly addressing the misfire this was built to surface: the local model routing a "write me
/// some code" request here instead of replying conversationally (a real, separately documented
/// router-accuracy issue, not something this preview can fix on its own — but it lets the user
/// *see and reject* the mistake before it executes, instead of finding out only after approving).
fn legacy_workspace_read_preview(capability_id: &str, input: &str) -> Option<String> {
    match capability_id {
        "file.read_workspace" => {
            let path = input
                .split_once(':')
                .map(|(_, path)| path.trim())
                .filter(|path| !path.is_empty());
            Some(match path {
                Some(path) => format!(
                    "Workspace içindeki şu dosyayı okuyup gösterecek (yeni bir şey yazmaz): {path}"
                ),
                None => "Workspace içinde bir dosya okuyacak, ama hangi dosya belirtilmemiş — muhtemelen başarısız olacak (bu yeni kod YAZMAZ, yalnız var olan bir dosyayı okur).".into(),
            })
        }
        "project.info" => Some(
            "Bu projenin kök dizini, Cargo.toml/README.md varlığı gibi genel bilgilerini gösterecek — yeni bir şey yazmaz.".into(),
        ),
        "code.project_outline" => Some(
            "src/ altındaki var olan .rs dosyalarının bir listesini gösterecek. Yeni kod YAZMAZ — yalnız zaten var olanları listeler.".into(),
        ),
        "docs.workspace_summary" => Some(
            "Proje kökündeki README.md dosyasının içeriğini gösterecek — yeni bir şey yazmaz.".into(),
        ),
        _ => None,
    }
}

/// Display-oriented result of `Runtime::apply_coding_patch_with_regression_check` — not
/// `PatchApplication` itself, because its `snapshot` field is consumed internally by the
/// keep-or-rollback decision before this is ever returned.
#[derive(Debug, Clone)]
pub struct RegressionCheckedPatch {
    pub proposal_id: String,
    pub changed_files: Vec<PathBuf>,
    pub verifier_evidence: Vec<String>,
    pub baseline: TestRunReport,
    pub post_patch: TestRunReport,
    /// Command lines that passed at the pre-patch baseline but failed after the patch was
    /// applied — the only failures actually attributed to the patch itself.
    pub regressions: Vec<String>,
    pub kept: bool,
}

/// A real TCP connect attempt — `true` only if the connection genuinely succeeded within
/// `timeout`. DNS/parse failures and connection failures are both treated as "not open" rather
/// than propagated as errors: a port scan's job is to report state per port, not to abort the
/// whole scan because one hostname briefly failed to resolve.
fn pentest_tcp_port_is_open(target: &str, port: u16, timeout: Duration) -> bool {
    let Ok(mut addrs) = (target, port).to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, timeout).is_ok()
}
