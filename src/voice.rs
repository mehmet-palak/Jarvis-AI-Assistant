//! F5 ses girişi sözleşmeleri — transkript kuyruğu ve ham ses saklama politikası.
//!
//! Bu modül bilinçli olarak **ses yakalamayı içermez**. Mikrofon erişimi ve konuşma tanıma ayrı
//! bir bağımlılık ve indirilmiş bir STT modeli gerektiriyor; onlar gelene kadar bile bu katmanın
//! doğru olması gerekiyor, çünkü F5'in gizlilik ve onay kuralları *sesin nereye gittiğiyle*
//! ilgili — nasıl yakalandığıyla değil. Yakalama eklendiğinde buradaki sözleşmeleri kullanacak,
//! kendi paralel kurallarını kurmayacak.
//!
//! İki kural burada yapısal hale getiriliyor:
//!
//! 1. **Transkript, kullanıcı onaylamadan istek olmaz.** Konuşma tanıma yanılabilir; yanlış
//!    anlaşılmış bir cümlenin sessizce JARVIS'e gitmesi, sesli kullanımın en büyük riski.
//!    `VoiceTranscript` bu yüzden ayrı bir tip: onaylanmamış bir transkriptten `Request`
//!    üretebilecek bir kod yolu yok.
//! 2. **Ham ses varsayılan olarak saklanmaz.** Saklanacaksa nerede ve ne zaman silineceği
//!    görünür olmalı — "belki bir yerde duruyor" durumu kabul edilemez.

use crate::{InputType, Request};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Ham ses kaydına ne olacağı. Varsayılan `DiscardImmediately`: transkript üretildiği anda ses
/// silinir. Bu, F5 planındaki "ham ses varsayılan olarak kalıcı değil" şartının kodda karşılığı —
/// bir ayar dosyasına yazılmış bir tercih değil, tipin kendi varsayılanı.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecordingRetention {
    /// Transkript çıkarıldığı anda ham ses silinir. Varsayılan.
    #[default]
    DiscardImmediately,
    /// Kullanıcı açıkça saklamayı seçti. Konum ve silinme zamanı kullanıcıya gösterilebilir
    /// olmak zorunda — bu yüzden ikisi de tipin içinde, yorumda değil.
    KeepUntil {
        path: PathBuf,
        delete_after_epoch: u64,
    },
}

impl RecordingRetention {
    /// Kullanıcıya gösterilecek dürüst tek satır. Sesli kullanımda kullanıcı ekrana bakmıyor
    /// olabilir, bu yüzden metin kısa ve kesin: "belki", "genellikle" yok.
    pub fn user_visible_summary(&self) -> String {
        match self {
            RecordingRetention::DiscardImmediately => {
                "Ham ses kaydı saklanmıyor — metne çevrildiği anda siliniyor.".into()
            }
            RecordingRetention::KeepUntil {
                path,
                delete_after_epoch,
            } => format!(
                "Ham ses kaydı burada saklanıyor: {} — {} epoch zamanında silinecek.",
                path.display(),
                delete_after_epoch
            ),
        }
    }

    pub fn keeps_audio(&self) -> bool {
        matches!(self, RecordingRetention::KeepUntil { .. })
    }
}

/// Kullanıcının göndermeden önce görüp düzeltebileceği bir transkript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceTranscript {
    pub transcript_id: String,
    /// Konuşma tanımanın ürettiği metin; kullanıcı düzenlediyse düzeltilmiş hali.
    pub text: String,
    /// Kayıt süresi (ms). Yalnız kullanıcıya gösterim için; boş/gürültü kayıtlarını ayırt etmede
    /// de işe yarar.
    pub duration_ms: u32,
    pub retention: RecordingRetention,
    /// Kullanıcı bu metni göndermeyi açıkça onayladı mı.
    pub confirmed: bool,
}

/// Bir transkriptin neden gönderilemeyeceği. `Result<_, String>` yerine tipli, çünkü arayüzün
/// her durum için farklı bir şey yapması gerekiyor: boş kayıtta sessizce iptal, onaysızda onay
/// isteme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRejection {
    /// Sessizlik veya gürültü — çevrilecek konuşma yok.
    Empty,
    /// Metin var ama kullanıcı henüz "gönder" demedi.
    NotConfirmed,
}

