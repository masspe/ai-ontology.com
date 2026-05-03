use ontology_graph::OntologyGraph;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::store::Store;

/// Handle to a background snapshotter. Dropping it does not stop the task;
/// call `stop()` to do so cleanly.
pub struct SnapshotHandle {
    notify: Arc<Notify>,
    join: JoinHandle<()>,
}

impl SnapshotHandle {
    /// Trigger an immediate snapshot in addition to the periodic one.
    pub fn trigger(&self) {
        self.notify.notify_one();
    }

    /// Stop the background task. Awaits its termination.
    pub async fn stop(self) {
        self.join.abort();
        let _ = self.join.await;
    }
}

/// Spawn a background task that calls `store.snapshot(graph)` every
/// `interval` and whenever the returned `SnapshotHandle::trigger()` is
/// called. Errors are logged but never panic the task.
pub fn spawn_snapshotter(
    store: Arc<dyn Store>,
    graph: Arc<OntologyGraph>,
    interval: Duration,
) -> SnapshotHandle {
    let notify = Arc::new(Notify::new());
    let n2 = notify.clone();
    let join = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {},
                _ = n2.notified() => {},
            }
            match store.snapshot(&graph).await {
                Ok(()) => debug!("snapshot complete"),
                Err(e) => warn!(error=%e, "snapshot failed"),
            }
        }
    });
    SnapshotHandle { notify, join }
}
