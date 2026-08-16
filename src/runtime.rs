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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
            .filter(|resolution| resolution.capability != "unknown");
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
}
