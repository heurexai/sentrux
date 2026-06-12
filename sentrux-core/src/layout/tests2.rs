use super::*;
use crate::core::settings::Settings;
use crate::layout::test_helpers::{
    default_focus, empty_entry_points, no_hidden,
    make_file, make_dir, simple_snapshot,
};
use crate::core::types::{CallEdge, ImportEdge};
use crate::core::snapshot::Snapshot;
use std::collections::HashMap;
use std::sync::Arc;
use types::{LayoutMode, RectKind, ScaleMode, SizeMode};

/// Helper: call compute_layout_from_snapshot with common defaults, constructing LayoutConfig.
fn layout(
    snap: &Snapshot,
    size_mode: SizeMode,
    scale_mode: ScaleMode,
    layout_mode: LayoutMode,
    vw: f64,
    vh: f64,
) -> types::RenderData {
    let settings = Settings::default();
    let focus = default_focus();
    let entry = empty_entry_points();
    let hidden = no_hidden();
    let cfg = LayoutConfig {
        size_mode, scale_mode, layout_mode,
        heat_map: None, settings: &settings, focus_mode: &focus,
        entry_point_files: &entry, hidden_paths: &hidden, impact_files: None,
    };
    compute_layout_from_snapshot(snap, vw, vh, None, &cfg)
}

// ─── Monotonicity: more lines → larger rect (linear scale) ────

#[test]
fn test_monotonicity_lines_to_area() {
    let snap = Snapshot {
        root: Arc::new(make_dir(
            "root",
            "root",
            vec![
                // Use 100 vs 1000 (10:1 ratio) instead of 10 vs 1000 (100:1).
                // Extreme ratios cause the smaller file to get a rect below
                // squarify_min_rect and be dropped from the layout entirely.
                make_file("small.rs", "root/small.rs", 100),
                make_file("large.rs", "root/large.rs", 1000),
            ],
        )),
        total_files: 2,
        total_lines: 1100,
        total_dirs: 1,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        call_graph: vec![],
        import_graph: vec![],
        inherit_graph: vec![],
        entry_points: vec![],
        exec_depth: HashMap::new(),
    };

    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Treemap, 800.0, 600.0);

    let files: Vec<_> = rd.rects.iter().filter(|r| r.kind == RectKind::File).collect();
    // Both files must be present — if one is missing, squarify dropped it
    let all_paths: Vec<_> = rd.rects.iter().map(|r| format!("{:?} {}", r.kind, r.path)).collect();
    let small = files.iter().find(|r| r.path == "root/small.rs")
        .unwrap_or_else(|| panic!("root/small.rs not found in rects: {:?}", all_paths));
    let large = files.iter().find(|r| r.path == "root/large.rs")
        .unwrap_or_else(|| panic!("root/large.rs not found in rects: {:?}", all_paths));
    let small_area = small.w * small.h;
    let large_area = large.w * large.h;
    assert!(
        large_area > small_area,
        "large file area {} should > small file area {}",
        large_area,
        small_area,
    );
}

// ─── Injection: 10× weight file → largest rect ────────────────

#[test]
fn test_injection_10x_weight_is_largest() {
    let snap = Snapshot {
        root: Arc::new(make_dir(
            "root",
            "root",
            vec![
                make_file("a.rs", "root/a.rs", 100),
                make_file("b.rs", "root/b.rs", 100),
                make_file("giant.rs", "root/giant.rs", 1000),
            ],
        )),
        total_files: 3,
        total_lines: 1200,
        total_dirs: 1,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        call_graph: vec![],
        import_graph: vec![],
        inherit_graph: vec![],
        entry_points: vec![],
        exec_depth: HashMap::new(),
    };

    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Treemap, 600.0, 400.0);

    let files: Vec<_> = rd.rects.iter().filter(|r| r.kind == RectKind::File).collect();
    let giant = files.iter().find(|r| r.path == "root/giant.rs").unwrap();
    let giant_area = giant.w * giant.h;

    for f in &files {
        if f.path != "root/giant.rs" {
            let area = f.w * f.h;
            assert!(
                giant_area > area,
                "giant area {} should > {} area {}",
                giant_area,
                f.path,
                area,
            );
        }
    }
}

// ─── Blueprint layout produces rects ───────────────────────────

#[test]
fn test_blueprint_produces_rects() {
    let snap = simple_snapshot();
    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Smooth, LayoutMode::Blueprint, 0.0, 0.0);

    let files: Vec<_> = rd.rects.iter().filter(|r| r.kind == RectKind::File).collect();
    assert!(files.len() >= 4, "should layout at least 4 files, got {}", files.len());
    assert!(!rd.anchors.is_empty(), "should have anchors");
    assert!(!rd.edge_paths.is_empty(), "should have edge paths (snapshot has edges)");
}

