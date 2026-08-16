use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
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
    analyze_repository, approve_patch, attachment_receipt_manifest, default_profile_files_dir,
    draft_coding_plan_with_provider, draft_patch_with_provider, ensure_profile_files_exist,
    format_sources_block, inspect_local_attachment, memory_export, memory_import, new_cancel_flag,
    parse_data_sensitivity, parse_memory_intent, parse_memory_namespace, preview_workspace_index,
    profile_manifest, propose_memory, propose_memory_with_trust_and_scope, propose_profile_field,
    propose_unrecognized_remember_intent_with_provider, AttachmentReceipt, AttachmentRef,
    CancelFlag, CodingPlan, DataSensitivity, InputType, LlamaEmbeddingProvider,
    LlamaServerProvider, LlamaVisionServerProvider, MemoryIntent, MemoryNamespace, MemoryProposal,
    OpenMeteoWeatherProvider, PatchProposal, ProfileField, Request, Runtime, SqliteStore,
    TaskState, TrustLevel, VisionProvider, WorkspaceCitation,
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
    /// 16 Ağustos 2026: model-destekli doğal dil bellek yedek yolunun sonucu — `Some` olduğunda
    /// `content` zaten önizleme metnidir ve normal `handle_with_provider_and_vision` hiç
    /// çağrılmamıştır; alıcı taraf bunu `app.pending_memory`'ye koyup normal onay akışını
    /// başlatmalı, asla doğrudan yazmamalı.
    memory_proposal: Option<MemoryProposal>,
    /// F4 "Coding plan UX" ← → "Patch generator" köprüsü: `/plan` başarıyla bir `CodingPlan`
    /// ürettiğinde dolduruluyor. `app.pending_coding_plan`'a konup `/patch`'in hangi plana göre
    /// çalışacağını belirliyor.
    coding_plan: Option<CodingPlan>,
    /// F4 "Patch preview/review": `/patch` başarıyla bir taslak ürettiğinde dolduruluyor.
    /// `app.pending_patch`'e konup `/approve-patch`/`/reject-patch`'in üzerinde çalışacağı teklif
    /// oluyor — tıpkı `pending_memory`'nin `/remember approve|reject` için yaptığı gibi.
    patch_proposal: Option<(CodingPlan, PatchProposal)>,
}

struct TuiNotification {
    title: &'static str,
    content: String,
}

struct App {
    messages: Vec<Message>,
    input: String,
    /// TUI usability fix (2026-08-16): char-index (not byte-index) position within `input` where
    /// typing/Backspace/Delete/paste act. Previously `input` was append-only (no cursor concept
    /// at all), so Left/Right arrow keys did nothing and Ctrl+Backspace could only ever delete
    /// from the end. Always kept in `0..=input.chars().count()`.
    input_cursor: usize,
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
    /// F4: en son `/plan`'ın ürettiği plan — `/patch`'in üzerinde çalıştığı taban.
    pending_coding_plan: Option<CodingPlan>,
    /// F4 "Patch preview/review": en son `/patch`'in ürettiği, henüz onaylanmamış teklif.
    pending_patch: Option<(CodingPlan, PatchProposal)>,
    /// F4 "Patch preview/review": kullanıcının `/patch-note` ile eklediği serbest metin —
    /// hiçbir doğrulamayı etkilemiyor, yalnız onay öncesi gösterilip son onay mesajına ekleniyor.
    pending_patch_note: Option<String>,
    /// F4 "Gerçek cancellation": bir arka plan test/komut çalışırken dolu — `/cancel` bunu
    /// `true`'ya çeviriyor. `Runtime::cancel`'dan (bir task başlamadan önce iptali) tamamen ayrı:
    /// bu, hâlâ çalışan izole bir süreci ortasında durduruyor.
    active_cancel: Option<CancelFlag>,
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
            input_cursor: 0,
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
            pending_coding_plan: None,
            pending_patch: None,
            pending_patch_note: None,
            active_cancel: None,
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
        app.push_system(
            "Local model hazır; GPU'ya (Vulkan, 28/36 katman) offload edilmiş, kalanı CPU/RAM'de.",
        );
        app.status = "Model hazır • GPU offload (Vulkan, 28/36 katman)".into();
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
    attach_profile_files_dir(&mut runtime);
    attach_weather_provider(&mut runtime);
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
        return "Model hazır • GPU offload (Vulkan, 28/36 katman)".into();
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

/// Best-effort: creates (if missing, via `ensure_profile_files_exist`'s own never-overwrite
/// contract) and attaches the user-editable profile files directory (`about_user.md`/
/// `about_jarvis.md`, 16 Ağustos 2026) so they are re-read into every conversation turn. Never
/// blocks startup — if no config home can be resolved, `Runtime` simply has no profile files
/// directory, exactly as before this feature existed.
fn attach_profile_files_dir(runtime: &mut Runtime) {
    let Some(dir) = default_profile_files_dir() else {
        return;
    };
    ensure_profile_files_exist(&dir);
    runtime.set_profile_files_dir(Some(dir));
}

/// Attaches the Open-Meteo weather provider (İstanbul/Ümraniye, kullanıcı onayıyla 16 Ağustos
/// 2026 seçildi — ücretsiz, API anahtarsız) so `Runtime::startup_briefing` can include today's
/// weather. This is JARVIS's only network-dependent feature and is not routed through the
/// governed capability pipeline — the model can never invoke it. Unlike
/// `attach_embedding_provider_if_reachable`, reachability is not probed here: a failed fetch is
/// handled gracefully by `startup_briefing` itself (the weather line is simply omitted).
fn attach_weather_provider(runtime: &mut Runtime) {
    runtime.set_weather_provider(Some(
        Box::new(OpenMeteoWeatherProvider::istanbul_umraniye()),
    ));
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
    app.push_system(
        runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .startup_briefing(),
    );
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
                if let Some(sources_block) = format_sources_block(&reply.sources) {
                    message.content.push_str(&sources_block);
                }
            }
            app.last_citations = reply.citations;
            app.status = reply.status;
            app.pending = false;
            // F4 "Gerçek cancellation": her yanıt tek bir arka plan işinin sonucu (app.pending
            // yeni bir gönderiyi zaten engelliyor) — aktif bir CancelFlag varsa bu iş bitmiş
            // demektir, artık iptal edilecek bir şey kalmadı.
            app.active_cancel = None;
            app.record_attachment_receipts(reply.attachment_receipts);
            if let Some(proposal) = reply.memory_proposal {
                app.pending_memory = Some(proposal);
            }
            if let Some(plan) = reply.coding_plan {
                app.pending_patch = None; // yeni plan eskisini geçersiz kılar
                app.pending_coding_plan = Some(plan);
            }
            if let Some(patch) = reply.patch_proposal {
                app.pending_patch = Some(patch);
            }
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
            Event::Paste(pasted) => {
                insert_pasted_text_at_cursor(&mut app.input, &mut app.input_cursor, &pasted);
                app.status =
                    "Panodaki metin taslağa eklendi • Enter gönder • Ctrl+Backspace kelime sil"
                        .into();
                continue;
            }
            Event::Mouse(mouse) => {
                if is_primary_selection_paste(mouse.kind) {
                    match primary_selection_text() {
                        Ok(pasted) if !pasted.is_empty() => {
                            insert_pasted_text_at_cursor(
                                &mut app.input,
                                &mut app.input_cursor,
                                &pasted,
                            );
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
        if is_clipboard_paste_shortcut(key) {
            match clipboard_text() {
                Ok(pasted) if !pasted.is_empty() => {
                    insert_pasted_text_at_cursor(&mut app.input, &mut app.input_cursor, &pasted);
                    app.status = "Panodaki metin taslağa eklendi • Enter gönder".into();
                }
                Ok(_) => app.status = "Panoda metin yok.".into(),
                Err(error) => app.status = format!("Panodan yapıştırılamadı: {error}"),
            }
            continue;
        }
        if is_delete_previous_word_shortcut(key) {
            delete_previous_word(&mut app.input, &mut app.input_cursor);
            continue;
        }
        // Terminal/readline-style shortcuts (2026-08-16) — the same bindings a shell, Claude
        // Code, or Codex's own terminal session already uses, so a user's muscle memory carries
        // straight over. Checked before the plain-char `match` below, since e.g. Ctrl+A must
        // never fall through to "insert the letter a".
        if is_move_to_start_shortcut(key) {
            move_cursor_to_start(&mut app.input_cursor);
            continue;
        }
        if is_move_to_end_shortcut(key) {
            move_cursor_to_end(&app.input, &mut app.input_cursor);
            continue;
        }
        if is_kill_to_end_shortcut(key) {
            kill_to_end_from_cursor(&mut app.input, &mut app.input_cursor);
            continue;
        }
        if is_kill_to_start_shortcut(key) {
            kill_to_start_from_cursor(&mut app.input, &mut app.input_cursor);
            continue;
        }
        if is_forward_delete_shortcut(key) {
            delete_forward_at_cursor(&mut app.input, &mut app.input_cursor);
            continue;
        }
        if is_insert_newline_shortcut(key) {
            insert_char_at_cursor(&mut app.input, &mut app.input_cursor, '\n');
            continue;
        }
        if should_clear_draft(key) {
            app.input.clear();
            app.input_cursor = 0;
            app.status = "Taslak temizlendi.".into();
            continue;
        }
        match key.code {
            KeyCode::Enter if app.pending => {
                app.status = "JARVIS önceki isteğe yanıt üretiyor; taslağını yazmaya devam edebilirsin, yanıt bitince Enter'a bas.".into();
            }
            KeyCode::Enter => submit(&mut app, &runtime, &provider, &vision, &sender),
            KeyCode::Backspace => backspace_at_cursor(&mut app.input, &mut app.input_cursor),
            KeyCode::Delete => delete_forward_at_cursor(&mut app.input, &mut app.input_cursor),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                move_cursor_word_left(&app.input, &mut app.input_cursor)
            }
            KeyCode::Left => move_cursor_left(&app.input, &mut app.input_cursor),
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                move_cursor_word_right(&app.input, &mut app.input_cursor)
            }
            KeyCode::Right => move_cursor_right(&app.input, &mut app.input_cursor),
            KeyCode::Char(character) => {
                insert_char_at_cursor(&mut app.input, &mut app.input_cursor, character)
            }
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

/// Ctrl+A — jump to the start of the draft. Distinct from the plain `Home` key (message-history
/// scroll, `apply_history_key_scroll`) so neither shortcut loses its existing meaning.
fn is_move_to_start_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('a' | 'A'))
}

/// Ctrl+E — jump to the end of the draft (the `End` key equivalent, kept off `End` itself for
/// the same reason as `is_move_to_start_shortcut`).
fn is_move_to_end_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('e' | 'E'))
}

