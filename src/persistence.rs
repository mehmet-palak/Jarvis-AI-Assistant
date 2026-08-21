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

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::{
    chunk_workspace_text, configured_retrieval_candidate_multiplier, configured_rrf_k,
    cosine_similarity, deserialize_embedding, extract_pdf_text, fts_query, now_epoch,
    reject_oversized_workspace_document, reject_secret_like_workspace_document_content,
    reject_secret_like_workspace_document_name, serialize_embedding, sha256_hex,
    validate_feedback_candidate, validate_memory_record, validate_model_config_run,
    validate_pentest_scope, validate_teacher_example, validate_workspace_document_content,
    validate_workspace_document_path, Approval, AuditEvent, CapabilityRegistry, DataSensitivity,
    EmbeddingProvider, FeedbackCandidate, FeedbackReview, FeedbackSignal, MemoryNamespace,
    MemoryProposal, MemoryRecord, ModelConfigRun, PentestCoverageEntry, PentestFinding,
    PentestFindingStatus, PentestMode, PentestScope, Risk, StoredPentestAsset, StoredPentestScope,
    Task, TeacherExample, TrustLevel, VerifyStatus, WorkspaceCitation, WorkspaceIngestionReport,
    CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION, MIN_RELEVANT_SIMILARITY,
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

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Constant-time byte comparison. Comparing a signature with `==` would short-circuit on the
/// first differing byte, which leaks (via timing) how many leading bytes an attacker's guess got
/// right — the standard reason signature/MAC comparisons never use the ordinary equality check.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Minimal HMAC-SHA256 (RFC 2104) over the `Sha256` primitive already used by `audit_hash` —
/// this project has no cryptography crate dependency, and one construction on top of an already
/// vetted hash function does not justify adding one.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut block_key = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        block_key[..hashed.len()].copy_from_slice(&hashed);
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        ipad[index] ^= block_key[index];
        opad[index] ^= block_key[index];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

/// Length-prefixes every field before hashing, exactly like `audit_hash` does — without the
/// length prefix, `("ab", "c")` and `("a", "bc")` would hash identically wherever two adjacent
/// fields are simply concatenated, which is exactly the kind of boundary ambiguity a signature
/// is supposed to rule out.
fn canonical_pentest_scope_bytes(name: &str, scope: &PentestScope) -> Vec<u8> {
    let mut bytes = Vec::new();
    let fields = [
        name.to_string(),
        scope.schema_version.to_string(),
        scope.authorization_ref.clone(),
        scope.targets.join("\n"),
        scope.excluded_targets.join("\n"),
        scope.expires_at.to_string(),
        scope.maximum_mode.as_str().to_string(),
        scope.max_runtime_seconds.to_string(),
    ];
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes
}

/// Durable task and audit storage for the implementation baseline.
#[derive(Debug)]
pub struct SqliteStore {
    connection: Connection,
}

/// Highest `schema_migrations.version` this build knows how to apply. Keep in sync with the
/// last `INSERT OR IGNORE INTO schema_migrations` row in `migrate()`.
const CURRENT_SCHEMA_VERSION: i64 = 17;

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
            );
            CREATE TABLE IF NOT EXISTS secrets (
                secret_id TEXT PRIMARY KEY,
                secret_key TEXT NOT NULL,
                secret_value TEXT NOT NULL,
                source TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS model_config_runs (
                run_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                recorded_at INTEGER NOT NULL,
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                model_fingerprint TEXT NOT NULL,
                prompt_fingerprint TEXT NOT NULL,
                server_settings TEXT NOT NULL,
                scenarios_passed INTEGER NOT NULL,
                scenarios_failed INTEGER NOT NULL,
                median_latency_ms INTEGER NOT NULL,
                notes TEXT NOT NULL,
                rollback_target TEXT
            );
            CREATE TABLE IF NOT EXISTS feedback_candidates (
                candidate_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                recorded_at INTEGER NOT NULL,
                prompt TEXT NOT NULL,
                response TEXT NOT NULL,
                signal TEXT NOT NULL,
                correction TEXT NOT NULL,
                sensitivity TEXT NOT NULL,
                provenance TEXT NOT NULL,
                review TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pentest_scopes (
                name TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                authorization_ref TEXT NOT NULL,
                targets TEXT NOT NULL,
                excluded_targets TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                maximum_mode TEXT NOT NULL,
                max_runtime_seconds INTEGER NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 0,
                revoked_at INTEGER,
                revoked_reason TEXT,
                signature TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS pentest_signing_key (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                key_hex TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pentest_assets (
                scope_name TEXT NOT NULL,
                asset TEXT NOT NULL,
                source TEXT NOT NULL,
                first_seen INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                PRIMARY KEY (scope_name, asset)
            );
            CREATE TABLE IF NOT EXISTS pentest_findings (
                finding_id TEXT PRIMARY KEY,
                scope_name TEXT NOT NULL,
                target TEXT NOT NULL,
                category TEXT NOT NULL,
                title TEXT NOT NULL,
                evidence TEXT NOT NULL,
                severity TEXT NOT NULL,
                status TEXT NOT NULL,
                recorded_at INTEGER NOT NULL,
                confirmed_at INTEGER,
                confirmation_evidence TEXT
            );
            CREATE TABLE IF NOT EXISTS pentest_coverage (
                scope_name TEXT NOT NULL,
                target TEXT NOT NULL,
                endpoint TEXT NOT NULL,
                parameter TEXT NOT NULL,
                vulnerability_class TEXT NOT NULL,
                tested_at INTEGER NOT NULL,
                PRIMARY KEY (scope_name, target, endpoint, parameter, vulnerability_class)
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
        self.ensure_column(
            "workspace_documents",
            "sensitivity",
            "TEXT NOT NULL DEFAULT 'INTERNAL'",
        )?;
        self.ensure_column(
            "memories",
            "trust_level",
            "TEXT NOT NULL DEFAULT 'USER_ASSERTED'",
        )?;
        self.ensure_column("memories", "scope_id", "TEXT")?;
        // F7.6 "Rapor öncesi yeniden doğrulama": hangi somut parametrenin (ör. `/.env` yolu)
        // kontrol edildiğini kaydediyor — kategori + hedef tek başına yeterli değil, bazı
        // kontrol türleri (açığa çıkmış dosya) tam olarak HANGİ yolun bulunduğunu bilmeden
        // hassas bir şekilde yeniden test edilemez.
        self.ensure_column("pentest_findings", "check_parameter", "TEXT")?;
        self.deduplicate_legacy_memory_records()?;
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
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (9, 'workspace document sensitivity')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (10, 'memory trust_level/scope_id, secret manager')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (11, 'F6 model/prompt config registry')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (12, 'F6 feedback review queue')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (13, 'F7 named pentest scope registry')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (14, 'F7.3 pentest asset inventory')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (15, 'F7.6 pentest finding registry')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (16, 'F7.6 pentest finding check_parameter')",
            [],
        )?;
        self.connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, name) VALUES (17, 'F7.7 pentest coverage matrix')",
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

    /// 16 Ağustos 2026'da kullanıcının gerçek `jarvis.db`'sinde bulunan bir üretim verisi hatasının
    /// kalıcı onarımı: "bellek güncelleme hatası" düzeltilmeden önce (bkz. `DEVELOPMENT_PLAN.md`
    /// "F3 sonrası düzeltmeler"), `propose_memory`'nin `memory_id`'yi değer+nonce'tan türetmesi
    /// yüzünden aynı `(namespace, key, scope_id)` için birden fazla satır oluşabiliyordu. O düzeltme
    /// yeni yazımların artık tek bir satıra güncellenmesini sağlıyor ama zaten var olan yinelenen
    /// satırları silmiyordu — bu da örneğin "Kaynaklar" listesinde aynı alanın iki kez görünmesine
    /// yol açıyordu. `repair_concurrent_audit_chain`'le aynı ilke: yıkıcı olmayan, kendi kendine
    /// iyileşen bir startup onarımı. Her grupta yalnız en son güncellenen satır kalır (eşitlikte
    /// `memory_id`'si büyük olan — deterministik bir kural, keyfi değil); diğerleri silinir. Silinen
    /// satır sayısını döner (test/kanıt için).
    fn deduplicate_legacy_memory_records(&mut self) -> SqlResult<usize> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stale_ids: Vec<String> = {
            let mut statement = transaction.prepare(
                "SELECT m.memory_id FROM memories m
                 WHERE EXISTS (
                     SELECT 1 FROM memories newer
                     WHERE newer.namespace = m.namespace
                       AND newer.memory_key = m.memory_key
                       AND (newer.scope_id IS m.scope_id)
                       AND (newer.updated_at, newer.memory_id) > (m.updated_at, m.memory_id)
                 )",
            )?;
            let mapped = statement.query_map([], |row| row.get::<_, String>(0))?;
            mapped.collect::<SqlResult<Vec<_>>>()?
        };
        for stale_id in &stale_ids {
            transaction.execute(
                "DELETE FROM memories WHERE memory_id = ?1",
                params![stale_id],
            )?;
        }
        transaction.commit()?;
        Ok(stale_ids.len())
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

    /// Every stored teacher example, oldest-first so an export is reproducible: the same
    /// database always produces the same ordering, and therefore the same manifest hash.
    pub fn teacher_examples(&self) -> Result<Vec<TeacherExample>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT example_id, schema_version, prompt, expected_capability, response,
                        evidence, verifier_status, provenance, human_reviewed, sensitivity
                 FROM teacher_examples ORDER BY example_id ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let evidence: String = row.get(5)?;
                let verifier_status: String = row.get(6)?;
                let sensitivity: String = row.get(9)?;
                Ok(TeacherExample {
                    example_id: row.get(0)?,
                    schema_version: row.get(1)?,
                    prompt: row.get(2)?,
                    expected_capability: row.get(3)?,
                    response: row.get(4)?,
                    evidence: evidence
                        .split('\n')
                        .filter(|item| !item.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    verifier_status: match verifier_status.as_str() {
                        "PASS" => VerifyStatus::Pass,
                        "FAIL" => VerifyStatus::Fail,
                        _ => VerifyStatus::Uncertain,
                    },
                    provenance: row.get(7)?,
                    human_reviewed: row.get::<_, i64>(8)? != 0,
                    sensitivity: DataSensitivity::from_str(&sensitivity)
                        .unwrap_or(DataSensitivity::Internal),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    /// F6 feedback intake write path. Stores a *pending* signal; nothing here can create
    /// training data — see `promote_feedback_candidate` for the only path that can, and the
    /// policy gate it must pass.
    pub fn record_feedback_candidate(&self, candidate: &FeedbackCandidate) -> Result<(), String> {
        validate_feedback_candidate(candidate)?;
        self.connection
            .execute(
                "INSERT OR REPLACE INTO feedback_candidates(
                    candidate_id, schema_version, recorded_at, prompt, response,
                    signal, correction, sensitivity, provenance, review
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    candidate.candidate_id,
                    candidate.schema_version,
                    candidate.recorded_at as i64,
                    candidate.prompt,
                    candidate.response,
                    candidate.signal.as_str(),
                    candidate.correction,
                    candidate.sensitivity.as_str(),
                    candidate.provenance,
                    candidate.review.as_str(),
                ],
            )
            .map_err(|error| format!("feedback candidate persist failed: {error}"))?;
        Ok(())
    }

    pub fn feedback_candidates(
        &self,
        review: Option<FeedbackReview>,
        limit: usize,
    ) -> Result<Vec<FeedbackCandidate>, String> {
        let (sql, filter) = match review {
            Some(review) => (
                "SELECT candidate_id, schema_version, recorded_at, prompt, response, signal,
                        correction, sensitivity, provenance, review
                 FROM feedback_candidates WHERE review = ?2
                 ORDER BY recorded_at DESC, candidate_id DESC LIMIT ?1",
                review.as_str().to_string(),
            ),
            None => (
                "SELECT candidate_id, schema_version, recorded_at, prompt, response, signal,
                        correction, sensitivity, provenance, review
                 FROM feedback_candidates WHERE ?2 = ?2
                 ORDER BY recorded_at DESC, candidate_id DESC LIMIT ?1",
                String::new(),
            ),
        };
        let mut statement = self
            .connection
            .prepare(sql)
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(rusqlite::params![limit as i64, filter], |row| {
                let signal: String = row.get(5)?;
                let sensitivity: String = row.get(7)?;
                let review: String = row.get(9)?;
                Ok(FeedbackCandidate {
                    candidate_id: row.get(0)?,
                    schema_version: row.get(1)?,
                    recorded_at: row.get::<_, i64>(2)? as u64,
                    prompt: row.get(3)?,
                    response: row.get(4)?,
                    signal: FeedbackSignal::parse(&signal).unwrap_or(FeedbackSignal::Negative),
                    correction: row.get(6)?,
                    sensitivity: DataSensitivity::from_str(&sensitivity)
                        .unwrap_or(DataSensitivity::Internal),
                    provenance: row.get(8)?,
                    review: FeedbackReview::parse(&review).unwrap_or(FeedbackReview::Pending),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn set_feedback_review(
        &self,
        candidate_id: &str,
        review: FeedbackReview,
    ) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE feedback_candidates SET review = ?2 WHERE candidate_id = ?1",
                rusqlite::params![candidate_id, review.as_str()],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("feedback candidate not found: {candidate_id}"));
        }
        Ok(())
    }

    /// F7.1 "İmzalı authorization/scope manifest". Not proof of what the bug bounty program
    /// granted — no local process can attest to that — but proof that the scope stored under
    /// `name` is exactly what `save_pentest_scope` wrote and has not changed since (a direct
    /// database edit, a restored backup from a different machine, or any path that bypasses the
    /// typed contract). The key is generated once per machine with `/dev/urandom` (this project
    /// has no cryptography crate dependency; HMAC-SHA256 needs only the `sha2` hasher already
    /// used by `audit_hash`, so this avoids adding one for a single construction) and is never
    /// exposed through any user-facing command — it lives in its own table, deliberately not the
    /// `secrets` table `/secret show` can read, so there is no command that could ever display or
    /// delete it by accident.
    fn pentest_signing_key(&self) -> Result<[u8; 32], String> {
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT key_hex FROM pentest_signing_key WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(hex_key) = existing {
            return decode_hex_32(&hex_key)
                .ok_or_else(|| "stored pentest signing key is corrupt".to_string());
        }
        let mut key = [0u8; 32];
        {
            use std::io::Read;
            let mut urandom = std::fs::File::open("/dev/urandom")
                .map_err(|error| format!("could not open /dev/urandom: {error}"))?;
            urandom
                .read_exact(&mut key)
                .map_err(|error| format!("could not read /dev/urandom: {error}"))?;
        }
        self.connection
            .execute(
                "INSERT INTO pentest_signing_key(id, key_hex, created_at) VALUES (1, ?1, ?2)",
                rusqlite::params![encode_hex(&key), now_epoch() as i64],
            )
            .map_err(|error| format!("pentest signing key persist failed: {error}"))?;
        Ok(key)
    }

    fn sign_pentest_scope(&self, name: &str, scope: &PentestScope) -> Result<String, String> {
        let key = self.pentest_signing_key()?;
        Ok(encode_hex(&hmac_sha256(
            &key,
            &canonical_pentest_scope_bytes(name, scope),
        )))
    }

    /// F7.1 "Çoklu program/scope yönetimi". Validation happens here (through the single
    /// policy-gate validator) for the same reason `record_model_config_run` does it here —
    /// no future caller can persist an invalid scope by skipping the check. `INSERT OR REPLACE`
    /// is deliberate: re-saving a scope under the same name (e.g. renewing an authorization)
    /// updates it in place rather than requiring a separate "update" call, but never touches
    /// `is_active`/`revoked_at` — those are their own explicit actions below, not side effects
    /// of a save. Re-saving also re-signs: the signature always covers whatever is on disk now.
    pub fn save_pentest_scope(&self, name: &str, scope: &PentestScope) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("pentest scope name is required".into());
        }
        validate_pentest_scope(scope)?;
        let signature = self.sign_pentest_scope(name, scope)?;
        let was_active: i64 = self
            .connection
            .query_row(
                "SELECT is_active FROM pentest_scopes WHERE name = ?1",
                rusqlite::params![name],
                |row| row.get(0),
            )
            .unwrap_or(0);
        self.connection
            .execute(
                "INSERT OR REPLACE INTO pentest_scopes(
                    name, schema_version, authorization_ref, targets, excluded_targets,
                    expires_at, maximum_mode, max_runtime_seconds, is_active, revoked_at,
                    revoked_reason, signature
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10)",
                rusqlite::params![
                    name,
                    scope.schema_version,
                    scope.authorization_ref,
                    scope.targets.join("\n"),
                    scope.excluded_targets.join("\n"),
                    scope.expires_at as i64,
                    scope.maximum_mode.as_str(),
                    scope.max_runtime_seconds,
                    was_active,
                    signature,
                ],
            )
            .map_err(|error| format!("pentest scope persist failed: {error}"))?;
        Ok(())
    }

    /// True only if the stored scope's signature matches what `pentest_signing_key` would
    /// produce for its current on-disk content right now. A tampered row (edited outside
    /// `save_pentest_scope`) fails this — that is the entire point.
    pub fn pentest_scope_signature_is_valid(&self, name: &str) -> Result<bool, String> {
        let Some(stored) = self.pentest_scope(name)? else {
            return Ok(false);
        };
        let expected = self.sign_pentest_scope(name, &stored.scope)?;
        Ok(constant_time_eq(
            expected.as_bytes(),
            stored.signature.as_bytes(),
        ))
    }

    pub fn pentest_scope(&self, name: &str) -> Result<Option<StoredPentestScope>, String> {
        self.connection
            .query_row(
                "SELECT name, schema_version, authorization_ref, targets, excluded_targets,
                        expires_at, maximum_mode, max_runtime_seconds, is_active, revoked_at, revoked_reason,
                        signature
                 FROM pentest_scopes WHERE name = ?1",
                rusqlite::params![name],
                Self::row_to_stored_pentest_scope,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn pentest_scopes(&self) -> Result<Vec<StoredPentestScope>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT name, schema_version, authorization_ref, targets, excluded_targets,
                        expires_at, maximum_mode, max_runtime_seconds, is_active, revoked_at, revoked_reason,
                        signature
                 FROM pentest_scopes ORDER BY name ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], Self::row_to_stored_pentest_scope)
            .map_err(|error| error.to_string())?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    fn row_to_stored_pentest_scope(row: &rusqlite::Row) -> SqlResult<StoredPentestScope> {
        let targets: String = row.get(3)?;
        let excluded_targets: String = row.get(4)?;
        let maximum_mode: String = row.get(6)?;
        let is_active: i64 = row.get(8)?;
        Ok(StoredPentestScope {
            name: row.get(0)?,
            scope: PentestScope {
                schema_version: row.get(1)?,
                authorization_ref: row.get(2)?,
                targets: targets
                    .split('\n')
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
                excluded_targets: excluded_targets
                    .split('\n')
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
                expires_at: row.get::<_, i64>(5)? as u64,
                maximum_mode: PentestMode::parse(&maximum_mode).unwrap_or(PentestMode::Safe),
                max_runtime_seconds: row.get(7)?,
            },
            is_active: is_active != 0,
            revoked_at: row.get::<_, Option<i64>>(9)?.map(|value| value as u64),
            revoked_reason: row.get(10)?,
            signature: row.get(11)?,
        })
    }

    /// F7.1 "aktif scope her zaman açıkça gösterilir". Exactly one scope is active at a time —
    /// clearing every row before setting the target one, in a single transaction, is what makes
    /// "which program am I authorized against right now" a single unambiguous answer instead of
    /// a set that could (through a missed code path) end up with zero or more than one.
    /// Activating a revoked scope is refused: revocation is a deliberate deauthorization and
    /// re-activating it would silently undo that decision.
    pub fn set_active_pentest_scope(&self, name: &str) -> Result<(), String> {
        let scope = self
            .pentest_scope(name)?
            .ok_or_else(|| format!("no stored pentest scope named '{name}'"))?;
        if scope.is_revoked() {
            return Err(format!(
                "pentest scope '{name}' is revoked and cannot be reactivated"
            ));
        }
        validate_pentest_scope(&scope.scope)?;
        self.connection
            .execute("UPDATE pentest_scopes SET is_active = 0", [])
            .map_err(|error| error.to_string())?;
        let changed = self
            .connection
            .execute(
                "UPDATE pentest_scopes SET is_active = 1 WHERE name = ?1",
                rusqlite::params![name],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("no stored pentest scope named '{name}'"));
        }
        Ok(())
    }

    pub fn deactivate_pentest_scope(&self) -> Result<(), String> {
        self.connection
            .execute("UPDATE pentest_scopes SET is_active = 0", [])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn active_pentest_scope(&self) -> Result<Option<StoredPentestScope>, String> {
        self.connection
            .query_row(
                "SELECT name, schema_version, authorization_ref, targets, excluded_targets,
                        expires_at, maximum_mode, max_runtime_seconds, is_active, revoked_at, revoked_reason,
                        signature
                 FROM pentest_scopes WHERE is_active = 1",
                [],
                Self::row_to_stored_pentest_scope,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// F7.1 "expiry/revoke". A revoke is its own event, not an edit to the original grant — the
    /// authorization the user recorded stays exactly as it was; `revoked_at`/`revoked_reason`
    /// are appended alongside it, mirroring how the audit hash-chain never rewrites a past
    /// entry. Revoking an active scope also deactivates it immediately: a revoked scope must
    /// never be the answer to "what am I authorized against right now."
    pub fn revoke_pentest_scope(&self, name: &str, reason: &str) -> Result<(), String> {
        if reason.trim().is_empty() {
            return Err("revoking a pentest scope requires a reason".into());
        }
        let changed = self
            .connection
            .execute(
                "UPDATE pentest_scopes SET revoked_at = ?2, revoked_reason = ?3, is_active = 0
                 WHERE name = ?1",
                rusqlite::params![name, now_epoch() as i64, reason],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("no stored pentest scope named '{name}'"));
        }
        Ok(())
    }

    /// F7.3 "Varlık envanteri kalıcı kaydı + yeni varlık ortaya çıkınca bildirim". Bir keşif
    /// turunun bulduğu isim listesini `scope_name` altında kalıcı hale getirir ve DAHA ÖNCE bu
    /// scope için hiç görülmemiş olanları geri döndürür — bildirim mantığı burada bir diff'ten
    /// başka bir şey değil: "yeni" olmak, `pentest_assets` tablosunda `(scope_name, asset)`
    /// birincil anahtarının INSERT'ten ÖNCE var olup olmadığına bakılarak belirleniyor.
    /// `INSERT ... ON CONFLICT DO UPDATE` tek bir atomik ifade: zaten bilinen bir varlık sessizce
    /// `last_seen`'i güncelliyor (periyodik yeniden taramanın doğal davranışı), yeni bir varlık
    /// ise `first_seen`'i şimdiki zamana yazıyor.
    pub fn record_pentest_assets(
        &self,
        scope_name: &str,
        source: &str,
        assets: &[String],
    ) -> Result<Vec<String>, String> {
        if assets.is_empty() {
            return Ok(Vec::new());
        }
        let mut already_known = std::collections::HashSet::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT asset FROM pentest_assets WHERE scope_name = ?1")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([scope_name], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?;
            for row in rows {
                already_known.insert(row.map_err(|error| error.to_string())?);
            }
        }
        let now = now_epoch() as i64;
        for asset in assets {
            self.connection
                .execute(
                    "INSERT INTO pentest_assets(scope_name, asset, source, first_seen, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?4)
                     ON CONFLICT(scope_name, asset) DO UPDATE SET last_seen = excluded.last_seen",
                    rusqlite::params![scope_name, asset, source, now],
                )
                .map_err(|error| format!("pentest asset kaydı başarısız: {error}"))?;
        }
        let mut new_assets: Vec<String> = assets
            .iter()
            .filter(|asset| !already_known.contains(*asset))
            .cloned()
            .collect();
        new_assets.sort();
        new_assets.dedup();
        Ok(new_assets)
    }

    /// Bir scope için şu ana kadar kaydedilmiş tüm varlıkları döndürür — en son görülen önce,
    /// böylece "bu programda en güncel ne var" sorusunun cevabı listenin başında.
    pub fn pentest_assets(&self, scope_name: &str) -> Result<Vec<StoredPentestAsset>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT scope_name, asset, source, first_seen, last_seen FROM pentest_assets
                 WHERE scope_name = ?1 ORDER BY last_seen DESC, asset ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([scope_name], |row| {
                Ok(StoredPentestAsset {
                    scope_name: row.get(0)?,
                    asset: row.get(1)?,
                    source: row.get(2)?,
                    first_seen: row.get::<_, i64>(3)? as u64,
                    last_seen: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    /// F7.6 "Evidence tabanlı finding formatı". `finding_id`, `memory_id`'nin kendi deseniyle
    /// aynı: `(scope_name, target, category, title)` dörtlüsünün içerik hash'i. Bu, F7.6'nın
    /// ayrı bir madde olarak istediği "eşleştirme (deduplication)"yi ayrı bir mekanizma icat
    /// etmeden sağlıyor — AYNI bulgu tekrar kaydedilirse `INSERT OR REPLACE` aynı satırı
    /// günceller (kanıt/zaman tazelenir), yeni bir satır oluşmaz. **Sır sızıntısı koruması:**
    /// kanıt metni, workspace RAG ingestion'ın zaten kullandığı
    /// `reject_secret_like_workspace_document_content` ile taranıyor — bariz bir sır/kimlik bilgisi
    /// deseni (PEM anahtar başlığı, bilinen token öneki) içeren bir kanıt reddediliyor; çağıran
    /// önce kanıtı redakte etmeli (ör. "API_KEY=[REDACTED]").
    #[allow(clippy::too_many_arguments)]
    pub fn record_pentest_finding(
        &self,
        scope_name: &str,
        target: &str,
        category: &str,
        title: &str,
        evidence: &str,
        severity_estimate: Risk,
        check_parameter: Option<&str>,
    ) -> Result<PentestFinding, String> {
        reject_secret_like_workspace_document_content(evidence)?;
        let identity = format!("finding-v1|{scope_name}|{target}|{category}|{title}");
        let finding_id = format!("finding-{}", &sha256_hex(&identity)[..16]);
        let recorded_at = now_epoch() as i64;
        self.connection
            .execute(
                "INSERT INTO pentest_findings(
                    finding_id, scope_name, target, category, title, evidence, severity,
                    status, recorded_at, confirmed_at, confirmation_evidence, check_parameter
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10)
                 ON CONFLICT(finding_id) DO UPDATE SET
                    evidence = excluded.evidence,
                    severity = excluded.severity,
                    recorded_at = excluded.recorded_at,
                    check_parameter = excluded.check_parameter",
                rusqlite::params![
                    finding_id,
                    scope_name,
                    target,
                    category,
                    title,
                    evidence,
                    severity_estimate.as_str(),
                    PentestFindingStatus::Suspected.as_str(),
                    recorded_at,
                    check_parameter,
                ],
            )
            .map_err(|error| format!("pentest finding kaydı başarısız: {error}"))?;
        self.pentest_finding(&finding_id)?
            .ok_or_else(|| "pentest finding kaydedildi ama geri okunamadı".to_string())
    }

    /// F7.6 "İnsan onayı" + F7.7'nin `confirm_finding` sözleşmesi: bir bulgu, taze bir yeniden
    /// üretme kanıtı VE açık bir insan onayı olmadan `Confirmed` durumuna geçemez —
    /// `commit_memory_proposal`'ın `user_approved: bool` deseniyle aynı. Yalnız hâlâ `Suspected`
    /// durumundaki bir bulgu onaylanabilir (zaten `Confirmed`/`Rejected` bir bulguyu sessizce
    /// yeniden onaylamak, o kararın ne zaman/neden verildiğini belirsizleştirirdi).
    pub fn confirm_pentest_finding(
        &self,
        finding_id: &str,
        confirmation_evidence: &str,
        human_approved: bool,
    ) -> Result<PentestFinding, String> {
        if !human_approved {
            return Err("bir bulguyu doğrulamak açık insan onayı gerektirir".into());
        }
        if confirmation_evidence.trim().is_empty() {
            return Err("doğrulama, taze bir yeniden üretme kanıtı gerektirir".into());
        }
        reject_secret_like_workspace_document_content(confirmation_evidence)?;
        let current = self
            .pentest_finding(finding_id)?
            .ok_or_else(|| format!("'{finding_id}' adında bir bulgu yok"))?;
        if current.status != PentestFindingStatus::Suspected {
            return Err(format!(
                "'{finding_id}' zaten '{}' durumunda — yalnız 'suspected' durumundaki bir bulgu doğrulanabilir",
                current.status.as_str()
            ));
        }
        let confirmed_at = now_epoch() as i64;
        self.connection
            .execute(
                "UPDATE pentest_findings SET status = ?2, confirmed_at = ?3, confirmation_evidence = ?4
                 WHERE finding_id = ?1",
                rusqlite::params![
                    finding_id,
                    PentestFindingStatus::Confirmed.as_str(),
                    confirmed_at,
                    confirmation_evidence,
                ],
            )
            .map_err(|error| error.to_string())?;
        self.pentest_finding(finding_id)?
            .ok_or_else(|| "bulgu doğrulandı ama geri okunamadı".to_string())
    }

    /// F7.6: bir bulgunun yanlış pozitif olduğuna insan karar verdi — silinmiyor (append-only
    /// felsefesi, audit chain'le aynı), yalnız durumu değişiyor.
    pub fn reject_pentest_finding(&self, finding_id: &str) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE pentest_findings SET status = ?2 WHERE finding_id = ?1",
                rusqlite::params![finding_id, PentestFindingStatus::Rejected.as_str()],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("'{finding_id}' adında bir bulgu yok"));
        }
        Ok(())
    }

    pub fn pentest_finding(&self, finding_id: &str) -> Result<Option<PentestFinding>, String> {
        self.connection
            .query_row(
                "SELECT finding_id, scope_name, target, category, title, evidence, severity,
                        status, recorded_at, confirmed_at, confirmation_evidence, check_parameter
                 FROM pentest_findings WHERE finding_id = ?1",
                [finding_id],
                Self::row_to_pentest_finding,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Bir scope'un tüm bulguları — en son kaydedilen önce.
    pub fn pentest_findings(&self, scope_name: &str) -> Result<Vec<PentestFinding>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT finding_id, scope_name, target, category, title, evidence, severity,
                        status, recorded_at, confirmed_at, confirmation_evidence, check_parameter
                 FROM pentest_findings WHERE scope_name = ?1 ORDER BY recorded_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([scope_name], Self::row_to_pentest_finding)
            .map_err(|error| error.to_string())?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    fn row_to_pentest_finding(row: &rusqlite::Row) -> SqlResult<PentestFinding> {
        let severity_raw: String = row.get(6)?;
        let status_raw: String = row.get(7)?;
        Ok(PentestFinding {
            finding_id: row.get(0)?,
            scope_name: row.get(1)?,
            target: row.get(2)?,
            category: row.get(3)?,
            title: row.get(4)?,
            evidence: row.get(5)?,
            severity_estimate: Risk::parse(&severity_raw).unwrap_or(Risk::Low),
            status: PentestFindingStatus::parse(&status_raw)
                .unwrap_or(PentestFindingStatus::Suspected),
            recorded_at: row.get::<_, i64>(8)? as u64,
            confirmed_at: row.get::<_, Option<i64>>(9)?.map(|value| value as u64),
            confirmation_evidence: row.get(10)?,
            check_parameter: row.get(11)?,
        })
    }

    /// F7.7 "Kapsam matrisi": bir `(hedef, endpoint, parametre, zafiyet_sınıfı)` kombinasyonunun
    /// test edildiğini kaydeder. `INSERT OR REPLACE` — aynı kombinasyon tekrar test edilirse
    /// yalnız `tested_at` tazelenir, yeni satır oluşmaz (bir kombinasyon ya test edilmiştir ya
    /// edilmemiştir, "kaç kez" bu tablonun sorusu değil).
    pub fn record_pentest_coverage(
        &self,
        scope_name: &str,
        target: &str,
        endpoint: &str,
        parameter: &str,
        vulnerability_class: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO pentest_coverage(
                    scope_name, target, endpoint, parameter, vulnerability_class, tested_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    scope_name,
                    target,
                    endpoint,
                    parameter,
                    vulnerability_class,
                    now_epoch() as i64,
                ],
            )
            .map_err(|error| format!("pentest coverage kaydı başarısız: {error}"))?;
        Ok(())
    }

    /// Belirli bir kombinasyonun daha önce test edilip edilmediği — "sıradaki iş" önerisinin
    /// zaten yapılmışı önermemesi için.
    pub fn pentest_coverage_contains(
        &self,
        scope_name: &str,
        target: &str,
        endpoint: &str,
        parameter: &str,
        vulnerability_class: &str,
    ) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT 1 FROM pentest_coverage
                 WHERE scope_name = ?1 AND target = ?2 AND endpoint = ?3
                   AND parameter = ?4 AND vulnerability_class = ?5",
                rusqlite::params![scope_name, target, endpoint, parameter, vulnerability_class],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(|error| error.to_string())
    }

    /// Bir scope'un tüm kapsam kayıtları — en son test edilen önce.
    pub fn pentest_coverage(&self, scope_name: &str) -> Result<Vec<PentestCoverageEntry>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT scope_name, target, endpoint, parameter, vulnerability_class, tested_at
                 FROM pentest_coverage WHERE scope_name = ?1 ORDER BY tested_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([scope_name], |row| {
                Ok(PentestCoverageEntry {
                    scope_name: row.get(0)?,
                    target: row.get(1)?,
                    endpoint: row.get(2)?,
                    parameter: row.get(3)?,
                    vulnerability_class: row.get(4)?,
                    tested_at: row.get::<_, i64>(5)? as u64,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    /// F6 registry write path. Validation happens here (through the single policy-gate
    /// validator) rather than at the call site so no future caller can persist an
    /// unattributable run row by skipping the check.
    pub fn record_model_config_run(&self, run: &ModelConfigRun) -> Result<(), String> {
        validate_model_config_run(run)?;
        self.connection
            .execute(
                "INSERT OR REPLACE INTO model_config_runs(
                    run_id, schema_version, recorded_at, provider_id, model_id,
                    model_fingerprint, prompt_fingerprint, server_settings,
                    scenarios_passed, scenarios_failed, median_latency_ms, notes, rollback_target
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    run.run_id,
                    run.schema_version,
                    run.recorded_at as i64,
                    run.provider_id,
                    run.model_id,
                    run.model_fingerprint,
                    run.prompt_fingerprint,
                    run.server_settings,
                    run.scenarios_passed,
                    run.scenarios_failed,
                    run.median_latency_ms as i64,
                    run.notes,
                    run.rollback_target,
                ],
            )
            .map_err(|error| format!("model config run persist failed: {error}"))?;
        Ok(())
    }

    /// Newest-first so the most recent configuration — the one a rollback decision is about — is
    /// always the first row a caller sees.
    pub fn model_config_runs(&self, limit: usize) -> Result<Vec<ModelConfigRun>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT run_id, schema_version, recorded_at, provider_id, model_id,
                        model_fingerprint, prompt_fingerprint, server_settings,
                        scenarios_passed, scenarios_failed, median_latency_ms, notes, rollback_target
                 FROM model_config_runs ORDER BY recorded_at DESC, run_id DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(ModelConfigRun {
                    run_id: row.get(0)?,
                    schema_version: row.get(1)?,
                    recorded_at: row.get::<_, i64>(2)? as u64,
                    provider_id: row.get(3)?,
                    model_id: row.get(4)?,
                    model_fingerprint: row.get(5)?,
                    prompt_fingerprint: row.get(6)?,
                    server_settings: row.get(7)?,
                    scenarios_passed: row.get(8)?,
                    scenarios_failed: row.get(9)?,
                    median_latency_ms: row.get::<_, i64>(10)? as u64,
                    notes: row.get(11)?,
                    rollback_target: row.get(12)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())
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
                    source, include_in_model_context, created_at, updated_at, expires_at,
                    trust_level, scope_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(memory_id) DO UPDATE SET
                    memory_key=excluded.memory_key,
                    memory_value=excluded.memory_value,
                    sensitivity=excluded.sensitivity,
                    source=excluded.source,
                    include_in_model_context=excluded.include_in_model_context,
                    updated_at=excluded.updated_at,
                    expires_at=excluded.expires_at,
                    trust_level=excluded.trust_level,
                    scope_id=excluded.scope_id",
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
                    proposal.record.trust_level.as_str(),
                    proposal.record.scope_id,
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
        self.index_workspace_document_with_embedding_and_sensitivity(
            approved_root,
            relative_path,
            embedding_provider,
            DataSensitivity::Internal,
        )
    }

    /// Same as `index_workspace_document_with_embedding`, with an explicit sensitivity level
    /// stored alongside the document (F3 post-close "retrieval öncesi permission/sensitivity
    /// filtresi", GPT önerisi 1/7). `Sensitive` documents are excluded from ordinary
    /// conversational retrieval by `search_workspace` — indexed and still directly readable
    /// (`file.read_workspace`), just never surfaced as an automatic citation. Defaults to
    /// `Internal` (unrestricted retrieval) through `index_workspace_document_with_embedding`,
    /// matching every document indexed before this level existed.
    pub fn index_workspace_document_with_embedding_and_sensitivity(
        &self,
        approved_root: &Path,
        relative_path: &Path,
        embedding_provider: Option<&dyn EmbeddingProvider>,
        sensitivity: DataSensitivity,
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
        let chunks = chunk_workspace_text(&content, &canonical_path);
        if chunks.is_empty() {
            return Err("workspace document has no indexable text".into());
        }
        let canonical_path_text = canonical_path.to_string_lossy().into_owned();
        let document_id = format!("document-{}", &sha256_hex(&canonical_path_text)[..16]);
        let content_sha256 = sha256_hex(&content);

        // Incremental re-index: if this exact path is already indexed with byte-for-byte
        // identical extracted content, skip the chunk delete/re-insert churn entirely and report
        // the existing state as unchanged, rather than redoing work indexing didn't need to redo.
        let existing_state: Option<(String, i64, i64, String)> = self
            .connection
            .query_row(
                "SELECT content_sha256, indexed_at, index_schema_version, sensitivity FROM workspace_documents WHERE canonical_path=?1",
                [&canonical_path_text],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .ok();
        if let Some((
            existing_sha256,
            existing_indexed_at,
            existing_index_schema_version,
            existing_sensitivity,
        )) = &existing_state
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
                // A re-index can still change the *sensitivity level* alone, without the content
                // changing — cheap to apply even on this fast path, so promoting/demoting a
                // document's sensitivity never requires an unrelated content edit to take effect.
                if existing_sensitivity != sensitivity.as_str() {
                    self.connection
                        .execute(
                            "UPDATE workspace_documents SET sensitivity=?1 WHERE document_id=?2",
                            params![sensitivity.as_str(), document_id],
                        )
                        .map_err(|error| format!("sensitivity update failed: {error}"))?;
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
                "INSERT INTO workspace_documents(document_id, canonical_path, content_sha256, indexed_at, index_schema_version, sensitivity)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(canonical_path) DO UPDATE SET
                    document_id=excluded.document_id,
                    content_sha256=excluded.content_sha256,
                    indexed_at=excluded.indexed_at,
                    sensitivity=excluded.sensitivity,
                    index_schema_version=excluded.index_schema_version",
                params![
                    document_id,
                    canonical_path_text,
                    content_sha256,
                    indexed_at as i64,
                    i64::from(CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION),
                    sensitivity.as_str(),
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

    /// F3 post-close "retrieval öncesi permission/sensitivity filtresi" (GPT önerisi 1/7):
    /// documents indexed as `Sensitive` (see `index_workspace_document_with_embedding_and_sensitivity`)
    /// never come back from ordinary search — this is the single point every retrieval path goes
    /// through (`hybrid_search_workspace` calls this for its FTS candidate pool), so the filter
    /// cannot be bypassed by a different retrieval entry point. The document is still indexed and
    /// still directly readable (`file.read_workspace`); it just never becomes an automatic
    /// citation in ordinary conversation.
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
                       AND documents.sensitivity != 'SENSITIVE'
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
        // re-ranking a single top-1 FTS hit would be a no-op. Both the multiplier and RRF_K are
        // configurable (F3 post-close "configurable RRF sabitleri", GPT önerisi 6/7) — see
        // `configured_rrf_k`/`configured_retrieval_candidate_multiplier` in workspace.rs.
        let candidate_pool = (limit * configured_retrieval_candidate_multiplier()).clamp(limit, 16);
        let candidates = self.search_workspace(query, candidate_pool)?;
        let ranked = match query_embedding {
            None => candidates,
            Some(_) if candidates.is_empty() => candidates,
            Some((embedding_model_id, query_embedding)) => {
                let rrf_k = configured_rrf_k();
                let mut scored: Vec<(f64, Option<f32>, WorkspaceCitation)> = candidates
                    .into_iter()
                    .enumerate()
                    .map(|(fts_rank, citation)| {
                        let similarity = self
                            .chunk_embedding(&citation.chunk_id, embedding_model_id)
                            .map(|stored| cosine_similarity(query_embedding, &stored));
                        (1.0 / (rrf_k + (fts_rank + 1) as f64), similarity, citation)
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
                        scored[index].0 += 1.0 / (rrf_k + (embedding_rank + 1) as f64);
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

    /// F3 post-close "`/rag status`" (GPT önerisi 5/7): total indexed chunk count, independent of
    /// any embedding model — this is the FTS-side size, always meaningful even with no embedding
    /// provider attached at all.
    pub fn workspace_chunk_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM workspace_chunks", [], |row| {
                row.get(0)
            })
    }

    /// How many chunks currently have a stored embedding from `embedding_model_id` specifically —
    /// compared against `workspace_chunk_count()`, this is the hybrid coverage `/rag status`
    /// shows ("kaç chunk'ın anlamsal arama karşılığı var").
    pub fn workspace_chunk_embedding_count_for_model(
        &self,
        embedding_model_id: &str,
    ) -> SqlResult<i64> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM workspace_chunk_embeddings WHERE embedding_model_id=?1",
            [embedding_model_id],
            |row| row.get(0),
        )
    }

    /// F3 post-close "`/rag verify`" (GPT önerisi 5/7): stored vectors whose chunk no longer
    /// exists in `workspace_chunks` — should never happen (re-indexing deletes a document's
    /// embeddings alongside its chunks), so a non-zero count here is a real integrity finding,
    /// not an expected/steady-state condition.
    pub fn workspace_orphaned_embedding_count(&self) -> SqlResult<i64> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM workspace_chunk_embeddings
             WHERE chunk_id NOT IN (SELECT chunk_id FROM workspace_chunks)",
            [],
            |row| row.get(0),
        )
    }

    /// F3 post-close "`/rag rebuild`" (GPT önerisi 5/7): deletes every stored embedding for
    /// `provider`'s model, then re-embeds every currently-indexed chunk from scratch — the
    /// embedding cache is explicitly a *derived* cache (ADR-0004), so this is always safe: the
    /// source of truth (`workspace_chunks`' text) is untouched, only the derived vectors are
    /// recomputed. Batched per document via `embed_and_store_chunks_batch`, not per chunk.
    /// Returns the number of chunks re-embedded.
    pub fn rebuild_embeddings_for_model(
        &self,
        provider: &dyn EmbeddingProvider,
    ) -> Result<usize, String> {
        let model_id = provider.embedding_model_id();
        self.connection
            .execute(
                "DELETE FROM workspace_chunk_embeddings WHERE embedding_model_id=?1",
                [model_id],
            )
            .map_err(|error| format!("clearing existing embeddings failed: {error}"))?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT document_id, chunk_id, content FROM workspace_chunks ORDER BY document_id",
            )
            .map_err(|error| format!("rebuild query setup failed: {error}"))?;
        let rows: Vec<(String, String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|error| format!("rebuild query failed: {error}"))?
            .collect::<SqlResult<Vec<_>>>()
            .map_err(|error| format!("rebuild row failed: {error}"))?;
        drop(statement);
        let total = rows.len();
        // Grouped by document (not one call for the whole workspace) so `embed_and_store_chunks_batch`
        // still tags each embedding with the right `document_id`, same as normal indexing does —
        // still one batched model call per document, not one per chunk.
        let mut by_document: std::collections::BTreeMap<String, Vec<(String, String)>> =
            std::collections::BTreeMap::new();
        for (document_id, chunk_id, content) in rows {
            by_document
                .entry(document_id)
                .or_default()
                .push((chunk_id, content));
        }
        for (document_id, chunks) in &by_document {
            self.embed_and_store_chunks_batch(provider, document_id, chunks);
        }
        Ok(total)
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
    /// `task_scope`: kullanıcının "concurrent task'lar birbirinin context'ini kirletmesin"
    /// kuralının uygulama noktası. `Task` namespace'i `namespaces` listesindeyse:
    /// - `task_scope=Some(id)` ise yalnız `scope_id==id` olan `Task` kayıtları döner — başka
    ///   hiçbir task'ın kaydı asla karışmaz.
    /// - `task_scope=None` ise `Task` namespace'i **tamamen hariç tutulur** (listede olsa bile) —
    ///   hangi task'ın bağlamında olduğumuz bilinmiyorsa, hiçbirinin kaydını karıştırmamak tek
    ///   güvenli varsayılan. Diğer dört namespace bundan etkilenmez.
    pub fn retrieve_memory(
        &self,
        namespaces: &[MemoryNamespace],
        task_scope: Option<&str>,
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
                            expires_at, trust_level, scope_id
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
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
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
                    trust_level,
                    scope_id,
                )| {
                    let namespace = MemoryNamespace::from_str(&namespace).ok()?;
                    if !namespaces.contains(&namespace) {
                        return None;
                    }
                    if namespace == MemoryNamespace::Task && scope_id.as_deref() != task_scope {
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
                        trust_level: TrustLevel::from_str(&trust_level),
                        scope_id,
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
                            expires_at, trust_level, scope_id
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
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
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
                    trust_level,
                    scope_id,
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
                        trust_level: TrustLevel::from_str(&trust_level),
                        scope_id,
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

    /// Kullanıcının "secret'ları doğrudan hafızaya yazmıyoruz, Secret Manager referansı
    /// tutuyoruz" kuralının depolama katmanı — `memories` tablosundan **tamamen ayrı** bir tablo.
    /// Gerçek değer yalnız burada; `Runtime::remember_secret` bunu çağırdıktan sonra `memories`'e
    /// yalnız bir yer tutucu (placeholder) satır ekler, gerçek değeri asla değil. `secret_id`
    /// anahtardan türetildiği için aynı anahtarı tekrar kaydetmek (bugün bellek için düzeltilen
    /// bug'la aynı desen) günceller, ikinci bir satır oluşturmaz.
    pub fn store_secret(&self, key: &str, value: &str, source: &str) -> Result<String, String> {
        if key.trim().is_empty() || value.trim().is_empty() {
            return Err("secret requires a non-empty key and value".into());
        }
        let secret_id = format!("secret-{}", &sha256_hex(&format!("secret-v1|{key}"))[..16]);
        let now = now_epoch() as i64;
        self.connection
            .execute(
                "INSERT INTO secrets(secret_id, secret_key, secret_value, source, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(secret_id) DO UPDATE SET
                    secret_value=excluded.secret_value,
                    source=excluded.source,
                    updated_at=excluded.updated_at",
                params![secret_id, key, value, source, now],
            )
            .map_err(|error| format!("secret persistence failed: {error}"))?;
        Ok(secret_id)
    }

    /// Gerçek sır değerini döner — yalnız kullanıcının kendi açık talebiyle (`/secret show
    /// <anahtar>`) çağrılmalı; hiçbir sohbet/model bağlamı derleme yolu bunu hiç çağırmaz (model
    /// bağlamı yalnız `memories`'teki yer tutucuyu görür, gerçek değeri asla).
    pub fn resolve_secret(&self, key: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT secret_value FROM secrets WHERE secret_key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("secret lookup failed: {error}"))
    }

    pub fn delete_secret(&self, key: &str) -> Result<bool, String> {
        self.connection
            .execute("DELETE FROM secrets WHERE secret_key=?1", [key])
            .map(|changed| changed > 0)
            .map_err(|error| format!("secret deletion failed: {error}"))
    }

    /// Yalnız anahtar adları — hiçbir zaman değer içermez. `/secrets` listelemesinin temeli.
    pub fn list_secret_keys(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT secret_key FROM secrets ORDER BY secret_key ASC")
            .map_err(|error| format!("secret list setup failed: {error}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("secret list query failed: {error}"))?;
        rows.collect::<SqlResult<Vec<_>>>()
            .map_err(|error| format!("secret list row failed: {error}"))
    }

    pub fn secret_count(&self) -> SqlResult<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM secrets", [], |row| row.get(0))
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

    /// F9 "Release pipeline: ... migration kontrolü". Şema göç kayıtlarının bütünlüğünü doğrular —
    /// bir class hatayı yakalamak için: `CURRENT_SCHEMA_VERSION` artırılıp ilgili
    /// `INSERT INTO schema_migrations` satırının EKLENMEMESİ (bu oturumda her yeni tabloda elle
    /// hatırlanması gereken, kolayca unutulan bir adımdı). Kontroller: (1) kayıtlı en yüksek sürüm
    /// tam olarak `CURRENT_SCHEMA_VERSION`; (2) 1..=güncel arası HİÇBİR sürüm atlanmamış (boşluk
    /// yok); (3) her göçün boş olmayan bir adı var (kime ne yaptığı belirsiz bir göç, geri
    /// alma/hata ayıklama sırasında işe yaramaz). Herhangi biri ihlal edilirse açıklayıcı bir
    /// `Err`. Ağ/indirme gerektirmez — release kapısının çevrimdışı doğasına uygun.
    pub fn verify_schema_migrations(&self) -> Result<(), String> {
        let mut statement = self
            .connection
            .prepare("SELECT version, name FROM schema_migrations ORDER BY version ASC")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let recorded: Vec<(i64, String)> = rows
            .collect::<SqlResult<Vec<_>>>()
            .map_err(|error| error.to_string())?;

        let highest = recorded.last().map(|(version, _)| *version).unwrap_or(0);
        if highest != CURRENT_SCHEMA_VERSION {
            return Err(format!(
                "şema göç tutarsızlığı: kayıtlı en yüksek sürüm {highest}, ama CURRENT_SCHEMA_VERSION {CURRENT_SCHEMA_VERSION} — muhtemelen bir sürüm artırıldı ama schema_migrations satırı eklenmedi"
            ));
        }
        for expected in 1..=CURRENT_SCHEMA_VERSION {
            let entry = recorded.iter().find(|(version, _)| *version == expected);
            match entry {
                None => {
                    return Err(format!(
                        "şema göç boşluğu: sürüm {expected} eksik (1..={CURRENT_SCHEMA_VERSION} arası her sürüm kayıtlı olmalı)"
                    ));
                }
                Some((_, name)) if name.trim().is_empty() => {
                    return Err(format!("şema göç {expected}'in adı boş"));
                }
                Some(_) => {}
            }
        }
        Ok(())
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

    /// F9 "Backup/retention komutları ... ve restore tatbikatı". `backup_to`'nun üstüne üç şey
    /// ekliyor:
    /// 1. **Restore tatbikatı (en kritik):** yedek alınır alınmaz AÇILIP doğrulanıyor — şema göç
    ///    bütünlüğü (`verify_schema_migrations`) VE audit hash-zinciri (`audit_chain_is_valid`)
    ///    geçmezse yedek BOZUK sayılıp siliniyor ve hata dönüyor. Bir yedeğin "gerçekten geri
    ///    yüklenebilir" olduğu, ona güvenmeden ÖNCE kanıtlanıyor — sessizce bozuk bir yedek
    ///    tutup felaket anında işe yaramadığını görmek, hiç yedek olmamaktan beterdir.
    /// 2. **Zaman damgalı isim:** `backups_dir` içinde `jarvis-backup-<epoch>.db` — mevcut bir
    ///    yedeğin üzerine asla yazılmıyor.
    /// 3. **Saklama (retention):** en yeni `retention_count` yedek tutuluyor, daha eskiler
    ///    siliniyor — sınırsız disk büyümesini önler. `retention_count` en az 1'e zorlanıyor
    ///    (yeni alınan yedeği asla hemen silmemek için).
    ///
    /// Döndürülen yol, doğrulanmış yeni yedeğin yolu. Ağ/indirme gerektirmez.
    pub fn create_verified_backup(
        &self,
        backups_dir: &Path,
        retention_count: usize,
    ) -> Result<PathBuf, String> {
        fs::create_dir_all(backups_dir)
            .map_err(|error| format!("yedek dizini oluşturulamadı: {error}"))?;
        let backup_path = backups_dir.join(format!("jarvis-backup-{}.db", now_epoch()));
        if backup_path.exists() {
            return Err(format!(
                "aynı saniyede bir yedek zaten var: {}",
                backup_path.display()
            ));
        }
        self.backup_to(&backup_path)
            .map_err(|error| format!("yedek yazılamadı: {error}"))?;

        // Restore tatbikatı: yedeği aç ve doğrula. Geçmezse sil ve hata ver.
        let verification = (|| -> Result<(), String> {
            let restored = SqliteStore::open(
                backup_path
                    .to_str()
                    .ok_or_else(|| "yedek yolu UTF-8 değil".to_string())?,
            )
            .map_err(|error| format!("yedek açılamadı (restore tatbikatı): {error}"))?;
            restored.verify_schema_migrations()?;
            if !restored
                .audit_chain_is_valid()
                .map_err(|error| error.to_string())?
            {
                return Err("yedeğin audit hash-zinciri doğrulanamadı".into());
            }
            Ok(())
        })();
        if let Err(reason) = verification {
            let _ = fs::remove_file(&backup_path);
            return Err(format!(
                "yedek doğrulanamadı, bozuk yedek silindi: {reason}"
            ));
        }

        self.prune_old_backups(backups_dir, retention_count.max(1))?;
        Ok(backup_path)
    }

    /// `backups_dir` içindeki `jarvis-backup-*.db` dosyalarından en yeni `keep` tanesini tutup
    /// gerisini siler. İsimdeki epoch'a göre değil dosya değişiklik zamanına göre değil —
    /// isimdeki sayısal epoch'a göre sıralıyor (isim, yedeğin alındığı anı taşıyor ve dış
    /// etkenlerden bağımsız).
    fn prune_old_backups(&self, backups_dir: &Path, keep: usize) -> Result<(), String> {
        let mut backups: Vec<(u64, PathBuf)> = fs::read_dir(backups_dir)
            .map_err(|error| format!("yedek dizini okunamadı: {error}"))?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?;
                let epoch = name
                    .strip_prefix("jarvis-backup-")?
                    .strip_suffix(".db")?
                    .parse::<u64>()
                    .ok()?;
                Some((epoch, path))
            })
            .collect();
        // En yeni önce.
        backups.sort_by_key(|(epoch, _)| std::cmp::Reverse(*epoch));
        for (_, path) in backups.into_iter().skip(keep) {
            fs::remove_file(&path)
                .map_err(|error| format!("eski yedek silinemedi ({}): {error}", path.display()))?;
        }
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
