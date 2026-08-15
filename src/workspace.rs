//! Workspace/RAG document indexing: folder-scope preview, per-file path/content validation,
//! PDF text extraction, chunking, and full-text search query building.
//!
//! Every document ever offered to the model goes through here first and arrives tagged as
//! `ContentProvenance::UntrustedProjectFile` (`WorkspaceCitation::as_untrusted_content`) — this
//! module has no authority to change that; it only decides what is *eligible* to be indexed at
//! all (contained under an approved root, not secret-like, not oversized, real UTF-8 text or a
//! parseable PDF).

use std::fs;
use std::path::{Path, PathBuf};

use crate::{ContentProvenance, ContentRef};

pub const MAX_WORKSPACE_DOCUMENT_BYTES: u64 = 512 * 1024;
pub const MAX_WORKSPACE_CHUNK_CHARS: usize = 1_200;

/// F3 "Metadata/FTS index: ... indeks sürümü". The chunking/extraction algorithm's own version,
/// persisted per document (`workspace_documents.index_schema_version`) — distinct from the
/// SQLite column-migration version (`persistence::CURRENT_SCHEMA_VERSION`). If a future JARVIS
/// build changes how documents get chunked, bumping this forces every existing document to be
/// treated as needing re-indexing even when its raw content hash has not changed, because the
/// *derived* chunks would otherwise be stale relative to the new algorithm.
pub const CURRENT_WORKSPACE_INDEX_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIngestionReport {
    pub schema_version: u16,
    pub document_id: String,
    pub canonical_path: PathBuf,
    pub content_sha256: String,
    pub chunk_count: usize,
    pub indexed_at: u64,
    /// F3 "Ingestion pipeline: ... dosya değişiklik algısı ve incremental re-index". `false`
    /// means this path's content hash matched what was already indexed, so the (potentially
    /// expensive) chunk delete/re-insert was skipped entirely; `indexed_at` is the *previous*
    /// indexing time in that case, not "now".
    pub content_changed: bool,
}

/// Result of indexing every file `preview_workspace_index` reported as `included`. A per-file
/// failure (e.g. it turned out to be binary once actually opened, even though the metadata-only
/// preview didn't know that) does not abort the rest of the folder.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceFolderIndexReport {
    pub indexed: Vec<WorkspaceIngestionReport>,
    pub failed: Vec<(PathBuf, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCitation {
    pub document_id: String,
    pub chunk_id: String,
    pub canonical_path: PathBuf,
    pub content_sha256: String,
    pub chunk_ordinal: usize,
    pub content: String,
}

impl WorkspaceCitation {
    pub fn as_untrusted_content(&self) -> ContentRef {
        ContentRef {
            source: format!(
                "workspace:{}#chunk-{}",
                self.canonical_path.display(),
                self.chunk_ordinal
            ),
            provenance: ContentProvenance::UntrustedProjectFile,
            content: self.content.clone(),
        }
    }
}

pub(crate) fn validate_workspace_document_path(
    root: &Path,
    requested: &Path,
) -> Result<PathBuf, String> {
    let root =
        fs::canonicalize(root).map_err(|error| format!("workspace root unavailable: {error}"))?;
    if !root.is_dir() {
        return Err("workspace root must be a directory".into());
    }
    if requested.is_absolute()
        || requested
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("workspace document path must be contained and relative".into());
    }
    let canonical = fs::canonicalize(root.join(requested))
        .map_err(|error| format!("workspace document cannot be resolved: {error}"))?;
    if !canonical.starts_with(&root) {
        return Err("workspace document escapes its approved root".into());
    }
    Ok(canonical)
}

/// F3 "Secret/hassas filtre" — the fixed, low-maintenance filename markers a credential-shaped
/// file tends to have. Deliberately name/extension-based, not a guess at file *meaning*: exact
/// names for well-known credential-store files, prefixes that catch a whole family (`.env`,
/// `.env.local`, `.env.production`; `id_rsa`, `id_ed25519`, ...), suffixes for key/cert
/// container formats. Single source of truth — both `reject_secret_like_workspace_document_name`
/// and the folder-level preview read this same list, so they can never silently drift apart.
const SECRET_LIKE_EXACT_NAMES: &[&str] = &[
    "credentials",
    "credentials.json",
    "secrets.yaml",
    "secrets.yml",
    ".secrets",
];
const SECRET_LIKE_NAME_PREFIXES: &[&str] = &[
    ".env",
    ".netrc",
    ".npmrc",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
];
const SECRET_LIKE_NAME_SUFFIXES: &[&str] =
    &[".env", ".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"];