/// Ctrl+K — readline's "kill to end of line".
fn is_kill_to_end_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('k' | 'K'))
}

/// Ctrl+U — readline's *real* meaning, "kill to start of line" (from the cursor backward), not
/// "clear the whole draft" (that's `Esc`, `should_clear_draft`) — this used to be bound to the
/// clear-everything behavior; changed to match actual shell/terminal muscle memory.
fn is_kill_to_start_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('u' | 'U'))
}

/// Ctrl+D as an alternate forward-delete, alongside the plain `Delete` key (`KeyCode::Delete` in
/// the main match). Deliberately does **not** implement the other common terminal meaning of
/// Ctrl+D on an empty line (send EOF, often exiting the shell) — an unexpected app-exit shortcut
/// in a chat composer would be a real, surprising way to lose an in-progress draft; `Ctrl+C`
/// already covers intentional exit.
fn is_forward_delete_shortcut(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('d' | 'D'))
}

/// Alt+Enter (and Shift+Enter, wherever the terminal actually reports the distinction — like
/// Ctrl+Backspace earlier, some terminals can't tell Shift+Enter apart from plain Enter without
/// an enhanced keyboard protocol; Alt+Enter is the more universally reliable one) inserts a
/// literal newline instead of submitting — the same "Enter sends, Shift/Alt+Enter for a new line"
/// convention Claude Code's own terminal session already uses.
fn is_insert_newline_shortcut(key: KeyEvent) -> bool {
    key.code == KeyCode::Enter
        && (key.modifiers.contains(KeyModifiers::ALT)
            || key.modifiers.contains(KeyModifiers::SHIFT))
}

