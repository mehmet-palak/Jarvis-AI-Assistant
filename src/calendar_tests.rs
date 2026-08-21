use super::*;

const SAMPLE_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nSUMMARY:Ekip toplantısı\r\nDTSTART:20260821T140000Z\r\nDTEND:20260821T150000Z\r\nLOCATION:Toplantı Odası 4\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Tüm gün etkinlik\r\nDTSTART;VALUE=DATE:20260822\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn parse_ics_reads_summary_time_and_location() {
    let events = parse_ics(SAMPLE_ICS);
    assert_eq!(events.len(), 2);

    let meeting = &events[0];
    assert_eq!(meeting.summary, "Ekip toplantısı");
    assert_eq!(meeting.location.as_deref(), Some("Toplantı Odası 4"));
    assert_eq!(
        meeting.start,
        EventDate {
            year: 2026,
            month: 8,
            day: 21,
            hour: Some(14),
            minute: Some(0)
        }
    );
    assert_eq!(meeting.end.unwrap().hour, Some(15));
    assert!(!meeting.start.is_all_day());

    let all_day = &events[1];
    assert_eq!(all_day.summary, "Tüm gün etkinlik");
    assert!(all_day.start.is_all_day());
    assert_eq!(
        all_day.start,
        EventDate {
            year: 2026,
            month: 8,
            day: 22,
            hour: None,
            minute: None
        }
    );
}

#[test]
fn parse_ics_skips_an_event_with_no_dtstart() {
    let ics = "BEGIN:VEVENT\nSUMMARY:Yerleştirilemez\nEND:VEVENT\n";
    assert!(parse_ics(ics).is_empty());
}

#[test]
fn unfolds_a_folded_summary_line() {
    // RFC 5545 satır katlaması: devam satırı boşlukla başlar ve öncekine eklenir.
    let ics = "BEGIN:VEVENT\r\nSUMMARY:Çok uzun bir baş\r\n lık devam ediyor\r\nDTSTART:20260901\r\nEND:VEVENT\r\n";
    let events = parse_ics(ics);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "Çok uzun bir başlık devam ediyor");
}

#[test]
fn parses_a_date_only_dtstart_as_all_day() {
    let date = parse_ics_date("20261231").expect("valid date");
    assert_eq!(
        date,
        EventDate {
            year: 2026,
            month: 12,
            day: 31,
            hour: None,
            minute: None
        }
    );
    assert!(date.is_all_day());
}

#[test]
fn parses_a_utc_datetime_and_strips_the_z() {
    let date = parse_ics_date("20260821T091500Z").expect("valid datetime");
    assert_eq!(
        date,
        EventDate {
            year: 2026,
            month: 8,
            day: 21,
            hour: Some(9),
            minute: Some(15)
        }
    );
}

#[test]
fn rejects_a_malformed_date() {
    assert!(parse_ics_date("2026-08-21").is_none());
    assert!(parse_ics_date("nonsense").is_none());
    assert!(parse_ics_date("20261301").is_none()); // 13. ay yok
}

#[test]
fn unescapes_ics_text_escapes() {
    assert_eq!(
        unescape_ics_text("Toplantı\\, sonra kahve"),
        "Toplantı, sonra kahve"
    );
    assert_eq!(unescape_ics_text("Satır1\\nSatır2"), "Satır1\nSatır2");
    assert_eq!(unescape_ics_text("A\\;B\\\\C"), "A;B\\C");
}

#[test]
fn sanitize_event_text_strips_control_bytes_and_caps_length() {
    // Kötü niyetli bir başlık: yeni satırlar + ANSI escape + kontrol baytı satırdan taşamamalı.
    let hostile = "İyi\nbaşlık\x1b[31m\x00 KÖTÜ";
    let clean = sanitize_event_text(hostile);
    assert!(!clean.contains('\n'));
    assert!(!clean.contains('\x1b'));
    assert!(!clean.contains('\x00'));
    // Her kontrol baytı (\n, \x1b, \x00) tek boşluğa iner; ardışık boşluklar tekleşir.
    assert_eq!(clean, "İyi başlık [31m KÖTÜ");

    let long = "a".repeat(MAX_EVENT_TEXT_CHARS + 50);
    assert_eq!(
        sanitize_event_text(&long).chars().count(),
        MAX_EVENT_TEXT_CHARS
    );
}

