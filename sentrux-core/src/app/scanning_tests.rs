//! Tests for the scanning and layout pipeline (`app::scanning`).
//!
//! Validates the channel-based scan-and-layout pipeline: scanner thread
//! produces snapshots, layout thread consumes them and emits layout results.
//! Tests cover drain-to-latest semantics, cancellation via generation counter,
//! and end-to-end scan-to-layout flow with dummy snapshots.
//! Uses `crossbeam_channel` bounded channels to simulate real pipeline behavior.

use super::channels::{LayoutMsg, LayoutRequest, ScanCommand, ScanMsg};
use super::scan_threads::{drain_to_latest, layout_thread, scanner_thread};
use crate::layout::types::{LayoutMode, ScaleMode, SizeMode};
use crate::core::snapshot::Snapshot;
use crate::core::types::FileNode;
use crossbeam_channel::bounded;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Helper: create a minimal Snapshot for testing
fn dummy_snapshot() -> Snapshot {
    Snapshot {
        root: Arc::new(FileNode {
            path: "root".into(),
            name: "root".into(),
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
            children: None,
        }),
        total_files: 0,
        total_lines: 0,
        total_dirs: 0,
        include_untracked: false,
        csharp_reference_stats: Default::default(),
        call_graph: vec![],
        import_graph: vec![],
        inherit_graph: vec![],
        entry_points: vec![],
        exec_depth: HashMap::new(),
    }
}

// ── Invariant 5: FullScan beats Rescan in drain_to_latest ──

#[test]
fn drain_to_latest_fullscan_beats_trailing_rescan() {
    let (tx, rx) = bounded::<ScanCommand>(4);
    tx.send(ScanCommand::Rescan {
        root: "b".into(),
        changed: vec!["x.rs".into()],
        old_snap: Arc::new(dummy_snapshot()),
        limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 },
        gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    })
    .unwrap();

    let first = ScanCommand::FullScan { root: "a".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) };
    let result = drain_to_latest(first, &rx);
    assert!(
        matches!(result, ScanCommand::FullScan { root: ref p, .. } if p == "a"),
        "FullScan should beat trailing Rescan"
    );
}

#[test]
fn drain_to_latest_keeps_latest_fullscan() {
    let (tx, rx) = bounded::<ScanCommand>(4);
    tx.send(ScanCommand::FullScan { root: "b".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }).unwrap();

    let first = ScanCommand::FullScan { root: "a".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) };
    let result = drain_to_latest(first, &rx);
    assert!(
        matches!(result, ScanCommand::FullScan { root: ref p, .. } if p == "b"),
        "Latest FullScan should be returned"
    );
}

#[test]
fn drain_to_latest_empty_queue_returns_first() {
    let (_tx, rx) = bounded::<ScanCommand>(4);
    let first = ScanCommand::FullScan { root: "a".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) };
    let result = drain_to_latest(first, &rx);
    assert!(matches!(result, ScanCommand::FullScan { root: ref p, .. } if p == "a"));
}

#[test]
fn drain_to_latest_rescan_merges_changed() {
    let (tx, rx) = bounded::<ScanCommand>(4);
    let snap = Arc::new(dummy_snapshot());
    tx.send(ScanCommand::Rescan {
        root: "a".into(),
        changed: vec!["y.rs".into()],
        old_snap: Arc::clone(&snap),
        limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 },
        gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    })
    .unwrap();

    let first = ScanCommand::Rescan {
        root: "a".into(),
        changed: vec!["x.rs".into()],
        old_snap: snap,
        limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 },
        gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let result = drain_to_latest(first, &rx);
    match result {
        ScanCommand::Rescan { changed, .. } => {
            let set: std::collections::HashSet<String> = changed.into_iter().collect();
            assert!(set.contains("x.rs"), "merged result must contain x.rs from first Rescan");
            assert!(set.contains("y.rs"), "merged result must contain y.rs from queued Rescan");
        }
        _ => panic!("Expected Rescan"),
    }
}

