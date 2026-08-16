//! F4 "Patch generator": turns an already-approved-scope `CodingPlan` into a real, machine-
//! validated `PatchProposal` — the one piece of F4's pipeline that actually asks the model to
//! produce code.
//!
//! Deliberate design choice: the model is never asked to emit diff/hunk syntax itself. It is
//! asked for the complete new content of one file at a time; the real unified diff is then
//! computed deterministically by `workbench::generate_unified_diff_for_file` (`git diff
//! --no-index`), and the whole result still has to pass `create_patch_proposal`'s existing
//! validation (path containment, plan membership, diff/file-count/byte limits) before it is ever
//! handed back. A small local model getting one line number wrong in a hand-authored diff hunk
//! makes the whole patch unappliable; getting one line wrong in a full-file rewrite just means
//! that one line is wrong, and the same validation that already exists catches anything the model
//! gets structurally wrong (a hallucinated path, an oversized file, a plan not otherwise scoped).

use std::fs;
use std::path::Path;

use crate::workbench::generate_unified_diff_for_file;
use crate::{create_patch_proposal, CodingPlan, ModelProvider, PatchProposal};

/// A model response of exactly this (trimmed) is treated as "no change needed in this file",
/// never as a zero-byte rewrite.
const NO_CHANGE_SENTINEL: &str = "NO_CHANGE";

/// `RepoOverview`'s general 512 KiB ceiling (`MAX_WORKSPACE_DOCUMENT_BYTES`) exists for read-only
/// scanning/RAG, where the model never has to hold the whole file in its own context window. A
/// full-file rewrite is different: the *entire* current content has to fit in the prompt AND the
/// model has to emit the *entire* new content back — both inside one context window. This
/// server's local model currently runs with an 8192-token context (see `docs/adr/0001-...` "Ek"
/// for why it was bumped from 2048, 16 Ağustos 2026); ~8000 bytes of code is roughly 2000 tokens,
/// leaving comfortable room for the prompt wrapper and a same-sized rewrite in response. A file
/// over this ceiling is skipped with a clear error rather than silently truncated.
const MAX_PATCH_GENERATOR_FILE_BYTES: u64 = 8_000;

/// Small local models habitually wrap output in a markdown code fence even when told not to —
/// this strips one outer fence (```/```lang ... ```) if present, leaving the content untouched
/// otherwise. Never removes fences that appear *inside* otherwise-plain content, only a fence
/// that spans the entire response.
fn strip_outer_code_fence(text: &str) -> String {
    let trimmed = text.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    let after_open = match after_open.find('\n') {
        Some(newline_index) => &after_open[newline_index + 1..],
        None => after_open,
    };
    match after_open.strip_suffix("```") {
        Some(inner) => inner.trim_end().to_string(),
        None => trimmed.to_string(),
    }
}

