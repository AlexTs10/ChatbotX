//! Port of `apps/worker/__tests__/sequence-worker-logging.test.ts`
//! (spec 5.3 "sequence-worker-logging"). TS-only proposal: the sequence
//! sources log through `tracing`, never through print macros.

use std::fs;
use std::path::Path;

/// Port of: apps/worker/__tests__/sequence-worker-logging.test.ts:14 "sequence worker logging > entrypoints use structured logger instead of console calls"
#[test]
fn entrypoints_use_structured_logger_instead_of_console_calls() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sequence");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("read src/sequence") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read source");
        for pattern in ["println!", "eprintln!", "print!(", "eprint!(", "dbg!("] {
            assert!(
                !source.contains(pattern),
                "{} uses {pattern}",
                path.display()
            );
        }
        checked += 1;
    }
    // The producer (worker-producer.ts) and the reconcile entry (worker.ts).
    assert!(dir.join("producer.rs").exists());
    assert!(dir.join("reconcile.rs").exists());
    assert!(checked >= 2);
}
