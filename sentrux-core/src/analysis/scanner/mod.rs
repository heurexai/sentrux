//! Full directory scanner — walks filesystem, counts lines, parses structure, builds graphs.
//!
//! Uses `ignore` crate for gitignore-aware walking, `tokei` for line counting,
//! and `rayon` for parallel file processing. Produces a complete `Snapshot`.
//! Reports progress via callback for UI progress bars.

pub mod common;
pub mod rescan;
pub(crate) mod sentruxignore;
mod tree;

use self::common::{
    count_lines_from_bytes, detect_lang, should_ignore_dir, should_ignore_file, ScanLimits,
    MAX_FILES,
};
use self::sentruxignore::SentruxIgnore;
use self::tree::build_tree;
use crate::core::snapshot::{ScanProgress, Snapshot};
use crate::core::types::AppError;
use crate::core::types::FileNode;
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// Collected file info from the filesystem walk phase.
/// Captures path and mtime to avoid redundant metadata calls.
struct CollectedFile {
    path: PathBuf,
    mtime: f64,
}

/// Optional scan behavior for callers that need to widen the default project file set.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScanOptions {
    /// Include untracked, non-ignored Git worktree files in addition to tracked files.
    pub include_untracked: bool,
}

/// Extract mtime as f64 seconds since UNIX_EPOCH from file metadata.
pub(crate) fn extract_mtime(meta: &fs::Metadata, path: &Path) -> f64 {
    meta.modified()
        .map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or_else(|e| {
                    crate::debug_log!("[scanner] mtime before epoch for {:?}: {}", path, e);
                    std::time::Duration::ZERO
                })
                .as_secs_f64()
        })
        .unwrap_or(0.0) // Filesystem doesn't support mtime (some network mounts)
}

/// Process a single walker entry: check filters, extract metadata, send to channel.
/// Returns Quit if the file limit was reached or the channel is disconnected.
fn process_walk_entry(
    entry: &ignore::DirEntry,
    file_size_limit: u64,
    count: &std::sync::atomic::AtomicUsize,
    tx: &crossbeam_channel::Sender<CollectedFile>,
) -> ignore::WalkState {
    use std::sync::atomic::Ordering;

    if !entry.file_type().is_some_and(|ft| ft.is_file()) {
        return ignore::WalkState::Continue;
    }
    let path = entry.path().to_path_buf();
    if should_ignore_file(&path) {
        return ignore::WalkState::Continue;
    }
    let meta = match fs::metadata(&path) {
        Ok(m) if m.len() <= file_size_limit => m,
        _ => return ignore::WalkState::Continue,
    };
    let prev = count.fetch_add(1, Ordering::AcqRel);
    if prev >= MAX_FILES {
        return ignore::WalkState::Quit;
    }
    let mtime = extract_mtime(&meta, &path);
    if tx.send(CollectedFile { path, mtime }).is_err() {
        return ignore::WalkState::Quit;
    }
    ignore::WalkState::Continue
}

/// Collect file paths using `git ls-files` for git repos (the universal, correct source
/// of "what files belong to this project"), falling back to filesystem walk for non-git dirs.
///
/// First-principles reasoning: the user's git index is the single source of truth for
/// what constitutes "their code." It handles .gitignore, monorepos, workspaces, and
/// any project structure without heuristics or hardcoded ignore lists.
fn collect_paths(
    root: &Path,
    file_size_limit: u64,
    options: ScanOptions,
) -> Result<Vec<CollectedFile>, AppError> {
    // Assemble the candidate list from the appropriate source...
    let candidates = collect_paths_raw(root, file_size_limit, options)?;
    // ...then apply `.sentruxignore` in ONE place so it covers BOTH the
    // `git ls-files` path (which can return git-TRACKED files that `.gitignore`
    // cannot drop) and the filesystem-walk fallback.
    Ok(apply_sentruxignore(root, candidates))
}

/// Assemble candidate files from `git ls-files` (preferred) or the filesystem
/// walk fallback, before `.sentruxignore` filtering is applied.
fn collect_paths_raw(
    root: &Path,
    file_size_limit: u64,
    options: ScanOptions,
) -> Result<Vec<CollectedFile>, AppError> {
    // Try git ls-files first — the universal correct approach
    if let Some(files) = collect_paths_git(root, file_size_limit, options)? {
        if !files.is_empty() {
            crate::debug_log!("[scan] using git ls-files ({} files)", files.len());
            return Ok(files);
        }
    }
    // Fallback: filesystem walk for non-git directories
    crate::debug_log!("[scan] not a git repo, falling back to filesystem walk");
    Ok(collect_paths_walk(root, file_size_limit))
}