fn read_current_content(workspace_root: &Path, relative_path: &Path) -> Result<String, String> {
    let absolute = workspace_root.join(relative_path);
    let bytes = fs::read(&absolute).map_err(|error| {
        format!(
            "patch generator cannot read {}: {error}",
            relative_path.display()
        )
    })?;
    if bytes.len() as u64 > MAX_PATCH_GENERATOR_FILE_BYTES {
        return Err(format!(
            "{} exceeds the {} KB size ceiling for a model-drafted full-file rewrite (this is \
             smaller than the general workspace read ceiling — the whole file has to fit in the \
             model's own context window, not just be readable)",
            relative_path.display(),
            MAX_PATCH_GENERATOR_FILE_BYTES / 1000
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        format!(
            "{} is not valid UTF-8 text; only text files can be patched",
            relative_path.display()
        )
    })
}

fn drafting_prompt(relative_path: &Path, current_content: &str, request_summary: &str) -> String {
    format!(
        "/no_think You are a careful coding assistant working on exactly one existing file in a local repository. \
You never see or touch any other file. Output ONLY the complete new content of this file after applying the \
requested change — nothing before or after it: no explanation, no markdown code fences, no commentary, just the \
raw file content from its first character to its last. If, having read the file, no change to THIS specific file \
is actually needed to satisfy the request, output exactly the single line `{NO_CHANGE_SENTINEL}` and nothing else. \
Preserve everything about the file you are not intentionally changing — indentation, comments, unrelated code.\n\n\
File path: {}\n\
Current content:\n{current_content}\n\n\
Change request: {request_summary}",
        relative_path.display()
    )
}

/// Drafts a real patch for every file in `plan.affected_files`, one model call per file. A file
/// the model marks `NO_CHANGE` (or whose proposed content turns out identical to the current
/// content) is simply left out of the result — this is not an error, it is the model correctly
/// deciding that file needs no edit. An error is only returned when *no* file ends up changed at
/// all (an empty patch is not a valid `PatchProposal` — `create_patch_proposal` itself would
/// reject it), or when a per-file model call itself fails.
pub fn draft_patch_with_provider(
    plan: &CodingPlan,
    provider: &dyn ModelProvider,
) -> Result<PatchProposal, String> {
    if plan.affected_files.is_empty() {
        return Err(
            "coding plan has no affected files; run /plan again with a more specific request"
                .into(),
        );
    }
    let mut combined_diff = String::new();
    let mut changed_files = Vec::new();
    for relative_path in &plan.affected_files {
        let current_content = read_current_content(&plan.workspace_root, relative_path)?;
        let prompt = drafting_prompt(relative_path, &current_content, &plan.request_summary);
        // A rewrite is usually close in size to the original — budget generously around that
        // (roughly 3 bytes/token for source code, plus headroom) rather than reusing a fixed
        // classification-sized budget that would truncate any real file.
        let token_budget = ((current_content.len() as u32 / 3) + 200).clamp(64, 4_000) as u16;
        let response = provider
            .complete_with_budget(&prompt, token_budget)
            .map_err(|error| {
                format!(
                    "patch draft failed for {}: {error}",
                    relative_path.display()
                )
            })?;
        let mut candidate = strip_outer_code_fence(&response.text);
        if candidate.trim() == NO_CHANGE_SENTINEL {
            continue;
        }
        // Chat-completion responses routinely drop a trailing newline the raw file actually had
        // (observed live against the real model, 16 Ağustos 2026) — restore it rather than let
        // every rewrite silently strip the file's original POSIX text-file convention.
        if current_content.ends_with('\n') && !candidate.ends_with('\n') {
            candidate.push('\n');
        }
        let Some(diff) =
            generate_unified_diff_for_file(relative_path, &current_content, &candidate)?
        else {
            continue; // model's rewrite was byte-identical to the current content — no real change
        };
        combined_diff.push_str(&diff);
        changed_files.push(relative_path.clone());
    }
    if changed_files.is_empty() {
        return Err(
            "the model proposed no actual changes for this request across every affected file"
                .into(),
        );
    }
    create_patch_proposal(plan, combined_diff, changed_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_read_only_coding_plan, ModelResponse};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Replies in order, one per `provider.complete` call — lets a test drive a distinct response
    /// per affected file without depending on prompt content.
    struct ScriptedProvider {
        replies: std::sync::Mutex<Vec<&'static str>>,
    }

    impl ScriptedProvider {
        fn new(replies: Vec<&'static str>) -> Self {
            Self {
                replies: std::sync::Mutex::new(replies),
            }
        }
    }

    impl ModelProvider for ScriptedProvider {
        fn provider_id(&self) -> &str {
            "test"
        }
        fn model_id(&self) -> &str {
            "scripted"
        }
        fn complete(&self, _prompt: &str) -> Result<ModelResponse, String> {
            let mut replies = self.replies.lock().expect("lock");
            if replies.is_empty() {
                return Err("scripted provider ran out of replies".into());
            }
            let text = replies.remove(0);
            Ok(ModelResponse {
                provider_id: self.provider_id().into(),
                model_id: self.model_id().into(),
                text: text.into(),
                structured_json: None,
                finish_reason: "stop".into(),
            })
        }
    }

    fn fixture_workspace(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "jarvis-patch-generator-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("fixture root");
        for (path, content) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, content).unwrap();
        }
        root
    }

    #[test]
    fn a_full_file_rewrite_from_the_model_becomes_a_validated_patch_proposal() {
        let root = fixture_workspace(
            "rewrite",
            &[("src/greet.rs", "fn greet() {\n    \"hi\"\n}\n")],
        );
        let plan = create_read_only_coding_plan(
            &root,
            "greet fonksiyonunu selamlama mesajını değiştir",
            vec![PathBuf::from("src/greet.rs")],
            vec!["cargo test".into()],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec!["fn greet() {\n    \"merhaba\"\n}\n"]);

        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch is drafted");
        assert_eq!(proposal.affected_files, vec![PathBuf::from("src/greet.rs")]);
        assert!(proposal.unified_diff.contains("-    \"hi\""));
        assert!(proposal.unified_diff.contains("+    \"merhaba\""));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_markdown_fenced_reply_is_unwrapped_before_diffing() {
        let root = fixture_workspace("fenced", &[("src/x.rs", "old\n")]);
        let plan = create_read_only_coding_plan(
            &root,
            "x dosyasını güncelle",
            vec![PathBuf::from("src/x.rs")],
            vec![],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec!["```rust\nnew\n```"]);

        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch is drafted");
        assert!(proposal.unified_diff.contains("+new"));
        assert!(!proposal.unified_diff.contains("```"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_no_change_reply_for_every_file_is_an_error_not_an_empty_patch() {
        let root = fixture_workspace("nochange", &[("src/x.rs", "same\n")]);
        let plan = create_read_only_coding_plan(
            &root,
            "belirsiz istek",
            vec![PathBuf::from("src/x.rs")],
            vec![],
        )
        .expect("valid plan");
        let provider = ScriptedProvider::new(vec!["NO_CHANGE"]);

        let result = draft_patch_with_provider(&plan, &provider);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no actual changes"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_multi_file_plan_only_includes_files_that_actually_changed() {
        let root = fixture_workspace(
            "multi",
            &[("src/a.rs", "fn a() {}\n"), ("src/b.rs", "fn b() {}\n")],
        );
        let plan = create_read_only_coding_plan(
            &root,
            "a fonksiyonunu değiştir, b'ye dokunma",
            vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")],
            vec![],
        )
        .expect("valid plan");
        // a.rs değişiyor, b.rs için model NO_CHANGE diyor.
        let provider = ScriptedProvider::new(vec!["fn a() {\n    1\n}\n", "NO_CHANGE"]);

        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch is drafted");
        assert_eq!(proposal.affected_files, vec![PathBuf::from("src/a.rs")]);

        fs::remove_dir_all(&root).ok();
    }

    /// Chat-completion output routinely drops a trailing newline the original file actually had
    /// (observed live against the real model). A rewrite must not silently strip that convention.
    #[test]
    fn a_dropped_trailing_newline_in_the_models_reply_is_restored() {
        let root = fixture_workspace("newline", &[("src/x.rs", "fn x() {}\n")]);
        let plan = create_read_only_coding_plan(
            &root,
            "x fonksiyonunu değiştir",
            vec![PathBuf::from("src/x.rs")],
            vec![],
        )
        .expect("valid plan");
        // Kasıtlı olarak sondaki newline'sız bir yanıt — modelin gerçek davranışını taklit ediyor.
        let provider = ScriptedProvider::new(vec!["fn x() {\n    1\n}"]);

        let proposal = draft_patch_with_provider(&plan, &provider).expect("patch is drafted");
        assert!(!proposal.unified_diff.contains("No newline at end of file"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_empty_plan_is_rejected_before_any_model_call() {
        let root = fixture_workspace("empty", &[]);
        let plan = create_read_only_coding_plan(&root, "istek belirsiz", vec![], vec![])
            .expect("valid plan");
        let provider = ScriptedProvider::new(vec![]);

        let result = draft_patch_with_provider(&plan, &provider);
        assert!(result.is_err());

        fs::remove_dir_all(&root).ok();
    }
}