// ─── Edge paths: import and call edges are routed ──────────────

#[test]
fn test_edge_paths_routed() {
    let snap = simple_snapshot();
    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Blueprint, 0.0, 0.0);

    let import_edges: Vec<_> = rd.edge_paths.iter().filter(|e| e.edge_type == "import").collect();
    let call_edges: Vec<_> = rd.edge_paths.iter().filter(|e| e.edge_type == "call").collect();

    assert!(!import_edges.is_empty(), "should have routed import edges");
    assert!(!call_edges.is_empty(), "should have routed call edges");

    // Each edge path should have at least 2 points
    for e in &rd.edge_paths {
        assert!(e.pts.len() >= 2, "edge path has {} pts < 2", e.pts.len());
    }
}

// ─── Routing: no backward segments into source rect ────────────

#[test]
fn test_routing_no_backward_segments() {
    use crate::layout::routing::compute_edge_path;
    use crate::layout::types::Anchor;

    let from = Anchor {
        file_path: "a.rs".into(),
        cx: 50.0, cy: 50.0,
        section_id: "s".into(),
        bx: 30.0, by: 10.0, bw: 40.0, bh: 80.0,
    };
    let to = Anchor {
        file_path: "b.rs".into(),
        cx: 200.0, cy: 120.0,
        section_id: "s".into(),
        bx: 180.0, by: 100.0, bw: 40.0, bh: 40.0,
    };

    for &lane in &[0.0, 4.0, -4.0, 8.0, -8.0, 12.0, -12.0] {
        if let Some((pts, _side)) = compute_edge_path(&from, &to, lane, &Settings::default()) {
            for (i, p) in pts.iter().enumerate().skip(1) {
                let inside_x = p.x > from.bx + 2.0 && p.x < from.bx + from.bw - 2.0;
                let inside_y = p.y > from.by + 2.0 && p.y < from.by + from.bh - 2.0;
                assert!(
                    !(inside_x && inside_y),
                    "lane={}: point {} ({:.1},{:.1}) is inside source rect ({:.0},{:.0},{:.0},{:.0})",
                    lane, i, p.x, p.y, from.bx, from.by, from.bw, from.bh,
                );
            }
        }
    }
}

// ─── Routing: cross-type edges between same files get separate lanes ──

#[test]
fn test_cross_type_lane_separation() {
    let snap = Snapshot {
        root: Arc::new(make_dir(
            "root", "root",
            vec![
                make_file("a.rs", "root/a.rs", 100),
                make_file("b.rs", "root/b.rs", 100),
            ],
        )),
        total_files: 2,
        total_lines: 200,
        total_dirs: 1,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        // Both an import AND a call between the same files
        import_graph: vec![ImportEdge::new("root/a.rs", "root/b.rs")],
        call_graph: vec![CallEdge {
            from_file: "root/a.rs".to_string(),
            from_func: "main".to_string(),
            to_file: "root/b.rs".to_string(),
            to_func: "run".to_string(),
        }],
        inherit_graph: vec![],
        entry_points: vec![],
        exec_depth: HashMap::new(),
    };

    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Blueprint, 0.0, 0.0);

    let ab_edges: Vec<_> = rd.edge_paths.iter()
        .filter(|e| e.from_file == "root/a.rs" && e.to_file == "root/b.rs")
        .collect();
    assert!(ab_edges.len() >= 2, "should have both import and call edge, got {}", ab_edges.len());

    if ab_edges.len() >= 2 {
        let pts0 = &ab_edges[0].pts;
        let pts1 = &ab_edges[1].pts;
        let identical = pts0.len() == pts1.len() && pts0.iter().zip(pts1.iter())
            .all(|(a, b)| (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01);
        assert!(
            !identical,
            "cross-type edges between same files must have different paths (lane separation)"
        );
    }
}

// ─── Deep nesting doesn't panic ────────────────────────────────

#[test]
fn test_boundary_deep_nesting() {
    let mut node = make_file("deep.rs", "d0/d1/d2/d3/d4/d5/d6/d7/d8/d9/deep.rs", 50);
    for i in (0..10).rev() {
        let path = (0..=i).map(|j| format!("d{}", j)).collect::<Vec<_>>().join("/");
        let name = format!("d{}", i);
        node = make_dir(&name, &path, vec![node]);
    }

    let snap = Snapshot {
        root: Arc::new(node),
        total_files: 1,
        total_lines: 50,
        total_dirs: 10,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        call_graph: vec![],
        import_graph: vec![],
        inherit_graph: vec![],
        entry_points: vec![],
        exec_depth: HashMap::new(),
    };

    // Should not panic
    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Treemap, 800.0, 600.0);
    assert!(!rd.rects.is_empty());
}