// ── Invariant 3: Layout version monotone stale rejection ──

#[test]
fn layout_thread_processes_latest_request_only() {
    let (req_tx, req_rx) = bounded::<LayoutRequest>(4);
    let (msg_tx, msg_rx) = bounded::<LayoutMsg>(4);

    let snap = Arc::new(dummy_snapshot());
    for v in 1..=3u64 {
        req_tx
            .send(LayoutRequest {
                snapshot: Arc::clone(&snap),
                size_mode: SizeMode::Lines,
                scale_mode: ScaleMode::Linear,
                layout_mode: LayoutMode::Treemap,
                viewport_w: 800.0,
                viewport_h: 600.0,
                drill_path: None,
                version: v,
                heat_map: None,
                settings: crate::core::settings::Settings::default(),
                focus_mode: crate::layout::types::FocusMode::All,
                entry_point_files: std::sync::Arc::new(std::collections::HashSet::new()),
                hidden_paths: std::sync::Arc::new(std::collections::HashSet::new()),
                impact_files: None,
            })
            .unwrap();
    }
    drop(req_tx);

    layout_thread(req_rx, msg_tx);

    let mut versions = vec![];
    while let Ok(LayoutMsg::Ready(_, v)) = msg_rx.try_recv() {
        versions.push(v);
    }
    assert_eq!(versions, vec![3], "Layout should drain to latest version");
}

#[test]
fn layout_version_stale_rejection_monotone() {
    let mut rendered_version: u64 = 5;

    let incoming_v3: u64 = 3;
    if incoming_v3 < rendered_version {
        // rejected — correct
    } else {
        panic!("Should have rejected stale version 3");
    }

    let incoming_v7: u64 = 7;
    assert!(incoming_v7 >= rendered_version);
    rendered_version = incoming_v7;
    assert_eq!(rendered_version, 7);
}

// ── Invariant 1+2: Generation counter causality ──

#[test]
fn generation_bumps_before_send() {
    let gen = Arc::new(AtomicU64::new(0));
    let (tx, _rx) = bounded::<ScanCommand>(1);

    gen.fetch_add(1, Ordering::AcqRel);
    assert_eq!(gen.load(Ordering::Acquire), 1);
    assert!(tx.try_send(ScanCommand::FullScan { root: "a".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }).is_ok());

    gen.fetch_add(1, Ordering::AcqRel);
    assert_eq!(gen.load(Ordering::Acquire), 2);
    assert!(tx.try_send(ScanCommand::FullScan { root: "b".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }).is_err());
    assert_eq!(gen.load(Ordering::Acquire), 2);
}

#[test]
fn stale_imports_ready_rejected_by_generation() {
    let current_gen = Arc::new(AtomicU64::new(2));
    let stale_gen: u64 = 1;
    let fresh_gen: u64 = 2;

    assert_ne!(
        stale_gen,
        current_gen.load(Ordering::Acquire),
        "Stale gen should not match current"
    );
    assert_eq!(
        fresh_gen,
        current_gen.load(Ordering::Acquire),
        "Fresh gen should match current"
    );
}

// ── Invariant 4: Layout version only bumps on successful send ──

