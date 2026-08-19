use super::*;
use crate::{policy::approval_channel_requirement, ApprovalChannelRequirement};

fn transcript(text: &str, confirmed: bool) -> VoiceTranscript {
    VoiceTranscript {
        transcript_id: "t-1".into(),
        text: text.into(),
        duration_ms: 2_400,
        retention: RecordingRetention::default(),
        confirmed,
    }
}

/// F5 madde 4: kullanıcı onaylamadan hiçbir transkript istek olmaz. Konuşma tanıma yanılabilir;
/// yanlış anlaşılmış bir cümlenin sessizce gönderilmesi sesli kullanımın en büyük riski.
#[test]
fn an_unconfirmed_transcript_never_becomes_a_request() {
    let rejection = transcript_into_request(&transcript("notlarımı sil", false), "r-1")
        .expect_err("onaysız transkript gönderilememeli");
    assert_eq!(rejection, TranscriptRejection::NotConfirmed);

    let request = transcript_into_request(&transcript("notlarımı sil", true), "r-1")
        .expect("onaylı transkript gönderilebilmeli");
    assert_eq!(request.content, "notlarımı sil");
}

/// Sessizlik veya gürültü ayrı bir durum: kullanıcıya "onayla" demek anlamsız, çünkü onaylanacak
/// bir şey yok. Bu yüzden reddetme tipli — arayüz iki duruma farklı tepki verebilsin.
#[test]
fn silence_is_rejected_as_empty_not_as_unconfirmed() {
    for silent in ["", "   ", "\n\t "] {
        let rejection = transcript_into_request(&transcript(silent, true), "r-1")
            .expect_err("sessizlik istek üretmemeli");
        assert_eq!(rejection, TranscriptRejection::Empty);
    }
}

/// Sesli isteğin `InputType::Voice` taşıması bir etiket değil, güvenlik sınırının dayanağı:
/// `approval_channel_requirement` tam olarak bu alana bakıyor. Sesli bir isteği CLI'dan gelmiş
/// gibi işaretlemek, F5'in sesli onay korumasını sessizce devre dışı bırakırdı.
#[test]
fn a_voice_request_keeps_its_origin_so_the_approval_gate_still_applies() {
    let request = transcript_into_request(&transcript("not oluştur: alışveriş", true), "r-1")
        .expect("onaylı transkript");
    assert_eq!(request.input_type, InputType::Voice);

    assert_eq!(
        approval_channel_requirement("note.create", &request.content, request.input_type),
        ApprovalChannelRequirement::WrittenConfirmationRequired,
        "sesten gelen istek, onay gerektiren eylemde yazılı doğrulama istemeli"
    );
}

/// F5 madde 5: ham ses varsayılan olarak saklanmaz. Bu bir ayar tercihi değil, tipin kendi
/// varsayılanı — bir yapılandırma dosyası unutulsa bile davranış gizliliği koruyor.
#[test]
fn raw_audio_is_not_kept_by_default() {
    let default = RecordingRetention::default();
    assert_eq!(default, RecordingRetention::DiscardImmediately);
    assert!(!default.keeps_audio());
    assert!(default.user_visible_summary().contains("saklanmıyor"));
}

/// Kullanıcı saklamayı seçerse, nerede ve ne zaman silineceği görünür olmak zorunda —
/// "belki bir yerde duruyor" durumu kabul edilemez, bu yüzden ikisi de tipin içinde.
#[test]
fn kept_audio_must_disclose_where_it_is_and_when_it_goes() {
    let retention = RecordingRetention::KeepUntil {
        path: std::path::PathBuf::from("/tmp/jarvis-voice/2026-08-19.wav"),
        delete_after_epoch: 1_755_600_000,
    };
    assert!(retention.keeps_audio());
    let summary = retention.user_visible_summary();
    assert!(
        summary.contains("/tmp/jarvis-voice/2026-08-19.wav"),
        "{summary}"
    );
    assert!(summary.contains("1755600000"), "{summary}");
}

