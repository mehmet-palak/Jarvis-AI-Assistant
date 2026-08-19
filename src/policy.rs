//! The single Policy/Risk Gate.
//!
//! Every capability a `Request` or model-proposed intent can reach is decided here, and only
//! here: `classify` maps free text to a capability ID as a proposal, `validate_*` reject
//! malformed contracts before they can influence a decision, and `policy_for` is the one function
//! that turns a capability into an `Allow`/`Deny`/`AskUser` decision with its required controls.
//! `Runtime` must not grow a second path that decides risk or approval outside this module —
//! that would silently duplicate the Policy Gate the architecture treats as singular.

use crate::now_epoch;
use crate::{
    revalidate_local_attachment, CapabilityRegistry, FeedbackCandidate, FeedbackReview,
    FeedbackSignal, ModelConfigRun, PentestMode, PentestScope, PolicyControl, PolicyDecision,
    PolicyResult, Request, Risk, TeacherExample, VerifyStatus,
};

pub fn classify(input: &str) -> &str {
    let lower = input.to_lowercase();
    if lower.contains("health") || lower.contains("durum") {
        "system.health"
    } else if lower.contains("saat") || lower.contains("zaman") || lower.contains("time") {
        "system.time"
    } else if lower.contains("dosya oku") || lower.contains("file.read") {
        "file.read_workspace"
    } else if lower.contains("proje bilgisi") || lower.contains("project.info") {
        "project.info"
    } else if lower.contains("kod projesi özeti") || lower.contains("code.project_outline") {
        "code.project_outline"
    } else if lower.contains("doküman özeti") || lower.contains("docs.workspace_summary") {
        "docs.workspace_summary"
    } else if lower.contains("not oluştur") || lower.contains("note.create") {
        "note.create"
    } else if lower.contains("file.append_note") {
        "file.append_note"
    } else {
        "unknown"
    }
}

pub fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema_version != 1 {
        return Err(format!(
            "unsupported request schema version: {}",
            request.schema_version
        ));
    }
    if request.request_id.trim().is_empty() {
        return Err("request_id is required".into());
    }
    if request.content.trim().is_empty() {
        return Err("request content is required".into());
    }
    for attachment in &request.attachments {
        revalidate_local_attachment(attachment)?;
    }
    Ok(())
}

pub fn validate_teacher_example(
    example: &TeacherExample,
    registry: &CapabilityRegistry,
) -> Result<(), String> {
    if example.schema_version != 1 {
        return Err(format!(
            "unsupported teacher example schema version: {}",
            example.schema_version
        ));
    }
    if example.example_id.trim().is_empty()
        || example.prompt.trim().is_empty()
        || example.response.trim().is_empty()
        || example.provenance.trim().is_empty()
    {
        return Err("teacher example requires id, prompt, response and provenance".into());
    }
    if !registry.contains(&example.expected_capability) {
        return Err("teacher example capability is not registered".into());
    }
    if example.verifier_status != VerifyStatus::Pass {
        return Err("teacher example verifier status must be PASS".into());
    }
    if example.evidence.is_empty() || example.evidence.iter().any(|item| item.trim().is_empty()) {
        return Err("teacher example requires non-empty verifier evidence".into());
    }
    if !example.human_reviewed {
        return Err("teacher example requires human review".into());
    }
    Ok(())
}

/// F6 feedback intake validation. The rule the plan states — feedback never becomes training
/// data directly — is enforced here as a contract, not left to callers.
pub fn validate_feedback_candidate(candidate: &FeedbackCandidate) -> Result<(), String> {
    if candidate.schema_version != 1 {
        return Err(format!(
            "unsupported feedback candidate schema version: {}",
            candidate.schema_version
        ));
    }
    if candidate.candidate_id.trim().is_empty()
        || candidate.prompt.trim().is_empty()
        || candidate.response.trim().is_empty()
        || candidate.provenance.trim().is_empty()
    {
        return Err("feedback candidate requires id, prompt, response and provenance".into());
    }
    // A correction that carries no corrected text is not a correction; storing it would create a
    // review-queue item nobody can act on.
    if candidate.signal == FeedbackSignal::Correction && candidate.correction.trim().is_empty() {
        return Err("a correction signal requires the corrected text".into());
    }
    if candidate.signal != FeedbackSignal::Correction && !candidate.correction.trim().is_empty() {
        return Err("only a correction signal may carry corrected text".into());
    }
    Ok(())
}

