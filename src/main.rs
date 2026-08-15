use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use jarvis_core::{
    attachment_receipt_manifest, inspect_local_attachment, memory_export, memory_import,
    parse_data_sensitivity, parse_memory_intent, parse_memory_namespace, preview_workspace_index,
    profile_manifest, propose_memory, propose_memory_with_trust_and_scope, propose_profile_field,
    AttachmentReceipt, AttachmentRef, DataSensitivity, InputType, LlamaEmbeddingProvider,
    LlamaServerProvider, LlamaVisionServerProvider, MemoryIntent, MemoryNamespace, MemoryProposal,
    ProfileField, Request, Runtime, SqliteStore, TaskState, TrustLevel, VisionProvider,
    WorkspaceCitation,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Terminal,
};

#[derive(Debug, Clone, Copy)]
enum MessageRole {
    User,
    Jarvis,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiExitAction {
    KeepModelInRam,
    StopModelAndExit,
}

#[derive(Debug, Clone)]
struct Message {
    role: MessageRole,
    content: String,
}

struct WorkerReply {
    message_index: usize,
    content: String,
    status: String,
    task_id: String,
    approval_pending: bool,
    notification: Option<TuiNotification>,
    sources: Vec<String>,
    /// F3 "Citation UX: ... kaynağı aç davranışı". Full citations behind this reply (not just
    /// the display strings in `sources`) so `/source <n>` can show the complete chunk text.
    citations: Vec<WorkspaceCitation>,
    attachment_receipts: Vec<AttachmentReceipt>,
}

struct TuiNotification {
    title: &'static str,
    content: String,
}

struct App {
    messages: Vec<Message>,
    input: String,
    status: String,
    model_state: String,
    last_model_check: Instant,
    scroll: u16,
    pending: bool,
    running: bool,
    attachments: Vec<AttachmentRef>,
    sent_attachment_receipts: Vec<AttachmentReceipt>,
    pending_memory: Option<MemoryProposal>,
    /// F3 "Citation UX: ... kaynağı aç davranışı". Citations behind the most recent JARVIS
    /// reply; `/source <n>` opens one by its 1-based position here. Cleared whenever a reply
    /// used none, mirroring `Runtime::last_workspace_citations` — never stale across turns.
    last_citations: Vec<WorkspaceCitation>,
}

impl App {
    fn new(model_state: &str) -> Self {
        let intro = if model_state == "ready" {
            "JARVIS hazır. Local CPU model server RAM'de. Mesajını yazıp Enter'a bas."
        } else {
            "JARVIS hazırlanıyor. Local model RAM'e yüklenirken mesajını yazabilirsin."
        };
        Self {
            messages: vec![Message {
                role: MessageRole::System,
                content: intro.into(),
            }],
            input: String::new(),
            status: "Hazır • Ctrl+C çıkış • /help kısayollar".into(),
            model_state: model_state.into(),
            last_model_check: Instant::now(),
            scroll: 0,
            pending: false,
            running: true,
            attachments: vec![],
            sent_attachment_receipts: vec![],
            pending_memory: None,
            last_citations: vec![],
        }
    }

    fn push_system(&mut self, content: impl Into<String>) {
        self.messages.push(Message {
            role: MessageRole::System,
            content: content.into(),
        });
    }

    fn record_attachment_receipts(&mut self, receipts: Vec<AttachmentReceipt>) {
        const MAX_SESSION_ATTACHMENT_RECEIPTS: usize = 50;
        self.sent_attachment_receipts.extend(receipts);
        let excess = self
            .sent_attachment_receipts
            .len()
            .saturating_sub(MAX_SESSION_ATTACHMENT_RECEIPTS);
        if excess > 0 {
            self.sent_attachment_receipts.drain(..excess);
        }
    }
}

fn model_label(model_state: &str) -> &'static str {
    if model_state == "ready" {
        "hazır"
    } else {
        "başlatılıyor"
    }
}

fn refresh_model_state(app: &mut App, provider: &LlamaServerProvider) {
    if app.last_model_check.elapsed() < Duration::from_secs(1) {
        return;
    }
    app.last_model_check = Instant::now();
    let current = provider.runtime_state().as_str().to_owned();
    if current == app.model_state {
        return;
    }
    app.model_state = current;
    if app.model_state == "ready" {
        app.push_system("Local model hazır; CPU/RAM üzerinde çalışıyor, VRAM kullanılmıyor.");
        app.status = "Model RAM'de hazır • CPU-only • VRAM: 0".into();
    } else {
        app.push_system("Local model sunucusuna şu an ulaşılamıyor.");
        app.status = "Model sunucusu erişilemiyor; gönderilmemiş mesajın korunur.".into();
        notify_desktop("JARVIS model hatası", &app.status);
    }
}

fn main() -> io::Result<()> {
    if cli_requests_desktop() {
        return run_native_desktop_client();
    }
    let project_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let store = SqliteStore::open(
        project_root
            .join("jarvis.db")
            .to_str()
            .expect("JARVIS database path must be UTF-8"),
    )
    .expect("JARVIS SQLite store açılamadı");
    let mut runtime = Runtime::with_store(store);
    attach_embedding_provider_if_reachable(&mut runtime);
    let runtime = Arc::new(Mutex::new(runtime));
    let provider = LlamaServerProvider::local_default();
    let vision = LlamaVisionServerProvider::local_default();
    let startup_note = ensure_local_model_server(&provider);
    run_tui(runtime, provider, vision, startup_note)
}

fn cli_requests_desktop() -> bool {
    std::env::args_os()
        .skip(1)
        .any(|argument| argument == "--desktop")
}

fn native_desktop_binary_path(current_executable: &Path) -> PathBuf {
    current_executable.with_file_name("jarvis-desktop")
}

fn run_native_desktop_client() -> io::Result<()> {
    let current_executable = std::env::current_exe()?;
    let desktop_executable = native_desktop_binary_path(&current_executable);
    if !desktop_executable.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "native JARVIS istemcisi bulunamadı: {}. `cargo build --release --offline` çalıştır.",
                desktop_executable.display()
            ),
        ));
    }
    let status = Command::new(&desktop_executable).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "native JARVIS istemcisi başarısız kapandı: {status}"
        )))
    }
}

fn ensure_local_model_server(provider: &LlamaServerProvider) -> String {
    if provider.runtime_state().as_str() == "ready" {
        return "Model RAM'de hazır • CPU-only • VRAM: 0".into();
    }
    match Command::new("systemctl")
        .args(["--user", "start", "jarvis-llama.service"])
        .status()
    {
        Ok(status) if status.success() => {
            "Model sunucusu başlatılıyor; ilk açılışta birkaç saniye sürebilir.".into()
        }
        Ok(status) => format!("Model sunucusu başlatılamadı (systemctl: {status})."),
        Err(error) => format!("Model sunucusu başlatılamadı: {error}"),
    }
}

/// Starts the separate CPU-only image server on demand. It stays loopback-only and is never
/// needed for a text-only turn. A caller that receives `Err` still routes through Runtime so
/// the user gets the standard path-safe vision failure instead of a fabricated answer.
/// Best-effort: attaches the local embedding adapter for hybrid RAG retrieval only if it is
/// already reachable. Unlike the text/vision services, this is never started on demand — hybrid
/// retrieval is an enhancement on top of already-working FTS, not something worth spending RAM
/// on for a session that never uses it. If unreachable, `Runtime` simply stays FTS-only, exactly
/// as it always has.
fn attach_embedding_provider_if_reachable(runtime: &mut Runtime) {
    let provider = LlamaEmbeddingProvider::local_default();
    if provider.is_reachable() {
        runtime.set_embedding_provider(Some(Box::new(provider)));
    }
}

fn ensure_local_vision_server(provider: &LlamaVisionServerProvider) -> Result<(), String> {
    let mut health_provider = provider.clone();
    health_provider.timeout_seconds = 1;
    if health_provider.runtime_state() == jarvis_core::ModelRuntimeState::Ready {
        return Ok(());
    }
    let status = Command::new("systemctl")
        .args(["--user", "start", "jarvis-vision.service"])
        .status()
        .map_err(|error| format!("vision service başlatılamadı: {error}"))?;
    if !status.success() {
        return Err(format!(
            "vision service başlatılamadı (systemctl: {status})"
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if health_provider.runtime_state() == jarvis_core::ModelRuntimeState::Ready {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("vision service zamanında hazır olmadı".into())
}

fn stop_local_model_server() -> Result<(), String> {
    let mut failures = Vec::new();
    for service in ["jarvis-llama.service", "jarvis-vision.service"] {
        match Command::new("systemctl")
            .args(["--user", "stop", service])
            .output()
        {
            Ok(output) if output.status.success() => {}
            // F2's optional vision unit may not yet be linked on a text-only installation.
            // Stopping `exit` must still release the normal text model without reporting a false
            // failure in that supported configuration.
            Ok(output)
                if service == "jarvis-vision.service"
                    && String::from_utf8_lossy(&output.stderr).contains("not loaded") => {}
            Ok(output) => failures.push(format!("{service} (systemctl: {})", output.status)),
            Err(error) => failures.push(format!("{service}: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "model sunucuları durdurulamadı: {}",
            failures.join(", ")
        ))
    }
}

fn run_tui(
    runtime: Arc<Mutex<Runtime>>,
    provider: LlamaServerProvider,
    vision: LlamaVisionServerProvider,
    startup_note: String,
) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let model_state = provider.runtime_state().as_str();
    let mut app = App::new(model_state);
    app.status = startup_note;
    let result = event_loop(&mut terminal, runtime, provider, vision, app);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: Arc<Mutex<Runtime>>,
    provider: LlamaServerProvider,
    vision: LlamaVisionServerProvider,
    mut app: App,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::channel::<WorkerReply>();
    while app.running {
        refresh_model_state(&mut app, &provider);
        while let Ok(reply) = receiver.try_recv() {
            if let Some(message) = app.messages.get_mut(reply.message_index) {
                message.content = reply.content;
                if !reply.sources.is_empty() {
                    message.content.push_str("\n\nKaynaklar:\n");
                    message.content.push_str(&reply.sources.join("\n"));
                }
            }
            app.last_citations = reply.citations;
            app.status = reply.status;
            app.pending = false;
            app.record_attachment_receipts(reply.attachment_receipts);
            if let Some(notification) = reply.notification {
                notify_desktop(notification.title, &notification.content);
            }
            if reply.approval_pending {
                app.push_system(format!(
                    "Bu istek onay bekliyor ({}) . Tek bekleyen işlem varsa /approve yaz; vazgeçmek için /cancel. Tüm bekleyenleri görmek için /approvals.",
                    reply.task_id
                ));
            }
        }
        terminal.draw(|frame| draw(frame.area(), frame, &app))?;
        if !event::poll(Duration::from_millis(80))? {
            continue;
        }
        let key = match event::read()? {
            Event::Paste(pasted) if !app.pending => {
                append_pasted_text(&mut app.input, &pasted);
                app.status =
                    "Panodaki metin taslağa eklendi • Enter gönder • Ctrl+Backspace kelime sil"
                        .into();
                continue;
            }
            Event::Paste(_) => continue,
            Event::Mouse(mouse) => {
                if is_primary_selection_paste(mouse.kind) && !app.pending {
                    match primary_selection_text() {
                        Ok(pasted) if !pasted.is_empty() => {
                            append_pasted_text(&mut app.input, &pasted);
                            app.status = "Birincil seçim taslağa eklendi • Enter gönder".into();
                        }
                        Ok(_) => app.status = "Birincil seçimde metin yok.".into(),
                        Err(error) => {
                            app.status = format!("Birincil seçim yapıştırılamadı: {error}")
                        }
                    }
                    continue;
                }
                apply_history_mouse_scroll(&mut app.scroll, mouse.kind);
                continue;
            }
            Event::Key(key) => key,
            _ => continue,
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if should_close_tui_for_key(key) {
            app.running = false;
            continue;
        }
        if apply_history_key_scroll(&mut app.scroll, key.code) {
            continue;
        }
        if app.pending {
            if key.code == KeyCode::Esc {
                app.status = "JARVIS yanıt üretirken girdi kilitli. Yanıt tamamlanınca yeni mesaj gönderebilirsin.".into();
            }
            continue;
        }
        if is_clipboard_paste_shortcut(key) {
            match clipboard_text() {
                Ok(pasted) if !pasted.is_empty() => {
                    append_pasted_text(&mut app.input, &pasted);
                    app.status = "Panodaki metin taslağa eklendi • Enter gönder".into();
                }
                Ok(_) => app.status = "Panoda metin yok.".into(),
                Err(error) => app.status = format!("Panodan yapıştırılamadı: {error}"),
            }
            continue;
        }
        if is_delete_previous_word_shortcut(key) {
            delete_previous_word(&mut app.input);
            continue;
        }
        if should_clear_draft(key) {
            app.input.clear();
            if is_clear_draft_shortcut(key) {
                app.status = "Taslak temizlendi.".into();
            }
            continue;
        }
        match key.code {
            KeyCode::Enter => submit(&mut app, &runtime, &provider, &vision, &sender),
            KeyCode::Backspace => {
                app.input.pop();
            }
            KeyCode::Char(character) => app.input.push(character),
            _ => {}
        }
    }
    Ok(())
}

fn is_clipboard_paste_shortcut(key: KeyEvent) -> bool {
    (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('v' | 'V')))
        || matches!(key.code, KeyCode::Char('\u{16}'))
}

fn is_delete_previous_word_shortcut(key: KeyEvent) -> bool {
    (key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Backspace | KeyCode::Char('w' | 'W')))
        || matches!(key.code, KeyCode::Char('\u{17}'))
}

fn is_clear_draft_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('u' | 'U'))
}

fn should_clear_draft(key: KeyEvent) -> bool {
    is_clear_draft_shortcut(key) || key.code == KeyCode::Esc
}

fn should_close_tui_for_key(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'C'))
}

