//! `packages/utils/src/image-optimize-settings.ts`: the re-encode policy
//! (`STORED_IMAGE_*`) and the per-process encode slots.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::oneshot;

use crate::env::Env;

const MEGAPIXEL: u64 = 1_000_000;
const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;
const MAX_WAITING_PER_SLOT: usize = 4;

/// `ProfileSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSettings {
    pub max_dimension: u32,
    pub quality: u8,
    pub min_saving_percent: u32,
}

/// `inline.maxPixels`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineMaxPixels {
    pub media: u64,
    pub avatar: u64,
}

/// `inline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineSettings {
    pub max_bytes: u64,
    pub max_pixels: InlineMaxPixels,
    pub max_concurrent_encodes: usize,
}

/// `job`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobSettings {
    pub min_source_bytes: u64,
    pub max_source_bytes: u64,
}

/// `ImageOptimizationSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOptimizationSettings {
    pub media: ProfileSettings,
    pub avatar: ProfileSettings,
    pub max_input_pixels: u64,
    pub encode_timeout_seconds: u64,
    pub inline: InlineSettings,
    pub job: JobSettings,
}

/// A rejected `STORED_IMAGE_*` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidImageSetting(pub String);

impl std::fmt::Display for InvalidImageSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid environment variables: {}", self.0)
    }
}

impl std::error::Error for InvalidImageSetting {}

/// `z.coerce.number()` with bounds; `integer` adds `.int()`.
fn number(
    env: &dyn Env,
    name: &str,
    default: f64,
    min: f64,
    max: f64,
    integer: bool,
    exclusive_min: bool,
) -> Result<f64, InvalidImageSetting> {
    let Some(raw) = env.var(name) else {
        return Ok(default);
    };
    let value = crate::js::to_number(&raw);
    let ok = value.is_finite()
        && (!integer || value.fract() == 0.0)
        && if exclusive_min {
            value > min
        } else {
            value >= min
        }
        && value <= max;
    if ok {
        Ok(value)
    } else {
        Err(InvalidImageSetting(name.to_owned()))
    }
}

/// The validated `imageOptimizationKeys()`: every variable is checked, so
/// one bad value makes the whole read fail (as in t3-env).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Keys {
    optimization_on: bool,
    settings: ImageOptimizationSettings,
}

fn keys(env: &dyn Env) -> Result<Keys, InvalidImageSetting> {
    let optimization_on = match env.var("STORED_IMAGE_OPTIMIZATION").as_deref() {
        None | Some("on") => true,
        Some("off") => false,
        Some(_) => return Err(InvalidImageSetting("STORED_IMAGE_OPTIMIZATION".to_owned())),
    };
    let positive =
        |name: &str, default: f64, max: f64| number(env, name, default, 1.0, max, true, false);
    let quality = |name: &str| number(env, name, 80.0, 1.0, 100.0, true, false);
    let saving = |name: &str, default: f64| number(env, name, default, 0.0, 90.0, true, false);
    let settings = ImageOptimizationSettings {
        media: ProfileSettings {
            max_dimension: positive("STORED_IMAGE_MEDIA_MAX_DIMENSION", 1600.0, 8192.0)? as u32,
            quality: quality("STORED_IMAGE_MEDIA_QUALITY")? as u8,
            min_saving_percent: saving("STORED_IMAGE_MEDIA_MIN_SAVING_PERCENT", 20.0)? as u32,
        },
        avatar: ProfileSettings {
            max_dimension: positive("STORED_IMAGE_AVATAR_MAX_DIMENSION", 256.0, 2048.0)? as u32,
            quality: quality("STORED_IMAGE_AVATAR_QUALITY")? as u8,
            min_saving_percent: saving("STORED_IMAGE_AVATAR_MIN_SAVING_PERCENT", 10.0)? as u32,
        },
        max_input_pixels: positive("STORED_IMAGE_MAX_INPUT_MEGAPIXELS", 16.0, 100.0)? as u64
            * MEGAPIXEL,
        encode_timeout_seconds: positive("STORED_IMAGE_ENCODE_TIMEOUT_SECONDS", 2.0, 60.0)? as u64,
        inline: InlineSettings {
            max_bytes: crate::js::math_round(
                number(
                    env,
                    "STORED_IMAGE_INLINE_MAX_MB",
                    2.0,
                    0.0,
                    50.0,
                    false,
                    true,
                )? * MIB as f64,
            ) as u64,
            max_pixels: InlineMaxPixels {
                media: positive("STORED_IMAGE_INLINE_MAX_MEGAPIXELS", 8.0, 100.0)? as u64
                    * MEGAPIXEL,
                avatar: positive("STORED_IMAGE_INLINE_AVATAR_MAX_MEGAPIXELS", 4.0, 100.0)? as u64
                    * MEGAPIXEL,
            },
            max_concurrent_encodes: number(
                env,
                "STORED_IMAGE_INLINE_CONCURRENCY",
                2.0,
                0.0,
                32.0,
                true,
                false,
            )? as usize,
        },
        job: JobSettings {
            min_source_bytes: number(
                env,
                "STORED_IMAGE_JOB_MIN_SOURCE_KB",
                32.0,
                0.0,
                10_240.0,
                true,
                false,
            )? as u64
                * KIB,
            max_source_bytes: positive("STORED_IMAGE_JOB_MAX_SOURCE_MB", 10.0, 100.0)? as u64 * MIB,
        },
    };
    Ok(Keys {
        optimization_on,
        settings,
    })
}