/// The single gate that decides whether a reviewed candidate may become a `TeacherExample`.
/// Kept next to the other policy decisions so "what is allowed to become training data" has one
/// answer in one place, exactly like `policy_for` does for capabilities.
pub fn feedback_candidate_is_promotable(candidate: &FeedbackCandidate) -> Result<(), String> {
    validate_feedback_candidate(candidate)?;
    if candidate.review != FeedbackReview::Approved {
        return Err("only a human-approved feedback candidate can become training data".into());
    }
    // Sensitive user content must never leave the machine inside a dataset export, and a dataset
    // is exactly the artifact most likely to be copied elsewhere.
    if candidate.sensitivity == crate::DataSensitivity::Sensitive {
        return Err("a sensitivity=Sensitive candidate is never eligible as training data".into());
    }
    // A negative signal says "this answer was wrong" — it identifies a problem, but carries no
    // correct answer to learn from. Only a positive example or an explicit correction does.
    if candidate.signal == FeedbackSignal::Negative {
        return Err("a negative signal alone carries no correct answer to learn from".into());
    }
    Ok(())
}

/// F6 registry contract validation. A run row is evidence about a model/prompt change, so an
/// unattributable or self-contradictory row is rejected outright rather than stored and later
/// mistaken for a real measurement.
pub fn validate_model_config_run(run: &ModelConfigRun) -> Result<(), String> {
    if run.schema_version != 1 {
        return Err(format!(
            "unsupported model config run schema version: {}",
            run.schema_version
        ));
    }
    if run.run_id.trim().is_empty()
        || run.provider_id.trim().is_empty()
        || run.model_id.trim().is_empty()
    {
        return Err("model config run requires id, provider and model".into());
    }
    // Without both fingerprints a row cannot answer "was this the same model and prompt?", which
    // is the only question the registry exists to answer.
    if run.model_fingerprint.trim().is_empty() || run.prompt_fingerprint.trim().is_empty() {
        return Err("model config run requires model and prompt fingerprints".into());
    }
    if run.scenarios_passed == 0 && run.scenarios_failed == 0 {
        return Err("model config run requires at least one evaluated scenario".into());
    }
    if run
        .rollback_target
        .as_ref()
        .is_some_and(|target| target.trim().is_empty())
    {
        return Err("model config run rollback target must not be blank".into());
    }
    if run.rollback_target.as_deref() == Some(run.run_id.as_str()) {
        return Err("model config run cannot roll back to itself".into());
    }
    Ok(())
}

/// Validates a machine-readable pentest authorization scope before any security capability can be
/// considered. This core intentionally performs no network activity and treats all targets as
/// exact ASCII host/IP identifiers; CIDR, wildcard and DNS-pinning support are future contracts.
pub fn validate_pentest_scope(scope: &PentestScope) -> Result<(), String> {
    if scope.schema_version != 1 {
        return Err(format!(
            "unsupported pentest scope schema version: {}",
            scope.schema_version
        ));
    }
    if scope.authorization_ref.trim().is_empty() {
        return Err("pentest scope requires an authorization reference".into());
    }
    if scope.expires_at <= now_epoch() {
        return Err("pentest scope is expired".into());
    }
    if scope.targets.is_empty() {
        return Err("pentest scope requires at least one allowlisted target".into());
    }
    if scope.max_runtime_seconds == 0 {
        return Err("pentest scope requires a positive runtime limit".into());
    }
    for target in scope.targets.iter().chain(&scope.excluded_targets) {
        normalize_pentest_target(target)?;
    }
    Ok(())
}

