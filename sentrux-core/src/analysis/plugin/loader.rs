//! Runtime plugin loader — discovers and loads language plugins from ~/.sentrux/plugins/.
//!
//! Each plugin directory contains:
//! - plugin.toml (manifest)
//! - grammars/<platform>.so|.dylib (compiled tree-sitter grammar)
//! - queries/tags.scm (tree-sitter queries)
//!
//! Loaded grammars are registered into the global LangRegistry alongside built-in languages.
//! Plugin languages take priority over built-in (allows user overrides).

use super::manifest::PluginManifest;
use super::profile::LanguageProfile;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use tree_sitter::Language;

/// Explicit plugin root for deterministic CLI analysis.
pub const PLUGIN_ROOT_ENV: &str = "SENTRUX_PLUGIN_ROOT";

/// When set, plugin loading must not copy bundled grammars or mutate user state.
pub const IMMUTABLE_ANALYSIS_ENV: &str = "SENTRUX_IMMUTABLE_ANALYSIS";

/// Result of loading a single plugin.
#[derive(Debug)]
pub struct LoadedPlugin {
    /// Plugin name from manifest
    pub name: String,
    /// Display name
    pub display_name: String,
    /// Version
    pub version: String,
    /// File extensions
    pub extensions: Vec<String>,
    /// Loaded tree-sitter grammar
    pub grammar: Language,
    /// Compiled tree-sitter query source
    pub query_src: String,
    /// Layer 2: language profile (semantics + thresholds)
    pub profile: LanguageProfile,
}

/// Error loading a plugin (non-fatal — logged and skipped).
#[derive(Debug)]
pub struct PluginLoadError {
    pub plugin_dir: PathBuf,
    pub error: String,
}

/// Get the default user plugins directory path (~/.sentrux/plugins/).
pub fn default_plugins_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".sentrux").join("plugins"))
}

/// Get the effective plugins directory path.
///
/// `SENTRUX_PLUGIN_ROOT` is intentionally honored here so CLI gates can be
/// anchored to a provisioned, immutable plugin inventory instead of ambient
/// user-home state.
pub fn plugins_dir() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os(PLUGIN_ROOT_ENV).filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(root));
    }
    default_plugins_dir()
}

/// Get the bundled plugins directory (next to the executable).
/// Used for distribution archives where grammars ship alongside the binary.
pub fn bundled_plugins_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("plugins")))
        .filter(|d| d.is_dir())
}

/// Discover and load all plugins from BOTH directories:
///   1. Bundled: <exe_dir>/plugins/ (grammars shipped with distribution)
///   2. User:   ~/.sentrux/plugins/ (configs from embedded sync + user plugins)
///
/// For each language, the grammar .dylib is searched in both locations.
/// The user dir's plugin.toml/tags.scm takes priority (embedded sync keeps them current).
pub fn load_all_plugins() -> (Vec<LoadedPlugin>, Vec<PluginLoadError>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();

    let dir = match plugins_dir() {
        Some(d) if d.is_dir() => d,
        _ => return (loaded, errors),
    };

    // If bundled plugins exist, copy any missing grammars to user dir.
    // Immutable CLI analysis must never repair or mutate plugin state.
    if std::env::var_os(IMMUTABLE_ANALYSIS_ENV).is_none() {
        if let Some(bundled) = bundled_plugins_dir() {
            copy_bundled_grammars(&bundled, &dir);
        }
    }

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            crate::debug_log!("[plugin] Failed to read plugins dir: {}", e);
            return (loaded, errors);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_single_plugin(&path) {
            Ok(plugin) => {
                // Verbose per-plugin logging removed — registry logs the total count
                loaded.push(plugin);
            }
            Err(e) => {
                crate::debug_log!("[plugin] Failed to load {}: {}", path.display(), e);
                errors.push(PluginLoadError {
                    plugin_dir: path,
                    error: e,
                });
            }
        }
    }

    (loaded, errors)
}

