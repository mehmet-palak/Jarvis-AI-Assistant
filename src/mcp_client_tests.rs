use super::*;

const TEST_KEY: [u8; 32] = [7u8; 32];

fn valid_manifest() -> McpServerManifest {
    McpServerManifest {
        schema_version: CURRENT_MCP_CLIENT_PROTOCOL_VERSION,
        id: "weather-tool".into(),
        display_name: "Yerel Hava Aracı".into(),
        kind: McpServerKind::ExternalTool,
        transport: McpTransport::Stdio {
            command: "/usr/bin/weather-mcp".into(),
            args: vec!["--stdio".into()],
        },
        declared_tools: vec!["weather.today".into()],
        capability_allowlist: vec!["system.time".into()],
        sensitivity_ceiling: DataSensitivity::Public,
        network_allowed: false,
        artifact_hash: "a".repeat(64),
    }
}

fn signed(manifest: McpServerManifest) -> SignedMcpManifest {
    let signature = sign_mcp_manifest(&TEST_KEY, &manifest);
    SignedMcpManifest {
        manifest,
        signature,
    }
}

#[test]
fn a_manifest_signs_and_verifies() {
    let manifest = valid_manifest();
    let signature = sign_mcp_manifest(&TEST_KEY, &manifest);
    assert!(verify_mcp_manifest(&TEST_KEY, &manifest, &signature));
}

#[test]
fn tampering_with_any_field_breaks_the_signature() {
    let manifest = valid_manifest();
    let signature = sign_mcp_manifest(&TEST_KEY, &manifest);

    // Yetenek beyaz-listesini genişletmek (yetki yükseltme denemesi) imzayı bozmalı.
    let mut escalated = manifest.clone();
    escalated
        .capability_allowlist
        .push("file.read_workspace".into());
    assert!(!verify_mcp_manifest(&TEST_KEY, &escalated, &signature));

    // Hassasiyet tavanını yükseltmek imzayı bozmalı.
    let mut higher = manifest.clone();
    higher.sensitivity_ceiling = DataSensitivity::Sensitive;
    assert!(!verify_mcp_manifest(&TEST_KEY, &higher, &signature));

    // Ağ iznini açmak imzayı bozmalı.
    let mut networked = manifest.clone();
    networked.network_allowed = true;
    assert!(!verify_mcp_manifest(&TEST_KEY, &networked, &signature));

    // Artefakt hash'ini değiştirmek imzayı bozmalı.
    let mut swapped = manifest;
    swapped.artifact_hash = "b".repeat(64);
    assert!(!verify_mcp_manifest(&TEST_KEY, &swapped, &signature));
}

#[test]
fn a_wrong_key_never_verifies() {
    let manifest = valid_manifest();
    let signature = sign_mcp_manifest(&TEST_KEY, &manifest);
    let other_key = [9u8; 32];
    assert!(!verify_mcp_manifest(&other_key, &manifest, &signature));
}

#[test]
fn validate_rejects_empty_allowlist_and_bad_hash_and_empty_id() {
    assert!(validate_mcp_manifest(&valid_manifest()).is_ok());

    let mut no_caps = valid_manifest();
    no_caps.capability_allowlist.clear();
    assert!(validate_mcp_manifest(&no_caps).is_err());

    let mut bad_hash = valid_manifest();
    bad_hash.artifact_hash = "kısa".into();
    assert!(validate_mcp_manifest(&bad_hash).is_err());

    let mut empty_id = valid_manifest();
    empty_id.id = "   ".into();
    assert!(validate_mcp_manifest(&empty_id).is_err());

    let mut empty_cmd = valid_manifest();
    empty_cmd.transport = McpTransport::Stdio {
        command: "".into(),
        args: vec![],
    };
    assert!(validate_mcp_manifest(&empty_cmd).is_err());
}

#[test]
fn external_protocol_version_is_range_checked() {
    assert!(validate_external_mcp_protocol_version(CURRENT_MCP_CLIENT_PROTOCOL_VERSION).is_ok());
    assert!(validate_external_mcp_protocol_version(0).is_err());
    assert!(validate_external_mcp_protocol_version(2).is_err());
}

