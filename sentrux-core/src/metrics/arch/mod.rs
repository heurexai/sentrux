//! Architecture-level metrics — beyond structural hygiene.
//!
//! Based on:
//! - Lakos 1996: levelization, upward dependency violations
//! - Robert C. Martin 2003: distance from main sequence, dependency direction
//! - Baldwin & Clark 2000: Design Structure Matrix (data only, rendering elsewhere)
//!
//! All metrics operate on the existing import graph — no additional parsing.
//!
//! Graph algorithms (SCC, levelization, blast radius, attack surface) are in
//! `arch_graph`. Re-exported here for backward compatibility.

use self::distance::{self as distance_mod, ModuleDistance};
use crate::core::snapshot::Snapshot;
use crate::core::types::ImportEdge;
use crate::metrics::{CouplingEdgeDetail, CycleDetail, FuncMetric, GodFileDetail};
use std::collections::{HashMap, HashSet};

pub mod distance;
pub mod graph;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests2;

// Re-export graph algorithms for backward compatibility.
pub use self::graph::{
    compute_attack_surface, compute_blast_radius, compute_levels, find_upward_violations,
    UpwardViolation,
};
pub(crate) use self::graph::{
    compute_levels_with_sccs, compute_sccs, find_upward_violations_with_sccs,
};

// ── Trait: ArchAnalyzer ──

/// Interface for computing architecture-level metrics from a snapshot.
///
/// Abstracts the architecture analysis so that:
/// - Tests can inject synthetic snapshots and verify grading logic
/// - Alternative analysis strategies (e.g., package-level vs file-level) can be swapped
/// - Pre-computed reports can be cached and returned directly
pub trait ArchAnalyzer {
    /// Compute the full architecture report from a snapshot.
    fn analyze(&self, snapshot: &Snapshot) -> ArchReport;

    /// Compute file levels from import edges.
    fn levels(&self, edges: &[ImportEdge]) -> (HashMap<String, u32>, u32);

    /// Compute blast radius from import edges.
    fn blast_radius(&self, edges: &[ImportEdge]) -> HashMap<String, u32>;
}

/// Default implementation using Lakos levelization and Martin distance metrics.
pub struct DefaultArchAnalyzer;

impl ArchAnalyzer for DefaultArchAnalyzer {
    fn analyze(&self, snapshot: &Snapshot) -> ArchReport {
        compute_arch(snapshot)
    }

    fn levels(&self, edges: &[ImportEdge]) -> (HashMap<String, u32>, u32) {
        compute_levels(edges)
    }

    fn blast_radius(&self, edges: &[ImportEdge]) -> HashMap<String, u32> {
        compute_blast_radius(edges)
    }
}

// ── Named constants [ref:736ae249] ──

// Upward-dependency ratio thresholds removed — continuous [0,1] scores replace letter grades.

// ── Public types ──

/// Complete architecture report — aggregates all arch-level metrics.
/// Produced by `compute_arch()` from a Snapshot's import graph.
#[derive(Debug, Clone)]
pub struct ArchReport {
    // ── Lakos 1996 — Levelization ──
    /// Per-file level in the DAG (0 = leaf, higher = more dependencies below)
    pub levels: HashMap<String, u32>,
    /// Maximum level across all files
    pub max_level: u32,
    /// Edges that violate levelization (from lower level to higher level)
    pub upward_violations: Vec<UpwardViolation>,
    /// Ratio of upward violations to total edges
    pub upward_ratio: f64,
    // (levelization_score removed — proxy metric, captured by root cause acyclicity+depth)

    // ── Blast radius (transitive reach from each file) ──
    /// Per-file transitive dependent count
    pub blast_radius: HashMap<String, u32>,
    /// Highest blast radius in the codebase
    pub max_blast_radius: u32,
    /// File with the highest blast radius
    pub max_blast_file: String,

    // ── Attack surface (transitive reach from entry points) ──
    /// Number of files reachable from any entry point
    pub attack_surface_files: u32,
    /// Ratio of reachable files to total graph files
    pub attack_surface_ratio: f64,
    /// Total files in the dependency graph
    pub total_graph_files: u32,

    // ── Distance from Main Sequence (Martin 2003) ──
    /// Per-module distance metrics
    pub distance_metrics: Vec<ModuleDistance>,
    /// Average distance across all modules
    pub avg_distance: f64,
    // (distance_score, blast_score, surface_score, arch_score removed
    //  — proxy metrics, all captured by root cause modularity Q)
}

