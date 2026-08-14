//! Native desktop client for JARVIS. It is intentionally a thin UI client over `jarvis-core`.
//! Closing this window never stops the persistent local model service; the user can explicitly
//! request that from the status panel.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText, Stroke, TextureHandle, TextureOptions};
use jarvis_core::{
    default_desktop_preferences_path, inspect_local_image, load_desktop_preferences,
    save_desktop_preferences, AttachmentRef, DesktopPreferences, InputType, LlamaServerProvider,
    Request, Runtime, SqliteStore, TaskState, ThemePreference,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    placeholder_index: usize,
    content: String,
    status: String,
    sources: Vec<String>,
    notification: Option<DesktopNotification>,
}

struct DesktopNotification {
    title: &'static str,
    content: String,
}

/// A small Linux-first lock for the native shell. The model server remains independently
/// persistent; this lock only prevents two desktop windows from racing over one UI session.
struct DesktopInstanceLock {
    path: PathBuf,
}

impl Drop for DesktopInstanceLock {
    fn drop(&mut self) {
        let owns_lock = fs::read_to_string(&self.path)
            .ok()
            .is_some_and(|content| content.trim() == std::process::id().to_string());
        if owns_lock {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn default_desktop_lock_path() -> PathBuf {
    let runtime_root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    runtime_root.join("jarvis").join("desktop.lock")
}

fn acquire_desktop_instance_lock(path: PathBuf) -> Result<DesktopInstanceLock, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "desktop lock path needs a parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("desktop lock directory cannot be created: {error}"))?;

    for _ in 0..2 {
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = writeln!(file, "{}", std::process::id()) {
                    let _ = fs::remove_file(&path);
                    return Err(format!("desktop lock cannot be written: {error}"));
                }
                return Ok(DesktopInstanceLock { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let lock_pid = fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| content.trim().parse::<u32>().ok());
                let live_lock = lock_pid
                    .map(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists())
                    .unwrap_or(true);
                if live_lock {
                    return Err("JARVIS desktop zaten açık; mevcut pencereyi kullan.".into());
                }
                fs::remove_file(&path).map_err(|remove_error| {
                    format!("bayat desktop lock kaldırılamadı: {remove_error}")
                })?;
            }
            Err(error) => return Err(format!("desktop lock oluşturulamadı: {error}")),
        }
    }
    Err("desktop lock alınamadı; tekrar dene.".into())
}

struct JarvisDesktop {
    runtime: Arc<Mutex<Runtime>>,
    provider: LlamaServerProvider,
    receiver: mpsc::Receiver<WorkerReply>,
    sender: mpsc::Sender<WorkerReply>,
    messages: Vec<Message>,
    message_search: String,
    role_filter: Option<MessageRole>,
    draft: String,
    queued_attachments: Vec<AttachmentRef>,
    previews: HashMap<String, TextureHandle>,
    pending_approval_tasks: Vec<String>,
    pending: bool,
    status: String,
    last_model_check: Instant,
    model_status: String,
    preferences_path: Option<PathBuf>,
    preferences: DesktopPreferences,
    baseline_pixels_per_point: f32,
    orb_phase: f32,
}

impl JarvisDesktop {
    fn new(
        runtime: Arc<Mutex<Runtime>>,
        provider: LlamaServerProvider,
        initial_status: String,
        context: &egui::Context,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let preferences_path = default_desktop_preferences_path();
        let (preferences, preferences_status) = match preferences_path.as_deref() {
            Some(path) => match load_desktop_preferences(path) {
                Ok(preferences) => (preferences, None),
                Err(error) => (
                    DesktopPreferences::default(),
                    Some(format!(
                        "UI ayarları okunamadı; güvenli varsayılanlar kullanıldı: {error}"
                    )),
                ),
            },
            None => (
                DesktopPreferences::default(),
                Some("UI ayar yolu bulunamadı; tercihler bu oturumda kalacak.".into()),
            ),
        };
        let pending_approval_tasks = runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .pending_approvals()
            .into_iter()
            .map(|approval| approval.task_id.clone())
            .collect();
        Self {
            runtime,
            provider,
            receiver,
            sender,
            messages: vec![Message {
                role: MessageRole::System,
                content: "JARVIS desktop hazır. Mesajlar salt-okunur kartlarda görünür; yalnız alttaki composer düzenlenebilir. Pencereyi kapatmak model sunucusunu RAM'den çıkarmaz.".into(),
            }],
            message_search: String::new(),
            role_filter: None,
            draft: String::new(),
            queued_attachments: vec![],
            previews: HashMap::new(),
            pending_approval_tasks,
            pending: false,
            status: preferences_status.unwrap_or(initial_status),
            last_model_check: Instant::now(),
            model_status: "kontrol ediliyor".into(),
            preferences_path,
            preferences,
            baseline_pixels_per_point: context.pixels_per_point(),
            orb_phase: 0.0,
        }
    }

