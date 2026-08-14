//! Versioned, local-only desktop preferences. This file contains UI choices only; no prompts,
//! conversation content, attachment paths, credentials or model data are persisted here.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Dark,
    Light,
}

impl ThemePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "system" => Ok(Self::System),
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            _ => Err(format!("unknown desktop theme: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPreferences {
    pub schema_version: u16,
    pub theme: ThemePreference,
    pub font_scale_percent: u16,
    pub notifications_enabled: bool,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: ThemePreference::System,
            font_scale_percent: 100,
            notifications_enabled: true,
        }
    }
}

pub fn default_desktop_preferences_path() -> Option<PathBuf> {
    let config_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_root.join("jarvis").join("desktop.json"))
}

pub fn validate_desktop_preferences(preferences: &DesktopPreferences) -> Result<(), String> {
    if preferences.schema_version != 1 {
        return Err(format!(
            "unsupported desktop preferences schema version: {}",
            preferences.schema_version
        ));
    }
    if !(75..=175).contains(&preferences.font_scale_percent) {
        return Err("desktop font scale must be between 75 and 175 percent".into());
    }
    Ok(())
}

pub fn load_desktop_preferences(path: &Path) -> Result<DesktopPreferences, String> {
    if !path.exists() {
        return Ok(DesktopPreferences::default());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("desktop preferences cannot be read: {error}"))?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("desktop preferences are not valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "desktop preferences must be a JSON object".to_string())?;
    let schema_version = object
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| "desktop preferences schema_version is required".to_string())?;
    let theme = object
        .get("theme")
        .and_then(Value::as_str)
        .ok_or_else(|| "desktop preferences theme is required".to_string())
        .and_then(ThemePreference::parse)?;
    let font_scale_percent = object
        .get("font_scale_percent")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| "desktop preferences font_scale_percent is required".to_string())?;
    let notifications_enabled = object
        .get("notifications_enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| "desktop preferences notifications_enabled is required".to_string())?;
    let preferences = DesktopPreferences {
        schema_version,
        theme,
        font_scale_percent,
        notifications_enabled,
    };
    validate_desktop_preferences(&preferences)?;
    Ok(preferences)
}

/// Saves by replace-after-write so a truncated write never replaces a valid earlier config.
pub fn save_desktop_preferences(
    path: &Path,
    preferences: &DesktopPreferences,
) -> Result<(), String> {
    validate_desktop_preferences(preferences)?;
    let parent = path
        .parent()
        .ok_or_else(|| "desktop preferences path needs a parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("desktop preferences directory cannot be created: {error}"))?;
    let serialized = serde_json::to_string_pretty(&json!({
        "schema_version": preferences.schema_version,
        "theme": preferences.theme.as_str(),
        "font_scale_percent": preferences.font_scale_percent,
        "notifications_enabled": preferences.notifications_enabled,
    }))
    .map_err(|error| format!("desktop preferences serialization failed: {error}"))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, format!("{serialized}\n"))
        .map_err(|error| format!("desktop preferences temporary write failed: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("desktop preferences atomic replace failed: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_config_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "jarvis-desktop-config-{name}-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn absent_preferences_use_safe_defaults_and_round_trip() {
        let path = temporary_config_path("roundtrip");
        assert_eq!(
            load_desktop_preferences(&path).unwrap(),
            DesktopPreferences::default()
        );
        let preferences = DesktopPreferences {
            theme: ThemePreference::Dark,
            font_scale_percent: 115,
            notifications_enabled: false,
            ..DesktopPreferences::default()
        };
        save_desktop_preferences(&path, &preferences).expect("preferences save");
        assert_eq!(load_desktop_preferences(&path).unwrap(), preferences);
        fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn invalid_schema_scale_or_json_is_rejected_without_defaulting_silently() {
        let path = temporary_config_path("invalid");
        fs::write(&path, "not json").expect("invalid fixture");
        assert!(load_desktop_preferences(&path).is_err());
        let invalid = DesktopPreferences {
            font_scale_percent: 200,
            ..DesktopPreferences::default()
        };
        assert!(save_desktop_preferences(&path, &invalid).is_err());
        fs::remove_file(path).expect("fixture cleanup");
    }
}
