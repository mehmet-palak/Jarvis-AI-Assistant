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
use std::path::Path;

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
        Self {
            store: Some(store),
            registry: CapabilityRegistry::baseline(),
            audit_sequence,
            audit_hash,
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
        self.chat_history
            .push(ConversationMessage { role, content });
        if role == "assistant" {
            while self.chat_history.len() > MAX_COMPLETED_CHAT_HISTORY_TURNS {
                self.chat_history.drain(0..2);
            }
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
                store
                    .retrieve_memory(
                        &[
                            MemoryNamespace::UserProfile,
                            MemoryNamespace::Project,
                            MemoryNamespace::Task,
                            MemoryNamespace::Session,
                            MemoryNamespace::EphemeralToolOutput,
                        ],
                        8,
                    )
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
        if !user_approved {
            return Err("workspace indexing requires explicit user approval".into());
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "workspace indexing requires an attached local store".to_string())?;
        let outcome = store.index_workspace_document_with_embedding(
            approved_root,
            relative_path,
            self.embedding_provider.as_deref(),
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

    /// Whether workspace retrieval is currently hybrid (FTS + embedding) or FTS-only. Surfaced to
    /// the user (`/status`) so hybrid mode is never a silent, invisible behavior change.
    pub fn embedding_status(&self) -> Option<&str> {
        self.embedding_provider
            .as_ref()
            .map(|provider| provider.embedding_model_id())
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
        if !user_approved {
            return Err("workspace folder indexing requires explicit user approval".into());
        }
        let preview = preview_workspace_index(approved_root, exclude_patterns)?;
        let mut report = WorkspaceFolderIndexReport::default();
        for relative_path in preview.included {
            match self.index_workspace_document(approved_root, &relative_path, true) {
                Ok(ingestion) => report.indexed.push(ingestion),
                Err(error) => report.failed.push((relative_path, error)),
            }
        }
        Ok(report)
    }

    fn approved_workspace_context(&self, query: &str) -> Vec<WorkspaceCitation> {
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
        let query_embedding_ref = query_embedding
            .as_ref()
            .map(|(model_id, vector)| (model_id.as_str(), vector.as_slice()));
        store
            .hybrid_search_workspace(query, query_embedding_ref, 4)
            .unwrap_or_default()
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
            // Both calls are read-only model inference. Running them concurrently keeps a
            // normal conversation from paying routing latency plus chat latency on CPU-only
            // hosts; a governed route simply discards the unneeded conversational result.
            std::thread::scope(|scope| {
                let route =
                    scope.spawn(|| route_with_provider(&request.content, &self.registry, provider));
                let conversation = scope.spawn(|| {
                    provider
                        .converse_messages(&model_messages)
                        .or_else(|_| provider.converse(&conversation))
                        .ok()
                });
                let routed = route
                    .join()
                    .ok()
                    .filter(|resolution| resolution.capability != "unknown");
                let response = conversation.join().ok().flatten();
                (routed, response)
            })
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
        for citation in citations {
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
