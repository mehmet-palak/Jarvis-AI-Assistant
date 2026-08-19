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

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