/// F5 uçtan uca — gerçek donanım/model gerektirir, bu yüzden `#[ignore]` (golden set'le aynı
/// desen: `cargo test` ve offline release gate bozulmaz).
///
/// Mikrofon önünde kimse olmayabileceği için kayıt yerine, Piper'ın kendi ürettiği sesi
/// whisper'a veriyoruz: bu, iki yığını da gerçekten çalıştıran ama insan gerektirmeyen tek
/// doğrulama yolu. Ses yakalamanın kendisi ayrıca `recording_start_stop_produces_a_wav` ile
/// gerçek mikrofondan doğrulanıyor.
#[test]
#[ignore = "gerçek whisper + piper kurulumu gerektirir"]
fn speech_synthesis_and_transcription_round_trip_on_real_models() {
    let paths = VoiceStackPaths::local_default();
    let availability = paths.availability();
    assert!(
        availability.transcription && availability.speech,
        "whisper ve piper kurulu olmalı: {:?}",
        availability
    );

    let spoken = "Bugünkü toplantı saat kaçta başlıyor?";
    let wav = paths.scratch_dir.join("roundtrip.wav");
    synthesize_speech(&paths, spoken, &wav).expect("seslendirme");
    assert!(wav.is_file(), "seslendirme bir WAV üretmeli");

    // Piper 22.05 kHz üretiyor, whisper 16 kHz istiyor; gerçek akışta kayıt zaten 16 kHz
    // olduğu için bu dönüşüm yalnız bu testin ihtiyacı.
    let wav16 = paths.scratch_dir.join("roundtrip-16k.wav");
    let converted = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(&wav)
        .args(["-ar", "16000", "-ac", "1"])
        .arg(&wav16)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    assert!(converted, "test için ffmpeg gerekli");

    let transcript = transcribe_recording(
        &paths,
        &wav16,
        RecordingRetention::DiscardImmediately,
        "rt-1",
    )
    .expect("transkripsiyon");

    println!("söylenen : {spoken}");
    println!("çevrilen : {}", transcript.text);

    // Tam eşleşme beklemiyoruz (ADR-0007: small modelde büyük harf/tek kelime farkları normal);
    // beklediğimiz, ayırt edici kelimelerin yakalanması.
    let lowered = transcript.text.to_lowercase();
    for keyword in ["toplantı", "saat", "başlıyor"] {
        assert!(
            lowered.contains(keyword),
            "transkript '{keyword}' kelimesini içermeli: {}",
            transcript.text
        );
    }

    // Gizlilik: DiscardImmediately, transkripsiyondan sonra ham sesi silmiş olmalı.
    assert!(
        !wav16.is_file(),
        "DiscardImmediately ham ses dosyasını silmeliydi"
    );
    assert!(!transcript.confirmed, "yeni transkript onaylanmamış olmalı");

    let _ = std::fs::remove_file(&wav);
}

/// Gerçek mikrofondan kayıt: başlat → kısa süre bekle → durdur → geçerli bir WAV oluşmalı.
/// İnsan konuşmasına gerek yok; ölçtüğümüz şey yakalama zincirinin çalıştığı.
#[test]
#[ignore = "gerçek mikrofon gerektirir"]
fn recording_start_stop_produces_a_wav() {
    let paths = VoiceStackPaths::local_default();
    assert!(paths.availability().capture, "pw-record kurulu olmalı");

    let recording = VoiceRecording::start(&paths, "mic-contract-test").expect("kayıt başlamalı");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let wav = recording.stop().expect("kayıt durmalı");

    let bytes = std::fs::metadata(&wav).expect("kayıt dosyası").len();
    assert!(bytes > 1000, "kayıt boş çıktı: {bytes} bayt");
    let _ = std::fs::remove_file(&wav);
}

/// İptal edilen kayıt diskte kalmaz — gizlilik varsayılanı iptal yolunda da geçerli.
#[test]
#[ignore = "gerçek mikrofon gerektirir"]
fn a_cancelled_recording_leaves_nothing_on_disk() {
    let paths = VoiceStackPaths::local_default();
    assert!(paths.availability().capture);

    let recording = VoiceRecording::start(&paths, "mic-cancel-test").expect("kayıt");
    let path = recording.path().to_path_buf();
    std::thread::sleep(std::time::Duration::from_millis(600));
    recording.cancel();

    assert!(!path.is_file(), "iptal edilen kayıt silinmeliydi: {path:?}");
}

