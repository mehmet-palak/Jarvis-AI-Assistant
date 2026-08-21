//! F8 "yerel entegrasyonlar" — tamamen yerel, ağsız bir takvim entegrasyonu. JARVIS kullanıcının
//! bilgisayarındaki bir iCalendar (`.ics`) dosyasını **yalnız okur** ve açılış brifinginde /
//! `/takvim` komutunda etkinlikleri gösterir. Hava durumunun (`weather.rs`) aksine bu özellik
//! İnternet'e hiç çıkmaz; profil dosyaları (`profile_files.rs`) gibi kullanıcının kendi düzenlediği
//! bir dosyayı okumaya dayanır.
//!
//! **"Data, instruction değil" ilkesi:** bir `.ics` dosyası başka birinin gönderdiği bir davetten
//! (external invite) gelmiş olabilir — yani etkinlik başlığı/konumu potansiyel olarak güvenilmez
//! metindir. Modele ya da brifinge çıkarken `sanitize_event_text` ile kontrol baytları atılır ve
//! uzunluk sınırlanır (pentest kanıt-temizliği ile aynı disiplin); böylece kötü niyetli bir başlık
//! satırdan taşamaz veya biçim enjekte edemez.
//!
//! **Bilinçli sınırlar (dürüstçe):** bu ayrıştırıcı yaygın `VEVENT` alanlarını (SUMMARY, DTSTART,
//! DTEND, LOCATION) kapsar; **RRULE (tekrarlayan etkinlikler), VTIMEZONE ve tam saat-dilimi
//! dönüşümü desteklenmez** — tarihler naif (UTC gün sınırı) yorumlanır. Kişisel bir günlük asistan
//! için yeterli; tam bir takvim istemcisi değil.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Tek bir `.ics` dosyasından okunacak azami bayt — bir kullanıcı yanlışlıkla devasa bir dışa
/// aktarma dosyası koyarsa belleği tek başına tüketmesin diye (`Read::take` ile sınırlı, F7'nin
/// replay/JS çekimlerindeki aynı OOM-sınırı deseni).
pub const MAX_ICS_BYTES: u64 = 4 * 1024 * 1024;

/// Brifinge/komuta çıkarken tek bir etkinlik metninin (başlık/konum) azami uzunluğu.
pub const MAX_EVENT_TEXT_CHARS: usize = 120;

/// Bir takvim etkinliğinin tarihi. `hour`/`minute` yoksa bu "tüm gün" (all-day) bir tarihtir
/// (ICS'te `VALUE=DATE` ya da `YYYYMMDD` biçimi).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
}

impl EventDate {
    /// Yalnız takvim gününü (saat yok) veren sıralanabilir bir ordinal — "aynı gün mü", "şu aralıkta
    /// mı" karşılaştırmaları için.
    pub fn day_ordinal(&self) -> i64 {
        self.year as i64 * 10_000 + self.month as i64 * 100 + self.day as i64
    }

    /// Gün + saat/dakika içeren, kronolojik sıralama için ordinal.
    pub fn sort_ordinal(&self) -> i64 {
        self.day_ordinal() * 10_000
            + self.hour.unwrap_or(0) as i64 * 100
            + self.minute.unwrap_or(0) as i64
    }

    pub fn is_all_day(&self) -> bool {
        self.hour.is_none()
    }
}

impl fmt::Display for EventDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let (Some(h), Some(m)) = (self.hour, self.minute) {
            write!(
                f,
                "{:04}-{:02}-{:02} {:02}:{:02}",
                self.year, self.month, self.day, h, m
            )
        } else {
            write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
        }
    }
}

/// Ayrıştırılmış tek bir takvim etkinliği. `summary`/`location` zaten `sanitize_event_text`'ten
/// geçmiş (temiz, sınırlı) metindir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub summary: String,
    pub location: Option<String>,
    pub start: EventDate,
    pub end: Option<EventDate>,
}

impl CalendarEvent {
    /// Etkinlik verilen günde (yıl/ay/gün) mi gerçekleşiyor? Saatli etkinlikler için başlangıç günü
    /// eşleşmeli; tüm-gün etkinlikleri için çok-günlük bir aralığı da (`[start, end)`) kapsar.
    pub fn occurs_on(&self, year: i32, month: u32, day: u32) -> bool {
        let target = year as i64 * 10_000 + month as i64 * 100 + day as i64;
        if self.start.day_ordinal() == target {
            return true;
        }
        if self.start.is_all_day() {
            if let Some(end) = self.end {
                // ICS'te tüm-gün DTEND dışlayıcıdır (etkinliğin bittiği günün ertesi).
                return self.start.day_ordinal() <= target && target < end.day_ordinal();
            }
        }
        false
    }
}

