//! Shared test helpers for metrics test modules.
//!
//! Eliminates duplication of `edge()`, `file()`, and `snap_with_edges()`
//! across mod_tests.rs, mod_tests2.rs, arch/tests.rs, arch/tests2.rs,
//! dsm/tests.rs, rules/tests.rs, and whatif/tests.rs.

use crate::core::types::ImportEdge;
use crate::core::snapshot::Snapshot;
use crate::core::types::FileNode;
use std::collections::HashMap;
use std::sync::Arc;

/// Build a minimal `ImportEdge` from two path strings.
pub fn edge(from: &str, to: &str) -> ImportEdge {
    ImportEdge::new(from, to)
}

/// Build a minimal `FileNode` (non-dir, 100 lines, rust lang) from a path.
pub fn file(path: &str) -> FileNode {
    FileNode {
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        is_dir: false,
        lines: 100,
        logic: 80,
        comments: 10,
        blanks: 10,
        funcs: 5,
        mtime: 0.0,
        gs: String::new(),
        lang: "rust".to_string(),
        sa: None,
        children: None,
    }
}

/// Build a minimal snapshot with given import edges and files.
/// `total_files` is set from the actual file count for struct completeness.
pub fn snap_with_edges(edges: Vec<ImportEdge>, files: Vec<FileNode>) -> Snapshot {
    let file_count = files.iter().filter(|f| !f.is_dir).count() as u32;
    Snapshot {
        root: Arc::new(FileNode {
            path: ".".to_string(),
            name: ".".to_string(),
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
            children: Some(files),
        }),
        total_files: file_count,
        total_lines: 0,
        total_dirs: 0,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        call_graph: Vec::new(),
        import_graph: edges,
        inherit_graph: Vec::new(),
        entry_points: Vec::new(),
        exec_depth: HashMap::new(),
    }
}