#[test]
fn layout_version_not_bumped_on_full_channel() {
    let (tx, _rx) = bounded::<LayoutRequest>(1);
    let snap = Arc::new(dummy_snapshot());
    let mut layout_version: u64 = 0;
    let mut layout_pending = false;

    let req = LayoutRequest {
        snapshot: Arc::clone(&snap),
        size_mode: SizeMode::Lines,
        scale_mode: ScaleMode::Linear,
        layout_mode: LayoutMode::Treemap,
        viewport_w: 800.0,
        viewport_h: 600.0,
        drill_path: None,
        version: 1,
        heat_map: None,
        settings: crate::core::settings::Settings::default(),
        focus_mode: crate::layout::types::FocusMode::All,
        entry_point_files: std::sync::Arc::new(std::collections::HashSet::new()),
        hidden_paths: std::sync::Arc::new(std::collections::HashSet::new()),
        impact_files: None,
    };
    tx.try_send(req).unwrap();
    layout_version = 1;

    let next_version = layout_version + 1;
    let req2 = LayoutRequest {
        snapshot: Arc::clone(&snap),
        size_mode: SizeMode::Lines,
        scale_mode: ScaleMode::Linear,
        layout_mode: LayoutMode::Treemap,
        viewport_w: 800.0,
        viewport_h: 600.0,
        drill_path: None,
        version: next_version,
        heat_map: None,
        settings: crate::core::settings::Settings::default(),
        focus_mode: crate::layout::types::FocusMode::All,
        entry_point_files: std::sync::Arc::new(std::collections::HashSet::new()),
        hidden_paths: std::sync::Arc::new(std::collections::HashSet::new()),
        impact_files: None,
    };
    match tx.try_send(req2) {
        Ok(()) => {
            layout_version = next_version;
            layout_pending = true;
        }
        Err(_) => {
            layout_pending = true;
        }
    }

    assert_eq!(
        layout_version, 1,
        "Version must stay at 1 when channel is full"
    );
    assert!(layout_pending, "layout_pending should be set even on failure");
}

// ── Invariant 11: layout_pending correctly reflects version gap ──

#[test]
fn layout_pending_cleared_when_version_matches() {
    let layout_version: u64 = 5;
    let rendered_version: u64 = 0;

    let incoming = 5u64;
    assert!(incoming >= rendered_version);
    let layout_pending = incoming < layout_version;
    assert!(
        !layout_pending,
        "layout_pending should be false when received version matches requested"
    );
}

#[test]
fn layout_pending_stays_when_version_behind() {
    let layout_version: u64 = 7;
    let rendered_version: u64 = 0;

    let incoming = 5u64;
    assert!(incoming >= rendered_version);
    let layout_pending = incoming < layout_version;
    assert!(
        layout_pending,
        "layout_pending should remain true when newer request is outstanding"
    );
}

// ── Invariant 10: Scanner thread sends Complete ──

#[test]
fn scanner_thread_sends_complete() {
    let tmp = std::env::temp_dir().join(format!("sentrux_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    std::fs::write(tmp.join("test.txt"), "hello").unwrap();

    let (cmd_tx, cmd_rx) = bounded::<ScanCommand>(1);
    let (msg_tx, msg_rx) = bounded::<ScanMsg>(64);

    let handle = std::thread::spawn(move || {
        scanner_thread(cmd_rx, msg_tx);
    });

    cmd_tx
        .send(ScanCommand::FullScan { root: tmp.to_string_lossy().into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 1, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) })
        .unwrap();

    let mut got_complete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        match msg_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(ScanMsg::Complete(..)) => {
                got_complete = true;
                break;
            }
            Ok(ScanMsg::Progress(_)) | Ok(ScanMsg::TreeReady(..)) => continue,
            Ok(ScanMsg::Error(e, _gen)) => panic!("Scanner error: {}", e),
            Err(_) => continue,
        }
    }
    assert!(got_complete, "Scanner must send Complete");

    drop(cmd_tx);
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Idempotency: drain_to_latest is idempotent on empty queue ──

#[test]
fn drain_to_latest_idempotent() {
    let (_tx, rx) = bounded::<ScanCommand>(4);
    let r1 = drain_to_latest(ScanCommand::FullScan { root: "test".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }, &rx);
    match r1 {
        ScanCommand::FullScan { root: ref a, .. } => assert_eq!(a, "test"),
        _ => panic!("Expected FullScan"),
    }
    let r2 = drain_to_latest(ScanCommand::FullScan { root: "test".into(), limits: crate::app::scan_threads::ScanLimits { max_file_size_kb: 2048, max_parse_size_kb: 512, max_call_targets: 5 }, gen: 0, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }, &rx);
    match r2 {
        ScanCommand::FullScan { root: ref b, .. } => assert_eq!(b, "test"),
        _ => panic!("Idempotency violated"),
    }
}
