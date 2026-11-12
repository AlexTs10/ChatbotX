//! Idle cost of the P2.7 stores (spec 2.12): starts every store with both
//! sweepers on a file database, writes a few entries with TTLs (so both
//! sweepers have a timer), then stays idle and reports the peak RSS
//! (`VmHWM`) and the context switches of all threads (wake-ups) per second.
//!
//! Linux only for the numbers (`/proc`). Usage:
//! `cargo run --release -p chatbotx-kv --example idle_kv -- [warmup_s] [window_s]`
//! (defaults 15 and 30).

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chatbotx_db::Db;
use chatbotx_kv::{CacheOpts, Kv, KvConfig, LockOpts, SystemClock};

/// Sum of voluntary + involuntary context switches of all live threads.
fn context_switches() -> Option<u64> {
    let mut total = 0;
    for task in std::fs::read_dir("/proc/self/task").ok()? {
        let status = std::fs::read_to_string(task.ok()?.path().join("status")).ok()?;
        for line in status.lines() {
            if let Some(rest) = line
                .strip_prefix("voluntary_ctxt_switches:")
                .or_else(|| line.strip_prefix("nonvoluntary_ctxt_switches:"))
            {
                total += rest.trim().parse::<u64>().ok()?;
            }
        }
    }
    Some(total)
}

fn status_field(name: &str) -> Option<String> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix(name).map(|rest| rest.trim().to_owned()))
}

fn thread_count() -> Option<usize> {
    Some(std::fs::read_dir("/proc/self/task").ok()?.count())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let warmup = args.next().and_then(|a| a.parse().ok()).unwrap_or(15u64);
    let window = args.next().and_then(|a| a.parse().ok()).unwrap_or(30u64);

    let dir = std::env::temp_dir().join(format!("chatbotx-kv-idle-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let db = Arc::new(Db::open(dir.join("kv.db"))?);
    db.migrate().await?;
    let kv = Kv::start(db, Arc::new(SystemClock), KvConfig::default());

    // One of each kind of entry, all with a TTL.
    kv.mem.put("marker:1", &true, Some(300))?;
    kv.mem.incr_with_window("rl:ws-1:0", 10);
    kv.cache
        .get_or_load("workspaces:1", CacheOpts::default(), || async {
            Ok::<_, Infallible>(Some("workspace".to_owned()))
        })
        .await?;
    kv.presence
        .heartbeat_many("workspace:presence:1", &["user-1"], 20_000);
    kv.durable.put("schedule:cursor", &42, Some(3600)).await?;
    kv.durable
        .reserve_time_window("comment-automation:pace:1", 1000.0)
        .await?;
    {
        let _held = kv
            .locks
            .acquire(
                "schedule:test",
                LockOpts {
                    ttl_secs: 10,
                    retry_secs: Some(0),
                },
            )
            .await?;
    }

    // Let the blocking-pool threads of the reader connections exit (10 s keep-alive).
    tokio::time::sleep(Duration::from_secs(warmup)).await;
    let threads = thread_count();
    let before = context_switches();
    let started = Instant::now();
    tokio::time::sleep(Duration::from_secs(window)).await;
    let elapsed = started.elapsed().as_secs_f64();
    let after = context_switches();

    println!("idle window: {elapsed:.1} s after {warmup} s warm-up");
    println!(
        "threads: {}",
        threads.map_or("n/a".to_owned(), |n| n.to_string())
    );
    match (before, after) {
        (Some(before), Some(after)) => {
            let switches = after.saturating_sub(before);
            println!(
                "context switches (all threads): {switches} = {:.3} wake-ups/s",
                switches as f64 / elapsed
            );
        }
        _ => println!("context switches: n/a (no /proc)"),
    }
    println!(
        "VmHWM: {}",
        status_field("VmHWM:").unwrap_or_else(|| "n/a".to_owned())
    );
    println!(
        "VmRSS: {}",
        status_field("VmRSS:").unwrap_or_else(|| "n/a".to_owned())
    );
    println!(
        "lock map entries: {}; estimated volatile bytes: {}",
        kv.locks.len(),
        kv.used_memory()
    );
    drop(kv);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
