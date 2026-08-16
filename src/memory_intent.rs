//! Natural-language memory command recognition: lets the user say "hafızana yaz: ...",
//! "belleğini güncelle: ..." or "belleğinden ... sil" directly in a normal chat message, instead
//! of only through the `/remember`/`/forget` slash-command syntax.
//!
//! This is still a 100% explicit-user-command path — never model-initiated. The decision to
//! write/update/delete is made by pattern-matching the *user's own raw input text* against a
//! fixed set of trigger phrases, exactly the same way slash-command parsing already works; the
//! model is never consulted about whether something should be remembered, and ordinary
//! conversation that happens to mention a fact in passing (no trigger phrase) is never
//! intercepted. A recognized trigger with an *unparseable* payload is reported back to the user
//! explicitly — never silently ignored or guessed at, and never silently falls through to normal
//! chat once a trigger phrase is clearly present.
//!
//! Deliberately a fixed phrase list, not general NLU: matches this project's existing style
//! (`workspace.rs`'s exclude-pattern matching, `fts_query`'s tokenizer) of simple, predictable,
//! explainable string checks over a bigger dependency. Extend the trigger/pattern lists here if a
//! common phrasing is missing — that is a small, local change.
//!
//! 16 Ağustos 2026: kullanıcı haklı bir soru sordu — "gerçek bir yapay zeka 'aklında tut' ile
//! 'hafızana yaz'ı ilişkilendiremiyorsa bu ne çeşit bir yapay zeka olur?". Cevap iki parçalı:
//! (1) yaygın eş anlamlılar doğrudan sabit listeye eklendi (yukarıdaki/aşağıdaki listeler); (2)
//! listede olmayan daha nadir ifadeler için model-destekli bir **yedek** yol eklendi
//! (`classify_unrecognized_remember_intent_with_provider`) — ama bu yol, fixed-trigger yolunun
//! aksine **asla doğrudan yazmaz**. Model'in kararı router'da ölçüldüğü gibi bazen yanlış
//! olabilir; bu yüzden sonucu her zaman normal `/remember`-tarzı önizleme/onay adımından geçer,
//! tek adımda otomatik kaydetmez.