/// Baseline snapshot for session diff / structural regression gate.
/// Captured at session start; subsequent scans compare against this
/// to detect regressions (e.g., quality_signal drop, new cycles).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchBaseline {
    /// When the baseline was captured (Unix epoch seconds)
    pub timestamp: f64,
    /// Quality signal at baseline (geometric mean of 3 category scores)
    pub quality_signal: f64,
    /// Coupling score at baseline
    pub coupling_score: f64,
    /// Problematic cross-module coupling edges at baseline.
    #[serde(default)]
    pub coupling_edges: Vec<CouplingEdgeDetail>,
    /// Number of circular dependency cycles at baseline
    pub cycle_count: usize,
    /// Detailed cycle edge chains at baseline.
    #[serde(default)]
    pub cycle_details: Vec<CycleDetail>,
    /// Number of god files (fan-out > threshold) at baseline
    pub god_file_count: usize,
    /// Detailed god-file offender list captured at baseline.
    #[serde(default)]
    pub god_files: Vec<GodFileDetail>,
    /// Number of hotspot files (fan-in > threshold) at baseline
    pub hotspot_count: usize,
    /// Number of complex functions (CC > threshold) at baseline
    pub complex_fn_count: usize,
    /// Detailed complex-function offenders at baseline.
    #[serde(default)]
    pub complex_functions: Vec<FuncMetric>,
    /// Maximum dependency depth at baseline
    pub max_depth: u32,
    /// Total import edges at baseline
    pub total_import_edges: usize,
    /// Cross-module import edges at baseline
    pub cross_module_edges: usize,
}

/// Diff between two snapshots (baseline vs current).
#[derive(Debug, Clone)]
pub struct ArchDiff {
    /// Quality signal from the baseline
    pub signal_before: f64,
    /// Quality signal from the current snapshot
    pub signal_after: f64,
    /// Coupling score from the baseline
    pub coupling_before: f64,
    /// Coupling score from the current snapshot
    pub coupling_after: f64,
    /// Cycle count from the baseline
    pub cycles_before: usize,
    /// Cycle count from the current snapshot
    pub cycles_after: usize,
    /// God file count from the baseline
    pub god_files_before: usize,
    /// God file count from the current snapshot
    pub god_files_after: usize,
    /// Complex function count from the baseline
    pub complex_functions_before: usize,
    /// Complex function count from the current snapshot
    pub complex_functions_after: usize,
    /// Detailed before/after offender diff for god files.
    pub god_file_diff: GodFileDiff,
    /// Detailed before/after offender diff for coupling edges.
    pub coupling_diff: CouplingDiff,
    /// Detailed before/after offender diff for cycles.
    pub cycle_diff: CycleDiff,
    /// Detailed before/after offender diff for complex functions.
    pub complex_function_diff: FunctionMetricDiff,
    /// True if quality_signal dropped or any metric degraded
    pub degraded: bool,
    /// Structured degradation records.
    pub degradations: Vec<ArchDegradation>,
    /// Human-readable violation descriptions
    pub violations: Vec<String>,
}

/// Structured degradation record for machine-readable gate output.
#[derive(Debug, Clone)]
pub struct ArchDegradation {
    /// Stable metric identifier.
    pub metric: String,
    /// Human-readable message.
    pub message: String,
    /// True for fail-closed hard structural metrics.
    pub hard_failure: bool,
}

/// Before/after god-file offender comparison.
#[derive(Debug, Clone)]
pub struct GodFileDiff {
    pub before_count: usize,
    pub after_count: usize,
    pub degraded: bool,
    pub baseline_details_available: bool,
    pub baseline: Vec<GodFileDetail>,
    pub current: Vec<GodFileDetail>,
    pub added: Vec<GodFileDetail>,
    pub removed: Vec<GodFileDetail>,
    pub persisting: Vec<GodFileDetail>,
    pub changed_rank_or_score: Vec<GodFileChange>,
}

/// Rank or score change for a god file present in both baseline and current scan.
#[derive(Debug, Clone)]
pub struct GodFileChange {
    pub path: String,
    pub before: GodFileDetail,
    pub after: GodFileDetail,
    pub rank_before: usize,
    pub rank_after: usize,
    pub score_before: f64,
    pub score_after: f64,
    pub fan_out_before: usize,
    pub fan_out_after: usize,
}

/// Before/after coupling offender comparison.
#[derive(Debug, Clone)]
pub struct CouplingDiff {
    pub before_score: f64,
    pub after_score: f64,
    pub before_cross_module_edges: usize,
    pub after_cross_module_edges: usize,
    pub degraded: bool,
    pub baseline_details_available: bool,
    pub current: Vec<CouplingEdgeDetail>,
    pub added: Vec<CouplingEdgeDetail>,
    pub removed: Vec<CouplingEdgeDetail>,
    pub persisting: Vec<CouplingEdgeDetail>,
}

/// Before/after cycle offender comparison.
#[derive(Debug, Clone)]
pub struct CycleDiff {
    pub before_count: usize,
    pub after_count: usize,
    pub degraded: bool,
    pub baseline_details_available: bool,
    pub current: Vec<CycleDetail>,
    pub added: Vec<CycleDetail>,
    pub removed: Vec<CycleDetail>,
    pub persisting: Vec<CycleDetail>,
}

