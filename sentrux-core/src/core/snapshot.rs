//! Immutable scan snapshot — the complete result of scanning a directory.
//!
//! A `Snapshot` captures the file tree, all three dependency graphs (import,
//! call, inherit), entry points, and execution depth. It is Arc-wrapped and
//! shared across threads (scanner, layout, renderer) without copying.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::core::types::{
    CSharpReferenceStats, CallEdge, EntryPoint, FileNode, ImportEdge, InheritEdge,
};

/// Scan progress report — shared between scanner and channels.
/// Lives in core (not scanner) to avoid channels importing scanner
/// just for this struct, which would create a deep transitive dependency chain.
pub struct ScanProgress {
    /// Human-readable description of current scan phase (e.g. "Parsing files")
    pub step: String,
    /// Progress percentage (0-100)
    pub pct: u8,
}

/// Complete scan result: file tree + dependency graphs + entry points.
/// Immutable after construction. Shared via `Arc<Snapshot>` across threads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Arc-wrapped file tree root to avoid deep-cloning on rescan.
    /// Rescan may produce a new Snapshot with updated graphs but same root.
    pub root: Arc<FileNode>,
    /// Total number of non-directory files in the scan
    pub total_files: u32,
    /// Total line count across all files
    pub total_lines: u32,
    /// Total number of directories
    pub total_dirs: u32,
    /// Whether this snapshot included untracked, non-ignored Git worktree files.
    #[serde(default)]
    pub include_untracked: bool,
    /// Diagnostics from the C# type-reference resolver.
    #[serde(default)]
    pub csharp_reference_stats: CSharpReferenceStats,
    /// Function-to-function call edges between files
    pub call_graph: Vec<CallEdge>,
    /// File-to-file import/require edges
    pub import_graph: Vec<ImportEdge>,
    /// Class inheritance/implementation edges
    pub inherit_graph: Vec<InheritEdge>,
    /// Detected application entry points
    pub entry_points: Vec<EntryPoint>,
    /// BFS distance from entry points per file
    pub exec_depth: HashMap<String, u32>,
}

/// A filesystem change event from the watcher.
/// Carries enough context for the scanner to decide whether to rescan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    /// Event timestamp as Unix epoch seconds
    pub ts: f64,
    /// Event kind: "create", "modify", or "remove"
    pub kind: String,
    /// Relative path from scan root
    pub path: String,
    /// Whether the path is a directory
    pub is_dir: bool,
    /// Git diff content for modified files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// Number of lines added in this change
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adds: Option<u32>,
    /// Number of lines deleted in this change
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dels: Option<u32>,
}

/// Structured analysis result for a file change (unused, reserved for future AI analysis).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    /// One-line summary of the change
    pub headline: String,
    /// Narrative explanation of what changed and why
    pub story: String,
    /// Risk assessment level (e.g. "low", "medium", "high")
    pub risk: String,
    /// Change category (e.g. "refactor", "feature", "bugfix")
    pub category: String,
}

/// Flatten a FileNode tree into a Vec of non-directory file references.
/// Pure utility — only depends on FileNode. Lives in core::snapshot (not graph.rs)
/// to avoid pulling the entire analysis::graph dependency chain into callers.
pub fn flatten_files_ref(node: &FileNode) -> Vec<&FileNode> {
    let mut result = Vec::new();
    flatten_files_ref_inner(node, &mut result);
    result
}

fn flatten_files_ref_inner<'a>(node: &'a FileNode, result: &mut Vec<&'a FileNode>) {
    if !node.is_dir {
        result.push(node);
    }
    if let Some(children) = &node.children {
        for child in children {
            flatten_files_ref_inner(child, result);
        }
    }
}

/// Flatten a FileNode tree into owned FileNodes by cloning each leaf.
/// Prefer `flatten_files_ref` when borrowing suffices to avoid allocations.
pub fn flatten_files(node: &FileNode) -> Vec<FileNode> {
    let mut result = Vec::new();
    flatten_files_inner(node, &mut result);
    result
}

fn flatten_files_inner(node: &FileNode, result: &mut Vec<FileNode>) {
    if !node.is_dir {
        result.push(node.clone());
    }
    if let Some(children) = &node.children {
        for child in children {
            flatten_files_inner(child, result);
        }
    }
}