impl TranscriptRejection {
    pub fn user_message(&self) -> &'static str {
        match self {
            TranscriptRejection::Empty => {
                "Kayıtta anlaşılır konuşma yok — hiçbir şey gönderilmedi."
            }
            TranscriptRejection::NotConfirmed => {
                "Metni gözden geçir ve göndermek istediğinde onayla."
            }
        }
    }
}

/// Onaylanmış bir transkripti normal `Request` hattına sokar.
///
/// Dönen istek `InputType::Voice` taşır ve bu bilgi *kaybolmaz*: F5'in sesli onay sınırı
/// (`approval_channel_requirement`) tam olarak bu alana bakıyor. Sesli bir isteği CLI'dan
/// gelmiş gibi işaretlemek, o güvenlik sınırını sessizce devre dışı bırakırdı.
pub fn transcript_into_request(
    transcript: &VoiceTranscript,
    request_id: &str,
) -> Result<Request, TranscriptRejection> {
    if transcript.text.trim().is_empty() {
        return Err(TranscriptRejection::Empty);
    }
    if !transcript.confirmed {
        return Err(TranscriptRejection::NotConfirmed);
    }
    Ok(Request {
        schema_version: 1,
        request_id: request_id.to_string(),
        input_type: InputType::Voice,
        content: transcript.text.trim().to_string(),
        attachments: Vec::new(),
    })
}

/// Ses yakalama, konuşma tanıma ve seslendirme için varsayılan yollar. Sabit değil,
/// yapılandırılabilir: ADR-0007 bunların hepsini alt süreç olarak çağırmayı seçiyor, ama nerede
/// durdukları makineye göre değişir.
#[derive(Debug, Clone)]
pub struct VoiceStackPaths {
    pub whisper_cli: PathBuf,
    pub whisper_model: PathBuf,
    pub piper_binary: PathBuf,
    pub piper_voice: PathBuf,
    /// Geçici WAV dosyalarının yazılacağı dizin.
    pub scratch_dir: PathBuf,
}