    fn poll_worker(&mut self) {
        while let Ok(reply) = self.receiver.try_recv() {
            if let Some(message) = self.messages.get_mut(reply.placeholder_index) {
                message.content = reply.content;
                if !reply.sources.is_empty() {
                    message.content.push_str("\n\nKaynaklar:\n");
                    message.content.push_str(&reply.sources.join("\n"));
                }
            }
            self.pending = false;
            self.status = reply.status;
            if let Some(notification) = reply.notification {
                notify_desktop(notification.title, &notification.content);
            }
            self.refresh_pending_approvals();
        }
    }

    fn refresh_pending_approvals(&mut self) {
        self.pending_approval_tasks = self
            .runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .pending_approvals()
            .into_iter()
            .map(|approval| approval.task_id.clone())
            .collect();
    }

    fn approve_task(&mut self, task_id: &str) {
        let result = self
            .runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .approve(task_id);
        match result {
            Some((task, tool, verification)) => {
                let content = tool.error.unwrap_or(tool.output);
                self.messages.push(Message {
                    role: MessageRole::System,
                    content: format!(
                        "Onaylı işlem tamamlandı ({}) • doğrulama: {:?}\n{}",
                        task.task_id, verification.status, content
                    ),
                });
                self.status = format!(
                    "Onaylı işlem tamamlandı • doğrulama: {:?}",
                    verification.status
                );
                if self.preferences.notifications_enabled {
                    notify_desktop("JARVIS onaylı işlemi tamamladı", &content);
                }
            }
            None => {
                self.status =
                    "Onay uygulanamadı; task bulunamadı, süresi doldu veya artık beklemiyor."
                        .into();
            }
        }
        self.refresh_pending_approvals();
    }

    fn reject_task(&mut self, task_id: &str) {
        let task = self
            .runtime
            .lock()
            .expect("JARVIS runtime lock poisoned")
            .cancel(task_id);
        match task {
            Some(task) => {
                self.messages.push(Message {
                    role: MessageRole::System,
                    content: format!("İşlem reddedildi: {}", task.task_id),
                });
                self.status = "İşlem reddedildi; yan etki çalıştırılmadı.".into();
            }
            None => {
                self.status =
                    "Reddetme uygulanamadı; task bulunamadı veya artık beklemiyor.".into();
            }
        }
        self.refresh_pending_approvals();
    }

    fn refresh_model_state(&mut self) {
        if self.last_model_check.elapsed().as_secs() < 3 {
            return;
        }
        self.last_model_check = Instant::now();
        let mut health_provider = self.provider.clone();
        health_provider.timeout_seconds = 1;
        self.model_status = match health_provider.runtime_state() {
            jarvis_core::ModelRuntimeState::Ready => "MODEL HAZIR".into(),
            _ => "MODEL BAŞLATILIYOR".into(),
        };
    }

    fn add_attachment(&mut self, context: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Görseller", &["png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };
        let attachment = match inspect_local_image(&path) {
            Ok(attachment) => attachment,
            Err(error) => {
                self.status = format!("Ek alınamadı: {error}");
                return;
            }
        };
        if self
            .queued_attachments
            .iter()
            .any(|queued| queued.sha256 == attachment.sha256)
        {
            self.status = "Bu görsel zaten ek kuyruğunda.".into();
            return;
        }
        match load_preview(context, &attachment) {
            Ok(preview) => {
                self.previews
                    .insert(attachment.attachment_id.clone(), preview);
                self.status = format!(
                    "Ek hazır: {} • {}×{} • gönderimden önce kaldırılabilir.",
                    attachment.original_name, attachment.width, attachment.height
                );
                self.queued_attachments.push(attachment);
            }
            Err(error) => {
                self.status =
                    format!("Ek önizlemesi açılamadı; güvenlik için kuyrukta tutulmadı: {error}");
            }
        }
    }