/// Drop any candidate file matched by `<root>/.sentruxignore`. Missing file =>
/// no-op. Unlike `.gitignore`, this removes git-TRACKED files too, which is the
/// entire purpose of `.sentruxignore`.
fn apply_sentruxignore(root: &Path, files: Vec<CollectedFile>) -> Vec<CollectedFile> {
    let ignore = SentruxIgnore::load(root);
    if ignore.is_empty() {
        return files;
    }
    let before = files.len();
    let kept: Vec<CollectedFile> = files
        .into_iter()
        // Candidates here are always files (not directories), so is_dir = false.
        .filter(|f| !ignore.is_ignored(&f.path, false))
        .collect();
    let dropped = before - kept.len();
    if dropped > 0 {
        crate::debug_log!("[scan] .sentruxignore dropped {} file(s)", dropped);
    }
    kept
}

/// Collect files via `git ls-files` — returns None if not a git repo or git fails.
/// This is the primary path: git already knows every tracked file, respects .gitignore,
/// handles monorepos/workspaces, and requires zero heuristic filtering.
fn collect_paths_git(
    root: &Path,
    file_size_limit: u64,
    options: ScanOptions,
) -> Result<Option<Vec<CollectedFile>>, AppError> {
    let tracked = match git_ls_files(root, &["ls-files", "-z"]) {
        GitLsFilesResult::Ok(files) => files,
        GitLsFilesResult::NotGitOrUnavailable => return Ok(None),
        GitLsFilesResult::Failed(message) => {
            crate::debug_log!("[scan] git ls-files failed: {message}");
            return Ok(None);
        }
    };
    let untracked = if options.include_untracked {
        match git_ls_files(root, &["ls-files", "-z", "--others", "--exclude-standard"]) {
            GitLsFilesResult::Ok(files) => files,
            GitLsFilesResult::NotGitOrUnavailable | GitLsFilesResult::Failed(_) => {
                return Err(AppError::Scan(format!(
                    "SENTRUX-GIT-UNTRACKED-ENUM-FAILED: failed to enumerate untracked files under {}",
                    root.display()
                )));
            }
        }
    } else {
        Vec::new()
    };

    let total_tracked = tracked.len();
    let total_untracked = untracked.len();
    let mut seen = HashSet::new();
    let entries: Vec<String> = tracked
        .into_iter()
        .chain(untracked)
        .filter(|rel| seen.insert(rel.clone()))
        .collect();
    let total_git = entries.len();
    let mut ignored_ext = 0u32;
    let mut meta_fail = 0u32;
    let mut too_big = 0u32;
    let files: Vec<CollectedFile> = entries
        .iter()
        .take(MAX_FILES)
        .filter_map(|rel| {
            let abs = root.join(rel);
            if should_ignore_file(&abs) {
                ignored_ext += 1;
                return None;
            }
            let meta = match fs::metadata(&abs) {
                Ok(m) => m,
                Err(_) => {
                    meta_fail += 1;
                    return None;
                }
            };
            if !meta.is_file() || meta.len() > file_size_limit {
                if meta.len() > file_size_limit {
                    too_big += 1;
                }
                return None;
            }
            let mtime = extract_mtime(&meta, &abs);
            Some(CollectedFile { path: abs, mtime })
        })
        .collect();

    let dropped = total_git - files.len();
    if dropped > 0 || options.include_untracked {
        eprintln!(
            "[scan] git ls-files: {} total ({} tracked, {} untracked), {} kept, {} dropped (ext:{}, meta:{}, big:{})",
            total_git, total_tracked, total_untracked, files.len(), dropped, ignored_ext, meta_fail, too_big
        );
    }
    Ok(Some(files))
}

enum GitLsFilesResult {
    Ok(Vec<String>),
    NotGitOrUnavailable,
    Failed(String),
}

fn git_ls_files(root: &Path, args: &[&str]) -> GitLsFilesResult {
    if args.iter().any(|arg| *arg == "--others") {
        let Some(value) = std::env::var_os("SENTRUX_TEST_FAIL_UNTRACKED_ENUM") else {
            return run_git_ls_files(root, args);
        };
        let value = value.to_string_lossy();
        let root_text = root.to_string_lossy();
        if value == "1" || root_text.contains(value.as_ref()) {
            return GitLsFilesResult::Failed("forced untracked enumeration failure".into());
        }
    }

    run_git_ls_files(root, args)
}

