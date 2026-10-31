//! The pause checkpoint every downloader calls before starting an item.

use std::time::Duration;

use lumen_core::CancelFlag;

const POLL: Duration = Duration::from_millis(100);

/// Returns once the download is not paused, or as soon as it is cancelled, so
/// cancelling a paused run never waits for a resume.
pub async fn wait_if_paused(cancel: &CancelFlag) {
    while cancel.is_paused() && !cancel.is_cancelled() {
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    async fn spawn_waiter(flag: &CancelFlag) -> (tokio::task::JoinHandle<()>, Arc<AtomicBool>) {
        let done = Arc::new(AtomicBool::new(false));
        let (flag, marker) = (flag.clone(), done.clone());
        let handle = tokio::spawn(async move {
            wait_if_paused(&flag).await;
            marker.store(true, Ordering::SeqCst);
        });
        (handle, done)
    }

    #[tokio::test]
    async fn returns_at_once_when_not_paused() {
        let flag = CancelFlag::new();
        tokio::time::timeout(Duration::from_millis(50), wait_if_paused(&flag))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn holds_while_paused_and_continues_on_resume() {
        let flag = CancelFlag::new();
        flag.pause();
        let (handle, done) = spawn_waiter(&flag).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(!done.load(Ordering::SeqCst), "must hold while paused");
        flag.resume();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(done.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancelling_a_paused_run_releases_it_without_a_resume() {
        let flag = CancelFlag::new();
        flag.pause();
        let (handle, _) = spawn_waiter(&flag).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        flag.cancel();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(flag.is_paused(), "the pause itself is left as it was");
    }
}
