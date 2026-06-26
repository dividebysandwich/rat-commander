//! In-memory async IO adapters used by backends that cannot stream directly.
//!
//! - [`MemReader`] serves a `Vec<u8>` as an `AsyncRead` (e.g. an FTP/SCP file
//!   fetched whole into memory).
//! - [`CollectWriter`] buffers all writes and, on `shutdown`, runs a user
//!   finish-future that uploads the bytes. The ops engine calls `shutdown()`
//!   after writing each file, which is when the upload happens.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

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

type FinishFut = Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

/// Buffers all writes, then uploads them via a finish-future on `shutdown`.
pub struct CollectWriter {
    buf: Vec<u8>,
    finish: Option<Box<dyn FnOnce(Vec<u8>) -> FinishFut + Send>>,
    fut: Option<FinishFut>,
}

impl CollectWriter {
    pub fn new<F>(finish: F) -> Self
    where
        F: FnOnce(Vec<u8>) -> FinishFut + Send + 'static,
    {
        CollectWriter {
            buf: Vec::new(),
            finish: Some(Box::new(finish)),
            fut: None,
        }
    }
}

impl AsyncWrite for CollectWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.get_mut().buf.extend_from_slice(data);
        Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.fut.is_none() {
            let buf = std::mem::take(&mut this.buf);
            let finish = this
                .finish
                .take()
                .expect("CollectWriter shutdown called once");
            this.fut = Some(finish(buf));
        }
        this.fut.as_mut().unwrap().as_mut().poll(cx)
    }
}