    fn submit(&mut self) {
        if self.pending {
            return;
        }
        let content = self.draft.trim().to_owned();
        if content.is_empty() {
            return;
        }
        self.draft.clear();
        let attachments = std::mem::take(&mut self.queued_attachments);
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
        self.messages.push(Message {
            role: MessageRole::User,
            content: format!("{content}{attachment_summary}"),
        });
        let placeholder_index = self.messages.len();
        self.messages.push(Message {
            role: MessageRole::Jarvis,
            content: "Düşünüyorum…".into(),
        });
        self.pending = true;
        self.status = "JARVIS yanıt üretiyor…".into();
        let sender = self.sender.clone();
        let runtime = Arc::clone(&self.runtime);
        let provider = self.provider.clone();
        let notifications_enabled = self.preferences.notifications_enabled;
        std::thread::spawn(move || {
            let request = Request {
                schema_version: 1,
                request_id: format!(
                    "desktop-{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("system clock must be after UNIX epoch")
                        .as_nanos()
                ),
                input_type: InputType::Gui,
                content,
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
                .collect();
            let content = tool.error.clone().unwrap_or(tool.output);
            let notification = desktop_notification(task.state, &content, notifications_enabled);
            let status = match task.state {
                TaskState::WaitingForUser => {
                    "İşlem onayını bekliyor; TUI komut akışı kullanılabilir.".into()
                }
                TaskState::Completed => {
                    format!("Yanıt hazır • doğrulama: {:?}", verification.status)
                }
                _ => format!(
                    "İşlem {:?} • doğrulama: {:?}",
                    task.state, verification.status
                ),
            };
            let _ = sender.send(WorkerReply {
                placeholder_index,
                content,
                status,
                sources,
                notification,
            });
        });
    }

    fn apply_preferences(&self, context: &egui::Context) {
        let visuals = match self.preferences.theme {
            ThemePreference::System | ThemePreference::Dark => jarvis_dark_visuals(),
            ThemePreference::Light => egui::Visuals::light(),
        };
        context.set_visuals(visuals);
        context.set_pixels_per_point(
            self.baseline_pixels_per_point * self.preferences.font_scale_percent as f32 / 100.0,
        );
    }

    fn save_preferences(&mut self) {
        self.status = match self.preferences_path.as_deref() {
            Some(path) => match save_desktop_preferences(path, &self.preferences) {
                Ok(()) => format!("UI ayarları kaydedildi: {}", path.display()),
                Err(error) => format!("UI ayarları kaydedilemedi: {error}"),
            },
            None => "UI ayar yolu bulunamadı; değişiklik bu oturumda kalacak.".into(),
        };
    }

    fn export_preferences(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("jarvis-desktop.json")
            .save_file()
        else {
            return;
        };
        self.status = match save_desktop_preferences(&path, &self.preferences) {
            Ok(()) => format!("UI ayarları dışa aktarıldı: {}", path.display()),
            Err(error) => format!("UI ayarları dışa aktarılamadı: {error}"),
        };
    }

    fn show_preferences_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Görünüm");
        ui.label("Bu tercihler yalnız yerel UI ayarlarıdır; sohbet veya ek yolu kaydedilmez.");

        let mut changed = false;
        egui::ComboBox::from_label("Tema")
            .selected_text(match self.preferences.theme {
                ThemePreference::System => "Sistem",
                ThemePreference::Dark => "Koyu",
                ThemePreference::Light => "Açık",
            })
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(
                        &mut self.preferences.theme,
                        ThemePreference::System,
                        "Sistem",
                    )
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.preferences.theme, ThemePreference::Dark, "Koyu")
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.preferences.theme, ThemePreference::Light, "Açık")
                    .changed();
            });
        changed |= ui
            .add(
                egui::Slider::new(&mut self.preferences.font_scale_percent, 75..=175)
                    .text("Yazı ölçeği (%)"),
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.preferences.notifications_enabled,
                "Yanıt bildirimi",
            )
            .changed();

        if changed {
            self.save_preferences();
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Varsayılanlara dön").clicked() {
                self.preferences = DesktopPreferences::default();
                self.save_preferences();
            }
            if ui.button("Dışa aktar").clicked() {
                self.export_preferences();
            }
        });
        if let Some(path) = &self.preferences_path {
            ui.small(format!("Ayar dosyası: {}", path.display()));
        }
    }

    fn show_approval_controls(&mut self, ui: &mut egui::Ui) {
        if self.pending_approval_tasks.is_empty() {
            return;
        }
        ui.separator();
        ui.heading("Onay bekleyen işlemler");
        ui.colored_label(
            Color32::from_rgb(205, 165, 65),
            "Onay yalnız aşağıdaki task için geçerlidir. Reddetmek hiçbir yan etki çalıştırmaz.",
        );
        for task_id in self.pending_approval_tasks.clone() {
            ui.group(|ui| {
                ui.monospace(&task_id);
                ui.horizontal(|ui| {
                    if ui.button("Onayla").clicked() {
                        self.approve_task(&task_id);
                    }
                    if ui.button("Reddet").clicked() {
                        self.reject_task(&task_id);
                    }
                });
            });
        }
    }

    fn show_system_panel(&mut self, ui: &mut egui::Ui) {
        hud_section_title(ui, "SİSTEM DURUMU");
        let model_color = if self.model_status == "MODEL HAZIR" {
            COLOR_GREEN
        } else {
            COLOR_GOLD
        };
        hud_status_row(ui, "CORE", "LOCAL-FIRST", COLOR_TEAL);
        hud_status_row(ui, "MODEL", &self.model_status, model_color);
        hud_status_row(ui, "DONANIM", "CPU-ONLY / VRAM 0", COLOR_TEAL_DIM);
        hud_status_row(ui, "ERİŞİM", "LOOPBACK", COLOR_TEAL_DIM);
        ui.add_space(10.0);

        egui::Frame::group(ui.style())
            .fill(COLOR_PANEL_ALT)
            .stroke(Stroke::new(1.0_f32, COLOR_TEAL_DIM))
            .show(ui, |ui| {
                ui.label(RichText::new("GÜVENLİK SINIRI").small().color(COLOR_TEAL));
                ui.small("Onay gerektiren bir işlem, yalnız kendi task kimliği için çalışır.");
            });
        self.show_approval_controls(ui);

        ui.add_space(8.0);
        if ui
            .add_sized(
                [ui.available_width(), 30.0],
                egui::Button::new("MODELİ RAM'DEN ÇIKAR"),
            )
            .clicked()
        {
            self.status = match stop_local_model_server() {
                Ok(()) => "Model sunucusu durduruldu; RAM serbest bırakıldı.".into(),
                Err(error) => error,
            };
        }
        ui.small("Pencereyi kapatmak modeli durdurmaz.");

        ui.add_space(10.0);
        egui::CollapsingHeader::new("GÖRÜNÜM AYARLARI")
            .default_open(false)
            .show(ui, |ui| self.show_preferences_controls(ui));
    }

    fn show_chat_console(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            self.show_chat_composer(ui, context);
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .id_salt("jarvis-chat-history")
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for message in self.messages.iter().filter(|message| {
                        message_matches_filter(message, &self.message_search, self.role_filter)
                    }) {
                        let (label, fill, label_color) = match message.role {
                            MessageRole::User => ("SEN", COLOR_USER, COLOR_GOLD),
                            MessageRole::Jarvis => ("J.A.R.V.I.S", COLOR_PANEL_ALT, COLOR_TEAL),
                            MessageRole::System => ("SİSTEM", COLOR_SYSTEM, COLOR_BLUE),
                        };
                        egui::Frame::group(ui.style())
                            .fill(fill)
                            .stroke(Stroke::new(1.0_f32, COLOR_TEAL_DIM))
                            .show(ui, |ui| {
                                ui.label(RichText::new(label).small().strong().color(label_color));
                                ui.add(egui::Label::new(&message.content).selectable(true).wrap());
                            });
                        ui.add_space(7.0);
                    }
                });

            ui.add_space(5.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("SOHBET ARŞİVİ").small().color(COLOR_TEAL));
                ui.add(
                    egui::TextEdit::singleline(&mut self.message_search)
                        .id(egui::Id::new("jarvis-message-search"))
                        .desired_width(130.0)
                        .hint_text("Ara"),
                );
                for (role, label) in [
                    (None, "Tümü"),
                    (Some(MessageRole::User), "Sen"),
                    (Some(MessageRole::Jarvis), "JARVIS"),
                    (Some(MessageRole::System), "Sistem"),
                ] {
                    ui.selectable_value(&mut self.role_filter, role, label);
                }
            });
        });
    }

    fn show_chat_composer(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        hud_section_title(ui, "KOMUT GİRİŞİ");
        ui.horizontal(|ui| {
            if ui.button("GÖRSEL EKLE  Ctrl+O").clicked() {
                self.add_attachment(context);
            }
            if !self.queued_attachments.is_empty() && ui.button("EKLERİ KALDIR").clicked() {
                self.queued_attachments.clear();
                self.previews.clear();
                self.status = "Ek kuyruğu temizlendi; hiçbir dosya silinmedi.".into();
            }
        });
        let mut remove_attachment = None;
        ui.horizontal_wrapped(|ui| {
            for attachment in &self.queued_attachments {
                egui::Frame::group(ui.style())
                    .fill(COLOR_PANEL_ALT)
                    .show(ui, |ui| {
                        if let Some(texture) = self.previews.get(&attachment.attachment_id) {
                            ui.add(egui::Image::new(texture).max_size(egui::vec2(80.0, 60.0)));
                        }
                        ui.label(&attachment.original_name);
                        if ui.small_button("Kaldır").clicked() {
                            remove_attachment = Some(attachment.attachment_id.clone());
                        }
                    });
            }
        });
        if let Some(attachment_id) = remove_attachment {
            self.queued_attachments
                .retain(|attachment| attachment.attachment_id != attachment_id);
            self.previews.remove(&attachment_id);
        }
        if !self.queued_attachments.is_empty() {
            ui.colored_label(
                COLOR_GOLD,
                "Vision modeli kurulana kadar ekler yalnız doğrulanmış metadata olarak taşınır.",
            );
        }
        let response = ui.add(
            egui::TextEdit::multiline(&mut self.draft)
                .desired_rows(3)
                .hint_text("Mesajını yaz. Enter gönderir; Shift+Enter yeni satır ekler."),
        );
        let submit_with_enter = response.has_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.pending, egui::Button::new("GÖNDER  ›"))
                .clicked()
                || submit_with_enter
            {
                self.submit();
            }
            if self.pending {
                ui.spinner();
                ui.label(RichText::new("JARVIS YANIT ÜRETİYOR").color(COLOR_GOLD));
            }
        });
    }

    fn show_hud(&mut self, ui: &mut egui::Ui) {
        let canvas = ui.max_rect();
        paint_hud_background(ui.painter(), canvas);
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.05).min(22.0));
            ui.label(
                RichText::new("J . A . R . V . I . S")
                    .size(20.0)
                    .strong()
                    .color(COLOR_TEAL),
            );
            ui.label(
                RichText::new("LOCAL AI COMMAND INTERFACE")
                    .small()
                    .color(COLOR_TEAL_DIM),
            );
            ui.add_space(10.0);
            let orb_side = ui
                .available_width()
                .min(ui.available_height() * 0.58)
                .clamp(160.0, 360.0);
            let (orb_rect, _) =
                ui.allocate_exact_size(egui::vec2(orb_side, orb_side), egui::Sense::hover());
            paint_orb(
                ui.painter(),
                orb_rect,
                self.orb_phase,
                self.pending,
                self.model_status == "MODEL HAZIR",
            );
            ui.add_space(8.0);
            let activity = hud_activity_label(self.pending, &self.model_status);
            let activity_color = if self.pending { COLOR_GOLD } else { COLOR_TEAL };
            ui.label(RichText::new(activity).strong().color(activity_color));
            ui.label(
                RichText::new("SİSTEM DURUMU: LOCAL / CPU / GOVERNED")
                    .small()
                    .color(COLOR_TEAL_DIM),
            );
        });
    }
}