/// Before/after function metric offender comparison.
#[derive(Debug, Clone)]
pub struct FunctionMetricDiff {
    pub before_count: usize,
    pub after_count: usize,
    pub degraded: bool,
    pub baseline_details_available: bool,
    pub current: Vec<FuncMetric>,
    pub added: Vec<FuncMetric>,
    pub removed: Vec<FuncMetric>,
    pub persisting: Vec<FuncMetric>,
}

// ── Baseline Save/Load ──

impl ArchBaseline {
    /// Create baseline from current health report.
    pub fn from_health(report: &crate::metrics::HealthReport) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
            quality_signal: report.quality_signal,
            coupling_score: report.coupling_score,
            coupling_edges: report.coupling_edges.clone(),
            cycle_count: report.circular_dep_count,
            cycle_details: report.circular_dep_details.clone(),
            god_file_count: report.god_files.len(),
            god_files: report.god_file_details.clone(),
            hotspot_count: report.hotspot_files.len(),
            complex_fn_count: report.complex_functions.len(),
            complex_functions: report.complex_functions.clone(),
            max_depth: report.max_depth,
            total_import_edges: report.total_import_edges,
            cross_module_edges: report.cross_module_edges,
        }
    }

    /// Save baseline to a JSON file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize baseline: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Failed to write baseline to {}: {e}", path.display()))?;
        Ok(())
    }

    /// Load baseline from a JSON file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read baseline from {}: {e}", path.display()))?;
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse baseline: {e}"))
    }

    /// Compare current health report against this baseline.
    /// Degradation = quality_signal dropped OR any specific metric worsened.
    pub fn diff(&self, current: &crate::metrics::HealthReport) -> ArchDiff {
        let mut degradations = Vec::new();
        let god_file_diff = compare_god_files(self, current);
        let coupling_diff = compare_coupling_edges(self, current);
        let cycle_diff = compare_cycles(self, current);
        let complex_function_diff = compare_function_metrics(
            self.complex_fn_count,
            &self.complex_functions,
            &current.complex_functions,
        );

        // Quality signal is the primary indicator
        let signal_delta = current.quality_signal - self.quality_signal;
        if signal_delta < -0.02 {
            degradations.push(ArchDegradation {
                metric: "quality".to_string(),
                message: format!(
                    "Quality signal dropped: {:.2} → {:.2} ({:+.2})",
                    self.quality_signal, current.quality_signal, signal_delta
                ),
                hard_failure: false,
            });
        }

        if current.coupling_score > self.coupling_score + 0.05 {
            degradations.push(ArchDegradation {
                metric: "coupling".to_string(),
                message: format!(
                    "Coupling degraded: {:.2} → {:.2}",
                    self.coupling_score, current.coupling_score
                ),
                hard_failure: true,
            });
        }
        if current.circular_dep_count > self.cycle_count {
            degradations.push(ArchDegradation {
                metric: "cycles".to_string(),
                message: format!(
                    "Cycles increased: {} → {}",
                    self.cycle_count, current.circular_dep_count
                ),
                hard_failure: true,
            });
        }
        if current.god_files.len() > self.god_file_count {
            degradations.push(ArchDegradation {
                metric: "godFiles".to_string(),
                message: format!(
                    "God files increased: {} → {}",
                    self.god_file_count,
                    current.god_files.len()
                ),
                hard_failure: true,
            });
        }
        if current.complex_functions.len() > self.complex_fn_count {
            degradations.push(ArchDegradation {
                metric: "complexFunctions".to_string(),
                message: format!(
                    "Complex functions increased: {} → {}",
                    self.complex_fn_count,
                    current.complex_functions.len()
                ),
                hard_failure: true,
            });
        }

        let violations = degradations
            .iter()
            .map(|degradation| degradation.message.clone())
            .collect::<Vec<_>>();
        let degraded = !degradations.is_empty();

        ArchDiff {
            signal_before: self.quality_signal,
            signal_after: current.quality_signal,
            coupling_before: self.coupling_score,
            coupling_after: current.coupling_score,
            cycles_before: self.cycle_count,
            cycles_after: current.circular_dep_count,
            god_files_before: self.god_file_count,
            god_files_after: current.god_files.len(),
            complex_functions_before: self.complex_fn_count,
            complex_functions_after: current.complex_functions.len(),
            god_file_diff,
            coupling_diff,
            cycle_diff,
            complex_function_diff,
            degraded,
            degradations,
            violations,
        }
    }
}

impl ArchDiff {
    /// True when a fail-closed structural metric regressed.
    pub fn has_hard_metric_regression(&self) -> bool {
        self.degradations
            .iter()
            .any(|degradation| degradation.hard_failure)
    }
}

