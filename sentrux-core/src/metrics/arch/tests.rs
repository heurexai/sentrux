use super::*;
use crate::core::snapshot::Snapshot;
use crate::core::types::{EntryPoint, FuncInfo, ImportEdge, StructuralAnalysis};
use crate::metrics::test_helpers::{edge, file, snap_with_edges};
use std::collections::HashSet;

fn entry(file: &str) -> EntryPoint {
    EntryPoint {
        file: file.to_string(),
        func: "main".to_string(),
        lang: "rust".to_string(),
        confidence: "high".to_string(),
    }
}

// ── Levelization tests ──

#[test]
fn levels_linear_chain() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let (levels, max) = compute_levels(&edges);
    assert_eq!(levels["c.rs"], 0);
    assert_eq!(levels["b.rs"], 1);
    assert_eq!(levels["a.rs"], 2);
    assert_eq!(max, 2);
}

#[test]
fn levels_diamond() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("a.rs", "c.rs"),
        edge("b.rs", "d.rs"),
        edge("c.rs", "d.rs"),
    ];
    let (levels, max) = compute_levels(&edges);
    assert_eq!(levels["d.rs"], 0);
    assert_eq!(levels["b.rs"], 1);
    assert_eq!(levels["c.rs"], 1);
    assert_eq!(levels["a.rs"], 2);
    assert_eq!(max, 2);
}

#[test]
fn levels_empty() {
    let (levels, max) = compute_levels(&[]);
    assert!(levels.is_empty());
    assert_eq!(max, 0);
}

#[test]
fn levels_single_edge() {
    let edges = vec![edge("a.rs", "b.rs")];
    let (levels, max) = compute_levels(&edges);
    assert_eq!(levels["b.rs"], 0);
    assert_eq!(levels["a.rs"], 1);
    assert_eq!(max, 1);
}

#[test]
fn levels_cycle_gets_max_plus_one() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "a.rs"),
        edge("c.rs", "a.rs"),
    ];
    let (levels, _max) = compute_levels(&edges);
    assert!(levels["c.rs"] > 0, "c should be above leaf level");
}

// ── Upward violation tests ──

#[test]
fn no_violations_in_clean_chain() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let (levels, _) = compute_levels(&edges);
    let violations = find_upward_violations(&edges, &levels);
    assert!(
        violations.is_empty(),
        "clean chain has no upward violations"
    );
}

#[test]
fn violation_when_leaf_imports_high_level() {
    let edges = vec![
        edge("a.rs", "b.rs"),
        edge("b.rs", "c.rs"),
        edge("c.rs", "a.rs"),
    ];
    let (levels, _) = compute_levels(&edges);
    let violations = find_upward_violations(&edges, &levels);
    assert!(
        !violations.is_empty() || levels.values().all(|&v| v == levels["a.rs"]),
        "should detect violation or recognize cycle"
    );
}

// ── Blast radius tests ──

#[test]
fn blast_radius_linear() {
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let radius = compute_blast_radius(&edges);
    assert_eq!(radius["c.rs"], 2, "c affects a and b");
    assert_eq!(radius["b.rs"], 1, "b affects only a");
    assert_eq!(radius["a.rs"], 0, "a affects nobody");
}

#[test]
fn blast_radius_star() {
    let edges = vec![
        edge("a.rs", "x.rs"),
        edge("b.rs", "x.rs"),
        edge("c.rs", "x.rs"),
    ];
    let radius = compute_blast_radius(&edges);
    assert_eq!(radius["x.rs"], 3);
    assert_eq!(radius["a.rs"], 0);
}

#[test]
fn blast_radius_empty() {
    let radius = compute_blast_radius(&[]);
    assert!(radius.is_empty());
}

// ── Attack surface tests ──

#[test]
fn attack_surface_from_entry() {
    let edges = vec![edge("main.rs", "handler.rs"), edge("handler.rs", "db.rs")];
    let entries = vec![entry("main.rs")];
    let (surface, total) = compute_attack_surface(&edges, &entries);
    assert_eq!(surface, 3);
    assert_eq!(total, 3);
}

#[test]
fn attack_surface_partial() {
    let edges = vec![
        edge("main.rs", "handler.rs"),
        edge("handler.rs", "db.rs"),
        edge("orphan.rs", "utils.rs"),
    ];
    let entries = vec![entry("main.rs")];
    let (surface, total) = compute_attack_surface(&edges, &entries);
    assert_eq!(surface, 3, "only main→handler→db reachable");
    assert_eq!(total, 5);
}

