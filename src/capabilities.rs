//! Capability registry and manifests.
//!
//! A capability is only reachable through this registry: `CapabilityRegistry::baseline()` is the
//! single source of truth for which capability IDs exist locally, and `capability_manifest`
//! carries the risk/effect-scope/sandbox/verifier metadata Policy and Tool Runtime rely on. Model
//! output and MCP tool IDs are only ever proposals; the Decision Core still checks them against
//! this registry before a task can be created.

use std::collections::HashMap;

use crate::{CapabilityManifest, Risk};

#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    manifests: HashMap<String, CapabilityManifest>,
}

impl CapabilityRegistry {
    pub fn baseline() -> Self {
        let mut registry = Self::default();
        for id in [
            "system.health",
            "system.time",
            "conversation.reply",
            "file.read_workspace",
            "project.info",
            "code.project_outline",
            "docs.workspace_summary",
            "note.create",
        ] {
            if let Some(manifest) = capability_manifest(id) {
                registry.manifests.insert(id.into(), manifest);
            }
        }
        registry
    }

    pub fn get(&self, capability: &str) -> Option<&CapabilityManifest> {
        self.manifests.get(capability)
    }

    pub fn contains(&self, capability: &str) -> bool {
        self.manifests.contains_key(capability)
    }

    /// Crate-internal mutation hook. Not part of the public contract: production code obtains
    /// manifests only through `baseline()`/`get()`; this exists so tests can construct
    /// contract-violation fixtures (for example a manifest whose sandbox profile no longer
    /// matches its capability) without a second, divergent way to build a registry.
    #[cfg(test)]
    pub(crate) fn get_mut(&mut self, capability: &str) -> Option<&mut CapabilityManifest> {
        self.manifests.get_mut(capability)
    }
}

pub fn capability_manifest(capability: &str) -> Option<CapabilityManifest> {
    match capability {
        "system.health" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Low,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "NO_EXEC_READ_ONLY".into(),
            verifier_profile: "health".into(),
        }),
        "note.create" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Medium,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "LOCAL_RESTRICTED".into(),
            verifier_profile: "file_exists".into(),
        }),
        "system.time" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Low,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "NO_EXEC_READ_ONLY".into(),
            verifier_profile: "timestamp_present".into(),
        }),
        "conversation.reply" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Low,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "NO_EXEC_READ_ONLY".into(),
            verifier_profile: "conversation_reply".into(),
        }),
        "file.read_workspace" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Medium,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "NO_EXEC_READ_ONLY".into(),
            verifier_profile: "file_read".into(),
        }),
        "project.info" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Medium,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "NO_EXEC_READ_ONLY".into(),
            verifier_profile: "project_root".into(),
        }),
        "code.project_outline" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Medium,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "NO_EXEC_READ_ONLY".into(),
            verifier_profile: "project_root".into(),
        }),
        "docs.workspace_summary" => Some(CapabilityManifest {
            capability_id: capability.into(),
            version: "1.0.0".into(),
            risk: Risk::Medium,
            effect_scope: "DIGITAL_LOCAL".into(),
            requires_network: false,
            sandbox_profile: "NO_EXEC_READ_ONLY".into(),
            verifier_profile: "file_read".into(),
        }),
        _ => None,
    }
}