/// F5 "ses seviyesi/VAD göstergesi": gerçek konuşma içeren kayıt ile sessiz kayıt ayırt
/// edilebilmeli. Ayırt edemeyen bir gösterge, kullanıcının 20 saniye konuşup "hiçbir şey
/// duyulmadı" ile karşılaşmasını engelleyemez.
#[test]
#[ignore = "gerçek piper kurulumu gerektirir"]
fn the_level_meter_separates_real_speech_from_silence() {
    let paths = VoiceStackPaths::local_default();
    assert!(paths.availability().speech, "piper kurulu olmalı");

    let speech_wav = paths.scratch_dir.join("level-speech.wav");
    synthesize_speech(&paths, "Bu bir ses seviyesi ölçüm cümlesidir.", &speech_wav)
        .expect("seslendirme");
    // Bitmiş bir cümlenin *sonu* sessizliktir (cümle sonu boşluğu) — ölçüldü: son 8000 baytın
    // RMS'i 0. Fonksiyon canlı, büyüyen bir kayıt için tasarlandı; orada "son" = "şu an".
    // Bu yüzden test, konuşmanın ortasında kesilmiş bir dosyayla o durumu taklit ediyor.
    let full = std::fs::read(&speech_wav).expect("wav okunmalı");
    let mid_speech = paths.scratch_dir.join("level-midspeech.wav");
    std::fs::write(&mid_speech, &full[..full.len() * 2 / 3]).expect("kesilmiş wav");
    let speech_level = recent_audio_level(&mid_speech);

    // Sessiz WAV: geçerli 44 baytlık başlık + sıfır örnekler.
    let silent_wav = paths.scratch_dir.join("level-silence.wav");
    let mut header = std::fs::read(&speech_wav).expect("başlık için")[..44].to_vec();
    header.extend(std::iter::repeat_n(0u8, 16_000));
    std::fs::write(&silent_wav, &header).expect("sessiz wav");
    let silent_level = recent_audio_level(&silent_wav);

    println!("konuşma seviyesi: {speech_level:.4} • sessizlik: {silent_level:.4}");
    assert!(
        speech_level > silent_level,
        "konuşma sessizlikten yüksek ölçülmeli ({speech_level} vs {silent_level})"
    );
    assert_eq!(level_description(silent_level), "sessiz");
    assert_ne!(level_description(speech_level), "sessiz");

    let _ = std::fs::remove_file(&speech_wav);
    let _ = std::fs::remove_file(&mid_speech);
    let _ = std::fs::remove_file(&silent_wav);
}

