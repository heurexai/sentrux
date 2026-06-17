//! `.sentruxignore` support — gitignore-style exclusions for Sentrux scans.
//!
//! Unlike `.gitignore`, a `.sentruxignore` file can exclude paths even when they
//! are git-TRACKED. This is the whole point: `git ls-files` returns tracked files
//! that `.gitignore` cannot remove, so Sentrux applies `.sentruxignore` as a second,
//! independent filter over the assembled candidate list (from both `git ls-files`
//! and the filesystem-walk fallback).
//!
//! Syntax is full gitignore semantics (directory prefixes, filename globs, nested
//! `**` globs, and `!` negation), provided for free by the `ignore` crate's
//! `GitignoreBuilder`. The file is read from the scan root; a missing file means
//! no exclusions (and is not an error).

use ignore::gitignore::Gitignore;
use std::path::Path;

/// A matcher built from `<root>/.sentruxignore`.
///
/// When the file is absent the matcher is empty and `is_ignored` always returns
/// `false`, so callers can apply it unconditionally with no behavior change.
pub(crate) struct SentruxIgnore {
    matcher: Gitignore,
    root: std::path::PathBuf,
}

impl SentruxIgnore {
    /// Build a matcher from `<root>/.sentruxignore`.
    ///
    /// Missing file -> empty matcher (no exclusions, no error). Parse errors are
    /// logged and the successfully-parsed globs are still used (matching the
    /// lenient behavior of the watcher's gitignore loader).
    pub(crate) fn load(root: &Path) -> Self {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
        // `add` returns Some(err) on a parse problem; a missing file is simply a
        // no-op that contributes zero globs, which is exactly what we want.
        if let Some(err) = builder.add(root.join(".sentruxignore")) {
            crate::debug_log!("[sentruxignore] parse warning: {}", err);
        }
        let matcher = builder.build().unwrap_or_else(|e| {
            crate::debug_log!("[sentruxignore] build error: {}, using empty matcher", e);
            ignore::gitignore::GitignoreBuilder::new(root)
                .build()
                .expect("empty gitignore matcher is always valid")
        });
        Self {
            matcher,
            root: root.to_path_buf(),
        }
    }

    /// True if this matcher contains no globs (i.e. there was no usable
    /// `.sentruxignore`). Lets callers skip work entirely in the common case.
    pub(crate) fn is_empty(&self) -> bool {
        self.matcher.num_ignores() == 0
    }

    /// Returns `true` if `path` is excluded by `.sentruxignore`.
    ///
    /// `path` may be absolute (it is made relative to the scan root) or already
    /// relative to the root. `is_dir` reflects whether the path is a directory —
    /// gitignore semantics treat directory matches specially. Negated patterns
    /// (`!keep.cs`) correctly re-include a path via `Match::is_ignore` being false.
    ///
    /// Uses `matched_path_or_any_parents` (not plain `matched`) so a directory
    /// entry like `generated/` correctly excludes nested files such as
    /// `generated/a.cs` — plain `matched` only tests the leaf path itself.
    pub(crate) fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        if self.is_empty() {
            return false;
        }
        // GitignoreBuilder was created with `root`, so it expects paths relative
        // to that root. Strip the prefix when given an absolute path; otherwise
        // use the path as-is (already relative). Passing a relative path avoids
        // the "path not under root" panic in matched_path_or_any_parents.
        let rel = path.strip_prefix(&self.root).unwrap_or(path);
        self.matcher
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sentrux-sigignore-{}-{}-{}",
            tag,
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

    #[test]
    fn missing_file_ignores_nothing() {
        let root = temp_root("missing");
        let ig = SentruxIgnore::load(&root);
        assert!(ig.is_empty());
        assert!(!ig.is_ignored(&root.join("src/main.rs"), false));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_prefix_excludes_files_under_it() {
        let root = temp_root("dir");
        fs::write(root.join(".sentruxignore"), "generated/\n").unwrap();
        let ig = SentruxIgnore::load(&root);
        assert!(!ig.is_empty());
        assert!(ig.is_ignored(&root.join("generated/a.cs"), false));
        assert!(ig.is_ignored(&root.join("generated"), true));
        assert!(!ig.is_ignored(&root.join("src/a.cs"), false));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn filename_glob_excludes_matching_files() {
        let root = temp_root("glob");
        fs::write(root.join(".sentruxignore"), "*.generated.cs\n").unwrap();
        let ig = SentruxIgnore::load(&root);
        assert!(ig.is_ignored(&root.join("src/foo.generated.cs"), false));
        assert!(ig.is_ignored(&root.join("deep/nested/bar.generated.cs"), false));
        assert!(!ig.is_ignored(&root.join("src/foo.cs"), false));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn nested_glob_excludes_matching_files() {
        let root = temp_root("nested");
        fs::write(root.join(".sentruxignore"), "**/*.foo\n").unwrap();
        let ig = SentruxIgnore::load(&root);
        assert!(ig.is_ignored(&root.join("a/b/c.foo"), false));
        assert!(ig.is_ignored(&root.join("top.foo"), false));
        assert!(!ig.is_ignored(&root.join("a/b/c.bar"), false));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn negation_reincludes_path() {
        let root = temp_root("negate");
        fs::write(root.join(".sentruxignore"), "*.cs\n!keep.cs\n").unwrap();
        let ig = SentruxIgnore::load(&root);
        assert!(ig.is_ignored(&root.join("drop.cs"), false));
        assert!(!ig.is_ignored(&root.join("keep.cs"), false));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relative_paths_also_match() {
        let root = temp_root("relative");
        fs::write(root.join(".sentruxignore"), "generated/\n").unwrap();
        let ig = SentruxIgnore::load(&root);
        // Path already relative to root (not absolute) must still match.
        assert!(ig.is_ignored(Path::new("generated/a.cs"), false));
        assert!(!ig.is_ignored(Path::new("src/a.cs"), false));
        let _ = fs::remove_dir_all(&root);
    }
}