/// Bir takvim sağlayıcısı — hava durumundaki `WeatherProvider` ile aynı desen. Şu an tek somut
/// gerçekleştirme yerel bir `.ics` dosyası; ileride (F10, ağ kararı verilince) başka sağlayıcılar
/// aynı trait'i gerçekleştirebilir.
pub trait CalendarProvider: fmt::Debug + Send + Sync {
    fn events(&self) -> Result<Vec<CalendarEvent>, String>;
}

/// Yerel bir `.ics` dosyasını okuyan sağlayıcı. İnternet'e çıkmaz; dosya boyutu `MAX_ICS_BYTES` ile
/// sınırlı okunur.
#[derive(Debug, Clone)]
pub struct LocalIcsCalendarProvider {
    pub path: PathBuf,
}

impl LocalIcsCalendarProvider {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl CalendarProvider for LocalIcsCalendarProvider {
    fn events(&self) -> Result<Vec<CalendarEvent>, String> {
        let file = fs::File::open(&self.path).map_err(|error| {
            format!(
                "takvim dosyası açılamadı ({}): {error}",
                self.path.display()
            )
        })?;
        let mut content = String::new();
        file.take(MAX_ICS_BYTES)
            .read_to_string(&mut content)
            .map_err(|error| format!("takvim dosyası okunamadı: {error}"))?;
        Ok(parse_ics(&content))
    }
}

/// Bir `.ics` metnini ayrıştırır — saf fonksiyon, dosya/ağ yok, bu yüzden doğrudan test edilebilir
/// (`weather.rs`'in `parse_open_meteo_response`'u ile aynı gerekçe). DTSTART'ı ve SUMMARY'si
/// çözülemeyen `VEVENT` blokları atlanır (yerleştirilemeyen bir etkinliği uydurmaktansa atlamak).
pub fn parse_ics(content: &str) -> Vec<CalendarEvent> {
    let lines = unfold_ics_lines(content);
    let mut events = Vec::new();
    let mut in_event = false;
    let mut summary: Option<String> = None;
    let mut location: Option<String> = None;
    let mut start: Option<EventDate> = None;
    let mut end: Option<EventDate> = None;

    for line in lines {
        let upper = line.to_ascii_uppercase();
        if upper == "BEGIN:VEVENT" {
            in_event = true;
            summary = None;
            location = None;
            start = None;
            end = None;
            continue;
        }
        if upper == "END:VEVENT" {
            if let (Some(s), Some(st)) = (summary.take(), start) {
                events.push(CalendarEvent {
                    summary: s,
                    location: location.take(),
                    start: st,
                    end,
                });
            }
            in_event = false;
            continue;
        }
        if !in_event {
            continue;
        }
        let Some((name, value)) = split_ics_property(&line) else {
            continue;
        };
        match name.as_str() {
            "SUMMARY" => summary = Some(sanitize_event_text(&unescape_ics_text(value))),
            "LOCATION" => location = Some(sanitize_event_text(&unescape_ics_text(value))),
            "DTSTART" => start = parse_ics_date(value),
            "DTEND" => end = parse_ics_date(value),
            _ => {}
        }
    }
    events
}

/// ICS satır-katlamasını (line folding) çözer: RFC 5545'e göre uzun satırlar `CRLF` + boşluk/tab ile
/// bölünür; devam satırları (boşluk veya tab ile başlayan) bir öncekine eklenir.
fn unfold_ics_lines(content: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for raw in content.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = result.last_mut() {
                last.push_str(&line[1..]);
                continue;
            }
        }
        result.push(line.to_string());
    }
    result
}

/// Bir ICS özellik satırını `(BÜYÜK_AD, değer)`'e ayırır. Ad, ilk `;` (parametre) ya da `:`
/// (değer) karakterine kadardır; parametreler atlanır. `DTSTART;VALUE=DATE:20260822` →
/// `("DTSTART", "20260822")`.
fn split_ics_property(line: &str) -> Option<(String, &str)> {
    let colon = line.find(':')?;
    let (head, rest) = line.split_at(colon);
    let value = &rest[1..];
    let name = head.split(';').next().unwrap_or(head);
    if name.is_empty() {
        return None;
    }
    Some((name.trim().to_ascii_uppercase(), value))
}

/// Bir ICS tarih/tarih-saat değerini ayrıştırır. Biçimler: `YYYYMMDD` (tüm gün), `YYYYMMDDTHHMMSS`
/// ve `YYYYMMDDTHHMMSSZ` (UTC). Saat dilimi dönüşümü yapılmaz (naif); tanınmayan biçim `None`.
fn parse_ics_date(raw: &str) -> Option<EventDate> {
    let value = raw.trim().trim_end_matches('Z');
    let (date_part, time_part) = match value.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (value, None),
    };
    if date_part.len() != 8 || !date_part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year: i32 = date_part[0..4].parse().ok()?;
    let month: u32 = date_part[4..6].parse().ok()?;
    let day: u32 = date_part[6..8].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (hour, minute) = match time_part {
        Some(time) if time.len() >= 4 && time[0..4].bytes().all(|byte| byte.is_ascii_digit()) => {
            (time[0..2].parse().ok(), time[2..4].parse().ok())
        }
        _ => (None, None),
    };
    Some(EventDate {
        year,
        month,
        day,
        hour,
        minute,
    })
}

