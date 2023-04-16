use std::future::Future;
use std::ops;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;

pub fn spawn<T>(task_future: T) -> JoinHandle<T::Output>
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    JoinHandle {
        inner: tokio::spawn(task_future),
    }
}

/// Handle to running task.
/// `.await`-ing it yields return value of the task.
/// Dropping it cancels the task.
#[derive(Debug)]
#[must_use]
pub struct JoinHandle<T> {
    inner: tokio::task::JoinHandle<T>,
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx).map(|res| Ok(res?))
    }
}

impl<T> ops::Drop for JoinHandle<T> {
    fn drop(&mut self) {
        self.inner.abort();
    }
}
