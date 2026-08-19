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