/// ICS TEXT kaçışlarını çözer: `\n`/`\N` → yeni satır, `\,` → `,`, `\;` → `;`, `\\` → `\`.
fn unescape_ics_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(character);
        }
    }
    out
}

/// Modele/brifinge çıkacak bir etkinlik metnini temizler: kontrol baytları (yeni satır, tab dahil)
/// tek boşluğa indirilir, ardışık boşluklar tekleştirilir, uzunluk `MAX_EVENT_TEXT_CHARS`'a
/// kırpılır. Kötü niyetli bir `.ics` başlığının satırdan taşmasını / biçim enjekte etmesini
/// engeller.
pub fn sanitize_event_text(raw: &str) -> String {
    let mut collapsed = String::with_capacity(raw.len());
    let mut last_was_space = false;
    for character in raw.chars() {
        if character.is_control() || character.is_whitespace() {
            if !last_was_space {
                collapsed.push(' ');
                last_was_space = true;
            }
        } else {
            collapsed.push(character);
            last_was_space = false;
        }
    }
    let trimmed = collapsed.trim();
    trimmed.chars().take(MAX_EVENT_TEXT_CHARS).collect()
}

/// Verilen etkinlikleri belirli bir günde (yıl/ay/gün) gerçekleşenlere göre süzer ve kronolojik
/// sıralar.
pub fn events_on_day(
    events: &[CalendarEvent],
    year: i32,
    month: u32,
    day: u32,
) -> Vec<CalendarEvent> {
    let mut matched: Vec<CalendarEvent> = events
        .iter()
        .filter(|event| event.occurs_on(year, month, day))
        .cloned()
        .collect();
    matched.sort_by_key(|event| event.start.sort_ordinal());
    matched
}

/// Bugünden itibaren `within_days` gün içindeki (bugün dahil) etkinlikleri kronolojik sıralar.
/// `today` çağıran tarafça verilir (saf fonksiyon kalsın diye).
pub fn events_within(
    events: &[CalendarEvent],
    today: (i32, u32, u32),
    within_days: i64,
) -> Vec<CalendarEvent> {
    let today_ordinal = today.0 as i64 * 10_000 + today.1 as i64 * 100 + today.2 as i64;
    let today_days = days_from_civil(today.0, today.1, today.2);
    let mut matched: Vec<CalendarEvent> = events
        .iter()
        .filter(|event| {
            let start = event.start;
            if start.day_ordinal() < today_ordinal {
                return false;
            }
            let start_days = days_from_civil(start.year, start.month, start.day);
            start_days - today_days <= within_days
        })
        .cloned()
        .collect();
    matched.sort_by_key(|event| event.start.sort_ordinal());
    matched
}

/// Yerel takvim dosyasının varsayılan yolu: `JARVIS_CALENDAR_PATH` ortam değişkeni varsa onu,
/// yoksa `~/.config/jarvis/calendar.ics` (`profile_files.rs`'in `default_profile_files_dir`'i ile
/// aynı config-kök arama mantığı). Dosyanın var olup olmaması burada kontrol edilmez — çağıran
/// taraf (main) yalnız varsa takar.
pub fn default_calendar_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("JARVIS_CALENDAR_PATH") {
        return Some(PathBuf::from(explicit));
    }
    let config_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_root.join("jarvis").join("calendar.ics"))
}

/// Sistem saatinden bugünün takvim tarihini (UTC) döndürür. Saat dilimi dönüşümü yapılmaz —
/// modül başındaki dürüst sınır.
pub fn today_utc() -> (i32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    civil_from_days(days)
}

/// Epoch'tan (1970-01-01) itibaren gün sayısını takvim tarihine çevirir — Howard Hinnant'ın
/// standart `civil_from_days` algoritması, harici bir tarih kütüphanesi gerektirmeden.
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month, day)
}

/// `civil_from_days`'in tersi — takvim tarihinden epoch gün sayısı. `events_within`'in gün farkı
/// hesabı için.
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = if month <= 2 {
        year as i64 - 1
    } else {
        year as i64
    };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400; // [0, 399]
    let doy =
        (153 * (if month > 2 {
            month as i64 - 3
        } else {
            month as i64 + 9
        }) + 2)
            / 5
            + day as i64
            - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
#[path = "calendar_tests.rs"]
mod tests;