const COLOR_BG: Color32 = Color32::from_rgb(2, 12, 12);
const COLOR_PANEL: Color32 = Color32::from_rgb(3, 15, 15);
const COLOR_PANEL_ALT: Color32 = Color32::from_rgb(6, 25, 24);
const COLOR_TEAL: Color32 = Color32::from_rgb(0, 212, 192);
const COLOR_TEAL_DIM: Color32 = Color32::from_rgb(0, 106, 98);
const COLOR_TEXT: Color32 = Color32::from_rgb(190, 244, 238);
const COLOR_GREEN: Color32 = Color32::from_rgb(0, 255, 136);
const COLOR_GOLD: Color32 = Color32::from_rgb(255, 204, 0);
const COLOR_BLUE: Color32 = Color32::from_rgb(68, 136, 255);
const COLOR_USER: Color32 = Color32::from_rgb(36, 39, 30);
const COLOR_SYSTEM: Color32 = Color32::from_rgb(19, 38, 47);

fn jarvis_dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = COLOR_BG;
    visuals.window_fill = COLOR_PANEL;
    visuals.extreme_bg_color = COLOR_BG;
    visuals.faint_bg_color = COLOR_PANEL_ALT;
    visuals.code_bg_color = COLOR_PANEL_ALT;
    visuals.widgets.noninteractive.bg_fill = COLOR_PANEL;
    visuals.widgets.noninteractive.fg_stroke.color = COLOR_TEXT;
    visuals.widgets.inactive.bg_fill = COLOR_PANEL_ALT;
    visuals.widgets.inactive.bg_stroke.color = COLOR_TEAL_DIM;
    visuals.widgets.inactive.fg_stroke.color = COLOR_TEXT;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(8, 49, 45);
    visuals.widgets.hovered.bg_stroke.color = COLOR_TEAL;
    visuals.widgets.hovered.fg_stroke.color = COLOR_TEAL;
    visuals.widgets.active.bg_fill = Color32::from_rgb(0, 76, 69);
    visuals.widgets.active.bg_stroke.color = COLOR_TEAL;
    visuals.widgets.active.fg_stroke.color = Color32::WHITE;
    visuals.selection.bg_fill = Color32::from_rgb(0, 91, 83);
    visuals.selection.stroke.color = COLOR_TEAL;
    visuals.hyperlink_color = COLOR_TEAL;
    visuals.warn_fg_color = COLOR_GOLD;
    visuals.error_fg_color = Color32::from_rgb(255, 70, 80);
    visuals
}