impl VoiceStackPaths {
    /// Bu makinedeki kurulum. `JARVIS_VOICE_*` ortam değişkenleriyle geçersiz kılınabilir, çünkü
    /// bir eval/CI koşumu farklı bir yerde kurulu olabilir.
    pub fn local_default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/mehmet".into());
        let from_env = |key: &str, fallback: PathBuf| {
            std::env::var(key).map(PathBuf::from).unwrap_or(fallback)
        };
        Self {
            whisper_cli: from_env(
                "JARVIS_WHISPER_CLI",
                PathBuf::from(&home)
                    .join("jarvis/third_party/whisper.cpp/build-cpu/bin/whisper-cli"),
            ),
            whisper_model: from_env(
                "JARVIS_WHISPER_MODEL",
                PathBuf::from(&home).join("jarvis/models/whisper/ggml-small-q5_1.bin"),
            ),
            piper_binary: from_env(
                "JARVIS_PIPER_BINARY",
                PathBuf::from(&home).join("jarvis/models/piper/piper/piper"),
            ),
            piper_voice: from_env(
                "JARVIS_PIPER_VOICE",
                PathBuf::from(&home).join("jarvis/models/piper/tr_TR-dfki-medium.onnx"),
            ),
            scratch_dir: from_env(
                "JARVIS_VOICE_SCRATCH",
                std::env::temp_dir().join("jarvis-voice"),
            ),
        }
    }

    /// Hangi parçaların gerçekten kurulu olduğu. Eksik bir parça hata değil, bir *durum*:
    /// ses yığını isteğe bağlı, kurulu değilse JARVIS metin modunda çalışmaya devam eder.
    pub fn availability(&self) -> VoiceStackAvailability {
        VoiceStackAvailability {
            capture: which_exists("pw-record"),
            transcription: self.whisper_cli.is_file() && self.whisper_model.is_file(),
            speech: self.piper_binary.is_file() && self.piper_voice.is_file(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceStackAvailability {
    pub capture: bool,
    pub transcription: bool,
    pub speech: bool,
}

impl VoiceStackAvailability {
    /// Sesli giriş için ikisi de gerekir; yalnız biri varsa özellik kullanılamaz ve bunu
    /// kullanıcıya açıkça söylemek gerekir — sessizce yarım çalışmak en kötüsü.
    pub fn voice_input_ready(&self) -> bool {
        self.capture && self.transcription
    }

    pub fn missing_summary(&self) -> Option<String> {
        let mut missing = Vec::new();
        if !self.capture {
            missing.push("ses yakalama (pw-record)");
        }
        if !self.transcription {
            missing.push("konuşma tanıma (whisper modeli)");
        }
        if missing.is_empty() {
            None
        } else {
            Some(format!("Eksik: {}", missing.join(", ")))
        }
    }
}

fn which_exists(binary: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {binary}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Süren bir kayıt. `stop()` çağrılana kadar `pw-record` arka planda yazmaya devam eder.
///
/// Kaydın *durdurulması* `SIGINT` ile yapılıyor, `SIGKILL` ile değil: `pw-record` WAV başlığını
/// dosya kapanırken tamamlıyor, öldürülürse geriye bozuk bir dosya kalır.
#[derive(Debug)]
pub struct VoiceRecording {
    child: Child,
    path: PathBuf,
}

impl VoiceRecording {
    /// 16 kHz mono s16 — whisper'ın istediği format. Başka bir şey seçmek her transkripsiyonda
    /// gereksiz bir yeniden örnekleme demek olurdu (ADR-0007).
    pub fn start(paths: &VoiceStackPaths, recording_id: &str) -> Result<Self, String> {
        std::fs::create_dir_all(&paths.scratch_dir)
            .map_err(|error| format!("ses geçici dizini oluşturulamadı: {error}"))?;
        let path = paths.scratch_dir.join(format!("{recording_id}.wav"));
        let child = Command::new("pw-record")
            .args(["--rate", "16000", "--channels", "1", "--format", "s16"])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("kayıt başlatılamadı (pw-record): {error}"))?;
        Ok(Self { child, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Kaydı düzgün kapatır ve dosya yolunu döndürür.
    pub fn stop(mut self) -> Result<PathBuf, String> {
        // SIGINT: pw-record'un WAV başlığını tamamlamasına izin verir.
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGINT);
        }
        self.child
            .wait()
            .map_err(|error| format!("kayıt durdurulamadı: {error}"))?;
        if !self.path.is_file() {
            return Err("kayıt dosyası oluşmadı".into());
        }
        Ok(self.path.clone())
    }

    /// Kullanıcı iptal etti: kaydı durdur ve dosyayı **sil**. Gizlilik varsayılanı burada da
    /// geçerli — iptal edilmiş bir kayıt diskte kalmaz.
    pub fn cancel(mut self) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGINT);
        }
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bir WAV dosyasını metne çevirir ve `RecordingRetention` politikasını uygular.
///
/// Saklama politikası burada uygulanıyor, çağıranın insafına bırakılmıyor: transkript üretilir
/// üretilmez, `DiscardImmediately` ise dosya siliniyor. Politikayı çağırana bırakmak, bir çağrı
/// yolunun onu unutmasıyla ham sesin sessizce diskte kalması demek olurdu.
pub fn transcribe_recording(
    paths: &VoiceStackPaths,
    wav_path: &Path,
    retention: RecordingRetention,
    transcript_id: &str,
) -> Result<VoiceTranscript, String> {
    let output = Command::new(&paths.whisper_cli)
        .arg("-m")
        .arg(&paths.whisper_model)
        .arg("-f")
        .arg(wav_path)
        .args(["-l", "tr", "-nt", "-t", "8", "--no-prints"])
        .output()
        .map_err(|error| format!("konuşma tanıma çalıştırılamadı: {error}"))?;
    if !output.status.success() {
        if !retention.keeps_audio() {
            let _ = std::fs::remove_file(wav_path);
        }
        return Err(format!(
            "konuşma tanıma başarısız: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if !retention.keeps_audio() {
        let _ = std::fs::remove_file(wav_path);
    }

    Ok(VoiceTranscript {
        transcript_id: transcript_id.to_string(),
        text,
        duration_ms: 0,
        retention,
        confirmed: false,
    })
}

/// Metni seslendirip bir WAV dosyası üretir. Oynatma çağıranın işi — bu katman yalnız sesi
/// üretir, böylece aynı fonksiyon hem oynatma hem dosyaya kaydetme için kullanılabilir.
pub fn synthesize_speech(
    paths: &VoiceStackPaths,
    text: &str,
    output_path: &Path,
) -> Result<(), String> {
    synthesize_speech_with(paths, text, output_path, &SpeechSettings::default())
}

/// Hız ayarıyla seslendirme. `synthesize_speech` bunun varsayılan ayarlı hâli.
pub fn synthesize_speech_with(
    paths: &VoiceStackPaths,
    text: &str,
    output_path: &Path,
    settings: &SpeechSettings,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("seslendirilecek metin boş".into());
    }
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("ses çıktı dizini oluşturulamadı: {error}"))?;
    }
    // Piper kendi paylaşılan kütüphanelerini yanında taşıyor; LD_LIBRARY_PATH olmadan çalışmaz.
    let piper_dir = paths
        .piper_binary
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut child = Command::new(&paths.piper_binary)
        .arg("--model")
        .arg(&paths.piper_voice)
        .arg("--output_file")
        .arg(output_path)
        .arg("--length_scale")
        .arg(format!("{:.3}", settings.piper_length_scale()))
        .env("LD_LIBRARY_PATH", &piper_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("seslendirme çalıştırılamadı: {error}"))?;
    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "seslendirme girdisi açılamadı".to_string())?;
        stdin
            .write_all(text.trim().as_bytes())
            .map_err(|error| format!("seslendirme girdisi yazılamadı: {error}"))?;
    }
    let status = child
        .wait()
        .map_err(|error| format!("seslendirme tamamlanamadı: {error}"))?;
    if !status.success() {
        return Err("seslendirme başarısız".into());
    }
    Ok(())
}

