//! Bounded-concurrency helper for fanning out many independent lark-cli calls.
//!
//! All fst commands that fan out N subprocess calls (audit export, backup
//! download-files) share the same shape: take a list, run an async worker per
//! item under a `Semaphore` cap, collect results back in input order. Capture
//! that shape once here so each caller only has to express "what to do with one
//! item".
//!
//! Worker closures capture their own `Arc<LarkCli>` / `Arc<PathBuf>` etc.; this
//! helper deliberately stays generic and stateless.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Standard progress bar style for fst loops.
const PROGRESS_TEMPLATE: &str = "[{bar:32.cyan/blue}] {pos}/{len} {percent:>3}% {wide_msg}";

/// Standard progress bar used by audit/backup loops. Cheap to clone
/// (`ProgressBar` internally wraps an `Arc`).
pub fn new_progress_bar(len: usize) -> ProgressBar {
    let bar = ProgressBar::new(len as u64);
    bar.set_style(
        ProgressStyle::with_template(PROGRESS_TEMPLATE)
            .expect("valid progress template")
            .progress_chars("##-"),
    );
    bar
}

/// Build a single-line spinner-style progress bar for an in-flight worker.
/// Used with [`MultiProgress`] to show N concurrent operations.
fn new_worker_bar(mp: &MultiProgress, msg: &str) -> ProgressBar {
    let bar = ProgressBar::new_spinner().with_message(msg.to_string());
    bar.set_style(
        ProgressStyle::with_template("{spinner} {wide_msg}")
            .expect("valid worker template")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    bar.enable_steady_tick(Duration::from_millis(100));
    mp.add(bar)
}

/// Run `worker` on every item in `items`, at most `concurrency` at a time.
///
/// Returns results in **input order**, not completion order. Errors must be
/// expressed inside the worker's return type (typically a status string in the
/// result struct); this helper does not abort early.
///
/// Two progress modes:
/// - `progress = Some(bar), multi = None`: a single aggregate bar; `bar.inc(1)`
///   is called once per item completion.
/// - `progress = Some(bar), multi = Some(mp)`: aggregate bar at the bottom,
///   plus one spinner per in-flight worker (labeled via `worker_label`).
///
/// When `multi` is `Some`, `worker_label` must also be `Some`; otherwise both
/// are ignored.
pub async fn run_concurrent<T, R, Fut>(
    items: Vec<T>,
    concurrency: usize,
    progress: Option<ProgressBar>,
    multi: Option<MultiProgress>,
    worker_label: Option<&dyn Fn(&T) -> String>,
    worker: impl Fn(usize, T) -> Fut + Send + Sync + 'static,
) -> Vec<R>
where
    T: Send + 'static,
    R: Send + 'static,
    Fut: Future<Output = R> + Send,
{
    let concurrency = concurrency.max(1);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let worker = Arc::new(worker);
    let total = items.len();

    // When using MultiProgress, the aggregate bar must live on the same
    // MultiProgress so spinner lines don't overlap it.
    let aggregate = match (&progress, &multi) {
        (Some(bar), Some(mp)) => {
            // Re-attach the caller-supplied aggregate to the MultiProgress so
            // all bars share one draw target. Cheap because ProgressBar is Arc.
            mp.add(bar.clone());
            Some(bar.clone())
        }
        (Some(bar), None) => Some(bar.clone()),
        _ => None,
    };

    let mut futures: FuturesUnordered<_> = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let permit = semaphore.clone();
            let worker = worker.clone();
            let aggregate = aggregate.clone();
            let mp = multi.clone();
            let label = worker_label.map(|f| f(&item));
            async move {
                let _permit = permit.acquire().await.expect("semaphore not closed");
                // Show a spinner while this worker runs.
                let worker_bar = match (&mp, label.as_ref()) {
                    (Some(mp), Some(label)) => Some(new_worker_bar(mp, label)),
                    _ => None,
                };
                let result = worker(index, item).await;
                if let Some(bar) = &worker_bar {
                    bar.finish_and_clear();
                }
                if let Some(aggregate) = &aggregate {
                    aggregate.inc(1);
                }
                (index, result)
            }
        })
        .collect();

    let mut indexed: Vec<(usize, R)> = Vec::with_capacity(total);
    while let Some(item) = futures.next().await {
        indexed.push(item);
    }

    indexed.sort_by_key(|(i, _)| *i);
    indexed.into_iter().map(|(_, r)| r).collect()
}