fn run_git_ls_files(root: &Path, args: &[&str]) -> GitLsFilesResult {
    let output = match std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
    {
        Ok(output) => output,
        Err(_) => return GitLsFilesResult::NotGitOrUnavailable,
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.contains("not a git repository") || stderr.contains("not a git repo") {
            return GitLsFilesResult::NotGitOrUnavailable;
        }
        return GitLsFilesResult::Failed(stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    GitLsFilesResult::Ok(
        stdout
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// Fallback: filesystem walk for non-git directories.
/// Uses `ignore` crate with hardcoded ignore list (only for non-git repos).
fn collect_paths_walk(root: &Path, file_size_limit: u64) -> Vec<CollectedFile> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let count = Arc::new(AtomicUsize::new(0));
    // MUST be unbounded: run() blocks until all walker threads finish, and
    // rx.iter() only runs after run() returns. A bounded channel deadlocks
    // when walker threads fill it and block on send() — nobody is reading.
    let (tx, rx) = crossbeam_channel::unbounded::<CollectedFile>();

    let count_w = Arc::clone(&count);
    WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(20))
        .threads(rayon::current_num_threads().min(8))
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                return !should_ignore_dir(&name);
            }
            true
        })
        .build_parallel()
        .run(|| {
            let tx = tx.clone();
            let count = Arc::clone(&count_w);
            Box::new(move |result| {
                if count.load(Ordering::Acquire) >= MAX_FILES {
                    return ignore::WalkState::Quit;
                }
                if let Ok(entry) = result {
                    return process_walk_entry(&entry, file_size_limit, &count, &tx);
                }
                ignore::WalkState::Continue
            })
        });
    drop(tx); // close sender so rx.iter() terminates

    rx.iter().collect()
}

/// Scan + parse a single file in one pass: read once, count lines, tree-sitter parse.
/// No tokei dependency — line counts computed from raw bytes + AST comment nodes.
fn scan_and_parse_file(
    collected: &CollectedFile,
    root: &Path,
    max_parse_size_kb: usize,
) -> FileNode {
    let rel = collected.path.strip_prefix(root).unwrap_or(&collected.path);
    // Normalize to forward slashes — ONE place, ALL platforms.
    // Every downstream consumer (resolver, graph builder, treemap) uses `/`.
    let rel_str = common::normalize_path(rel.to_string_lossy());
    let name = collected
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let lang = detect_lang(&collected.path);

    // Read content ONCE — used for both line counting and tree-sitter parse
    let content = match fs::read(&collected.path) {
        Ok(c) => c,
        Err(_) => {
            return FileNode {
                path: rel_str,
                name,
                is_dir: false,
                lines: 0,
                logic: 0,
                comments: 0,
                blanks: 0,
                funcs: 0,
                mtime: collected.mtime,
                gs: String::new(),
                lang,
                sa: None,
                children: None,
            };
        }
    };

    // Count total lines + blank lines from raw bytes (microseconds, zero alloc)
    let lc = count_lines_from_bytes(&content);

    // Tree-sitter parse (if language supported and file within parse size limit)
    let (sa, comment_count) =
        if !lang.is_empty() && lang != "unknown" && content.len() <= max_parse_size_kb * 1024 {
            match crate::analysis::parser::parse_file_from_content(&content, &lang) {
                Some(sa) => {
                    let cl = sa.comment_lines.unwrap_or(0);
                    (Some(sa), cl)
                }
                None => (None, 0),
            }
        } else {
            (None, 0)
        };

    let total = lc.total;
    let blanks = lc.blanks;
    let comments = comment_count;
    let logic = total.saturating_sub(comments).saturating_sub(blanks);
    let funcs = sa
        .as_ref()
        .and_then(|s| s.functions.as_ref())
        .map_or(0, |v| v.len() as u32);

    FileNode {
        path: rel_str,
        name,
        is_dir: false,
        lines: total,
        logic,
        comments,
        blanks,
        funcs,
        mtime: collected.mtime,
        gs: String::new(),
        lang,
        sa,
        children: None,
    }
}

