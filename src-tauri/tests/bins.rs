//! `BinManager::ensure_ready` is tested here and not in the crate: it reaches Tauri code that
//! imports `TaskDialogIndirect`, so the test executable needs the Common Controls manifest that
//! `build.rs` embeds only in integration tests.

use boladao_lib::bins::BinManager;
use futures_util::FutureExt;
use std::fs;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[test]
fn abandoned_ensure_ready_keeps_the_manager_alive_until_the_extraction_ends() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("bin");
    fs::create_dir_all(&root).unwrap();
    // A file where the yt-dlp folder belongs makes the extraction fail at once, writing nothing.
    fs::write(root.join("ytdlp"), b"").unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    // Keeps the only blocking thread busy, so the extraction waits in the queue.
    let (release, wait) = mpsc::channel::<()>();
    let (busy, is_busy) = mpsc::channel::<()>();
    rt.spawn_blocking(move || {
        busy.send(()).unwrap();
        let _ = wait.recv();
    });
    is_busy.recv().unwrap();

    let mgr = BinManager::with_root(root);
    let alive = Arc::downgrade(&mgr);
    {
        let _ctx = rt.enter();
        // The caller polls once and gives up, as a timeout or an aborted task would.
        assert!(mgr.ensure_ready(None, "stable".into()).now_or_never().is_none());
    }
    drop(mgr);
    assert!(
        alive.upgrade().is_some(),
        "the queued extraction must hold the manager after the caller drops it"
    );

    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive.strong_count() > 0 {
        assert!(Instant::now() < deadline, "the extraction never released the manager");
        std::thread::sleep(Duration::from_millis(10));
    }
}