#[test]
fn sensitivity_ceiling_is_enforced_by_rank() {
    assert!(sensitivity_within_ceiling(
        DataSensitivity::Public,
        DataSensitivity::Internal
    ));
    assert!(sensitivity_within_ceiling(
        DataSensitivity::Internal,
        DataSensitivity::Internal
    ));
    assert!(!sensitivity_within_ceiling(
        DataSensitivity::Sensitive,
        DataSensitivity::Internal
    ));
    assert!(!sensitivity_within_ceiling(
        DataSensitivity::Internal,
        DataSensitivity::Public
    ));
}

#[test]
fn authorize_connect_happy_path_passes_all_gates() {
    let signed = signed(valid_manifest());
    let hash = "a".repeat(64);
    assert!(authorize_mcp_connect(
        &signed,
        &TEST_KEY,
        McpServerStatus::Active,
        &hash,
        &hash,
        CURRENT_MCP_CLIENT_PROTOCOL_VERSION,
    )
    .is_ok());
}

#[test]
fn authorize_connect_refuses_a_non_active_server() {
    let signed = signed(valid_manifest());
    let hash = "a".repeat(64);
    for status in [McpServerStatus::Quarantined, McpServerStatus::Revoked] {
        let outcome = authorize_mcp_connect(
            &signed,
            &TEST_KEY,
            status,
            &hash,
            &hash,
            CURRENT_MCP_CLIENT_PROTOCOL_VERSION,
        );
        assert_eq!(outcome, Err(McpConnectRejection::NotActive(status)));
    }
}

#[test]
fn authorize_connect_refuses_a_tampered_manifest() {
    // İmzalı manifesti aldıktan SONRA bir alanı değiştir: imza artık tutmamalı.
    let mut signed = signed(valid_manifest());
    signed
        .manifest
        .capability_allowlist
        .push("file.read_workspace".into());
    let hash = "a".repeat(64);
    let outcome = authorize_mcp_connect(
        &signed,
        &TEST_KEY,
        McpServerStatus::Active,
        &hash,
        &hash,
        CURRENT_MCP_CLIENT_PROTOCOL_VERSION,
    );
    assert_eq!(outcome, Err(McpConnectRejection::SignatureMismatch));
}

#[test]
fn authorize_connect_refuses_a_changed_artifact_rug_pull() {
    let signed = signed(valid_manifest());
    let approved = "a".repeat(64);
    let live = "b".repeat(64); // artefakt onaylandığından beri değişti
    let outcome = authorize_mcp_connect(
        &signed,
        &TEST_KEY,
        McpServerStatus::Active,
        &approved,
        &live,
        CURRENT_MCP_CLIENT_PROTOCOL_VERSION,
    );
    assert_eq!(outcome, Err(McpConnectRejection::ArtifactChanged));
}

#[test]
fn authorize_connect_refuses_an_unknown_protocol() {
    let signed = signed(valid_manifest());
    let hash = "a".repeat(64);
    let outcome = authorize_mcp_connect(
        &signed,
        &TEST_KEY,
        McpServerStatus::Active,
        &hash,
        &hash,
        99,
    );
    assert!(matches!(
        outcome,
        Err(McpConnectRejection::UnsupportedProtocol(_))
    ));
}

#[test]
fn hash_artifact_is_deterministic_and_content_sensitive() {
    let dir = std::env::temp_dir().join(format!(
        "jarvis-mcp-artifact-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("server.bin");

    std::fs::write(&path, b"benign server v1").unwrap();
    let first = hash_artifact(&path).expect("hashes");
    let again = hash_artifact(&path).expect("hashes");
    assert_eq!(first, again, "aynı içerik aynı hash");
    assert_eq!(first.len(), 64);

    std::fs::write(&path, b"malicious server v2").unwrap();
    let changed = hash_artifact(&path).expect("hashes");
    assert_ne!(
        first, changed,
        "içerik değişince hash değişmeli (rug-pull tespiti)"
    );

    std::fs::remove_dir_all(&dir).expect("test cleanup");
}
