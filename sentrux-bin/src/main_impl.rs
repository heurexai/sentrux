//! Sentrux binary — GUI, CLI, and MCP entry points.
//!
//! All logic lives in `sentrux-core`. This crate is just the entry point
//! that wires together the three modes:
//! - GUI mode (default): interactive treemap/blueprint visualizer
//! - MCP mode (`sentrux mcp`): Model Context Protocol server for AI agent integration
//! - Check mode (`sentrux check [path]`): CLI architectural rules enforcement
//! - Gate mode (`sentrux gate [--save] [path]`): structural regression testing

use clap::{Parser, Subcommand};
use sentrux_core::analysis;
use sentrux_core::app;
use sentrux_core::core;
use sentrux_core::metrics;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

fn edition_name() -> &'static str {
    let tier = sentrux_core::license::current_tier();
    if tier >= sentrux_core::license::Tier::Pro {
        "Pro"
    } else {
        "" // Don't show "Free" or "Community" — just "sentrux"
    }
}

const FORK_STAMP: &str = "Heurex fork";

fn version_string() -> &'static str {
    use std::sync::OnceLock;
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let edition = edition_name();
        let base = if edition.is_empty() {
            format!("{} ({})", env!("CARGO_PKG_VERSION"), FORK_STAMP)
        } else {
            format!(
                "{} ({}, {})",
                env!("CARGO_PKG_VERSION"),
                FORK_STAMP,
                edition
            )
        };
        if let Some(latest) = sentrux_core::app::update_check::available_update() {
            format!(
                "{}\n  Update available: v{} → brew upgrade sentrux",
                base, latest
            )
        } else {
            base
        }
    })
}

#[derive(Parser)]
#[command(
    name = "sentrux",
    about = "Live codebase visualization and structural quality gate (Heurex fork)",
    long_about = "\
Live codebase visualization and structural quality gate (Heurex fork).

Agent quick start:
  Current structural assessment:
    sentrux check --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>

  Baseline regression / gate failure:
    sentrux gate --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>

  Verify provisioned plugins before running a Praxis/scaffold gate:
    sentrux plugin verify --json --plugin-root <plugins> --require-language csharp

  Create or refresh a baseline intentionally:
    sentrux gate --save --json --plugin-root <plugins> --require-language csharp <repo>

Important JSON paths for agents:
  check --json:
    pass
    analysis.complete
    analysis.fatalDiagnostics[]
    analysis.inventory.languages[]
    analysis.structuralCoverage.unparsedCodeFiles[]
    scan.include_untracked
    violations[]
    metrics.godFiles.files[]
    metrics.coupling.problemEdges[]
    metrics.cycles.cycles[]
    metrics.complexFunctions.functions[]
    metrics.depth.deepestFiles[]

  gate --json:
    passed
    analysis.complete
    analysis.fatalDiagnostics[]
    analysis.inventory.languages[]
    analysis.structuralCoverage.unparsedCodeFiles[]
    scan.include_untracked
    degradations[]
    hardMetricFailureDespiteQualityImprovement
    metrics.godFiles.addedGodFiles[]
    metrics.coupling.offenders.added[]
    metrics.cycles.cycles.added[]
    metrics.complexFunctions.functions.addedFunctions[]

Exit codes:
  0  no blocking assessment or gate failure
  1  rules failed, structural degradation detected, incomplete analysis, scan failed, or baseline missing
  2  invalid command line usage

Config and state files:
  .sentrux/rules.toml       rules for `check`
  .sentruxignore            scan exclusions, including tracked files
  .sentrux/baseline.json    baseline for `gate`

Tip: when debugging why a gate failed before commit, prefer
`sentrux gate --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>`
first so brand-new files are part of the regression scan and required language
analysis is proven complete. If the baseline is old and lacks offender details,
run `sentrux check --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>`
to inspect the current offenders, then refresh the baseline only after the
operator decides the new structure is acceptable.",
    version = version_string(),
    arg_required_else_help = false,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Directory to open in the GUI
    #[arg(global = false)]
    path: Option<String>,

    /// Start MCP server (hidden alias for `sentrux mcp`)
    #[arg(long = "mcp", hide = true)]
    mcp_flag: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Enforce architectural rules defined in .sentrux/rules.toml
    #[command(long_about = "\
Enforce architectural rules defined in .sentrux/rules.toml.

Agent command:
  sentrux check --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>

Use `check` when the agent needs the current assessment, independent of any
saved baseline. This is the best command for answering: what files or edges are
currently causing Sentrux to fail?

Important JSON paths:
  pass
  analysis.complete
  analysis.fatalDiagnostics[]
  analysis.inventory.languages[]
  analysis.structuralCoverage.requiredLanguages[]
  analysis.structuralCoverage.unparsedCodeFiles[]
  rules_checked
  scan.include_untracked
  csharp_references.unresolved
  violations[]
  metrics.quality.rootCauses
  metrics.godFiles.files[]
  metrics.coupling.problemEdges[]
  metrics.cycles.cycles[]
  metrics.depth.deepestFiles[]
  metrics.complexFunctions.functions[]
  metrics.longFunctions.functions[]
  metrics.largeFiles.files[]
  metrics.duplicates.groups[]
  metrics.deadFunctions.functions[]

Typical debug flow:
  1. Run with `--json --include-untracked`.
  2. If `pass` is false, inspect `violations[]` first.
  3. For god-file failures, inspect `metrics.godFiles.files[]`.
  4. For cycle failures, inspect `metrics.cycles.cycles[].edge_chain`.
  5. For coupling failures, inspect `metrics.coupling.problemEdges[]`.
  6. For quality/root-cause failures, inspect `metrics.quality.rootCauses`
     plus the matching offender lists above.

Exit code:
  0  all checked rules passed
  1  one or more rules failed, analysis was incomplete, or scanning failed
  2  invalid command line usage