fn hud_section_title(ui: &mut egui::Ui, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).small().strong().color(COLOR_TEAL));
        let start = ui.cursor().min;
        let end = egui::pos2(ui.max_rect().right(), start.y + 1.0);
        ui.painter()
            .line_segment([start, end], Stroke::new(1.0_f32, COLOR_TEAL_DIM));
    });
}

fn hud_status_row(ui: &mut egui::Ui, name: &str, value: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).small().color(COLOR_TEAL_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).small().strong().color(color));
        });
    });
}

fn hud_activity_label(pending: bool, model_status: &str) -> &'static str {
    if pending {
        "DÜŞÜNÜYOR"
    } else if model_status == "MODEL HAZIR" {
        "HAZIR / DİNLİYORUM"
    } else {
        "CORE BAŞLATILIYOR"
    }
}

fn paint_hud_background(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 0.0, COLOR_BG);
    let inset = 18.0;
    let edge = 28.0;
    let stroke = Stroke::new(1.0_f32, COLOR_TEAL_DIM);
    let top_left = rect.left_top() + egui::vec2(inset, inset);
    let top_right = rect.right_top() + egui::vec2(-inset, inset);
    let bottom_left = rect.left_bottom() + egui::vec2(inset, -inset);
    let bottom_right = rect.right_bottom() + egui::vec2(-inset, -inset);
    for (corner, x_direction, y_direction) in [
        (top_left, 1.0, 1.0),
        (top_right, -1.0, 1.0),
        (bottom_left, 1.0, -1.0),
        (bottom_right, -1.0, -1.0),
    ] {
        painter.line_segment(
            [corner, corner + egui::vec2(edge * x_direction, 0.0)],
            stroke,
        );
        painter.line_segment(
            [corner, corner + egui::vec2(0.0, edge * y_direction)],
            stroke,
        );
    }
    let center = rect.center();
    for fraction in [0.24, 0.38, 0.52, 0.66] {
        let radius = rect.width().min(rect.height()) * fraction;
        painter.circle_stroke(
            center,
            radius,
            Stroke::new(0.5_f32, Color32::from_rgba_unmultiplied(0, 106, 98, 44)),
        );
    }
}

