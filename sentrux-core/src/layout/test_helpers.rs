//! Shared test helpers for layout test modules.
//!
//! Eliminates duplication of `make_file()`, `make_dir()`, `run_layout()`,
//! and `simple_snapshot()` between tests.rs and tests2.rs.

use crate::core::settings::Settings;
use crate::core::types::{CallEdge, FileNode, ImportEdge};
use crate::core::snapshot::Snapshot;
use super::types::{FocusMode, LayoutMode, ScaleMode, SizeMode};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub fn default_focus() -> FocusMode { FocusMode::All }
pub fn empty_entry_points() -> HashSet<String> { HashSet::new() }
pub fn no_hidden() -> HashSet<String> { HashSet::new() }

/// Build LayoutConfig and call compute_layout_from_snapshot with common defaults.
pub fn run_layout(
    snap: &Snapshot,
    size_mode: SizeMode,
    scale_mode: ScaleMode,
    layout_mode: LayoutMode,
    vw: f64,
    vh: f64,
) -> super::types::RenderData {
    let settings = Settings::default();
    let focus = default_focus();
    let entry = empty_entry_points();
    let hidden = no_hidden();
    let cfg = super::LayoutConfig {
        size_mode, scale_mode, layout_mode,
        heat_map: None, settings: &settings, focus_mode: &focus,
        entry_point_files: &entry, hidden_paths: &hidden, impact_files: None,
    };
    super::compute_layout_from_snapshot(
        snap, vw, vh, None, &cfg,
    )
}

pub fn make_file(name: &str, path: &str, lines: u32) -> FileNode {
    FileNode {
        path: path.to_string(),
        name: name.to_string(),
        is_dir: false,
        lines,
        logic: lines * 7 / 10,
        comments: lines / 10,
        blanks: lines * 2 / 10,
        funcs: (lines / 50).max(1),
        mtime: 1000000.0,
        gs: String::new(),
        lang: "rust".to_string(),
        sa: None,
        children: None,
    }
}

pub fn make_dir(name: &str, path: &str, children: Vec<FileNode>) -> FileNode {
    let total_lines: u32 = children.iter().map(|c| c.lines).sum();
    FileNode {
        path: path.to_string(),
        name: name.to_string(),
        is_dir: true,
        lines: total_lines,
        logic: 0,
        comments: 0,
        blanks: 0,
        funcs: 0,
        mtime: 0.0,
        gs: String::new(),
        lang: String::new(),
        sa: None,
        children: Some(children),
    }
}

pub fn simple_snapshot() -> Snapshot {
    let root = make_dir(
        "project",
        "project",
        vec![
            make_dir(
                "src",
                "project/src",
                vec![
                    make_file("main.rs", "project/src/main.rs", 100),
                    make_file("lib.rs", "project/src/lib.rs", 200),
                    make_file("util.rs", "project/src/util.rs", 50),
                ],
            ),
            make_dir(
                "tests",
                "project/tests",
                vec![
                    make_file("test_main.rs", "project/tests/test_main.rs", 80),
                ],
            ),
            make_file("Cargo.toml", "project/Cargo.toml", 20),
        ],
    );

    Snapshot {
        root: Arc::new(root),
        total_files: 5,
        total_lines: 450,
        total_dirs: 3,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        call_graph: vec![CallEdge {
            from_file: "project/src/main.rs".to_string(),
            from_func: "main".to_string(),
            to_file: "project/src/lib.rs".to_string(),
            to_func: "run".to_string(),
        }],
        import_graph: vec![ImportEdge::new("project/src/main.rs", "project/src/util.rs")],
        inherit_graph: vec![],
        entry_points: vec![],
        exec_depth: HashMap::new(),
    }
}