/// Load a single plugin from a directory.
fn load_single_plugin(plugin_dir: &Path) -> Result<LoadedPlugin, String> {
    // 1. Parse manifest
    let manifest = PluginManifest::load(plugin_dir)?;

    // 2. Load query source
    let query_path = plugin_dir.join("queries").join("tags.scm");
    let query_src = std::fs::read_to_string(&query_path)
        .map_err(|e| format!("Failed to read {}: {}", query_path.display(), e))?;

    // 3. Validate query captures match declared capabilities
    manifest.validate_query_captures(&query_src)?;

    // 4. Load grammar binary
    let grammar_file = PluginManifest::grammar_filename();
    if grammar_file == "unsupported" {
        return Err("Unsupported platform for runtime grammar loading".into());
    }
    let grammar_path = plugin_dir.join("grammars").join(grammar_file);
    if !grammar_path.exists() {
        return Err(format!(
            "Grammar binary not found: {}. Build it for this platform.",
            grammar_path.display()
        ));
    }

    // 5. Verify checksum if provided
    verify_checksum(&manifest, &grammar_path, grammar_file)?;

    // 6. Load the grammar via dynamic library
    let symbol_name = manifest
        .grammar
        .symbol_name
        .as_deref()
        .unwrap_or(&manifest.plugin.name);
    let grammar = load_grammar_dynamic(&grammar_path, symbol_name)?;

    // 7. Verify ABI version
    #[allow(deprecated)]
    let abi = grammar.version();
    if abi < manifest.grammar.abi_version as usize {
        return Err(format!(
            "Grammar ABI version {} < required {}",
            abi, manifest.grammar.abi_version
        ));
    }

    // 8. Test-compile the query to catch errors early
    tree_sitter::Query::new(&grammar, &query_src)
        .map_err(|e| format!("Query compilation failed: {:?}", e))?;

    let profile = LanguageProfile {
        name: manifest.plugin.name.clone(),
        semantics: manifest.semantics,
        thresholds: manifest.thresholds,
        color_rgb: manifest.plugin.color_rgb.unwrap_or([80, 85, 90]),
    };

    Ok(LoadedPlugin {
        name: manifest.plugin.name,
        display_name: manifest.plugin.display_name,
        version: manifest.plugin.version,
        extensions: manifest.plugin.extensions,
        grammar,
        query_src,
        profile,
    })
}

/// Verify SHA256 checksum of grammar binary against manifest.
pub fn grammar_platform_key() -> &'static str {
    let filename = PluginManifest::grammar_filename();
    filename.rsplit_once('.').map_or(filename, |(key, _)| key)
}

/// Return the manifest checksum value for the current platform, if present.
pub fn manifest_checksum_for_platform<'a>(manifest: &'a PluginManifest) -> Option<&'a str> {
    manifest
        .checksums
        .get(grammar_platform_key())
        .map(String::as_str)
}

