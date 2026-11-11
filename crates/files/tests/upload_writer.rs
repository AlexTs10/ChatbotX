//! Port of `packages/filesystem/__tests__/upload-writer.test.ts` (P2.9).
//!
//! Mock mapping (spec section 5):
//! - `new PassThrough({ highWaterMark: 1 })` -> `FakeStream` (implements
//!   `UploadSink`). With `reading` it accepts and collects every chunk at
//!   once (the JS `data` listener). Without `reading` a write never
//!   completes and `need_drain` turns true (JS `writableNeedDrain`).
//! - `stream.writableEnded` / `stream.destroyed` -> `close()` / `destroy()`
//!   flags. `vi.spyOn(stream, "write")` -> a count of `write` calls.
//! - The hand-settled `done` promise -> a `oneshot` channel behind the
//!   shared `UploadDone` future; the test keeps a clone to await it.
//! - `vi.waitFor` -> poll every 50 ms for up to 1000 ms (vitest defaults).
//! - Every await on the writer is bounded by 5 s (vitest test timeout).
//! - `rejects.toThrow(msg)` -> the error message contains `msg`.

use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chatbotx_files::{FilesError, UploadDone, UploadHandle, UploadSink, to_upload_writer};
use futures::FutureExt;
use tokio::sync::oneshot;

// --- fakes -----------------------------------------------------------------

#[derive(Default)]
struct StreamState {
    chunks: Mutex<Vec<String>>,
    write_calls: AtomicUsize,
    need_drain: AtomicBool,
    ended: AtomicBool,
    destroyed: AtomicBool,
}

impl StreamState {
    fn joined_chunks(&self) -> String {
        self.chunks.lock().expect("lock").join("")
    }
    fn write_calls(&self) -> usize {
        self.write_calls.load(Ordering::SeqCst)
    }
    fn writable_need_drain(&self) -> bool {
        self.need_drain.load(Ordering::SeqCst)
    }
    fn writable_ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }
    fn destroyed(&self) -> bool {
        self.destroyed.load(Ordering::SeqCst)
    }
}

/// JS `PassThrough({ highWaterMark: 1 })`, with or without a `data` reader.
struct FakeStream {
    state: Arc<StreamState>,
    reading: bool,
}

#[async_trait]
impl UploadSink for FakeStream {
    async fn write(&mut self, chunk: Bytes) -> io::Result<()> {
        self.state.write_calls.fetch_add(1, Ordering::SeqCst);
        if self.reading {
            self.state
                .chunks
                .lock()
                .expect("lock")
                .push(String::from_utf8_lossy(&chunk).into_owned());
            Ok(())
        } else {
            // Nothing reads: the buffer stays above highWaterMark and the
            // write callback never fires.
            self.state.need_drain.store(true, Ordering::SeqCst);
            std::future::pending().await
        }
    }

    fn close(&mut self) {
        self.state.ended.store(true, Ordering::SeqCst);
    }

    fn destroy(&mut self) {
        self.state.destroyed.store(true, Ordering::SeqCst);
    }
}

/// JS `settle = { resolve, reject }` of the `done` promise.
struct Settle(Option<oneshot::Sender<Result<(), FilesError>>>);

impl Settle {
    fn resolve(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(Ok(()));
        }
    }

    /// JS `settle.reject(new Error(message))`.
    fn reject(&mut self, message: &str) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(Err(FilesError::Storage(message.to_owned())));
        }
    }
}

struct FakeUpload {
    upload: UploadHandle,
    state: Arc<StreamState>,
    settle: Settle,
    /// JS `upload.done`, for the test to await.
    done: UploadDone,
}

/// JS `createFakeUpload({ reading })`: a streaming upload whose outcome the
/// test decides.
fn create_fake_upload(reading: bool) -> FakeUpload {
    let state = Arc::new(StreamState::default());
    let (tx, rx) = oneshot::channel::<Result<(), FilesError>>();
    let done: UploadDone = async move {
        match rx.await {
            Ok(outcome) => outcome,
            // Never settled: stays pending, like the JS promise.
            Err(_) => std::future::pending().await,
        }
    }
    .boxed()
    .shared();
    FakeUpload {
        upload: UploadHandle {
            sink: Box::new(FakeStream {
                state: state.clone(),
                reading,
            }),
            done: done.clone(),
        },
        state,
        settle: Settle(Some(tx)),
        done,
    }
}

/// Vitest's default test timeout (5 s): a hang fails the test.
async fn within_test_timeout<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("test timed out")
}

/// `vi.waitFor` with its defaults (timeout 1000 ms, interval 50 ms).
async fn wait_for(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_millis(1000), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("vi.waitFor timed out");
}

fn assert_message_contains(error: &FilesError, expected: &str) {
    assert!(
        error.to_string().contains(expected),
        "expected error containing {expected:?}, got {:?}",
        error.to_string()
    );
}

// --- tests -----------------------------------------------------------------

