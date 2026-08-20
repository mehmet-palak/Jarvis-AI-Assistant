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

/// F5 "Sesli approval UX: yüksek riskli aksiyon için yalnız ses değil, ekranda açık yazılı onay
/// veya güvenli ikinci doğrulama."
///
/// Speech is a weaker authorization channel than a keypress: it can be misheard, it can be
/// produced by someone else in the room, and it can be replayed from a recording. This gate is
/// therefore about *how the approval arrived*, not about what the capability does — the
/// capability's own risk still comes from `policy_for`, and this never widens it.
///
/// The rule is one-directional on purpose: voice may always *decline*, and may approve anything
/// that `policy_for` already treats as approval-free. What it may not do is be the sole
/// authorization for an action the policy gate already decided needs an explicit human approval.
pub fn voice_approval_is_sufficient(capability: &str, input: &str) -> bool {
    !policy_for(capability, input).approval_required
}

/// The confirmation an approval must carry to be accepted. Returned rather than a bare bool so
/// the UI can say *why* it is asking for a keypress instead of silently ignoring the spoken
/// "evet" — an approval prompt the user does not understand is its own security problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChannelRequirement {
    /// Any channel, including voice, may approve.
    AnyChannel,
    /// Voice alone is not enough: the user must confirm on screen.
    WrittenConfirmationRequired,
}

pub fn approval_channel_requirement(
    capability: &str,
    input: &str,
    origin: crate::InputType,
) -> ApprovalChannelRequirement {
    if origin != crate::InputType::Voice || voice_approval_is_sufficient(capability, input) {
        ApprovalChannelRequirement::AnyChannel
    } else {
        ApprovalChannelRequirement::WrittenConfirmationRequired
    }
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
        parse_pentest_target_pattern(target)?;
    }
    Ok(())
}

/// F7.1 "CIDR, wildcard, punycode ve DNS pinning/rebinding savunması — bug bounty scope'u
/// ifade etmek için zorunlu." A bug bounty program's scope is almost never a flat list of exact
/// hostnames; it is expressed with wildcards (`*.example.com`) and IP ranges (`10.0.0.0/24`).
/// The scope *list* therefore holds patterns, but the *target actually under test* is always
/// concrete — you probe one real host, never a pattern — so the two are validated differently:
/// `parse_pentest_target_pattern` accepts patterns for `scope.targets`/`excluded_targets`,
/// `normalize_pentest_target` (below) stays exact-only for the live target argument.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PentestTargetPattern {
    /// Exact ASCII hostname or IPv4 address, canonical form (lowercase, no trailing dot).
    ExactHost(String),
    /// `*.base.tld` — matches any strict subdomain of `base`, never `base` itself. List the
    /// apex separately (e.g. both `example.com` and `*.example.com`) if it is also in scope;
    /// treating a wildcard as covering its own apex is a common, dangerous over-grant.
    Wildcard { base: String },
    /// IPv4 CIDR range, stored as the network address (host bits already verified zero) and
    /// prefix length.
    Cidr { network: u32, prefix_len: u8 },
}

/// Narrower than a `/16` is rejected as a single scope entry. This is a deliberate safety bound,
/// not a technical limit: a typo'd or overly generous authorization (`10.0.0.0/8` instead of
/// `10.0.0.0/24`) would silently put ~16 million addresses in scope. A program that genuinely
/// authorizes something broader lists multiple `/16` (or narrower) entries instead — that keeps
/// the size of what one line of scope grants bounded and reviewable.
const MIN_PENTEST_CIDR_PREFIX_LEN: u8 = 16;

