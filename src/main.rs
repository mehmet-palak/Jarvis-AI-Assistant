use std::io;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use jarvis_core::{
    inspect_local_image, propose_memory, AttachmentRef, DataSensitivity, InputType,
    LlamaServerProvider, MemoryNamespace, MemoryProposal, Request, Runtime, SqliteStore, TaskState,
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
    notify_user: bool,
    sources: Vec<String>,
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
    pending_memory: Option<MemoryProposal>,
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
            pending_memory: None,
        }
    }

    fn push_system(&mut self, content: impl Into<String>) {
        self.messages.push(Message {
            role: MessageRole::System,
            content: content.into(),
        });
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
    }
}

fn main() -> io::Result<()> {
    let project_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let store = SqliteStore::open(
        project_root
            .join("jarvis.db")
            .to_str()
            .expect("JARVIS database path must be UTF-8"),
    )
    .expect("JARVIS SQLite store açılamadı");
    let runtime = Arc::new(Mutex::new(Runtime::with_store(store)));
    let provider = LlamaServerProvider::local_default();
    let startup_note = ensure_local_model_server(&provider);
    run_tui(runtime, provider, startup_note)
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

fn stop_local_model_server() -> Result<(), String> {
    let status = Command::new("systemctl")
        .args(["--user", "stop", "jarvis-llama.service"])
        .status()
        .map_err(|error| format!("model sunucusu durdurulamadı: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "model sunucusu durdurulamadı (systemctl: {status})"
        ))
    }
}

fn run_tui(
    runtime: Arc<Mutex<Runtime>>,
    provider: LlamaServerProvider,
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
    let result = event_loop(&mut terminal, runtime, provider, app);
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
            app.status = reply.status;
            app.pending = false;
            if reply.notify_user {
                notify_response_ready(
                    app.messages
                        .get(reply.message_index)
                        .map(|message| message.content.as_str())
                        .unwrap_or_default(),
                );
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
                match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll = app.scroll.saturating_add(3),
                    MouseEventKind::ScrollDown => app.scroll = app.scroll.saturating_sub(3),
                    _ => {}
                }
                continue;
            }
            Event::Key(key) => key,
            _ => continue,
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            app.running = false;
            continue;
        }
        match key.code {
            KeyCode::Up => {
                app.scroll = app.scroll.saturating_add(3);
                continue;
            }
            KeyCode::PageUp => {
                app.scroll = app.scroll.saturating_add(8);
                continue;
            }
            KeyCode::Down => {
                app.scroll = app.scroll.saturating_sub(3);
                continue;
            }
            KeyCode::PageDown | KeyCode::End => {
                app.scroll = 0;
                continue;
            }
            KeyCode::Home => {
                app.scroll = u16::MAX;
                continue;
            }
            _ => {}
        }
        if app.pending {
            if key.code == KeyCode::Esc {
                app.status = "JARVIS yanıt üretirken girdi kilitli. Yanıt tamamlanınca yeni mesaj gönderebilirsin.".into();
            }
            continue;
        }
        let control_paste = (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('v' | 'V')))
            || matches!(key.code, KeyCode::Char('\u{16}'));
        if control_paste {
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
        let control_word_delete = (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Backspace | KeyCode::Char('w' | 'W')))
            || matches!(key.code, KeyCode::Char('\u{17}'));
        if control_word_delete {
            delete_previous_word(&mut app.input);
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('u' | 'U'))
        {
            app.input.clear();
            app.status = "Taslak temizlendi.".into();
            continue;
        }
        match key.code {
            KeyCode::Enter => submit(&mut app, &runtime, &provider, &sender),
            KeyCode::Backspace => {
                app.input.pop();
            }
            KeyCode::Char(character) => app.input.push(character),
            KeyCode::Esc => app.input.clear(),
            _ => {}
        }
    }
    Ok(())
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
    let output = Command::new("wl-paste")
        .arg("--no-newline")
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

