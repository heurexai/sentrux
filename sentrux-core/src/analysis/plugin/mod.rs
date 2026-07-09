//! Language plugin system — runtime-loaded tree-sitter grammars.
//!
//! Plugins live in ~/.sentrux/plugins/<lang>/ and follow the Sentrux Plugin Spec:
//! - plugin.toml (manifest with metadata, capabilities, checksums)
//! - grammars/<platform>.so|.dylib (compiled tree-sitter grammar)
//! - queries/tags.scm (tree-sitter queries for structural extraction)
//!
//! Plugins are loaded at startup and registered alongside built-in languages.
//! Plugin languages take priority over built-in (allows user overrides).

pub mod embedded;
pub mod loader;
pub mod manifest;
pub mod profile;

pub use loader::{
    default_plugins_dir, grammar_platform_key, grammar_sha256, load_all_plugins,
    load_grammar_dynamic, manifest_checksum_for_platform, plugins_dir, LoadedPlugin,
    PluginLoadError, IMMUTABLE_ANALYSIS_ENV, PLUGIN_ROOT_ENV,
};
pub use manifest::PluginManifest;
pub use profile::{
    ComplexityNodes, LanguageProfile, LanguageSemantics, LanguageThresholds, ProjectConfig,
    ResolverConfig, DEFAULT_PROFILE,
};

/// Silently sync embedded plugin configs to ~/.sentrux/plugins/ at startup.
/// Overwrites plugin.toml and tags.scm if the binary version is newer.
/// Preserves grammar .dylib files (expensive, platform-specific).
/// Users never need to think about plugin versions.
pub fn sync_embedded_plugins() {
    let dir = match plugins_dir() {
        Some(d) => d,
        None => return,
    };

    for &(name, toml_content, scm_content) in embedded::EMBEDDED_PLUGINS {
        let plugin_dir = dir.join(name);
        let toml_path = plugin_dir.join("plugin.toml");
        let scm_dir = plugin_dir.join("queries");
        let scm_path = scm_dir.join("tags.scm");

        let installed_toml = if toml_path.exists() {
            Some(std::fs::read_to_string(&toml_path).unwrap_or_default())
        } else {
            None
        };
        let target_toml = installed_toml
            .as_deref()
            .map(|installed| merge_existing_checksums(toml_content, installed))
            .unwrap_or_else(|| toml_content.to_string());

        // Check if config needs update: compare CONTENT, not just version.
        // This handles: grammar tarballs overwriting with old configs,
        // user corruption, any mismatch between embedded and installed.
        let needs_update = installed_toml
            .as_deref()
            .map(|installed| installed.trim() != target_toml.trim())
            .unwrap_or(true);
        let scm_needs_update = if scm_path.exists() && !scm_content.is_empty() {
            let installed_scm = std::fs::read_to_string(&scm_path).unwrap_or_default();
            installed_scm.trim() != scm_content.trim()
        } else {
            !scm_content.is_empty()
        };

        if !needs_update && !scm_needs_update {
            continue;
        }

        // Create directories
        let _ = std::fs::create_dir_all(&plugin_dir);
        let _ = std::fs::create_dir_all(&scm_dir);
        let _ = std::fs::create_dir_all(plugin_dir.join("grammars"));

        // Write plugin.toml and tags.scm — preserve grammar .dylib
        if needs_update {
            let _ = std::fs::write(&toml_path, &target_toml);
        }
        if scm_needs_update && !scm_content.is_empty() {
            let _ = std::fs::write(&scm_path, scm_content);
        }
    }
}

fn merge_existing_checksums(base_toml: &str, installed_toml: &str) -> String {
    let checksum_lines = extract_checksum_lines(installed_toml);
    if checksum_lines.is_empty() {
        return base_toml.to_string();
    }

    let mut output = Vec::new();
    let mut in_checksums = false;
    let mut inserted = false;
    for line in base_toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[checksums]" {
            output.push(line.to_string());
            output.extend(checksum_lines.iter().cloned());
            in_checksums = true;
            inserted = true;
            continue;
        }
        if in_checksums && trimmed.starts_with('[') {
            in_checksums = false;
        }
        if in_checksums {
            continue;
        }
        output.push(line.to_string());
    }
    if !inserted {
        output.push(String::new());
        output.push("[checksums]".to_string());
        output.extend(checksum_lines);
    }
    let mut merged = output.join("\n");
    if base_toml.ends_with('\n') {
        merged.push('\n');
    }
    merged
}

fn extract_checksum_lines(toml_content: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_checksums = false;
    for line in toml_content.lines() {
        let trimmed = line.trim();
        if trimmed == "[checksums]" {
            in_checksums = true;
            continue;
        }
        if in_checksums && trimmed.starts_with('[') {
            break;
        }
        if in_checksums && trimmed.contains('=') && !trimmed.starts_with('#') {
            lines.push(trimmed.to_string());
        }
    }
    lines
}
