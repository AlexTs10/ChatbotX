//! Time source. Moved to `chatbotx_kv::clock` (P2.7 spec 2.1); re-exported
//! here so the jobs API stays the same. Production uses [`SystemClock`];
//! tests inject a manual clock (and `tokio::time::pause()` for sleeps).

pub use chatbotx_kv::clock::{Clock, SystemClock};
