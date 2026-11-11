//! Streaming upload writer (spec 1.11, 1.12; JS `upload-writer.ts`).

use async_trait::async_trait;
use bytes::Bytes;
use futures::FutureExt;
use futures::future::{BoxFuture, Either, Shared, select};

use crate::FilesError;

/// The write side of a streaming upload (JS `PassThrough`).
#[async_trait]
pub trait UploadSink: Send {
    /// Resolves when the chunk is accepted (JS write callback).
    async fn write(&mut self, chunk: Bytes) -> std::io::Result<()>;
    /// JS `stream.end()`.
    fn close(&mut self);
    /// JS `stream.destroy()`.
    fn destroy(&mut self);
}

/// Outcome of the upload (JS `done` promise). Cloneable so the owner of the
/// upload can also await it.
pub type UploadDone = Shared<BoxFuture<'static, Result<(), FilesError>>>;

pub struct UploadHandle {
    pub sink: Box<dyn UploadSink>,
    pub done: UploadDone,
}

/// Writer over a streaming upload. Once the upload fails, a pending write is
/// rejected with that failure (a stalled sink would otherwise hold the
/// caller forever), and every later write is rejected at once without
/// touching the sink. Writes must be awaited one at a time.
pub struct UploadWriter {
    sink: Box<dyn UploadSink>,
    done: UploadDone,
    failure: Option<FilesError>,
    bytes_written: u64,
}

pub fn to_upload_writer(upload: UploadHandle) -> UploadWriter {
    UploadWriter {
        sink: upload.sink,
        done: upload.done,
        failure: None,
        bytes_written: 0,
    }
}

fn sink_error(error: std::io::Error) -> FilesError {
    FilesError::Storage(error.to_string())
}

impl UploadWriter {
    /// The latched upload failure, if the upload has failed by now.
    fn latched_failure(&mut self) -> Option<FilesError> {
        if self.failure.is_none() {
            if let Some(Err(error)) = self.done.clone().now_or_never() {
                self.failure = Some(error);
            }
        }
        self.failure.clone()
    }

    /// Resolves once the sink has taken the chunk; rejects as soon as the
    /// upload fails. Only accepted writes count toward
    /// [`bytes_written`](Self::bytes_written) (UTF-8 bytes).
    pub async fn write(&mut self, chunk: &str) -> Result<(), FilesError> {
        if let Some(error) = self.latched_failure() {
            return Err(error);
        }
        let len = chunk.len() as u64;
        let write = self.sink.write(Bytes::copy_from_slice(chunk.as_bytes()));
        let outcome = match select(write, self.done.clone()).await {
            Either::Left((result, _)) => result.map_err(sink_error),
            Either::Right((Err(error), _write)) => {
                self.failure = Some(error.clone());
                Err(error)
            }
            // The upload completed while a write was pending: wait for the
            // write itself (the JS write callback decides).
            Either::Right((Ok(()), write)) => write.await.map_err(sink_error),
        };
        if outcome.is_ok() {
            self.bytes_written += len;
        }
        outcome
    }

    /// UTF-8 bytes of every write the upload accepted so far.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Ends the sink and waits for the upload to complete.
    pub async fn finish(mut self) -> Result<(), FilesError> {
        self.sink.close();
        self.done.await
    }

    /// Destroys the sink and waits for the upload to settle, failed or not.
    pub async fn abort(mut self) {
        self.sink.destroy();
        let _ = self.done.await;
    }
}
