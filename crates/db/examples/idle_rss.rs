//! Opens a migrated temp-file database (writer thread + 3 readers), runs one read query
//! and one write, then prints the resident set size (`VmRSS` / `VmHWM` on Linux).
//! Run: `cargo build --release -p chatbotx-db --example idle_rss && /usr/bin/time -v target/release/examples/idle_rss`

use chatbotx_db::Db;

fn proc_status(field: &str) -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .map(|l| l.trim().to_owned())
        })
        .unwrap_or_else(|| format!("{field} n/a"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("chatbotx-db-rss-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    {
        let db = Db::open(dir.join("rss.db"))?;
        let report = db.migrate().await?;
        let tenants: i64 = db
            .read(|conn| Ok(conn.query_row(r#"SELECT count(*) FROM "Tenant""#, [], |r| r.get(0))?))
            .await?;
        db.write(|tx| {
            tx.execute(r#"INSERT INTO "Verification" ("id", "identifier", "value", "expiresAt") VALUES ('v', 'i', 'x', 0)"#, [])?;
            Ok(())
        })
        .await?;
        println!("applied {:?}, tenants {tenants}", report.applied);
        println!(
            "after open + migrate + 1 read + 1 write: {} | {}",
            proc_status("VmRSS"),
            proc_status("VmHWM")
        );
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
