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

/// Shared by the plain-text and PDF indexing paths: a secret-like file name is excluded
/// regardless of format.
pub(crate) fn reject_secret_like_workspace_document_name(path: &Path) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name == ".env"
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.starts_with("id_rsa")
    {
        return Err("workspace secret-like files are excluded from indexing".into());
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
    std::str::from_utf8(bytes)
        .map_err(|error| format!("workspace document must be UTF-8 text: {error}"))?;
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
    /// Excluded because the file name looks like a secret (`.env`, `*.pem`, `*.key`, `id_rsa*`).
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
            if file_name == ".env"
                || file_name.ends_with(".pem")
                || file_name.ends_with(".key")
                || file_name.starts_with("id_rsa")
            {
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
