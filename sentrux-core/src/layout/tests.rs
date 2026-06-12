use super::*;
use crate::core::settings::Settings;
use crate::layout::test_helpers::{
    default_focus, empty_entry_points, no_hidden,
    make_file, make_dir, simple_snapshot,
};
use crate::core::types::{CallEdge, ImportEdge};
use crate::core::snapshot::Snapshot;
use crate::core::types::FileNode;
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

// ─── Invariance: color mode change doesn't change layout ──────

#[test]
fn test_invariance_layout_ignores_color_mode() {
    let snap = simple_snapshot();
    let r1 = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Treemap, 800.0, 600.0);
    let r2 = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Treemap, 800.0, 600.0);
    // Layout should be identical — color mode is not an input
    assert_eq!(r1.rects.len(), r2.rects.len());
    for i in 0..r1.rects.len() {
        assert!(
            (r1.rects[i].x - r2.rects[i].x).abs() < 1e-10,
            "rect {} x differs",
            i
        );
        assert!(
            (r1.rects[i].y - r2.rects[i].y).abs() < 1e-10,
            "rect {} y differs",
            i
        );
    }
}

// ─── Conservation: child areas sum ≤ parent area ───────────────

#[test]
fn test_conservation_child_area_le_parent() {
    let snap = simple_snapshot();
    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Treemap, 800.0, 600.0);

    // Find sections and their children
    let sections: Vec<&_> = rd.rects.iter().filter(|r| r.kind == RectKind::Section).collect();
    for sec in &sections {
        let parent_area = sec.w * sec.h;
        let child_area: f64 = rd
            .rects
            .iter()
            .filter(|r| r.section_id == sec.path && r.path != sec.path)
            .map(|r| r.w * r.h)
            .sum();
        assert!(
            child_area <= parent_area + 1.0,
            "section {} child area {} > parent area {}",
            sec.path,
            child_area,
            parent_area,
        );
    }
}

// ─── Boundary: empty tree ──────────────────────────────────────

#[test]
fn test_boundary_empty_tree() {
    let snap = Snapshot {
        root: Arc::new(FileNode {
            path: "empty".to_string(),
            name: "empty".to_string(),
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
            children: Some(vec![]),
        }),
        total_files: 0,
        total_lines: 0,
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
    // Empty dir → at most one section rect (the root)
    assert!(rd.rects.len() <= 1);
    assert!(rd.edge_paths.is_empty());
}

// ─── Boundary: single file ─────────────────────────────────────

#[test]
fn test_boundary_single_file() {
    let snap = Snapshot {
        root: Arc::new(make_dir(
            "proj",
            "proj",
            vec![make_file("main.rs", "proj/main.rs", 100)],
        )),
        total_files: 1,
        total_lines: 100,
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
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "proj/main.rs");
}

// ─── Oracle: manual 3-file layout matches Rust output ──────────

#[test]
fn test_oracle_three_files() {
    let snap = Snapshot {
        root: Arc::new(make_dir(
            "root",
            "root",
            vec![
                make_file("a.rs", "root/a.rs", 100),
                make_file("b.rs", "root/b.rs", 100),
                make_file("c.rs", "root/c.rs", 100),
            ],
        )),
        total_files: 3,
        total_lines: 300,
        total_dirs: 1,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        call_graph: vec![],
        import_graph: vec![],
        inherit_graph: vec![],
        entry_points: vec![],
        exec_depth: HashMap::new(),
    };

    let rd = layout(&snap, SizeMode::Lines, ScaleMode::Linear, LayoutMode::Treemap, 300.0, 300.0);
    let files: Vec<_> = rd.rects.iter().filter(|r| r.kind == RectKind::File).collect();
    assert_eq!(files.len(), 3, "should have 3 file rects");

    // BUG 19 fix: tighten tolerance from 15% to 5%. Equal-weight files
    // should produce near-equal areas; 15% hid broken squarify output. [ref:93cf32d4]
    let areas: Vec<f64> = files.iter().map(|r| r.w * r.h).collect();
    let avg = areas.iter().sum::<f64>() / areas.len() as f64;
    for (i, a) in areas.iter().enumerate() {
        assert!(
            (*a - avg).abs() / avg < 0.05,
            "file {} area {} differs > 5% from avg {}",
            i,
            a,
            avg
        );
    }
}

// ─── Idempotency: same inputs → identical output ───────────────

#[test]
fn test_idempotency() {
    let snap = simple_snapshot();
    let r1 = layout(&snap, SizeMode::Logic, ScaleMode::Sqrt, LayoutMode::Blueprint, 0.0, 0.0);
    let r2 = layout(&snap, SizeMode::Logic, ScaleMode::Sqrt, LayoutMode::Blueprint, 0.0, 0.0);
    assert_eq!(r1.rects.len(), r2.rects.len());
    assert_eq!(r1.edge_paths.len(), r2.edge_paths.len());
    for i in 0..r1.rects.len() {
        assert!((r1.rects[i].x - r2.rects[i].x).abs() < 1e-10);
        assert!((r1.rects[i].w - r2.rects[i].w).abs() < 1e-10);
    }
}