/// Compute SHA-256 for a grammar binary.
pub fn grammar_sha256(grammar_path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(grammar_path)
        .map_err(|e| format!("Failed to read grammar for checksum: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("Failed to read grammar for checksum: {}", e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verify SHA256 checksum of grammar binary against manifest.
fn verify_checksum(
    manifest: &PluginManifest,
    grammar_path: &Path,
    platform_key: &str,
) -> Result<(), String> {
    let key = platform_key
        .rsplit_once('.')
        .map_or(platform_key, |(k, _)| k);
    let expected = match manifest.checksums.get(key) {
        Some(hash) if !hash.trim().is_empty() => hash.trim().to_ascii_lowercase(),
        None => return Ok(()), // No checksum in manifest = skip verification
        Some(_) => return Ok(()),
    };

    let actual = grammar_sha256(grammar_path)?;
    if actual != expected {
        return Err(format!(
            "Checksum mismatch for {}: expected {}, got {}",
            grammar_path.display(),
            expected,
            actual
        ));
    }
    Ok(())
}

/// Load a tree-sitter Language from a dynamic library (.so/.dylib).
///
/// The library must export a function named `tree_sitter_<name>` that returns
/// a `*const TSLanguage` pointer. This is the standard tree-sitter convention.
pub fn load_grammar_dynamic(path: &Path, lang_name: &str) -> Result<Language, String> {
    // Safety: we're loading a tree-sitter grammar .so/.dylib which exports
    // a single `tree_sitter_<name>()` function returning *const TSLanguage.
    // This is the same mechanism nvim-treesitter, helix, and zed use.
    unsafe {
        let lib = libloading::Library::new(path)
            .map_err(|e| format!("Failed to load {}: {}", path.display(), e))?;

        // tree-sitter convention: exported function is `tree_sitter_<name>`
        let func_name = format!("tree_sitter_{}", lang_name);
        let func: libloading::Symbol<unsafe extern "C" fn() -> Language> =
            lib.get(func_name.as_bytes()).map_err(|e| {
                format!(
                    "Symbol '{}' not found in {}: {}. The grammar must export tree_sitter_{}().",
                    func_name,
                    path.display(),
                    e,
                    lang_name
                )
            })?;

        let language = func();

        // Leak the library to keep it alive for the lifetime of the process.
        // tree-sitter Language holds pointers into the library's memory.
        std::mem::forget(lib);

        Ok(language)
    }
}

/// Copy grammar .dylib files from bundled distribution to user plugins dir.
/// Only copies if the user dir doesn't already have the grammar.
/// This handles: user extracts distribution → first launch → grammars copied.
fn copy_bundled_grammars(bundled_dir: &Path, user_dir: &Path) {
    let grammar_file = PluginManifest::grammar_filename();
    let entries = match std::fs::read_dir(bundled_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bundled_grammar = path.join("grammars").join(grammar_file);
        let user_grammar = user_dir.join(&name).join("grammars").join(grammar_file);
        if bundled_grammar.exists() && !user_grammar.exists() {
            let _ = std::fs::create_dir_all(user_dir.join(&name).join("grammars"));
            if std::fs::copy(&bundled_grammar, &user_grammar).is_ok() {
                crate::debug_log!("[plugin] Copied bundled grammar: {}", name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugins_dir() {
        let dir = plugins_dir();
        assert!(dir.is_some());
        assert!(dir.unwrap().ends_with(".sentrux/plugins"));
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let (loaded, errors) = load_all_plugins();
        // Should not crash even if dir doesn't exist
        let _ = (loaded, errors);
    }

    /// Diagnostic: dump all node types for grammars that fail to load.
    /// Run: cargo test dump_failing_grammar_nodes -- --ignored --nocapture
    #[test]
    #[ignore]
    fn dump_failing_grammar_nodes() {
        let dir = plugins_dir().unwrap();
        // Only dump languages that are NOT currently loaded (to avoid test pollution)
        let failing: [&str; 0] = [];
        for name in &failing {
            let plugin_dir = dir.join(name);
            let grammar_file = PluginManifest::grammar_filename();
            let grammar_path = plugin_dir.join("grammars").join(grammar_file);
            if !grammar_path.exists() {
                println!("\nSKIP {} — no grammar", name);
                continue;
            }
            // Try loading with the plugin name, then with symbol_name from toml
            let symbol = if let Ok(manifest) = PluginManifest::load(&plugin_dir) {
                manifest.grammar.symbol_name.unwrap_or(name.to_string())
            } else {
                name.to_string()
            };
            match load_grammar_dynamic(&grammar_path, &symbol) {
                Ok(lang) => {
                    println!(
                        "\n=== {} ({} node types, symbol: tree_sitter_{}) ===",
                        name,
                        lang.node_kind_count(),
                        symbol
                    );
                    for id in 0..lang.node_kind_count() as u16 {
                        if lang.node_kind_is_named(id) {
                            let kind = lang.node_kind_for_id(id).unwrap_or("?");
                            // Also check fields
                            println!("  {}", kind);
                        }
                    }
                    // Dump field names
                    println!("  --- fields ---");
                    for id in 0..lang.field_count() as u16 {
                        if let Some(fname) = lang.field_name_for_id(id) {
                            println!("  field: {}", fname);
                        }
                    }
                }
                Err(e) => println!("\nFAIL {}: {}", name, e),
            }
        }
    }
}
