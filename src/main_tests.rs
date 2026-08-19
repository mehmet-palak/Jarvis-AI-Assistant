use super::{
    apply_history_key_scroll, apply_history_mouse_scroll, backspace_at_cursor, build_input_lines,
    cursor_visual_position, delete_forward_at_cursor, delete_previous_word, draft_rows,
    embedding_attach_decision, history_line_count, history_lines, input_view,
    insert_char_at_cursor, insert_pasted_text_at_cursor, is_clipboard_paste_shortcut,
    is_delete_previous_word_shortcut, is_forward_delete_shortcut, is_insert_newline_shortcut,
    is_kill_to_end_shortcut, is_kill_to_start_shortcut, is_move_to_end_shortcut,
    is_move_to_start_shortcut, is_primary_selection_paste, kill_to_end_from_cursor,
    kill_to_start_from_cursor, move_cursor_left, move_cursor_right, move_cursor_to_end,
    move_cursor_to_start, move_cursor_word_left, move_cursor_word_right,
    native_desktop_binary_path, notification_arguments, notification_preview,
    parse_remember_namespace_prefix, return_to_latest, should_clear_draft,
    should_close_tui_for_key, submit, try_notify_desktop, tui_exit_action, tui_notification, App,
    EmbeddingAttach, Message, MessageRole, TuiExitAction, WorkerReply,
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
    let (_, cursor_row, _column, window_start_row) = input_view("0123456789abcdefghij", 10, 5, 2);
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

/// F6 19 Ağustos 2026: hibrit RAG'ın embedding servisi ne boot'ta açılıyordu ne de uygulama
/// tarafından başlatılıyordu, dolayısıyla pratikte hiç çalışmıyordu — retrieval sessizce
/// FTS-only'ye düşüyordu. Bu test o davranışın geri gelmemesini garanti eder: indekslenmiş
/// belge varken servis kapalıysa artık başlatılmalı, ama hiç belge yokken hâlâ hiçbir maliyet
/// ödenmemeli (özgün "kullanılmayan servise RAM harcama" niyeti korunuyor).
#[test]
fn embedding_service_is_started_on_demand_only_when_rag_is_actually_in_use() {
    assert_eq!(
        embedding_attach_decision(true, 0),
        EmbeddingAttach::Attach,
        "servis zaten ayaktaysa doğrudan kullanılmalı"
    );
    assert_eq!(
        embedding_attach_decision(false, 0),
        EmbeddingAttach::Skip,
        "hiç indekslenmiş belge yokken RAM harcanmamalı"
    );
    assert_eq!(
        embedding_attach_decision(false, 12),
        EmbeddingAttach::StartThenAttach,
        "REGRESYON: indekslenmiş belge varken embedding servisi başlatılmalı, \
         yoksa hibrit retrieval sessizce hiç çalışmaz"
    );
    assert_eq!(embedding_attach_decision(true, 12), EmbeddingAttach::Attach);
}