/// Collect files, scan + parse each in parallel. One read per file, cancellable.
/// Replaces the old three-phase approach (collect → tokei → scan → parse).
fn walk_and_scan_files(
    root: &Path,
    max_file_size: u64,
    max_parse_size_kb: usize,
    options: ScanOptions,
    scan_t0: std::time::Instant,
    emit: &dyn Fn(&str, u8),
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<FileNode>, AppError> {
    emit("Collecting files\u{2026}", 5);
    let collected = collect_paths(root, max_file_size * 1024, options)?;
    let total_files = collected.len();
    crate::debug_log!(
        "[scan] collect_paths: {:.1}ms ({} files)",
        scan_t0.elapsed().as_secs_f64() * 1000.0,
        total_files
    );

    emit(&format!("Scanning & parsing ({total_files} files)"), 15);

    // Parallel scan + parse per file with cancel check.
    // Progress is reported via atomic counter — the emit callback runs on
    // the main scan thread after rayon completes, not inside rayon workers.
    let files: Vec<FileNode> = collected
        .par_iter()
        .filter_map(|c| {
            if let Some(ct) = cancel {
                if ct.load(std::sync::atomic::Ordering::Relaxed) {
                    return None;
                }
            }
            Some(scan_and_parse_file(c, root, max_parse_size_kb))
        })
        .collect();

    crate::debug_log!(
        "[scan] scan_and_parse: {:.1}ms ({} files)",
        scan_t0.elapsed().as_secs_f64() * 1000.0,
        files.len()
    );
    emit(&format!("Scanned {total_files} files"), 50);
    Ok(files)
}

/// Apply git statuses to file nodes in-place.
fn apply_git_statuses(
    files: &mut [FileNode],
    root_path: &str,
    scan_t0: std::time::Instant,
    emit: &dyn Fn(&str, u8),
) {
    let total_files = files.len();
    emit(&format!("Git status ({total_files} files)"), 40);
    let git_statuses = crate::analysis::git::get_statuses(root_path);
    for file in files.iter_mut() {
        if let Some(gs) = git_statuses.get(&file.path) {
            file.gs = gs.clone();
        }
    }
    crate::debug_log!(
        "[scan] git_status: {:.1}ms",
        scan_t0.elapsed().as_secs_f64() * 1000.0
    );
}

/// Poll parse progress until completion, emitting progress updates.
/// Accepts the parse thread handle to detect panics — if the thread dies
/// before all work is done, we break instead of spinning forever. [C2 fix]
/// Context for the tree-building and graph-building phase of a scan.
struct BuildContext<'a> {
    root: &'a Path,
    max_call_targets: usize,
    include_untracked: bool,
    scan_t0: std::time::Instant,
    emit: &'a dyn Fn(&str, u8),
    on_tree_ready: Option<&'a dyn Fn(Snapshot)>,
}

/// Build the file tree and emit a tree-ready snapshot, then build graphs.
fn build_tree_and_graphs(files: Vec<FileNode>, bctx: &BuildContext<'_>) -> ScanResult {
    // Use u64 to prevent overflow when summing line counts across many files. [ref:4e8f1175]
    let total_lines: u32 = files
        .iter()
        .map(|f| f.lines as u64)
        .sum::<u64>()
        .min(u32::MAX as u64) as u32;
    let total_files = files.len() as u32;
    let root_name = bctx
        .root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    (bctx.emit)(&format!("Building tree ({total_files} files)"), 65);
    let (tree, total_dirs) = build_tree(files, &root_name);
    let tree = Arc::new(tree);

    if let Some(cb) = bctx.on_tree_ready {
        cb(Snapshot {
            root: Arc::clone(&tree),
            total_files,
            total_lines,
            total_dirs,
            include_untracked: bctx.include_untracked,
            csharp_reference_stats: Default::default(),
            call_graph: Vec::new(),
            import_graph: Vec::new(),
            inherit_graph: Vec::new(),
            entry_points: Vec::new(),
            exec_depth: HashMap::new(),
        });
    }

    crate::debug_log!(
        "[scan] tree_ready sent at: {:.1}ms",
        bctx.scan_t0.elapsed().as_secs_f64() * 1000.0
    );
    (bctx.emit)(
        &format!("Building graphs ({total_files} files, {total_dirs} dirs)"),
        85,
    );
    let flat_files = crate::core::snapshot::flatten_files_ref(&tree);
    let gr =
        crate::analysis::graph::build_graphs(&flat_files, Some(bctx.root), bctx.max_call_targets);

    crate::debug_log!(
        "[scan] build_graphs done at: {:.1}ms | {} import, {} call, {} inherit edges",
        bctx.scan_t0.elapsed().as_secs_f64() * 1000.0,
        gr.import_edges.len(),
        gr.call_edges.len(),
        gr.inherit_edges.len()
    );
    (bctx.emit)("Done", 100);

    ScanResult {
        snapshot: Snapshot {
            root: tree,
            total_files,
            total_lines,
            total_dirs,
            include_untracked: bctx.include_untracked,
            csharp_reference_stats: gr.csharp_reference_stats,
            call_graph: gr.call_edges,
            import_graph: gr.import_edges,
            inherit_graph: gr.inherit_edges,
            entry_points: gr.entry_points,
            exec_depth: gr.exec_depth,
        },
    }
}

