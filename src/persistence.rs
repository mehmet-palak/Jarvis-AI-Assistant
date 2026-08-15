//! Durable task, approval, audit, teacher-example, memory and workspace-RAG storage.
//!
//! `SqliteStore` is the crate's single persistence boundary. Every write here goes through typed
//! contracts (`Task`, `Approval`, `AuditEvent`, `TeacherExample`, `MemoryRecord`,
//! `WorkspaceIngestionReport`) and, where the architecture requires it, through the same
//! validation the rest of the crate uses (`validate_teacher_example`, `validate_memory_record`,
//! workspace path/content checks) before anything reaches SQLite.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, Result as SqlResult, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::{
    chunk_workspace_text, cosine_similarity, deserialize_embedding, extract_pdf_text, fts_query,
    now_epoch, reject_oversized_workspace_document, reject_secret_like_workspace_document_content,
    reject_secret_like_workspace_document_name, serialize_embedding, sha256_hex,
    validate_memory_record, validate_teacher_example, validate_workspace_document_content,
    validate_workspace_document_path, Approval, AuditEvent, CapabilityRegistry, DataSensitivity,
    EmbeddingProvider, MemoryNamespace, MemoryProposal, MemoryRecord, Task, TeacherExample,
    WorkspaceCitation, WorkspaceIngestionReport, CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION,
    MIN_RELEVANT_SIMILARITY,
};