/// Kaydın son parçasındaki ses seviyesi (0.0–1.0 arası RMS).
///
/// F5 "ses seviyesi/VAD göstergesi": kullanıcı kayıt sırasında mikrofonun gerçekten ses alıp
/// almadığını görebilmeli. Sessiz bir kaydın 20 saniye sonra "hiçbir şey duyulmadı" ile
/// bitmesi, en sinir bozucu başarısızlık biçimi.
///
/// Yalnız dosyanın *sonunu* okur (son ~0.25 saniye): büyüyen bir kaydı her yenilemede baştan
/// okumak, kayıt uzadıkça göstergeyi yavaşlatırdı.
pub fn recent_audio_level(wav_path: &Path) -> f32 {
    // 16 kHz mono s16 → saniyede 32000 bayt; 0.25 saniye = 8000 bayt.
    const TAIL_BYTES: u64 = 8_000;

    let Ok(mut file) = std::fs::File::open(wav_path) else {
        return 0.0;
    };
    let Some(audio_start) = wav_data_offset(&mut file) else {
        return 0.0;
    };
    let Ok(metadata) = file.metadata() else {
        return 0.0;
    };
    let len = metadata.len();
    if len <= audio_start {
        return 0.0;
    }
    let read_len = (len - audio_start).min(TAIL_BYTES);
    let start = len - read_len;

    use std::io::{Read, Seek, SeekFrom};
    if file.seek(SeekFrom::Start(start)).is_err() {
        return 0.0;
    }
    let mut buffer = vec![0u8; read_len as usize];
    if file.read_exact(&mut buffer).is_err() {
        return 0.0;
    }

    let mut sum_squares = 0.0f64;
    let mut count = 0u32;
    for chunk in buffer.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as f64 / i16::MAX as f64;
        sum_squares += sample * sample;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    ((sum_squares / count as f64).sqrt() as f32).clamp(0.0, 1.0)
}

