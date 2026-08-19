use super::*;

fn example(
    id: &str,
    reviewed: bool,
    status: VerifyStatus,
    sensitivity: DataSensitivity,
) -> TeacherExample {
    TeacherExample {
        schema_version: 1,
        example_id: id.into(),
        prompt: format!("prompt-{id}"),
        expected_capability: "system.health".into(),
        response: format!("response-{id}"),
        evidence: vec!["verifier PASS".into()],
        verifier_status: status,
        provenance: "local test".into(),
        human_reviewed: reviewed,
        sensitivity,
    }
}

/// F6 madde 2: export bir veritabanı dökümü değil. İncelenmemiş, verifier'ı geçmemiş veya
/// hassas bir örnek dışarı çıkamaz — ve sessizce düşmez, gerekçesiyle raporlanır.
#[test]
fn only_reviewed_verified_non_sensitive_examples_are_exported() {
    let examples = vec![
        example("ok", true, VerifyStatus::Pass, DataSensitivity::Internal),
        example(
            "unreviewed",
            false,
            VerifyStatus::Pass,
            DataSensitivity::Internal,
        ),
        example(
            "failed",
            true,
            VerifyStatus::Fail,
            DataSensitivity::Internal,
        ),
        example(
            "secret",
            true,
            VerifyStatus::Pass,
            DataSensitivity::Sensitive,
        ),
    ];
    let export = build_dataset_export(1, &examples, &[]);

    assert_eq!(export.records.len(), 1);
    assert_eq!(export.records[0].example_id, "ok");
    assert_eq!(
        export.excluded.len(),
        3,
        "dışlananlar gerekçesiyle raporlanmalı"
    );
    assert!(export
        .excluded
        .iter()
        .any(|item| item.example_id == "secret" && item.reason.contains("SENSITIVE")));
}

/// Marker'lar uygunluğu ezer: iyi görünen ama zehirli işaretlenmiş bir örnek asla export
/// edilemez. Sıralama bilinçli — tersi olsaydı zehirli örnek "düzgün" olduğu için geçerdi.
#[test]
fn a_poisoned_marker_beats_an_otherwise_eligible_example() {
    let examples = vec![example(
        "ok",
        true,
        VerifyStatus::Pass,
        DataSensitivity::Internal,
    )];
    let markers = vec![DatasetMarker {
        example_id: "ok".into(),
        kind: DatasetMarkerKind::Poisoned,
        reason: "manipüle edilmiş içerik".into(),
    }];
    let export = build_dataset_export(2, &examples, &markers);

    assert!(export.records.is_empty(), "zehirli örnek export edilemez");
    assert_eq!(export.excluded.len(), 1);
    assert!(export.excluded[0].reason.contains("poisoned"));
    // Marker manifest'te kalır: tüketici, id'nin kasten dışarıda bırakıldığını görebilmeli.
    assert_eq!(export.markers.len(), 1);
    assert!(export.to_manifest_text().contains("marker\tok\tpoisoned"));
}

/// Manifest hash'i içerik-adresli olmalı: aynı yönetilen içerik aynı hash'i, farklı içerik
/// farklı hash'i vermeli. "Bu model hangi dataset ile eğitildi" sorusunun cevabı budur.
#[test]
fn the_manifest_hash_changes_only_when_governed_content_changes() {
    let examples = vec![example(
        "a",
        true,
        VerifyStatus::Pass,
        DataSensitivity::Internal,
    )];
    let first = build_dataset_export(1, &examples, &[]);
    let same = build_dataset_export(1, &examples, &[]);
    assert_eq!(
        first.manifest_hash, same.manifest_hash,
        "aynı içerik aynı hash"
    );
    assert_eq!(first.manifest_hash.len(), 64);

    // Sürüm numarası da kimliğin parçası.
    let other_version = build_dataset_export(2, &examples, &[]);
    assert_ne!(first.manifest_hash, other_version.manifest_hash);

    // Bir silme marker'ı eklemek içeriği değiştirir.
    let with_marker = build_dataset_export(
        1,
        &examples,
        &[DatasetMarker {
            example_id: "b".into(),
            kind: DatasetMarkerKind::Deleted,
            reason: "kullanıcı silme talebi".into(),
        }],
    );
    assert_ne!(first.manifest_hash, with_marker.manifest_hash);
}

