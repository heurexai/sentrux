//! Source code analysis — scanning, parsing, and graph extraction.
//!
//! Walks the filesystem, counts lines (via tokei), parses structure with
//! tree-sitter, resolves imports to file paths, and builds the three
//! dependency graphs (import, call, inherit).

#[cfg(test)]
pub(crate) mod test_helpers;

pub mod entry_points;
pub mod git;
pub mod graph;
pub mod lang_registry;
pub mod parser;
pub mod plugin;
pub mod project_refs;
pub mod resolver;
pub mod scanner;