/// Port of: packages/filesystem/__tests__/upload-writer.test.ts:23 "toUploadWriter > writes chunks in order and finishes once the upload completes"
#[tokio::test]
async fn to_upload_writer_writes_chunks_in_order_and_finishes_once_the_upload_completes() {
    let FakeUpload {
        upload,
        state,
        mut settle,
        ..
    } = create_fake_upload(true);
    let mut writer = to_upload_writer(upload);

    within_test_timeout(writer.write("a,b\n")).await.unwrap();
    within_test_timeout(writer.write("1,2\n")).await.unwrap();
    // JS calls finish() before it resolves `done`.
    let finished = tokio::spawn(writer.finish());
    tokio::task::yield_now().await;
    settle.resolve();
    within_test_timeout(finished)
        .await
        .expect("finish task")
        .unwrap();

    assert_eq!(state.joined_chunks(), "a,b\n1,2\n");
    assert!(state.writable_ended());
}

/// Port of: packages/filesystem/__tests__/upload-writer.test.ts:37 "toUploadWriter > counts the UTF-8 bytes of every accepted write"
#[tokio::test]
async fn to_upload_writer_counts_the_utf_8_bytes_of_every_accepted_write() {
    let FakeUpload { upload, .. } = create_fake_upload(true);
    let mut writer = to_upload_writer(upload);
    assert_eq!(writer.bytes_written(), 0);

    within_test_timeout(writer.write("\u{FEFF}a\n"))
        .await
        .unwrap(); // BOM is 3 bytes
    within_test_timeout(writer.write("Nguy\u{1EC5}n\n"))
        .await
        .unwrap(); // ễ is 3 bytes

    // JS Buffer.byteLength(...) == Rust str::len (UTF-8 bytes).
    assert_eq!(
        writer.bytes_written(),
        "\u{FEFF}a\nNguy\u{1EC5}n\n".len() as u64
    );
}

/// Port of: packages/filesystem/__tests__/upload-writer.test.ts:48 "toUploadWriter > does not count a rejected write"
#[tokio::test]
async fn to_upload_writer_does_not_count_a_rejected_write() {
    let FakeUpload {
        upload,
        mut settle,
        done,
        ..
    } = create_fake_upload(true);
    let mut writer = to_upload_writer(upload);
    settle.reject("part upload failed");
    let _ = done.await; // upload.done.catch(() => undefined)

    let error = within_test_timeout(writer.write("lost"))
        .await
        .expect_err("expected rejection");
    assert_message_contains(&error, "part upload failed");
    assert_eq!(writer.bytes_written(), 0);
}

/// Port of: packages/filesystem/__tests__/upload-writer.test.ts:58 "toUploadWriter > rejects a write stuck on backpressure as soon as the upload fails"
#[tokio::test]
async fn to_upload_writer_rejects_a_write_stuck_on_backpressure_as_soon_as_the_upload_fails() {
    // Nothing reads the stream, as when every lib-storage part upload failed.
    let FakeUpload {
        upload,
        state,
        mut settle,
        ..
    } = create_fake_upload(false);
    let mut writer = to_upload_writer(upload);

    let pending = tokio::spawn(async move { writer.write("a large chunk").await });
    wait_for(|| state.writable_need_drain()).await;
    settle.reject("part upload failed");

    let error = within_test_timeout(pending)
        .await
        .expect("write task")
        .expect_err("expected rejection");
    assert_message_contains(&error, "part upload failed");
}

/// Port of: packages/filesystem/__tests__/upload-writer.test.ts:70 "toUploadWriter > rejects every later write at once, without touching the stream"
#[tokio::test]
async fn to_upload_writer_rejects_every_later_write_at_once_without_touching_the_stream() {
    let FakeUpload {
        upload,
        state,
        mut settle,
        done,
    } = create_fake_upload(true);
    let mut writer = to_upload_writer(upload);
    settle.reject("part upload failed");
    let _ = done.await; // upload.done.catch(() => undefined)
    let writes_before = state.write_calls(); // vi.spyOn(upload.stream, "write")

    let error = within_test_timeout(writer.write("x"))
        .await
        .expect_err("expected rejection");
    assert_message_contains(&error, "part upload failed");
    assert_eq!(state.write_calls(), writes_before);
}

/// Port of: packages/filesystem/__tests__/upload-writer.test.ts:81 "toUploadWriter > finish surfaces the upload failure"
#[tokio::test]
async fn to_upload_writer_finish_surfaces_the_upload_failure() {
    let FakeUpload {
        upload, mut settle, ..
    } = create_fake_upload(true);
    let writer = to_upload_writer(upload);
    settle.reject("complete failed");

    let error = within_test_timeout(writer.finish())
        .await
        .expect_err("expected rejection");
    assert_message_contains(&error, "complete failed");
}

/// Port of: packages/filesystem/__tests__/upload-writer.test.ts:89 "toUploadWriter > abort stops the stream and waits for the upload to settle, failed or not"
#[tokio::test]
async fn to_upload_writer_abort_stops_the_stream_and_waits_for_the_upload_to_settle_failed_or_not()
{
    let FakeUpload {
        upload,
        state,
        mut settle,
        ..
    } = create_fake_upload(true);
    let writer = to_upload_writer(upload);
    settle.reject("aborted");

    // resolves.toBeUndefined(): abort() returns `()` and must not hang.
    within_test_timeout(writer.abort()).await;
    assert!(state.destroyed());
}