fn parse_pentest_target_pattern(raw: &str) -> Result<PentestTargetPattern, String> {
    let raw = raw.trim();
    if let Some((network_part, prefix_part)) = raw.split_once('/') {
        let network = parse_canonical_ipv4(network_part)
            .ok_or_else(|| "pentest CIDR target has an invalid network address".to_string())?;
        let prefix_len: u8 = prefix_part
            .parse()
            .map_err(|_| "pentest CIDR target has an invalid prefix length".to_string())?;
        if prefix_len > 32 {
            return Err("pentest CIDR prefix length must be between 0 and 32".into());
        }
        if prefix_len < MIN_PENTEST_CIDR_PREFIX_LEN {
            return Err(format!(
                "pentest CIDR range is too broad — minimum accepted prefix is /{MIN_PENTEST_CIDR_PREFIX_LEN}, use several narrower ranges instead"
            ));
        }
        let mask = cidr_mask(prefix_len);
        if network & !mask != 0 {
            return Err(
                "pentest CIDR network address must not set host bits (e.g. 10.0.0.0/24, not 10.0.0.5/24)"
                    .into(),
            );
        }
        return Ok(PentestTargetPattern::Cidr {
            network,
            prefix_len,
        });
    }
    if let Some(base) = raw.strip_prefix("*.") {
        let base = validate_pentest_hostname(base)?;
        if base.split('.').count() < 2 {
            return Err(
                "pentest wildcard target is too broad — base domain needs at least two labels (e.g. *.example.com, not *.com)"
                    .into(),
            );
        }
        return Ok(PentestTargetPattern::Wildcard { base });
    }
    Ok(PentestTargetPattern::ExactHost(normalize_pentest_target(
        raw,
    )?))
}

fn pentest_target_pattern_matches(pattern: &PentestTargetPattern, concrete_target: &str) -> bool {
    match pattern {
        PentestTargetPattern::ExactHost(host) => host == concrete_target,
        PentestTargetPattern::Wildcard { base } => concrete_target
            .strip_suffix(base)
            .and_then(|prefix| prefix.strip_suffix('.'))
            .is_some_and(|prefix| !prefix.is_empty()),
        PentestTargetPattern::Cidr {
            network,
            prefix_len,
        } => parse_canonical_ipv4(concrete_target)
            .is_some_and(|ip| ip & cidr_mask(*prefix_len) == *network),
    }
}

fn cidr_mask(prefix_len: u8) -> u32 {
    if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    }
}

/// Rejects non-canonical decimal octets (leading zeros) alongside the usual range check. This
/// matters for target canonicalization specifically: some parsers read a leading-zero octet
/// (`010`) as octal, so the *same string* can resolve to two different addresses depending on
/// which tool reads it — exactly the kind of ambiguity that could let a probe drift outside the
/// intended target without anyone noticing.
fn parse_canonical_ipv4(candidate: &str) -> Option<u32> {
    let labels: Vec<&str> = candidate.split('.').collect();
    if labels.len() != 4 {
        return None;
    }
    let mut address: u32 = 0;
    for label in labels {
        if label.is_empty() || (label.len() > 1 && label.starts_with('0')) {
            return None;
        }
        let octet: u32 = label.parse().ok()?;
        if octet > 255 {
            return None;
        }
        address = (address << 8) | octet;
    }
    Some(address)
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
        .map(|item| parse_pentest_target_pattern(item))
        .collect::<Result<Vec<_>, _>>()?;
    // Exclusions are checked first and by pattern: a narrow excluded wildcard (e.g.
    // `*.internal.example.com`) must win even against a broader allowed wildcard
    // (`*.example.com`) — excluding something is always safe to honor, allowing something is not.
    if excluded
        .iter()
        .any(|pattern| pentest_target_pattern_matches(pattern, &target))
    {
        return Err("pentest target is explicitly excluded by scope".into());
    }
    let allowed = scope
        .targets
        .iter()
        .map(|item| parse_pentest_target_pattern(item))
        .collect::<Result<Vec<_>, _>>()?;
    if !allowed
        .iter()
        .any(|pattern| pentest_target_pattern_matches(pattern, &target))
    {
        return Err("pentest target is outside the authorization allowlist".into());
    }
    if requested_mode > scope.maximum_mode {
        return Err("requested pentest mode exceeds the authorization scope".into());
    }
    Ok(())
}

/// Validates a single hostname label sequence — shared by the exact-host path and by the base
/// domain of a wildcard pattern, so the two can never quietly diverge on what counts as a valid
/// label.
fn validate_pentest_hostname(host: &str) -> Result<String, String> {
    let host = host.to_ascii_lowercase();
    if host.is_empty()
        || !host.is_ascii()
        || host.contains('*')
        || host.contains('/')
        || host.contains(':')
        || host.split('.').any(|label| {
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
    Ok(host)
}

/// Normalizes the *concrete* target actually under test — never a pattern. A wildcard or CIDR
/// entry describes what may be tested; the thing you actually connect to is always one exact
/// host or address, so this rejects `*` and `/` rather than trying to interpret them.
fn normalize_pentest_target(target: &str) -> Result<String, String> {
    validate_pentest_hostname(target.trim().trim_end_matches('.'))
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