fn paint_orb(
    painter: &egui::Painter,
    rect: egui::Rect,
    phase: f32,
    pending: bool,
    model_ready: bool,
) {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.27;
    let pulse = if pending {
        1.0 + phase.sin() * 0.08
    } else if model_ready {
        1.0 + (phase * 0.45).sin() * 0.025
    } else {
        1.0
    };
    let energy = if pending { COLOR_GOLD } else { COLOR_TEAL };
    painter.circle_filled(
        center,
        radius * 1.72 * pulse,
        Color32::from_rgba_unmultiplied(energy.r(), energy.g(), energy.b(), 14),
    );
    painter.circle_filled(
        center,
        radius * 0.72 * pulse,
        Color32::from_rgba_unmultiplied(energy.r(), energy.g(), energy.b(), 50),
    );
    painter.circle_filled(center, radius * 0.48 * pulse, COLOR_PANEL_ALT);
    painter.circle_stroke(center, radius * 0.48 * pulse, Stroke::new(2.0_f32, energy));
    painter.circle_stroke(
        center,
        radius * 0.82,
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 212, 192, 160)),
    );
    painter.circle_stroke(
        center,
        radius * 1.13,
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 106, 98, 210)),
    );
    for (index, (radius_scale, span)) in [(1.33, 0.95), (1.53, 0.68), (1.75, 0.45)]
        .into_iter()
        .enumerate()
    {
        let start = phase * (0.4 + index as f32 * 0.12) + index as f32 * 2.1;
        let points = (0..42)
            .map(|step| {
                let angle = start + span * step as f32 / 41.0;
                center
                    + egui::vec2(
                        angle.cos() * radius * radius_scale,
                        angle.sin() * radius * radius_scale,
                    )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(2.0_f32, if pending { COLOR_GOLD } else { COLOR_TEAL }),
        ));
    }
    for point in 0..12 {
        let angle = phase * 0.7 + point as f32 * std::f32::consts::TAU / 12.0;
        let orbit = radius * 1.45;
        painter.circle_filled(
            center + egui::vec2(angle.cos() * orbit, angle.sin() * orbit),
            if point % 3 == 0 { 2.2 } else { 1.2 },
            COLOR_TEAL,
        );
    }
}