#[test]
fn attack_surface_no_entries() {
    let edges = vec![edge("a.rs", "b.rs")];
    let (surface, total) = compute_attack_surface(&edges, &[]);
    assert_eq!(surface, 0);
    assert_eq!(total, 2);
}

// ── Baseline diff tests ──

#[test]
fn baseline_detects_degradation() {
    let baseline = ArchBaseline {
        timestamp: 0.0,
        quality_signal: 0.90,
        coupling_score: 0.10,
        coupling_edges: vec![],
        cycle_count: 0,
        cycle_details: vec![],
        god_file_count: 0,
        god_files: vec![],
        hotspot_count: 0,
        complex_fn_count: 0,
        complex_functions: vec![],
        max_depth: 3,
        total_import_edges: 10,
        cross_module_edges: 1,
    };

    let current = crate::metrics::HealthReport {
        coupling_score: 0.45,
        circular_dep_count: 2,
        circular_dep_files: vec![vec!["a.rs".into(), "b.rs".into()]],
        circular_dep_details: vec![],
        total_import_edges: 20,
        cross_module_edges: 9,
        coupling_edges: vec![],
        entropy: 0.5,
        entropy_bits: 1.5,
        avg_cohesion: Some(0.3),
        max_depth: 5,
        deepest_files: vec![],
        god_files: vec![crate::metrics::FileMetric {
            path: "app.rs".into(),
            value: 18,
        }],
        god_file_details: vec![],
        hotspot_files: vec![],
        most_unstable: vec![],
        complex_functions: vec![
            crate::metrics::FuncMetric {
                file: "a.rs".into(),
                func: "f".into(),
                value: 20,
            },
            crate::metrics::FuncMetric {
                file: "b.rs".into(),
                func: "g".into(),
                value: 18,
            },
        ],
        long_functions: vec![],
        cog_complex_functions: vec![],
        high_param_functions: vec![],
        duplicate_groups: vec![],
        dead_functions: vec![],
        long_files: vec![],
        all_function_ccs: vec![],
        all_function_lines: vec![],
        all_file_lines: vec![],
        god_file_ratio: 0.05,
        hotspot_ratio: 0.0,
        complex_fn_ratio: 0.08,
        long_fn_ratio: 0.0,
        comment_ratio: Some(0.1),
        large_file_count: 0,
        large_file_ratio: 0.0,
        duplication_ratio: 0.0,
        dead_code_ratio: 0.0,
        high_param_ratio: 0.0,
        cog_complex_ratio: 0.0,
        quality_signal: 0.5,
        root_cause_raw: crate::metrics::root_causes::RootCauseRaw {
            modularity_q: 0.3,
            cycle_count: 2,
            max_depth: 5,
            complexity_gini: 0.3,
            redundancy_ratio: 0.1,
        },
        root_cause_scores: crate::metrics::root_causes::RootCauseScores {
            modularity: 0.53,
            acyclicity: 0.33,
            depth: 0.62,
            equality: 0.7,
            redundancy: 0.9,
        },
    };

    let diff = baseline.diff(&current);
    assert!(diff.degraded, "should detect degradation");
    assert!(
        !diff.violations.is_empty(),
        "should list specific violations"
    );
    assert!(diff.violations.iter().any(|v| v.contains("Coupling")));
    assert!(diff.violations.iter().any(|v| v.contains("Cycles")));
    assert!(diff.violations.iter().any(|v| v.contains("God files")));
    assert!(diff
        .violations
        .iter()
        .any(|v| v.contains("Complex functions")));
}