use crate::{
    propose_memory, propose_profile_field, turkish_case_fold, DataSensitivity, MemoryProposal,
    ModelProvider, ProfileField,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryIntent {
    /// A fully-built, ready-to-commit proposal. The natural-language sentence itself *is* the
    /// user's one explicit command — there is deliberately no second confirmation step here,
    /// unlike `/remember ... approve`; saying "hafızana yaz: ..." is already unambiguous intent.
    Remember(MemoryProposal),
    /// Kullanıcının "secret'ları doğrudan hafızaya yazmıyoruz, Secret Manager referansı
    /// tutuyoruz" kuralı — ayrı bir tetikleyici kümesiyle tanınır (örn. "hafızana gizli kaydet:
    /// ..."), her zaman açık `anahtar = değer` gerektirir (bir isim gibi doğal cümle kalıbı
    /// yok — kimlik bilgilerinin böyle bir kalıbı olmaz). Bu, `Runtime::remember_secret`'ı
    /// çağırmalı, sıradan `commit_memory_proposal`'ı değil.
    RememberSecret { key: String, value: String },
    /// A known profile field ("adım", "hitabım", "dilim", ...) to delete.
    ForgetProfileField(ProfileField),
    /// A free-form memory key (not a known profile field) to delete, across every namespace.
    ForgetKey(String),
    /// A trigger phrase was recognized but the payload after it could not be understood as
    /// either a known fact pattern or an explicit `anahtar = değer` pair.
    UnparseableRemember,
    /// A secret-remember trigger was recognized but no `anahtar = değer` payload followed it.
    UnparseableRememberSecret,
    /// A forget-trigger was recognized but no target (field name or key) followed it.
    UnparseableForget,
}

const REMEMBER_TRIGGERS: &[&str] = &[
    "hafızana yaz",
    "hafızanı güncelle",
    "belleğine yaz",
    "belleğini güncelle",
    "hatırla ki",
    "hatırla",
    "unutma ki",
    // 16 Ağustos 2026'da kullanıcı "aklında tut" gibi anlamca aynı ama listede olmayan bir
    // kalıp kullandı ve haklı olarak "gerçek bir yapay zeka bunu anlamalı" dedi. Bu ek maddeler
    // gerçekten yaygın eş anlamlıları kapatıyor — hâlâ sabit bir liste (model devreye girmiyor,
    // risk sıfır), yalnız daha geniş. Listede olmayan daha nadir ifadeler için model-destekli
    // yedek yol aşağıda (bkz. `classify_unrecognized_remember_intent_with_provider`).
    "aklında tut ki",
    "aklında tut",
    "aklında olsun ki",
    "aklında olsun",
    "aklında bulunsun",
    "kayıtlara geç",
    "remember that",
    "keep in mind that",
    "keep in mind",
];

/// Ayrı ve öncelikli bir tetikleyici kümesi — kimlik bilgisi gibi bir değeri sıradan
/// `REMEMBER_TRIGGERS`'tan geçirmek, onu yanlışlıkla modele giden sıradan bellek bağlamına
/// (`approved_memory_context`) sokabilirdi. Her zaman `Runtime::remember_secret`'a yönlenir —
/// gerçek değer yalnız ayrı `secrets` tablosuna gider, `memories`'e asla.
const REMEMBER_SECRET_TRIGGERS: &[&str] = &[
    "hafızana gizli kaydet",
    "hafızana gizli olarak kaydet",
    "gizli bilgi kaydet",
    "gizli bilgi olarak kaydet",
    "sırrını sakla",
    "sır olarak kaydet",
];

// Turkish is subject-object-verb: the target sits *between* the prefix and the verb ("hafızandan
// isim bilgimi sil"), not right after a fixed "hafızandan sil" phrase — a single-prefix trigger
// (like `REMEMBER_TRIGGERS` above) cannot express this, so forget needs a prefix+suffix pair.
const FORGET_TRIGGER_PREFIXES: &[&str] = &[
    "hafızandan ",
    "hafızadan ",
    "hafızamdan ",
    "belleğinden ",
    "belleğimden ",
    "aklından ",
];
const FORGET_TRIGGER_SUFFIXES: &[&str] = &[" sil", " çıkar", " unut"];

/// If `input` (Turkish-fold-insensitively) starts with `trigger`, returns the remainder of the
/// *original* string (original casing preserved) with a leading `:`/`,`/whitespace trimmed off.
/// `turkish_case_fold` maps one input character to one output character for ordinary Turkish/
/// Latin chat text, so counting characters in the folded prefix and reading that many characters
/// back out of the original stays aligned.
fn strip_trigger<'a>(input: &'a str, trigger: &str) -> Option<&'a str> {
    let folded = turkish_case_fold(input);
    if !folded.starts_with(trigger) {
        return None;
    }
    let trigger_char_count = trigger.chars().count();
    let byte_offset = input
        .char_indices()
        .nth(trigger_char_count)
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    Some(
        input[byte_offset..]
            .trim_start_matches([':', ',', ' '])
            .trim(),
    )
}

fn find_trigger<'a>(input: &'a str, triggers: &[&str]) -> Option<&'a str> {
    triggers
        .iter()
        .find_map(|trigger| strip_trigger(input, trigger))
}

/// Finds a `(prefix, ... , suffix)` forget pattern and returns the target text strictly between
/// them, original casing preserved. `None` if no prefix/suffix pair matches, or if it matches but
/// leaves no actual target (e.g. "hafızandan sil" with nothing in between — prefix and suffix
/// would overlap or leave an empty gap).
fn extract_forget_target(input: &str) -> Option<String> {
    let folded = turkish_case_fold(input.trim());
    for prefix in FORGET_TRIGGER_PREFIXES {
        let Some(after_prefix) = folded.strip_prefix(prefix) else {
            continue;
        };
        for suffix in FORGET_TRIGGER_SUFFIXES {
            let Some(target_folded) = after_prefix.strip_suffix(suffix) else {
                continue;
            };
            if target_folded.trim().is_empty() {
                continue; // matched trigger shape, but no target between prefix and suffix
            }
            let prefix_char_count = prefix.chars().count();
            let target_char_count = target_folded.chars().count();
            let target: String = input
                .trim()
                .chars()
                .skip(prefix_char_count)
                .take(target_char_count)
                .collect();
            return Some(target.trim().to_string()).filter(|value| !value.is_empty());
        }
    }
    None
}

