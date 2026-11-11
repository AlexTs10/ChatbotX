//! Times the exact filtered cosine scan of `conversion/specs/P2.5.md` section 2.2 on one
//! file of 5,000 chunks (the `HEAVY_MAX_CHUNKS_PER_FILE` cap), in a migrated temp-file
//! database. A second file of 5,000 chunks is written interleaved with the first, so the
//! scan reads its rows by random access (the realistic case). Prints the query plan, the
//! first run and the median of 20 runs.
//! Run: `cargo run --release -p chatbotx-db --example vector_scan`

use std::time::{Duration, Instant};

use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;

const DIM: usize = 1536;
const CHUNKS: i64 = 5_000;
const RUNS: usize = 20;

const FILE_SCAN: &str = r#"SELECT "id", "content", "aiFileId", "chunkIndex",
       1 - vector_distance_cos("embedding", vector32(?1)) AS "similarity"
FROM "AIEmbedding" INDEXED BY "AIEmbedding_aiFileId_chunkIndex_idx"
WHERE "workspaceId" = ?2
  AND "aiFileId" IN (SELECT value FROM json_each(?3))
  AND "status" = 'success'
  AND "embedding" IS NOT NULL
ORDER BY vector_distance_cos("embedding", vector32(?1)) ASC NULLS LAST, "id"
LIMIT ?4"#;

fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6_364_136_223_846_793_005) | 1;
    (0..DIM)
        .map(|_| {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((x >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

fn blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn json(v: &[f32]) -> String {
    let items: Vec<String> = v.iter().map(|f| format!("{f}")).collect();
    format!("[{}]", items.join(","))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("chatbotx-db-vector-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    {
        let db = Db::open(dir.join("vector.db"))?;
        db.migrate().await?;
        let load = Instant::now();
        db.write(|tx| {
            tx.execute(r#"INSERT INTO "User" ("id", "email") VALUES (1, 'o@example.com')"#, [])?;
            tx.execute(r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (1, 'W', 1)"#, [])?;
            for file in [10, 11] {
                tx.execute(
                    r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "workspaceId")
                       VALUES (?1, 'f.txt', 'ai/f.txt', 10, 'text/plain', 1)"#,
                    params![file],
                )?;
            }
            let mut stmt = tx.prepare(
                r#"INSERT INTO "AIEmbedding" ("id", "content", "embedding", "status", "chunkIndex", "workspaceId", "aiFileId")
                   VALUES (?1, ?2, ?3, 'success', ?4, 1, ?5)"#,
            )?;
            let mut id = 0i64;
            for chunk in 0..CHUNKS {
                for file in [10, 11] {
                    id += 1;
                    let content = format!("chunk {chunk} of file {file}: {}", "lorem ipsum ".repeat(80));
                    stmt.execute(params![id, content, blob(&vector(id as u64)), chunk, file])?;
                }
            }
            Ok(())
        })
        .await?;
        println!(
            "loaded {} chunks ({} per file, 2 files interleaved) in {:?}",
            2 * CHUNKS,
            CHUNKS,
            load.elapsed()
        );

        let query = json(&vector(987_654_321));
        let plan = {
            let query = query.clone();
            db.read(move |conn| {
                let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {FILE_SCAN}"))?;
                let lines = stmt
                    .query_map(params![query, 1, "[10]", 10], |r| r.get::<_, String>(3))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(lines.join("\n"))
            })
            .await?
        };
        println!("plan:\n{plan}");

        let mut times: Vec<Duration> = Vec::with_capacity(RUNS);
        let mut top = Vec::new();
        for _ in 0..RUNS {
            let query = query.clone();
            let started = Instant::now();
            top = db
                .read(move |conn| {
                    let mut stmt = conn.prepare_cached(FILE_SCAN)?;
                    let rows = stmt
                        .query_map(params![query, 1, "[10]", 10], |r| {
                            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(4)?))
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await?;
            times.push(started.elapsed());
        }
        let first = times[0];
        times.sort();
        println!(
            "scan of {CHUNKS} chunks, LIMIT 10: first run {:?}, median {:?}, min {:?}, max {:?}",
            first,
            times[RUNS / 2],
            times[0],
            times[RUNS - 1]
        );
        println!("top 3 (id, similarity): {:?}", &top[..3.min(top.len())]);
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