/// WAV `data` chunk'ının başladığı bayt konumu.
///
/// Sabit 44 varsaymak yerine gerçekten aranıyor: `pw-record` ve Piper kanonik 44 baytlık başlık
/// üretiyor ama WAV biçimi `data` öncesinde başka chunk'lara (`LIST`, `fact`) izin veriyor.
/// Yanlış konumdan okumak, başlık baytlarını ses örneği sanıp **sahte bir seviye** göstermek
/// demek olurdu — göstergenin hiç çalışmamasından daha kötü, çünkü kullanıcı ona güvenir.
fn wav_data_offset(file: &mut std::fs::File) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    let mut offset = 12u64;
    // Kötü/kesik bir dosyada sonsuza dek dönmemek için chunk sayısı sınırlı.
    for _ in 0..16 {
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut chunk = [0u8; 8];
        if file.read_exact(&mut chunk).is_err() {
            return None;
        }
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as u64;
        if &chunk[0..4] == b"data" {
            return Some(offset + 8);
        }
        // Chunk'lar çift bayta hizalı; tek boyutlu bir chunk bir dolgu baytı taşır.
        offset += 8 + size + (size % 2);
    }
    None
}

/// Ses seviyesini ekranda gösterilecek bir çubuğa çevirir. Metin olarak da okunabilir olması
/// önemli: ekran okuyucu kullanan biri için `level_description` var (erişilebilirlik maddesi).
pub fn level_meter(level: f32, width: usize) -> String {
    // Konuşma seviyeleri RMS olarak düşüktür (0.02–0.2 tipik); doğrusal ölçek göstergeyi hep
    // boş gösterirdi, bu yüzden karekök ile açılıyor.
    let scaled = (level.sqrt() * 2.2).clamp(0.0, 1.0);
    let filled = (scaled * width as f32).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// Ses seviyesinin metin karşılığı — ekran okuyucular ve görsel gösterge okunamadığı durumlar
/// için. F5 erişilebilirlik maddesi: her görsel bilginin metin eşdeğeri olmalı.
pub fn level_description(level: f32) -> &'static str {
    match level {
        l if l < 0.005 => "sessiz",
        l if l < 0.03 => "çok kısık",
        l if l < 0.10 => "normal",
        l if l < 0.25 => "yüksek",
        _ => "çok yüksek",
    }
}

/// Seslendirme davranışı. Hepsi kullanıcı tercihi; varsayılan **sessiz** — JARVIS kendiliğinden
/// konuşmaya başlamaz, kullanıcı açıkça istemedikçe.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechSettings {
    /// Her yanıt bittiğinde otomatik seslendirilsin mi. Varsayılan `false` (opt-in).
    pub auto_play: bool,
    /// Konuşma hızı çarpanı. Piper'da `--length_scale` ters çalışır (küçük = hızlı), dönüşüm
    /// `piper_length_scale()` içinde yapılıyor ki kullanıcı sezgisel yönü görsün.
    pub speed: f32,
    /// Sessiz mod: açıkken `/speak` dahil hiçbir ses çıkmaz. Otomatik oynatmayı kapatmaktan
    /// farklı — bu, "şu an hiç ses istemiyorum" durumu (toplantı, gece).
    pub muted: bool,
    /// Konuşma metne çevrildikten sonra gönderilmeden önce gözden geçirilsin mi.
    ///
    /// Varsayılan `false`: kullanıcı konuşup bıraktığında istek doğrudan gidiyor ve yanıt sesli
    /// dönüyor — konuşmanın doğal akışı bu, araya Enter koymak onu kesiyor. `true` yapıldığında
    /// transkript taslakta bekler; konuşma tanımanın yanıldığı durumlarda (gürültülü ortam,
    /// teknik terimler) düzeltme şansı verir.
    pub review_transcript: bool,
}

impl Default for SpeechSettings {
    fn default() -> Self {
        Self {
            auto_play: false,
            speed: 1.0,
            muted: false,
            review_transcript: false,
        }
    }
}

impl SpeechSettings {
    pub const MIN_SPEED: f32 = 0.5;
    pub const MAX_SPEED: f32 = 2.0;

    /// Piper'ın `--length_scale` değeri. Kullanıcının gördüğü "hız" ile ters orantılı.
    pub fn piper_length_scale(&self) -> f32 {
        1.0 / self.speed.clamp(Self::MIN_SPEED, Self::MAX_SPEED)
    }

    /// Bir yanıt tamamlandığında ses çıkmalı mı. İki ayrı tercihin birleşimi, çünkü ikisi
    /// farklı şeyler: `auto_play` kalıcı bir alışkanlık, `muted` anlık bir durum.
    pub fn should_speak_reply(&self) -> bool {
        self.auto_play && !self.muted
    }