pub fn authorize_pentest_target(
    scope: &PentestScope,
    target: &str,
    requested_mode: PentestMode,
) -> Result<(), String> {
    validate_pentest_scope(scope)?;
    let target = normalize_pentest_target(target)?;
    let excluded = scope
        .excluded_targets
        .iter()
        .map(|item| normalize_pentest_target(item))
        .collect::<Result<Vec<_>, _>>()?;
    if excluded.iter().any(|item| item == &target) {
        return Err("pentest target is explicitly excluded by scope".into());
    }
    let allowed = scope
        .targets
        .iter()
        .map(|item| normalize_pentest_target(item))
        .collect::<Result<Vec<_>, _>>()?;
    if !allowed.iter().any(|item| item == &target) {
        return Err("pentest target is outside the authorization allowlist".into());
    }
    if requested_mode > scope.maximum_mode {
        return Err("requested pentest mode exceeds the authorization scope".into());
    }
    Ok(())
}

fn normalize_pentest_target(target: &str) -> Result<String, String> {
    let target = target.trim().trim_end_matches('.').to_ascii_lowercase();
    if target.is_empty()
        || !target.is_ascii()
        || target.contains('*')
        || target.contains('/')
        || target.contains(':')
        || target.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || label.starts_with("xn--")
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err("pentest target must be an exact ASCII hostname or IPv4 address".into());
    }
    Ok(target)
}

pub fn policy_for(capability: &str, _input: &str) -> PolicyResult {
    match capability {
        "conversation.reply" => PolicyResult {
            decision: PolicyDecision::Allow,
            risk: Risk::Low,
            reason: "local non-action conversation".into(),
            approval_required: false,
            required_controls: vec![PolicyControl::AuditRequired],
        },
        "system.health" => PolicyResult {
            decision: PolicyDecision::Allow,
            risk: Risk::Low,
            reason: "read-only local status".into(),
            approval_required: false,
            required_controls: vec![
                PolicyControl::VerifierRequired,
                PolicyControl::AuditRequired,
                PolicyControl::ReadOnlyFilesystem,
            ],
        },
        "system.time" => PolicyResult {
            decision: PolicyDecision::Allow,
            risk: Risk::Low,
            reason: "read-only local time".into(),
            approval_required: false,
            required_controls: vec![
                PolicyControl::VerifierRequired,
                PolicyControl::AuditRequired,
                PolicyControl::ReadOnlyFilesystem,
            ],
        },
        "file.read_workspace"
        | "project.info"
        | "code.project_outline"
        | "docs.workspace_summary" => PolicyResult {
            // These actions do not write or execute, but their results may disclose the
            // user's private project data. An explicit, task-bound approval also prevents an
            // injected model intent from becoming a silent workspace read.
            decision: PolicyDecision::AskUser,
            risk: Risk::Medium,
            reason: "Özel çalışma alanına erişim için açık kullanıcı onayı gerekir.".into(),
            approval_required: true,
            required_controls: vec![
                PolicyControl::UserApproval,
                PolicyControl::ExplainBeforeExecute,
                PolicyControl::VerifierRequired,
                PolicyControl::AuditRequired,
                PolicyControl::ReadOnlyFilesystem,
            ],
        },
        "note.create" => PolicyResult {
            decision: PolicyDecision::AskUser,
            risk: Risk::Medium,
            reason: "Kalıcı bir not dosyası oluşturur.".into(),
            approval_required: true,
            required_controls: vec![
                PolicyControl::UserApproval,
                PolicyControl::ExplainBeforeExecute,
                PolicyControl::VerifierRequired,
                PolicyControl::AuditRequired,
            ],
        },
        "file.append_note" => PolicyResult {
            decision: PolicyDecision::AskUser,
            risk: Risk::Medium,
            reason: "Var olan bir dosyaya kalıcı bir satır ekler.".into(),
            approval_required: true,
            required_controls: vec![
                PolicyControl::UserApproval,
                PolicyControl::ExplainBeforeExecute,
                PolicyControl::VerifierRequired,
                PolicyControl::AuditRequired,
            ],
        },
        _ => PolicyResult {
            decision: PolicyDecision::Deny,
            risk: Risk::High,
            reason: "unknown capability".into(),
            approval_required: false,
            required_controls: vec![PolicyControl::AuditRequired],
        },
    }
}