fn tui_exit_action(input: &str) -> Option<TuiExitAction> {
    match input {
        "/quit" => Some(TuiExitAction::KeepModelInRam),
        "exit" | "/exit" => Some(TuiExitAction::StopModelAndExit),
        _ => None,
    }
}

/// `/index <path> [sensitivity]`: if the last whitespace-separated word of `rest` parses as a
/// known sensitivity word (English or Turkish, `parse_data_sensitivity`), it is split off and
/// the rest is the path; otherwise the whole input is the path and sensitivity defaults to
/// `Internal` — a path that legitimately ends in a word like "public" (unlikely, but possible)
/// only breaks this if it is *also* nothing else on the line, the same ambiguity every trailing-
/// flag command design accepts.
fn split_trailing_sensitivity(rest: &str) -> (&str, DataSensitivity) {
    if let Some((path, last_word)) = rest.trim_end().rsplit_once(char::is_whitespace) {
        if let Some(sensitivity) = parse_data_sensitivity(last_word) {
            return (path.trim_end(), sensitivity);
        }
    }
    (rest, DataSensitivity::Internal)
}

/// `/remember [namespace-word] [görev <task-id>] anahtar = değer`: an optional leading namespace
/// word (`parse_memory_namespace` — profil/proje/görev/oturum/geçici, English too) selects the
/// namespace; `Task` additionally consumes the *next* word as its `scope_id`. Falls back to
/// `(UserProfile, None, rest as-is)` whenever there is no leading namespace word, OR when
/// stripping one would leave no real "anahtar = değer" behind (`looks_like_key_value` check) —
/// this is what keeps `/remember proje = X` behaving exactly as it always did for a user whose
/// literal key happens to be "proje", rather than misreading it as an (invalid, empty-key)
/// namespace selection.
fn parse_remember_namespace_prefix(rest: &str) -> (MemoryNamespace, Option<String>, String) {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let Some(&first_word) = words.first() else {
        return (MemoryNamespace::UserProfile, None, String::new());
    };
    let Some(namespace) = parse_memory_namespace(first_word) else {
        return (MemoryNamespace::UserProfile, None, rest.to_string());
    };
    let (scope_id, remainder_words) = if namespace == MemoryNamespace::Task {
        match words.get(1) {
            Some(&task_id) => (Some(task_id.to_string()), &words[2..]),
            None => (None, &words[1..]),
        }
    } else {
        (None, &words[1..])
    };
    let remainder = remainder_words.join(" ");
    let looks_like_key_value = remainder
        .split_once('=')
        .is_some_and(|(key, _)| !key.trim().is_empty());
    if looks_like_key_value {
        (namespace, scope_id, remainder)
    } else {
        (MemoryNamespace::UserProfile, None, rest.to_string())
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn apply_history_mouse_scroll(scroll: &mut u16, kind: MouseEventKind) -> bool {
    match kind {
        MouseEventKind::ScrollUp => *scroll = scroll.saturating_add(3),
        MouseEventKind::ScrollDown => *scroll = scroll.saturating_sub(3),
        _ => return false,
    }
    true
}

fn is_primary_selection_paste(kind: MouseEventKind) -> bool {
    matches!(kind, MouseEventKind::Down(MouseButton::Middle))
}

fn apply_history_key_scroll(scroll: &mut u16, key: KeyCode) -> bool {
    match key {
        KeyCode::Up => *scroll = scroll.saturating_add(3),
        KeyCode::PageUp => *scroll = scroll.saturating_add(8),
        KeyCode::Down => *scroll = scroll.saturating_sub(3),
        KeyCode::PageDown | KeyCode::End => *scroll = 0,
        KeyCode::Home => *scroll = u16::MAX,
        _ => return false,
    }
    true
}

fn append_pasted_text(input: &mut String, pasted: &str) {
    let mut previous_was_line_break = false;
    for character in pasted.chars() {
        if matches!(character, '\n' | '\r') {
            if !input.chars().last().is_some_and(char::is_whitespace) {
                input.push(' ');
            }
            previous_was_line_break = true;
        } else {
            if previous_was_line_break && character.is_whitespace() {
                continue;
            }
            input.push(character);
            previous_was_line_break = false;
        }
    }
}

fn clipboard_text() -> Result<String, String> {
    clipboard_text_from(&["--no-newline"])
}

fn primary_selection_text() -> Result<String, String> {
    clipboard_text_from(&["--primary", "--no-newline"])
}

fn clipboard_text_from(arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("wl-paste")
        .args(arguments)
        .output()
        .map_err(|error| format!("wl-paste çalıştırılamadı: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    String::from_utf8(output.stdout).map_err(|_| "panodaki veri UTF-8 metin değil".into())
}

fn delete_previous_word(input: &mut String) {
    let without_trailing_whitespace = input.trim_end_matches(char::is_whitespace).len();
    input.truncate(without_trailing_whitespace);
    let start = input
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(index, _)| index);
    input.truncate(start);
    input.truncate(input.trim_end_matches(char::is_whitespace).len());
}

/// Preview shown after `/remember key = value` or any `/remember sensitivity|ttl ...`
/// adjustment. Nothing is persisted until `/remember approve`; this lets the user see and change
/// the exact sensitivity/TTL a record will be saved with, rather than every record silently
/// getting one fixed default.
fn pending_memory_preview(proposal: &MemoryProposal) -> String {
    let ttl = match proposal.record.expires_at {
        Some(expires_at) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            format!(
                "{} saat sonra silinir",
                expires_at.saturating_sub(now) / 3600
            )
        }
        None => "kalıcı".into(),
    };
    format!(
        "Bellek teklifi (henüz kaydedilmedi): {} = {}\nNamespace: {} • sensitivity: {} • süre: {} • model context: {}\nDeğiştir: /remember sensitivity <public|internal|sensitive> • /remember ttl <saat|none> • /remember model-context <evet|hayır>\nOnay: /remember approve • Vazgeç: /remember reject",
        proposal.record.key,
        proposal.record.value,
        proposal.record.namespace.as_str(),
        proposal.record.sensitivity.as_str(),
        ttl,
        if proposal.record.include_in_model_context {
            "evet"
        } else {
            "hayır"
        },
    )
}

/// Re-proposes the pending memory record with a different sensitivity, keeping everything else
/// (key/value/namespace/TTL) the same. Still just a proposal — `/remember approve` is required.
fn adjust_pending_memory_sensitivity(app: &mut App, word: &str) {
    let Some(proposal) = app.pending_memory.as_ref() else {
        app.push_system("Önce /remember anahtar = değer ile bir teklif oluştur.");
        return;
    };
    let Some(sensitivity) = parse_data_sensitivity(word) else {
        app.push_system("Geçersiz sensitivity. Kullan: public, internal veya sensitive.");
        return;
    };
    let record = proposal.record.clone();
    match propose_memory(
        record.namespace,
        record.key,
        record.value,
        sensitivity,
        record.source,
        record.include_in_model_context,
        record.expires_at,
    ) {
        Ok(updated) => {
            app.push_system(pending_memory_preview(&updated));
            app.pending_memory = Some(updated);
        }
        Err(error) => app.push_system(format!("Güncellenemedi: {error}")),
    }
}

/// Re-proposes the pending memory record with a different TTL (`ttl <saat>` or `ttl none` for
/// permanent), keeping key/value/namespace/sensitivity the same.
fn adjust_pending_memory_ttl(app: &mut App, word: &str) {
    let Some(proposal) = app.pending_memory.as_ref() else {
        app.push_system("Önce /remember anahtar = değer ile bir teklif oluştur.");
        return;
    };
    let expires_at = if word.eq_ignore_ascii_case("none") || word == "kalıcı" {
        None
    } else {
        match word.parse::<u64>() {
            Ok(hours) if hours > 0 => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0);
                Some(now + hours * 3600)
            }
            _ => {
                app.push_system("Kullanım: /remember ttl <saat sayısı> veya /remember ttl none");
                return;
            }
        }
    };
    let record = proposal.record.clone();
    match propose_memory(
        record.namespace,
        record.key,
        record.value,
        record.sensitivity,
        record.source,
        record.include_in_model_context,
        expires_at,
    ) {
        Ok(updated) => {
            app.push_system(pending_memory_preview(&updated));
            app.pending_memory = Some(updated);
        }
        Err(error) => app.push_system(format!("Güncellenemedi: {error}")),
    }
}

/// Re-proposes the pending memory record with `include_in_model_context` toggled explicitly by
/// the user, instead of every record silently always being included. `retrieve_memory` already
/// excludes `include_in_model_context=0` rows at read time; this is what actually lets the user
/// exercise that control, which previously had no command attached to it.
fn adjust_pending_memory_model_context(app: &mut App, word: &str) {
    let Some(proposal) = app.pending_memory.as_ref() else {
        app.push_system("Önce /remember anahtar = değer ile bir teklif oluştur.");
        return;
    };
    let include_in_model_context = match word.to_ascii_lowercase().as_str() {
        "evet" | "yes" | "true" => true,
        "hayır" | "hayir" | "no" | "false" => false,
        _ => {
            app.push_system("Kullanım: /remember model-context <evet|hayır>");
            return;
        }
    };
    let record = proposal.record.clone();
    match propose_memory(
        record.namespace,
        record.key,
        record.value,
        record.sensitivity,
        record.source,
        include_in_model_context,
        record.expires_at,
    ) {
        Ok(updated) => {
            app.push_system(pending_memory_preview(&updated));
            app.pending_memory = Some(updated);
        }
        Err(error) => app.push_system(format!("Güncellenemedi: {error}")),
    }
}