impl eframe::App for JarvisDesktop {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();
        self.refresh_model_state();
        self.apply_preferences(context);
        self.orb_phase = (self.orb_phase + 0.028) % std::f32::consts::TAU;
        context.request_repaint_after(std::time::Duration::from_millis(45));

        let attach_shortcut = !self.pending
            && context.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::O));
        if attach_shortcut {
            self.add_attachment(context);
        }

        egui::TopBottomPanel::top("jarvis-hud-header")
            .exact_height(54.0)
            .frame(
                egui::Frame::new()
                    .fill(COLOR_PANEL)
                    .stroke(Stroke::new(1.0_f32, COLOR_TEAL_DIM)),
            )
            .show(context, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.heading(RichText::new("J . A . R . V . I . S").color(COLOR_TEAL));
                    ui.label(
                        RichText::new("LOCAL COMMAND INTERFACE")
                            .small()
                            .color(COLOR_TEAL_DIM),
                    );
                    ui.separator();
                    let model_color = if self.model_status == "MODEL HAZIR" {
                        COLOR_GREEN
                    } else {
                        COLOR_GOLD
                    };
                    ui.colored_label(model_color, RichText::new(&self.model_status).strong());
                    ui.label(RichText::new("VRAM 0").small().color(COLOR_TEAL_DIM));
                    ui.label(
                        RichText::new(format!("EK KUYRUĞU {}", self.queued_attachments.len()))
                            .small()
                            .color(COLOR_TEAL_DIM),
                    );
                });
            });

        egui::TopBottomPanel::bottom("jarvis-hud-status")
            .exact_height(31.0)
            .frame(
                egui::Frame::new()
                    .fill(COLOR_PANEL)
                    .stroke(Stroke::new(1.0_f32, COLOR_TEAL_DIM)),
            )
            .show(context, |ui| {
                ui.label(
                    RichText::new(format!("[ CORE LOG ]  {}", self.status))
                        .small()
                        .color(COLOR_TEXT),
                );
            });

        egui::SidePanel::left("jarvis-system-panel")
            .exact_width(270.0)
            .frame(
                egui::Frame::new()
                    .fill(COLOR_PANEL)
                    .stroke(Stroke::new(1.0_f32, COLOR_TEAL_DIM)),
            )
            .show(context, |ui| self.show_system_panel(ui));

        egui::SidePanel::right("jarvis-chat-panel")
            .resizable(true)
            .default_width(410.0)
            .min_width(310.0)
            .frame(
                egui::Frame::new()
                    .fill(COLOR_PANEL)
                    .stroke(Stroke::new(1.0_f32, COLOR_TEAL_DIM)),
            )
            .show(context, |ui| self.show_chat_console(ui, context));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(COLOR_BG))
            .show(context, |ui| self.show_hud(ui));
    }
}

fn message_matches_filter(
    message: &Message,
    search: &str,
    role_filter: Option<MessageRole>,
) -> bool {
    role_filter.is_none_or(|role| message.role == role)
        && turkish_search_fold(&message.content).contains(&turkish_search_fold(search))
}

fn turkish_search_fold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            'I' => folded.push('ı'),
            'İ' => folded.push('i'),
            _ => folded.extend(character.to_lowercase()),
        }
    }
    folded
}

fn load_preview(
    context: &egui::Context,
    attachment: &AttachmentRef,
) -> Result<TextureHandle, String> {
    let image = image::open(&attachment.canonical_path)
        .map_err(|error| format!("görsel decode edilemedi: {error}"))?
        .thumbnail(320, 240)
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Ok(context.load_texture(
        format!("attachment-preview-{}", attachment.attachment_id),
        color_image,
        TextureOptions::LINEAR,
    ))
}