    pub fn set_speed(&mut self, speed: f32) -> Result<(), String> {
        if !(Self::MIN_SPEED..=Self::MAX_SPEED).contains(&speed) {
            return Err(format!(
                "hız {} ile {} arasında olmalı",
                Self::MIN_SPEED,
                Self::MAX_SPEED
            ));
        }
        self.speed = speed;
        Ok(())
    }

    /// Kullanıcıya tek satırda durum. Erişilebilirlik: ses davranışının tamamı metin olarak
    /// okunabilir olmalı, yalnız bir simgeyle değil.
    pub fn summary(&self) -> String {
        format!(
            "Seslendirme: {} • otomatik oynatma: {} • hız: {:.2}x • sesli istek: {}",
            if self.muted {
                "sessiz mod AÇIK"
            } else {
                "açık"
            },
            if self.auto_play { "açık" } else { "kapalı" },
            self.speed,
            if self.review_transcript {
                "önce gözden geçir"
            } else {
                "doğrudan gönder ve sesli yanıtla"
            }
        )
    }
}

/// Bir yanıtın **seslendirilecek** kısmını çıkarır.
///
/// 20 Ağustos 2026, gerçek kullanımda bulundu: JARVIS bir C++ sınıfı yazdı ve sesli mod açıkken
/// kodun tamamını — süslü parantezler, `std::unique_lock`, noktalı virgüller dahil — yüksek
/// sesle okudu. Kullanıcının tarifiyle "büyü yapıyor gibi". Kod okunacak bir şey değil, ekranda
/// görülecek bir şeydir; kaynak listesi de öyle (dosya yolları sesli okunduğunda anlamsız).
///
/// Bu yüzden konuşma metni yanıtın kendisi değil, yanıtın **anlatı kısmı**: kod blokları tek bir
/// kısa cümleyle özetleniyor, biçimlendirme işaretleri temizleniyor, kaynak bloğu atılıyor.
/// Ekrandaki metin hiç değişmiyor — yalnız seslendirilen sürüm sadeleşiyor.
pub fn speakable_summary(reply: &str) -> String {
    // Kaynak/bellek bloğu: ekranda değerli, seslendirildiğinde gürültü.
    let mut body = reply;
    for marker in ["\n\nKaynaklar:", "\n\n(bu yanıtta "] {
        if let Some(index) = body.find(marker) {
            body = &body[..index];
        }
    }

    let mut spoken = String::new();
    let mut code_blocks = 0usize;
    let mut inside_code = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            if !inside_code {
                code_blocks += 1;
            }
            inside_code = !inside_code;
            continue;
        }
        if inside_code {
            continue;
        }
        let cleaned = strip_inline_markup(line);
        if cleaned.trim().is_empty() {
            continue;
        }
        spoken.push_str(cleaned.trim());
        spoken.push(' ');
    }

    let mut spoken = spoken.trim().to_string();
    if code_blocks > 0 {
        // Kullanıcı kodun *var olduğunu* duymalı, içeriğini değil — ekrana bakması gerektiğini
        // bilmesi lazım.
        let note = if code_blocks == 1 {
            "Kodu ekrana yazdım.".to_string()
        } else {
            format!("{code_blocks} kod bloğunu ekrana yazdım.")
        };
        if spoken.is_empty() {
            spoken = note;
        } else {
            spoken.push(' ');
            spoken.push_str(&note);
        }
    }
    spoken
}

/// Satır içi markdown işaretlerini temizler. Sesli okunduğunda `**`, backtick ve başlık
/// diyezleri ya sessizce yutulur ya da garip duraklamalara yol açar; ikisi de istenmez.
fn strip_inline_markup(line: &str) -> String {
    let without_heading = line.trim_start().trim_start_matches('#').trim_start();
    let without_bullet = without_heading
        .strip_prefix("- ")
        .or_else(|| without_heading.strip_prefix("* "))
        .unwrap_or(without_heading);
    // Tek geçiş: `**` zaten `*` temizliğinin içinde kalıyor, `_` boşluğa dönüyor.
    without_bullet
        .chars()
        .map(|character| match character {
            '`' | '*' => '\0',
            '_' => ' ',
            other => other,
        })
        .filter(|character| *character != '\0')
        .collect()
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