fn is_secret_like_file_name(file_name_lowercase: &str) -> bool {
    SECRET_LIKE_EXACT_NAMES.contains(&file_name_lowercase)
        || SECRET_LIKE_NAME_PREFIXES
            .iter()
            .any(|prefix| file_name_lowercase.starts_with(prefix))
        || SECRET_LIKE_NAME_SUFFIXES
            .iter()
            .any(|suffix| file_name_lowercase.ends_with(suffix))
}

/// Fixed rejection reasons the caller (`Runtime::index_workspace_document`) matches on to decide
/// whether a failed indexing attempt gets the dedicated `workspace.index.rejected_secret_like`
/// audit event instead of the generic one — never by re-deriving the check, only by comparing
/// against these same constants, so the audit classification can never drift from the actual
/// filter.
pub const SECRET_LIKE_NAME_REJECTION: &str =
    "workspace secret-like files are excluded from indexing";
pub const SECRET_LIKE_CONTENT_REJECTION: &str =
    "workspace document contains a likely embedded credential and is excluded from indexing";

/// True for either flavor of secret-like rejection message (`SECRET_LIKE_NAME_REJECTION` or
/// `SECRET_LIKE_CONTENT_REJECTION`) — used only for audit-event classification, never for control
/// flow that changes what gets indexed.
pub(crate) fn is_secret_like_rejection(error: &str) -> bool {
    error == SECRET_LIKE_NAME_REJECTION || error == SECRET_LIKE_CONTENT_REJECTION
}

/// Shared by the plain-text and PDF indexing paths: a secret-like file name is excluded
/// regardless of format.
pub(crate) fn reject_secret_like_workspace_document_name(path: &Path) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if is_secret_like_file_name(&file_name) {
        return Err(SECRET_LIKE_NAME_REJECTION.into());
    }
    Ok(())
}

/// F3 "Secret/hassas filtre" — a credential can be *pasted inside* an otherwise ordinary file
/// (notes, a script, a log) that no filename check would ever catch. This scans already-decoded
/// text (plain-text or PDF-extracted) for a small set of high-confidence, low-false-positive
/// markers: PEM private-key block headers and well-known token prefixes that are essentially
/// never written as prose. No regex dependency, matching this module's existing simple-substring
/// style; deliberately narrow (a generic word like "password" is not included) because a false
/// positive here silently blocks a legitimate document from being indexed at all.
const SECRET_LIKE_CONTENT_MARKERS: &[&str] = &[
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "-----BEGIN DSA PRIVATE KEY-----",
    "-----BEGIN PGP PRIVATE KEY BLOCK-----",
    "AWS_SECRET_ACCESS_KEY",
    "AKIA",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
];

pub(crate) fn reject_secret_like_workspace_document_content(content: &str) -> Result<(), String> {
    if SECRET_LIKE_CONTENT_MARKERS
        .iter()
        .any(|marker| content.contains(marker))
    {
        return Err(SECRET_LIKE_CONTENT_REJECTION.into());
    }
    Ok(())
}

/// Shared by the plain-text and PDF indexing paths: the on-disk byte size limit applies before
/// any parsing (a PDF is parsed only after this passes, so an oversized file is never even
/// handed to the PDF parser).
pub(crate) fn reject_oversized_workspace_document(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_WORKSPACE_DOCUMENT_BYTES {
        return Err(format!(
            "workspace document exceeds {} KiB indexing limit",
            MAX_WORKSPACE_DOCUMENT_BYTES / 1024
        ));
    }
    Ok(())
}

pub(crate) fn validate_workspace_document_content(path: &Path, bytes: &[u8]) -> Result<(), String> {
    reject_secret_like_workspace_document_name(path)?;
    reject_oversized_workspace_document(bytes)?;
    if bytes.contains(&0) {
        return Err("binary workspace documents are excluded from indexing".into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("workspace document must be UTF-8 text: {error}"))?;
    reject_secret_like_workspace_document_content(text)?;
    Ok(())
}

/// F3 "Document parser katmanı: Markdown/TXT/PDF başlangıcı". Extracts plain text from a PDF's
/// bytes for indexing. Wrapped in `catch_unwind`: PDF parsers are a well-known crash surface on
/// malformed/adversarial input, and a single bad PDF must never take down the whole JARVIS
/// process just because the user tried to index it — this is this project's "sandbox decision"
/// for the PDF parser specifically (in-process, panic-isolated, not a separate process/container;
/// Office/HTML parsers get their own sandbox decision when they're added, per this same item).
pub(crate) fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes)
    })) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(error)) => Err(format!("PDF text extraction failed: {error}")),
        Err(_) => Err("PDF text extraction crashed on a malformed file".into()),
    }
}