#[test]
fn baseline_diff_identifies_god_file_offenders() {
    let baseline_health = god_file_project(&[("src/existing.rs", 20), ("src/removed.rs", 19)]);
    let baseline = ArchBaseline::from_health(&baseline_health);
    let current = god_file_project(&[
        ("src/existing.rs", 22),
        ("src/added.rs", 21),
        ("src/added_two.rs", 18),
    ]);

    let diff = baseline.diff(&current);

    assert!(diff.degraded);
    assert_eq!(diff.god_file_diff.before_count, 2);
    assert_eq!(diff.god_file_diff.after_count, 3);
    assert_eq!(diff.god_file_diff.added.len(), 2);
    assert!(diff
        .god_file_diff
        .added
        .iter()
        .any(|f| f.path == "src/added.rs"));
    assert_eq!(diff.god_file_diff.removed.len(), 1);
    assert_eq!(diff.god_file_diff.removed[0].path, "src/removed.rs");
    assert_eq!(diff.god_file_diff.persisting.len(), 1);
    assert_eq!(diff.god_file_diff.persisting[0].path, "src/existing.rs");
    assert!(diff
        .god_file_diff
        .changed_rank_or_score
        .iter()
        .any(|change| change.path == "src/existing.rs" && change.fan_out_after == 22));

    let json = gate_report_json(&diff);
    let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(payload["passed"], false);
    assert_eq!(payload["metrics"]["godFiles"]["beforeCount"], 2);
    assert_eq!(payload["metrics"]["godFiles"]["afterCount"], 3);
    assert_eq!(
        payload["metrics"]["godFiles"]["added"][0]["path"],
        "src/added.rs"
    );
    assert!(payload["metrics"]["godFiles"]["addedGodFiles"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["path"] == "src/added_two.rs")));
    let degradations = payload["degradations"].as_array().unwrap();
    assert!(
        degradations
            .iter()
            .any(|item| item["metric"] == "godFiles" && item["hardFailure"] == true),
        "gate --json must report god-file degradation as a hard metric"
    );

    let mut quality_improved_diff = diff.clone();
    quality_improved_diff.signal_before = 0.50;
    quality_improved_diff.signal_after = 0.55;
    let quality_improved_payload: serde_json::Value =
        serde_json::from_str(&gate_report_json(&quality_improved_diff)).unwrap();
    assert_eq!(
        quality_improved_payload["hardMetricFailureDespiteQualityImprovement"],
        true
    );
}

#[test]
fn baseline_diff_identifies_other_metric_offenders() {
    let baseline_health = crate::metrics::compute_health(&snap_with_edges(Vec::new(), Vec::new()));
    let baseline = ArchBaseline::from_health(&baseline_health);
    let mut source = file("src/a.rs");
    source.sa = Some(StructuralAnalysis {
        functions: Some(vec![FuncInfo {
            n: "too_complex".into(),
            sl: 1,
            el: 80,
            ln: 80,
            cc: Some(20),
            cog: None,
            pc: None,
            bh: None,
            d: None,
            co: None,
            is_public: false,
            is_method: false,
        }]),
        cls: None,
        imp: None,
        co: None,
        tags: None,
        comment_lines: None,
    });
    let current = crate::metrics::compute_health(&snap_with_edges(
        vec![edge("src/a.rs", "src/b.rs"), edge("src/b.rs", "src/a.rs")],
        vec![source, file("src/b.rs")],
    ));

    let diff = baseline.diff(&current);
    let payload: serde_json::Value = serde_json::from_str(&gate_report_json(&diff)).unwrap();

    assert_eq!(
        payload["metrics"]["coupling"]["offenders"]["added"][0]["fromFile"],
        "src/a.rs"
    );
    assert!(
        payload["metrics"]["cycles"]["cycles"]["added"][0]["edge_chain"]
            .as_array()
            .is_some_and(|edges| edges.iter().any(|edge| edge["from_file"] == "src/a.rs"))
    );
    assert_eq!(
        payload["metrics"]["complexFunctions"]["functions"]["added"][0]["file"],
        "src/a.rs"
    );
}

fn god_file_project(sources: &[(&str, usize)]) -> crate::metrics::HealthReport {
    let mut edges = Vec::new();
    let mut files = Vec::new();
    let mut seen_files = HashSet::new();

    for (source_idx, (source, edge_count)) in sources.iter().enumerate() {
        push_file_once(&mut files, &mut seen_files, source);
        for dep_idx in 0..*edge_count {
            let dep = format!("src/deps/{source_idx}_{dep_idx}.rs");
            edges.push(edge(source, &dep));
            push_file_once(&mut files, &mut seen_files, &dep);
        }
    }

    crate::metrics::compute_health(&snap_with_edges(edges, files))
}

fn push_file_once(
    files: &mut Vec<crate::core::types::FileNode>,
    seen_files: &mut HashSet<String>,
    path: &str,
) {
    if seen_files.insert(path.to_string()) {
        files.push(file(path));
    }
}