fn submit(
    app: &mut App,
    runtime: &Arc<Mutex<Runtime>>,
    provider: &LlamaServerProvider,
    vision: &LlamaVisionServerProvider,
    sender: &mpsc::Sender<WorkerReply>,
) {
    let input = app.input.trim().to_owned();
    app.input.clear();
    if input.is_empty() {
        return;
    }
    // Natural-language memory commands ("hafızana yaz: ...", "belleğinden ... sil") are
    // recognized here, before any slash-command parsing — a plain sentence, not a "/"-prefixed
    // command, but still a single, unambiguous, explicit user instruction (`memory_intent.rs`).
    // No slash command starts with these trigger phrases, so this can never shadow an existing
    // command; anything without a recognized trigger phrase falls straight through untouched.
    match parse_memory_intent(&input) {
        Some(MemoryIntent::Remember(proposal)) => {
            let key = proposal.record.key.clone();
            let value = proposal.record.value.clone();
            let namespace = proposal.record.namespace.as_str();
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .commit_memory_proposal(&proposal, true)
            {
                Ok(_) => app.push_system(format!(
                    "Not aldım: {key} = {value} • namespace={namespace}"
                )),
                Err(error) => app.push_system(format!("Kaydedemedim: {error}")),
            }
            return;
        }
        Some(MemoryIntent::ForgetProfileField(field)) => {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .delete_profile_field(field)
            {
                Ok(true) => {
                    app.push_system(format!("{} bilgisini bellekten sildim.", field.label()))
                }
                Ok(false) => app.push_system(format!("{} zaten kayıtlı değildi.", field.label())),
                Err(error) => app.push_system(format!("Silemedim: {error}")),
            }
            return;
        }
        Some(MemoryIntent::ForgetKey(key)) => {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .delete_memory_by_key(&key)
            {
                Ok(0) => {
                    app.push_system(format!("'{key}' anahtarıyla kayıtlı bir bellek bulamadım."))
                }
                Ok(count) => app.push_system(format!("'{key}' ile eşleşen {count} kayıt silindi.")),
                Err(error) => app.push_system(format!("Silemedim: {error}")),
            }
            return;
        }
        Some(MemoryIntent::RememberSecret { key, value }) => {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .remember_secret(&key, &value)
            {
                Ok(()) => app.push_system(format!(
                    "Sırrı kaydettim: {key} • gerçek değer yalnız Secret Manager'da, sıradan belleğe/modele hiç gitmiyor. Görüntülemek için: /secret show {key}"
                )),
                Err(error) => app.push_system(format!("Kaydedemedim: {error}")),
            }
            return;
        }
        Some(MemoryIntent::UnparseableRemember) => {
            app.push_system(
                "Ne kaydetmemi istediğini anlayamadım. Örnek: 'hafızana yaz: adım Ali' ya da 'hafızana yaz: anahtar = değer'.",
            );
            return;
        }
        Some(MemoryIntent::UnparseableRememberSecret) => {
            app.push_system(
                "Neyi gizli kaydetmemi istediğini anlayamadım. Örnek: 'hafızana gizli kaydet: api_key = değer'.",
            );
            return;
        }
        Some(MemoryIntent::UnparseableForget) => {
            app.push_system(
                "Neyi silmemi istediğini anlayamadım. Örnek: 'hafızandan isim bilgimi sil'.",
            );
            return;
        }
        None => {} // trigger phrase yok — normal sohbet, aşağıda değişmeden devam eder
    }
    if let Some(task_id) = input
        .strip_prefix("/approve ")
        .or_else(|| input.strip_prefix("approve "))
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
    {
        approve_task(app, runtime, task_id);
        return;
    }
    if let Some(task_id) = input
        .strip_prefix("/cancel ")
        .or_else(|| input.strip_prefix("cancel "))
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
    {
        cancel_task(app, runtime, task_id);
        return;
    }
    if let Some(path) = input.strip_prefix("/attach ").map(str::trim) {
        match inspect_local_attachment(path) {
            Ok(attachment) => {
                if app
                    .attachments
                    .iter()
                    .any(|queued| queued.sha256 == attachment.sha256)
                {
                    app.push_system("Bu ek zaten ek kuyruğunda.");
                } else {
                    let details = if attachment.kind.is_image() {
                        format!("{}×{}", attachment.width, attachment.height)
                    } else {
                        format!("{} KiB", attachment.byte_size.div_ceil(1024))
                    };
                    let availability = if attachment.kind.is_image() {
                        "Görsel gönderildiğinde yalnız ayrı local vision sunucusuna gider; normal text modeline piksel veya yerel yol verilmez. Vision hazır değilse JARVIS güvenli hata verir ve görseli gördüğünü iddia etmez."
                    } else {
                        "Belge içeriği bu ek kuyruğundan modele veya araca verilmez; yalnız metadata güvenle taşınır. İndeksleme için ayrı, açık onaylı RAG akışı gerekir."
                    };
                    app.push_system(format!(
                        "Ek kuyruğa alındı: {} • {} • {} • SHA-256:{}…\n{}",
                        attachment.original_name,
                        attachment.mime_type(),
                        details,
                        &attachment.sha256[..12],
                        availability,
                    ));
                    app.attachments.push(attachment);
                }
            }
            Err(error) => app.push_system(format!("Ek alınamadı: {error}")),
        }
        return;
    }
    if input == "/attachment-history clear" {
        let count = app.sent_attachment_receipts.len();
        app.sent_attachment_receipts.clear();
        app.push_system(format!(
            "{count} oturum ek makbuzu temizlendi; hiçbir orijinal dosya silinmedi."
        ));
        return;
    }
    if let Some(attachment_id) = input
        .strip_prefix("/attachment-history remove ")
        .map(str::trim)
        .filter(|attachment_id| !attachment_id.is_empty())
    {
        if let Some(index) = app
            .sent_attachment_receipts
            .iter()
            .position(|receipt| receipt.attachment_id == attachment_id)
        {
            app.sent_attachment_receipts.remove(index);
            app.push_system("Ek makbuzu kaldırıldı; hiçbir orijinal dosya silinmedi.");
        } else {
            app.push_system("Bu ID ile oturum ek makbuzu yok.");
        }
        return;
    }
    if let Some(path) = input
        .strip_prefix("/attachment-export ")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        match attachment_receipt_manifest(&app.sent_attachment_receipts).and_then(|manifest| {
            std::fs::write(path, manifest)
                .map_err(|error| format!("attachment receipt manifest write failed: {error}"))
        }) {
            Ok(()) => app.push_system(
                "Ek metadata makbuzları dışa aktarıldı; yerel yol, dosya içeriği, prompt veya model yanıtı içermez.",
            ),
            Err(error) => app.push_system(format!("Ek makbuzları dışa aktarılamadı: {error}")),
        }
        return;
    }
    if input == "/attachment-history" {
        if app.sent_attachment_receipts.is_empty() {
            app.push_system("Oturum ek makbuzu yok.");
        } else {
            let receipts = app
                .sent_attachment_receipts
                .iter()
                .map(|receipt| {
                    format!(
                        "{}\n  Kaldır: /attachment-history remove {}",
                        receipt.display_summary(),
                        receipt.attachment_id
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            app.push_system(format!(
                "Bu oturumdaki ek metadata makbuzları (yerel yol/byte/prompt yok):\n{receipts}\nTümü: /attachment-history clear • JSON dışa aktar: /attachment-export <dosya-yolu>"
            ));
        }
        return;
    }
    if let Some(specification) = input.strip_prefix("/remember ").map(str::trim) {
        match specification {
            "approve" => {
                let Some(proposal) = app.pending_memory.take() else {
                    app.push_system("Onaylanacak bellek teklifi yok.");
                    return;
                };
                match runtime
                    .lock()
                    .expect("JARVIS runtime lock poisoned")
                    .commit_memory_proposal(&proposal, true)
                {
                    Ok(record) => app.push_system(format!(
                        "Bellek kaydedildi: {} = {} • namespace={} • modele dahil={}",
                        record.key,
                        record.value,
                        record.namespace.as_str(),
                        record.include_in_model_context
                    )),
                    Err(error) => app.push_system(format!("Bellek kaydedilemedi: {error}")),
                }
                return;
            }
            "reject" => {
                if app.pending_memory.take().is_some() {
                    app.push_system("Bellek teklifi reddedildi; hiçbir veri kaydedilmedi.");
                } else {
                    app.push_system("Reddedilecek bellek teklifi yok.");
                }
                return;
            }
            _ => {}
        }
        if let Some(word) = specification.strip_prefix("sensitivity ").map(str::trim) {
            adjust_pending_memory_sensitivity(app, word);
            return;
        }
        if let Some(word) = specification.strip_prefix("ttl ").map(str::trim) {
            adjust_pending_memory_ttl(app, word);
            return;
        }
        if let Some(word) = specification.strip_prefix("model-context ").map(str::trim) {
            adjust_pending_memory_model_context(app, word);
            return;
        }
        // F3 sonrası: Session/Task/Project namespace'lerine gerçek bir yazma yolu — önceden
        // yalnız UserProfile yazılabiliyordu, diğer dördü şema olarak vardı ama hiçbir üretim
        // yolu onlara yazmıyordu. `/remember proje anahtar = değer`,
        // `/remember görev <task-id> anahtar = değer`, `/remember oturum anahtar = değer`.
        // Namespace kelimesi yoksa (veya sonrası gerçek bir "anahtar = değer" gibi görünmüyorsa —
        // örn. kullanıcının anahtarı gerçekten "proje" ise) eskisi gibi UserProfile'a düşer, kod
        // değişmeden önceki davranış aynen korunur.
        let (namespace, scope_id, remainder) = parse_remember_namespace_prefix(specification);
        let Some((key, value)) = remainder
            .split_once('=')
            .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        else {
            app.push_system("Kullanım: /remember [profil|proje|görev <task-id>|oturum|geçici] anahtar = değer • ardından /remember approve veya /remember reject");
            return;
        };
        // Session/EphemeralToolOutput geçerli bir expiry olmadan hiç kaydedilemez
        // (`validate_memory_record` bunu zorunlu kılıyor) — kullanıcı `/remember ttl <saat>` ile
        // kendi süresini vermezse akış bir hatayla tıkanmasın diye makul bir varsayılan süre.
        let default_expires_at = match namespace {
            MemoryNamespace::Session => Some(now_epoch() + 4 * 3_600),
            MemoryNamespace::EphemeralToolOutput => Some(now_epoch() + 30 * 60),
            _ => None,
        };
        match propose_memory_with_trust_and_scope(
            namespace,
            key,
            value,
            DataSensitivity::Internal,
            "tui-user-approved-profile",
            true,
            default_expires_at,
            TrustLevel::UserAsserted,
            scope_id,
        ) {
            Ok(proposal) => {
                app.push_system(pending_memory_preview(&proposal));
                app.pending_memory = Some(proposal);
            }
            Err(error) => app.push_system(format!("Bellek teklifi geçersiz: {error}")),
        }
        return;
    }
    // Kullanıcının "secret'ları doğrudan hafızaya yazmıyoruz; sadece Secret Manager referansı
    // tutuluyor" kuralı. `/remember`'dan tamamen ayrı bir komut — gerçek değer asla `memories`
    // tablosuna gitmiyor, yalnız ayrı `secrets` tablosuna. İkinci bir onay adımı yok (`/secret`
    // yazmanın kendisi zaten açık komut) — tıpkı doğal dil bellek komutları gibi.
    if let Some(rest) = input.strip_prefix("/secret ").map(str::trim) {
        if let Some(key) = rest.strip_prefix("show ").map(str::trim) {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .reveal_secret(key)
            {
                Ok(Some(value)) => app.push_system(format!("{key} = {value}")),
                Ok(None) => app.push_system(format!("'{key}' adıyla kayıtlı bir sır yok.")),
                Err(error) => app.push_system(format!("Görüntülenemedi: {error}")),
            }
            return;
        }
        if let Some(key) = rest.strip_prefix("forget ").map(str::trim) {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .forget_secret(key)
            {
                Ok(true) => app.push_system(format!("'{key}' sırrı silindi.")),
                Ok(false) => app.push_system(format!("'{key}' adıyla kayıtlı bir sır yok.")),
                Err(error) => app.push_system(format!("Silinemedi: {error}")),
            }
            return;
        }
        let Some((key, value)) = rest
            .split_once('=')
            .map(|(key, value)| (key.trim(), value.trim()))
        else {
            app.push_system(
                "Kullanım: /secret anahtar = değer • /secret show <anahtar> • /secret forget <anahtar> • /secrets (listele)",
            );
            return;
        };
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .remember_secret(key, value)
        {
            Ok(()) => app.push_system(format!(
                "Sırrı kaydettim: {key} • gerçek değer yalnız Secret Manager'da, sıradan belleğe/modele hiç gitmiyor."
            )),
            Err(error) => app.push_system(format!("Kaydedemedim: {error}")),
        }
        return;
    }
    if input == "/secrets" {
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .list_secret_keys()
        {
            Ok(keys) if keys.is_empty() => app.push_system("Kayıtlı sır yok.".to_string()),
            Ok(keys) => app.push_system(format!(
                "Kayıtlı sır anahtarları (değerler gösterilmez): {}",
                keys.join(", ")
            )),
            Err(error) => app.push_system(format!("Listelenemedi: {error}")),
        }
        return;
    }
    if let Some(word) = input.strip_prefix("/forget namespace ").map(str::trim) {
        let Some(namespace) = parse_memory_namespace(word) else {
            app.push_system(
                "Bilinmeyen namespace. Kullan: profil, proje, görev, oturum veya geçici.",
            );
            return;
        };
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .delete_memory_namespace(namespace)
        {
            Ok(count) => app.push_system(format!(
                "{namespace} namespace'inden {count} kayıt silindi. Diğer namespace'ler etkilenmedi.",
                namespace = namespace.as_str()
            )),
            Err(error) => app.push_system(format!("Namespace silinemedi: {error}")),
        }
        return;
    }
    if let Some(memory_id) = input.strip_prefix("/forget ").map(str::trim) {
        let result = if memory_id == "all" {
            runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .forget_all_memory()
                .map(|count| format!("{count} bellek kaydı silindi."))
        } else {
            runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .delete_memory(memory_id)
                .map(|deleted| {
                    if deleted {
                        format!("Bellek silindi: {memory_id}")
                    } else {
                        "Bu ID ile bellek kaydı yok.".into()
                    }
                })
        };
        match result {
            Ok(message) => app.push_system(message),
            Err(error) => app.push_system(format!("Bellek silinemedi: {error}")),
        }
        return;
    }
    if let Some(path) = input
        .strip_prefix("/profile export ")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let snapshot = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .profile_snapshot();
        match snapshot.and_then(|snapshot| {
            profile_manifest(&snapshot).and_then(|manifest| {
                std::fs::write(path, manifest)
                    .map_err(|error| format!("profile manifest write failed: {error}"))
            })
        }) {
            Ok(()) => app.push_system(
                "Profil dışa aktarıldı; yalnız bilinen alan adları/değerleri/güncelleme zamanı içerir.",
            ),
            Err(error) => app.push_system(format!("Profil dışa aktarılamadı: {error}")),
        }
        return;
    }
    if input == "/profile reset" {
        let mut runtime_guard = runtime.lock().expect("JARVIS runtime lock poisoned");
        match runtime_guard.profile_snapshot() {
            Ok(snapshot) => {
                let populated = snapshot.populated_fields();
                if populated.is_empty() {
                    app.push_system("Silinecek profil alanı yok.");
                } else {
                    let mut deleted = 0usize;
                    for field in populated {
                        if let Some(record) = snapshot.record_for(field) {
                            if runtime_guard
                                .delete_memory(&record.memory_id)
                                .unwrap_or(false)
                            {
                                deleted += 1;
                            }
                        }
                    }
                    app.push_system(format!(
                        "{deleted} profil alanı silindi. Serbest anahtarlarla kaydedilmiş diğer bellekler (/memory) etkilenmedi."
                    ));
                }
            }
            Err(error) => app.push_system(format!("Profil okunamadı: {error}")),
        }
        return;
    }
    if let Some(field_name) = input.strip_prefix("/profile delete ").map(str::trim) {
        let Some(field) = ProfileField::from_user_input(field_name) else {
            app.push_system(
                "Bilinmeyen profil alanı. Kullan: ad, hitap, dil veya rol.".to_string(),
            );
            return;
        };
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .delete_profile_field(field)
        {
            Ok(true) => app.push_system(format!("{} silindi.", field.label())),
            Ok(false) => app.push_system(format!("{} zaten ayarlanmamış.", field.label())),
            Err(error) => app.push_system(format!("Silinemedi: {error}")),
        }
        return;
    }
    if let Some(specification) = input.strip_prefix("/profile set ").map(str::trim) {
        let Some((field_name, value)) = specification.split_once('=') else {
            app.push_system(
                "Kullanım: /profile set <ad|hitap|dil|rol> = <değer> • ardından /remember approve veya /remember reject",
            );
            return;
        };
        let Some(field) = ProfileField::from_user_input(field_name) else {
            app.push_system(
                "Bilinmeyen profil alanı. Kullan: ad, hitap, dil veya rol.".to_string(),
            );
            return;
        };
        match propose_profile_field(field, value.trim(), "tui-profile", true) {
            Ok(proposal) => {
                app.push_system(format!(
                    "Profil teklifi (henüz kaydedilmedi): {} = {}\nOnay: /remember approve • Vazgeç: /remember reject",
                    field.label(),
                    proposal.record.value,
                ));
                app.pending_memory = Some(proposal);
            }
            Err(error) => app.push_system(format!("Profil teklifi geçersiz: {error}")),
        }
        return;
    }
    if input == "/profile" {
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .profile_snapshot()
        {
            Ok(snapshot) => {
                let lines = ProfileField::ALL
                    .into_iter()
                    .map(|field| match snapshot.record_for(field) {
                        Some(record) => format!(
                            "{}: {} • modele dahil={}",
                            field.label(),
                            record.value,
                            record.include_in_model_context
                        ),
                        None => format!("{}: ayarlanmamış", field.label()),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                app.push_system(format!(
                    "Profil:\n{lines}\nDeğiştir: /profile set <ad|hitap|dil|rol> = <değer> • Sil: /profile delete <alan> • Hepsini sil: /profile reset • Dışa aktar: /profile export <dosya-yolu>"
                ));
            }
            Err(error) => app.push_system(format!("Profil okunamadı: {error}")),
        }
        return;
    }
    if let Some(rest) = input.strip_prefix("/index-preview ").map(str::trim) {
        let mut parts = rest.split_whitespace();
        let Some(folder) = parts.next() else {
            app.push_system("Kullanım: /index-preview <proje-içi-göreli-klasör> [hariç-desen ...]");
            return;
        };
        let exclude_patterns: Vec<String> = parts.map(str::to_owned).collect();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = root.join(folder);
        match preview_workspace_index(&target, &exclude_patterns) {
            Ok(preview) => {
                let sample = preview
                    .included
                    .iter()
                    .take(10)
                    .map(|path| format!("  {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                let more = preview.included.len().saturating_sub(10);
                let more_note = if more > 0 {
                    format!("\n  … ve {more} dosya daha")
                } else {
                    String::new()
                };
                app.push_system(format!(
                    "Önizleme: {} — {} dosya indekslenecek (~{} KiB), {} şifre-benzeri, {} boyut limiti üstü, {} desenle hariç tutuldu.\n{sample}{more_note}\nGerçekten indekslemek için: /index-folder {folder}{}",
                    preview.root.display(),
                    preview.included.len(),
                    preview.estimated_total_bytes.div_ceil(1024),
                    preview.excluded_secret_like.len(),
                    preview.excluded_oversized.len(),
                    preview.excluded_by_pattern.len(),
                    if exclude_patterns.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", exclude_patterns.join(" "))
                    }
                ));
            }
            Err(error) => app.push_system(format!("Önizleme alınamadı: {error}")),
        }
        return;
    }
    if let Some(rest) = input.strip_prefix("/index-folder ").map(str::trim) {
        let mut parts = rest.split_whitespace();
        let Some(folder) = parts.next() else {
            app.push_system("Kullanım: /index-folder <proje-içi-göreli-klasör> [hariç-desen ...] [public|internal|sensitive]");
            return;
        };
        let mut exclude_patterns: Vec<String> = parts.map(str::to_owned).collect();
        // F3 post-close "retrieval öncesi permission/sensitivity filtresi": an optional trailing
        // word marks every file in the folder — "finans klasörüm hassas" in one command, not one
        // per file.
        let sensitivity = exclude_patterns
            .last()
            .and_then(|word| parse_data_sensitivity(word))
            .inspect(|_| {
                exclude_patterns.pop();
            })
            .unwrap_or(DataSensitivity::Internal);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = root.join(folder);
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .index_workspace_folder_with_sensitivity(&target, &exclude_patterns, sensitivity, true)
        {
            Ok(report) => {
                let changed = report
                    .indexed
                    .iter()
                    .filter(|ingestion| ingestion.content_changed)
                    .count();
                let unchanged = report.indexed.len() - changed;
                let mut message = format!(
                    "{changed} dosya indekslendi, {unchanged} dosya zaten güncel (değişmemiş, atlandı)."
                );
                if !report.failed.is_empty() {
                    let failures = report
                        .failed
                        .iter()
                        .map(|(path, error)| format!("{}: {error}", path.display()))
                        .collect::<Vec<_>>()
                        .join("; ");
                    message.push_str(&format!("\nBaşarısız: {failures}"));
                }
                app.push_system(message);
            }
            Err(error) => app.push_system(format!("Klasör indekslenemedi: {error}")),
        }
        return;
    }
    if let Some(rest) = input.strip_prefix("/index ").map(str::trim) {
        // F3 post-close "retrieval öncesi permission/sensitivity filtresi": an optional trailing
        // word ("hassas"/"sensitive"/...) marks this file so it never surfaces as an automatic
        // citation in ordinary conversation — omit it and the file indexes at the same default
        // (Internal) it always has.
        let (relative_path, sensitivity) = split_trailing_sensitivity(rest);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .index_workspace_document_with_sensitivity(
                &root,
                std::path::Path::new(relative_path),
                sensitivity,
                true,
            )
        {
            Ok(report) if report.content_changed => app.push_system(format!(
                "Dosya indekslendi: {} • {} parça • SHA-256:{}…\nBu klasöre ait metin artık yalnız ilgili sorularda kaynaklı data olarak kullanılabilir.",
                report.canonical_path.display(),
                report.chunk_count,
                &report.content_sha256[..12]
            )),
            Ok(report) => app.push_system(format!(
                "Dosya değişmemiş, zaten güncel: {} • {} parça (yeniden işlenmedi).",
                report.canonical_path.display(),
                report.chunk_count
            )),
            Err(error) => app.push_system(format!("Dosya indekslenemedi: {error}")),
        }
        return;
    }
    if let Some(rest) = input.strip_prefix("/source ").map(str::trim) {
        // F3 "Citation UX: ... kaynağı aç davranışı". Shows the full, untruncated chunk text and
        // the complete file path for one citation from the *last* JARVIS reply — the short
        // excerpt already printed under that reply is only a preview.
        match rest.parse::<usize>() {
            Ok(index) if index >= 1 && index <= app.last_citations.len() => {
                let citation = &app.last_citations[index - 1];
                app.push_system(format!(
                    "[{index}] {}#chunk-{}\n\n{}",
                    citation.canonical_path.display(),
                    citation.chunk_ordinal,
                    citation.content
                ));
            }
            Ok(_) if app.last_citations.is_empty() => {
                app.push_system("Son yanıtın kaynağı yok, açılacak bir şey bulunamadı.");
            }
            Ok(_) => app.push_system(format!(
                "Geçersiz kaynak numarası. Son yanıtta 1-{} arası kaynak var.",
                app.last_citations.len()
            )),
            Err(_) => app.push_system("Kullanım: /source <numara> (örn. /source 1)"),
        }
        return;
    }
    if let Some(exit_action) = tui_exit_action(&input) {
        if exit_action == TuiExitAction::StopModelAndExit {
            app.status = match stop_local_model_server() {
                Ok(()) => "JARVIS kapandı; model RAM'den çıkarıldı.".into(),
                Err(error) => error,
            };
        }
        app.running = false;
        return;
    }
    if let Some(path) = input
        .strip_prefix("/memory export ")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let records = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .list_memory();
        match records
            .and_then(|records| memory_export(&records))
            .and_then(|manifest| {
                std::fs::write(path, manifest)
                    .map_err(|error| format!("memory export write failed: {error}"))
            }) {
            Ok(()) => app.push_system(
                "Tüm bellek dışa aktarıldı (tüm namespace'ler); ham memory_id/source içermez.",
            ),
            Err(error) => app.push_system(format!("Bellek dışa aktarılamadı: {error}")),
        }
        return;
    }
    if let Some(path) = input
        .strip_prefix("/memory import ")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        match std::fs::read_to_string(path) {
            Ok(json) => match memory_import("tui-memory-import", &json) {
                Ok((proposals, skipped)) => {
                    let mut runtime_guard = runtime.lock().expect("JARVIS runtime lock poisoned");
                    let mut imported = 0usize;
                    let mut failed = Vec::new();
                    for proposal in &proposals {
                        match runtime_guard.commit_memory_proposal(proposal, true) {
                            Ok(_) => imported += 1,
                            Err(error) => failed.push(format!("{}: {error}", proposal.record.key)),
                        }
                    }
                    let mut message =
                        format!("{imported}/{} kayıt içe aktarıldı.", proposals.len());
                    if !skipped.is_empty() {
                        message.push_str(&format!(
                            "\nAtlanan (bozuk) satırlar: {}",
                            skipped.join("; ")
                        ));
                    }
                    if !failed.is_empty() {
                        message.push_str(&format!("\nKaydedilemeyenler: {}", failed.join("; ")));
                    }
                    app.push_system(message);
                }
                Err(error) => app.push_system(format!("İçe aktarma dosyası geçersiz: {error}")),
            },
            Err(error) => app.push_system(format!("Dosya okunamadı: {error}")),
        }
        return;
    }
    match input.as_str() {
        "/clear" => {
            app.messages.clear();
            // Konuşma geçmişi artık diske de yazılıyor (2026-08-16), bu yüzden /clear yalnız
            // görünen listeyi değil modelin gerçek bağlamını ve diskteki kopyayı da siliyor —
            // aksi halde ekran boş görünürken model eski geçmişi sessizce tutmaya devam ederdi.
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .clear_chat_history()
            {
                Ok(_) => app.push_system(
                    "Sohbet temizlendi — hem görünen liste hem modele giden bağlam hem diskteki kayıt silindi.",
                ),
                Err(error) => app.push_system(format!("Sohbet görünümü temizlendi ama disk kaydı silinemedi: {error}")),
            }
            return;
        }
        "/attachments clear" => {
            let count = app.attachments.len();
            app.attachments.clear();
            app.push_system(format!("{count} ek gönderilmeden kuyruktan kaldırıldı."));
            return;
        }
        "/attachments" => {
            if app.attachments.is_empty() {
                app.push_system("Ek kuyruğu boş.");
            } else {
                let queued = app
                    .attachments
                    .iter()
                    .map(|attachment| {
                        let details = if attachment.kind.is_image() {
                            format!("{}×{}", attachment.width, attachment.height)
                        } else {
                            format!("{} KiB", attachment.byte_size.div_ceil(1024))
                        };
                        format!(
                            "{} • {} • {} • {}…",
                            attachment.original_name,
                            attachment.mime_type(),
                            details,
                            &attachment.attachment_id[11..]
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                app.push_system(format!(
                    "Gönderilmeyi bekleyen ekler:\n{queued}\nTümünü kaldırmak için /attachments clear"
                ));
            }
            return;
        }
        "/memory" => {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .list_memory()
            {
                Ok(records) if records.is_empty() => app.push_system("Kaydedilmiş bellek yok."),
                Ok(records) => {
                    let listed = records
                        .iter()
                        .map(|record| {
                            format!(
                                "{} • {} = {} • {} • modele dahil={} • sil: /forget {}",
                                record.memory_id,
                                record.key,
                                record.value,
                                record.namespace.as_str(),
                                record.include_in_model_context,
                                record.memory_id
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    app.push_system(format!(
                        "Kaydedilmiş bellekler:\n{listed}\nTümünü sil: /forget all"
                    ));
                }
                Err(error) => app.push_system(format!("Bellek listelenemedi: {error}")),
            }
            return;
        }
        "/help" => {
            app.push_system("Kısayollar: Enter gönder • Ctrl+V yapıştır • Ctrl+Backspace veya Ctrl+W önceki kelimeyi sil • Ctrl+U taslağı temizle • Esc taslağı sil. Geçmiş: ↑/↓ veya PageUp/PageDown. Ek: /attach <PNG/JPEG/TXT/MD/PDF-yolu>, /attachments, /attachments clear, /attachment-history, /attachment-history remove <id>|clear, /attachment-export <dosya-yolu>. Belge ekleri metadata-only'dir; indeksleme için ayrı /index akışı kullanılır. Bellek: /remember [profil|proje|görev <task-id>|oturum|geçici] anahtar = değer (namespace verilmezse profil), /remember sensitivity <public|internal|sensitive>, /remember ttl <saat|none>, /remember model-context <evet|hayır>, /remember approve|reject, /memory, /forget <id>|all, /forget namespace <profil|proje|görev|oturum|geçici>, /memory export <dosya-yolu>, /memory import <dosya-yolu>. Sır: /secret anahtar = değer (Secret Manager'a gider, sıradan belleğe/modele hiç gitmez), /secret show <anahtar>, /secret forget <anahtar>, /secrets (yalnız anahtarları listeler). Profil: /profile, /profile set <ad|hitap|dil|rol> = <değer> (onay: /remember approve), /profile delete <alan>, /profile reset, /profile export <dosya-yolu>. RAG: /index <proje-içi-göreli-dosya> [public|internal|sensitive], /index-preview <proje-içi-göreli-klasör> [hariç-desen ...], /index-folder <proje-içi-göreli-klasör> [hariç-desen ...] [public|internal|sensitive], /source <numara> (son yanıtın kaynağının tamamını aç), /rag status, /rag rebuild, /rag verify. Komutlar: /status, /approvals, /approve, /cancel, /clear, /quit, exit. `exit` modeli RAM'den çıkarır; /quit veya Ctrl+C yalnız arayüzü kapatır.");
            return;
        }
        "/approvals" | "approvals" => {
            show_pending_approvals(app, runtime);
            return;
        }
        "/approve" | "approve" => {
            let task_id = single_pending_task_id(runtime);
            match task_id {
                Ok(task_id) => approve_task(app, runtime, &task_id),
                Err(message) => app.push_system(message),
            }
            return;
        }
        "/cancel" | "cancel" => {
            let task_id = single_pending_task_id(runtime);
            match task_id {
                Ok(task_id) => cancel_task(app, runtime, &task_id),
                Err(message) => app.push_system(message),
            }
            return;
        }
        "/status" => {
            let rag_mode = runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .embedding_status()
                .map(|model_id| format!("hybrid (FTS + {model_id})"))
                .unwrap_or_else(|| "FTS-only".into());
            app.push_system(format!(
                "Model server: {} • CPU-only • VRAM layer: 0 • RAG: {rag_mode} • {}",
                model_label(&app.model_state),
                app.status
            ));
            return;
        }
        "/rag status" => {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .rag_status()
            {
                Ok(status) => {
                    let mode = status
                        .embedding_model
                        .as_deref()
                        .map(|model_id| format!("hybrid (FTS + {model_id})"))
                        .unwrap_or_else(|| "FTS-only".into());
                    let coverage = if status.embedding_model.is_some() {
                        format!(
                            ", embed edilmiş: {}/{}",
                            status.embedded_chunk_count, status.chunk_count
                        )
                    } else {
                        String::new()
                    };
                    app.push_system(format!(
                        "RAG: {mode} • {} belge, {} chunk{coverage} • bu oturum: {} hibrit, {} yalnız-FTS sorgu",
                        status.document_count,
                        status.chunk_count,
                        status.hybrid_queries_this_session,
                        status.fts_only_queries_this_session
                    ));
                }
                Err(error) => app.push_system(format!("RAG durumu okunamadı: {error}")),
            }
            return;
        }
        "/rag rebuild" => {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .rebuild_rag_index()
            {
                Ok(count) => app.push_system(format!(
                    "{count} chunk için embedding sıfırdan yeniden hesaplandı."
                )),
                Err(error) => app.push_system(format!("Yeniden inşa edilemedi: {error}")),
            }
            return;
        }
        "/rag verify" => {
            match runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .verify_rag_index()
            {
                Ok(report) if report.is_healthy() => app.push_system(format!(
                    "RAG indeksi sağlıklı • {} belge, {} chunk, {} embedding.",
                    report.document_count, report.chunk_count, report.embedded_chunk_count
                )),
                Ok(report) => {
                    let mut problems = Vec::new();
                    if report.orphaned_embedding_count > 0 {
                        problems.push(format!(
                            "{} sahipsiz embedding kaydı",
                            report.orphaned_embedding_count
                        ));
                    }
                    if let Some(missing) = report.chunks_missing_embedding {
                        if missing > 0 {
                            problems.push(format!("{missing} chunk'ta embedding eksik"));
                        }
                    }
                    app.push_system(format!(
                        "RAG indeksinde sorun var: {}. Düzeltmek için /rag rebuild dene.",
                        problems.join(", ")
                    ));
                }
                Err(error) => app.push_system(format!("Doğrulanamadı: {error}")),
            }
            return;
        }
        _ => {}
    }
    if app.model_state != "ready" {
        app.input = input;
        app.status = "Model henüz RAM'e yükleniyor; mesajın kutuda tutuluyor. Birkaç saniye sonra Enter'a tekrar bas.".into();
        return;
    }
    // A newly submitted turn should always return the view to the newest conversation content.
    return_to_latest(&mut app.scroll);
    let attachments = std::mem::take(&mut app.attachments);
    let attachment_receipts = attachments
        .iter()
        .map(AttachmentReceipt::from_attachment)
        .collect::<Vec<_>>();
    let needs_vision = attachments
        .iter()
        .any(|attachment| attachment.kind.is_image());
    let attachment_summary = if attachments.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n[Ekler: {}]",
            attachments
                .iter()
                .map(|attachment| attachment.original_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    app.messages.push(Message {
        role: MessageRole::User,
        content: format!("{input}{attachment_summary}"),
    });
    let message_index = app.messages.len();
    app.messages.push(Message {
        role: MessageRole::Jarvis,
        content: "Düşünüyorum…".into(),
    });
    app.pending = true;
    app.status = if needs_vision {
        "Görsel analiz hazırlanıyor; yalnız local vision sunucusu kullanılacak…".into()
    } else {
        "JARVIS yanıt üretiyor…".into()
    };
    let runtime = Arc::clone(runtime);
    let provider = provider.clone();
    let vision = vision.clone();
    let sender = sender.clone();
    std::thread::spawn(move || {
        let vision_available = if needs_vision {
            ensure_local_vision_server(&vision).is_ok()
        } else {
            false
        };
        let mut vision_for_request = vision;
        if !vision_available {
            // A service startup failure should become the privacy-safe Runtime failure promptly,
            // rather than leaving the UI waiting for the normal image-analysis timeout.
            vision_for_request.timeout_seconds = 1;
        }
        let request = Request {
            schema_version: 1,
            request_id: format!(
                "tui-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock must be after UNIX epoch")
                    .as_nanos()
            ),
            input_type: InputType::Gui,
            content: input,
            attachments,
        };
        let mut runtime_guard = runtime.lock().expect("JARVIS runtime lock poisoned");
        let (task, tool, verification) = runtime_guard.handle_with_provider_and_vision(
            request,
            &provider,
            needs_vision.then_some(&vision_for_request),
        );
        // F3 "Citation UX": read back the exact citations that grounded this reply (full chunk
        // content, not just the compact evidence strings) while the lock is still held, so
        // `/source <n>` can later show the complete text without re-querying the store.
        let citations = runtime_guard.last_workspace_citations().to_vec();
        drop(runtime_guard);
        // F3 "Citation UX: ... hangi belge/parçadan geldiği, kısa alıntı, dosya konumu". Numbered
        // so a citation line and the later `/source <n>` command point at the same thing.
        let mut sources: Vec<String> = citations
            .iter()
            .enumerate()
            .map(|(index, citation)| {
                format!(
                    "• [{}] {}#chunk-{} — \"{}\" (tamamı için: /source {})",
                    index + 1,
                    citation.canonical_path.display(),
                    citation.chunk_ordinal,
                    citation.short_excerpt(96),
                    index + 1
                )
            })
            .collect();
        sources.extend(tool.evidence.iter().filter_map(|evidence| {
            evidence
                .strip_prefix("vision.analysis:")
                .filter(|attachment_id| *attachment_id != "unavailable")
                .map(|attachment_id| format!("• Local vision analizi: {attachment_id}"))
                .or_else(|| {
                    // "Neden kullanıldı" görünürlüğü: hangi kayıtlı bilgi (profil/proje/vb.)
                    // bu yanıt için modele verildiğini gösterir. Değeri değil, yalnız
                    // namespace:anahtar'ı — kaynak satırı uzun/hassas bir değeri tekrar
                    // etmesin diye.
                    evidence
                        .strip_prefix("memory.used:")
                        .map(|reference| format!("• Kayıtlı bilgi kullanıldı: {reference}"))
                })
        }));
        let content = tool.error.clone().unwrap_or(tool.output);
        let approval_pending = task.state == TaskState::WaitingForUser;
        let notification = tui_notification(task.state, &content);
        let status = match task.state {
            TaskState::WaitingForUser => "İşlem onayını bekliyor.".into(),
            TaskState::Completed => format!("Yanıt hazır • doğrulama: {:?}", verification.status),
            _ => format!(
                "İşlem {:?} • doğrulama: {:?}",
                task.state, verification.status
            ),
        };
        let _ = sender.send(WorkerReply {
            message_index,
            content,
            status,
            task_id: task.task_id,
            approval_pending,
            notification,
            sources,
            citations,
            attachment_receipts,
        });
    });
}

fn return_to_latest(scroll: &mut u16) {
    *scroll = 0;
}

fn notification_preview(content: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 180;
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = compact.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if compact.chars().count() > MAX_PREVIEW_CHARS {
        preview.push('…');
    }
    preview
}

fn tui_notification(task_state: TaskState, content: &str) -> Option<TuiNotification> {
    if content.trim().is_empty() {
        return None;
    }
    let title = match task_state {
        TaskState::Completed => "JARVIS yanıtı hazır",
        TaskState::WaitingForUser => "JARVIS onayı bekliyor",
        TaskState::Failed | TaskState::Interrupted => "JARVIS işlem hatası",
        TaskState::Queued | TaskState::Running | TaskState::Cancelled => return None,
    };
    Some(TuiNotification {
        title,
        content: content.into(),
    })
}

fn notification_arguments(title: &str, content: &str) -> Option<Vec<String>> {
    let preview = notification_preview(content);
    if preview.is_empty() {
        return None;
    }
    Some(vec![
        "--app-name=JARVIS".into(),
        "--icon=dialog-information".into(),
        "--expire-time=6000".into(),
        title.into(),
        preview,
    ])
}

/// Notifications are best-effort: a missing notification daemon must never affect a completed
/// task or the terminal UI. `notify-send` integrates with Hyprland's standard notification path.
fn notify_desktop(title: &str, content: &str) {
    let _ = try_notify_desktop(title, content, |arguments| {
        Command::new("notify-send")
            .args(arguments)
            .status()
            .map(|_| ())
            .map_err(|error| error.to_string())
    });
}

/// Returns whether a valid notification was attempted. The sender error is deliberately ignored:
/// desktop notification infrastructure is display-only and never changes task/UI state.
fn try_notify_desktop<F>(title: &str, content: &str, sender: F) -> bool
where
    F: FnOnce(&[String]) -> Result<(), String>,
{
    let Some(arguments) = notification_arguments(title, content) else {
        return false;
    };
    let _ = sender(&arguments);
    true
}

fn single_pending_task_id(runtime: &Arc<Mutex<Runtime>>) -> Result<String, String> {
    let runtime = runtime.lock().expect("JARVIS runtime lock poisoned");
    let pending: Vec<String> = runtime
        .pending_approvals()
        .into_iter()
        .map(|approval| approval.task_id.clone())
        .collect();
    match pending.as_slice() {
        [] => Err("Onay bekleyen işlem yok.".into()),
        [task_id] => Ok(task_id.clone()),
        _ => Err(
            "Birden fazla işlem onay bekliyor. /approvals ile ID'yi görüp /approve <task-id> yaz."
                .into(),
        ),
    }
}

fn show_pending_approvals(app: &mut App, runtime: &Arc<Mutex<Runtime>>) {
    let runtime = runtime.lock().expect("JARVIS runtime lock poisoned");
    let pending: Vec<String> = runtime
        .pending_approvals()
        .into_iter()
        .map(|approval| format!("{} • {}", approval.task_id, approval.action_id))
        .collect();
    if pending.is_empty() {
        app.push_system("Onay bekleyen işlem yok.");
    } else {
        app.push_system(format!(
            "Onay bekleyen işlemler:\n{}\nOnay: /approve <task-id> • Vazgeç: /cancel <task-id>",
            pending.join("\n")
        ));
    }
}

fn approve_task(app: &mut App, runtime: &Arc<Mutex<Runtime>>, task_id: &str) {
    let result = runtime
        .lock()
        .expect("JARVIS runtime lock poisoned")
        .approve(task_id);
    match result {
        Some((task, tool, verification)) => {
            let content = tool.error.unwrap_or(tool.output);
            app.push_system(format!(
                "Onaylı işlem tamamlandı ({}) • verifier={:?}\n{}",
                task.task_id, verification.status, content
            ));
            app.status = format!(
                "task={} • {:?} • verifier={:?}",
                task.task_id, task.state, verification.status
            );
        }
        None => app
            .push_system("Onay uygulanamadı: task bulunamadı, süresi doldu veya artık beklemiyor."),
    }
}

fn cancel_task(app: &mut App, runtime: &Arc<Mutex<Runtime>>, task_id: &str) {
    let task = runtime
        .lock()
        .expect("JARVIS runtime lock poisoned")
        .cancel(task_id);
    match task {
        Some(task) => {
            app.push_system(format!("İşlem iptal edildi: {}", task.task_id));
            app.status = format!("task={} • {:?}", task.task_id, task.state);
        }
        None => app.push_system("İptal uygulanamadı: task bulunamadı veya artık onay beklemiyor."),
    }
}

/// Keeps the cursor and the newest portion of a long draft visible in a fixed-height input box.
/// Input is character-based so backspace and Turkish text remain safe; it deliberately does not
/// edit the read-only message history above it.
fn input_view(input: &str, width: u16, rows: u16) -> (Vec<Line<'static>>, u16, u16) {
    let width = usize::from(width.max(1));
    let rows = usize::from(rows.max(1));
    let capacity = width.saturating_mul(rows);
    let mut visible: Vec<char> = input.chars().collect();
    let was_clipped = visible.len() > capacity;
    if was_clipped {
        let start = visible.len() - capacity;
        visible = visible[start..].to_vec();
        if let Some(first) = visible.first_mut() {
            *first = '…';
        }
    }

    let lines = visible
        .chunks(width)
        .map(|chunk| Line::from(chunk.iter().collect::<String>()))
        .collect::<Vec<_>>();
    let visible_len = visible.len();
    let (cursor_row, cursor_column) = if visible_len == capacity {
        ((rows - 1) as u16, (width - 1) as u16)
    } else {
        ((visible_len / width) as u16, (visible_len % width) as u16)
    };
    (lines, cursor_row, cursor_column)
}

fn wrapped_rows(text: &str, width: u16) -> usize {
    let width = usize::from(width.max(1));
    text.chars().count().max(1).div_ceil(width)
}

fn draft_rows(input: &str, width: u16) -> u16 {
    wrapped_rows(input, width).min(u16::MAX as usize) as u16
}

fn history_lines(messages: &[Message]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for message in messages {
        let (label, color) = match message.role {
            MessageRole::User => (" SEN ", Color::Yellow),
            MessageRole::Jarvis => (" JARVIS ", Color::Green),
            MessageRole::System => (" SİSTEM ", Color::Cyan),
        };
        lines.push(Line::from(Span::styled(
            label,
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        )));
        for line in message.content.lines() {
            lines.push(Line::from(Span::raw(format!("  {line}"))));
        }
        lines.push(Line::raw(""));
    }
    lines
}

/// Uses Ratatui's own Unicode-aware word-wrapper for both rendering and measuring.  A manual
/// character count can disagree on emoji or word boundaries and leave the live turn off-screen.
fn history_line_count(lines: &[Line<'static>], width: u16) -> usize {
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
}

fn draw(area: Rect, frame: &mut ratatui::Frame, app: &App) {
    const HEADER_HEIGHT: u16 = 3;
    const FOOTER_HEIGHT: u16 = 1;
    const MIN_HISTORY_HEIGHT: u16 = 6;
    const MAX_INPUT_ROWS: u16 = 12;

    let draft_width = area.width.saturating_sub(4);
    let maximum_rows = area
        .height
        .saturating_sub(2 + HEADER_HEIGHT + MIN_HISTORY_HEIGHT + FOOTER_HEIGHT + 2)
        .clamp(1, MAX_INPUT_ROWS);
    let visible_draft_rows = draft_rows(&app.input, draft_width).clamp(1, maximum_rows);
    let input_height = visible_draft_rows + 2;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Min(MIN_HISTORY_HEIGHT),
            Constraint::Length(input_height),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(area);
    let server_badge = if app.model_state == "ready" {
        Span::styled(
            " MODEL HAZIR ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " MODEL BAŞLATILIYOR ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " JARVIS ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  Local-first personal AI",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  •  "),
        server_badge,
        Span::raw(format!("  •  VRAM: 0  •  EK: {}", app.attachments.len())),
    ]))
    .block(Block::default().borders(Borders::ALL).title(" Sohbet "));
    frame.render_widget(header, layout[0]);

    let lines = history_lines(&app.messages);
    let history_block = Block::default().borders(Borders::ALL);
    let history_inner = history_block.inner(layout[1]);
    let viewport_height = usize::from(history_inner.height);
    // Reserve a distinct column for the scrollbar only when it is needed. The same Ratatui
    // wrapper measures and renders the text, so the newest turn lands at the actual bottom.
    let full_width = history_inner.width.max(1);
    let needs_scrollbar = history_line_count(&lines, full_width) > viewport_height;
    let content_width = if needs_scrollbar {
        history_inner.width.saturating_sub(1).max(1)
    } else {
        full_width
    };
    let content_height = history_line_count(&lines, content_width);
    let needs_scrollbar = content_height > viewport_height;
    let max_scroll = content_height.saturating_sub(viewport_height);
    // `scroll` is an offset from the newest message: zero means follow the live conversation.
    let scroll_position = max_scroll.saturating_sub(usize::from(app.scroll).min(max_scroll));
    let history_content_area = Rect {
        x: history_inner.x,
        y: history_inner.y,
        width: content_width,
        height: history_inner.height,
    };
    let history = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_position.min(u16::MAX as usize) as u16, 0));
    let history_title = if needs_scrollbar {
        " Mesajlar — ↑↓ kaydır "
    } else {
        " Mesajlar "
    };
    frame.render_widget(history_block.title(history_title), layout[1]);
    frame.render_widget(history, history_content_area);
    if needs_scrollbar {
        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(scroll_position)
            .viewport_content_length(viewport_height);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_symbol("█")
            .track_symbol(Some("░"))
            .thumb_style(Style::default().fg(Color::Cyan))
            .track_style(Style::default().fg(Color::DarkGray));
        frame.render_stateful_widget(scrollbar, history_inner, &mut scrollbar_state);
    }

    let input_title = if app.pending {
        " Mesaj — yanıt bekleniyor "
    } else if app.attachments.is_empty() {
        " Mesaj — Enter gönder "
    } else {
        " Mesaj — ekler hazır, Enter gönder "
    };
    let input_width = layout[2].width.saturating_sub(2);
    let input_rows = layout[2].height.saturating_sub(2);
    let (input_lines, cursor_row, cursor_column) = input_view(&app.input, input_width, input_rows);
    let input = Paragraph::new(input_lines)
        .block(Block::default().borders(Borders::ALL).title(input_title))
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });
    frame.render_widget(input, layout[2]);
    let footer = Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, layout[3]);
    if !app.pending {
        let cursor_x = (layout[2].x + 1 + cursor_column).min(layout[2].right().saturating_sub(2));
        let cursor_y = (layout[2].y + 1 + cursor_row).min(layout[2].bottom().saturating_sub(2));
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        append_pasted_text, apply_history_key_scroll, apply_history_mouse_scroll,
        delete_previous_word, draft_rows, history_line_count, history_lines, input_view,
        is_clear_draft_shortcut, is_clipboard_paste_shortcut, is_delete_previous_word_shortcut,
        is_primary_selection_paste, native_desktop_binary_path, notification_arguments,
        notification_preview, parse_remember_namespace_prefix, return_to_latest,
        should_clear_draft, should_close_tui_for_key, submit, try_notify_desktop, tui_exit_action,
        tui_notification, App, Message, MessageRole, TuiExitAction, WorkerReply,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
    use jarvis_core::{
        parse_memory_intent, LlamaServerProvider, LlamaVisionServerProvider, MemoryNamespace,
        ProfileField, Runtime, SqliteStore, TaskState, WorkspaceCitation,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// TUI komut testleri kalıcı bellek/profil gerektirdiği için gerçek (geçici) bir SQLite
    /// store'a bağlı bir Runtime kurar; `Runtime::new()` (store'suz) bu komutlarda hep hata döner.
    fn stored_runtime_fixture() -> (
        Arc<Mutex<Runtime>>,
        LlamaServerProvider,
        LlamaVisionServerProvider,
        mpsc::Sender<WorkerReply>,
    ) {
        let store = SqliteStore::in_memory().expect("in-memory sqlite store");
        let runtime = Arc::new(Mutex::new(Runtime::with_store(store)));
        let provider = LlamaServerProvider::local_default();
        let vision = LlamaVisionServerProvider::local_default();
        let (sender, _receiver) = mpsc::channel();
        (runtime, provider, vision, sender)
    }

    #[test]
    fn long_draft_keeps_its_tail_visible() {
        let (lines, _, _) = input_view("0123456789abcdef", 5, 2);
        let rendered = lines
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.starts_with('…'));
        assert!(rendered.ends_with("bcdef"));
    }

    #[test]
    fn cursor_advances_to_next_input_row() {
        let (_, row, column) = input_view("12345", 5, 3);
        assert_eq!((row, column), (1, 0));
    }

    #[test]
    fn draft_rows_grow_only_when_the_text_needs_another_line() {
        assert_eq!(draft_rows("12345", 5), 1);
        assert_eq!(draft_rows("123456", 5), 2);
    }

    #[test]
    fn history_measurement_uses_the_renderer_word_wrap_rules() {
        let messages = vec![Message {
            role: MessageRole::User,
            content: "merhaba 👋 bu mesaj kaydırma alanında görünür kalmalı".into(),
        }];
        let lines = history_lines(&messages);
        assert_eq!(history_line_count(&lines, 80), 3);
        assert!(history_line_count(&lines, 12) > 3);
    }

    #[test]
    fn notification_preview_is_compact_and_bounded() {
        let content = format!("ilk satır\n ikinci satır {}", "x".repeat(200));
        let preview = notification_preview(&content);
        assert!(preview.starts_with("ilk satır ikinci satır"));
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), 181);
    }

    #[test]
    fn notifications_cover_reply_approval_and_error_without_daemon_authority() {
        assert_eq!(
            tui_notification(TaskState::Completed, "hazır")
                .expect("reply notification")
                .title,
            "JARVIS yanıtı hazır"
        );
        assert_eq!(
            tui_notification(TaskState::WaitingForUser, "onay gerekli")
                .expect("approval notification")
                .title,
            "JARVIS onayı bekliyor"
        );
        assert_eq!(
            tui_notification(TaskState::Failed, "model yok")
                .expect("error notification")
                .title,
            "JARVIS işlem hatası"
        );
        assert!(tui_notification(TaskState::Cancelled, "iptal").is_none());
        assert!(notification_arguments("JARVIS", "\n  ").is_none());
        let arguments = notification_arguments("JARVIS", "ilk satır\nikinci satır")
            .expect("notification arguments");
        assert_eq!(arguments[0], "--app-name=JARVIS");
        assert_eq!(arguments[3], "JARVIS");
        assert_eq!(arguments[4], "ilk satır ikinci satır");
        assert!(try_notify_desktop("JARVIS", "hazır", |_arguments| {
            Err("notification daemon unavailable".into())
        }));
        assert!(!try_notify_desktop("JARVIS", "\n  ", |_arguments| Ok(())));
    }

    #[test]
    fn pasted_multiline_text_stays_in_one_message_draft() {
        let mut input = "Merhaba ".to_owned();
        append_pasted_text(&mut input, "dostum\n  nasılsın?");
        assert_eq!(input, "Merhaba dostum nasılsın?");
    }

    #[test]
    fn word_delete_keeps_utf8_boundaries_intact() {
        let mut input = "merhaba dünya güzel".to_owned();
        delete_previous_word(&mut input);
        assert_eq!(input, "merhaba dünya");
        delete_previous_word(&mut input);
        assert_eq!(input, "merhaba");
    }

    #[test]
    fn editing_shortcuts_cover_terminal_and_control_character_variants() {
        assert!(is_clipboard_paste_shortcut(KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_clipboard_paste_shortcut(KeyEvent::new(
            KeyCode::Char('\u{16}'),
            KeyModifiers::NONE,
        )));
        assert!(is_delete_previous_word_shortcut(KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::CONTROL,
        )));
        assert!(is_delete_previous_word_shortcut(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_clear_draft_shortcut(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        )));
        assert!(should_clear_draft(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!is_delete_previous_word_shortcut(KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn exit_actions_keep_or_release_the_model_only_when_explicit() {
        assert_eq!(
            tui_exit_action("/quit"),
            Some(TuiExitAction::KeepModelInRam)
        );
        assert_eq!(
            tui_exit_action("exit"),
            Some(TuiExitAction::StopModelAndExit)
        );
        assert_eq!(
            tui_exit_action("/exit"),
            Some(TuiExitAction::StopModelAndExit)
        );
        assert_eq!(tui_exit_action("selam"), None);
        assert!(should_close_tui_for_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(!should_close_tui_for_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn unavailable_model_keeps_the_user_draft_for_retry() {
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let provider = LlamaServerProvider::local_default();
        let vision = LlamaVisionServerProvider::local_default();
        let (sender, _receiver) = mpsc::channel();
        let mut app = App::new("missing_executable");
        app.input = "Bu taslak kaybolmamalı".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert_eq!(app.input, "Bu taslak kaybolmamalı");
        assert!(!app.pending);
        assert!(app.status.contains("kutuda tutuluyor"));
    }

    #[test]
    fn compact_terminal_still_shows_the_latest_turn_and_a_scrollbar() {
        let backend = TestBackend::new(56, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new("ready");
        app.messages.push(Message {
            role: MessageRole::User,
            content: "önceki kullanıcı mesajı ".repeat(18),
        });
        app.messages.push(Message {
            role: MessageRole::Jarvis,
            content: "EN_YENI_YANIT görünür kalmalı".into(),
        });
        terminal
            .draw(|frame| super::draw(frame.area(), frame, &app))
            .expect("render compact terminal");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("EN_YENI_YANIT"));
        assert!(rendered.contains("Mesajlar — ↑↓ kaydır"));
    }

    #[test]
    fn tui_resize_keeps_the_composer_cursor_and_latest_turn_in_bounds() {
        let backend = TestBackend::new(48, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new("ready");
        app.input = "Türkçe taslak uzun olsa da imleç composer içinde kalmalı".into();
        app.messages.push(Message {
            role: MessageRole::User,
            content: "Önceki uzun tur ".repeat(14),
        });
        app.messages.push(Message {
            role: MessageRole::Jarvis,
            content: "EN_YENI_TUR responsive terminalde görünür kalmalı".into(),
        });

        for area in [
            ratatui::layout::Rect::new(0, 0, 48, 18),
            ratatui::layout::Rect::new(0, 0, 112, 38),
            ratatui::layout::Rect::new(0, 0, 56, 22),
        ] {
            terminal.resize(area).expect("resize test terminal");
            terminal
                .draw(|frame| super::draw(frame.area(), frame, &app))
                .expect("render after resize");
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("JARVIS"));
            assert!(rendered.contains("Mesaj"));
            assert!(rendered.contains("EN_YENI_TUR"));
            let cursor = terminal.get_cursor_position().expect("composer cursor");
            assert!(cursor.x < area.width);
            assert!(cursor.y < area.height);
        }
    }

    #[test]
    fn keyboard_and_mouse_navigation_follow_and_leave_the_latest_turn() {
        let mut scroll = 0;
        assert!(apply_history_key_scroll(&mut scroll, KeyCode::Up));
        assert_eq!(scroll, 3);
        assert!(apply_history_mouse_scroll(
            &mut scroll,
            MouseEventKind::ScrollUp
        ));
        assert_eq!(scroll, 6);
        assert!(apply_history_key_scroll(&mut scroll, KeyCode::PageUp));
        assert_eq!(scroll, 14);
        assert!(apply_history_mouse_scroll(
            &mut scroll,
            MouseEventKind::ScrollDown
        ));
        assert_eq!(scroll, 11);
        assert!(apply_history_key_scroll(&mut scroll, KeyCode::End));
        assert_eq!(scroll, 0);
        assert!(apply_history_key_scroll(&mut scroll, KeyCode::Home));
        assert_eq!(scroll, u16::MAX);
        assert!(!apply_history_key_scroll(&mut scroll, KeyCode::Char('x')));
        return_to_latest(&mut scroll);
        assert_eq!(scroll, 0);
    }

    #[test]
    fn desktop_launcher_uses_a_sibling_binary_instead_of_the_working_directory() {
        let executable = std::path::Path::new("/opt/jarvis/bin/jarvis");
        assert_eq!(
            native_desktop_binary_path(executable),
            std::path::PathBuf::from("/opt/jarvis/bin/jarvis-desktop")
        );
    }

    #[test]
    fn middle_mouse_is_reserved_for_wayland_primary_selection_paste() {
        assert!(is_primary_selection_paste(MouseEventKind::Down(
            MouseButton::Middle
        )));
        assert!(!is_primary_selection_paste(MouseEventKind::ScrollDown));
        assert!(!is_primary_selection_paste(MouseEventKind::Down(
            MouseButton::Left
        )));
    }

    #[test]
    fn profile_shows_unset_fields_until_a_set_and_approve_round_trip_saves_one() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/profile".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Ad: ayarlanmamış"));

        app.input = "/profile set ad = Mehmet".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.pending_memory.is_some());
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Profil teklifi"));

        app.input = "/remember approve".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.pending_memory.is_none());
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Bellek kaydedildi"));

        app.input = "/profile".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("Ad: Mehmet"));
    }

    #[test]
    fn profile_set_rejects_an_unknown_field_and_an_invalid_value_without_arming_anything() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/profile set favori_renk = teal".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.pending_memory.is_none());
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Bilinmeyen profil alanı"));

        app.input = "/profile set ad =    ".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.pending_memory.is_none());
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Profil teklifi geçersiz"));
    }

    #[test]
    fn profile_delete_removes_only_the_named_field() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        for command in ["/profile set ad = Mehmet", "/remember approve"] {
            app.input = command.into();
            submit(&mut app, &runtime, &provider, &vision, &sender);
        }
        for command in ["/profile set dil = tr", "/remember approve"] {
            app.input = command.into();
            submit(&mut app, &runtime, &provider, &vision, &sender);
        }

        app.input = "/profile delete ad".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("Ad silindi"));

        app.input = "/profile".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let shown = &app.messages.last().unwrap().content;
        assert!(shown.contains("Ad: ayarlanmamış"));
        assert!(shown.contains("Dil: tr"));
    }

    /// User-requested UX: "hafızana yaz" must save in a single step, no separate
    /// `/remember approve` — and saying the same fact again must *update*, not duplicate.
    #[test]
    fn natural_language_remember_saves_in_one_step_and_updates_on_repeat() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "hafızana yaz: benim adım Ali".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("Not aldım"));
        assert_eq!(
            runtime
                .lock()
                .unwrap()
                .profile_snapshot()
                .unwrap()
                .record_for(ProfileField::DisplayName)
                .unwrap()
                .value,
            "Ali"
        );

        app.input = "hafızanı güncelle: benim adım Mehmet".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let memories = runtime.lock().unwrap().list_memory().unwrap();
        assert_eq!(
            memories
                .iter()
                .filter(|record| record.key == "display_name")
                .count(),
            1,
            "a second natural-language remember on the same fact must update, not duplicate"
        );
        assert_eq!(
            runtime
                .lock()
                .unwrap()
                .profile_snapshot()
                .unwrap()
                .record_for(ProfileField::DisplayName)
                .unwrap()
                .value,
            "Mehmet"
        );
    }

    /// User-requested UX: "belleğinden ... sil" must delete in a single step — both the known
    /// profile-field phrasing and a free-form key.
    #[test]
    fn natural_language_forget_deletes_a_profile_field_and_a_free_form_key() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        for command in ["/profile set ad = Ali", "/remember approve"] {
            app.input = command.into();
            submit(&mut app, &runtime, &provider, &vision, &sender);
        }
        app.input = "hafızana yaz: favori_renk = turkuaz".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        app.input = "hafızandan isim bilgimi sil".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("sildim"));
        assert!(runtime
            .lock()
            .unwrap()
            .profile_snapshot()
            .unwrap()
            .record_for(ProfileField::DisplayName)
            .is_none());

        app.input = "belleğinden favori_renk sil".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("silindi"));
        assert!(runtime
            .lock()
            .unwrap()
            .list_memory()
            .unwrap()
            .iter()
            .all(|record| record.key != "favori_renk"));
    }

    /// A recognized trigger phrase with an unparseable payload must get a clear correction
    /// message, never silently fall through to normal chat or silently do nothing.
    #[test]
    fn natural_language_memory_trigger_with_unparseable_payload_is_reported() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "hafızana yaz: bugün hava çok güzel".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("anlayamadım"));

        app.input = "hafızandan sil".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("anlayamadım"));
    }

    /// A sentence that merely *mentions* a fact in passing, with no trigger phrase, must never
    /// be intercepted as a memory command — it has to reach ordinary conversation handling.
    #[test]
    fn sentence_without_a_trigger_phrase_is_never_treated_as_a_memory_command() {
        assert_eq!(
            parse_memory_intent("adım Ali, bu tarif bana uygun mu?"),
            None
        );
    }

    /// `/clear`'s new contract (2026-08-16, conversation history now persists to disk): it must
    /// call `Runtime::clear_chat_history` — a real reset, not only a cosmetic one — and say so.
    #[test]
    fn clear_command_resets_conversation_and_reports_a_real_reset() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        app.messages.push(Message {
            role: MessageRole::User,
            content: "merhaba".into(),
        });

        app.input = "/clear".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert_eq!(
            app.messages.len(),
            1,
            "only the confirmation system message should remain"
        );
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("diskteki kayıt silindi"));
    }

    /// F3 post-close "`/rag status`" (GPT önerisi 4+5/7): the TUI wiring must actually call
    /// `Runtime::rag_status` and show real counts, not a static placeholder.
    #[test]
    fn rag_status_command_reports_document_and_chunk_counts() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/index Cargo.toml".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        app.input = "/rag status".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let shown = &app.messages.last().unwrap().content;
        assert!(shown.contains("FTS-only"));
        assert!(shown.contains("1 belge"));
    }

    /// `/rag rebuild` must fail with a clear message when no embedding provider is attached —
    /// never silently pretend to do something in FTS-only mode.
    #[test]
    fn rag_rebuild_command_fails_clearly_without_an_embedding_provider() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/rag rebuild".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Yeniden inşa edilemedi"));
    }

    /// `/rag verify` must report a healthy index in plain FTS-only mode (no embedding provider
    /// means "eksik embedding" does not even apply).
    #[test]
    fn rag_verify_command_reports_healthy_in_fts_only_mode() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/index Cargo.toml".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        app.input = "/rag verify".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("sağlıklı"));
    }

    /// F3 post-close "retrieval öncesi permission/sensitivity filtresi" (GPT önerisi 1/7): the
    /// TUI wiring accepts an optional trailing sensitivity word for both `/index` and
    /// `/index-folder`, without breaking the ordinary (no sensitivity word) case.
    #[test]
    fn index_commands_accept_an_optional_trailing_sensitivity_word() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/index Cargo.toml sensitive".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("indekslendi"));

        app.input = "/index-folder docs/adr sensitive".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("dosya indekslendi"));

        let status = runtime.lock().unwrap().rag_status().unwrap();
        assert!(
            status.document_count >= 2,
            "both the single-file and folder index must have actually indexed something"
        );
    }

    #[test]
    fn profile_reset_clears_every_populated_field_but_leaves_free_form_memory_alone() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        for command in [
            "/profile set ad = Mehmet",
            "/remember approve",
            "/profile set dil = tr",
            "/remember approve",
            "/remember favori_renk = teal",
            "/remember approve",
        ] {
            app.input = command.into();
            submit(&mut app, &runtime, &provider, &vision, &sender);
        }

        app.input = "/profile reset".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("2 profil alanı silindi"));

        app.input = "/profile".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let shown = &app.messages.last().unwrap().content;
        assert!(shown.contains("Ad: ayarlanmamış"));
        assert!(shown.contains("Dil: ayarlanmamış"));

        // Profil dışı serbest anahtar /profile reset'ten etkilenmemeli, /memory'de kalmalı.
        app.input = "/memory".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("favori_renk = teal"));
    }

    #[test]
    fn profile_export_writes_a_manifest_with_only_known_fields() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        for command in ["/profile set ad = Mehmet", "/remember approve"] {
            app.input = command.into();
            submit(&mut app, &runtime, &provider, &vision, &sender);
        }

        let path = std::env::temp_dir().join(format!(
            "jarvis-profile-export-test-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        app.input = format!("/profile export {}", path.display());
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Profil dışa aktarıldı"));

        let written = std::fs::read_to_string(&path).expect("export file exists");
        assert!(written.contains("jarvis-user-profile"));
        assert!(written.contains("\"value\": \"Mehmet\""));
        assert!(!written.contains("memory_id"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn remember_defaults_to_internal_and_permanent_until_the_user_changes_it() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/remember proje = jarvis".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let preview = &app.messages.last().unwrap().content;
        assert!(preview.contains("sensitivity: INTERNAL"));
        assert!(preview.contains("süre: kalıcı"));
    }

    /// Kullanıcının kuralı: "concurrent task'lar birbirinin context'ini kirletmesin" — yalnız
    /// izolasyon değil, önce gerçek bir yazma yolu da gerekiyordu. Bu, F3 sonrası kapatılan gerçek
    /// bir boşluktu: önceden `/remember` her zaman UserProfile'a yazıyordu, Project/Task/Session'a
    /// hiçbir üretim yolu yoktu.
    #[test]
    fn remember_writes_to_project_task_and_session_namespaces() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/remember proje mimari-karar = Rust kullanıyoruz".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        app.input = "/remember approve".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        app.input = "/remember görev task-abc123 karar = kutuphane-x".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        app.input = "/remember approve".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        // Session bir expiry olmadan hiç kaydedilemez; /remember ttl vermeden de akış tıkanmasın
        // diye makul bir varsayılan süre kendiliğinden atanmalı.
        app.input = "/remember oturum kisa-not = deger".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        app.input = "/remember approve".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Bellek kaydedildi"));

        let records = runtime.lock().unwrap().list_memory().unwrap();
        let project_record = records
            .iter()
            .find(|record| record.namespace == MemoryNamespace::Project)
            .expect("project record must exist");
        assert_eq!(project_record.key, "mimari-karar");
        assert_eq!(project_record.value, "Rust kullanıyoruz");

        let task_record = records
            .iter()
            .find(|record| record.namespace == MemoryNamespace::Task)
            .expect("task record must exist");
        assert_eq!(task_record.key, "karar");
        assert_eq!(task_record.scope_id.as_deref(), Some("task-abc123"));

        let session_record = records
            .iter()
            .find(|record| record.namespace == MemoryNamespace::Session)
            .expect("session record must exist");
        assert!(
            session_record.expires_at.is_some(),
            "Session must get an automatic default expiry when the user does not set one"
        );
    }

    /// Saf ayrıştırma mantığı, TUI/Runtime olmadan: belirsizlik her zaman güvenli tarafa
    /// (UserProfile, orijinal metin bozulmadan) düşmeli.
    #[test]
    fn parse_remember_namespace_prefix_disambiguates_a_real_literal_key() {
        assert_eq!(
            parse_remember_namespace_prefix("proje mimari-karar = Rust kullanıyoruz"),
            (
                MemoryNamespace::Project,
                None,
                "mimari-karar = Rust kullanıyoruz".to_string()
            )
        );
        // "proje" burada gerçek bir anahtar adı — namespace seçimi gibi görünse de arkasında
        // gerçek bir "anahtar = değer" olmadığı için (boş anahtar) eski davranışa düşmeli.
        assert_eq!(
            parse_remember_namespace_prefix("proje = jarvis"),
            (
                MemoryNamespace::UserProfile,
                None,
                "proje = jarvis".to_string()
            )
        );
        assert_eq!(
            parse_remember_namespace_prefix("favori_renk = turkuaz"),
            (
                MemoryNamespace::UserProfile,
                None,
                "favori_renk = turkuaz".to_string()
            )
        );
    }

    #[test]
    fn parse_remember_namespace_prefix_consumes_a_task_id_only_for_task_namespace() {
        assert_eq!(
            parse_remember_namespace_prefix("görev task-abc123 karar = kutuphane-x"),
            (
                MemoryNamespace::Task,
                Some("task-abc123".to_string()),
                "karar = kutuphane-x".to_string()
            )
        );
        // Görev kelimesinden sonra hiçbir şey yoksa (ne task-id ne anahtar), boş anahtara düşer
        // — bu da yine güvenli UserProfile geri dönüşünü tetikler.
        assert_eq!(
            parse_remember_namespace_prefix("görev"),
            (MemoryNamespace::UserProfile, None, "görev".to_string())
        );
    }

    /// Kullanıcının "secret'ları doğrudan hafızaya yazmıyoruz; sadece Secret Manager referansı
    /// tutuluyor" kuralı — TUI komutlarının uçtan uca kanıtı: gerçek değer `/memory` listesinde
    /// hiç görünmemeli, yalnız `/secret show` ile açıkça istenince görünmeli.
    #[test]
    fn secret_command_write_show_forget_and_list_round_trip() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/secret api_key = sk-abc123".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Sırrı kaydettim"));

        app.input = "/memory".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(!app.messages.last().unwrap().content.contains("sk-abc123"));

        app.input = "/secrets".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("api_key"));
        assert!(!app.messages.last().unwrap().content.contains("sk-abc123"));

        app.input = "/secret show api_key".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("sk-abc123"));

        app.input = "/secret forget api_key".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("silindi"));

        app.input = "/secret show api_key".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("kayıtlı bir sır yok"));
    }

    /// Doğal dil tetikleyicisi de aynı Secret Manager yoluna gitmeli — tek adımda, gerçek değer
    /// yine sıradan belleğe hiç yazılmadan.
    #[test]
    fn natural_language_secret_trigger_uses_the_secret_manager_not_ordinary_memory() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "hafızana gizli kaydet: api_key = sk-xyz789".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Sırrı kaydettim"));

        let records = runtime.lock().unwrap().list_memory().unwrap();
        assert!(!records
            .iter()
            .any(|record| record.value.contains("sk-xyz789")));
        assert_eq!(
            runtime.lock().unwrap().reveal_secret("api_key").unwrap(),
            Some("sk-xyz789".to_string())
        );
    }

    #[test]
    fn remember_sensitivity_and_ttl_change_the_pending_proposal_before_approval() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/remember proje = jarvis".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        app.input = "/remember sensitivity sensitive".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("sensitivity: SENSITIVE"));

        app.input = "/remember ttl 24".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("saat sonra silinir"));

        app.input = "/remember approve".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        let saved = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .list_memory()
            .expect("memory list");
        let record = saved
            .iter()
            .find(|record| record.key == "proje")
            .expect("saved record");
        assert_eq!(record.sensitivity.as_str(), "SENSITIVE");
        assert!(record.expires_at.is_some());
    }

    #[test]
    fn remember_ttl_none_reverts_to_permanent_and_invalid_sensitivity_is_rejected() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/remember proje = jarvis".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        app.input = "/remember ttl 24".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        app.input = "/remember ttl none".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("süre: kalıcı"));

        app.input = "/remember sensitivity gizli-degil".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Geçersiz sensitivity"));
        // The invalid attempt must not have discarded the still-pending proposal.
        assert!(app.pending_memory.is_some());
    }

    #[test]
    fn remember_sensitivity_or_ttl_without_a_pending_proposal_is_a_clear_no_op() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/remember sensitivity public".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Önce /remember anahtar"));
        assert!(app.pending_memory.is_none());
    }

    #[test]
    fn remember_model_context_toggle_is_actually_respected_at_retrieval() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/remember proje = jarvis".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        app.input = "/remember model-context hayır".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("model context: hayır"));
        app.input = "/remember approve".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        let saved = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .list_memory()
            .expect("memory list");
        let record = saved
            .iter()
            .find(|record| record.key == "proje")
            .expect("saved record");
        assert!(!record.include_in_model_context);
    }

    #[test]
    fn forget_namespace_deletes_only_that_namespace_and_rejects_unknown_words() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        for command in ["/remember ad = Mehmet", "/remember approve"] {
            app.input = command.into();
            submit(&mut app, &runtime, &provider, &vision, &sender);
        }

        app.input = "/forget namespace bilinmeyen-namespace".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Bilinmeyen namespace"));

        app.input = "/forget namespace profil".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("USER_PROFILE namespace'inden 1 kayıt silindi"));

        let remaining = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .list_memory()
            .expect("memory list");
        assert!(remaining.is_empty());
    }

    fn fixture_citation(path: &str, ordinal: usize, content: &str) -> WorkspaceCitation {
        WorkspaceCitation {
            document_id: "document-test".into(),
            chunk_id: format!("chunk-{path}-{ordinal}"),
            canonical_path: path.into(),
            content_sha256: "sha256-test".into(),
            chunk_ordinal: ordinal,
            content: content.into(),
        }
    }

    /// F3 "Citation UX: ... kaynağı aç davranışı": `/source <n>` must print the *full*
    /// (untruncated) chunk content and path for the n'th citation behind the last reply.
    #[test]
    fn source_command_opens_the_full_citation_content_by_position() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        app.last_citations = vec![
            fixture_citation("a.md", 0, "birinci belgenin tam metni"),
            fixture_citation("b.md", 2, "ikinci belgenin tam metni"),
        ];

        app.input = "/source 2".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let shown = &app.messages.last().unwrap().content;
        assert!(shown.contains("b.md"));
        assert!(shown.contains("chunk-2"));
        assert!(shown.contains("ikinci belgenin tam metni"));
        assert!(!shown.contains("birinci belgenin tam metni"));
    }

    /// F3 "Citation UX": out-of-range, non-numeric and "no citations at all" inputs must each get
    /// a clear, distinct message — never a panic or a silent no-op.
    #[test]
    fn source_command_rejects_out_of_range_non_numeric_and_missing_citations() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/source 1".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("kaynağı yok"));

        app.last_citations = vec![fixture_citation("a.md", 0, "tek belge")];
        app.input = "/source 5".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Geçersiz kaynak numarası"));

        app.input = "/source abc".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("Kullanım"));
    }

    #[test]
    fn memory_export_then_import_round_trips_through_the_tui_commands() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        for command in ["/remember ad = Mehmet", "/remember approve"] {
            app.input = command.into();
            submit(&mut app, &runtime, &provider, &vision, &sender);
        }

        let path = std::env::temp_dir().join(format!(
            "jarvis-memory-export-test-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        app.input = format!("/memory export {}", path.display());
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("Tüm bellek dışa aktarıldı"));

        // Start from an empty store so the import is what actually brings the record back.
        app.input = "/forget namespace profil".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .list_memory()
            .expect("memory list")
            .is_empty());

        app.input = format!("/memory import {}", path.display());
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("1/1 kayıt içe aktarıldı"));

        let restored = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .list_memory()
            .expect("memory list");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].key, "ad");
        assert_eq!(restored[0].value, "Mehmet");

        let _ = std::fs::remove_file(&path);
    }
}