#[test]
fn events_on_day_filters_and_sorts_chronologically() {
    let events = parse_ics(
        "BEGIN:VEVENT\nSUMMARY:Öğleden sonra\nDTSTART:20260821T150000Z\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:Sabah\nDTSTART:20260821T090000Z\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:Başka gün\nDTSTART:20260825T090000Z\nEND:VEVENT\n",
    );
    let today = events_on_day(&events, 2026, 8, 21);
    assert_eq!(today.len(), 2);
    assert_eq!(today[0].summary, "Sabah"); // kronolojik: 09:00 önce
    assert_eq!(today[1].summary, "Öğleden sonra");
}

#[test]
fn all_day_multi_day_event_occurs_on_each_day_in_range() {
    // Tüm-gün DTEND dışlayıcıdır: 22-23 arası → 22'de var, 23'te yok.
    let ics = "BEGIN:VEVENT\nSUMMARY:İki gün\nDTSTART;VALUE=DATE:20260822\nDTEND;VALUE=DATE:20260823\nEND:VEVENT\n";
    let events = parse_ics(ics);
    assert!(events[0].occurs_on(2026, 8, 22));
    assert!(!events[0].occurs_on(2026, 8, 23));
}

#[test]
fn events_within_includes_today_and_next_days_but_not_past() {
    let events = parse_ics(
        "BEGIN:VEVENT\nSUMMARY:Dün\nDTSTART:20260820T090000Z\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:Bugün\nDTSTART:20260821T090000Z\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:3 gün sonra\nDTSTART:20260824T090000Z\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:10 gün sonra\nDTSTART:20260831T090000Z\nEND:VEVENT\n",
    );
    let within = events_within(&events, (2026, 8, 21), 7);
    let summaries: Vec<&str> = within.iter().map(|event| event.summary.as_str()).collect();
    assert_eq!(summaries, vec!["Bugün", "3 gün sonra"]); // dün dışarıda, 10 gün sonrası dışarıda
}

#[test]
fn civil_date_conversion_round_trips_and_matches_known_dates() {
    // Bilinen sabitler: 1970-01-01 = gün 0.
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    // Gidiş-dönüş: birçok gün için tutarlı olmalı.
    for days in [-10_000_i64, -1, 0, 1, 20_000, 20_321, 50_000] {
        let (year, month, day) = civil_from_days(days);
        assert_eq!(
            days_from_civil(year, month, day),
            days,
            "gün {days} gidiş-dönüş bozuldu"
        );
    }
    // Artık yıl sınırı: 2024-02-29 gerçek bir gün.
    let leap = days_from_civil(2024, 2, 29);
    assert_eq!(civil_from_days(leap), (2024, 2, 29));
}

#[test]
fn local_ics_provider_reads_a_real_file() {
    let dir = std::env::temp_dir().join(format!(
        "jarvis-calendar-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("takvim.ics");
    std::fs::write(&path, SAMPLE_ICS).unwrap();

    let provider = LocalIcsCalendarProvider::new(path);
    let events = provider.events().expect("dosya okunur ve ayrıştırılır");
    assert_eq!(events.len(), 2);

    std::fs::remove_dir_all(&dir).expect("test cleanup");
}

#[test]
fn local_ics_provider_reports_a_missing_file_as_an_error_not_a_panic() {
    let provider = LocalIcsCalendarProvider::new(PathBuf::from("/kesinlikle/yok/takvim.ics"));
    assert!(provider.events().is_err());
}
