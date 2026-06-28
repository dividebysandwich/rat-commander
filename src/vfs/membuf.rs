//! In-memory async IO adapters used by backends that cannot stream directly.
//!
//! - [`MemReader`] serves a `Vec<u8>` as an `AsyncRead`.
//! - [`pipe_upload`] returns an `AsyncWrite` whose bytes are streamed to a
//!   user-supplied upload future through a bounded duplex pipe. Backpressure
//!   couples the writer's progress to the real network transfer, so the ops
//!   engine's progress bar reflects the actual upload rather than a local buffer
//!   fill. The transfer is finalized (and its result awaited) on `shutdown`.
//! - [`pipe_download`] is the mirror image: it returns an `AsyncRead` fed by a
//!   user-supplied download future, so a multi-gigabyte remote file streams
//!   through chunk-by-chunk (bounded memory) instead of being buffered whole
//!   before the copy can begin.

use crate::vfs::{BoxRead, BoxWrite};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::oneshot;

/// An async reader over an owned byte buffer.
pub struct MemReader {
    data: Vec<u8>,
    pos: usize,
}

impl MemReader {
    pub fn new(data: Vec<u8>) -> Self {
        MemReader { data, pos: 0 }
    }
}

impl AsyncRead for MemReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let remaining = self.data.len() - self.pos;
        let n = remaining.min(buf.remaining());
        if n > 0 {
            let start = self.pos;
            buf.put_slice(&self.data[start..start + n]);
            self.pos += n;
        }
        Poll::Ready(Ok(()))
    }
}

/// Create a streaming download reader. `download` receives the **write** end of
/// a bounded duplex pipe and should stream the remote file's bytes into it. The
/// returned [`BoxRead`] yields those bytes as they arrive; the pipe's bounded
/// capacity applies backpressure so the producer proceeds at the consumer's
/// (and thus the network's) pace, keeping memory flat. When `download` finishes
/// — or the reader is dropped — the pipe closes and the other side sees EOF.
pub fn pipe_download<F, Fut>(capacity: usize, download: F) -> BoxRead
where
    F: FnOnce(DuplexStream) -> Fut + Send + 'static,
    Fut: Future<Output = std::io::Result<()>> + Send + 'static,
{
    let (w, r) = tokio::io::duplex(capacity);
    tokio::spawn(async move {
        let _ = download(w).await;
    });
    Box::new(r)
}

type IoFut = Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

/// Create a streaming upload writer. `upload` receives the read end of a bounded
/// duplex pipe and should stream it to its destination (returning the transfer's
/// result). The returned [`AsyncWrite`] feeds bytes into the pipe — its writes
/// apply backpressure so they proceed at the upload's pace, and its `shutdown`
/// closes the pipe and awaits the upload result.
pub fn pipe_upload<F, Fut>(capacity: usize, upload: F) -> BoxWrite
where
    F: FnOnce(DuplexStream) -> Fut + Send + 'static,
    Fut: Future<Output = std::io::Result<()>> + Send + 'static,
{
    let (tx, rx) = tokio::io::duplex(capacity);
    let (done_tx, done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = done_tx.send(upload(rx).await);
    });
    Box::new(PipeWriter {
        tx: Some(tx),
        done: Some(done_rx),
        fut: None,
    })
}

struct PipeWriter {
    tx: Option<DuplexStream>,
    done: Option<oneshot::Receiver<std::io::Result<()>>>,
    fut: Option<IoFut>,
}

impl AsyncWrite for PipeWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut().tx.as_mut() {
            Some(tx) => Pin::new(tx).poll_write(cx, data),
            None => Poll::Ready(Ok(0)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut().tx.as_mut() {
            Some(tx) => Pin::new(tx).poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.fut.is_none() {
            // Close the pipe (so the upload task sees EOF), then await its result.
            let mut tx = this.tx.take();
            let done = this.done.take();
            this.fut = Some(Box::pin(async move {
                if let Some(tx) = tx.as_mut() {
                    tx.shutdown().await?;
                }
                drop(tx);
                match done {
                    Some(rx) => rx.await.unwrap_or(Ok(())),
                    None => Ok(()),
                }
            }));
        }
        this.fut.as_mut().unwrap().as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn pipe_upload_streams_and_finalizes_on_shutdown() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = received.clone();
        // Small capacity forces backpressure (the reader must drain concurrently).
        let mut w = pipe_upload(8, move |mut rx| async move {
            let mut buf = Vec::new();
            rx.read_to_end(&mut buf).await?;
            *sink.lock().await = buf;
            Ok(())
        });
        let data = b"hello pipe upload streaming test";
        w.write_all(data).await.unwrap();
        w.shutdown().await.unwrap(); // closes the pipe and awaits the upload
        assert_eq!(received.lock().await.as_slice(), data);
    }

    #[tokio::test]
    async fn pipe_download_streams_bytes_to_reader() {
        // Small capacity forces backpressure — the producer can only proceed as
        // the reader drains, which is exactly how a network download should pace.
        let mut r = pipe_download(8, move |mut w| async move {
            w.write_all(b"streamed download payload").await?;
            w.shutdown().await?;
            Ok(())
        });
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        assert_eq!(out.as_slice(), b"streamed download payload");
    }

    #[tokio::test]
    async fn pipe_upload_propagates_upload_error_on_shutdown() {
        let mut w = pipe_upload(64, move |mut rx| async move {
            let mut buf = Vec::new();
            let _ = rx.read_to_end(&mut buf).await;
            Err(std::io::Error::other("boom"))
        });
        let _ = w.write_all(b"data").await;
        assert!(w.shutdown().await.is_err(), "upload error surfaces on shutdown");
    }
}