fn ensure_local_model_server(provider: &LlamaServerProvider) -> String {
    let mut health_provider = provider.clone();
    health_provider.timeout_seconds = 1;
    if health_provider.runtime_state() == jarvis_core::ModelRuntimeState::Ready {
        return "Model RAM'de hazır • CPU-only • VRAM: 0".into();
    }
    match Command::new("systemctl")
        .args(["--user", "start", "jarvis-llama.service"])
        .status()
    {
        Ok(status) if status.success() => "Model sunucusu başlatılıyor…".into(),
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

fn notification_preview(content: &str) -> String {
    const LIMIT: usize = 180;
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = compact.chars().take(LIMIT).collect::<String>();
    if compact.chars().count() > LIMIT {
        preview.push('…');
    }
    preview
}

fn desktop_notification(
    task_state: TaskState,
    content: &str,
    notifications_enabled: bool,
) -> Option<DesktopNotification> {
    if !notifications_enabled || content.trim().is_empty() {
        return None;
    }
    let title = match task_state {
        TaskState::Completed => "JARVIS yanıtı hazır",
        TaskState::WaitingForUser => "JARVIS onayı bekliyor",
        TaskState::Failed | TaskState::Interrupted => "JARVIS işlem hatası",
        TaskState::Queued | TaskState::Running | TaskState::Cancelled => return None,
    };
    Some(DesktopNotification {
        title,
        content: content.into(),
    })
}

fn notify_desktop(title: &str, content: &str) {
    let preview = notification_preview(content);
    if preview.is_empty() {
        return;
    }
    let _ = Command::new("notify-send")
        .args([
            "--app-name=JARVIS",
            "--icon=dialog-information",
            "--expire-time=6000",
            title,
            &preview,
        ])
        .status();
}

fn main() -> eframe::Result<()> {
    let _instance_lock = match acquire_desktop_instance_lock(default_desktop_lock_path()) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("{error}");
            return Ok(());
        }
    };
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
    let initial_status = ensure_local_model_server(&provider);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([900.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "JARVIS",
        options,
        Box::new(move |creation_context| {
            Ok(Box::new(JarvisDesktop::new(
                runtime,
                provider,
                initial_status,
                &creation_context.egui_ctx,
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_desktop_instance_lock, desktop_notification, message_matches_filter,
        turkish_search_fold, Message, MessageRole,
    };
    use jarvis_core::TaskState;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_lock_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "jarvis-desktop-lock-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_nanos()
            ))
            .join("desktop.lock")
    }

    #[test]
    fn message_search_is_case_insensitive_for_turkish_i_variants_and_role_scoped() {
        let user_message = Message {
            role: MessageRole::User,
            content: "İstanbul'da bugün hava nasıl?".into(),
        };
        assert_eq!(turkish_search_fold("İSTANBUL"), "istanbul");
        assert!(message_matches_filter(&user_message, "istanbul", None));
        assert!(message_matches_filter(
            &user_message,
            "hava",
            Some(MessageRole::User)
        ));
        assert!(!message_matches_filter(
            &user_message,
            "hava",
            Some(MessageRole::Jarvis)
        ));
        assert!(!message_matches_filter(&user_message, "ankara", None));
    }

    #[test]
    fn desktop_lock_is_single_instance_and_recovers_from_a_stale_pid() {
        let path = temporary_lock_path();
        let first_lock = acquire_desktop_instance_lock(path.clone()).expect("first lock");
        assert!(acquire_desktop_instance_lock(path.clone()).is_err());
        drop(first_lock);
        assert!(!path.exists());

        fs::write(&path, "4294967295\n").expect("stale lock fixture");
        let recovered_lock =
            acquire_desktop_instance_lock(path.clone()).expect("stale lock recovery");
        drop(recovered_lock);
        assert!(!path.exists());
        fs::remove_dir(path.parent().expect("test lock parent")).expect("test cleanup");
    }

    #[test]
    fn desktop_lock_drop_never_removes_a_replaced_lock() {
        let path = temporary_lock_path();
        let lock = acquire_desktop_instance_lock(path.clone()).expect("first lock");
        fs::write(&path, "another-process\n").expect("replacement fixture");
        drop(lock);
        assert_eq!(fs::read_to_string(&path).unwrap(), "another-process\n");
        fs::remove_file(&path).expect("fixture cleanup");
        fs::remove_dir(path.parent().expect("test lock parent")).expect("test cleanup");
    }

    #[test]
    fn notifications_respect_the_user_preference_and_task_outcome() {
        assert!(desktop_notification(TaskState::Completed, "hazır", false).is_none());
        assert_eq!(
            desktop_notification(TaskState::Completed, "hazır", true)
                .expect("completed notification")
                .title,
            "JARVIS yanıtı hazır"
        );
        assert_eq!(
            desktop_notification(TaskState::WaitingForUser, "onay gerek", true)
                .expect("approval notification")
                .title,
            "JARVIS onayı bekliyor"
        );
        assert_eq!(
            desktop_notification(TaskState::Failed, "model erişilemedi", true)
                .expect("failure notification")
                .title,
            "JARVIS işlem hatası"
        );
        assert!(desktop_notification(TaskState::Cancelled, "iptal", true).is_none());
    }
}