/// Main scan function: collect files, scan + parse each in parallel, build tree + graphs.
/// Single read per file — no tokei dependency, immediate cancellation between files.
pub fn scan_directory(
    root_path: &str,
    on_progress: Option<&dyn Fn(ScanProgress)>,
    on_tree_ready: Option<&dyn Fn(Snapshot)>,
    limits: &ScanLimits,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<ScanResult, AppError> {
    scan_directory_with_options(
        root_path,
        on_progress,
        on_tree_ready,
        limits,
        cancel,
        ScanOptions::default(),
    )
}

/// Main scan function with caller-selected scan options.
pub fn scan_directory_with_options(
    root_path: &str,
    on_progress: Option<&dyn Fn(ScanProgress)>,
    on_tree_ready: Option<&dyn Fn(Snapshot)>,
    limits: &ScanLimits,
    cancel: Option<&std::sync::atomic::AtomicBool>,
    options: ScanOptions,
) -> Result<ScanResult, AppError> {
    let scan_t0 = std::time::Instant::now();
    let root = Path::new(root_path);
    if !root.exists() || !root.is_dir() {
        return Err(AppError::Path(format!(
            "Not a valid directory: {}",
            root_path
        )));
    }

    let emit = |step: &str, pct: u8| {
        if let Some(cb) = on_progress {
            cb(ScanProgress {
                step: step.into(),
                pct,
            });
        }
    };

    // Single pass: collect + scan + parse per file (no tokei, no separate parse phase)
    let mut files = walk_and_scan_files(
        root,
        limits.max_file_size_kb,
        limits.max_parse_size_kb,
        options,
        scan_t0,
        &emit,
        cancel,
    )?;

    // Check cancel
    if let Some(ct) = cancel {
        if ct.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(AppError::Scan("Scan cancelled".into()));
        }
    }

    apply_git_statuses(&mut files, root_path, scan_t0, &emit);

    let bctx = BuildContext {
        root,
        max_call_targets: limits.max_call_targets,
        include_untracked: options.include_untracked,
        scan_t0,
        emit: &emit,
        on_tree_ready,
    };
    Ok(build_tree_and_graphs(files, &bctx))
}

#[cfg(test)]
mod scan_options_tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn include_untracked_adds_git_worktree_files() {
        let root =
            std::env::temp_dir().join(format!("sentrux-include-untracked-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let result = (|| {
            run_git(&root, &["init"]);
            fs::write(root.join("tracked.rs"), "pub fn tracked() {}\n").unwrap();
            run_git(&root, &["add", "tracked.rs"]);
            fs::write(root.join("new_file.rs"), "pub fn new_file() {}\n").unwrap();

            let limits = ScanLimits {
                max_file_size_kb: 2048,
                max_parse_size_kb: 512,
                max_call_targets: 5,
            };
            let root_str = root.to_str().unwrap();
            let tracked_only = scan_directory(root_str, None, None, &limits, None).unwrap();
            let with_untracked = scan_directory_with_options(
                root_str,
                None,
                None,
                &limits,
                None,
                ScanOptions {
                    include_untracked: true,
                },
            )
            .unwrap();

            assert_eq!(tracked_only.snapshot.total_files, 1);
            assert_eq!(with_untracked.snapshot.total_files, 2);
        })();