/// True if `input` (folded) at least *starts* the forget-trigger shape, even if no valid target
/// followed — used only to distinguish "not a forget command at all" (fall through to normal
/// chat) from "a forget command with nothing to forget" (`UnparseableForget`).
fn looks_like_forget_trigger(input: &str) -> bool {
    let folded = turkish_case_fold(input.trim());
    FORGET_TRIGGER_PREFIXES
        .iter()
        .any(|prefix| folded.starts_with(prefix))
        && FORGET_TRIGGER_SUFFIXES
            .iter()
            .any(|suffix| folded.ends_with(suffix))
}

/// A fixed set of common Turkish sentence shapes for the fields `/profile set` already knows —
/// this is what lets "benim adım Ali" resolve directly to `ProfileField::DisplayName`, not just
/// the already-supported `ad = Ali` short form.
fn match_fact_pattern(payload: &str) -> Option<(ProfileField, String)> {
    let folded = turkish_case_fold(payload);
    for prefix in ["benim adım ", "adım "] {
        if folded.starts_with(prefix) {
            return Some((
                ProfileField::DisplayName,
                remainder_preserving_case(payload, prefix),
            ));
        }
    }
    for prefix in ["dilim ", "benim dilim "] {
        if folded.starts_with(prefix) {
            return Some((
                ProfileField::Language,
                remainder_preserving_case(payload, prefix),
            ));
        }
    }
    for prefix in ["rolüm ", "benim rolüm ", "tercihim "] {
        if folded.starts_with(prefix) {
            return Some((
                ProfileField::RolePreference,
                remainder_preserving_case(payload, prefix),
            ));
        }
    }
    for (prefix, suffix) in [("beni ", " diye çağır"), ("bana ", " diye hitap et")] {
        if folded.starts_with(prefix) && folded.ends_with(suffix) {
            let prefix_stripped = remainder_preserving_case(payload, prefix);
            let keep_chars = prefix_stripped.chars().count() - suffix.chars().count();
            let value: String = prefix_stripped.chars().take(keep_chars).collect();
            if !value.trim().is_empty() {
                return Some((ProfileField::PreferredAddress, value.trim().to_string()));
            }
        }
    }
    None
}

/// Resolves a natural-language `(key, value)` pair to a memory proposal the same way regardless
/// of which path found it (fixed trigger's explicit `anahtar = değer` payload, or the
/// model-assisted fallback below): a known profile field alias routes through the profile path,
/// anything else becomes a generic `UserProfile` memory record. One shared place, so the two
/// paths can never quietly diverge on this decision.
fn propose_natural_language_memory(key: &str, value: &str, source: &str) -> Option<MemoryProposal> {
    let proposal = if let Some(field) = ProfileField::from_user_input(key) {
        propose_profile_field(field, value, source, true)
    } else {
        propose_memory(
            crate::MemoryNamespace::UserProfile,
            key,
            value,
            DataSensitivity::Internal,
            source,
            true,
            None,
        )
    };
    proposal.ok()
}

/// Loose, cheap pre-check — no model involved — used only to decide whether it's worth paying
/// for a model call at all. Deliberately broad substrings, not exact triggers: a false positive
/// here only costs one extra (short) model call that resolves to `None`; a false negative just
/// means an unusual phrasing falls through to ordinary conversation, exactly as it always has.
pub fn might_express_an_unrecognized_remember_intent(input: &str) -> bool {
    let folded = turkish_case_fold(input);
    [
        "hafıza",
        "hatırla",
        "aklı",
        "kayıtlara geç",
        "keep in mind",
        "remember",
    ]
    .iter()
    .any(|hint| folded.contains(hint))
}