/// Başlık uzunluğunu sabit varsaymamak gerçek bir gereksinim: `data` chunk'ından önce fazladan
/// bir chunk bulunan bir WAV'da sabit 44 varsayımı başlık baytlarını ses sanardı ve **sahte bir
/// seviye** gösterirdi. Sahte gösterge, hiç göstergesi olmamasından kötüdür.
#[test]
fn the_level_reader_finds_the_data_chunk_instead_of_assuming_a_fixed_header() {
    let dir = std::env::temp_dir().join("jarvis-voice-wavparse");
    std::fs::create_dir_all(&dir).expect("dizin");
    let path = dir.join("extra-chunk.wav");

    // RIFF/WAVE + fmt + fazladan bir LIST chunk'ı + data (hepsi sessiz örnekler).
    let mut wav: Vec<u8> = Vec::new();
    wav.extend(b"RIFF");
    wav.extend(0u32.to_le_bytes()); // boyut alanı bu test için önemsiz
    wav.extend(b"WAVE");
    wav.extend(b"fmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend([0u8; 16]);
    wav.extend(b"LIST");
    wav.extend(10u32.to_le_bytes());
    wav.extend([0u8; 10]);
    wav.extend(b"data");
    wav.extend(200u32.to_le_bytes());
    wav.extend(std::iter::repeat_n(0u8, 200));
    std::fs::write(&path, &wav).expect("wav yaz");

    // Tamamı sessiz örnek: doğru konumdan okunursa 0 çıkar. Yanlış konumdan okunsaydı
    // "fmt "/"LIST" baytları rastgele örnek olarak yorumlanır ve sıfırdan farklı çıkardı.
    assert_eq!(recent_audio_level(&path), 0.0);

    // RIFF olmayan bir dosya sessizce 0 dönmeli, panik değil.
    let bogus = dir.join("not-a-wav.bin");
    std::fs::write(&bogus, b"bu bir wav degil").expect("yaz");
    assert_eq!(recent_audio_level(&bogus), 0.0);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Gösterge çubuğu her zaman istenen genişlikte olmalı — taşan bir çubuk durum satırını bozar.
/// Ayrıca uç değerler (0 ve 1) doğru uçlara oturmalı.
#[test]
fn the_level_meter_bar_is_always_exactly_the_requested_width() {
    for level in [0.0, 0.001, 0.05, 0.5, 1.0, 5.0] {
        let bar = level_meter(level, 12);
        assert_eq!(
            bar.chars().count(),
            12,
            "seviye {level} için genişlik bozuk"
        );
    }
    assert!(
        level_meter(0.0, 10).starts_with('░'),
        "sessizlik boş görünmeli"
    );
    assert_eq!(
        level_meter(1.0, 10),
        "█".repeat(10),
        "tam seviye dolu olmalı"
    );
}

/// Erişilebilirlik: her seviye değerinin bir metin karşılığı olmalı ve bu karşılıklar seviye
/// arttıkça değişmeli — ekran okuyucu kullanan biri çubuğu göremez, açıklamayı duyar.
#[test]
fn every_level_has_a_distinct_text_description_for_screen_readers() {
    let descriptions: Vec<&str> = [0.0, 0.01, 0.05, 0.15, 0.5]
        .iter()
        .map(|level| level_description(*level))
        .collect();
    assert_eq!(descriptions[0], "sessiz");
    // Ardışık eşiklerin hepsi farklı olmalı, yoksa açıklama bilgi taşımıyor demektir.
    for pair in descriptions.windows(2) {
        assert_ne!(
            pair[0], pair[1],
            "ardışık seviyeler aynı açıklamayı vermemeli"
        );
    }
}

/// F5 "TTS playback": varsayılan davranış JARVIS'in kendiliğinden konuşmaması. Sesli bir asistanın
/// izinsiz konuşmaya başlaması, kullanıcının kontrolünü kaybettiği ilk yerdir.
#[test]
fn speech_is_opt_in_and_mute_overrides_everything() {
    let mut settings = SpeechSettings::default();
    assert!(
        !settings.auto_play,
        "otomatik oynatma varsayılan olarak kapalı olmalı"
    );
    assert!(!settings.should_speak_reply());

    settings.auto_play = true;
    assert!(settings.should_speak_reply());

    // Sessiz mod, otomatik oynatma açık olsa bile her şeyi bastırır — ikisi farklı şeyler:
    // auto_play kalıcı bir alışkanlık, muted anlık bir durum (toplantı, gece).
    settings.muted = true;
    assert!(
        !settings.should_speak_reply(),
        "sessiz mod otomatik oynatmayı bastırmalı"
    );
    assert!(settings.summary().contains("sessiz mod AÇIK"));
}

/// Hız sınırları uygulanmalı ve Piper'ın ters ölçeğine doğru çevrilmeli: kullanıcı "daha hızlı"
/// derken length_scale'in *küçülmesi* gerekir. Ters çevirmeyi unutmak, hız düğmesini yavaşlatıcı
/// yapardı.
#[test]
fn speech_speed_is_bounded_and_inverted_for_piper() {
    let mut settings = SpeechSettings::default();
    assert!((settings.piper_length_scale() - 1.0).abs() < 1e-6);

    settings.set_speed(2.0).expect("üst sınır kabul edilmeli");
    assert!(
        settings.piper_length_scale() < 1.0,
        "daha hızlı konuşma daha küçük length_scale demek"
    );

    settings.set_speed(0.5).expect("alt sınır kabul edilmeli");
    assert!(settings.piper_length_scale() > 1.0);

    assert!(
        settings.set_speed(5.0).is_err(),
        "sınır dışı hız reddedilmeli"
    );
    assert!(settings.set_speed(0.1).is_err());
}

/// F5 E2E: model kurulu değilse özellik sessizce yarım çalışmamalı — eksik parça açıkça
/// söylenmeli. "Sessizce daha kötü çalışmak" bu projede daha önce gerçek bir hataydı
/// (embedding servisi), aynı hatayı ses tarafında tekrarlamıyoruz.
#[test]
fn a_missing_model_is_reported_not_silently_ignored() {
    let paths = VoiceStackPaths {
        whisper_cli: std::path::PathBuf::from("/nonexistent/whisper-cli"),
        whisper_model: std::path::PathBuf::from("/nonexistent/model.bin"),
        piper_binary: std::path::PathBuf::from("/nonexistent/piper"),
        piper_voice: std::path::PathBuf::from("/nonexistent/voice.onnx"),
        scratch_dir: std::env::temp_dir().join("jarvis-voice-missing"),
    };
    let availability = paths.availability();
    assert!(!availability.transcription);
    assert!(!availability.speech);
    assert!(!availability.voice_input_ready());

    let summary = availability
        .missing_summary()
        .expect("eksikler bildirilmeli");
    assert!(summary.contains("konuşma tanıma"), "{summary}");

    // Var olmayan bir motorla çeviri denemesi hata döndürmeli, boş transkript değil —
    // boş transkript "sessizlik" ile karışırdı.
    let error = transcribe_recording(
        &paths,
        std::path::Path::new("/nonexistent/audio.wav"),
        RecordingRetention::DiscardImmediately,
        "missing",
    )
    .expect_err("eksik motor hata vermeli");
    assert!(error.contains("çalıştırılamadı"), "{error}");
}

/// Boş metni seslendirmeye çalışmak sessizce başarılı olmamalı: çağıran, ses çıkmadığını
/// bilmeli.
#[test]
fn synthesizing_empty_text_is_an_error_not_a_silent_no_op() {
    let paths = VoiceStackPaths::local_default();
    let error = synthesize_speech(&paths, "   ", std::path::Path::new("/tmp/never.wav"))
        .expect_err("boş metin reddedilmeli");
    assert!(error.contains("boş"), "{error}");
}