/// Silme marker'ı, satırı yok etmek yerine "bilinen-kötü" olarak kalıcı kılar — aksi halde aynı
/// içerik daha sonra yeniymiş gibi tekrar kuyruğa girebilirdi.
#[test]
fn a_deletion_marker_is_recorded_rather_than_erasing_the_id() {
    let export = build_dataset_export(
        3,
        &[],
        &[DatasetMarker {
            example_id: "silinen".into(),
            kind: DatasetMarkerKind::Deleted,
            reason: "kullanıcı talebi".into(),
        }],
    );
    assert!(export.records.is_empty());
    assert!(export
        .to_manifest_text()
        .contains("marker\tsilinen\tdeleted\tkullanıcı talebi"));
}

fn config_run(
    id: &str,
    passed: u32,
    latency: u64,
    rollback: Option<&str>,
) -> crate::ModelConfigRun {
    crate::ModelConfigRun {
        schema_version: 1,
        run_id: id.into(),
        recorded_at: 1_000,
        provider_id: "llama-server".into(),
        model_id: "Qwen3-8B".into(),
        model_fingerprint: "aaa".into(),
        prompt_fingerprint: "p1".into(),
        server_settings: String::new(),
        scenarios_passed: passed,
        scenarios_failed: 10 - passed,
        median_latency_ms: latency,
        notes: String::new(),
        rollback_target: rollback.map(str::to_owned),
    }
}

/// F6 madde 5: bir senaryo kaybı, hız kazancı ne olursa olsun regresyondur. F6'nın tamamlanma
/// ölçütü "hedef metriği iyileştirir ve regresyon üretmez" — doğruluğu hıza takas etmek değil.
#[test]
fn losing_a_scenario_is_a_regression_even_when_it_gets_faster() {
    let comparison = compare_model_config_runs(
        &config_run("eski", 10, 20_000, None),
        &config_run("yeni", 9, 5_000, Some("eski")),
    );
    assert_eq!(comparison.verdict, ModelConfigVerdict::Regressed);
    assert!(
        comparison.reason.contains("senaryo kaybı"),
        "{}",
        comparison.reason
    );
}

/// Kalite aynıyken belirgin yavaşlama da regresyondur; sıradan dalgalanma ise değildir.
#[test]
fn a_large_slowdown_with_no_quality_gain_is_a_regression_but_noise_is_not() {
    let regressed = compare_model_config_runs(
        &config_run("eski", 10, 10_000, None),
        &config_run("yeni", 10, 20_000, Some("eski")),
    );
    assert_eq!(regressed.verdict, ModelConfigVerdict::Regressed);

    let noise = compare_model_config_runs(
        &config_run("eski", 10, 10_000, None),
        &config_run("yeni", 10, 11_000, Some("eski")),
    );
    assert_eq!(
        noise.verdict,
        ModelConfigVerdict::Unchanged,
        "sıradan dalgalanma rollback önermemeli"
    );
}

/// Daha fazla senaryo geçmek her zaman iyileşmedir; kalite aynıyken hızlanmak da öyle.
#[test]
fn more_passing_scenarios_or_the_same_quality_faster_counts_as_improvement() {
    let better_quality = compare_model_config_runs(
        &config_run("eski", 8, 10_000, None),
        &config_run("yeni", 10, 12_000, Some("eski")),
    );
    assert_eq!(better_quality.verdict, ModelConfigVerdict::Improved);

    let faster = compare_model_config_runs(
        &config_run("eski", 10, 20_000, None),
        &config_run("yeni", 10, 12_000, Some("eski")),
    );
    assert_eq!(faster.verdict, ModelConfigVerdict::Improved);
}