        let _ = fs::remove_dir_all(&root);
        result
    }

    #[test]
    fn include_untracked_fails_when_untracked_enumeration_fails() {
        struct EnvGuard;
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                std::env::remove_var("SENTRUX_TEST_FAIL_UNTRACKED_ENUM");
            }
        }

        let root = unique_root("untracked-enum-fail");
        let _guard = EnvGuard;
        let result = (|| {
            run_git(&root, &["init"]);
            fs::write(root.join("tracked.rs"), "pub fn tracked() {}\n").unwrap();
            run_git(&root, &["add", "tracked.rs"]);
            std::env::set_var(
                "SENTRUX_TEST_FAIL_UNTRACKED_ENUM",
                root.to_string_lossy().to_string(),
            );

            let err = match scan_directory_with_options(
                root.to_str().unwrap(),
                None,
                None,
                &test_limits(),
                None,
                ScanOptions {
                    include_untracked: true,
                },
            ) {
                Ok(_) => panic!("scan unexpectedly succeeded"),
                Err(err) => err,
            };
            assert!(
                err.to_string()
                    .contains("SENTRUX-GIT-UNTRACKED-ENUM-FAILED"),
                "expected stable untracked enumeration failure, got {err}"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
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

    fn unique_root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sentrux-{}-{}-{}",
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

    fn test_limits() -> ScanLimits {
        ScanLimits {
            max_file_size_kb: 2048,
            max_parse_size_kb: 512,
            max_call_targets: 5,
        }
    }

    /// CRITICAL behavior: `.sentruxignore` must drop git-TRACKED files matched by
    /// a directory entry — something `.gitignore` cannot do.
    #[test]
    fn sentruxignore_directory_excludes_tracked_file() {
        let root = unique_root("sig-dir");
        let result = (|| {
            run_git(&root, &["init"]);
            fs::create_dir_all(root.join("generated")).unwrap();
            fs::write(root.join("keep.rs"), "pub fn keep() {}\n").unwrap();
            fs::write(root.join("generated/gen.rs"), "pub fn gen() {}\n").unwrap();
            // Both files are git-TRACKED — .gitignore could not exclude them.
            run_git(&root, &["add", "keep.rs", "generated/gen.rs"]);

            let root_str = root.to_str().unwrap();
            let before = scan_directory(root_str, None, None, &test_limits(), None).unwrap();
            assert_eq!(
                before.snapshot.total_files, 2,
                "baseline must see both tracked files"
            );

            // Add .sentruxignore excluding the generated/ directory.
            fs::write(root.join(".sentruxignore"), "generated/\n").unwrap();
            let after = scan_directory(root_str, None, None, &test_limits(), None).unwrap();
            assert_eq!(
                after.snapshot.total_files, 1,
                "tracked file under generated/ must be dropped by .sentruxignore"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    /// A filename glob pattern excludes matching tracked files.
    #[test]
    fn sentruxignore_filename_glob_excludes_tracked_files() {
        let root = unique_root("sig-glob");
        let result = (|| {
            run_git(&root, &["init"]);
            fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
            fs::write(root.join("a.generated.rs"), "pub fn g() {}\n").unwrap();
            run_git(&root, &["add", "a.rs", "a.generated.rs"]);
            fs::write(root.join(".sentruxignore"), "*.generated.rs\n").unwrap();

            let root_str = root.to_str().unwrap();
            let after = scan_directory(root_str, None, None, &test_limits(), None).unwrap();
            assert_eq!(
                after.snapshot.total_files, 1,
                "*.generated.rs must be excluded, a.rs must remain"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    /// Negation (`!keep`) re-includes a path excluded by a broader pattern.
    #[test]
    fn sentruxignore_negation_reincludes_tracked_file() {
        let root = unique_root("sig-neg");
        let result = (|| {
            run_git(&root, &["init"]);
            fs::write(root.join("drop.rs"), "pub fn drop_me() {}\n").unwrap();
            fs::write(root.join("keep.rs"), "pub fn keep() {}\n").unwrap();
            run_git(&root, &["add", "drop.rs", "keep.rs"]);
            fs::write(root.join(".sentruxignore"), "*.rs\n!keep.rs\n").unwrap();

            let root_str = root.to_str().unwrap();
            let after = scan_directory(root_str, None, None, &test_limits(), None).unwrap();
            assert_eq!(
                after.snapshot.total_files, 1,
                "only keep.rs should remain after negation"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }

    /// Absence of `.sentruxignore` changes nothing.
    #[test]
    fn sentruxignore_absent_changes_nothing() {
        let root = unique_root("sig-absent");
        let result = (|| {
            run_git(&root, &["init"]);
            fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
            fs::write(root.join("b.rs"), "pub fn b() {}\n").unwrap();
            run_git(&root, &["add", "a.rs", "b.rs"]);

            let root_str = root.to_str().unwrap();
            let after = scan_directory(root_str, None, None, &test_limits(), None).unwrap();
            assert_eq!(
                after.snapshot.total_files, 2,
                "no .sentruxignore means no exclusions"
            );
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }
}

/// Re-export for backward compatibility.
pub use self::common::ScanResult;