fn submit(
    app: &mut App,
    runtime: &Arc<Mutex<Runtime>>,
    provider: &LlamaServerProvider,
    sender: &mpsc::Sender<WorkerReply>,
) {
    let input = app.input.trim().to_owned();
    app.input.clear();
    if input.is_empty() {
        return;
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
        match inspect_local_image(path) {
            Ok(attachment) => {
                if app
                    .attachments
                    .iter()
                    .any(|queued| queued.sha256 == attachment.sha256)
                {
                    app.push_system("Bu görsel zaten ek kuyruğunda.");
                } else {
                    app.push_system(format!(
                        "Ek kuyruğa alındı: {} • {} • {}×{} • SHA-256:{}…\nMevcut text-only model görsel piksellerini analiz etmez; ek metadata olarak güvenle taşınır. Vision modeli kurulduğunda aynı contract görsel girdiye bağlanacak.",
                        attachment.original_name,
                        attachment.mime_type(),
                        attachment.width,
                        attachment.height,
                        &attachment.sha256[..12],
                    ));
                    app.attachments.push(attachment);
                }
            }
            Err(error) => app.push_system(format!("Ek alınamadı: {error}")),
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
        let Some((key, value)) = specification
            .split_once('=')
            .map(|(key, value)| (key.trim(), value.trim()))
        else {
            app.push_system("Kullanım: /remember anahtar = değer • ardından /remember approve veya /remember reject");
            return;
        };
        match propose_memory(
            MemoryNamespace::UserProfile,
            key,
            value,
            DataSensitivity::Internal,
            "tui-user-approved-profile",
            true,
            None,
        ) {
            Ok(proposal) => {
                app.push_system(format!(
                    "Bellek teklifi (henüz kaydedilmedi): {} = {}\nNamespace: {} • sensitivity: {} • model context: evet\nOnay: /remember approve • Vazgeç: /remember reject",
                    proposal.record.key,
                    proposal.record.value,
                    proposal.record.namespace.as_str(),
                    proposal.record.sensitivity.as_str(),
                ));
                app.pending_memory = Some(proposal);
            }
            Err(error) => app.push_system(format!("Bellek teklifi geçersiz: {error}")),
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
    if let Some(relative_path) = input.strip_prefix("/index ").map(str::trim) {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        match runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .index_workspace_document(&root, std::path::Path::new(relative_path), true)
        {
            Ok(report) => app.push_system(format!(
                "Dosya indekslendi: {} • {} parça • SHA-256:{}…\nBu klasöre ait metin artık yalnız ilgili sorularda kaynaklı data olarak kullanılabilir.",
                report.canonical_path.display(),
                report.chunk_count,
                &report.content_sha256[..12]
            )),
            Err(error) => app.push_system(format!("Dosya indekslenemedi: {error}")),
        }
        return;
    }
    match input.as_str() {
        "exit" | "/exit" => {
            app.status = match stop_local_model_server() {
                Ok(()) => "JARVIS kapandı; model RAM'den çıkarıldı.".into(),
                Err(error) => error,
            };
            app.running = false;
            return;
        }
        "/quit" => {
            app.running = false;
            return;
        }
        "/clear" => {
            app.messages.clear();
            app.push_system("Sohbet görünümü temizlendi. Local session context güvenlik için RAM'de kalır; yeni oturum için JARVIS'i yeniden başlatabilirsin.");
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
                        format!(
                            "{} • {} • {}×{} • {}…",
                            attachment.original_name,
                            attachment.mime_type(),
                            attachment.width,
                            attachment.height,
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
            app.push_system("Kısayollar: Enter gönder • Ctrl+V yapıştır • Ctrl+Backspace veya Ctrl+W önceki kelimeyi sil • Ctrl+U taslağı temizle • Esc taslağı sil. Geçmiş: ↑/↓ veya PageUp/PageDown. Ek: /attach <PNG/JPEG-yolu>, /attachments, /attachments clear. Bellek: /remember anahtar = değer, /remember approve|reject, /memory, /forget <id>|all. RAG: /index <proje-içi-göreli-dosya>. Komutlar: /status, /approvals, /approve, /cancel, /clear, /quit, exit. `exit` modeli RAM'den çıkarır; /quit veya Ctrl+C yalnız arayüzü kapatır.");
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
            app.push_system(format!(
                "Model server: {} • CPU-only • VRAM layer: 0 • {}",
                model_label(&app.model_state),
                app.status
            ));
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
    app.scroll = 0;
    let attachments = std::mem::take(&mut app.attachments);
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
    app.status = "JARVIS yanıt üretiyor…".into();
    let runtime = Arc::clone(runtime);
    let provider = provider.clone();
    let sender = sender.clone();
    std::thread::spawn(move || {
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
        let (task, tool, verification) = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .handle_with_provider(request, &provider);
        let sources = tool
            .evidence
            .iter()
            .filter_map(|evidence| evidence.strip_prefix("workspace.citation:"))
            .map(|source| format!("• {source}"))
            .collect::<Vec<_>>();
        let content = tool.error.clone().unwrap_or(tool.output);
        let approval_pending = task.state == TaskState::WaitingForUser;
        let notify_user =
            task.state == TaskState::Completed && !approval_pending && !content.trim().is_empty();
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
            notify_user,
            sources,
        });
    });
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

/// Notifications are best-effort: a missing notification daemon must never affect a completed
/// task or the terminal UI. `notify-send` integrates with Hyprland's standard notification path.
fn notify_response_ready(content: &str) {
    let preview = notification_preview(content);
    if preview.is_empty() {
        return;
    }
    let _ = Command::new("notify-send")
        .args([
            "--app-name=JARVIS",
            "--icon=dialog-information",
            "--expire-time=6000",
            "JARVIS yanıtı hazır",
            preview.as_str(),
        ])
        .status();
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
        append_pasted_text, delete_previous_word, draft_rows, history_line_count, history_lines,
        input_view, notification_preview, Message, MessageRole,
    };

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
}