/// `getImageOptimizationSettings`: read per call; `Err` for a bad value.
pub fn get_image_optimization_settings(
    env: &dyn Env,
) -> Result<ImageOptimizationSettings, InvalidImageSetting> {
    keys(env).map(|k| k.settings)
}

/// `isStoredImageOptimizationEnabled`: `STORED_IMAGE_OPTIMIZATION` is `on`
/// (default); a misconfigured `STORED_IMAGE_*` value turns it off, logged.
pub fn is_stored_image_optimization_enabled(env: &dyn Env) -> bool {
    match keys(env) {
        Ok(k) => k.optimization_on,
        Err(err) => {
            tracing::error!(module = "image-optimize", err = %err, "Invalid STORED_IMAGE_* setting; image optimization is off");
            false
        }
    }
}

struct SlotState {
    active: usize,
    waiting: VecDeque<oneshot::Sender<()>>,
}

/// The per-process encode slots (`runInEncodeSlot`'s module state). Use
/// [`EncodeSlots::global`] in the app; tests make their own.
pub struct EncodeSlots {
    state: Mutex<SlotState>,
}

impl std::fmt::Debug for EncodeSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodeSlots").finish_non_exhaustive()
    }
}

impl Default for EncodeSlots {
    fn default() -> EncodeSlots {
        EncodeSlots::new()
    }
}

struct SlotGuard<'a> {
    slots: &'a EncodeSlots,
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.slots.state.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(next) = state.waiting.pop_front() {
            if next.send(()).is_ok() {
                return;
            }
        }
        state.active = state.active.saturating_sub(1);
    }
}

impl EncodeSlots {
    /// Empty slots.
    pub fn new() -> EncodeSlots {
        EncodeSlots {
            state: Mutex::new(SlotState {
                active: 0,
                waiting: VecDeque::new(),
            }),
        }
    }

    /// The process-wide slots.
    pub fn global() -> Arc<EncodeSlots> {
        static GLOBAL: OnceLock<Arc<EncodeSlots>> = OnceLock::new();
        GLOBAL.get_or_init(|| Arc::new(EncodeSlots::new())).clone()
    }

    /// `runInEncodeSlot`: runs `work` in one of `limit` slots. A free slot is
    /// taken in the caller's turn; with `wait: false` (or a full queue of
    /// `limit × 4` waiters) a busy pool gives `None` at once.
    pub fn run_in_encode_slot<'a, T, E, W, Fut>(
        &'a self,
        limit: usize,
        wait: bool,
        work: W,
    ) -> impl Future<Output = Result<Option<T>, E>> + 'a
    where
        W: FnOnce() -> Fut + 'a,
        Fut: Future<Output = Result<T, E>> + 'a,
        T: 'a,
        E: 'a,
    {
        enum Start<W> {
            Now(W),
            Wait(W, oneshot::Receiver<()>),
            Refused,
        }
        let start = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.active < limit {
                state.active += 1;
                Start::Now(work)
            } else if !wait || state.waiting.len() >= limit * MAX_WAITING_PER_SLOT {
                Start::Refused
            } else {
                let (sender, receiver) = oneshot::channel();
                state.waiting.push_back(sender);
                Start::Wait(work, receiver)
            }
        };
        async move {
            let work = match start {
                Start::Refused => return Ok(None),
                Start::Now(work) => work,
                Start::Wait(work, receiver) => {
                    if receiver.await.is_err() {
                        return Ok(None);
                    }
                    work
                }
            };
            let _guard = SlotGuard { slots: self };
            work().await.map(Some)
        }
    }
}