fn compare_god_files(
    baseline: &ArchBaseline,
    current: &crate::metrics::HealthReport,
) -> GodFileDiff {
    let baseline_details_available = baseline.god_file_count == 0 || !baseline.god_files.is_empty();
    let before = baseline.god_files.clone();
    let after = god_file_details_from_health(current);

    if !baseline_details_available {
        return GodFileDiff {
            before_count: baseline.god_file_count,
            after_count: current.god_files.len(),
            degraded: current.god_files.len() > baseline.god_file_count,
            baseline_details_available,
            baseline: before,
            current: after,
            added: Vec::new(),
            removed: Vec::new(),
            persisting: Vec::new(),
            changed_rank_or_score: Vec::new(),
        };
    }

    let before_by_path = before
        .iter()
        .map(|detail| (detail.path.as_str(), detail))
        .collect::<HashMap<_, _>>();
    let after_by_path = after
        .iter()
        .map(|detail| (detail.path.as_str(), detail))
        .collect::<HashMap<_, _>>();

    let added = after
        .iter()
        .filter(|detail| !before_by_path.contains_key(detail.path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let removed = before
        .iter()
        .filter(|detail| !after_by_path.contains_key(detail.path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let persisting = after
        .iter()
        .filter(|detail| before_by_path.contains_key(detail.path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let changed_rank_or_score = after
        .iter()
        .filter_map(|after_detail| {
            let before_detail = before_by_path.get(after_detail.path.as_str())?;
            let rank_changed = before_detail.rank != after_detail.rank;
            let score_changed = (before_detail.score - after_detail.score).abs() > f64::EPSILON;
            let fan_out_changed = before_detail.fan_out != after_detail.fan_out;
            if rank_changed || score_changed || fan_out_changed {
                Some(GodFileChange {
                    path: after_detail.path.clone(),
                    before: (*before_detail).clone(),
                    after: after_detail.clone(),
                    rank_before: before_detail.rank,
                    rank_after: after_detail.rank,
                    score_before: before_detail.score,
                    score_after: after_detail.score,
                    fan_out_before: before_detail.fan_out,
                    fan_out_after: after_detail.fan_out,
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    GodFileDiff {
        before_count: baseline.god_file_count,
        after_count: current.god_files.len(),
        degraded: current.god_files.len() > baseline.god_file_count,
        baseline_details_available,
        baseline: before,
        current: after,
        added,
        removed,
        persisting,
        changed_rank_or_score,
    }
}

fn compare_coupling_edges(
    baseline: &ArchBaseline,
    current: &crate::metrics::HealthReport,
) -> CouplingDiff {
    let baseline_details_available =
        baseline.coupling_score <= 0.0 || !baseline.coupling_edges.is_empty();
    let before = baseline.coupling_edges.clone();
    let after = current.coupling_edges.clone();

    if !baseline_details_available {
        return CouplingDiff {
            before_score: baseline.coupling_score,
            after_score: current.coupling_score,
            before_cross_module_edges: baseline.cross_module_edges,
            after_cross_module_edges: current.cross_module_edges,
            degraded: current.coupling_score > baseline.coupling_score + 0.05,
            baseline_details_available,
            current: after,
            added: Vec::new(),
            removed: Vec::new(),
            persisting: Vec::new(),
        };
    }

    let before_keys = before
        .iter()
        .map(coupling_edge_key)
        .collect::<HashSet<_>>();
    let after_keys = after.iter().map(coupling_edge_key).collect::<HashSet<_>>();

    CouplingDiff {
        before_score: baseline.coupling_score,
        after_score: current.coupling_score,
        before_cross_module_edges: baseline.cross_module_edges,
        after_cross_module_edges: current.cross_module_edges,
        degraded: current.coupling_score > baseline.coupling_score + 0.05,
        baseline_details_available,
        current: after.clone(),
        added: after
            .iter()
            .filter(|detail| !before_keys.contains(&coupling_edge_key(detail)))
            .cloned()
            .collect(),
        removed: before
            .iter()
            .filter(|detail| !after_keys.contains(&coupling_edge_key(detail)))
            .cloned()
            .collect(),
        persisting: after
            .iter()
            .filter(|detail| before_keys.contains(&coupling_edge_key(detail)))
            .cloned()
            .collect(),
    }
}

fn compare_cycles(baseline: &ArchBaseline, current: &crate::metrics::HealthReport) -> CycleDiff {
    let baseline_details_available = baseline.cycle_count == 0 || !baseline.cycle_details.is_empty();
    let before = baseline.cycle_details.clone();
    let after = current.circular_dep_details.clone();

    if !baseline_details_available {
        return CycleDiff {
            before_count: baseline.cycle_count,
            after_count: current.circular_dep_count,
            degraded: current.circular_dep_count > baseline.cycle_count,
            baseline_details_available,
            current: after,
            added: Vec::new(),
            removed: Vec::new(),
            persisting: Vec::new(),
        };
    }

    let before_keys = before.iter().map(cycle_key).collect::<HashSet<_>>();
    let after_keys = after.iter().map(cycle_key).collect::<HashSet<_>>();

    CycleDiff {
        before_count: baseline.cycle_count,
        after_count: current.circular_dep_count,
        degraded: current.circular_dep_count > baseline.cycle_count,
        baseline_details_available,
        current: after.clone(),
        added: after
            .iter()
            .filter(|detail| !before_keys.contains(&cycle_key(detail)))
            .cloned()
            .collect(),
        removed: before
            .iter()
            .filter(|detail| !after_keys.contains(&cycle_key(detail)))
            .cloned()
            .collect(),
        persisting: after
            .iter()
            .filter(|detail| before_keys.contains(&cycle_key(detail)))
            .cloned()
            .collect(),
    }
}

fn compare_function_metrics(
    baseline_count: usize,
    baseline_functions: &[FuncMetric],
    current_functions: &[FuncMetric],
) -> FunctionMetricDiff {
    let baseline_details_available = baseline_count == 0 || !baseline_functions.is_empty();
    let before = baseline_functions.to_vec();
    let after = current_functions.to_vec();

    if !baseline_details_available {
        return FunctionMetricDiff {
            before_count: baseline_count,
            after_count: current_functions.len(),
            degraded: current_functions.len() > baseline_count,
            baseline_details_available,
            current: after,
            added: Vec::new(),
            removed: Vec::new(),
            persisting: Vec::new(),
        };
    }

    let before_keys = before.iter().map(func_metric_key).collect::<HashSet<_>>();
    let after_keys = after.iter().map(func_metric_key).collect::<HashSet<_>>();

    FunctionMetricDiff {
        before_count: baseline_count,
        after_count: current_functions.len(),
        degraded: current_functions.len() > baseline_count,
        baseline_details_available,
        current: after.clone(),
        added: after
            .iter()
            .filter(|detail| !before_keys.contains(&func_metric_key(detail)))
            .cloned()
            .collect(),
        removed: before
            .iter()
            .filter(|detail| !after_keys.contains(&func_metric_key(detail)))
            .cloned()
            .collect(),
        persisting: after
            .iter()
            .filter(|detail| before_keys.contains(&func_metric_key(detail)))
            .cloned()
            .collect(),
    }
}

fn coupling_edge_key(detail: &CouplingEdgeDetail) -> String {
    format!("{}->{}", detail.from_file, detail.to_file)
}

fn cycle_key(detail: &CycleDetail) -> String {
    if !detail.edge_chain.is_empty() {
        return detail
            .edge_chain
            .iter()
            .map(|edge| format!("{}->{}", edge.from_file, edge.to_file))
            .collect::<Vec<_>>()
            .join("|");
    }

    let mut files = detail.files.clone();
    files.sort_unstable();
    files.join("|")
}

fn func_metric_key(detail: &FuncMetric) -> String {
    format!("{}::{}", detail.file, detail.func)
}

fn god_file_details_from_health(current: &crate::metrics::HealthReport) -> Vec<GodFileDetail> {
    if !current.god_file_details.is_empty() {
        return current.god_file_details.clone();
    }

    current
        .god_files
        .iter()
        .enumerate()
        .map(|(idx, metric)| GodFileDetail {
            rank: idx + 1,
            path: metric.path.clone(),
            language: String::new(),
            reason: "fan-out exceeds threshold".to_string(),
            classification: "fan_out_above_threshold".to_string(),
            score: metric.value as f64,
            threshold: 0,
            loc: 0,
            imports: 0,
            fan_in: 0,
            fan_out: metric.value,
            call_edges: 0,
            degree_centrality: 0.0,
            instability: 0.0,
            max_complexity: None,
            function_count: 0,
        })
        .collect()
}

/// Format a full structural gate diff as stable JSON for build gates.
pub fn gate_report_json(diff: &ArchDiff) -> String {
    let quality_before = (diff.signal_before * 10000.0).round() as i32;
    let quality_after = (diff.signal_after * 10000.0).round() as i32;
    let payload = serde_json::json!({
        "passed": !diff.degraded,
        "quality": {
            "before": quality_before,
            "after": quality_after,
            "delta": quality_after - quality_before
        },
        "metrics": {
            "coupling": {
                "before": diff.coupling_before,
                "after": diff.coupling_after,
                "delta": diff.coupling_after - diff.coupling_before,
                "degraded": diff.coupling_after > diff.coupling_before + 0.05,
                "offenders": coupling_diff_json(&diff.coupling_diff)
            },
            "cycles": {
                "beforeCount": diff.cycles_before,
                "afterCount": diff.cycles_after,
                "delta": diff.cycles_after as i64 - diff.cycles_before as i64,
                "degraded": diff.cycles_after > diff.cycles_before,
                "cycles": cycle_diff_json(&diff.cycle_diff)
            },
            "complexFunctions": {
                "beforeCount": diff.complex_functions_before,
                "afterCount": diff.complex_functions_after,
                "delta": diff.complex_functions_after as i64 - diff.complex_functions_before as i64,
                "degraded": diff.complex_functions_after > diff.complex_functions_before,
                "functions": function_metric_diff_json(&diff.complex_function_diff)
            },
            "godFiles": god_file_diff_json(&diff.god_file_diff)
        },
        "degradations": diff.degradations.iter().map(|degradation| serde_json::json!({
            "metric": &degradation.metric,
            "message": &degradation.message,
            "hardFailure": degradation.hard_failure
        })).collect::<Vec<_>>(),
        "summary": if diff.degraded { "DEGRADED" } else { "No degradation detected" },
        "hardMetricFailureDespiteQualityImprovement": diff.degraded
            && diff.signal_after >= diff.signal_before
            && diff.has_hard_metric_regression()
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
}

fn coupling_diff_json(diff: &CouplingDiff) -> serde_json::Value {
    serde_json::json!({
        "beforeScore": diff.before_score,
        "afterScore": diff.after_score,
        "beforeCrossModuleEdges": diff.before_cross_module_edges,
        "afterCrossModuleEdges": diff.after_cross_module_edges,
        "degraded": diff.degraded,
        "baselineDetailsAvailable": diff.baseline_details_available,
        "current": diff.current.iter().map(super::coupling_edge_detail_json).collect::<Vec<_>>(),
        "added": diff.added.iter().map(super::coupling_edge_detail_json).collect::<Vec<_>>(),
        "removed": diff.removed.iter().map(super::coupling_edge_detail_json).collect::<Vec<_>>(),
        "persisting": diff.persisting.iter().map(super::coupling_edge_detail_json).collect::<Vec<_>>(),
        "addedCouplingEdges": diff.added.iter().map(super::coupling_edge_detail_json).collect::<Vec<_>>(),
        "removedCouplingEdges": diff.removed.iter().map(super::coupling_edge_detail_json).collect::<Vec<_>>(),
        "persistingCouplingEdges": diff.persisting.iter().map(super::coupling_edge_detail_json).collect::<Vec<_>>()
    })
}

fn cycle_diff_json(diff: &CycleDiff) -> serde_json::Value {
    serde_json::json!({
        "beforeCount": diff.before_count,
        "afterCount": diff.after_count,
        "degraded": diff.degraded,
        "baselineDetailsAvailable": diff.baseline_details_available,
        "current": diff.current.iter().map(super::cycle_detail_json).collect::<Vec<_>>(),
        "added": diff.added.iter().map(super::cycle_detail_json).collect::<Vec<_>>(),
        "removed": diff.removed.iter().map(super::cycle_detail_json).collect::<Vec<_>>(),
        "persisting": diff.persisting.iter().map(super::cycle_detail_json).collect::<Vec<_>>(),
        "addedCycles": diff.added.iter().map(super::cycle_detail_json).collect::<Vec<_>>(),
        "removedCycles": diff.removed.iter().map(super::cycle_detail_json).collect::<Vec<_>>(),
        "persistingCycles": diff.persisting.iter().map(super::cycle_detail_json).collect::<Vec<_>>()
    })
}

fn function_metric_diff_json(diff: &FunctionMetricDiff) -> serde_json::Value {
    serde_json::json!({
        "beforeCount": diff.before_count,
        "afterCount": diff.after_count,
        "degraded": diff.degraded,
        "baselineDetailsAvailable": diff.baseline_details_available,
        "current": diff.current.iter().map(super::func_metric_json).collect::<Vec<_>>(),
        "added": diff.added.iter().map(super::func_metric_json).collect::<Vec<_>>(),
        "removed": diff.removed.iter().map(super::func_metric_json).collect::<Vec<_>>(),
        "persisting": diff.persisting.iter().map(super::func_metric_json).collect::<Vec<_>>(),
        "addedFunctions": diff.added.iter().map(super::func_metric_json).collect::<Vec<_>>(),
        "removedFunctions": diff.removed.iter().map(super::func_metric_json).collect::<Vec<_>>(),
        "persistingFunctions": diff.persisting.iter().map(super::func_metric_json).collect::<Vec<_>>()
    })
}

fn god_file_diff_json(diff: &GodFileDiff) -> serde_json::Value {
    let added = diff
        .added
        .iter()
        .map(super::god_file_detail_json)
        .collect::<Vec<_>>();
    let removed = diff
        .removed
        .iter()
        .map(super::god_file_detail_json)
        .collect::<Vec<_>>();
    let persisting = diff
        .persisting
        .iter()
        .map(super::god_file_detail_json)
        .collect::<Vec<_>>();
    let changed = diff
        .changed_rank_or_score
        .iter()
        .map(god_file_change_json)
        .collect::<Vec<_>>();
    serde_json::json!({
        "beforeCount": diff.before_count,
        "afterCount": diff.after_count,
        "degraded": diff.degraded,
        "baselineDetailsAvailable": diff.baseline_details_available,
        "current": diff.current.iter().map(super::god_file_detail_json).collect::<Vec<_>>(),
        "added": added,
        "removed": removed,
        "persisting": persisting,
        "changedRankOrScore": changed,
        "addedGodFiles": diff.added.iter().map(super::god_file_detail_json).collect::<Vec<_>>(),
        "removedGodFiles": diff.removed.iter().map(super::god_file_detail_json).collect::<Vec<_>>(),
        "persistingGodFiles": diff.persisting.iter().map(super::god_file_detail_json).collect::<Vec<_>>(),
        "changedRankOrScoreGodFiles": diff.changed_rank_or_score.iter().map(god_file_change_json).collect::<Vec<_>>()
    })
}

fn god_file_change_json(change: &GodFileChange) -> serde_json::Value {
    serde_json::json!({
        "path": &change.path,
        "rankBefore": change.rank_before,
        "rankAfter": change.rank_after,
        "scoreBefore": change.score_before,
        "scoreAfter": change.score_after,
        "fanOutBefore": change.fan_out_before,
        "fanOutAfter": change.fan_out_after,
        "before": super::god_file_detail_json(&change.before),
        "after": super::god_file_detail_json(&change.after)
    })
}

// ── Grading ──

/// Blast radius concentration score [0,1].
/// 1.0 = no concentrated blast, 0.0 = catastrophic concentration.
/// [ref:28b7bc6f]
pub fn score_blast_concentration(blast_radius: &HashMap<String, u32>, edges: &[ImportEdge]) -> f64 {
    if blast_radius.is_empty() || edges.is_empty() {
        return 1.0;
    }
    let total_files = blast_radius.len();
    if total_files == 0 {
        return 1.0;
    }

    let (mod_fan_out, mod_fan_in) = compute_blast_module_coupling(edges);
    let file_fan_in = compute_blast_file_fan_in(edges);

    let max_non_foundation =
        find_max_non_foundation_blast(blast_radius, &mod_fan_out, &mod_fan_in, &file_fan_in);

    let ratio = max_non_foundation as f64 / total_files as f64;
    (1.0 - ratio).clamp(0.0, 1.0)
}

/// Compute MODULE-level coupling, excluding mod-declaration edges.
/// Rust `pub mod foo;` creates structural containment edges that inflate
/// parent module fan-out without representing functional dependencies.
fn compute_blast_module_coupling(
    edges: &[ImportEdge],
) -> (
    HashMap<String, HashSet<String>>,
    HashMap<String, HashSet<String>>,
) {
    let mut mod_fan_out: HashMap<String, HashSet<String>> = HashMap::new();
    let mut mod_fan_in: HashMap<String, HashSet<String>> = HashMap::new();
    for edge in edges {
        if crate::metrics::types::is_mod_declaration_edge(edge) {
            continue;
        }
        let from_mod = crate::core::path_utils::module_of(&edge.from_file).to_string();
        let to_mod = crate::core::path_utils::module_of(&edge.to_file).to_string();
        if from_mod != to_mod {
            mod_fan_out
                .entry(from_mod.clone())
                .or_default()
                .insert(to_mod.clone());
            mod_fan_in.entry(to_mod).or_default().insert(from_mod);
        }
    }
    (mod_fan_out, mod_fan_in)
}

/// Compute file-level fan-in for foundation file detection.
fn compute_blast_file_fan_in(edges: &[ImportEdge]) -> HashMap<&str, usize> {
    let mut file_fan_in: HashMap<&str, usize> = HashMap::new();
    for edge in edges {
        *file_fan_in.entry(edge.to_file.as_str()).or_default() += 1;
    }
    file_fan_in
}

/// Find the maximum blast radius among non-foundation files.
/// A file is foundation if its MODULE is stable OR the FILE itself has high fan-in.
fn find_max_non_foundation_blast(
    blast_radius: &HashMap<String, u32>,
    mod_fan_out: &HashMap<String, HashSet<String>>,
    mod_fan_in: &HashMap<String, HashSet<String>>,
    file_fan_in: &HashMap<&str, usize>,
) -> u32 {
    const MOD_STABILITY_THRESHOLD: f64 = 0.25;
    const MIN_MOD_FAN_IN: usize = 2;
    /// File-level foundation: a file with enough direct dependents is "too
    /// important to change casually" regardless of its fan-out.
    const MIN_FILE_FAN_IN_FOUNDATION: usize = 5;

    let is_foundation_module = |module: &str| -> bool {
        let ce = mod_fan_out.get(module).map_or(0, |s| s.len());
        let ca = mod_fan_in.get(module).map_or(0, |s| s.len());
        let total = ca + ce;
        if total == 0 {
            return false;
        }
        let instability = ce as f64 / total as f64;
        instability <= MOD_STABILITY_THRESHOLD && ca >= MIN_MOD_FAN_IN
    };

    let mut max_non_foundation: u32 = 0;
    for (path, &blast) in blast_radius {
        let module = crate::core::path_utils::module_of(path).to_string();
        let ca = file_fan_in.get(path.as_str()).copied().unwrap_or(0);
        // Package-index files (__init__.py, index.js, mod.rs, etc.) are barrel
        // re-exporters — their high blast radius reflects re-exports, not genuine
        // change risk. Treat them as foundation regardless of instability.
        let is_barrel = super::is_package_index_for_path(path);
        let is_foundation =
            is_barrel || is_foundation_module(&module) || ca >= MIN_FILE_FAN_IN_FOUNDATION;
        if !is_foundation && blast > max_non_foundation {
            max_non_foundation = blast;
        }
    }

    // If ALL files are in foundation modules, blast radius is architecturally
    // expected (stable foundations naturally have high reach). Return 0 so that
    // the grade computes as 'A' — penalizing stable-only codebases is wrong.
    max_non_foundation
}

/// Attack surface score [0,1]: 1.0 = minimal exposure, 0.0 = everything reachable.
pub fn score_attack_surface(ratio: f64) -> f64 {
    (1.0 - ratio).clamp(0.0, 1.0)
}

/// Check if a project is an application (has main entry points) vs a library.
/// Applications naturally have ~100% reachable code — grading attack surface
/// penalizes correct architecture. Libraries benefit from encapsulation.
pub fn is_application(snapshot: &Snapshot) -> bool {
    snapshot.entry_points.iter().any(|ep| ep.func == "main")
}

/// Levelization score [0,1]: 1.0 = no upward violations.
pub(crate) fn score_levelization(upward_ratio: f64) -> f64 {
    (1.0 - upward_ratio * 10.0).clamp(0.0, 1.0)
}

// ── Main entry point ──

/// Compute architecture report from a snapshot.
pub fn compute_arch(snapshot: &Snapshot) -> ArchReport {
    let edges = &snapshot.import_graph;

    // Filter mod-declaration edges (Rust `pub mod foo;`) from levelization.
    // Mod declarations are structural containment — NOT functional dependencies.
    // Without this filter, parent→child + child→parent(facade) creates false cycles.
    // Health metrics already filter these for coupling/depth/cycles; arch must too.
    let dep_edges: Vec<ImportEdge> = edges
        .iter()
        .filter(|e| !crate::metrics::types::is_mod_declaration_edge(e))
        .cloned()
        .collect();

    // Compute SCCs once and share between levelization + violation detection.
    let sccs = compute_sccs(&dep_edges);
    let (levels, max_level) = compute_levels_with_sccs(&dep_edges, &sccs);
    let upward_violations = find_upward_violations_with_sccs(&dep_edges, &levels, &sccs);
    let upward_ratio = if dep_edges.is_empty() {
        0.0
    } else {
        upward_violations.len() as f64 / dep_edges.len() as f64
    };
    // Blast radius (already filters mod-declaration edges internally)
    let blast_radius = compute_blast_radius(edges);
    let (max_blast_file, max_blast_radius) = blast_radius
        .iter()
        .max_by_key(|(_, &v)| v)
        .map(|(k, &v)| (k.clone(), v))
        .unwrap_or_default();

    // Attack surface + distance — diagnostic data only, no scoring
    let (
        attack_surface_files,
        total_graph_files,
        attack_surface_ratio,
        distance_metrics,
        avg_distance,
    ) = compute_arch_diagnostics(snapshot, &dep_edges);

    ArchReport {
        levels,
        max_level,
        upward_violations,
        upward_ratio,
        blast_radius,
        max_blast_radius,
        max_blast_file,
        attack_surface_files,
        attack_surface_ratio,
        total_graph_files,
        distance_metrics,
        avg_distance,
    }
}

/// Compute attack surface and distance diagnostics for compute_arch.
/// No scoring — diagnostic data only. The one true score is quality_signal.
fn compute_arch_diagnostics(
    snapshot: &Snapshot,
    dep_edges: &[ImportEdge],
) -> (u32, u32, f64, Vec<ModuleDistance>, f64) {
    let (attack_surface_files, total_graph_files) =
        compute_attack_surface(dep_edges, &snapshot.entry_points);
    let attack_surface_ratio = if total_graph_files > 0 {
        attack_surface_files as f64 / total_graph_files as f64
    } else {
        0.0
    };

    // Distance from Main Sequence (Martin 2003) — diagnostic only
    let distance_metrics =
        distance_mod::compute_distance_from_main_seq(snapshot, &snapshot.import_graph);
    let avg_distance = {
        let non_foundation: Vec<&ModuleDistance> = distance_metrics
            .iter()
            .filter(|m| !m.is_foundation)
            .collect();
        if non_foundation.is_empty() {
            0.0
        } else {
            non_foundation.iter().map(|m| m.distance).sum::<f64>() / non_foundation.len() as f64
        }
    };

    (
        attack_surface_files,
        total_graph_files,
        attack_surface_ratio,
        distance_metrics,
        avg_distance,
    )
}