fn should_clear_draft(key: KeyEvent) -> bool {
    key.code == KeyCode::Esc
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

/// Byte offset of the `char_index`-th character in `s` (char-count based, so it stays correct
/// for multi-byte Turkish letters like ı/ş/ğ/ü/ö/ç). Clamped to `s.len()` when `char_index` is at
/// or past the end — the natural "insert/delete at the end" position.
fn char_index_to_byte_index(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map_or(s.len(), |(index, _)| index)
}

/// Inserts `pasted` (with the same newline→space collapsing `append_pasted_text` always did) at
/// `*cursor` instead of always at the end, then advances `*cursor` past the inserted text. When
/// `*cursor` is already at the end (the common "just typed, then pasted" case) this behaves
/// exactly like the old end-only `append_pasted_text`.
fn insert_pasted_text_at_cursor(input: &mut String, cursor: &mut usize, pasted: &str) {
    let byte_index = char_index_to_byte_index(input, *cursor);
    let mut previous_char = input[..byte_index].chars().last();
    let mut previous_was_line_break = false;
    let mut insertion = String::new();
    for character in pasted.chars() {
        if matches!(character, '\n' | '\r') {
            if !previous_char.is_some_and(char::is_whitespace) {
                insertion.push(' ');
                previous_char = Some(' ');
            }
            previous_was_line_break = true;
        } else {
            if previous_was_line_break && character.is_whitespace() {
                continue;
            }
            insertion.push(character);
            previous_char = Some(character);
            previous_was_line_break = false;
        }
    }
    let inserted_chars = insertion.chars().count();
    input.insert_str(byte_index, &insertion);
    *cursor += inserted_chars;
}

fn insert_char_at_cursor(input: &mut String, cursor: &mut usize, character: char) {
    let byte_index = char_index_to_byte_index(input, *cursor);
    input.insert(byte_index, character);
    *cursor += 1;
}

/// Deletes the character immediately before `*cursor` (a no-op at the start of the draft),
/// mirroring what every other text editor does with plain Backspace — unlike the old behavior,
/// which always deleted the *last* character of `input` regardless of where the cursor was.
fn backspace_at_cursor(input: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let end = char_index_to_byte_index(input, *cursor);
    let start = char_index_to_byte_index(input, *cursor - 1);
    input.replace_range(start..end, "");
    *cursor -= 1;
}

/// Deletes the character at `*cursor` (forward delete / the `Delete` key); the cursor itself does
/// not move, since the text after it shifts left into its place.
fn delete_forward_at_cursor(input: &mut String, cursor: &mut usize) {
    let char_count = input.chars().count();
    if *cursor >= char_count {
        return;
    }
    let start = char_index_to_byte_index(input, *cursor);
    let end = char_index_to_byte_index(input, *cursor + 1);
    input.replace_range(start..end, "");
}

fn move_cursor_left(input: &str, cursor: &mut usize) {
    let _ = input;
    *cursor = cursor.saturating_sub(1);
}

fn move_cursor_right(input: &str, cursor: &mut usize) {
    *cursor = (*cursor + 1).min(input.chars().count());
}

/// Char index of the start of the "word" immediately before `cursor` — skips any whitespace right
/// before the cursor first, then skips back over non-whitespace, exactly like a terminal's
/// readline Ctrl+Left. Used both by Ctrl+Left (cursor move) and by Ctrl+Backspace/Ctrl+W (delete).
fn word_start_before_cursor(input: &str, cursor: usize) -> usize {
    let chars: Vec<char> = input.chars().collect();
    let mut index = cursor.min(chars.len());
    while index > 0 && chars[index - 1].is_whitespace() {
        index -= 1;
    }
    while index > 0 && !chars[index - 1].is_whitespace() {
        index -= 1;
    }
    index
}

/// Char index just past the "word" immediately after `cursor` — the Ctrl+Right mirror of
/// `word_start_before_cursor`.
fn word_end_after_cursor(input: &str, cursor: usize) -> usize {
    let chars: Vec<char> = input.chars().collect();
    let mut index = cursor.min(chars.len());
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    while index < chars.len() && !chars[index].is_whitespace() {
        index += 1;
    }
    index
}

fn move_cursor_word_left(input: &str, cursor: &mut usize) {
    *cursor = word_start_before_cursor(input, *cursor);
}

fn move_cursor_word_right(input: &str, cursor: &mut usize) {
    *cursor = word_end_after_cursor(input, *cursor);
}

/// Terminal/readline-style shortcuts (2026-08-16): Ctrl+A, the same as every shell/Claude Code/
/// Codex terminal session — jump to the very start of the draft (distinct from the plain `Home`
/// key, which scrolls the *message history*, not the composer).
fn move_cursor_to_start(cursor: &mut usize) {
    *cursor = 0;
}

/// Ctrl+E mirror of `move_cursor_to_start` — jump to the very end of the draft.
fn move_cursor_to_end(input: &str, cursor: &mut usize) {
    *cursor = input.chars().count();
}

/// Ctrl+K ("kill to end of line" in every readline-based shell) — deletes from the cursor to the
/// end of the draft; the cursor itself does not move.
fn kill_to_end_from_cursor(input: &mut String, cursor: &mut usize) {
    let start = char_index_to_byte_index(input, *cursor);
    input.truncate(start);
}

/// Ctrl+U's *real* readline meaning ("kill to start of line") — not "clear the whole draft" (that
/// stays on `Esc`, unchanged). Deletes from the start of the draft up to the cursor; the cursor
/// moves to the new start (0).
fn kill_to_start_from_cursor(input: &mut String, cursor: &mut usize) {
    let end = char_index_to_byte_index(input, *cursor);
    input.replace_range(0..end, "");
    *cursor = 0;
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

/// Deletes the word immediately before `*cursor` (Ctrl+Backspace / Ctrl+W) *and* the single run
/// of whitespace separating it from whatever comes before — the same "no orphan space left
/// behind" readline behavior the old end-only version had, generalized to work from any cursor
/// position instead of always the end of the whole draft.
fn delete_previous_word(input: &mut String, cursor: &mut usize) {
    let word_start = word_start_before_cursor(input, *cursor);
    let chars: Vec<char> = input.chars().collect();
    let mut deletion_start = word_start;
    while deletion_start > 0 && chars[deletion_start - 1].is_whitespace() {
        deletion_start -= 1;
    }
    let cursor_byte = char_index_to_byte_index(input, *cursor);
    let deletion_start_byte = char_index_to_byte_index(input, deletion_start);
    input.replace_range(deletion_start_byte..cursor_byte, "");
    *cursor = deletion_start;
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
    app.input_cursor = 0;
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
    if input == "/analyze" || input.starts_with("/analyze ") {
        let rest = input.strip_prefix("/analyze").unwrap_or("").trim();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = if rest.is_empty() {
            root
        } else {
            root.join(rest)
        };
        match analyze_repository(&target) {
            Ok(overview) => {
                let languages = if overview.detected_languages.is_empty() {
                    "tespit edilemedi".to_string()
                } else {
                    overview.detected_languages.join(", ")
                };
                let manifests = overview
                    .dependency_manifests
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let test_commands = overview.suggested_test_commands.join(" • ");
                let risks = if overview.risk_notes.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\nNotlar:\n{}",
                        overview
                            .risk_notes
                            .iter()
                            .map(|note| format!("  • {note}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                };
                app.push_system(format!(
                    "Repo analizi (salt-okunur, hiçbir dosyaya dokunmadı): {}\nDiller: {}\nManifest(ler): {}\nÖnerilen test komutu: {}\n{} dosya (~{} KiB){risks}",
                    overview.root.display(),
                    languages,
                    if manifests.is_empty() {
                        "yok".to_string()
                    } else {
                        manifests
                    },
                    if test_commands.is_empty() {
                        "önerilemedi".to_string()
                    } else {
                        test_commands
                    },
                    overview.file_count,
                    overview.total_bytes.div_ceil(1024),
                ));
            }
            Err(error) => app.push_system(format!("Analiz edilemedi: {error}")),
        }
        return;
    }
    if input == "/plan" || input.starts_with("/plan ") {
        let rest = input.strip_prefix("/plan").unwrap_or("").trim();
        if rest.is_empty() {
            app.push_system("Kullanım: /plan <değişiklik isteği>");
            return;
        }
        // F4 "Coding plan UX": modele bir istek verilip hangi dosyaların ilgili olduğunu ve bir
        // test planını önermesi isteniyor — hiçbir dosya açılmıyor/yazılmıyor, yalnız bir
        // `CodingPlan` üretiliyor. Model çağrısı gerektirdiği için (analiz kendisi hızlı/yerel
        // olsa da) genel sohbet worker'ıyla aynı arka plan iş parçacığı deseni kullanılıyor.
        let request_summary = rest.to_string();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let message_index = app.messages.len();
        app.messages.push(Message {
            role: MessageRole::Jarvis,
            content: "Plan hazırlanıyor (salt-okunur, hiçbir dosyaya dokunulmuyor)…".into(),
        });
        app.pending = true;
        app.status = "Coding plan taslağı hazırlanıyor…".into();
        let provider = provider.clone();
        let sender = sender.clone();
        std::thread::spawn(move || {
            let (content, coding_plan) = match analyze_repository(&root) {
                Ok(overview) => {
                    match draft_coding_plan_with_provider(&overview, &request_summary, &provider) {
                        Ok(plan) => {
                            let files = if plan.affected_files.is_empty() {
                                "  (hiçbiri — model isteği belirli dosyalarla ilişkilendiremedi; isteği daha somut yaz)".to_string()
                            } else {
                                plan.affected_files
                                    .iter()
                                    .map(|path| format!("  • {}", path.display()))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            };
                            let tests = if plan.test_plan.is_empty() {
                                "  (önerilemedi)".to_string()
                            } else {
                                plan.test_plan
                                    .iter()
                                    .map(|test| format!("  • {test}"))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            };
                            let risks = plan
                                .risk_notes
                                .iter()
                                .map(|note| format!("  • {note}"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            let assumptions_block = if plan.assumptions.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    "\nVarsayımlar:\n{}",
                                    plan.assumptions
                                        .iter()
                                        .map(|item| format!("  • {item}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                )
                            };
                            let questions_block = if plan.open_questions.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    "\nAçık sorular:\n{}",
                                    plan.open_questions
                                        .iter()
                                        .map(|item| format!("  • {item}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                )
                            };
                            let next_step = if plan.affected_files.is_empty() {
                                String::new()
                            } else {
                                "\nDevam etmek için: /patch (bu plana göre gerçek bir diff taslağı üretir, hiçbir dosyaya henüz dokunmaz)".to_string()
                            };
                            let content = format!(
                                "Coding plan (salt-okunur, hiçbir dosyaya dokunulmadı):\nİstek: {}\nEtkilenebilecek dosyalar:\n{files}\nTest planı:\n{tests}{assumptions_block}{questions_block}\nNotlar:\n{risks}{next_step}",
                                plan.request_summary,
                            );
                            (content, Some(plan))
                        }
                        Err(error) => (format!("Plan üretilemedi: {error}"), None),
                    }
                }
                Err(error) => (format!("Repo analiz edilemedi: {error}"), None),
            };
            let _ = sender.send(WorkerReply {
                message_index,
                content,
                status: "Plan hazır (salt-okunur, hiçbir onay gerektirmez).".into(),
                task_id: String::new(),
                approval_pending: false,
                notification: None,
                sources: vec![],
                citations: vec![],
                attachment_receipts: vec![],
                memory_proposal: None,
                coding_plan,
                patch_proposal: None,
            });
        });
        return;
    }
    if input == "/patch" {
        // F4 "Patch generator": model her etkilenen dosyanın tam yeni içeriğini üretiyor (diff
        // sözdizimi değil — küçük yerel modeller hunk satır numaralarında güvenilir değil), gerçek
        // diff makine tarafında (`git diff --no-index`) hesaplanıyor. Hiçbir dosyaya henüz
        // dokunulmuyor; yalnız bir öneri.
        let Some(plan) = app.pending_coding_plan.clone() else {
            app.push_system("Önce /plan <istek> ile bir plan oluştur.");
            return;
        };
        if plan.affected_files.is_empty() {
            app.push_system(
                "Bu planda etkilenen dosya yok; /patch üretilemez. Önce /plan ile daha somut bir istek dene.",
            );
            return;
        }
        let message_index = app.messages.len();
        app.messages.push(Message {
            role: MessageRole::Jarvis,
            content: "Patch taslağı hazırlanıyor (model her dosyanın tamamını yeniden yazıyor, henüz hiçbir şey diske yazılmadı)…".into(),
        });
        app.pending = true;
        app.status = "Patch taslağı hazırlanıyor…".into();
        let provider = provider.clone();
        let sender = sender.clone();
        std::thread::spawn(move || {
            let (content, patch_proposal) = match draft_patch_with_provider(&plan, &provider) {
                Ok(proposal) => {
                    const MAX_DIFF_PREVIEW_CHARS: usize = 4_000;
                    let diff_preview: String = proposal
                        .unified_diff
                        .chars()
                        .take(MAX_DIFF_PREVIEW_CHARS)
                        .collect();
                    let truncated_note = if proposal.unified_diff.chars().count()
                        > diff_preview.chars().count()
                    {
                        "\n... (önizleme kısaltıldı; onaylanırsa TAM diff uygulanır)".to_string()
                    } else {
                        String::new()
                    };
                    let files = proposal
                        .affected_files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let content = format!(
                        "Patch taslağı (henüz uygulanmadı):\nDosyalar: {files}\n\n{diff_preview}{truncated_note}\n\nOnaylamak için /approve-patch, vazgeçmek için /reject-patch"
                    );
                    (content, Some((plan.clone(), proposal)))
                }
                Err(error) => (format!("Patch üretilemedi: {error}"), None),
            };
            let _ = sender.send(WorkerReply {
                message_index,
                content,
                status: "Patch taslağı hazır • /approve-patch veya /reject-patch".into(),
                task_id: String::new(),
                approval_pending: false,
                notification: None,
                sources: vec![],
                citations: vec![],
                attachment_receipts: vec![],
                memory_proposal: None,
                coding_plan: None,
                patch_proposal,
            });
        });
        return;
    }
    if input == "/reject-patch" {
        app.pending_patch_note = None;
        if app.pending_patch.take().is_some() {
            app.push_system("Patch teklifi reddedildi; hiçbir dosya değişmedi.");
        } else {
            app.push_system("Reddedilecek bir patch teklifi yok.");
        }
        return;
    }
    if input == "/patch-note" || input.starts_with("/patch-note ") {
        // F4 "Patch preview/review": kullanıcı değişiklik notu — hiçbir doğrulamayı etkilemiyor,
        // yalnız onay öncesi gösterilip son onay mesajına ekleniyor.
        if app.pending_patch.is_none() {
            app.push_system("Not eklenecek bir patch teklifi yok. Önce /plan ve /patch çalıştır.");
            return;
        }
        let note = input.strip_prefix("/patch-note").unwrap_or("").trim();
        if note.is_empty() {
            app.pending_patch_note = None;
            app.push_system("Patch notu temizlendi.");
        } else {
            app.pending_patch_note = Some(note.to_string());
            app.push_system(format!("Patch notu kaydedildi: {note}"));
        }
        return;
    }
    if input == "/patch-files" {
        // F4 "Patch preview/review": "satır bazlı görünüm" — çok-dosyalı bir patch'in her
        // dosyasını kendi diff'iyle ayrı ayrı gösteriyor, `scope_patch_proposal_to_files`'ı tek
        // dosyalık bir alt küme için yeniden kullanarak (ikinci bir bölme mantığı icat etmeden).
        let Some((plan, proposal)) = app.pending_patch.as_ref() else {
            app.push_system("Gösterilecek bir patch teklifi yok. Önce /plan ve /patch çalıştır.");
            return;
        };
        let mut blocks = Vec::new();
        for file in &proposal.affected_files {
            match jarvis_core::scope_patch_proposal_to_files(
                plan,
                proposal,
                std::slice::from_ref(file),
            ) {
                Ok(scoped) => blocks.push(format!(
                    "=== {} ===\n{}",
                    file.display(),
                    scoped.unified_diff
                )),
                Err(error) => blocks.push(format!(
                    "=== {} ===\n(gösterilemedi: {error})",
                    file.display()
                )),
            }
        }
        app.push_system(format!(
            "Patch, dosya dosya (onaylamak için: /approve-patch [dosya1 dosya2 ...] — hiçbiri verilmezse tümü):\n{}",
            blocks.join("\n\n")
        ));
        return;
    }
    if input == "/abort" {
        // F4 "Gerçek cancellation": `Runtime::cancel`'dan (bir task başlamadan önce iptali) ayrı —
        // bu, hâlâ çalışan izole bir test/komut sürecini SIGTERM→grace period→SIGKILL ile durduruyor.
        match app.active_cancel.as_ref() {
            Some(cancel) => {
                cancel.store(true, Ordering::SeqCst);
                app.push_system(
                    "İptal isteği gönderildi; şu anki adım kısa sürede (SIGTERM, gerekirse SIGKILL) duracak.",
                );
            }
            None => app.push_system("İptal edilecek aktif bir arka plan işlemi yok."),
        }
        return;
    }
    if input == "/approve-patch" || input.starts_with("/approve-patch ") {
        // F4 "Patch apply transaction" + "Test/verifier runner": onay → izole uygula → izole
        // test çalıştır. Testler geçmezse (ya da /abort ile iptal edilirse) değişiklik otomatik
        // geri alınır — bu, önce yazıp hatada geri alan mevcut `apply_approved_patch` deseninin
        // aynısını test sonucuna da genişletiyor.
        let Some((plan, proposal)) = app.pending_patch.take() else {
            app.push_system("Onaylanacak bir patch teklifi yok. Önce /plan ve /patch çalıştır.");
            return;
        };
        let note = app.pending_patch_note.take();
        // F4 "Patch preview/review": seçilebilir dosya scope'u — kullanıcı çok-dosyalı bir
        // patch'in yalnız bir alt kümesini onaylayabiliyor. Dosya verilmezse (bare `/approve-patch`)
        // tüm patch onaylanıyor, eskisi gibi.
        let selected: Vec<PathBuf> = input
            .strip_prefix("/approve-patch")
            .unwrap_or("")
            .split_whitespace()
            .map(PathBuf::from)
            .collect();
        let proposal = if selected.is_empty() {
            proposal
        } else {
            match jarvis_core::scope_patch_proposal_to_files(&plan, &proposal, &selected) {
                Ok(scoped) => scoped,
                Err(error) => {
                    app.push_system(format!("Dosya seçimi geçersiz: {error}"));
                    return;
                }
            }
        };
        let approval = match approve_patch(&proposal, true) {
            Ok(approval) => approval,
            Err(error) => {
                app.push_system(format!("Patch onaylanamadı: {error}"));
                return;
            }
        };
        if plan.test_plan.is_empty() {
            // Test planı yoksa taban çizgisi/regresyon karşılaştırması anlamsız — doğrudan,
            // senkron uygulama (git apply hızlı, arka plan thread'i gerektirmiyor).
            let application = {
                let mut runtime_guard = runtime.lock().expect("JARVIS runtime lock poisoned");
                runtime_guard.apply_coding_patch(&plan, &proposal, &approval)
            };
            let note_line = note
                .as_ref()
                .map(|text| format!("Not: {text}\n"))
                .unwrap_or_default();
            match application {
                Ok(application) => {
                    let changed = application
                        .changed_files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    app.push_system(format!(
                        "{note_line}Patch uygulandı: {changed}\nDoğrulama kanıtı:\n{}\nBu plan için test komutu yok; değişiklik kalıcı.",
                        application.verifier_evidence.join("\n")
                    ));
                }
                Err(error) => app.push_system(format!("{note_line}Patch uygulanamadı: {error}")),
            }
            return;
        }
        // Test planı varsa taban çizgisi (patch ÖNCESİ) → uygula → test (patch SONRASI) → karşılaştır
        // zinciri tek bir arka plan thread'inde çalışıyor — taban çizgisi patch'ten önce
        // ölçülmeli, bu yüzden uygulama artık senkron adım değil.
        let cancel = new_cancel_flag();
        app.active_cancel = Some(cancel.clone());
        let message_index = app.messages.len();
        app.messages.push(Message {
            role: MessageRole::Jarvis,
            content: "Taban çizgisi ölçülüyor, ardından patch izole uygulanıp testler tekrar çalıştırılacak (iptal için /abort)…".into(),
        });
        app.pending = true;
        app.status = "Taban çizgisi + patch + testler çalıştırılıyor…".into();
        let runtime = Arc::clone(runtime);
        let sender = sender.clone();
        std::thread::spawn(move || {
            let note_line = note
                .as_ref()
                .map(|text| format!("Not: {text}\n"))
                .unwrap_or_default();
            let outcome = runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .apply_coding_patch_with_regression_check(
                    &plan,
                    &proposal,
                    &approval,
                    Some(&cancel),
                );
            let content = match outcome {
                Err(error) => format!("{note_line}Patch uygulanamadı: {error}"),
                Ok((outcome, finalize)) => {
                    let render = |report: &jarvis_core::TestRunReport| {
                        report
                            .ran
                            .iter()
                            .map(|run| {
                                format!(
                                    "  • {} → {}{}",
                                    run.command_line(),
                                    run.exit_code
                                        .map(|code| code.to_string())
                                        .unwrap_or_else(|| "?".into()),
                                    run.stopped
                                        .map(|reason| format!(" ({reason:?})"))
                                        .unwrap_or_default(),
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    let baseline_summary = render(&outcome.baseline);
                    let post_summary = render(&outcome.post_patch);
                    let changed = outcome
                        .changed_files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let outcome_text = match finalize {
                        Err(error) => format!(
                            "Testler tamamlandı ama geri alma sırasında sorun oldu: {error}\nTaban çizgisi:\n{baseline_summary}\nPatch sonrası:\n{post_summary}"
                        ),
                        Ok(()) if outcome.kept => {
                            let pre_existing_note = if outcome.baseline.ran.iter().any(|run| !run.succeeded()) {
                                "\n(Not: taban çizgisinde zaten başarısız olan komut(lar) vardı — bunlar bu patch'e karşı kullanılmadı.)"
                            } else {
                                ""
                            };
                            format!(
                                "Patch uygulandı ve kalıcı: {changed}\nDoğrulama kanıtı:\n{}\nTaban çizgisi:\n{baseline_summary}\nPatch sonrası:\n{post_summary}{pre_existing_note}",
                                outcome.verifier_evidence.join("\n")
                            )
                        }
                        Ok(()) if !outcome.regressions.is_empty() => format!(
                            "Gerçek regresyon tespit edildi, değişiklik otomatik geri alındı: {}\nTaban çizgisi:\n{baseline_summary}\nPatch sonrası:\n{post_summary}",
                            outcome.regressions.join(", ")
                        ),
                        Ok(()) => format!(
                            "Testler geçmedi veya iptal edildi — değişiklik otomatik geri alındı (dosyalar patch öncesi hâline döndü):\nTaban çizgisi:\n{baseline_summary}\nPatch sonrası:\n{post_summary}"
                        ),
                    };
                    format!("{note_line}{outcome_text}")
                }
            };
            let _ = sender.send(WorkerReply {
                message_index,
                content,
                status: "Test/doğrulama tamamlandı.".into(),
                task_id: String::new(),
                approval_pending: false,
                notification: None,
                sources: vec![],
                citations: vec![],
                attachment_receipts: vec![],
                memory_proposal: None,
                coding_plan: None,
                patch_proposal: None,
            });
        });
        return;
    }
    if let Some(rest) = input.strip_prefix("/note-append ") {
        // F4 "Yerel üretkenlik tool framework"'ün ikinci gerçek tool'u — F4'ün coding-patch
        // sandbox'ından tamamen ayrı, `note.create` ile aynı Policy → Task → Approval →
        // execute → Verifier zincirinden geçen basit bir "var olan dosyaya satır ekle" işlemi.
        // Model çağrısı gerektirmiyor — deterministik `classify()` üzerinden `Runtime::handle`.
        let Some((path, line)) = rest.split_once('|') else {
            app.push_system("Kullanım: /note-append <proje-içi-göreli-dosya> | <eklenecek satır>");
            return;
        };
        let (path, line) = (path.trim(), line.trim());
        if path.is_empty() || line.is_empty() {
            app.push_system("Kullanım: /note-append <proje-içi-göreli-dosya> | <eklenecek satır>");
            return;
        }
        let request = Request {
            schema_version: 1,
            request_id: format!(
                "tui-append-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock must be after UNIX epoch")
                    .as_nanos()
            ),
            input_type: InputType::Gui,
            content: format!("file.append_note: {path}|{line}"),
            attachments: vec![],
        };
        let (task, _, _) = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .handle(request);
        if task.state == TaskState::WaitingForUser {
            let preview = runtime
                .lock()
                .expect("JARVIS runtime lock poisoned")
                .preview_pending_action(&task.task_id)
                .unwrap_or_else(|| "(önizleme yok)".into());
            app.push_system(format!(
                "Onay bekliyor ({}):\n{preview}\nOnay: /approve • Vazgeç: /cancel",
                task.task_id
            ));
        } else {
            app.push_system(format!(
                "İstek beklenmeyen bir durumda tamamlandı: {:?}",
                task.state
            ));
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
            app.push_system("Kısayollar (terminal/Claude Code alışkanlıkları): Enter gönder • Alt+Enter veya Shift+Enter taslağa yeni satır ekler • Ctrl+V yapıştır • ←/→ imleç, Ctrl+←/→ kelime kelime • Ctrl+A/Ctrl+E taslağın başına/sonuna • Ctrl+Backspace veya Ctrl+W ya da Ctrl+K/Ctrl+U önceki/sonraki kısmı sil (Ctrl+K imleçten sona, Ctrl+U imleçten başa) • Ctrl+D ileri sil • Esc taslağın tamamını sil. Geçmiş: ↑/↓ veya PageUp/PageDown. Ek: /attach <PNG/JPEG/TXT/MD/PDF-yolu>, /attachments, /attachments clear, /attachment-history, /attachment-history remove <id>|clear, /attachment-export <dosya-yolu>. Belge ekleri metadata-only'dir; indeksleme için ayrı /index akışı kullanılır. Bellek: /remember [profil|proje|görev <task-id>|oturum|geçici] anahtar = değer (namespace verilmezse profil), /remember sensitivity <public|internal|sensitive>, /remember ttl <saat|none>, /remember model-context <evet|hayır>, /remember approve|reject, /memory, /forget <id>|all, /forget namespace <profil|proje|görev|oturum|geçici>, /memory export <dosya-yolu>, /memory import <dosya-yolu>. Sır: /secret anahtar = değer (Secret Manager'a gider, sıradan belleğe/modele hiç gitmez), /secret show <anahtar>, /secret forget <anahtar>, /secrets (yalnız anahtarları listeler). Profil: /profile, /profile set <ad|hitap|dil|rol> = <değer> (onay: /remember approve), /profile delete <alan>, /profile reset, /profile export <dosya-yolu>. RAG: /index <proje-içi-göreli-dosya> [public|internal|sensitive], /index-preview <proje-içi-göreli-klasör> [hariç-desen ...], /index-folder <proje-içi-göreli-klasör> [hariç-desen ...] [public|internal|sensitive], /source <numara> (son yanıtın kaynağının tamamını aç), /rag status, /rag rebuild, /rag verify. F4: /analyze [proje-içi-göreli-klasör] (salt-okunur repo analizi — dil/manifest/test komutu tespiti, hiçbir dosyaya dokunmaz; klasör verilmezse proje kökü), /plan <değişiklik isteği> (salt-okunur coding plan taslağı — model hangi dosyaların ilgili olduğunu ve bir test planını önerir, hiçbir dosyaya dokunmaz/yazmaz), /patch (en son plana göre modelden gerçek bir diff taslağı üretir — dosyaları tam yeniden yazdırıp gerçek diff'i git ile hesaplar, hâlâ hiçbir şey diske yazılmaz), /patch-files (patch'i dosya dosya, her birinin kendi diff'iyle gösterir), /patch-note <metin> (onay öncesi serbest bir not ekler, boş çağrılırsa temizler), /approve-patch [dosya1 dosya2 ...] (izole ortamda uygular, ardından plan'ın test komutlarını izole çalıştırır — testler geçmezse veya iptal edilirse değişiklik otomatik geri alınır; dosya adı verilirse patch'in yalnız o alt kümesi onaylanır, hiçbiri verilmezse tümü), /reject-patch (taslağı at, hiçbir şey değişmez), /abort (şu an çalışan izole test/komutu SIGTERM→SIGKILL ile durdurur), /note-append <proje-içi-göreli-dosya> | <satır> (var olan bir dosyaya kalıcı bir satır eklemek için onay ister — model çağrısı yok, doğrudan Policy/Approval/Verifier zincirinden geçer). Komutlar: /status, /approvals, /approve, /cancel, /clear, /quit, exit. `exit` modeli RAM'den çıkarır; /quit veya Ctrl+C yalnız arayüzü kapatır.");
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
                "Model server: {} • GPU offload (Vulkan, 28/36 katman) • RAG: {rag_mode} • {}",
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
        app.input_cursor = input.chars().count();
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
        // Model-assisted fallback for a natural-language memory request that no fixed trigger
        // phrase covers (16 Ağustos 2026, kullanıcı bulgusu: "aklında tut" gibi eş anlamlılar
        // listeye eklendi ama hepsini önceden tahmin etmek mümkün değil). Yalnız düz metin
        // turlarında denenir — bir ek varsa hiç çalışmaz, güvenilmeyen bağlam bu karara hiç
        // karışmaz. Ucuz bir ipucu eşleşmezse (`might_express_an_unrecognized_remember_intent`,
        // fonksiyonun kendi içinde) model hiç çağrılmaz; sıradan hiçbir mesaj ek maliyet ödemez.
        // Fixed-trigger yolunun aksine asla doğrudan yazmaz — yalnız bir öneri üretir, normal
        // önizleme/onay akışına (`pending_memory`) girer, tıpkı `/remember anahtar = değer` gibi.
        if attachments.is_empty() {
            if let Some(proposal) =
                propose_unrecognized_remember_intent_with_provider(&input, &provider)
            {
                let preview = pending_memory_preview(&proposal);
                let _ = sender.send(WorkerReply {
                    message_index,
                    content: preview,
                    status: "Önerilen bir bellek kaydı var • /remember approve|reject".into(),
                    task_id: String::new(),
                    approval_pending: false,
                    notification: None,
                    sources: vec![],
                    citations: vec![],
                    attachment_receipts: vec![],
                    memory_proposal: Some(proposal),
                    coding_plan: None,
                    patch_proposal: None,
                });
                return;
            }
        }
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
            memory_proposal: None,
            coding_plan: None,
            patch_proposal: None,
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
        .map(|approval| {
            // F4 "Yerel üretkenlik tool framework" — "ExplainBeforeExecute" artık gerçekten
            // uygulanıyor: kullanıcı onaylamadan önce tam olarak ne olacağını görüyor, yalnız
            // task/action ID değil.
            match runtime.preview_pending_action(&approval.task_id) {
                Some(preview) => {
                    format!("{} • {}\n  {preview}", approval.task_id, approval.action_id)
                }
                None => format!("{} • {}", approval.task_id, approval.action_id),
            }
        })
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

/// Keeps `cursor` (a char index into `input`) visible in a fixed-height input box, scrolling the
/// window only as far as needed to do so. Input is character-based so backspace and Turkish text
/// remain safe; it deliberately does not edit the read-only message history above it.
///
/// When the whole draft fits in `width * rows`, the window never scrolls (identical to the old
/// always-show-everything behavior). When it doesn't, the window slides just far enough that
/// `cursor` stays inside it — which reduces to "always show the tail" exactly when `cursor` is at
/// the end of the draft, matching the previous append-only behavior byte-for-byte.
/// Splits `input` on `'\n'` into one `Line` per logical line — the same pattern `history_lines`
/// already uses, letting Ratatui's own `Paragraph::wrap`/`line_count` do all within-line
/// character-wrapping and row-counting (2026-08-16: multi-line drafts, `is_insert_newline_
/// shortcut`), instead of a hand-rolled character grid that had no concept of an embedded
/// newline at all.
fn build_input_lines(input: &str) -> Vec<Line<'static>> {
    input
        .split('\n')
        .map(|line| Line::from(line.to_string()))
        .collect()
}

/// Rows a single *logical* line (no embedded `'\n'`) wraps into at `width` — an empty line still
/// takes exactly one row, matching how a blank line looks in any real text editor.
fn wrapped_rows(text: &str, width: u16) -> usize {
    let width = usize::from(width.max(1));
    text.chars().count().max(1).div_ceil(width)
}

/// The cursor's (row, column) among *all* visual rows the whole draft wraps into at `width`,
/// before any vertical windowing — `cursor` is the char index `App::input_cursor` tracks,
/// counting the `'\n'` characters that separate logical lines like any other character.
fn cursor_visual_position(input: &str, cursor: usize, width: u16) -> (usize, usize) {
    let width_usize = usize::from(width.max(1));
    let mut remaining = cursor;
    let mut visual_row = 0usize;
    for line in input.split('\n') {
        let line_len = line.chars().count();
        if remaining <= line_len {
            // Deliberately *not* clamped to the line's own last real cell: a cursor sitting
            // right after exactly filling a row (`remaining` a nonzero multiple of `width`) lands
            // one row below, column 0 — the same "wrapped to a fresh row, ready to keep typing"
            // spot any real text editor shows there, even though Ratatui's own wrap (`wrapped_
            // rows`) doesn't allocate that row until there is real content in it. `input_view`
            // accounts for this when sizing its scroll window.
            let row_in_line = remaining / width_usize;
            let column = remaining % width_usize;
            return (visual_row + row_in_line, column);
        }
        remaining -= line_len + 1; // the `'\n'` itself, consumed between this line and the next
        visual_row += wrapped_rows(line, width);
    }
    // `cursor` was past the end of every line — should not happen if callers keep it clamped to
    // `input.chars().count()`, but land at a sane spot rather than panic.
    (visual_row, 0)
}

/// Builds the composer's `Line`s plus everything `draw` needs to render and scroll them: the
/// cursor's (row, column) *within the visible window*, and the vertical `scroll` offset to hand
/// `Paragraph::scroll` — mirroring exactly how the history pane already scrolls
/// (`history_line_count` + `.scroll((scroll_position, 0))`), just anchored to "keep the cursor
/// visible" instead of "follow the newest message".
fn input_view(
    input: &str,
    cursor: usize,
    width: u16,
    rows: u16,
) -> (Vec<Line<'static>>, u16, u16, u16) {
    let rows_usize = usize::from(rows.max(1));
    let lines = build_input_lines(input);
    let total_rows = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1));
    let (cursor_row, cursor_column) = cursor_visual_position(input, cursor, width);
    // `cursor_row` can be one row past `total_rows` (see `cursor_visual_position`'s doc comment)
    // when the cursor sits right after exactly filling a row — the window still needs to be tall
    // enough to keep that "about to wrap" cursor visible even though Ratatui's own wrap hasn't
    // allocated a row there yet.
    let total_rows = total_rows.max(cursor_row + 1);

    let window_start_row = if total_rows <= rows_usize {
        0
    } else {
        cursor_row
            .saturating_sub(rows_usize.saturating_sub(1))
            .min(total_rows - rows_usize)
    };
    let cursor_row_in_window = cursor_row
        .saturating_sub(window_start_row)
        .min(rows_usize.saturating_sub(1));
    (
        lines,
        cursor_row_in_window as u16,
        cursor_column as u16,
        window_start_row as u16,
    )
}

/// Total visual rows the whole draft occupies at `width`, embedded `'\n'`s included — the single
/// source of truth `draw` uses to size the composer box, computed the same way as
/// `input_view`'s own `total_rows` so the two can never disagree.
fn draft_rows(input: &str, width: u16) -> u16 {
    Paragraph::new(build_input_lines(input))
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .min(u16::MAX as usize) as u16
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
        Span::raw(format!("  •  GPU: 28/36  •  EK: {}", app.attachments.len())),
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
    let (input_lines, cursor_row, cursor_column, input_scroll) =
        input_view(&app.input, app.input_cursor, input_width, input_rows);
    let input = Paragraph::new(input_lines)
        .block(Block::default().borders(Borders::ALL).title(input_title))
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false })
        .scroll((input_scroll, 0));
    frame.render_widget(input, layout[2]);
    let footer = Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, layout[3]);
    // Editing stays live while `pending` too (F4-adjacent TUI fix, 2026-08-16) — the draft cursor
    // is always real now, not just while idle.
    let cursor_x = (layout[2].x + 1 + cursor_column).min(layout[2].right().saturating_sub(2));
    let cursor_y = (layout[2].y + 1 + cursor_row).min(layout[2].bottom().saturating_sub(2));
    frame.set_cursor_position((cursor_x, cursor_y));
}

#[cfg(test)]
mod tests {
    use super::{
        apply_history_key_scroll, apply_history_mouse_scroll, backspace_at_cursor,
        build_input_lines, cursor_visual_position, delete_forward_at_cursor, delete_previous_word,
        draft_rows, history_line_count, history_lines, input_view, insert_char_at_cursor,
        insert_pasted_text_at_cursor, is_clipboard_paste_shortcut,
        is_delete_previous_word_shortcut, is_forward_delete_shortcut, is_insert_newline_shortcut,
        is_kill_to_end_shortcut, is_kill_to_start_shortcut, is_move_to_end_shortcut,
        is_move_to_start_shortcut, is_primary_selection_paste, kill_to_end_from_cursor,
        kill_to_start_from_cursor, move_cursor_left, move_cursor_right, move_cursor_to_end,
        move_cursor_to_start, move_cursor_word_left, move_cursor_word_right,
        native_desktop_binary_path, notification_arguments, notification_preview,
        parse_remember_namespace_prefix, return_to_latest, should_clear_draft,
        should_close_tui_for_key, submit, try_notify_desktop, tui_exit_action, tui_notification,
        App, Message, MessageRole, TuiExitAction, WorkerReply,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
    use jarvis_core::{
        parse_memory_intent, LlamaServerProvider, LlamaVisionServerProvider, MemoryNamespace,
        ProfileField, Runtime, SqliteStore, TaskState, WorkspaceCitation,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use std::path::PathBuf;
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

    /// `input_view` now returns the *whole* draft (never clipped) plus a `window_start_row` the
    /// caller hands to `Paragraph::scroll` — the same pattern `history_line_count` +
    /// `.scroll((scroll_position, 0))` already uses for the message pane, rather than the old
    /// hand-rolled character grid that had to splice in its own "…" ellipsis markers.
    #[test]
    fn a_cursor_past_the_visible_capacity_scrolls_the_window_down_to_it() {
        // "0123456789abcdef" (16 chars) at width=5, rows=2 → 10 visible cells; cursor at the very
        // end (16) cannot fit in a window starting at row 0, so the view must scroll.
        let (lines, cursor_row, _cursor_column, window_start_row) =
            input_view("0123456789abcdefghijklmno", 26, 5, 2);
        assert_eq!(
            lines.len(),
            1,
            "no embedded newline, still one logical line"
        );
        assert!(
            window_start_row > 0,
            "must scroll to keep the cursor visible"
        );
        assert!(
            (cursor_row as usize) < 2,
            "cursor row must land inside the 2-row window"
        );
    }

    #[test]
    fn cursor_advances_to_next_input_row() {
        let (_, row, column, window_start_row) = input_view("12345", 5, 5, 3);
        assert_eq!((row, column, window_start_row), (1, 0, 0));
    }

    /// TUI bug #4 (2026-08-16): the draft had no cursor concept at all — Left/Right did nothing.
    /// A cursor placed in the middle of a long draft still scrolls the window to keep it visible.
    #[test]
    fn a_cursor_in_the_middle_of_a_long_draft_scrolls_a_window_around_it() {
        let (_, cursor_row, _column, window_start_row) =
            input_view("0123456789abcdefghij", 10, 5, 2);
        assert!(window_start_row > 0, "past capacity, must have scrolled");
        assert!(
            (cursor_row as usize) < 2,
            "cursor row must land inside the 2-row window"
        );
    }

    #[test]
    fn a_cursor_at_the_very_start_shows_the_head_not_the_tail() {
        let (_, row, column, window_start_row) = input_view("0123456789abcdefghij", 0, 5, 2);
        assert_eq!(
            window_start_row, 0,
            "cursor at 0 must never scroll away from the head"
        );
        assert_eq!((row, column), (0, 0));
    }

    /// TUI usability fix (2026-08-16): Alt+Enter/Shift+Enter insert a literal newline
    /// (`is_insert_newline_shortcut`) instead of submitting — the draft must actually render each
    /// logical line on its own row, not as one long wrapped blob with an invisible control char.
    #[test]
    fn a_draft_with_an_embedded_newline_renders_as_two_separate_logical_lines() {
        let lines = build_input_lines("ilk satır\nikinci satır");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].to_string(), "ilk satır");
        assert_eq!(lines[1].to_string(), "ikinci satır");
    }

    #[test]
    fn cursor_visual_position_counts_the_newline_itself_as_one_character() {
        let input = "abc\ndefgh";
        // Right at the start of the second logical line (just past the '\n').
        let cursor = "abc\n".chars().count();
        let (row, column) = cursor_visual_position(input, cursor, 80);
        assert_eq!((row, column), (1, 0));
    }

    #[test]
    fn draft_rows_counts_every_logical_line_an_embedded_newline_creates() {
        assert_eq!(draft_rows("tek satır", 80), 1);
        assert_eq!(draft_rows("iki\nsatır", 80), 2);
        assert_eq!(draft_rows("üç\nayrı\nsatır", 80), 3);
    }

    #[test]
    fn cursor_movement_and_editing_respect_the_cursor_position_not_just_the_end() {
        // "aş cd": a(0) ş(1) ' '(2) c(3) d(4) — 5 chars, ş is a 2-byte UTF-8 char.
        let mut input = "aş cd".to_owned();
        let mut cursor = input.chars().count(); // 5, at the end

        move_cursor_left(&input, &mut cursor);
        move_cursor_left(&input, &mut cursor);
        assert_eq!(cursor, 3); // sitting right before 'c'

        // Backspace deletes *before* the cursor, not always the last character of the string.
        backspace_at_cursor(&mut input, &mut cursor); // deletes the space at index 2
        assert_eq!(input, "aşcd");
        assert_eq!(cursor, 2);

        // Delete removes the character *at* the cursor without moving it.
        delete_forward_at_cursor(&mut input, &mut cursor); // deletes 'c' at index 2
        assert_eq!(input, "aşd");
        assert_eq!(cursor, 2);

        move_cursor_right(&input, &mut cursor);
        assert_eq!(cursor, 3); // now at the end
        insert_char_at_cursor(&mut input, &mut cursor, 'z');
        assert_eq!(input, "aşdz");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn ctrl_left_and_ctrl_right_jump_by_word_like_a_shells_readline() {
        let input = "merhaba dünya güzel".to_owned();
        let mut cursor = input.chars().count();

        move_cursor_word_left(&input, &mut cursor);
        assert_eq!(input.chars().skip(cursor).collect::<String>(), "güzel");

        move_cursor_word_left(&input, &mut cursor);
        assert_eq!(
            input.chars().skip(cursor).collect::<String>(),
            "dünya güzel"
        );

        move_cursor_word_right(&input, &mut cursor);
        move_cursor_word_right(&input, &mut cursor);
        assert_eq!(cursor, input.chars().count());
    }

    #[test]
    fn pasting_in_the_middle_of_a_draft_inserts_at_the_cursor_not_the_end() {
        let mut input = "merhaba dünya".to_owned();
        let mut cursor = "merhaba".chars().count(); // right after "merhaba"
        insert_pasted_text_at_cursor(&mut input, &mut cursor, " güzel");
        assert_eq!(input, "merhaba güzel dünya");
        assert_eq!(cursor, "merhaba güzel".chars().count());
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
            content: "merhaba bu mesaj kaydırma alanında görünür kalmalı".into(),
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
        let mut cursor = input.chars().count();
        insert_pasted_text_at_cursor(&mut input, &mut cursor, "dostum\n  nasılsın?");
        assert_eq!(input, "Merhaba dostum nasılsın?");
        assert_eq!(cursor, input.chars().count());
    }

    #[test]
    fn word_delete_keeps_utf8_boundaries_intact() {
        let mut input = "merhaba dünya güzel".to_owned();
        let mut cursor = input.chars().count();
        delete_previous_word(&mut input, &mut cursor);
        assert_eq!(input, "merhaba dünya");
        assert_eq!(cursor, input.chars().count());
        delete_previous_word(&mut input, &mut cursor);
        assert_eq!(input, "merhaba");
        assert_eq!(cursor, input.chars().count());
    }

    /// TUI bug #3 (2026-08-16): Ctrl+Backspace previously always deleted from the *end* of the
    /// whole draft, ignoring the cursor. Deleting a word from the middle must only remove that
    /// word, leaving the text after the cursor untouched.
    #[test]
    fn word_delete_from_the_middle_of_a_draft_only_removes_that_word() {
        let mut input = "merhaba dünya güzel".to_owned();
        let mut cursor = "merhaba dünya".chars().count(); // right after "dünya"
        delete_previous_word(&mut input, &mut cursor);
        // "dünya" and its separating space are both gone; the space before "güzel" is untouched,
        // so the cursor lands right after "merhaba" — matching what was left of the draft.
        assert_eq!(input, "merhaba güzel");
        assert_eq!(cursor, "merhaba".chars().count());
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
        assert!(should_clear_draft(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!is_delete_previous_word_shortcut(KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
    }

    /// Terminal/readline-style shortcuts (2026-08-16): the same bindings a shell, Claude Code, or
    /// Codex's own terminal session already uses.
    #[test]
    fn readline_style_shortcuts_are_recognized_by_their_real_keys_only() {
        assert!(is_move_to_start_shortcut(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_move_to_end_shortcut(KeyEvent::new(
            KeyCode::Char('e'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_kill_to_end_shortcut(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        // Ctrl+U's *real* readline meaning ("kill to start"), not the old "clear everything".
        assert!(is_kill_to_start_shortcut(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_forward_delete_shortcut(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_insert_newline_shortcut(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT,
        )));
        assert!(is_insert_newline_shortcut(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
        )));
        // None of these fire for a plain, unmodified letter — must never shadow ordinary typing.
        assert!(!is_move_to_start_shortcut(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(!is_insert_newline_shortcut(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn move_to_start_and_end_jump_the_cursor_past_any_word_boundary() {
        let input = "merhaba dünya".to_owned();
        let mut cursor = 3; // somewhere in the middle
        move_cursor_to_start(&mut cursor);
        assert_eq!(cursor, 0);
        move_cursor_to_end(&input, &mut cursor);
        assert_eq!(cursor, input.chars().count());
    }

    #[test]
    fn kill_to_end_and_kill_to_start_split_the_draft_at_the_cursor() {
        let mut input = "merhaba dünya".to_owned();
        let mut cursor = "merhaba".chars().count(); // right after "merhaba"

        let mut forward = input.clone();
        let mut forward_cursor = cursor;
        kill_to_end_from_cursor(&mut forward, &mut forward_cursor);
        assert_eq!(forward, "merhaba");
        assert_eq!(forward_cursor, cursor, "Ctrl+K must not move the cursor");

        kill_to_start_from_cursor(&mut input, &mut cursor);
        assert_eq!(input, " dünya");
        assert_eq!(cursor, 0, "Ctrl+U leaves the cursor at the new start");
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
        app.input_cursor = app.input.chars().count();
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

    /// F4 "Read-only proje analisti"nin ilk TUI komutu — gerçek proje kökü üzerinde (bu repo'nun
    /// kendisi) çalıştırılıp Rust/Cargo.toml'un doğru tespit edildiğini, ve manifest'i olmayan
    /// bir alt klasörün "tespit edilemedi" notunu doğru verdiğini kanıtlar.
    #[test]
    fn analyze_command_detects_this_repos_own_rust_manifest_and_reports_unknown_subfolders() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/analyze".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let reply = app.messages.last().unwrap().content.clone();
        assert!(reply.contains("Rust"), "reply was: {reply}");
        assert!(reply.contains("Cargo.toml"), "reply was: {reply}");
        assert!(reply.contains("cargo test"), "reply was: {reply}");
        assert!(reply.contains("salt-okunur"), "reply was: {reply}");

        app.input = "/analyze docs/adr".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let reply = app.messages.last().unwrap().content.clone();
        assert!(
            reply.contains("tespit edilemedi"),
            "a manifest-less subfolder must report unknown, not guess: {reply}"
        );
    }

    /// `/plan`'ın model-bağımlı asıl davranışı (`draft_coding_plan_with_provider`) zaten
    /// `project_analyst.rs`'de sahte bir sağlayıcıyla ağsız test ediliyor — burada yalnız argüman
    /// doğrulaması (model çağrısına hiç girmeyen, tamamen senkron yol) test ediliyor, `submit()`'in
    /// gerçek `LlamaServerProvider`'a bağlı olması yüzünden.
    #[test]
    fn plan_command_without_a_request_shows_usage_and_never_touches_the_model() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/plan".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(
            app.messages.last().unwrap().content.contains("Kullanım:"),
            "content was: {}",
            app.messages.last().unwrap().content
        );
        assert!(
            !app.pending,
            "an empty /plan must return synchronously, never spawn a worker"
        );
    }

    #[test]
    fn patch_without_a_pending_plan_is_a_synchronous_no_op() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/patch".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("/plan"));
        assert!(!app.pending, "no plan means no worker should ever spawn");
    }

    #[test]
    fn patch_with_an_empty_affected_files_plan_is_rejected_before_touching_the_model() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        app.pending_coding_plan = Some(
            jarvis_core::create_read_only_coding_plan(
                std::env::current_dir().expect("cwd"),
                "belirsiz istek",
                vec![],
                vec![],
            )
            .expect("valid empty-scope plan"),
        );

        app.input = "/patch".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("üretilemez"));
        assert!(!app.pending);
    }

    #[test]
    fn reject_patch_clears_the_pending_proposal_without_touching_any_file() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        app.input = "/reject-patch".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("yok"));

        let plan = jarvis_core::create_read_only_coding_plan(
            std::env::current_dir().expect("cwd"),
            "test",
            vec![PathBuf::from("src/lib.rs")],
            vec![],
        )
        .expect("valid plan");
        let proposal = jarvis_core::create_patch_proposal(
            &plan,
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n",
            vec![PathBuf::from("src/lib.rs")],
        )
        .expect("valid proposal");
        app.pending_patch = Some((plan, proposal));
        app.input = "/reject-patch".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("reddedildi"));
        assert!(app.pending_patch.is_none());
    }

    /// Kendi benzersiz geçici dizinini oluşturur (paylaşılan `temp_dir()` kökü değil) — testler
    /// paralel çalışırken aynı `a.txt`/`b.txt`'e çarpışmasın diye.
    fn two_file_pending_patch_fixture(
        name: &str,
    ) -> (PathBuf, jarvis_core::CodingPlan, jarvis_core::PatchProposal) {
        let root = std::env::temp_dir().join(format!(
            "jarvis-main-two-file-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("fixture root");
        std::fs::write(root.join("a.txt"), "old-a\n").expect("fixture a");
        std::fs::write(root.join("b.txt"), "old-b\n").expect("fixture b");
        let plan = jarvis_core::create_read_only_coding_plan(
            &root,
            "iki dosyayı değiştir",
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
            vec![],
        )
        .expect("valid plan");
        let diff =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old-a\n+new-a\n\
diff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-old-b\n+new-b\n";
        let proposal = jarvis_core::create_patch_proposal(
            &plan,
            diff,
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
        )
        .expect("valid two-file proposal");
        (root, plan, proposal)
    }

    #[test]
    fn patch_note_requires_a_pending_patch_and_can_be_set_and_cleared() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/patch-note önemli bir not".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("yok"));
        assert!(app.pending_patch_note.is_none());

        let (root, plan, proposal) = two_file_pending_patch_fixture("note");
        app.pending_patch = Some((plan, proposal));

        app.input = "/patch-note önemli bir not".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert_eq!(app.pending_patch_note.as_deref(), Some("önemli bir not"));
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("önemli bir not"));

        app.input = "/patch-note".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.pending_patch_note.is_none());
        assert!(app.messages.last().unwrap().content.contains("temizlendi"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn patch_files_shows_each_files_own_diff_block() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");

        app.input = "/patch-files".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("yok"));

        let (root, plan, proposal) = two_file_pending_patch_fixture("files");
        app.pending_patch = Some((plan, proposal));
        app.input = "/patch-files".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let reply = app.messages.last().unwrap().content.clone();
        assert!(reply.contains("a.txt"), "reply was: {reply}");
        assert!(reply.contains("b.txt"), "reply was: {reply}");
        assert!(reply.contains("new-a"));
        assert!(reply.contains("new-b"));

        std::fs::remove_dir_all(&root).ok();
    }

    /// F4 "Patch preview/review" seçilebilir dosya scope'u: `/approve-patch <dosya>` yalnız o
    /// dosyayı uygulamalı, diğerini hiç değiştirmemeli. Bu ortamda gerçek `bwrap` `CLONE_NEWNET`
    /// reddi yüzünden başlatılamayabilir — bu yüzden test iki geçerli sonuçtan birini kabul
    /// ediyor: ya yalnız seçilen dosya değişti ya da hiçbiri değişmedi (asla ikisi de değil, ve
    /// asla seçilmeyen dosya tek başına değişmedi).
    #[test]
    fn approve_patch_with_a_file_argument_scopes_to_only_that_file() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        let root = std::env::temp_dir().join(format!(
            "jarvis-main-scoped-approve-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("fixture root");
        std::fs::write(root.join("a.txt"), "old-a\n").expect("fixture a");
        std::fs::write(root.join("b.txt"), "old-b\n").expect("fixture b");
        let plan = jarvis_core::create_read_only_coding_plan(
            &root,
            "iki dosyayı değiştir",
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
            vec![],
        )
        .expect("valid plan");
        let diff =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old-a\n+new-a\n\
diff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-old-b\n+new-b\n";
        let proposal = jarvis_core::create_patch_proposal(
            &plan,
            diff,
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
        )
        .expect("valid proposal");
        app.pending_patch = Some((plan, proposal));

        app.input = "/approve-patch a.txt".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        let a_content = std::fs::read_to_string(root.join("a.txt")).unwrap();
        let b_content = std::fs::read_to_string(root.join("b.txt")).unwrap();
        assert_eq!(
            b_content, "old-b\n",
            "the unselected file must never change"
        );
        assert!(
            a_content == "old-a\n" || a_content == "new-a\n",
            "the selected file must end up in exactly one valid state, got: {a_content:?}"
        );
        assert!(app.pending_patch.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn approve_patch_rejects_a_file_argument_outside_the_proposal() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        let (root, plan, proposal) = two_file_pending_patch_fixture("reject-scope");
        app.pending_patch = Some((plan, proposal));

        app.input = "/approve-patch c.txt".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("geçersiz"));
        assert!(
            app.pending_patch.is_none(),
            "the proposal is consumed (take()) even on a rejected selection, matching the \
             existing /approve-patch failure convention"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn abort_without_an_active_job_is_a_clear_no_op() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        app.input = "/abort".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("yok"));
    }

    /// F4 "Yerel üretkenlik tool framework": `/note-append` model'e hiç dokunmadan, doğrudan
    /// Policy/Task/Approval zincirinden geçiyor — onaydan önce tam önizleme gösteriliyor, onaydan
    /// sonra dosyaya gerçekten yazılıyor ve doğrulanıyor.
    #[test]
    fn note_append_asks_for_approval_shows_a_preview_and_writes_only_after_approve() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        let relative_path = format!(
            "main-append-test-{}-{}.txt",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        );
        let full_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("append-notes")
            .join(&relative_path);

        app.input = format!("/note-append {relative_path} | merhaba dünya");
        submit(&mut app, &runtime, &provider, &vision, &sender);
        let reply = app.messages.last().unwrap().content.clone();
        assert!(reply.contains("Onay bekliyor"), "reply was: {reply}");
        assert!(reply.contains("merhaba dünya"), "reply was: {reply}");
        assert!(
            !full_path.exists(),
            "nothing must be written before approval"
        );

        app.input = "/approve".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert_eq!(
            std::fs::read_to_string(&full_path).unwrap(),
            "merhaba dünya\n"
        );

        std::fs::remove_file(&full_path).ok();
    }

    #[test]
    fn note_append_without_a_pipe_separator_shows_usage_and_never_touches_the_runtime() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        app.input = "/note-append eksik-ayirici".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("Kullanım:"));
    }

    #[test]
    fn abort_with_an_active_job_flips_the_cancel_flag() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        let cancel = jarvis_core::new_cancel_flag();
        app.active_cancel = Some(cancel.clone());
        app.input = "/abort".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("İptal"));
        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// F4 "Patch apply transaction" wired end to end through the real TUI command, no model
    /// involved (the proposal is built directly, mirroring what `/patch` would have produced).
    ///
    /// `jarvis_core` is linked here as a *regular* (non-`cfg(test)`) dependency of the `jarvis`
    /// binary's own test build — unlike `cargo test --lib` inside `jarvis_core` itself, its
    /// `#[cfg(test)]` plain-`git`-without-bwrap fallback is **not** active here, so this exercises
    /// the real, production `systemd-run` cgroup + bubblewrap path (ADR-0001: no host-shell
    /// fallback ever) end to end, on real hardware. Two real bugs were found and fixed live
    /// (2026-08-16) making this actually pass for the first time — both were previously
    /// misdiagnosed as "this dev sandbox denies `CLONE_NEWNET`", which was never the true cause:
    /// (1) `apply_worker_rlimits` set a *fixed* `RLIMIT_NPROC=64`, but that limit counts *all*
    /// threads the real UID owns system-wide, not "how many this worker spawns" — an ordinary
    /// desktop already owns thousands (browser, IDE, ...), so the fixed 64 made bwrap's own
    /// internal `unshare(CLONE_NEWUSER)` fail immediately; (2) `--tmpfs /tmp` was mounted *after*
    /// the workspace bind, so a workspace whose real path happens to live under `/tmp` (routine —
    /// `std::env::temp_dir()`-based roots, exactly what this test and real ad-hoc scratch
    /// workspaces use) got silently shadowed and disappeared inside the sandbox.
    #[test]
    fn approve_patch_with_no_test_plan_applies_immediately_and_stays_synchronous() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        let root = std::env::temp_dir().join(format!(
            "jarvis-main-approve-patch-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("fixture root");
        std::fs::write(root.join("demo.txt"), "old\n").expect("fixture file");

        let plan = jarvis_core::create_read_only_coding_plan(
            &root,
            "demo.txt içeriğini değiştir",
            vec![PathBuf::from("demo.txt")],
            vec![], // test planı yok -> tamamen senkron kalmalı
        )
        .expect("valid plan");
        let proposal = jarvis_core::create_patch_proposal(
            &plan,
            "diff --git a/demo.txt b/demo.txt\n--- a/demo.txt\n+++ b/demo.txt\n@@ -1 +1 @@\n-old\n+new\n",
            vec![PathBuf::from("demo.txt")],
        )
        .expect("valid proposal");
        app.pending_patch = Some((plan, proposal));

        app.input = "/approve-patch".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);

        let on_disk = std::fs::read_to_string(root.join("demo.txt")).unwrap();
        let reply = app.messages.last().unwrap().content.clone();
        assert_eq!(
            on_disk, "new\n",
            "the real isolated worker must actually apply the patch to disk; reply was: {reply}"
        );
        assert!(reply.contains("kalıcı"), "reply was: {reply}");
        assert!(!app.pending, "no test plan means no worker should spawn");
        assert!(app.pending_patch.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    /// The sibling of the test above, one layer further: a *non-empty* `test_plan` routes through
    /// `Runtime::apply_coding_patch_with_regression_check`, which runs the allowlist command
    /// runner (`run_allowlisted_command`/`run_test_plan`) — the *other* major consumer of
    /// `isolated_worker_command`, previously exercised only through the `#[cfg(test)]` bypass just
    /// like patch-apply was. Proven directly against `Runtime` (not through the TUI's background
    /// thread) to keep this a fast, synchronous, still-real assertion. `python3 -m platform` is
    /// allowlisted (only `-m` is, for `python3`), fast, and side-effect-free — enough to prove the
    /// real command actually ran inside the sandbox rather than being skipped or faked.
    #[test]
    fn approve_patch_with_a_real_test_plan_runs_it_through_the_real_isolated_worker() {
        let (runtime, _provider, _vision, _sender) = stored_runtime_fixture();
        let root = std::env::temp_dir().join(format!(
            "jarvis-main-approve-patch-testplan-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("fixture root");
        std::fs::write(root.join("demo.txt"), "old\n").expect("fixture file");

        let plan = jarvis_core::create_read_only_coding_plan(
            &root,
            "demo.txt içeriğini değiştir",
            vec![PathBuf::from("demo.txt")],
            vec!["python3 -m platform".to_string()],
        )
        .expect("valid plan");
        let proposal = jarvis_core::create_patch_proposal(
            &plan,
            "diff --git a/demo.txt b/demo.txt\n--- a/demo.txt\n+++ b/demo.txt\n@@ -1 +1 @@\n-old\n+new\n",
            vec![PathBuf::from("demo.txt")],
        )
        .expect("valid proposal");
        let approval = jarvis_core::approve_patch(&proposal, true).expect("user approved");

        let (checked, outcome) = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .apply_coding_patch_with_regression_check(&plan, &proposal, &approval, None)
            .expect("regression-checked apply must run");

        assert!(outcome.is_ok(), "outcome was: {outcome:?}");
        assert!(checked.kept, "no regression expected: {checked:?}");
        assert_eq!(
            checked.baseline.ran.len(),
            1,
            "the allowlisted command must actually run, not be skipped: {:?}",
            checked.baseline
        );
        assert!(checked.baseline.all_ran_passed());
        assert_eq!(checked.post_patch.ran.len(), 1);
        assert!(checked.post_patch.all_ran_passed());
        let on_disk = std::fs::read_to_string(root.join("demo.txt")).unwrap();
        assert_eq!(on_disk, "new\n");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn approve_patch_without_a_pending_proposal_is_a_clear_no_op() {
        let (runtime, provider, vision, sender) = stored_runtime_fixture();
        let mut app = App::new("ready");
        app.input = "/approve-patch".into();
        submit(&mut app, &runtime, &provider, &vision, &sender);
        assert!(app.messages.last().unwrap().content.contains("yok"));
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