/// A cheap, metadata-only scan of a folder a user is considering for indexing — the "izin UX'i"
/// preview shown before folder-level indexing runs. It never opens a file's content (that full
/// secret/binary/UTF-8/size check still runs per-file in `index_workspace_document` at actual
/// indexing time); this only reports scope so the user can decide before anything is read.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceIndexPreview {
    pub root: PathBuf,
    /// Relative paths that would currently be eligible for indexing.
    pub included: Vec<PathBuf>,
    /// Excluded because the file name looks like a secret/credential store (see
    /// `SECRET_LIKE_EXACT_NAMES`/`SECRET_LIKE_NAME_PREFIXES`/`SECRET_LIKE_NAME_SUFFIXES`).
    pub excluded_secret_like: Vec<PathBuf>,
    /// Excluded because the file is larger than `MAX_WORKSPACE_DOCUMENT_BYTES`.
    pub excluded_oversized: Vec<PathBuf>,
    /// Excluded because it matched a caller-supplied exclude pattern.
    pub excluded_by_pattern: Vec<PathBuf>,
    /// Sum of `included` file sizes — a size estimate, not an exact post-chunking count.
    pub estimated_total_bytes: u64,
}

/// Directories never worth indexing regardless of user-supplied exclude patterns: version
/// control internals and dependency/build caches. The user can still index files inside them by
/// pointing `/index` directly at one; this only affects the folder-level preview/scan default.
const WORKSPACE_INDEX_SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".venv"];

/// True if `relative` matches a caller-supplied exclude pattern. Deliberately simple — a bare
/// `*.ext` matches by extension, anything else matches by substring containment — rather than a
/// full glob engine, so this has no new dependency and stays predictable to explain to a user.
fn path_matches_exclude_pattern(relative: &Path, pattern: &str) -> bool {
    if let Some(extension) = pattern.strip_prefix("*.") {
        return relative
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension));
    }
    relative.to_string_lossy().contains(pattern)
}

/// Scans `root` (which must already be a real, contained directory — same containment rule as
/// `validate_workspace_document_path`) and reports what folder-level indexing would include.
pub fn preview_workspace_index(
    root: &Path,
    exclude_patterns: &[String],
) -> Result<WorkspaceIndexPreview, String> {
    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("workspace root unavailable: {error}"))?;
    if !canonical_root.is_dir() {
        return Err("workspace root must be a directory".into());
    }
    let mut preview = WorkspaceIndexPreview {
        root: canonical_root.clone(),
        ..Default::default()
    };
    let mut pending_dirs = vec![canonical_root.clone()];
    while let Some(directory) = pending_dirs.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("workspace directory unreadable: {error}"))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("workspace directory entry error: {error}"))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(&canonical_root)
                .unwrap_or(&path)
                .to_path_buf();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("workspace entry type unknown: {error}"))?;
            if file_type.is_dir() {
                let is_skipped_dir = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| WORKSPACE_INDEX_SKIP_DIRS.contains(&name));
                if !is_skipped_dir {
                    pending_dirs.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue; // symlinks and other special entries are not indexed
            }
            if exclude_patterns
                .iter()
                .any(|pattern| path_matches_exclude_pattern(&relative, pattern))
            {
                preview.excluded_by_pattern.push(relative);
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if is_secret_like_file_name(&file_name) {
                preview.excluded_secret_like.push(relative);
                continue;
            }
            let byte_size = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            if byte_size > MAX_WORKSPACE_DOCUMENT_BYTES {
                preview.excluded_oversized.push(relative);
                continue;
            }
            preview.estimated_total_bytes += byte_size;
            preview.included.push(relative);
        }
    }
    preview.included.sort();
    preview.excluded_secret_like.sort();
    preview.excluded_oversized.sort();
    preview.excluded_by_pattern.sort();
    Ok(preview)
}

pub(crate) fn chunk_workspace_text(content: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in content.lines() {
        let needed = line.chars().count() + usize::from(!current.is_empty());
        if !current.is_empty() && current.chars().count() + needed > MAX_WORKSPACE_CHUNK_CHARS {
            chunks.push(current);
            current = String::new();
        }
        if line.chars().count() > MAX_WORKSPACE_CHUNK_CHARS {
            for segment in line
                .chars()
                .collect::<Vec<_>>()
                .chunks(MAX_WORKSPACE_CHUNK_CHARS)
            {
                if !current.is_empty() {
                    chunks.push(current);
                    current = String::new();
                }
                chunks.push(segment.iter().collect());
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

pub(crate) fn fts_query(query: &str) -> Result<String, String> {
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2)
        .take(12)
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err("workspace search query needs at least one two-character term".into());
    }
    // Natural-language queries contain stop words and inflections (for example Turkish
    // question suffixes). Requiring every term causes valid sources to disappear; each term is
    // still quoted and parameter-bound, so broadening retrieval does not broaden authority.
    Ok(terms.join(" OR "))
}
