//! What the engine needs from an async runtime, on tokio (desktop) and on the
//! browser's event loop (wasm32), where there are no threads and
//! `std::time::Instant` panics.
use std::{future::Future, time::Duration};

pub use matrix_sdk::executor::{JoinHandle, spawn};
pub use matrix_sdk::sleep::sleep;
pub use web_time::Instant;

/// `limit` passed before the future finished.
#[derive(Debug)]
pub struct Elapsed;

pub async fn timeout<F: Future>(limit: Duration, future: F) -> Result<F::Output, Elapsed> {
    use futures_util::future::{Either, select};
    let future = std::pin::pin!(future);
    let timer = std::pin::pin!(sleep(limit));
    match select(future, timer).await {
        Either::Left((output, _)) => Ok(output),
        Either::Right(_) => Err(Elapsed),
    }
}

/// CPU-bound work (image decoding, file writes): on tokio's blocking pool on
/// the desktop, inline in the browser.
#[cfg(not(target_arch = "wasm32"))]
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}
#[cfg(target_arch = "wasm32")]
pub async fn blocking<T>(work: impl FnOnce() -> T) -> Option<T> {
    Some(work())
}