/// Model-assisted fallback for a remember-style request that no fixed trigger phrase covers.
/// Only ever worth calling after `parse_memory_intent` returned `None` *and*
/// `might_express_an_unrecognized_remember_intent` found a loose hint — ordinary messages never
/// pay for this. Unlike the fixed-trigger path, this never produces a ready-to-commit result: the
/// model's judgment can be wrong (the same class of risk measured and fixed for the capability
/// router, 16 Ağustos 2026), so the caller must always route the `(key, value)` result through
/// `propose_natural_language_memory` and the normal preview/approve step — never commit directly.
pub fn classify_unrecognized_remember_intent_with_provider(
    input: &str,
    provider: &dyn ModelProvider,
) -> Option<(String, String)> {
    let prompt = format!(
        "/no_think Decide whether the user's message explicitly asks you to remember or save a specific piece of information about them for later, in natural language rather than a fixed command phrase. \
A question merely asking whether something is or was remembered (for example \"mı/mi/mu/mü\", \"hatırlar mısın\", \"did you remember\") is not itself a remember request — output NONE for those. \
If the message clearly asks you to save/keep in mind a fact, output exactly one line: REMEMBER <key> = <value> — a short lowercase key (Turkish or English) naming what is being remembered, and the value taken only from what the user actually said, never invented. \
Otherwise (a question, a passing remark, ordinary conversation, or too vague to extract a concrete fact) output exactly: NONE. \
User message: {}",
        input.trim()
    );
    let response = provider.complete(&prompt).ok()?;
    let text = response.text.trim();
    let rest = text.strip_prefix("REMEMBER ")?;
    let (key, value) = rest.split_once('=')?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

/// Combines the two steps callers need for the fallback path: classify with the model, then
/// resolve the result into a ready-for-preview `MemoryProposal` the same way the fixed-trigger
/// path does. Returns `None` if the pre-check hint is absent, the model says `NONE`, or the
/// extracted `(key, value)` fails proposal validation (e.g. an empty/too-long value).
pub fn propose_unrecognized_remember_intent_with_provider(
    input: &str,
    provider: &dyn ModelProvider,
) -> Option<MemoryProposal> {
    if !might_express_an_unrecognized_remember_intent(input) {
        return None;
    }
    let (key, value) = classify_unrecognized_remember_intent_with_provider(input, provider)?;
    propose_natural_language_memory(&key, &value, "chat-natural-language-model-assisted")
}

/// Same character-count-aligned trick as `strip_trigger`, for a fixed lowercase prefix instead
/// of a trigger phrase list.
fn remainder_preserving_case(original: &str, folded_prefix: &str) -> String {
    let prefix_char_count = folded_prefix.chars().count();
    original
        .chars()
        .skip(prefix_char_count)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Splits `payload` on the first `=` or `:` into (key, value), mirroring `/remember anahtar =
/// değer`'s own parsing so natural language and the slash command accept the same explicit form.
fn split_key_value(payload: &str) -> Option<(&str, &str)> {
    payload
        .split_once('=')
        .or_else(|| payload.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
}

/// Parses `input` for a memory-management intent. Returns `None` when no trigger phrase is
/// present at all — the caller must then treat `input` as ordinary conversation, unmodified.
pub fn parse_memory_intent(input: &str) -> Option<MemoryIntent> {
    let trimmed = input.trim();
    if let Some(payload) = find_trigger(trimmed, REMEMBER_SECRET_TRIGGERS) {
        return Some(match split_key_value(payload) {
            Some((key, value)) => MemoryIntent::RememberSecret {
                key: key.to_string(),
                value: value.to_string(),
            },
            None => MemoryIntent::UnparseableRememberSecret,
        });
    }
    if let Some(payload) = find_trigger(trimmed, REMEMBER_TRIGGERS) {
        if payload.is_empty() {
            return Some(MemoryIntent::UnparseableRemember);
        }
        if let Some((field, value)) = match_fact_pattern(payload) {
            return Some(
                propose_profile_field(field, &value, "chat-natural-language", true)
                    .map(MemoryIntent::Remember)
                    .unwrap_or(MemoryIntent::UnparseableRemember),
            );
        }
        if let Some((key, value)) = split_key_value(payload) {
            return Some(
                propose_natural_language_memory(key, value, "chat-natural-language")
                    .map(MemoryIntent::Remember)
                    .unwrap_or(MemoryIntent::UnparseableRemember),
            );
        }
        return Some(MemoryIntent::UnparseableRemember);
    }
    match extract_forget_target(trimmed) {
        Some(target) => {
            // "isim bilgimi sil" / "isim bilgini sil" — strip a trailing "bilgimi"/"bilgisini"/
            // "bilgini"/"bilgisi" filler word before resolving the field/key, so that common
            // phrasing resolves the same as the bare word ("isim sil").
            let folded_target = turkish_case_fold(&target);
            let resolved = ["bilgimi", "bilgisini", "bilgini", "bilgisi"]
                .iter()
                .find_map(|filler| {
                    folded_target
                        .strip_suffix(filler)
                        .map(|_| remainder_dropping_suffix_words(&target, 1))
                })
                .unwrap_or(target);
            if resolved.trim().is_empty() {
                return Some(MemoryIntent::UnparseableForget);
            }
            Some(match ProfileField::from_user_input(resolved.trim()) {
                Some(field) => MemoryIntent::ForgetProfileField(field),
                None => MemoryIntent::ForgetKey(resolved.trim().to_string()),
            })
        }
        None if looks_like_forget_trigger(trimmed) => Some(MemoryIntent::UnparseableForget),
        None => None,
    }
}

/// Drops the last `word_count` whitespace-separated words from `text`.
fn remainder_dropping_suffix_words(text: &str, word_count: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= word_count {
        return String::new();
    }
    words[..words.len() - word_count].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryNamespace, ModelResponse};

    #[derive(Debug)]
    struct FixedRouteReplyProvider(&'static str);

    impl ModelProvider for FixedRouteReplyProvider {
        fn provider_id(&self) -> &str {
            "test"
        }
        fn model_id(&self) -> &str {
            "fixed-reply"
        }
        fn complete(&self, _prompt: &str) -> Result<ModelResponse, String> {
            Ok(ModelResponse {
                provider_id: self.provider_id().into(),
                model_id: self.model_id().into(),
                text: self.0.into(),
                structured_json: None,
                finish_reason: "stop".into(),
            })
        }
    }

    #[test]
    fn no_trigger_phrase_is_ordinary_conversation() {
        assert_eq!(parse_memory_intent("bugün hava nasıl"), None);
        assert_eq!(parse_memory_intent("adım Ali, bu tarif iyi mi?"), None);
    }

    /// Kullanıcının 16 Ağustos 2026'da bulduğu gerçek eksiklik: "aklında tut" gibi anlamca
    /// "hafızana yaz" ile aynı olan yaygın eş anlamlılar listede yoktu. Bu genişletme hâlâ sabit
    /// bir liste (model devreye girmiyor) — yalnız daha kapsamlı.
    #[test]
    fn newly_added_remember_synonyms_are_recognized_the_same_as_the_original_triggers() {
        for phrase in [
            "aklında tut: adım Mehmet",
            "aklında tut ki: adım Mehmet",
            "aklında olsun: adım Mehmet",
            "aklında bulunsun: adım Mehmet",
            "kayıtlara geç: adım Mehmet",
            "remember that: adım Mehmet",
            "keep in mind: adım Mehmet",
            "keep in mind that: adım Mehmet",
        ] {
            let intent = parse_memory_intent(phrase);
            assert!(
                matches!(intent, Some(MemoryIntent::Remember(_))),
                "expected {phrase:?} to resolve to Remember, got {intent:?}"
            );
        }
    }

    /// Kullanıcının "secret'ları doğrudan hafızaya yazmıyoruz, Secret Manager referansı
    /// tutuyoruz" kuralı — ayrı bir tetikleyici kümesi, ayrı bir `MemoryIntent` varyantı.
    #[test]
    fn secret_trigger_resolves_to_remember_secret_with_key_and_value() {
        assert_eq!(
            parse_memory_intent("hafızana gizli kaydet: api_key = sk-abc123"),
            Some(MemoryIntent::RememberSecret {
                key: "api_key".to_string(),
                value: "sk-abc123".to_string(),
            })
        );
        assert_eq!(
            parse_memory_intent("sırrını sakla: db_password = deger"),
            Some(MemoryIntent::RememberSecret {
                key: "db_password".to_string(),
                value: "deger".to_string(),
            })
        );
    }

    #[test]
    fn secret_trigger_with_no_key_value_payload_is_reported_not_silently_dropped() {
        assert_eq!(
            parse_memory_intent("hafızana gizli kaydet: bugün hava güzel"),
            Some(MemoryIntent::UnparseableRememberSecret)
        );
    }

    /// Sıradan "hafızana yaz" tetikleyicisi bir sırrı yakalamamalı — iki tetikleyici kümesi
    /// birbirinden tamamen ayrı, karışmamalı.
    #[test]
    fn ordinary_remember_trigger_never_matches_the_secret_phrasing() {
        assert!(matches!(
            parse_memory_intent("hafızana gizli kaydet: api_key = deger"),
            Some(MemoryIntent::RememberSecret { .. })
        ));
    }

    #[test]
    fn a_known_fact_pattern_resolves_to_the_matching_profile_field() {
        let Some(MemoryIntent::Remember(proposal)) =
            parse_memory_intent("hafızana yaz: benim adım Ali")
        else {
            panic!("expected a ready Remember proposal");
        };
        assert_eq!(proposal.record.namespace, MemoryNamespace::UserProfile);
        assert_eq!(proposal.record.key, "display_name");
        assert_eq!(proposal.record.value, "Ali");
    }

    #[test]
    fn bare_adim_pattern_without_benim_also_resolves() {
        let Some(MemoryIntent::Remember(proposal)) = parse_memory_intent("hatırla ki adım Zeynep")
        else {
            panic!("expected a ready Remember proposal");
        };
        assert_eq!(proposal.record.key, "display_name");
        assert_eq!(proposal.record.value, "Zeynep");
    }

    #[test]
    fn preferred_address_pattern_extracts_the_middle_value() {
        let Some(MemoryIntent::Remember(proposal)) =
            parse_memory_intent("belleğini güncelle: beni Reis diye çağır")
        else {
            panic!("expected a ready Remember proposal");
        };
        assert_eq!(proposal.record.key, "preferred_address");
        assert_eq!(proposal.record.value, "Reis");
    }

    #[test]
    fn explicit_key_value_form_is_accepted_after_a_trigger() {
        let Some(MemoryIntent::Remember(proposal)) =
            parse_memory_intent("hafızana yaz: favori_renk = turkuaz")
        else {
            panic!("expected a ready Remember proposal");
        };
        assert_eq!(proposal.record.key, "favori_renk");
        assert_eq!(proposal.record.value, "turkuaz");
    }

    #[test]
    fn explicit_key_value_form_still_routes_a_known_field_alias_to_the_profile_path() {
        let Some(MemoryIntent::Remember(proposal)) = parse_memory_intent("hafızana yaz: dil = tr")
        else {
            panic!("expected a ready Remember proposal");
        };
        assert_eq!(
            proposal.record.key, "language",
            "the 'dil' alias must resolve to the canonical profile key, not a raw 'dil' key"
        );
    }

    #[test]
    fn a_trigger_with_no_understandable_payload_is_reported_not_silently_dropped() {
        assert_eq!(
            parse_memory_intent("hafızana yaz"),
            Some(MemoryIntent::UnparseableRemember)
        );
        assert_eq!(
            parse_memory_intent("hafızana yaz: bugün hava çok güzel"),
            Some(MemoryIntent::UnparseableRemember)
        );
    }

    #[test]
    fn forget_resolves_a_known_field_name_and_a_free_form_key_differently() {
        assert_eq!(
            parse_memory_intent("hafızandan isim bilgimi sil"),
            Some(MemoryIntent::ForgetProfileField(ProfileField::DisplayName))
        );
        assert_eq!(
            parse_memory_intent("belleğinden favori_renk sil"),
            Some(MemoryIntent::ForgetKey("favori_renk".into()))
        );
    }

    #[test]
    fn forget_with_no_target_is_reported_not_silently_dropped() {
        assert_eq!(
            parse_memory_intent("hafızandan sil"),
            Some(MemoryIntent::UnparseableForget)
        );
    }

    #[test]
    fn trigger_matching_is_case_and_turkish_i_insensitive() {
        let Some(MemoryIntent::Remember(proposal)) =
            parse_memory_intent("HAFIZANA YAZ: benim adım Işık")
        else {
            panic!("expected a ready Remember proposal even with uppercase Turkish 'I'");
        };
        assert_eq!(proposal.record.value, "Işık");
    }

    /// Sıfır maliyet: hint yoksa (hafıza/hatırla/aklı/... geçmeyen sıradan bir cümle) model hiç
    /// çağrılmıyor — sayaçlı bir sağlayıcıyla kanıtlanıyor.
    #[test]
    fn unrecognized_remember_fallback_never_calls_the_model_without_a_loose_hint() {
        #[derive(Debug, Default)]
        struct PanicIfCalledProvider;
        impl ModelProvider for PanicIfCalledProvider {
            fn provider_id(&self) -> &str {
                "test"
            }
            fn model_id(&self) -> &str {
                "panic-if-called"
            }
            fn complete(&self, _prompt: &str) -> Result<ModelResponse, String> {
                panic!("must never be called when no loose hint is present");
            }
        }
        assert_eq!(
            propose_unrecognized_remember_intent_with_provider(
                "bugün hava çok güzeldi",
                &PanicIfCalledProvider,
            ),
            None
        );
    }

    /// Hint varsa ve model gerçek bir kayıt öneriyorsa (`REMEMBER key = value`), bu doğrudan
    /// yazılmıyor — önizlenebilir bir `MemoryProposal`'a çözülüyor (çağıran taraf onu normal
    /// onay akışına sokmalı).
    #[test]
    fn unrecognized_remember_fallback_resolves_a_model_reply_into_a_previewable_proposal() {
        let provider = FixedRouteReplyProvider("REMEMBER favori_renk = mavi");
        let proposal = propose_unrecognized_remember_intent_with_provider(
            "aklında kalsın, favori rengim mavi",
            &provider,
        )
        .expect("a hinted message with a REMEMBER reply must resolve to a proposal");
        assert_eq!(proposal.record.key, "favori_renk");
        assert_eq!(proposal.record.value, "mavi");
        assert_eq!(
            proposal.record.source,
            "chat-natural-language-model-assisted"
        );
    }

    /// Model "NONE" derse (soru/sıradan sohbet), hiçbir öneri üretilmiyor.
    #[test]
    fn unrecognized_remember_fallback_produces_nothing_when_the_model_says_none() {
        let provider = FixedRouteReplyProvider("NONE");
        assert_eq!(
            propose_unrecognized_remember_intent_with_provider(
                "bunu hatırlar mısın acaba",
                &provider,
            ),
            None
        );
    }

    /// Bilinen bir profil alanı adı (`isim`/`ad` vb.) modelin döndürdüğü anahtarla eşleşirse,
    /// aynı fixed-trigger yolunun kullandığı profil yoluna gitmeli — iki path'in davranışı
    /// çelişmemeli.
    #[test]
    fn unrecognized_remember_fallback_routes_a_known_field_alias_to_the_profile_path() {
        let provider = FixedRouteReplyProvider("REMEMBER isim = Ayşe");
        // Deliberately doesn't start with any exact `REMEMBER_TRIGGERS` phrase (so
        // `parse_memory_intent` alone would return `None`) but still contains the loose "hafıza"
        // hint — this exercises the fallback path specifically, not the fixed-trigger one.
        let proposal = propose_unrecognized_remember_intent_with_provider(
            "bu bilgiyi hafızanda tutmanı istiyorum, ismim Ayşe",
            &provider,
        )
        .expect("a known field alias must still resolve");
        assert_eq!(proposal.record.namespace, MemoryNamespace::UserProfile);
        assert_eq!(proposal.record.key, "display_name");
        assert_eq!(proposal.record.value, "Ayşe");
    }
}