pub(crate) fn audit_hash(sequence: u64, previous_hash: &str, task_id: &str, event: &str) -> String {
    let mut hasher = Sha256::new();
    for field in [
        sequence.to_string(),
        previous_hash.to_owned(),
        task_id.to_owned(),
        event.to_owned(),
    ] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Durable task and audit storage for the implementation baseline.
#[derive(Debug)]
pub struct SqliteStore {
    connection: Connection,
}

/// Highest `schema_migrations.version` this build knows how to apply. Keep in sync with the
/// last `INSERT OR IGNORE INTO schema_migrations` row in `migrate()`.
const CURRENT_SCHEMA_VERSION: i64 = 8;

impl SqliteStore {
    pub fn open(path: &str) -> SqlResult<Self> {
        Self::backup_if_schema_migration_pending(path);
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    /// F3 "Memory migration/backup ... rollback": this project has no down-migrations (SQLite
    /// `ALTER TABLE ADD COLUMN` isn't easily reversible, and a single-user local app doesn't
    /// warrant that machinery). Instead, if `path` already exists and its on-disk schema is
    /// older than this build's, the file is copied to a timestamped sibling *before* `migrate()`
    /// touches it — "rollback" means restoring that file. A brand-new or already-current database
    /// is never backed up here, so this doesn't add a backup on every normal startup. Best-effort:
    /// any failure here (missing permissions, race) never blocks opening the real store.
    fn backup_if_schema_migration_pending(path: &str) {
        if !Path::new(path).is_file() {
            return;
        }
        let Ok(probe) = Connection::open(path) else {
            return;
        };
        let on_disk_version: i64 = probe
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if on_disk_version >= CURRENT_SCHEMA_VERSION {
            return;
        }
        // Reuse the same safe, atomic `VACUUM INTO` primitive `backup_to` already uses instead
        // of a raw file copy, which could race a mid-write journal/WAL file.
        let probe_store = Self { connection: probe };
        let backup_path = PathBuf::from(format!("{path}.pre-migration-backup-{}.db", now_epoch()));
        let _ = probe_store.backup_to(&backup_path);
    }

    pub fn in_memory() -> SqlResult<Self> {
        let connection = Connection::open_in_memory()?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> SqlResult<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tasks (
                task_id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                state TEXT NOT NULL,
                capability TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                event TEXT NOT NULL,
                event_sequence INTEGER NOT NULL DEFAULT 0,
                previous_hash TEXT NOT NULL DEFAULT '',
                event_hash TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS approvals (
                approval_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                action_id TEXT NOT NULL,
                approved INTEGER NOT NULL,
                expires_at INTEGER NOT NULL DEFAULT 0,
                scope_hash TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS teacher_examples (
                example_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                prompt TEXT NOT NULL,
                expected_capability TEXT NOT NULL,
                response TEXT NOT NULL,
                evidence TEXT NOT NULL,
                verifier_status TEXT NOT NULL,
                provenance TEXT NOT NULL,
                human_reviewed INTEGER NOT NULL,
                sensitivity TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memories (
                memory_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                namespace TEXT NOT NULL,
                memory_key TEXT NOT NULL,
                memory_value TEXT NOT NULL,
                sensitivity TEXT NOT NULL,
                source TEXT NOT NULL,
                include_in_model_context INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                expires_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS workspace_documents (
                document_id TEXT PRIMARY KEY,
                canonical_path TEXT NOT NULL UNIQUE,
                content_sha256 TEXT NOT NULL,
                indexed_at INTEGER NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS workspace_chunks USING fts5(
                chunk_id UNINDEXED,
                document_id UNINDEXED,
                chunk_ordinal UNINDEXED,
                content
            );
            CREATE TABLE IF NOT EXISTS workspace_chunk_embeddings (
                chunk_id TEXT PRIMARY KEY,
                document_id TEXT NOT NULL,
                content_sha256 TEXT NOT NULL,
                embedding_model_id TEXT NOT NULL,
                embedding BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chat_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );",
        )?;
        self.ensure_approval_column("expires_at", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_approval_column("scope_hash", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_audit_column("event_sequence", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_audit_column("previous_hash", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_audit_column("event_hash", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column(
            "workspace_documents",
            "index_schema_version",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        self.backfill_legacy_audit_chain()?;
        if self.repair_concurrent_audit_chain()? {
            self.append_audit_chain(
                "system-audit-recovery",
                "audit.recovered.concurrent_sequence",
            )?;
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (1, 'initial task/audit/approval schema')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (2, 'teacher example governance schema')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (3, 'SHA-256 audit hash chain')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (4, 'controlled memory records')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (5, 'workspace FTS document index')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (6, 'workspace document index schema version tracking')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (7, 'workspace chunk embeddings for hybrid RAG')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (8, 'persistent chat history')",
            [],
        )?;
        Ok(())
    }

    fn ensure_approval_column(&self, name: &str, definition: &str) -> SqlResult<()> {
        self.ensure_column("approvals", name, definition)
    }

    fn ensure_audit_column(&self, name: &str, definition: &str) -> SqlResult<()> {
        self.ensure_column("audit_events", name, definition)
    }

    fn ensure_column(&self, table: &str, name: &str, definition: &str) -> SqlResult<()> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        if columns.filter_map(Result::ok).any(|column| column == name) {
            return Ok(());
        }
        self.connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {name} {definition}"),
            [],
        )?;
        Ok(())
    }

    fn backfill_legacy_audit_chain(&self) -> SqlResult<()> {
        let incomplete: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE event_sequence=0 OR event_hash=''",
            [],
            |row| row.get(0),
        )?;
        if incomplete == 0 {
            return Ok(());
        }
        let rows = {
            let mut statement = self
                .connection
                .prepare("SELECT id, task_id, event FROM audit_events ORDER BY id ASC")?;
            let mapped = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            mapped.collect::<SqlResult<Vec<_>>>()?
        };
        let mut previous_hash = "GENESIS".to_owned();
        for (index, (id, task_id, event)) in rows.into_iter().enumerate() {
            let sequence = (index + 1) as u64;
            let event_hash = audit_hash(sequence, &previous_hash, &task_id, &event);
            self.connection.execute(
                "UPDATE audit_events SET event_sequence=?1, previous_hash=?2, event_hash=?3 WHERE id=?4",
                params![sequence as i64, previous_hash, event_hash, id],
            )?;
            previous_hash = event_hash;
        }
        Ok(())
    }

    /// Repairs only the known multi-process race shape: duplicate event sequences. The original
    /// rows and insertion order remain intact, while their sequence/hash links are rebuilt in
    /// durable insertion order. A tampered chain without duplicate sequences is not auto-repaired
    /// and still fails the integrity gate. The caller adds a visible recovery audit event.
    fn repair_concurrent_audit_chain(&mut self) -> SqlResult<bool> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let duplicate_sequences: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM (SELECT event_sequence FROM audit_events GROUP BY event_sequence HAVING COUNT(*) > 1)",
            [],
            |row| row.get(0),
        )?;
        if duplicate_sequences == 0 {
            return Ok(false);
        }
        let rows = {
            let mut statement = transaction
                .prepare("SELECT id, task_id, event FROM audit_events ORDER BY id ASC")?;
            let mapped = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            mapped.collect::<SqlResult<Vec<_>>>()?
        };
        let mut previous_hash = "GENESIS".to_owned();
        for (index, (id, task_id, event)) in rows.into_iter().enumerate() {
            let sequence = (index + 1) as u64;
            let event_hash = audit_hash(sequence, &previous_hash, &task_id, &event);
            transaction.execute(
                "UPDATE audit_events SET event_sequence=?1, previous_hash=?2, event_hash=?3 WHERE id=?4",
                params![sequence as i64, previous_hash, event_hash, id],
            )?;
            previous_hash = event_hash;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub(crate) fn save_task(&self, task: &Task) -> SqlResult<()> {
        self.connection.execute(
            "INSERT INTO tasks(task_id, request_id, state, capability) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(task_id) DO UPDATE SET state=excluded.state, capability=excluded.capability",
            params![task.task_id, task.request_id, task.state.as_str(), task.capability],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn append_audit(&self, event: &AuditEvent) -> SqlResult<()> {
        self.connection.execute(
            "INSERT INTO audit_events(task_id, event, event_sequence, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event.task_id, event.event, event.sequence as i64, event.previous_hash, event.event_hash],
        )?;
        Ok(())
    }

    /// Allocates and persists the next audit event while holding SQLite's write lock. Runtime
    /// instances can exist in both the TUI and native desktop process, so a cached tail is not a
    /// safe source of sequence numbers. The transaction reads the current tail after acquiring
    /// the lock, then inserts one event and commits the matching hash in the same critical section.
    pub(crate) fn append_audit_chain(
        &mut self,
        task_id: &str,
        event: &str,
    ) -> SqlResult<AuditEvent> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (sequence, previous_hash) = transaction
            .query_row(
                "SELECT event_sequence, event_hash FROM audit_events ORDER BY event_sequence DESC, id DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
            )
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok((0, "GENESIS".into())),
                error => Err(error),
            })?;
        let sequence = sequence + 1;
        let event_hash = audit_hash(sequence, &previous_hash, task_id, event);
        transaction.execute(
            "INSERT INTO audit_events(task_id, event, event_sequence, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![task_id, event, sequence as i64, previous_hash, event_hash],
        )?;
        transaction.commit()?;
        Ok(AuditEvent {
            task_id: task_id.into(),
            event: event.into(),
            sequence,
            previous_hash,
            event_hash,
        })
    }

    pub(crate) fn save_approval(&self, approval: &Approval) -> SqlResult<()> {
        self.connection.execute(
            "INSERT INTO approvals(approval_id, task_id, action_id, approved, expires_at, scope_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(approval_id) DO UPDATE SET approved=excluded.approved, expires_at=excluded.expires_at, scope_hash=excluded.scope_hash",
            params![approval.approval_id, approval.task_id, approval.action_id, approval.approved, approval.expires_at as i64, approval.scope_hash],
        )?;
        Ok(())
    }

    /// Persists only a reviewable, verifier-passing example. No unverified model completion may
    /// become training data through this API.
    pub fn append_teacher_example(
        &self,
        example: &TeacherExample,
        registry: &CapabilityRegistry,
    ) -> Result<(), String> {
        validate_teacher_example(example, registry)?;
        self.connection
            .execute(
                "INSERT INTO teacher_examples(
                    example_id, schema_version, prompt, expected_capability, response, evidence,
                    verifier_status, provenance, human_reviewed, sensitivity
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    example.example_id,
                    example.schema_version,
                    example.prompt,
                    example.expected_capability,
                    example.response,
                    example.evidence.join("\n"),
                    "PASS",
                    example.provenance,
                    example.human_reviewed,
                    example.sensitivity.as_str(),
                ],
            )
            .map_err(|error| format!("teacher example persistence failed: {error}"))?;
        Ok(())
    }

    pub fn task_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
    }

    pub fn audit_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
    }

    pub fn audit_chain_is_valid(&self) -> SqlResult<bool> {
        let rows = {
            let mut statement = self.connection.prepare(
                "SELECT task_id, event, event_sequence, previous_hash, event_hash FROM audit_events ORDER BY event_sequence ASC, id ASC",
            )?;
            let mapped = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            mapped.collect::<SqlResult<Vec<_>>>()?
        };
        let mut previous_hash = "GENESIS".to_owned();
        for (expected_sequence, (task_id, event, sequence, stored_previous, stored_hash)) in
            rows.into_iter().enumerate()
        {
            let expected_sequence = (expected_sequence + 1) as u64;
            if sequence != expected_sequence || stored_previous != previous_hash {
                return Ok(false);
            }
            let expected_hash = audit_hash(sequence, &previous_hash, &task_id, &event);
            if stored_hash != expected_hash {
                return Ok(false);
            }
            previous_hash = stored_hash;
        }
        Ok(true)
    }

    pub(crate) fn audit_tail(&self) -> SqlResult<(u64, String)> {
        self.connection.query_row(
            "SELECT event_sequence, event_hash FROM audit_events ORDER BY event_sequence DESC, id DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
        ).or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok((0, "GENESIS".into())),
            error => Err(error),
        })
    }

    pub fn teacher_example_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM teacher_examples", [], |row| {
                row.get(0)
            })
    }

    /// Commits a proposed memory record only after the caller has obtained an explicit user
    /// decision. Normal conversation handling never calls this method implicitly.
    pub fn commit_memory_proposal(
        &self,
        proposal: &MemoryProposal,
        user_approved: bool,
    ) -> Result<MemoryRecord, String> {
        if !user_approved {
            return Err("memory write requires explicit user approval".into());
        }
        validate_memory_record(&proposal.record)?;
        self.connection
            .execute(
                "INSERT INTO memories(
                    memory_id, schema_version, namespace, memory_key, memory_value, sensitivity,
                    source, include_in_model_context, created_at, updated_at, expires_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(memory_id) DO UPDATE SET
                    memory_key=excluded.memory_key,
                    memory_value=excluded.memory_value,
                    sensitivity=excluded.sensitivity,
                    source=excluded.source,
                    include_in_model_context=excluded.include_in_model_context,
                    updated_at=excluded.updated_at,
                    expires_at=excluded.expires_at",
                params![
                    proposal.record.memory_id,
                    proposal.record.schema_version,
                    proposal.record.namespace.as_str(),
                    proposal.record.key,
                    proposal.record.value,
                    proposal.record.sensitivity.as_str(),
                    proposal.record.source,
                    proposal.record.include_in_model_context,
                    proposal.record.created_at as i64,
                    now_epoch() as i64,
                    proposal.record.expires_at.map(|value| value as i64),
                ],
            )
            .map_err(|error| format!("memory persistence failed: {error}"))?;
        let mut record = proposal.record.clone();
        record.updated_at = now_epoch();
        Ok(record)
    }

    /// Indexes one explicitly approved, contained UTF-8 document. Secret-like and binary files
    /// are rejected before any content enters SQLite. Re-indexing the same path replaces its old
    /// chunks, so retrieval can never cite stale content for that path. FTS-only — no embedding
    /// provider, so `search_workspace` results for this content stay keyword-only.
    pub fn index_workspace_document(
        &self,
        approved_root: &Path,
        relative_path: &Path,
    ) -> Result<WorkspaceIngestionReport, String> {
        self.index_workspace_document_with_embedding(approved_root, relative_path, None)
    }

    /// Same as `index_workspace_document`, but if `embedding_provider` is given, also computes
    /// and stores a per-chunk embedding for hybrid retrieval. Best-effort: if the embedding
    /// service is unreachable or errors on a chunk, that chunk's embedding is simply skipped —
    /// FTS indexing of the same chunk still succeeds, this can never fail the whole ingestion.
    /// A chunk whose exact content (by SHA-256) was already embedded anywhere else in the
    /// workspace reuses that stored vector instead of calling the embedding model again — this is
    /// what keeps "only the changed chunks get re-embedded" true across re-indexing and across
    /// files that happen to share identical boilerplate.
    pub fn index_workspace_document_with_embedding(
        &self,
        approved_root: &Path,
        relative_path: &Path,
        embedding_provider: Option<&dyn EmbeddingProvider>,
    ) -> Result<WorkspaceIngestionReport, String> {
        let canonical_path = validate_workspace_document_path(approved_root, relative_path)?;
        let metadata = fs::metadata(&canonical_path)
            .map_err(|error| format!("workspace document metadata failed: {error}"))?;
        if !metadata.is_file() {
            return Err("workspace document must be a regular file".into());
        }
        let bytes = fs::read(&canonical_path)
            .map_err(|error| format!("workspace document read failed: {error}"))?;
        let is_pdf = canonical_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
        let content = if is_pdf {
            // A PDF is inherently binary, so it takes the shared secret-name/size checks but
            // skips the plain-text path's binary/UTF-8 rejection — extraction handles that.
            reject_secret_like_workspace_document_name(&canonical_path)?;
            reject_oversized_workspace_document(&bytes)?;
            let extracted = extract_pdf_text(&bytes)?;
            reject_secret_like_workspace_document_content(&extracted)?;
            extracted
        } else {
            validate_workspace_document_content(&canonical_path, &bytes)?;
            String::from_utf8(bytes)
                .map_err(|error| format!("workspace document must be UTF-8 text: {error}"))?
        };
        let chunks = chunk_workspace_text(&content);
        if chunks.is_empty() {
            return Err("workspace document has no indexable text".into());
        }
        let canonical_path_text = canonical_path.to_string_lossy().into_owned();
        let document_id = format!("document-{}", &sha256_hex(&canonical_path_text)[..16]);
        let content_sha256 = sha256_hex(&content);

        // Incremental re-index: if this exact path is already indexed with byte-for-byte
        // identical extracted content, skip the chunk delete/re-insert churn entirely and report
        // the existing state as unchanged, rather than redoing work indexing didn't need to redo.
        let existing_state: Option<(String, i64, i64)> = self
            .connection
            .query_row(
                "SELECT content_sha256, indexed_at, index_schema_version FROM workspace_documents WHERE canonical_path=?1",
                [&canonical_path_text],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            )
            .ok();
        if let Some((existing_sha256, existing_indexed_at, existing_index_schema_version)) =
            &existing_state
        {
            // A version bump forces re-indexing even for byte-identical content: the *derived*
            // chunks could be stale relative to a changed chunking/extraction algorithm.
            if existing_sha256 == &content_sha256
                && *existing_index_schema_version
                    >= i64::from(CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION)
            {
                // Text is unchanged, but an embedding provider may have been attached *after*
                // this document was last indexed (exactly today's situation: files were indexed
                // FTS-only, embeddings added later). Backfill missing vectors for this model
                // without touching the FTS chunk text at all.
                if let Some(provider) = embedding_provider {
                    if !self.document_fully_embedded_for_model(
                        &document_id,
                        provider.embedding_model_id(),
                    ) {
                        self.backfill_missing_embeddings(provider, &document_id);
                    }
                }
                return Ok(WorkspaceIngestionReport {
                    schema_version: 1,
                    document_id,
                    canonical_path,
                    content_sha256,
                    chunk_count: chunks.len(),
                    indexed_at: *existing_indexed_at as u64,
                    content_changed: false,
                });
            }
        }

        let indexed_at = now_epoch();
        self.connection
            .execute(
                "INSERT INTO workspace_documents(document_id, canonical_path, content_sha256, indexed_at, index_schema_version)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(canonical_path) DO UPDATE SET
                    document_id=excluded.document_id,
                    content_sha256=excluded.content_sha256,
                    indexed_at=excluded.indexed_at,
                    index_schema_version=excluded.index_schema_version",
                params![
                    document_id,
                    canonical_path_text,
                    content_sha256,
                    indexed_at as i64,
                    i64::from(CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION)
                ],
            )
            .map_err(|error| format!("workspace document persistence failed: {error}"))?;
        self.connection
            .execute(
                "DELETE FROM workspace_chunks WHERE document_id=?1",
                [&document_id],
            )
            .map_err(|error| format!("workspace index cleanup failed: {error}"))?;
        // Derived cache, not a data source: old embeddings for this document's previous chunk
        // set are dropped alongside the chunks themselves, so a shrunk document never leaves
        // orphaned vectors behind.
        self.connection
            .execute(
                "DELETE FROM workspace_chunk_embeddings WHERE document_id=?1",
                [&document_id],
            )
            .map_err(|error| format!("workspace embedding cleanup failed: {error}"))?;
        let mut inserted_chunks: Vec<(String, String)> = Vec::with_capacity(chunks.len());
        for (ordinal, chunk) in chunks.iter().enumerate() {
            let chunk_id = format!("chunk-{document_id}-{ordinal}");
            self.connection
                .execute(
                    "INSERT INTO workspace_chunks(chunk_id, document_id, chunk_ordinal, content)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![chunk_id, document_id, ordinal as i64, chunk],
                )
                .map_err(|error| format!("workspace chunk persistence failed: {error}"))?;
            inserted_chunks.push((chunk_id, chunk.clone()));
        }
        // All chunks are inserted before any embedding call — embedding happens once, in one
        // batch, after the loop, rather than interleaved one-call-per-chunk inside it.
        if let Some(provider) = embedding_provider {
            self.embed_and_store_chunks_batch(provider, &document_id, &inserted_chunks);
        }
        Ok(WorkspaceIngestionReport {
            schema_version: 1,
            document_id,
            canonical_path,
            content_sha256,
            chunk_count: chunks.len(),
            indexed_at,
            content_changed: true,
        })
    }

    pub fn search_workspace(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WorkspaceCitation>, String> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let query = fts_query(query)?;
        let rows = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT chunks.chunk_id, chunks.document_id, documents.canonical_path,
                            documents.content_sha256, chunks.chunk_ordinal, chunks.content
                     FROM workspace_chunks AS chunks
                     JOIN workspace_documents AS documents ON documents.document_id=chunks.document_id
                     WHERE workspace_chunks MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )
                .map_err(|error| format!("workspace search setup failed: {error}"))?;
            let mapped = statement
                .query_map(params![query, limit.min(16) as i64], |row| {
                    Ok(WorkspaceCitation {
                        chunk_id: row.get(0)?,
                        document_id: row.get(1)?,
                        canonical_path: PathBuf::from(row.get::<_, String>(2)?),
                        content_sha256: row.get(3)?,
                        chunk_ordinal: row.get::<_, i64>(4)? as usize,
                        content: row.get(5)?,
                    })
                })
                .map_err(|error| format!("workspace search query failed: {error}"))?;
            mapped
                .collect::<SqlResult<Vec<_>>>()
                .map_err(|error| format!("workspace search row failed: {error}"))?
        };
        Ok(rows)
    }

    /// True if every chunk currently belonging to `document_id` already has a stored embedding
    /// from `embedding_model_id`. Lets re-indexing skip embedding work only when it is genuinely
    /// already done for *this* model — not just "some embedding, from some model, exists".
    fn document_fully_embedded_for_model(
        &self,
        document_id: &str,
        embedding_model_id: &str,
    ) -> bool {
        let chunk_count: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM workspace_chunks WHERE document_id=?1",
                [document_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if chunk_count == 0 {
            return true; // nothing to embed
        }
        let embedded_count: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM workspace_chunk_embeddings WHERE document_id=?1 AND embedding_model_id=?2",
                params![document_id, embedding_model_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        embedded_count >= chunk_count
    }

    /// Embeds every existing chunk of `document_id` that does not yet have a stored embedding
    /// from `provider`'s model — used both for freshly-chunked documents and to retroactively
    /// backfill documents that were indexed before an embedding provider was ever attached (F3
    /// madde 13: adding embeddings today must not require the user to notice and force a
    /// re-index of everything they already indexed this session).
    fn backfill_missing_embeddings(&self, provider: &dyn EmbeddingProvider, document_id: &str) {
        let model_id = provider.embedding_model_id();
        let rows: Vec<(String, String)> = {
            let Ok(mut statement) = self.connection.prepare(
                "SELECT chunk_id, content FROM workspace_chunks WHERE document_id=?1
                 AND chunk_id NOT IN (
                     SELECT chunk_id FROM workspace_chunk_embeddings WHERE embedding_model_id=?2
                 )",
            ) else {
                return;
            };
            let Ok(mapped) = statement.query_map(params![document_id, model_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) else {
                return;
            };
            mapped.filter_map(Result::ok).collect()
        };
        self.embed_and_store_chunks_batch(provider, document_id, &rows);
    }

    /// Embeds a set of chunks and stores their vectors, in as few model calls as possible.
    /// Content-hash reuse (**and the same model** — a different embedding model is a different,
    /// incomparable vector space) is checked per chunk first, exactly like before batching
    /// existed; identical content is only ever embedded once even *within* this same batch
    /// (`content_by_hash` dedup below), not just across separate calls. Whatever remains after
    /// reuse is sent to the model in a single `embed_batch` call rather than one round-trip per
    /// chunk — F3 post-close "batch embedding", a real speed win on `/index-folder` with many
    /// files. Never returns an error — a failed/unreachable embedding service only means these
    /// chunks stay FTS-only, it must never fail the document's own (already-successful) FTS
    /// indexing.
    fn embed_and_store_chunks_batch(
        &self,
        provider: &dyn EmbeddingProvider,
        document_id: &str,
        chunks: &[(String, String)],
    ) {
        let model_id = provider.embedding_model_id();
        let mut content_by_hash: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut pending: Vec<(&str, String)> = Vec::new(); // (chunk_id, content_sha256)
        for (chunk_id, content) in chunks {
            let content_sha256 = sha256_hex(content);
            let reused: Option<Vec<u8>> = self
                .connection
                .query_row(
                    "SELECT embedding FROM workspace_chunk_embeddings
                     WHERE content_sha256=?1 AND embedding_model_id=?2 LIMIT 1",
                    params![content_sha256, model_id],
                    |row| row.get(0),
                )
                .ok();
            match reused {
                Some(embedding_bytes) => {
                    let _ = self.connection.execute(
                        "INSERT OR REPLACE INTO workspace_chunk_embeddings(chunk_id, document_id, content_sha256, embedding_model_id, embedding)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![chunk_id, document_id, content_sha256, model_id, embedding_bytes],
                    );
                }
                None => {
                    content_by_hash
                        .entry(content_sha256.clone())
                        .or_insert_with(|| content.clone());
                    pending.push((chunk_id, content_sha256));
                }
            }
        }
        if pending.is_empty() {
            return;
        }
        let unique_hashes: Vec<&String> = content_by_hash.keys().collect();
        let texts: Vec<&str> = unique_hashes
            .iter()
            .map(|hash| content_by_hash[hash.as_str()].as_str())
            .collect();
        let Ok(vectors) = provider.embed_batch(&texts) else {
            return; // best-effort: FTS indexing of these chunks already succeeded
        };
        if vectors.len() != unique_hashes.len() {
            return; // malformed/mismatched response — degrade to FTS-only for this batch
        }
        let vector_by_hash: std::collections::HashMap<&str, Vec<u8>> = unique_hashes
            .into_iter()
            .zip(vectors)
            .map(|(hash, vector)| (hash.as_str(), serialize_embedding(&vector)))
            .collect();
        for (chunk_id, content_sha256) in &pending {
            if let Some(embedding_bytes) = vector_by_hash.get(content_sha256.as_str()) {
                let _ = self.connection.execute(
                    "INSERT OR REPLACE INTO workspace_chunk_embeddings(chunk_id, document_id, content_sha256, embedding_model_id, embedding)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![chunk_id, document_id, content_sha256, model_id, embedding_bytes],
                );
            }
        }
    }

    fn chunk_embedding(&self, chunk_id: &str, embedding_model_id: &str) -> Option<Vec<f32>> {
        self.connection
            .query_row(
                "SELECT embedding FROM workspace_chunk_embeddings WHERE chunk_id=?1 AND embedding_model_id=?2",
                params![chunk_id, embedding_model_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .ok()
            .map(|bytes| deserialize_embedding(&bytes))
    }

    /// FTS + embedding hybrid retrieval via Reciprocal Rank Fusion: both signals contribute to
    /// the final order, neither is "primary" with the other as a fallback. `query_embedding` is
    /// `None` whenever the caller has no reachable embedding provider — in that case this
    /// degrades to exactly `search_workspace`'s plain FTS order (still duplicate-suppressed). A
    /// chunk without a stored embedding *from the same model* (indexed before an embedding
    /// provider was ever attached, or left over from a since-swapped model) still participates
    /// via its FTS rank alone; it is never penalized to zero. `query_embedding` carries its own
    /// model id so a stored vector from a different model can never be compared against it by
    /// mistake.
    ///
    /// F3 "Retrieval policy": also enforces the parts of that policy that belong at the ranking
    /// layer, not the caller — relevance threshold (`MIN_RELEVANT_SIMILARITY`) and duplicate
    /// suppression (identical chunk text kept once, highest-ranked occurrence only). `limit` is
    /// a ceiling on the *count* returned, never a guarantee that many will actually come back —
    /// a query with no genuinely relevant match yields fewer results, down to zero, rather than
    /// padding out to `limit` with weak matches. This is the concrete backstop behind "kaynağı
    /// olmayan cevabı engelleme": a caller can never receive a citation dressed up as a source
    /// when nothing actually cleared the relevance bar.
    pub fn hybrid_search_workspace(
        &self,
        query: &str,
        query_embedding: Option<(&str, &[f32])>,
        limit: usize,
    ) -> Result<Vec<WorkspaceCitation>, String> {
        if limit == 0 {
            return Ok(vec![]);
        }
        // A broader FTS candidate pool gives the embedding signal something real to re-rank;
        // re-ranking a single top-1 FTS hit would be a no-op.
        let candidates = self.search_workspace(query, (limit * 4).clamp(limit, 16))?;
        let ranked = match query_embedding {
            None => candidates,
            Some(_) if candidates.is_empty() => candidates,
            Some((embedding_model_id, query_embedding)) => {
                const RRF_K: f64 = 60.0;
                let mut scored: Vec<(f64, Option<f32>, WorkspaceCitation)> = candidates
                    .into_iter()
                    .enumerate()
                    .map(|(fts_rank, citation)| {
                        let similarity = self
                            .chunk_embedding(&citation.chunk_id, embedding_model_id)
                            .map(|stored| cosine_similarity(query_embedding, &stored));
                        (1.0 / (RRF_K + (fts_rank + 1) as f64), similarity, citation)
                    })
                    .collect();
                let mut similarity_order: Vec<usize> = (0..scored.len()).collect();
                similarity_order.sort_by(|&a, &b| {
                    let similarity_a = scored[a].1.unwrap_or(f32::NEG_INFINITY);
                    let similarity_b = scored[b].1.unwrap_or(f32::NEG_INFINITY);
                    similarity_b
                        .partial_cmp(&similarity_a)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                for (embedding_rank, &index) in similarity_order.iter().enumerate() {
                    if scored[index].1.is_some_and(f32::is_finite) {
                        scored[index].0 += 1.0 / (RRF_K + (embedding_rank + 1) as f64);
                    }
                }
                // Relevance threshold: a stored-but-weak vector disqualifies the chunk even
                // though it FTS-matched; a chunk with no stored vector at all is exempt (its FTS
                // match is still the whole story for it, unchanged from before this policy).
                scored.retain(|(_, similarity, _)| {
                    similarity.is_none_or(|value| value >= MIN_RELEVANT_SIMILARITY)
                });
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                scored
                    .into_iter()
                    .map(|(_, _, citation)| citation)
                    .collect()
            }
        };
        // Duplicate suppression: identical chunk text (the same content indexed under more than
        // one document/path) adds no new information the second time and only spends context
        // budget for nothing — keep the first, highest-ranked occurrence only.
        let mut seen_content = HashSet::new();
        Ok(ranked
            .into_iter()
            .filter(|citation| seen_content.insert(citation.content.clone()))
            .take(limit)
            .collect())
    }

    pub fn workspace_document_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM workspace_documents", [], |row| {
                row.get(0)
            })
    }

    /// User-requested (2026-08-16): conversation history no longer has to live only in RAM.
    /// `role` is always `"user"` or `"assistant"` (`Runtime::append_chat_turn`'s own two call
    /// sites); nothing else ever calls this. Best-effort by design at the call site — a failed
    /// write here must never fail the actual conversation turn.
    pub fn append_chat_message(&self, role: &str, content: &str) -> SqlResult<()> {
        self.connection.execute(
            "INSERT INTO chat_messages(role, content, created_at) VALUES (?1, ?2, ?3)",
            params![role, content, now_epoch() as i64],
        )?;
        Ok(())
    }

    /// The most recent `limit` messages, oldest-first — ready to load straight into
    /// `Runtime.chat_history` on startup so a new session picks up where the last one left off.
    pub fn recent_chat_messages(&self, limit: usize) -> SqlResult<Vec<(String, String)>> {
        let mut statement = self
            .connection
            .prepare("SELECT role, content FROM chat_messages ORDER BY id DESC LIMIT ?1")?;
        let mut rows: Vec<(String, String)> = statement
            .query_map([limit as i64], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<SqlResult<Vec<_>>>()?;
        rows.reverse();
        Ok(rows)
    }

    /// Deletes every row except the most recent `keep` — keeps on-disk storage bounded exactly
    /// the same way `Runtime.chat_history`'s in-memory cap already is, so persistence can never
    /// grow the table without limit.
    pub fn prune_chat_messages_to(&self, keep: usize) -> SqlResult<()> {
        self.connection.execute(
            "DELETE FROM chat_messages WHERE id NOT IN (
                SELECT id FROM chat_messages ORDER BY id DESC LIMIT ?1
             )",
            [keep as i64],
        )?;
        Ok(())
    }

    /// A real, hard delete of every persisted chat message — what `/clear` now does in addition
    /// to clearing the TUI's visible list and `Runtime.chat_history`, so "clear" is an actual
    /// reset rather than only a cosmetic one now that history survives a restart.
    pub fn clear_chat_messages(&self) -> SqlResult<usize> {
        self.connection.execute("DELETE FROM chat_messages", [])
    }

    pub fn chat_message_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get(0))
    }

    /// Returns records that are still valid and explicitly opted into model context. This is a
    /// retrieval API, not an implicit prompt mutation: callers decide when to include it.
    pub fn retrieve_memory(
        &self,
        namespaces: &[MemoryNamespace],
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, String> {
        if namespaces.is_empty() || limit == 0 {
            return Ok(vec![]);
        }
        let now = now_epoch() as i64;
        let rows = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT memory_id, schema_version, namespace, memory_key, memory_value,
                            sensitivity, source, include_in_model_context, created_at, updated_at,
                            expires_at
                     FROM memories
                     WHERE include_in_model_context=1
                       AND (expires_at IS NULL OR expires_at > ?1)
                     ORDER BY updated_at DESC, memory_id ASC",
                )
                .map_err(|error| format!("memory retrieval setup failed: {error}"))?;
            let mapped = statement
                .query_map([now], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u16>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, bool>(7)?,
                        row.get::<_, i64>(8)? as u64,
                        row.get::<_, i64>(9)? as u64,
                        row.get::<_, Option<i64>>(10)?.map(|value| value as u64),
                    ))
                })
                .map_err(|error| format!("memory retrieval query failed: {error}"))?;
            mapped
                .collect::<SqlResult<Vec<_>>>()
                .map_err(|error| format!("memory retrieval row failed: {error}"))?
        };
        let records = rows
            .into_iter()
            .filter_map(
                |(
                    memory_id,
                    schema_version,
                    namespace,
                    key,
                    value,
                    sensitivity,
                    source,
                    include_in_model_context,
                    created_at,
                    updated_at,
                    expires_at,
                )| {
                    let namespace = MemoryNamespace::from_str(&namespace).ok()?;
                    if !namespaces.contains(&namespace) {
                        return None;
                    }
                    let sensitivity = DataSensitivity::from_str(&sensitivity).ok()?;
                    Some(MemoryRecord {
                        schema_version,
                        memory_id,
                        namespace,
                        key,
                        value,
                        sensitivity,
                        source,
                        include_in_model_context,
                        created_at,
                        updated_at,
                        expires_at,
                    })
                },
            )
            .take(limit.min(64))
            .collect::<Vec<_>>();
        Ok(records)
    }

    /// Lists all user-visible memory records, including expired and model-context-disabled items,
    /// so the user can inspect and delete every value JARVIS retains.
    pub fn list_memory(&self) -> Result<Vec<MemoryRecord>, String> {
        let rows = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT memory_id, schema_version, namespace, memory_key, memory_value,
                            sensitivity, source, include_in_model_context, created_at, updated_at,
                            expires_at
                     FROM memories ORDER BY updated_at DESC, memory_id ASC",
                )
                .map_err(|error| format!("memory list setup failed: {error}"))?;
            let mapped = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u16>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, bool>(7)?,
                        row.get::<_, i64>(8)? as u64,
                        row.get::<_, i64>(9)? as u64,
                        row.get::<_, Option<i64>>(10)?.map(|value| value as u64),
                    ))
                })
                .map_err(|error| format!("memory list query failed: {error}"))?;
            mapped
                .collect::<SqlResult<Vec<_>>>()
                .map_err(|error| format!("memory list row failed: {error}"))?
        };
        rows.into_iter()
            .map(
                |(
                    memory_id,
                    schema_version,
                    namespace,
                    key,
                    value,
                    sensitivity,
                    source,
                    include_in_model_context,
                    created_at,
                    updated_at,
                    expires_at,
                )| {
                    Ok(MemoryRecord {
                        schema_version,
                        memory_id,
                        namespace: MemoryNamespace::from_str(&namespace)?,
                        key,
                        value,
                        sensitivity: DataSensitivity::from_str(&sensitivity)?,
                        source,
                        include_in_model_context,
                        created_at,
                        updated_at,
                        expires_at,
                    })
                },
            )
            .collect()
    }

    pub fn memory_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
    }

    pub fn delete_memory(&self, memory_id: &str) -> Result<bool, String> {
        if memory_id.trim().is_empty() {
            return Err("memory id is required".into());
        }
        self.connection
            .execute("DELETE FROM memories WHERE memory_id=?1", [memory_id])
            .map(|changed| changed > 0)
            .map_err(|error| format!("memory deletion failed: {error}"))
    }

    pub fn delete_memory_namespace(&self, namespace: MemoryNamespace) -> Result<usize, String> {
        self.connection
            .execute(
                "DELETE FROM memories WHERE namespace=?1",
                [namespace.as_str()],
            )
            .map_err(|error| format!("memory namespace deletion failed: {error}"))
    }

    pub fn forget_all_memory(&self) -> Result<usize, String> {
        self.connection
            .execute("DELETE FROM memories", [])
            .map_err(|error| format!("full memory deletion failed: {error}"))
    }

    pub fn schema_version(&self) -> SqlResult<i64> {
        self.connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
    }

    pub fn recover_interrupted_tasks(&self) -> SqlResult<usize> {
        self.connection.execute(
            "UPDATE tasks SET state='INTERRUPTED' WHERE state='RUNNING'",
            [],
        )
    }

    /// Creates a transaction-consistent SQLite snapshot. The destination must not already exist,
    /// so a backup operation can never overwrite an earlier recovery point.
    pub fn backup_to(&self, destination: &std::path::Path) -> SqlResult<()> {
        if destination.exists() {
            return Err(rusqlite::Error::InvalidPath(destination.to_path_buf()));
        }
        let destination = destination.to_string_lossy();
        self.connection
            .execute("VACUUM INTO ?1", [destination.as_ref()])?;
        Ok(())
    }

    /// Crate-internal, test-only escape hatch: some integrity tests need to corrupt raw rows
    /// (for example a tampered audit event) to prove `audit_chain_is_valid` actually detects it.
    /// Production code never reaches into the raw connection; every real write goes through a
    /// typed method above.
    #[cfg(test)]
    pub(crate) fn raw_connection(&self) -> &Connection {
        &self.connection
    }

    pub fn task_state(&self, task_id: &str) -> SqlResult<Option<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT state FROM tasks WHERE task_id=?1")?;
        let mut rows = statement.query([task_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }
}