EXCLUSIONS (.sentruxignore) — Heurex fork:
A `.sentruxignore` file placed at the scan root excludes matching paths from the
scan and the quality graph. It uses full gitignore syntax:
  - directory entries        e.g.  generated/
  - filename globs           e.g.  *.generated.cs
  - nested globs             e.g.  **/*.designer.cs
  - negation (re-include)    e.g.  !keep.cs
Unlike .gitignore, .sentruxignore excludes matching files EVEN WHEN THEY ARE
GIT-TRACKED. A missing .sentruxignore means no exclusions (it is not an error).
This is file-based; there is no CLI flag for it.")]
    Check {
        /// Directory to check
        #[arg(default_value = ".")]
        path: String,

        /// Include untracked, non-ignored Git worktree files in the scan
        #[arg(long)]
        include_untracked: bool,

        /// Explicit provisioned plugin root to use for immutable analysis
        #[arg(long, value_name = "DIR")]
        plugin_root: Option<String>,

        /// Language that must have a verified plugin and complete structural coverage
        #[arg(long = "require-language", value_name = "LANG")]
        require_languages: Vec<String>,

        /// Emit machine-readable JSON diagnostics
        #[arg(long)]
        json: bool,
    },

    /// Structural regression gate — compare against a saved baseline
    #[command(long_about = "\
Structural regression gate: compare the current scan against
.sentrux/baseline.json.

Agent command for a gate failure:
  sentrux gate --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>

Use `gate` when the agent needs to explain a regression relative to the saved
baseline. It is the best command for answering: what changed since the baseline
that made Sentrux fail?

Important JSON paths:
  passed
  analysis.complete
  analysis.fatalDiagnostics[]
  analysis.inventory.languages[]
  analysis.structuralCoverage.requiredLanguages[]
  analysis.structuralCoverage.unparsedCodeFiles[]
  scan.include_untracked
  quality.before / quality.after / quality.delta
  degradations[]
  hardMetricFailureDespiteQualityImprovement
  metrics.godFiles.addedGodFiles[]
  metrics.godFiles.removedGodFiles[]
  metrics.godFiles.persistingGodFiles[]
  metrics.godFiles.changedRankOrScoreGodFiles[]
  metrics.coupling.offenders.added[]
  metrics.coupling.offenders.removed[]
  metrics.coupling.offenders.current[]
  metrics.cycles.cycles.added[]
  metrics.cycles.cycles.current[]
  metrics.complexFunctions.functions.addedFunctions[]
  metrics.complexFunctions.functions.current[]

Typical debug flow:
  1. Run `sentrux gate --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>`
     when debugging a pre-commit or worktree gate failure.
  2. Inspect `degradations[]` to identify the failing hard metric.
  3. Inspect the matching `added` offender list for the root cause.
  4. If `baselineDetailsAvailable` is false for a metric, the baseline was
     created by an older Sentrux that only stored aggregate counts. In that
     case run `sentrux check --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>`
     to inspect the current offenders, then refresh the baseline only after the
     operator accepts the new structure.

Create or refresh a baseline intentionally:
  sentrux gate --save --json --plugin-root <plugins> --require-language csharp <repo>

Exit code:
  0  current structure did not regress against the baseline
  1  degradation detected, incomplete analysis, baseline missing/unreadable, or scan failed
  2  invalid command line usage")]
    Gate {
        /// Save current metrics as the new baseline
        #[arg(long)]
        save: bool,

        /// Include untracked, non-ignored Git worktree files in the scan
        #[arg(long)]
        include_untracked: bool,

        /// Explicit provisioned plugin root to use for immutable analysis
        #[arg(long, value_name = "DIR")]
        plugin_root: Option<String>,

        /// Language that must have a verified plugin and complete structural coverage
        #[arg(long = "require-language", value_name = "LANG")]
        require_languages: Vec<String>,

        /// Emit machine-readable JSON diagnostics
        #[arg(long)]
        json: bool,

        /// Directory to gate
        #[arg(default_value = ".")]
        path: String,
    },

    /// Open the GUI with a pre-loaded directory
    Scan {
        /// Directory to visualize
        path: Option<String>,
    },

    /// Start the MCP (Model Context Protocol) server for AI agent integration
    Mcp,

    /// Manage language plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },

    /// Control anonymous aggregate usage analytics
    Analytics {
        #[command(subcommand)]
        action: Option<AnalyticsAction>,
    },

    /// Open browser to purchase / sign in for Sentrux Pro
    Login,

    /// Manage Pro license and plugin
    Pro {
        #[command(subcommand)]
        action: ProAction,
    },
}

#[derive(Subcommand)]
enum ProAction {
    /// Activate Pro with a license key
    Activate {
        /// License key JSON string or path to key file
        key: String,
    },
    /// Show Pro license status
    Status,
    /// Deactivate Pro (remove license + plugin)
    Deactivate,
    /// Update Pro plugin to latest version
    Update,
}

#[derive(Subcommand)]
enum AnalyticsAction {
    /// Turn analytics on
    On,
    /// Turn analytics off
    Off,
}

#[derive(Subcommand)]
enum PluginAction {
    /// List installed plugins
    List,

    /// Install all standard language plugins
    AddStandard,

    /// Install a single language plugin from the plugin registry
    Add {
        /// Plugin name (e.g. "python", "rust")
        name: String,
    },

    /// Remove an installed plugin
    Remove {
        /// Plugin name to remove
        name: String,
    },

    /// Create a new plugin template
    Init {
        /// Language name for the new plugin
        name: String,
    },

    /// Validate a plugin directory
    Validate {
        /// Path to the plugin directory
        dir: String,
    },

    /// Verify a provisioned plugin root without downloading or mutating it
    Verify {
        /// Emit machine-readable JSON diagnostics
        #[arg(long)]
        json: bool,

        /// Explicit provisioned plugin root to verify
        #[arg(long, value_name = "DIR")]
        plugin_root: Option<String>,

        /// Language that must be present, checksum-verified, loadable, and query-valid
        #[arg(long = "require-language", value_name = "LANG")]
        require_languages: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

pub fn run() -> eframe::Result<()> {
    let cli = Cli::parse();
    let immutable_analysis = command_requires_immutable_analysis(&cli.command);

    if immutable_analysis {
        std::env::set_var(sentrux_core::analysis::plugin::IMMUTABLE_ANALYSIS_ENV, "1");
    } else {
        // Initialize license + Pro plugin (reads ~/.sentrux/license.key, loads pro.dylib if valid)
        sentrux_core::license::init();

        // Step 1: Download missing grammar binaries (may overwrite configs with old versions)
        ensure_grammars_installed();

        // Step 2: Sync embedded plugin configs LAST — always wins over downloaded configs.
        // This ensures configs match the binary version even if the grammar tarball
        // included old plugin.toml/tags.scm files.
        sentrux_core::analysis::plugin::sync_embedded_plugins();

        // Non-blocking update check (once per day, background thread)
        app::update_check::check_for_updates_async(env!("CARGO_PKG_VERSION"));
    }

    // Hidden --mcp flag for backward compat with MCP client configs
    if cli.mcp_flag {
        app::mcp_server::run_mcp_server(None);
        return Ok(());
    }

    match cli.command {
        Some(Command::Check {
            path,
            include_untracked,
            plugin_root,
            require_languages,
            json,
        }) => {
            std::process::exit(run_check(
                &path,
                include_untracked,
                json,
                CliAnalysisOptions::new(plugin_root, require_languages),
            ));
        }
        Some(Command::Gate {
            save,
            include_untracked,
            plugin_root,
            require_languages,
            json,
            path,
        }) => {
            std::process::exit(run_gate(
                &path,
                save,
                include_untracked,
                json,
                CliAnalysisOptions::new(plugin_root, require_languages),
            ));
        }
        Some(Command::Mcp) => {
            app::mcp_server::run_mcp_server(None);
            Ok(())
        }
        Some(Command::Plugin { action }) => {
            run_plugin(action);
            Ok(())
        }
        Some(Command::Analytics { action }) => {
            run_analytics(action);
            Ok(())
        }
        Some(Command::Login) => {
            run_login();
            Ok(())
        }
        Some(Command::Pro { action }) => {
            run_pro(action);
            Ok(())
        }
        Some(Command::Scan { path }) => run_gui(path),
        None => run_gui(cli.path),
    }
}

fn command_requires_immutable_analysis(command: &Option<Command>) -> bool {
    matches!(
        command,
        Some(Command::Check { .. })
            | Some(Command::Gate { .. })
            | Some(Command::Plugin {
                action: PluginAction::Verify { .. },
            })
    )
}

#[derive(Debug, Clone)]
struct CliAnalysisOptions {
    plugin_root: Option<String>,
    require_languages: Vec<String>,
}

impl CliAnalysisOptions {
    fn new(plugin_root: Option<String>, require_languages: Vec<String>) -> Self {
        let mut seen = std::collections::BTreeSet::new();
        let require_languages = require_languages
            .into_iter()
            .map(|lang| lang.trim().to_ascii_lowercase())
            .filter(|lang| !lang.is_empty())
            .filter(|lang| seen.insert(lang.clone()))
            .collect();
        Self {
            plugin_root,
            require_languages,
        }
    }
}

#[derive(Debug, Clone)]
struct AnalysisDiagnostic {
    code: String,
    severity: String,
    message: String,
    language: Option<String>,
    path: Option<String>,
    expected_grammar: Option<String>,
    plugin_root: Option<String>,
    details: serde_json::Value,
}

impl AnalysisDiagnostic {
    fn fatal(code: &str, message: impl Into<String>) -> Self {
        Self::new(code, "fatal", message)
    }

    fn warning(code: &str, message: impl Into<String>) -> Self {
        Self::new(code, "warning", message)
    }

    fn new(code: &str, severity: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: severity.to_string(),
            message: message.into(),
            language: None,
            path: None,
            expected_grammar: None,
            plugin_root: None,
            details: serde_json::Value::Null,
        }
    }

    fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    fn expected_grammar(mut self, expected_grammar: impl Into<String>) -> Self {
        self.expected_grammar = Some(expected_grammar.into());
        self
    }

    fn plugin_root(mut self, plugin_root: Option<&Path>) -> Self {
        self.plugin_root = plugin_root.map(|path| path.display().to_string());
        self
    }

    fn details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    fn is_fatal(&self) -> bool {
        self.severity == "fatal"
    }

    fn to_json(&self) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert("code".to_string(), serde_json::json!(&self.code));
        object.insert("severity".to_string(), serde_json::json!(&self.severity));
        object.insert("message".to_string(), serde_json::json!(&self.message));
        if let Some(language) = &self.language {
            object.insert("language".to_string(), serde_json::json!(language));
        }
        if let Some(path) = &self.path {
            object.insert("path".to_string(), serde_json::json!(path));
        }
        if let Some(expected_grammar) = &self.expected_grammar {
            object.insert(
                "expectedGrammar".to_string(),
                serde_json::json!(expected_grammar),
            );
        }
        if let Some(plugin_root) = &self.plugin_root {
            object.insert("pluginRoot".to_string(), serde_json::json!(plugin_root));
        }
        if !self.details.is_null() {
            object.insert("details".to_string(), self.details.clone());
        }
        serde_json::Value::Object(object)
    }
}

#[derive(Debug, Clone)]
struct PluginInventoryItem {
    language: String,
    status: String,
    path: String,
    display_name: Option<String>,
    version: Option<String>,
    extensions: Vec<String>,
    expected_grammar: String,
    grammar_path: Option<String>,
    grammar_sha256: Option<String>,
    checksum_expected: Option<String>,
    checksum_actual: Option<String>,
    checksum_verified: Option<bool>,
    diagnostics: Vec<AnalysisDiagnostic>,
}

impl PluginInventoryItem {
    fn new(language: &str, plugin_dir: &Path) -> Self {
        Self {
            language: language.to_string(),
            status: "unknown".to_string(),
            path: plugin_dir.display().to_string(),
            display_name: None,
            version: None,
            extensions: Vec::new(),
            expected_grammar: sentrux_core::analysis::plugin::PluginManifest::grammar_filename()
                .to_string(),
            grammar_path: None,
            grammar_sha256: None,
            checksum_expected: None,
            checksum_actual: None,
            checksum_verified: None,
            diagnostics: Vec::new(),
        }
    }

    fn finalize_status(&mut self) {
        if self.diagnostics.iter().any(AnalysisDiagnostic::is_fatal) {
            self.status = "failed".to_string();
        } else if self.status == "unknown" {
            self.status = "ok".to_string();
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "language": &self.language,
            "status": &self.status,
            "path": &self.path,
            "displayName": &self.display_name,
            "version": &self.version,
            "extensions": &self.extensions,
            "expectedGrammar": &self.expected_grammar,
            "grammarPath": &self.grammar_path,
            "grammarSha256": &self.grammar_sha256,
            "checksumExpected": &self.checksum_expected,
            "checksumActual": &self.checksum_actual,
            "checksumVerified": self.checksum_verified,
            "diagnostics": self.diagnostics.iter().map(AnalysisDiagnostic::to_json).collect::<Vec<_>>()
        })
    }
}

#[derive(Debug, Clone)]
struct AnalysisContext {
    plugin_root: Option<PathBuf>,
    plugin_root_source: String,
    required_languages: Vec<String>,
    inventory: Vec<PluginInventoryItem>,
    fatal_diagnostics: Vec<AnalysisDiagnostic>,
    warnings: Vec<AnalysisDiagnostic>,
}

impl AnalysisContext {
    fn complete_without_coverage(&self) -> bool {
        self.fatal_diagnostics.is_empty()
    }
}

#[derive(Debug, Clone)]
struct StructuralCoverage {
    total_files: u32,
    code_files: usize,
    parsed_files: usize,
    unparsed_files: usize,
    parse_coverage: f64,
    function_count: usize,
    class_count: usize,
    import_edges: usize,
    call_edges: usize,
    required_languages: Vec<serde_json::Value>,
    unparsed_code_files: Vec<serde_json::Value>,
    fatal_diagnostics: Vec<AnalysisDiagnostic>,
}

impl StructuralCoverage {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "totalFiles": self.total_files,
            "codeFiles": self.code_files,
            "parsedFiles": self.parsed_files,
            "unparsedFiles": self.unparsed_files,
            "parseCoverage": self.parse_coverage,
            "functionCount": self.function_count,
            "classCount": self.class_count,
            "importEdges": self.import_edges,
            "callEdges": self.call_edges,
            "requiredLanguages": &self.required_languages,
            "unparsedCodeFiles": &self.unparsed_code_files
        })
    }
}

fn prepare_analysis_context(options: &CliAnalysisOptions) -> AnalysisContext {
    let (plugin_root, plugin_root_source) = select_plugin_root(options.plugin_root.as_deref());
    let mut context = AnalysisContext {
        plugin_root,
        plugin_root_source,
        required_languages: options.require_languages.clone(),
        inventory: Vec::new(),
        fatal_diagnostics: Vec::new(),
        warnings: Vec::new(),
    };

    if context.plugin_root_source == "default" {
        context.warnings.push(
            AnalysisDiagnostic::warning(
                "SENTRUX-PLUGIN-ROOT-DEFAULTED",
                "No --plugin-root or SENTRUX_PLUGIN_ROOT was provided; using the default user plugin root.",
            )
            .plugin_root(context.plugin_root.as_deref()),
        );
    }

    let Some(root) = context.plugin_root.clone() else {
        if !context.required_languages.is_empty() {
            context.fatal_diagnostics.push(AnalysisDiagnostic::fatal(
                "SENTRUX-PLUGIN-ROOT-MISSING",
                "No plugin root is available, but required languages were requested.",
            ));
        }
        return context;
    };

    if !root.is_dir() {
        let diagnostic = AnalysisDiagnostic::fatal(
            "SENTRUX-PLUGIN-ROOT-MISSING",
            format!(
                "Plugin root does not exist or is not a directory: {}",
                root.display()
            ),
        )
        .plugin_root(Some(&root));
        if context.required_languages.is_empty() {
            context.warnings.push(AnalysisDiagnostic {
                severity: "warning".to_string(),
                ..diagnostic
            });
        } else {
            context.fatal_diagnostics.push(diagnostic);
        }
        return context;
    }

    let languages = if context.required_languages.is_empty() {
        discover_plugin_languages(&root)
    } else {
        context.required_languages.clone()
    };

    for language in languages {
        let strict = context
            .required_languages
            .iter()
            .any(|lang| lang == &language);
        let item = verify_plugin_language(&root, &language, strict);
        if strict {
            for diagnostic in &item.diagnostics {
                if diagnostic.is_fatal() {
                    context.fatal_diagnostics.push(diagnostic.clone());
                } else {
                    context.warnings.push(diagnostic.clone());
                }
            }
        }
        context.inventory.push(item);
    }

    context
}

fn apply_cli_plugin_root(options: &CliAnalysisOptions) {
    if let Some(root) = options
        .plugin_root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
    {
        std::env::set_var(sentrux_core::analysis::plugin::PLUGIN_ROOT_ENV, root);
    }
}

fn select_plugin_root(cli_root: Option<&str>) -> (Option<PathBuf>, String) {
    if let Some(root) = cli_root.map(str::trim).filter(|root| !root.is_empty()) {
        return (Some(PathBuf::from(root)), "cli".to_string());
    }
    if let Some(root) =
        std::env::var_os(sentrux_core::analysis::plugin::PLUGIN_ROOT_ENV).filter(|v| !v.is_empty())
    {
        return (Some(PathBuf::from(root)), "env".to_string());
    }
    (
        sentrux_core::analysis::plugin::default_plugins_dir(),
        "default".to_string(),
    )
}

fn discover_plugin_languages(root: &Path) -> Vec<String> {
    let mut languages = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("plugin.toml").exists() {
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    languages.push(name.to_ascii_lowercase());
                }
            }
        }
    }
    languages.sort();
    languages
}

fn verify_plugin_language(root: &Path, language: &str, strict: bool) -> PluginInventoryItem {
    let plugin_dir = root.join(language);
    let mut item = PluginInventoryItem::new(language, &plugin_dir);
    let expected_grammar =
        sentrux_core::analysis::plugin::PluginManifest::grammar_filename().to_string();

    if !plugin_dir.is_dir() {
        item.status = "missingPlugin".to_string();
        item.diagnostics.push(
            AnalysisDiagnostic::fatal(
                "SENTRUX-LANGUAGE-PLUGIN-MISSING",
                format!("Required language plugin is missing: {language}"),
            )
            .language(language)
            .path(plugin_dir.display().to_string())
            .expected_grammar(expected_grammar)
            .plugin_root(Some(root)),
        );
        item.finalize_status();
        return item;
    }

    let manifest_path = plugin_dir.join("plugin.toml");
    if !manifest_path.exists() {
        item.status = "missingManifest".to_string();
        item.diagnostics.push(
            AnalysisDiagnostic::fatal(
                "SENTRUX-PLUGIN-MANIFEST-MISSING",
                format!("Plugin manifest is missing for language {language}."),
            )
            .language(language)
            .path(manifest_path.display().to_string())
            .plugin_root(Some(root)),
        );
        item.finalize_status();
        return item;
    }

    let manifest = match sentrux_core::analysis::plugin::PluginManifest::load(&plugin_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            item.status = "manifestInvalid".to_string();
            item.diagnostics.push(
                AnalysisDiagnostic::fatal(
                    "SENTRUX-PLUGIN-MANIFEST-INVALID",
                    format!("Plugin manifest is invalid for language {language}: {error}"),
                )
                .language(language)
                .path(manifest_path.display().to_string())
                .plugin_root(Some(root)),
            );
            item.finalize_status();
            return item;
        }
    };

    item.display_name = Some(manifest.plugin.display_name.clone());
    item.version = Some(manifest.plugin.version.clone());
    item.extensions = manifest.plugin.extensions.clone();

    let query_path = plugin_dir.join("queries").join("tags.scm");
    let query_src = match std::fs::read_to_string(&query_path) {
        Ok(query_src) => query_src,
        Err(error) => {
            item.status = "missingQuery".to_string();
            item.diagnostics.push(
                AnalysisDiagnostic::fatal(
                    "SENTRUX-QUERY-MISSING",
                    format!("Query file is missing for language {language}: {error}"),
                )
                .language(language)
                .path(query_path.display().to_string())
                .plugin_root(Some(root)),
            );
            item.finalize_status();
            return item;
        }
    };

    if let Err(error) = manifest.validate_query_captures(&query_src) {
        item.status = "queryInvalid".to_string();
        item.diagnostics.push(
            AnalysisDiagnostic::fatal(
                "SENTRUX-QUERY-INVALID",
                format!("Query captures are invalid for language {language}: {error}"),
            )
            .language(language)
            .path(query_path.display().to_string())
            .plugin_root(Some(root)),
        );
    }

    if expected_grammar == "unsupported" {
        item.status = "unsupportedPlatform".to_string();
        item.diagnostics.push(
            AnalysisDiagnostic::fatal(
                "SENTRUX-GRAMMAR-UNSUPPORTED-PLATFORM",
                "This platform has no supported grammar filename.",
            )
            .language(language)
            .plugin_root(Some(root)),
        );
        item.finalize_status();
        return item;
    }

    let grammar_path = plugin_dir.join("grammars").join(&expected_grammar);
    item.grammar_path = Some(grammar_path.display().to_string());
    if !grammar_path.exists() {
        item.status = "missingGrammar".to_string();
        item.diagnostics.push(
            AnalysisDiagnostic::fatal(
                "SENTRUX-GRAMMAR-MISSING",
                format!(
                    "Grammar binary is missing for required language {language}: {}",
                    grammar_path.display()
                ),
            )
            .language(language)
            .path(grammar_path.display().to_string())
            .expected_grammar(expected_grammar)
            .plugin_root(Some(root)),
        );
        item.finalize_status();
        return item;
    }

    match sentrux_core::analysis::plugin::grammar_sha256(&grammar_path) {
        Ok(actual) => {
            item.grammar_sha256 = Some(actual.clone());
            item.checksum_actual = Some(actual.clone());
            let expected =
                sentrux_core::analysis::plugin::manifest_checksum_for_platform(&manifest)
                    .map(str::trim)
                    .filter(|hash| !hash.is_empty())
                    .map(str::to_ascii_lowercase);
            item.checksum_expected = expected.clone();
            match expected {
                Some(expected) if expected == actual => item.checksum_verified = Some(true),
                Some(expected) => {
                    item.checksum_verified = Some(false);
                    item.status = "checksumInvalid".to_string();
                    item.diagnostics.push(
                        AnalysisDiagnostic::fatal(
                            "SENTRUX-GRAMMAR-CHECKSUM-MISMATCH",
                            format!(
                                "Grammar checksum mismatch for {language}: expected {expected}, got {actual}."
                            ),
                        )
                        .language(language)
                        .path(grammar_path.display().to_string())
                        .expected_grammar(
                            sentrux_core::analysis::plugin::PluginManifest::grammar_filename(),
                        )
                        .plugin_root(Some(root))
                        .details(serde_json::json!({
                            "expectedSha256": expected,
                            "actualSha256": actual
                        })),
                    );
                }
                None if strict => {
                    item.checksum_verified = Some(false);
                    item.status = "checksumMissing".to_string();
                    item.diagnostics.push(
                        AnalysisDiagnostic::fatal(
                            "SENTRUX-GRAMMAR-CHECKSUM-MISSING",
                            format!(
                                "Grammar checksum is missing for required language {language}."
                            ),
                        )
                        .language(language)
                        .path(manifest_path.display().to_string())
                        .expected_grammar(
                            sentrux_core::analysis::plugin::PluginManifest::grammar_filename(),
                        )
                        .plugin_root(Some(root))
                        .details(serde_json::json!({
                            "actualSha256": actual
                        })),
                    );
                }
                None => item.checksum_verified = None,
            }
        }
        Err(error) => {
            item.status = "checksumFailed".to_string();
            item.diagnostics.push(
                AnalysisDiagnostic::fatal(
                    "SENTRUX-GRAMMAR-CHECKSUM-FAILED",
                    format!("Failed to compute grammar checksum for {language}: {error}"),
                )
                .language(language)
                .path(grammar_path.display().to_string())
                .plugin_root(Some(root)),
            );
            item.finalize_status();
            return item;
        }
    }

    let symbol_name = manifest
        .grammar
        .symbol_name
        .as_deref()
        .unwrap_or(&manifest.plugin.name);
    match sentrux_core::analysis::plugin::load_grammar_dynamic(&grammar_path, symbol_name) {
        Ok(grammar) => {
            #[allow(deprecated)]
            let abi = grammar.version();
            if abi < manifest.grammar.abi_version as usize {
                item.status = "abiInvalid".to_string();
                item.diagnostics.push(
                    AnalysisDiagnostic::fatal(
                        "SENTRUX-GRAMMAR-ABI-INCOMPATIBLE",
                        format!(
                            "Grammar ABI version {abi} is lower than required {} for language {language}.",
                            manifest.grammar.abi_version
                        ),
                    )
                    .language(language)
                    .path(grammar_path.display().to_string())
                    .plugin_root(Some(root)),
                );
            }
            if let Err(error) = tree_sitter::Query::new(&grammar, &query_src) {
                item.status = "queryInvalid".to_string();
                item.diagnostics.push(
                    AnalysisDiagnostic::fatal(
                        "SENTRUX-QUERY-INVALID",
                        format!("Tree-sitter query failed to compile for language {language}: {error:?}"),
                    )
                    .language(language)
                    .path(query_path.display().to_string())
                    .plugin_root(Some(root)),
                );
            }
        }
        Err(error) => {
            item.status = "loadFailed".to_string();
            item.diagnostics.push(
                AnalysisDiagnostic::fatal(
                    "SENTRUX-GRAMMAR-LOAD-FAILED",
                    format!("Grammar failed to load for language {language}: {error}"),
                )
                .language(language)
                .path(grammar_path.display().to_string())
                .plugin_root(Some(root)),
            );
        }
    }

    item.finalize_status();
    item
}

fn compute_structural_coverage(
    snapshot: &core::snapshot::Snapshot,
    required_languages: &[String],
) -> StructuralCoverage {
    let files = core::snapshot::flatten_files_ref(&snapshot.root);
    let mut code_files = 0usize;
    let mut parsed_files = 0usize;
    let mut function_count = 0usize;
    let mut class_count = 0usize;
    let mut unparsed_code_files = Vec::new();

    for file in &files {
        if file.lang.is_empty() || file.lang == "unknown" {
            continue;
        }
        code_files += 1;
        if let Some(sa) = &file.sa {
            parsed_files += 1;
            function_count += sa.functions.as_ref().map_or(0, Vec::len);
            class_count += sa.cls.as_ref().map_or(0, Vec::len);
        } else {
            unparsed_code_files.push(serde_json::json!({
                "path": file.path,
                "language": file.lang,
                "lines": file.lines,
                "reason": "structural parser did not produce an AST for this file"
            }));
        }
    }

    let mut required_details = Vec::new();
    let mut fatal_diagnostics = Vec::new();
    for language in required_languages {
        let mut files_for_language = 0usize;
        let mut parsed_for_language = 0usize;
        let mut unparsed_for_language = Vec::new();
        for file in &files {
            if &file.lang != language {
                continue;
            }
            files_for_language += 1;
            if file.sa.is_some() {
                parsed_for_language += 1;
            } else {
                unparsed_for_language.push(file.path.clone());
            }
        }
        let complete = unparsed_for_language.is_empty();
        if !complete {
            fatal_diagnostics.push(
                AnalysisDiagnostic::fatal(
                    "SENTRUX-STRUCTURAL-COVERAGE-INCOMPLETE",
                    format!(
                        "Required language {language} had {files_for_language} file(s), but {} did not parse structurally.",
                        unparsed_for_language.len()
                    ),
                )
                .language(language)
                .details(serde_json::json!({
                    "unparsedFiles": unparsed_for_language
                })),
            );
        }
        required_details.push(serde_json::json!({
            "language": language,
            "files": files_for_language,
            "parsedFiles": parsed_for_language,
            "unparsedFiles": files_for_language.saturating_sub(parsed_for_language),
            "complete": complete
        }));
    }

    StructuralCoverage {
        total_files: snapshot.total_files,
        code_files,
        parsed_files,
        unparsed_files: code_files.saturating_sub(parsed_files),
        parse_coverage: if code_files == 0 {
            1.0
        } else {
            ((parsed_files as f64 / code_files as f64) * 10_000.0).round() / 10_000.0
        },
        function_count,
        class_count,
        import_edges: snapshot.import_graph.len(),
        call_edges: snapshot.call_graph.len(),
        required_languages: required_details,
        unparsed_code_files,
        fatal_diagnostics,
    }
}

fn analysis_complete(context: &AnalysisContext, coverage: Option<&StructuralCoverage>) -> bool {
    context.complete_without_coverage()
        && coverage
            .map(|coverage| coverage.fatal_diagnostics.is_empty())
            .unwrap_or(true)
}

fn analysis_json(
    context: &AnalysisContext,
    coverage: Option<&StructuralCoverage>,
) -> serde_json::Value {
    analysis_json_with_extra(context, coverage, &[])
}

fn analysis_json_with_extra(
    context: &AnalysisContext,
    coverage: Option<&StructuralCoverage>,
    extra_fatal: &[AnalysisDiagnostic],
) -> serde_json::Value {
    let mut fatal_diagnostics = context.fatal_diagnostics.clone();
    if let Some(coverage) = coverage {
        fatal_diagnostics.extend(coverage.fatal_diagnostics.clone());
    }
    fatal_diagnostics.extend(extra_fatal.iter().cloned());
    serde_json::json!({
        "complete": fatal_diagnostics.is_empty(),
        "pluginRoot": context.plugin_root.as_ref().map(|path| path.display().to_string()),
        "pluginRootSource": &context.plugin_root_source,
        "requiredLanguages": &context.required_languages,
        "immutable": true,
        "mutationPolicy": {
            "downloadsAllowed": false,
            "pluginSyncAllowed": false,
            "proPluginLoadAllowed": false
        },
        "inventory": {
            "platformGrammar": sentrux_core::analysis::plugin::PluginManifest::grammar_filename(),
            "platformKey": sentrux_core::analysis::plugin::grammar_platform_key(),
            "languages": context.inventory.iter().map(PluginInventoryItem::to_json).collect::<Vec<_>>()
        },
        "structuralCoverage": coverage.map(StructuralCoverage::to_json),
        "fatalDiagnostics": fatal_diagnostics.iter().map(AnalysisDiagnostic::to_json).collect::<Vec<_>>(),
        "warnings": context.warnings.iter().map(AnalysisDiagnostic::to_json).collect::<Vec<_>>()
    })
}

fn print_analysis_failure_text(context: &AnalysisContext, coverage: Option<&StructuralCoverage>) {
    let payload = analysis_json(context, coverage);
    println!("Analysis incomplete:");
    if let Some(items) = payload["fatalDiagnostics"].as_array() {
        for item in items {
            println!(
                "  - {}: {}",
                item["code"].as_str().unwrap_or("SENTRUX-UNKNOWN"),
                item["message"].as_str().unwrap_or("analysis failed")
            );
            if let Some(path) = item["path"].as_str() {
                println!("    path: {path}");
            }
            if let Some(language) = item["language"].as_str() {
                println!("    language: {language}");
            }
        }
    }
}

fn fatal_payload_json(
    command: &str,
    path: &str,
    include_untracked: bool,
    context: &AnalysisContext,
    coverage: Option<&StructuralCoverage>,
    diagnostic: AnalysisDiagnostic,
) -> String {
    let analysis = analysis_json_with_extra(context, coverage, &[diagnostic.clone()]);
    let payload = serde_json::json!({
        "pass": false,
        "passed": false,
        "command": command,
        "path": path,
        "scan": {
            "include_untracked": include_untracked
        },
        "analysis": analysis,
        "fatalDiagnostics": [diagnostic.to_json()]
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
}

fn scan_error_diagnostic(error: &core::types::AppError) -> AnalysisDiagnostic {
    let message = error.to_string();
    if message.contains("SENTRUX-GIT-UNTRACKED-ENUM-FAILED") {
        AnalysisDiagnostic::fatal(
            "SENTRUX-GIT-UNTRACKED-ENUM-FAILED",
            "Failed to enumerate untracked Git files while --include-untracked was requested.",
        )
        .details(serde_json::json!({ "error": message }))
    } else {
        AnalysisDiagnostic::fatal("SENTRUX-SCAN-FAILED", message)
    }
}

// ---------------------------------------------------------------------------
// Check
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Analytics
// ---------------------------------------------------------------------------

fn analytics_opt_out_path() -> Option<std::path::PathBuf> {
    sentrux_core::analysis::plugin::plugins_dir()
        .map(|d| d.parent().unwrap().join("telemetry_opt_out"))
}

fn run_login() {
    println!();
    println!("  Sentrux Pro — purchase at https://sentrux.dev/pro");
    println!();
    println!("  After purchase, activate with:");
    println!("    sentrux pro activate <license-key>");
    println!();
    println!("  Or paste your license key file:");
    println!("    sentrux pro activate /path/to/license.key");
    println!();
    // Try to open the browser
    let _ = open_url("https://sentrux.dev/pro");
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}

fn run_pro(action: ProAction) {
    match action {
        ProAction::Activate { key } => pro_activate(&key),
        ProAction::Status => pro_status(),
        ProAction::Deactivate => pro_deactivate(),
        ProAction::Update => pro_update(),
    }
}

fn pro_activate(key_input: &str) {
    // key_input is either a JSON string or a path to a file
    let key_json = if key_input.starts_with('{') {
        key_input.to_string()
    } else if std::path::Path::new(key_input).exists() {
        match std::fs::read_to_string(key_input) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Failed to read key file: {}", e);
                return;
            }
        }
    } else {
        eprintln!("Invalid key: not a JSON string or file path");
        return;
    };

    // Validate the key
    match sentrux_core::license::validate_license(&key_json) {
        Some(license) => {
            // Save to disk
            let dir = match dirs::home_dir() {
                Some(h) => h.join(".sentrux"),
                None => {
                    eprintln!("Cannot find home directory");
                    return;
                }
            };
            let _ = std::fs::create_dir_all(&dir);
            let key_path = dir.join("license.key");
            match std::fs::write(&key_path, &key_json) {
                Ok(_) => {
                    println!("License activated!");
                    println!("  User:    {}", license.user);
                    println!("  Tier:    {}", license.tier);
                    println!("  Expires: {}", license.expires);
                    println!("  Saved:   {}", key_path.display());
                    println!();
                    println!("Restart sentrux to enable Pro features.");
                }
                Err(e) => eprintln!("Failed to save license: {}", e),
            }
        }
        None => {
            eprintln!("Invalid or expired license key.");
        }
    }
}

fn pro_status() {
    let tier = sentrux_core::license::current_tier();
    println!("Tier: {}", tier);

    // Try to read and show license details
    if let Some(home) = dirs::home_dir() {
        let key_path = home.join(".sentrux").join("license.key");
        if let Ok(content) = std::fs::read_to_string(&key_path) {
            if let Some(license) = sentrux_core::license::validate_license(&content) {
                println!("User:    {}", license.user);
                println!("Expires: {}", license.expires);
                println!("ID:      {}", license.id);
            } else {
                println!("License: invalid or expired");
            }
        } else {
            println!("License: not found");
        }

        let dylib_name = if cfg!(target_os = "macos") {
            "pro.dylib"
        } else if cfg!(target_os = "windows") {
            "pro.dll"
        } else {
            "pro.so"
        };
        let dylib_path = home.join(".sentrux").join("pro").join(dylib_name);
        if dylib_path.exists() {
            println!("Plugin:  {} (installed)", dylib_path.display());
        } else {
            println!("Plugin:  not installed");
        }
    }

    if let Some((name, version)) = sentrux_core::pro_registry::plugin_info() {
        println!("Loaded:  {} v{}", name, version);
    }

    if sentrux_core::pro_registry::is_loaded() {
        println!("Status:  Pro features active");
    } else {
        println!("Status:  Free");
    }
}

fn pro_deactivate() {
    if let Some(home) = dirs::home_dir() {
        let key_path = home.join(".sentrux").join("license.key");
        let pro_dir = home.join(".sentrux").join("pro");

        if key_path.exists() {
            let _ = std::fs::remove_file(&key_path);
            println!("License removed.");
        }
        if pro_dir.exists() {
            let _ = std::fs::remove_dir_all(&pro_dir);
            println!("Pro plugin removed.");
        }
        println!("Deactivated. Restart sentrux to return to free mode.");
    }
}

fn pro_update() {
    println!("Pro plugin update: not yet implemented.");
    println!("For now, download the latest pro.dylib from https://sentrux.dev/pro");
    println!("and place it in ~/.sentrux/pro/");
}

fn run_analytics(action: Option<AnalyticsAction>) {
    let path = analytics_opt_out_path();
    match action {
        None => {
            // No subcommand = show state (like `brew analytics`)
            let opted_out = path.as_ref().map_or(false, |p| p.exists());
            if opted_out {
                println!("Analytics are disabled.");
            } else {
                println!("Analytics are enabled.");
            }
        }
        Some(AnalyticsAction::On) => {
            if let Some(p) = &path {
                let _ = std::fs::remove_file(p);
            }
            println!("Analytics are enabled.");
        }
        Some(AnalyticsAction::Off) => {
            if let Some(p) = &path {
                let _ = std::fs::create_dir_all(p.parent().unwrap());
                let _ = std::fs::write(p, "1");
            }
            println!("Analytics are disabled.");
        }
    }
}

// ---------------------------------------------------------------------------
// Check
// ---------------------------------------------------------------------------

/// Run architectural rules check from CLI. Returns exit code.
fn run_check(
    path: &str,
    include_untracked: bool,
    json_output: bool,
    analysis_options: CliAnalysisOptions,
) -> i32 {
    apply_cli_plugin_root(&analysis_options);
    let context = prepare_analysis_context(&analysis_options);
    let root = Path::new(path);
    if !root.is_dir() {
        let diagnostic =
            AnalysisDiagnostic::fatal("SENTRUX-PATH-INVALID", format!("Not a directory: {path}"))
                .path(path);
        if json_output {
            println!(
                "{}",
                fatal_payload_json("check", path, include_untracked, &context, None, diagnostic)
            );
        } else {
            eprintln!("Error: not a directory: {path}");
        }
        return 1;
    }

    if !analysis_complete(&context, None) {
        if json_output {
            let payload = serde_json::json!({
                "pass": false,
                "command": "check",
                "path": path,
                "scan": {
                    "include_untracked": include_untracked
                },
                "analysis": analysis_json(&context, None)
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
            );
        } else {
            print_analysis_failure_text(&context, None);
        }
        return 1;
    }

    let config = match metrics::rules::RulesConfig::try_load(root) {
        Some(c) => c,
        None => {
            let diagnostic = AnalysisDiagnostic::fatal(
                "SENTRUX-RULES-MISSING",
                format!("No .sentrux/rules.toml found in {path}."),
            )
            .path(
                root.join(".sentrux")
                    .join("rules.toml")
                    .display()
                    .to_string(),
            );
            if json_output {
                println!(
                    "{}",
                    fatal_payload_json(
                        "check",
                        path,
                        include_untracked,
                        &context,
                        None,
                        diagnostic
                    )
                );
            } else {
                eprintln!("No .sentrux/rules.toml found in {path}");
                eprintln!("Create one to define architectural constraints.");
            }
            return 1;
        }
    };

    if include_untracked {
        eprintln!("Scanning {path} including untracked files...");
    } else {
        eprintln!("Scanning {path}...");
    }
    let result = match analysis::scanner::scan_directory_with_options(
        path,
        None,
        None,
        &cli_scan_limits(),
        None,
        analysis::scanner::ScanOptions { include_untracked },
    ) {
        Ok(r) => r,
        Err(e) => {
            let diagnostic = scan_error_diagnostic(&e);
            if json_output {
                println!(
                    "{}",
                    fatal_payload_json(
                        "check",
                        path,
                        include_untracked,
                        &context,
                        None,
                        diagnostic
                    )
                );
            } else {
                eprintln!("Scan failed: {e}");
            }
            return 1;
        }
    };

    let coverage =
        compute_structural_coverage(&result.snapshot, &analysis_options.require_languages);
    let health = metrics::compute_health(&result.snapshot);
    let arch_report = metrics::arch::compute_arch(&result.snapshot);
    let check = metrics::rules::check_rules(
        &config,
        &health,
        &arch_report,
        &result.snapshot.import_graph,
    );
    let analysis_is_complete = analysis_complete(&context, Some(&coverage));

    if json_output {
        print_check_json(
            &check,
            &health,
            &result.snapshot,
            analysis_json(&context, Some(&coverage)),
            check.passed && analysis_is_complete,
        )
    } else {
        let rules_exit =
            print_check_results(&check, &health, &result.snapshot, &context, &coverage);
        if !analysis_is_complete {
            println!();
            print_analysis_failure_text(&context, Some(&coverage));
            1
        } else {
            rules_exit
        }
    }
}

/// Print check results and return exit code (0 = pass, 1 = violations).
fn print_check_results(
    check: &metrics::rules::RuleCheckResult,
    health: &metrics::HealthReport,
    snapshot: &core::snapshot::Snapshot,
    context: &AnalysisContext,
    coverage: &StructuralCoverage,
) -> i32 {
    println!("sentrux check — {} rules checked\n", check.rules_checked);
    println!(
        "Quality: {}\n",
        (health.quality_signal * 10000.0).round() as u32
    );
    println!("Scan: include_untracked={}", snapshot.include_untracked);
    print_analysis_summary(context, coverage);
    print_csharp_reference_stats(snapshot);

    if check.violations.is_empty() {
        println!("\n✓ All rules pass");
        print_cycle_details(health);
        0
    } else {
        println!();
        for v in &check.violations {
            let icon = match v.severity {
                metrics::rules::Severity::Error => "✗",
                metrics::rules::Severity::Warning => "⚠",
            };
            println!("{icon} [{:?}] {}: {}", v.severity, v.rule, v.message);
            for f in &v.files {
                println!("    {f}");
            }
        }
        print_cycle_details(health);
        println!("\n✗ {} violation(s) found", check.violations.len());
        1
    }
}

fn print_csharp_reference_stats(snapshot: &core::snapshot::Snapshot) {
    let stats = &snapshot.csharp_reference_stats;
    println!(
        "C# references: candidates={} resolved={} unresolved={} ambiguous={} (diagnostic only)",
        stats.candidates,
        stats.resolved_references,
        stats.unresolved_references,
        stats.ambiguous_references,
    );
}

fn print_cycle_details(health: &metrics::HealthReport) {
    if health.circular_dep_details.is_empty() {
        return;
    }
    println!("\nCycle edge chains:");
    for (idx, cycle) in health.circular_dep_details.iter().enumerate() {
        println!("  Cycle #{} ({} files):", idx + 1, cycle.files.len());
        for edge in &cycle.edge_chain {
            println!(
                "    {} -> {}  [{}]",
                edge.from_file,
                edge.to_file,
                format_edge_sources(&edge.sources),
            );
        }
    }
}

fn format_edge_sources(sources: &[core::types::ImportEdgeSource]) -> String {
    let effective_sources = if sources.is_empty() {
        vec![core::types::ImportEdgeSource::default()]
    } else {
        sources.to_vec()
    };
    effective_sources
        .iter()
        .map(format_edge_source)
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_edge_source(source: &core::types::ImportEdgeSource) -> String {
    let mut parts = vec![source.kind.to_string()];
    if let Some(symbol) = &source.symbol {
        parts.push(format!("symbol={symbol}"));
    }
    if let Some(line) = source.line {
        let column = source.column.unwrap_or(1);
        parts.push(format!("at={line}:{column}"));
    }
    parts.join(" ")
}

fn print_check_json(
    check: &metrics::rules::RuleCheckResult,
    health: &metrics::HealthReport,
    snapshot: &core::snapshot::Snapshot,
    analysis: serde_json::Value,
    overall_pass: bool,
) -> i32 {
    println!(
        "{}",
        check_report_json_with_analysis(check, health, snapshot, analysis, overall_pass)
    );
    if overall_pass {
        0
    } else {
        1
    }
}

fn check_report_json_with_analysis(
    check: &metrics::rules::RuleCheckResult,
    health: &metrics::HealthReport,
    snapshot: &core::snapshot::Snapshot,
    analysis: serde_json::Value,
    overall_pass: bool,
) -> String {
    let mut payload: serde_json::Value =
        serde_json::from_str(&metrics::check_report_json(check, health, snapshot))
            .unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = payload.as_object_mut() {
        object.insert("pass".to_string(), serde_json::json!(overall_pass));
        object.insert("analysis".to_string(), analysis);
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
}

fn print_analysis_summary(context: &AnalysisContext, coverage: &StructuralCoverage) {
    println!(
        "Analysis: complete={} plugin_root={} required_languages=[{}] parsed={}/{} code_files",
        analysis_complete(context, Some(coverage)),
        context
            .plugin_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(none)".to_string()),
        context.required_languages.join(", "),
        coverage.parsed_files,
        coverage.code_files,
    );
}

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

/// Run structural regression gate from CLI. Returns exit code.
fn run_gate(
    path: &str,
    save_mode: bool,
    include_untracked: bool,
    json_output: bool,
    analysis_options: CliAnalysisOptions,
) -> i32 {
    apply_cli_plugin_root(&analysis_options);
    let context = prepare_analysis_context(&analysis_options);
    let root = Path::new(path);
    if !root.is_dir() {
        let diagnostic =
            AnalysisDiagnostic::fatal("SENTRUX-PATH-INVALID", format!("Not a directory: {path}"))
                .path(path);
        if json_output {
            println!(
                "{}",
                fatal_payload_json("gate", path, include_untracked, &context, None, diagnostic)
            );
        } else {
            eprintln!("Error: not a directory: {path}");
        }
        return 1;
    }

    if !analysis_complete(&context, None) {
        if json_output {
            println!(
                "{}",
                gate_analysis_failure_json(path, include_untracked, &context, None)
            );
        } else {
            print_analysis_failure_text(&context, None);
        }
        return 1;
    }

    let baseline_path = root.join(".sentrux").join("baseline.json");

    if include_untracked {
        eprintln!("Scanning {path} including untracked files...");
    } else {
        eprintln!("Scanning {path}...");
    }
    let result = match analysis::scanner::scan_directory_with_options(
        path,
        None,
        None,
        &cli_scan_limits(),
        None,
        analysis::scanner::ScanOptions { include_untracked },
    ) {
        Ok(r) => r,
        Err(e) => {
            let diagnostic = scan_error_diagnostic(&e);
            if json_output {
                println!(
                    "{}",
                    fatal_payload_json("gate", path, include_untracked, &context, None, diagnostic)
                );
            } else {
                eprintln!("Scan failed: {e}");
            }
            return 1;
        }
    };

    let coverage =
        compute_structural_coverage(&result.snapshot, &analysis_options.require_languages);
    let analysis_is_complete = analysis_complete(&context, Some(&coverage));
    if !analysis_is_complete {
        if json_output {
            println!(
                "{}",
                gate_analysis_failure_json(path, include_untracked, &context, Some(&coverage))
            );
        } else {
            print_analysis_failure_text(&context, Some(&coverage));
        }
        return 1;
    }

    let health = metrics::compute_health(&result.snapshot);
    let arch_report = metrics::arch::compute_arch(&result.snapshot);
    let analysis = analysis_json(&context, Some(&coverage));

    if save_mode {
        gate_save(
            &baseline_path,
            &health,
            &arch_report,
            include_untracked,
            json_output,
            analysis,
        )
    } else {
        gate_compare(
            &baseline_path,
            &health,
            &arch_report,
            include_untracked,
            json_output,
            analysis,
        )
    }
}

fn scan_gate_health(
    path: &str,
    include_untracked: bool,
) -> Result<(metrics::HealthReport, metrics::arch::ArchReport), String> {
    let result = analysis::scanner::scan_directory_with_options(
        path,
        None,
        None,
        &cli_scan_limits(),
        None,
        analysis::scanner::ScanOptions { include_untracked },
    )
    .map_err(|e| format!("Scan failed: {e}"))?;

    let health = metrics::compute_health(&result.snapshot);
    let arch_report = metrics::arch::compute_arch(&result.snapshot);
    Ok((health, arch_report))
}

fn gate_save(
    baseline_path: &std::path::Path,
    health: &metrics::HealthReport,
    _arch_report: &metrics::arch::ArchReport,
    include_untracked: bool,
    json_output: bool,
    analysis: serde_json::Value,
) -> i32 {
    if let Some(parent) = baseline_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create directory {}: {e}", parent.display());
            return 1;
        }
    }
    let baseline = metrics::arch::ArchBaseline::from_health(health);
    match baseline.save(baseline_path) {
        Ok(()) => {
            if json_output {
                let payload = serde_json::json!({
                    "saved": true,
                    "baselinePath": baseline_path.display().to_string(),
                    "scan": {
                        "include_untracked": include_untracked
                    },
                    "analysis": analysis,
                    "quality": (health.quality_signal * 10000.0).round() as u32,
                    "metrics": {
                        "coupling": {
                            "score": health.coupling_score,
                            "crossModuleEdges": health.cross_module_edges,
                            "problemEdges": health.coupling_edges.iter().map(metrics::coupling_edge_detail_json).collect::<Vec<_>>()
                        },
                        "cycles": {
                            "count": health.circular_dep_details.len(),
                            "cycles": health.circular_dep_details.iter().map(metrics::cycle_detail_json).collect::<Vec<_>>()
                        },
                        "godFiles": {
                            "count": health.god_file_details.len(),
                            "files": health.god_file_details.iter().map(metrics::god_file_detail_json).collect::<Vec<_>>()
                        },
                        "complexFunctions": {
                            "count": health.complex_functions.len(),
                            "functions": health.complex_functions.iter().map(metrics::func_metric_json).collect::<Vec<_>>()
                        }
                    }
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
                );
            } else {
                println!("Baseline saved to {}", baseline_path.display());
                println!(
                    "Quality: {}",
                    (health.quality_signal * 10000.0).round() as u32
                );
                println!("Scan: include_untracked={include_untracked}");
                println!("Analysis: complete=true");
                println!("\nRun `sentrux gate` after making changes to compare.");
            }
            0
        }
        Err(e) => {
            eprintln!("Failed to save baseline: {e}");
            1
        }
    }
}

fn gate_compare(
    baseline_path: &std::path::Path,
    health: &metrics::HealthReport,
    arch_report: &metrics::arch::ArchReport,
    include_untracked: bool,
    json_output: bool,
    analysis: serde_json::Value,
) -> i32 {
    let baseline = match metrics::arch::ArchBaseline::load(baseline_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "Failed to load baseline at {}: {e}",
                baseline_path.display()
            );
            eprintln!("Run `sentrux gate --save` first to create one.");
            return 1;
        }
    };

    let diff = baseline.diff(health);

    if json_output {
        println!(
            "{}",
            gate_report_json_with_analysis(&diff, include_untracked, analysis, !diff.degraded)
        );
        return if diff.degraded { 1 } else { 0 };
    }

    println!("sentrux gate — structural regression check\n");
    println!("Scan:         include_untracked={include_untracked}");
    println!(
        "Analysis:     complete={}",
        analysis["complete"].as_bool().unwrap_or(false)
    );
    println!(
        "Quality:      {} -> {}",
        (diff.signal_before * 10000.0).round() as u32,
        (diff.signal_after * 10000.0).round() as u32
    );
    println!(
        "Coupling:     {:.2} → {:.2}",
        diff.coupling_before, diff.coupling_after
    );
    println!(
        "Cycles:       {} → {}",
        diff.cycles_before, diff.cycles_after
    );
    println!(
        "God files:    {} → {}",
        diff.god_files_before, diff.god_files_after
    );

    if !arch_report.distance_metrics.is_empty() {
        println!(
            "\nDistance from Main Sequence: {:.2}",
            arch_report.avg_distance
        );
    }

    if diff.degraded {
        println!("\n✗ DEGRADED");
        for v in &diff.violations {
            println!("  ✗ {v}");
        }
        if diff.signal_after >= diff.signal_before && diff.has_hard_metric_regression() {
            println!("\nHard metric regression caused failure despite aggregate quality improving or staying flat.");
        }
        print_coupling_diff(&diff.coupling_diff);
        print_cycle_diff(&diff.cycle_diff);
        print_god_file_diff(&diff.god_file_diff);
        print_function_metric_diff("Complex functions:", &diff.complex_function_diff);
        1
    } else {
        println!("\n✓ No degradation detected");
        print_coupling_diff(&diff.coupling_diff);
        print_cycle_diff(&diff.cycle_diff);
        print_god_file_diff(&diff.god_file_diff);
        print_function_metric_diff("Complex functions:", &diff.complex_function_diff);
        0
    }
}

fn gate_analysis_failure_json(
    path: &str,
    include_untracked: bool,
    context: &AnalysisContext,
    coverage: Option<&StructuralCoverage>,
) -> String {
    let analysis = analysis_json(context, coverage);
    let payload = serde_json::json!({
        "passed": false,
        "command": "gate",
        "path": path,
        "scan": {
            "include_untracked": include_untracked
        },
        "analysis": analysis,
        "degradations": [{
            "metric": "analysis",
            "message": "Structural analysis was incomplete; gate failed closed.",
            "hardFailure": true
        }]
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
}

fn gate_report_json_with_scan(diff: &metrics::arch::ArchDiff, include_untracked: bool) -> String {
    let mut payload: serde_json::Value =
        serde_json::from_str(&metrics::arch::gate_report_json(diff))
            .unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "scan".to_string(),
            serde_json::json!({
                "include_untracked": include_untracked
            }),
        );
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
}

fn gate_report_json_with_analysis(
    diff: &metrics::arch::ArchDiff,
    include_untracked: bool,
    analysis: serde_json::Value,
    overall_pass: bool,
) -> String {
    let mut payload: serde_json::Value =
        serde_json::from_str(&metrics::arch::gate_report_json(diff))
            .unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = payload.as_object_mut() {
        object.insert("passed".to_string(), serde_json::json!(overall_pass));
        object.insert(
            "scan".to_string(),
            serde_json::json!({
                "include_untracked": include_untracked
            }),
        );
        object.insert("analysis".to_string(), analysis);
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
}

fn print_coupling_diff(diff: &metrics::arch::CouplingDiff) {
    if !diff.degraded && diff.added.is_empty() && diff.removed.is_empty() {
        return;
    }

    if !diff.baseline_details_available {
        println!("\nCoupling offender diff unavailable: baseline only contains aggregate counts.");
        println!(
            "Run `sentrux gate --save` with this Sentrux version to capture coupling edge details."
        );
        print_coupling_edge_section("Current coupling problem edges:", &diff.current);
        return;
    }

    print_coupling_edge_section("Added coupling problem edges:", &diff.added);
    print_coupling_edge_section("Removed coupling problem edges:", &diff.removed);
    if diff.degraded && diff.added.is_empty() {
        print_coupling_edge_section("Current coupling problem edges:", &diff.current);
    }
}

fn print_coupling_edge_section(title: &str, edges: &[metrics::CouplingEdgeDetail]) {
    if edges.is_empty() {
        return;
    }
    println!("\n{title}");
    for edge in edges {
        println!("  - {} -> {}", edge.from_file, edge.to_file);
        println!("    reason: {}", edge.reason);
        println!("    modules: {} -> {}", edge.from_module, edge.to_module);
        println!("    sources: {}", format_edge_sources(&edge.sources));
    }
}

fn print_cycle_diff(diff: &metrics::arch::CycleDiff) {
    if !diff.degraded && diff.added.is_empty() && diff.removed.is_empty() {
        return;
    }

    if !diff.baseline_details_available {
        println!("\nCycle offender diff unavailable: baseline only contains aggregate counts.");
        println!(
            "Run `sentrux gate --save` with this Sentrux version to capture cycle edge chains."
        );
        print_cycle_detail_section("Current cycles:", &diff.current);
        return;
    }

    print_cycle_detail_section("Added cycles:", &diff.added);
    print_cycle_detail_section("Removed cycles:", &diff.removed);
    if diff.degraded && diff.added.is_empty() {
        print_cycle_detail_section("Current cycles:", &diff.current);
    }
}

fn print_cycle_detail_section(title: &str, cycles: &[metrics::CycleDetail]) {
    if cycles.is_empty() {
        return;
    }
    println!("\n{title}");
    for (idx, cycle) in cycles.iter().enumerate() {
        println!("  Cycle #{} ({} files):", idx + 1, cycle.files.len());
        for edge in &cycle.edge_chain {
            println!(
                "    {} -> {}  [{}]",
                edge.from_file,
                edge.to_file,
                format_edge_sources(&edge.sources),
            );
        }
    }
}

fn print_god_file_diff(diff: &metrics::arch::GodFileDiff) {
    if diff.before_count == diff.after_count
        && diff.changed_rank_or_score.is_empty()
        && diff.added.is_empty()
        && diff.removed.is_empty()
    {
        return;
    }

    if !diff.baseline_details_available {
        println!("\nGod file offender diff unavailable: baseline only contains aggregate counts.");
        println!(
            "Run `sentrux gate --save` with this Sentrux version to capture offender details."
        );
        if !diff.current.is_empty() {
            print_god_file_section("Current god files:", &diff.current, true);
        }
        return;
    }

    print_god_file_section("Added god files:", &diff.added, true);
    print_god_file_section("Removed god files:", &diff.removed, false);
    print_god_file_section("Existing god files:", &diff.persisting, false);
    if !diff.changed_rank_or_score.is_empty() {
        println!("\nChanged rank or score god files:");
        for change in &diff.changed_rank_or_score {
            println!(
                "  - {} (rank {} -> {}, score {:.2} -> {:.2}, fan_out {} -> {})",
                change.path,
                change.rank_before,
                change.rank_after,
                change.score_before,
                change.score_after,
                change.fan_out_before,
                change.fan_out_after,
            );
        }
    }
}

fn print_function_metric_diff(title: &str, diff: &metrics::arch::FunctionMetricDiff) {
    if !diff.degraded && diff.added.is_empty() && diff.removed.is_empty() {
        return;
    }

    if !diff.baseline_details_available {
        println!("\n{title} offender diff unavailable: baseline only contains aggregate counts.");
        println!(
            "Run `sentrux gate --save` with this Sentrux version to capture function details."
        );
        print_function_metric_section("Current functions:", &diff.current);
        return;
    }

    print_function_metric_section("Added functions:", &diff.added);
    print_function_metric_section("Removed functions:", &diff.removed);
    if diff.degraded && diff.added.is_empty() {
        print_function_metric_section("Current functions:", &diff.current);
    }
}

fn print_function_metric_section(title: &str, functions: &[metrics::FuncMetric]) {
    if functions.is_empty() {
        return;
    }
    println!("\n{title}");
    for function in functions {
        println!(
            "  - {}:{} (value={})",
            function.file, function.func, function.value
        );
    }
}

fn print_god_file_section(title: &str, files: &[metrics::GodFileDetail], include_details: bool) {
    if files.is_empty() {
        return;
    }
    println!("\n{title}");
    for file in files {
        println!("  - {}", file.path);
        if include_details {
            println!("    reason: {}", file.reason);
            println!(
                "    score: {:.2}, threshold: {}, fan_out: {}",
                file.score, file.threshold, file.fan_out,
            );
            println!(
                "    imports: {}, call_edges: {}, loc: {}, fan_in: {}, max_complexity: {}",
                file.imports,
                file.call_edges,
                file.loc,
                file.fan_in,
                file.max_complexity
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

fn run_plugin(action: PluginAction) {
    match action {
        PluginAction::List => plugin_list(),
        PluginAction::Init { name } => plugin_init(&name),
        PluginAction::Validate { dir } => plugin_validate(&dir),
        PluginAction::Verify {
            json,
            plugin_root,
            require_languages,
        } => std::process::exit(plugin_verify(json, plugin_root, require_languages)),
        PluginAction::AddStandard => plugin_add_standard(),
        PluginAction::Add { name } => plugin_add(&name),
        PluginAction::Remove { name } => plugin_remove(&name),
    }
}

fn plugin_list() {
    let dir = sentrux_core::analysis::plugin::plugins_dir();
    println!(
        "Plugin directory: {}",
        dir.as_ref()
            .map_or("(none)".into(), |d| d.display().to_string())
    );
    let (loaded, errors) = sentrux_core::analysis::plugin::load_all_plugins();
    if loaded.is_empty() && errors.is_empty() {
        println!("No plugins installed.");
        println!("\nInstall a plugin by placing it in ~/.sentrux/plugins/<name>/");
    } else {
        for p in &loaded {
            println!(
                "  {} v{} [{}] — {}",
                p.name,
                p.version,
                p.extensions.join(", "),
                p.display_name
            );
        }
        for e in &errors {
            println!("  (error) {} — {}", e.plugin_dir.display(), e.error);
        }
    }
}

fn plugin_init(name: &str) {
    let dir = sentrux_core::analysis::plugin::plugins_dir().unwrap_or_else(|| {
        eprintln!("Cannot determine home directory");
        std::process::exit(1);
    });
    let plugin_dir = dir.join(name);
    if plugin_dir.exists() {
        eprintln!("Plugin directory already exists: {}", plugin_dir.display());
        std::process::exit(1);
    }
    std::fs::create_dir_all(plugin_dir.join("grammars")).unwrap();
    std::fs::create_dir_all(plugin_dir.join("queries")).unwrap();
    std::fs::create_dir_all(plugin_dir.join("tests")).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.toml"),
        format!(
            r#"[plugin]
name = "{name}"
display_name = "{name}"
version = "0.1.0"
extensions = ["TODO"]
min_sentrux_version = "0.1.3"

[plugin.metadata]
author = ""
description = ""

[grammar]
source = "https://github.com/TODO/tree-sitter-{name}"
ref = "main"
abi_version = 14

[queries]
capabilities = ["functions", "classes", "imports"]

[checksums]
"#
        ),
    )
    .unwrap();
    std::fs::write(plugin_dir.join("queries").join("tags.scm"),
        ";; TODO: Write tree-sitter queries for this language\n;;\n;; Required captures:\n;;   @func.def / @func.name — function definitions\n;;   @class.def / @class.name — class definitions\n;;   @import.path — import statements\n;;   @call.name — function calls (optional)\n"
    ).unwrap();
    println!("Created plugin template at {}", plugin_dir.display());
    println!("\nNext steps:");
    println!("  1. Edit plugin.toml — set extensions, grammar source");
    println!(
        "  2. Build the grammar: tree-sitter generate && cc -shared -o grammars/{} src/parser.c",
        sentrux_core::analysis::plugin::manifest::PluginManifest::grammar_filename()
    );
    println!("  3. Write queries/tags.scm");
    println!(
        "  4. Test: sentrux plugin validate {}",
        plugin_dir.display()
    );
}

fn plugin_validate(dir: &str) {
    let plugin_dir = std::path::Path::new(dir);
    print!("Validating {}... ", plugin_dir.display());
    match sentrux_core::analysis::plugin::manifest::PluginManifest::load(plugin_dir) {
        Ok(manifest) => {
            println!("plugin.toml OK");
            println!("  name: {}", manifest.plugin.name);
            println!("  version: {}", manifest.plugin.version);
            println!("  extensions: [{}]", manifest.plugin.extensions.join(", "));
            println!(
                "  capabilities: [{}]",
                manifest.queries.capabilities.join(", ")
            );
            let query_path = plugin_dir.join("queries").join("tags.scm");
            match std::fs::read_to_string(&query_path) {
                Ok(qs) => match manifest.validate_query_captures(&qs) {
                    Ok(()) => println!("  queries/tags.scm: OK (captures valid)"),
                    Err(e) => println!("  queries/tags.scm: FAIL — {}", e),
                },
                Err(e) => println!("  queries/tags.scm: MISSING — {}", e),
            }
            let gf = sentrux_core::analysis::plugin::manifest::PluginManifest::grammar_filename();
            let gp = plugin_dir.join("grammars").join(gf);
            if gp.exists() {
                println!("  grammars/{}: OK", gf);
            } else {
                println!("  grammars/{}: MISSING — build the grammar first", gf);
            }
        }
        Err(e) => {
            println!("FAIL — {}", e);
            std::process::exit(1);
        }
    }
}

fn plugin_verify(
    json_output: bool,
    plugin_root: Option<String>,
    require_languages: Vec<String>,
) -> i32 {
    std::env::set_var(sentrux_core::analysis::plugin::IMMUTABLE_ANALYSIS_ENV, "1");
    let options = CliAnalysisOptions::new(plugin_root, require_languages);
    apply_cli_plugin_root(&options);
    let context = prepare_analysis_context(&options);
    let passed = analysis_complete(&context, None);
    if json_output {
        let payload = serde_json::json!({
            "passed": passed,
            "command": "plugin verify",
            "analysis": analysis_json(&context, None)
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        );
    } else {
        println!(
            "Plugin root: {} ({})",
            context
                .plugin_root
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "(none)".to_string()),
            context.plugin_root_source
        );
        println!(
            "Required languages: [{}]",
            context.required_languages.join(", ")
        );
        for item in &context.inventory {
            println!("  {}: {}", item.language, item.status);
            for diagnostic in &item.diagnostics {
                println!("    {}: {}", diagnostic.code, diagnostic.message);
            }
        }
        if !passed {
            println!();
            print_analysis_failure_text(&context, None);
        }
    }
    if passed {
        0
    } else {
        1
    }
}

fn plugin_add_standard() {
    sentrux_core::analysis::plugin::sync_embedded_plugins();
    ensure_grammars_installed();
    println!("Done. All plugins synced from embedded data.");
}

fn plugin_add(name: &str) {
    let dir = sentrux_core::analysis::plugin::plugins_dir().unwrap_or_else(|| {
        eprintln!("Cannot determine home directory");
        std::process::exit(1);
    });
    let plugin_dir = dir.join(name);
    if plugin_dir.exists() {
        eprintln!(
            "Plugin '{}' already installed at {}",
            name,
            plugin_dir.display()
        );
        eprintln!("Remove it first: sentrux plugin remove {}", name);
        std::process::exit(1);
    }

    let platform = sentrux_core::analysis::plugin::manifest::PluginManifest::grammar_filename();
    let platform_key = platform.rsplit_once('.').map_or(platform, |(k, _)| k);

    let version = match sentrux_core::analysis::plugin::embedded::EMBEDDED_PLUGINS
        .iter()
        .find(|&&(n, _, _)| n == name)
        .and_then(|&(_, toml, _)| {
            toml.lines()
                .find(|l| l.starts_with("version"))
                .and_then(|l| l.split('"').nth(1))
        }) {
        Some(v) => v,
        None => {
            eprintln!(
                "Plugin '{}' not found in embedded data. Is it a valid plugin name?",
                name
            );
            std::process::exit(1);
        }
    };
    let url = format!(
        "https://github.com/sentrux/plugins/releases/download/{name}-v{version}/{name}-{platform_key}.tar.gz"
    );
    println!("Downloading {name} plugin for {platform_key}...");
    println!("  {url}");

    std::fs::create_dir_all(&dir).unwrap();
    let tarball = dir.join(format!("{name}.tar.gz"));
    download_and_extract_plugin(&dir, name, &tarball, &url, &plugin_dir);
}

fn download_and_extract_plugin(
    dir: &std::path::Path,
    name: &str,
    tarball: &std::path::Path,
    url: &str,
    plugin_dir: &std::path::Path,
) {
    let output = std::process::Command::new("curl")
        .args(["-fsSL", url, "-o"])
        .arg(tarball)
        .status();

    match output {
        Ok(s) if s.success() => {
            let extract = std::process::Command::new("tar")
                .args(["xzf", &format!("{}.tar.gz", name)])
                .current_dir(dir)
                .status();
            let _ = std::fs::remove_file(tarball);
            match extract {
                Ok(s) if s.success() => {
                    println!("Installed {} to {}", name, plugin_dir.display());
                }
                _ => {
                    eprintln!("Failed to extract plugin archive");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            let _ = std::fs::remove_file(tarball);
            eprintln!("Failed to download plugin '{}'.", name);
            eprintln!("Check available plugins: https://github.com/sentrux/plugins/releases");
            std::process::exit(1);
        }
    }
}

fn plugin_remove(name: &str) {
    let dir = sentrux_core::analysis::plugin::plugins_dir().unwrap_or_else(|| {
        eprintln!("Cannot determine home directory");
        std::process::exit(1);
    });
    let plugin_dir = dir.join(name);
    if !plugin_dir.exists() {
        eprintln!("Plugin '{}' not installed.", name);
        std::process::exit(1);
    }
    std::fs::remove_dir_all(&plugin_dir).unwrap();
    println!("Removed plugin '{}'", name);
}

// ---------------------------------------------------------------------------
// GUI
// ---------------------------------------------------------------------------

/// Probe which wgpu backends have usable GPU adapters on this system.
/// Returns only backends that actually have hardware support, avoiding
/// blind attempts that panic on unsupported drivers.
fn probe_available_backends() -> Vec<eframe::wgpu::Backends> {
    let candidates = [
        (
            "Primary+GL",
            eframe::wgpu::Backends::PRIMARY | eframe::wgpu::Backends::GL,
        ),
        ("GL-only", eframe::wgpu::Backends::GL),
        ("Primary", eframe::wgpu::Backends::PRIMARY),
    ];

    let mut available = Vec::new();
    for (label, backends) in &candidates {
        let instance = eframe::wgpu::Instance::new(&eframe::wgpu::InstanceDescriptor {
            backends: *backends,
            ..Default::default()
        });
        let adapters: Vec<_> = instance.enumerate_adapters(eframe::wgpu::Backends::all());
        if !adapters.is_empty() {
            sentrux_core::debug_log!("[gpu] probe {label}: {} adapter(s) found", adapters.len());
            available.push(*backends);
        } else {
            sentrux_core::debug_log!("[gpu] probe {label}: no adapters");
        }
    }
    available
}

fn run_gui(path: Option<String>) -> eframe::Result<()> {
    let initial_path = path
        .map(|p| {
            std::path::Path::new(&p)
                .canonicalize()
                .map(|c| c.to_string_lossy().to_string())
                .unwrap_or(p)
        })
        .filter(|p| std::path::Path::new(p).is_dir());

    // Determine backends: respect user override, otherwise probe hardware.
    let env_backends = eframe::wgpu::Backends::from_env();
    let backend_attempts: Vec<eframe::wgpu::Backends> = if let Some(user_choice) = env_backends {
        // User explicitly chose via WGPU_BACKEND — respect it, no fallback
        vec![user_choice]
    } else {
        let probed = probe_available_backends();
        if probed.is_empty() {
            // No hardware GPU — try software rendering via glow (OpenGL)
            return run_gui_glow(initial_path);
        }
        probed
    };

    let version = env!("CARGO_PKG_VERSION");
    let title = {
        let edition = edition_name();
        if edition.is_empty() {
            format!("Sentrux {FORK_STAMP} v{}", version)
        } else {
            format!("Sentrux {FORK_STAMP} {} v{}", edition, version)
        }
    };
    let title = title.as_str();

    for (i, backends) in backend_attempts.iter().enumerate() {
        sentrux_core::debug_log!(
            "[gpu] attempt {}/{}: backends {:?}",
            i + 1,
            backend_attempts.len(),
            backends
        );

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1600.0, 1000.0])
                .with_maximized(true)
                .with_title(title),
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                    eframe::egui_wgpu::WgpuSetupCreateNew {
                        instance_descriptor: eframe::wgpu::InstanceDescriptor {
                            backends: *backends,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
            ..Default::default()
        };

        let path_clone = initial_path.clone();
        // catch_unwind as safety net: wgpu can panic on surface creation
        // even when adapter enumeration succeeded (driver bugs, missing DRI3)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            eframe::run_native(
                "Sentrux",
                options,
                Box::new(move |cc| Ok(Box::new(app::SentruxApp::new(cc, path_clone)))),
            )
        }));

        match result {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(e)) => {
                sentrux_core::debug_log!("[gpu] backend {:?} failed: {e}", backends);
            }
            Err(_panic) => {
                sentrux_core::debug_log!("[gpu] backend {:?} panicked (driver issue)", backends);
            }
        }

        if i + 1 == backend_attempts.len() {
            // All wgpu backends failed — fall back to glow (software OpenGL)
            return run_gui_glow(initial_path);
        }
    }
    Ok(())
}

/// Fallback GUI using glow (OpenGL) renderer — works on systems without
/// hardware GPU (VMs, RDP, headless servers with software OpenGL).
fn run_gui_glow(initial_path: Option<String>) -> eframe::Result<()> {
    sentrux_core::debug_log!("[gpu] falling back to glow (software OpenGL)");
    let version = env!("CARGO_PKG_VERSION");
    let title = {
        let edition = edition_name();
        if edition.is_empty() {
            format!("Sentrux {FORK_STAMP} v{}", version)
        } else {
            format!("Sentrux {FORK_STAMP} {} v{}", edition, version)
        }
    };
    let title = title.as_str();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1600.0, 1000.0])
            .with_maximized(true)
            .with_title(title),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Sentrux",
        options,
        Box::new(move |cc| Ok(Box::new(app::SentruxApp::new(cc, initial_path)))),
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cli_scan_limits() -> analysis::scanner::common::ScanLimits {
    let s = core::settings::Settings::default();
    analysis::scanner::common::ScanLimits {
        max_file_size_kb: s.max_file_size_kb,
        max_parse_size_kb: s.max_parse_size_kb,
        max_call_targets: s.max_call_targets,
    }
}

/// Ensure grammar binaries are installed for all embedded plugins.
/// Downloads ONE tarball with ALL grammars — not 49 individual downloads.
///
/// Architecture:
///   Each binary release on GitHub includes asset:
///     grammars-darwin-arm64.tar.gz (all grammars in one archive)
///   This function downloads that ONE file and extracts all grammars at once.
///
/// Handles: first launch, upgrade, accidental deletion.
fn ensure_grammars_installed() {
    // CI sets this to prevent overwriting already-installed grammars
    // with a 404 from a version tag that doesn't have grammar assets yet
    if std::env::var("SENTRUX_SKIP_GRAMMAR_DOWNLOAD").is_ok() {
        return;
    }

    let dir = match sentrux_core::analysis::plugin::plugins_dir() {
        Some(d) => d,
        None => return,
    };

    let platform = sentrux_core::analysis::plugin::manifest::PluginManifest::grammar_filename();
    let platform_key = platform.rsplit_once('.').map_or(platform, |(k, _)| k);

    let _ = std::fs::create_dir_all(&dir);

    // Check if ANY grammar is missing
    let any_missing = sentrux_core::analysis::plugin::embedded::EMBEDDED_PLUGINS
        .iter()
        .any(|&(name, toml, _)| {
            toml.contains("[grammar]") && !dir.join(name).join("grammars").join(platform).exists()
        });

    if !any_missing {
        return;
    }

    let version = env!("CARGO_PKG_VERSION");
    let release_repo = option_env!("SENTRUX_GRAMMAR_RELEASE_REPO").unwrap_or("heurexai/sentrux");
    let url = format!(
        "https://github.com/{release_repo}/releases/download/v{version}/grammars-{platform_key}.tar.gz"
    );
    let tarball = dir.join("grammars.tar.gz");

    eprintln!();
    eprintln!("  Downloading language grammars for v{version}...");
    eprintln!("  (one-time download, ~30MB)");
    eprint!("  [░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░]   0%");
    let _ = std::io::Write::flush(&mut std::io::stderr());

    let ok = std::process::Command::new("curl")
        .args(["-fsSL", "--progress-bar", &url, "-o"])
        .arg(&tarball)
        .stderr(std::process::Stdio::inherit()) // Show curl progress
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());

    if ok {
        // Extract: tarball contains <lang>/grammars/<platform>.dylib for each language
        let extracted = std::process::Command::new("tar")
            .args(["xzf"])
            .arg(&tarball)
            .current_dir(&dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        let _ = std::fs::remove_file(&tarball);

        if extracted {
            // Count how many grammars we now have
            let count = sentrux_core::analysis::plugin::embedded::EMBEDDED_PLUGINS
                .iter()
                .filter(|&&(name, _, _)| dir.join(name).join("grammars").join(platform).exists())
                .count();
            eprintln!("  {count} language grammars ready.");
        } else {
            eprintln!("  Failed to extract grammars archive.");
        }
    } else {
        let _ = std::fs::remove_file(&tarball);
        eprintln!("  Download failed. Check your network and try again.");
        eprintln!("  URL: {url}");
    }
    eprintln!();
}

#[cfg(test)]
mod gate_cli_tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command as ProcessCommand;

    #[test]
    fn gate_accepts_include_untracked_flag() {
        let cli = Cli::try_parse_from(["sentrux", "gate", "--include-untracked", "."]).unwrap();

        match cli.command {
            Some(Command::Gate {
                include_untracked, ..
            }) => assert!(include_untracked),
            _ => panic!("expected gate command"),
        }
    }

    #[test]
    fn check_accepts_plugin_root_and_required_languages() {
        let cli = Cli::try_parse_from([
            "sentrux",
            "check",
            "--plugin-root",
            "C:\\plugins",
            "--require-language",
            "csharp",
            "--require-language",
            "rust",
            ".",
        ])
        .unwrap();

        match cli.command {
            Some(Command::Check {
                plugin_root,
                require_languages,
                ..
            }) => {
                assert_eq!(plugin_root.as_deref(), Some("C:\\plugins"));
                assert_eq!(require_languages, vec!["csharp", "rust"]);
            }
            _ => panic!("expected check command"),
        }
    }

    #[test]
    fn plugin_verify_accepts_json_plugin_root_and_required_language() {
        let cli = Cli::try_parse_from([
            "sentrux",
            "plugin",
            "verify",
            "--json",
            "--plugin-root",
            "C:\\plugins",
            "--require-language",
            "csharp",
        ])
        .unwrap();

        match cli.command {
            Some(Command::Plugin {
                action:
                    PluginAction::Verify {
                        json,
                        plugin_root,
                        require_languages,
                    },
            }) => {
                assert!(json);
                assert_eq!(plugin_root.as_deref(), Some("C:\\plugins"));
                assert_eq!(require_languages, vec!["csharp"]);
            }
            _ => panic!("expected plugin verify command"),
        }
    }

    #[test]
    fn required_language_missing_plugin_reports_stable_json_diagnostic() {
        let root = unique_root("missing-plugin");
        let result = (|| {
            let context = prepare_analysis_context(&CliAnalysisOptions::new(
                Some(root.to_string_lossy().to_string()),
                vec!["csharp".to_string()],
            ));
            let payload = analysis_json(&context, None);
            assert_eq!(payload["complete"], false);
            let codes = diagnostic_codes(&payload);
            assert!(
                codes.contains(&"SENTRUX-LANGUAGE-PLUGIN-MISSING".to_string()),
                "expected missing plugin diagnostic, got {payload}"
            );
            assert_eq!(payload["inventory"]["languages"][0]["language"], "csharp");
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    #[test]
    fn required_language_missing_checksum_reports_stable_json_diagnostic() {
        let root = unique_root("missing-checksum");
        let result = (|| {
            let plugin = root.join("csharp");
            fs::create_dir_all(plugin.join("queries")).unwrap();
            fs::create_dir_all(plugin.join("grammars")).unwrap();
            fs::write(
                plugin.join("plugin.toml"),
                r#"[plugin]
name = "csharp"
display_name = "C#"
version = "0.0.0"
extensions = ["cs"]

[grammar]
source = "https://example.invalid/tree-sitter-c-sharp"
ref = "main"
abi_version = 14

[queries]
capabilities = []

[checksums]
"#,
            )
            .unwrap();
            fs::write(plugin.join("queries").join("tags.scm"), "").unwrap();
            fs::write(
                plugin
                    .join("grammars")
                    .join(sentrux_core::analysis::plugin::PluginManifest::grammar_filename()),
                b"not a dynamic grammar",
            )
            .unwrap();

            let context = prepare_analysis_context(&CliAnalysisOptions::new(
                Some(root.to_string_lossy().to_string()),
                vec!["csharp".to_string()],
            ));
            let payload = analysis_json(&context, None);
            let codes = diagnostic_codes(&payload);
            assert!(
                codes.contains(&"SENTRUX-GRAMMAR-CHECKSUM-MISSING".to_string()),
                "expected missing checksum diagnostic, got {payload}"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    #[test]
    fn coverage_failure_reports_exact_unparsed_required_file() {
        let file = core::types::FileNode {
            path: "src/Program.cs".to_string(),
            name: "Program.cs".to_string(),
            is_dir: false,
            lines: 12,
            logic: 10,
            comments: 0,
            blanks: 2,
            funcs: 0,
            mtime: 0.0,
            gs: String::new(),
            lang: "csharp".to_string(),
            sa: None,
            children: None,
        };
        let root = core::types::FileNode {
            path: String::new(),
            name: "repo".to_string(),
            is_dir: true,
            lines: 0,
            logic: 0,
            comments: 0,
            blanks: 0,
            funcs: 0,
            mtime: 0.0,
            gs: String::new(),
            lang: String::new(),
            sa: None,
            children: Some(vec![file]),
        };
        let snapshot = core::snapshot::Snapshot {
            root: std::sync::Arc::new(root),
            total_files: 1,
            total_lines: 12,
            total_dirs: 1,
            include_untracked: true,
            csharp_reference_stats: Default::default(),
            call_graph: Vec::new(),
            import_graph: Vec::new(),
            inherit_graph: Vec::new(),
            entry_points: Vec::new(),
            exec_depth: std::collections::HashMap::new(),
        };

        let coverage = compute_structural_coverage(&snapshot, &["csharp".to_string()]);
        assert_eq!(coverage.fatal_diagnostics.len(), 1);
        assert_eq!(
            coverage.fatal_diagnostics[0].code,
            "SENTRUX-STRUCTURAL-COVERAGE-INCOMPLETE"
        );
        assert!(
            coverage
                .unparsed_code_files
                .iter()
                .any(|file| { file["path"] == "src/Program.cs" && file["language"] == "csharp" }),
            "expected src/Program.cs in unparsed files: {:?}",
            coverage.unparsed_code_files
        );
    }

    #[test]
    fn gate_include_untracked_catches_untracked_regression_with_offender_details() {
        let root = unique_root("gate-untracked");
        let result = (|| {
            init_repo(&root);
            write_source(&root, "src/Baseline.cs", baseline_csharp("Baseline"));
            run_git(&root, &["add", "src/Baseline.cs"]);

            let root_str = root.to_str().unwrap();
            assert_eq!(
                run_gate(
                    root_str,
                    true,
                    false,
                    true,
                    CliAnalysisOptions::new(None, Vec::new())
                ),
                0
            );

            write_source(&root, "src/NewComplex.cs", complex_csharp("NewComplex"));

            assert_eq!(
                run_gate(
                    root_str,
                    false,
                    false,
                    true,
                    CliAnalysisOptions::new(None, Vec::new())
                ),
                0,
                "default gate must preserve tracked-only behavior"
            );
            assert_eq!(
                run_gate(
                    root_str,
                    false,
                    true,
                    true,
                    CliAnalysisOptions::new(None, Vec::new())
                ),
                1,
                "gate --include-untracked must fail on the new complex function"
            );

            let payload = gate_payload(&root, true);
            assert_eq!(payload["passed"], false);
            assert_eq!(payload["scan"]["include_untracked"], true);
            assert!(has_degradation(&payload, "complexFunctions"));
            assert!(
                added_complex_function_files(&payload)
                    .iter()
                    .any(|file| file == "src/NewComplex.cs"),
                "expected src/NewComplex.cs in added complex-function offenders: {payload}"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    #[test]
    fn gate_flags_tracked_modified_regression_with_and_without_include_untracked() {
        let root = unique_root("gate-tracked-modified");
        let result = (|| {
            init_repo(&root);
            write_source(&root, "src/Tracked.cs", baseline_csharp("Tracked"));
            run_git(&root, &["add", "src/Tracked.cs"]);

            let root_str = root.to_str().unwrap();
            assert_eq!(
                run_gate(
                    root_str,
                    true,
                    false,
                    true,
                    CliAnalysisOptions::new(None, Vec::new())
                ),
                0
            );

            write_source(&root, "src/Tracked.cs", complex_csharp("Tracked"));

            assert_eq!(
                run_gate(
                    root_str,
                    false,
                    false,
                    true,
                    CliAnalysisOptions::new(None, Vec::new())
                ),
                1,
                "tracked working-tree edits must still be scanned without the flag"
            );
            assert_eq!(
                run_gate(
                    root_str,
                    false,
                    true,
                    true,
                    CliAnalysisOptions::new(None, Vec::new())
                ),
                1,
                "tracked working-tree edits must also be scanned with the flag"
            );

            let payload_without_flag = gate_payload(&root, false);
            assert_eq!(payload_without_flag["scan"]["include_untracked"], false);
            assert!(
                added_complex_function_files(&payload_without_flag)
                    .iter()
                    .any(|file| file == "src/Tracked.cs"),
                "expected src/Tracked.cs in added complex-function offenders: {payload_without_flag}"
            );

            let payload_with_flag = gate_payload(&root, true);
            assert_eq!(payload_with_flag["scan"]["include_untracked"], true);
            assert!(
                added_complex_function_files(&payload_with_flag)
                    .iter()
                    .any(|file| file == "src/Tracked.cs"),
                "expected src/Tracked.cs in added complex-function offenders: {payload_with_flag}"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    fn gate_payload(root: &Path, include_untracked: bool) -> serde_json::Value {
        let root_str = root.to_str().unwrap();
        let (health, _) = scan_gate_health(root_str, include_untracked).unwrap();
        let baseline =
            metrics::arch::ArchBaseline::load(&root.join(".sentrux").join("baseline.json"))
                .unwrap();
        let diff = baseline.diff(&health);
        serde_json::from_str(&gate_report_json_with_scan(&diff, include_untracked)).unwrap()
    }

    fn has_degradation(payload: &serde_json::Value, metric: &str) -> bool {
        payload["degradations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["metric"] == metric)
    }

    fn added_complex_function_files(payload: &serde_json::Value) -> Vec<String> {
        payload["metrics"]["complexFunctions"]["functions"]["addedFunctions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["file"].as_str())
            .map(normalize_path)
            .collect()
    }

    fn diagnostic_codes(payload: &serde_json::Value) -> Vec<String> {
        payload["fatalDiagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["code"].as_str())
            .map(str::to_string)
            .collect()
    }

    fn normalize_path(path: &str) -> String {
        path.replace('\\', "/")
    }

    fn init_repo(root: &Path) {
        fs::create_dir_all(root.join("src")).unwrap();
        run_git(root, &["init"]);
    }

    fn write_source(root: &Path, relative: &str, source: String) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, source).unwrap();
    }

    fn baseline_csharp(class_name: &str) -> String {
        format!("public class {class_name} {{ public int Ok(int x) {{ return x + 1; }} }}\n")
    }

    fn complex_csharp(class_name: &str) -> String {
        let mut source = format!(
            "public class {class_name} {{\n    public int TooComplex(int x) {{\n        var total = 0;\n"
        );
        for value in 0..30 {
            source.push_str(&format!(
                "        if (x == {value}) {{ total += {value}; }}\n"
            ));
        }
        source.push_str("        return total;\n    }\n}\n");
        source
    }

    fn unique_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sentrux-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = ProcessCommand::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
