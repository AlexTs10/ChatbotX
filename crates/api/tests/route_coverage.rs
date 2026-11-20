//! Q.P6-audit: the measured list of what still answers "not ported".
//!
//! Every public route of `PUBLIC_ROUTES` is called through the real router
//! (`public_api_router_with` + `LiveAuthDeps`) with a full, unrestricted
//! owner workspace token and a minimal input (path params `1`, body `{}`).
//! The 10 channel-token and mobile-user routes call their handler modules
//! directly. Every `/rpc` procedure of `RPC_PROCEDURES` and every server
//! action the SPA calls (`serverAction` / `serverFunction` / `serverQuery` in
//! `apps/web/src`) is called with the owner session. An answer whose body
//! says "not ported" puts the id on the list; the list must equal the
//! checked-in file under `tests/fixtures/`, so it only shrinks on purpose.
//! An action with its own server route (`/rpc/actions/<name>`) counts as
//! ported. A SPA stub that nothing imports is still listed (COVERAGE.md
//! marks it).
//! Each call gets a fresh database; 8 run at once. A panic, a 30 s
//! time-out or a 401 / 403 is not listed (printed).

mod rpc_common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, header};
use axum::response::{IntoResponse, Response};
use chatbotx_api::auth::{LiveAuthDeps, MobileUser, hash_token};
use chatbotx_api::handlers::channels::{self, ChannelCall};
use chatbotx_api::handlers::{Call, me};
use chatbotx_api::routes::{PUBLIC_ROUTES, PublicRoute, RouteAuth, public_api_router_with};
use chatbotx_api::rpc::RPC_PROCEDURES;
use chatbotx_db::rusqlite::params;
use chatbotx_kv::{Kv, KvConfig, SystemClock};
use rpc_common::{Fixture, OWNER, OWNER_TOKEN, WS, fixture};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const TOKEN: &str = "cbx_ws_route_coverage";
const TIMEOUT: Duration = Duration::from_secs(30);

/// One call's outcome.
enum Outcome {
    NotPorted,
    Ported,
    /// 401 / 403: the call did not reach a handler (checked by hand).
    Denied,
    Panicked,
    TimedOut,
}

async fn body_text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn outcome_of(text: &str) -> Outcome {
    if text.to_ascii_lowercase().contains("not ported") {
        Outcome::NotPorted
    } else if ["UNAUTHORIZED", "FORBIDDEN", "INVALID_CHATBOT_TOKEN"]
        .iter()
        .any(|code| text.contains(&format!("\"code\":\"{code}\"")))
    {
        Outcome::Denied
    } else {
        Outcome::Ported
    }
}

/// Runs `call` in its own task (a panic or a hang does not stop the sweep).
async fn measure<F>(call: F) -> Outcome
where
    F: Future<Output = String> + Send + 'static,
{
    match tokio::time::timeout(TIMEOUT, tokio::spawn(call)).await {
        Ok(Ok(text)) => outcome_of(&text),
        Ok(Err(_)) => Outcome::Panicked,
        Err(_) => Outcome::TimedOut,
    }
}

/// The ids, sorted, of each outcome.
#[derive(Default)]
struct Tally {
    not_ported: Vec<String>,
    ported: usize,
    denied: Vec<String>,
    panicked: Vec<String>,
    timed_out: Vec<String>,
}

impl Tally {
    fn add(&mut self, id: String, outcome: Outcome) {
        match outcome {
            Outcome::NotPorted => self.not_ported.push(id),
            Outcome::Ported => self.ported += 1,
            Outcome::Denied => self.denied.push(id),
            Outcome::Panicked => self.panicked.push(id),
            Outcome::TimedOut => self.timed_out.push(id),
        }
    }

    /// Prints the result and compares it with the checked-in list.
    fn check(mut self, what: &str, list: &str) {
        self.not_ported.sort();
        self.not_ported.dedup();
        println!(
            "{what}: {} not ported, {} ported, {} denied {:?}, {} panicked {:?}, {} timed out {:?}",
            self.not_ported.len(),
            self.ported,
            self.denied.len(),
            self.denied,
            self.panicked.len(),
            self.panicked,
            self.timed_out.len(),
            self.timed_out,
        );
        println!("---- {what} not ported ----");
        for id in &self.not_ported {
            println!("{id}");
        }
        println!("---- end ----");
        let expected: Vec<&str> = list
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        assert_eq!(
            self.not_ported, expected,
            "{what}: the not-ported list changed; update tests/fixtures (printed above)"
        );
    }
}

/// The owner's full, unrestricted workspace token.
async fn seed_token(fx: &Fixture) {
    let hash = hash_token(TOKEN);
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "WorkspaceApiToken" ("id", "workspaceId", "name", "permission", "tokenHash")
                   VALUES (9001, ?1, 'coverage', 'full', ?2)"#,
                params![WS, hash],
            )?;
            Ok(())
        })
        .await
        .expect("seed token");
}

/// A fresh router (fresh rate counters, so 800 calls are not rate limited).
fn public_router(fx: &Fixture) -> axum::Router {
    let db = Arc::clone(&fx.api.db);
    let deps = LiveAuthDeps {
        kv: Kv::start(Arc::clone(&db), Arc::new(SystemClock), KvConfig::default()),
        db,
        clock: Arc::new(SystemClock),
        auth_base_url: "http://app/api/auth".to_string(),
        mcp_resource_url: "http://app/mcp".to_string(),
        is_cloud: false,
        mobile_audience: None,
    };
    public_api_router_with(Arc::new(deps), Arc::clone(&fx.api))
}

fn concrete_path(template: &str) -> String {
    template
        .split('/')
        .map(|segment| {
            if segment.starts_with('{') {
                "1"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// One call, on its own fresh database (a delete or a workspace deletion
/// must not change the next call's answer): the answer body.
type Probe = std::pin::Pin<Box<dyn Future<Output = String> + Send>>;

/// Calls at once (each has its own in-memory database).
// ponytail: one migrated database per call (~11 min on the Dell for 1,765
// calls); copy a migrated template file per call if the run time matters.
const PARALLEL: usize = 8;

/// Runs every probe, [`PARALLEL`] at a time.
async fn measure_all(probes: Vec<(String, Probe)>) -> Tally {
    let mut tally = Tally::default();
    let mut probes = probes.into_iter();
    let mut running = tokio::task::JoinSet::new();
    loop {
        while running.len() < PARALLEL {
            let Some((id, probe)) = probes.next() else {
                break;
            };
            running.spawn(async move { (id, measure(probe).await) });
        }
        let Some(joined) = running.join_next().await else {
            break;
        };
        let (id, outcome) = joined.expect("measure catches panics");
        tally.add(id, outcome);
    }
    tally
}

fn public_probe(route: &'static PublicRoute) -> Probe {
    Box::pin(async move {
        let fx = fixture().await;
        seed_token(&fx).await;
        let api = Arc::clone(&fx.api);
        let id = route.id;
        match route.auth {
            RouteAuth::WorkspaceToken(_) => {
                let has_body = !matches!(route.method, "GET" | "DELETE");
                let request = Request::builder()
                    .method(route.method)
                    .uri(format!("/api{}", concrete_path(route.path)))
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-forwarded-for", "203.0.113.9")
                    .body(if has_body {
                        Body::from("{}")
                    } else {
                        Body::empty()
                    })
                    .expect("request");
                let response = public_router(&fx)
                    .oneshot(request)
                    .await
                    .expect("infallible");
                body_text(response).await
            }
            RouteAuth::ChannelToken => {
                let call = ChannelCall {
                    inbox_id: 1,
                    workspace_id: WS,
                };
                match channels::handle(&api, id, call, b"{}").await {
                    Some(reply) => {
                        body_text(reply.unwrap_or_else(IntoResponse::into_response)).await
                    }
                    None => "not ported".to_string(),
                }
            }
            RouteAuth::MobileUser | RouteAuth::MobileUserPasswordChange => {
                let user = MobileUser {
                    id: OWNER,
                    name: None,
                    email: "owner@rpc.test".to_string(),
                    image: None,
                    must_change_password: false,
                    device_id: None,
                };
                let call = Call {
                    workspace_id: 0,
                    params: &[],
                    query: None,
                    body: b"{}",
                };
                let reply = me::handle(&api, id, call, &user, &HeaderMap::new()).await;
                body_text(reply.unwrap_or_else(IntoResponse::into_response)).await
            }
        }
    })
}

#[tokio::test]
async fn public_routes_not_ported() {
    let probes = PUBLIC_ROUTES
        .iter()
        .map(|route| (route.id.to_string(), public_probe(route)))
        .collect();
    measure_all(probes).await.check(
        "public routes",
        include_str!("fixtures/not_ported_routes.txt"),
    );
}

/// The text of the files with one of `exts` under `dir` (relative to this crate).
fn source_files(dir: &str, exts: &[&str]) -> Vec<String> {
    let mut files = Vec::new();
    let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir)];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("source dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| exts.iter().any(|e| ext == *e))
            {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                files.push(text);
            }
        }
    }
    files
}

/// The actions the server serves on their own route
/// (`.route("/rpc/actions/<name>", ..)` in `crates/api` / `crates/server`),
/// outside the `/rpc` router this test calls.
fn own_route_actions() -> Vec<String> {
    let mut names = Vec::new();
    for dir in ["src", "../server/src"] {
        for text in source_files(dir, &["rs"]) {
            for (at, _) in text.match_indices("\"/rpc/actions/") {
                let rest = &text[at + "\"/rpc/actions/".len()..];
                if let Some(end) = rest.find('"') {
                    names.push(rest[..end].to_string());
                }
            }
        }
    }
    names
}

/// `serverAction` / `serverFunction` / `serverQuery` names in `apps/web/src`
/// and the input the SPA sends (`lib/server-action.ts`).
fn spa_actions() -> Vec<(String, Value)> {
    let ws = WS.to_string();
    let mut actions = Vec::new();
    for text in source_files("../../apps/web/src", &["ts", "tsx"]) {
        for kind in ["serverAction", "serverFunction", "serverQuery"] {
            for (at, _) in text.match_indices(kind) {
                let Some((name, rest)) = call_name(&text, at, kind) else {
                    continue;
                };
                let args_end = rest.find(')').unwrap_or(rest.len());
                let input = match kind {
                    "serverQuery" => json!({ "args": [], "workspaceId": ws }),
                    "serverAction" if rest[..args_end].contains("workspace: true") => {
                        json!({ "args": [ws], "workspaceId": ws })
                    }
                    _ => json!({ "args": [] }),
                };
                actions.push((name, input));
            }
        }
    }
    actions.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.to_string().cmp(&b.1.to_string()))
    });
    actions.dedup();
    actions
}

/// The quoted first argument of a `kind<...>("name"` call at `at`, and the
/// text after it.
fn call_name<'a>(text: &'a str, at: usize, kind: &str) -> Option<(String, &'a str)> {
    if text[..at]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    let bytes = text.as_bytes();
    let mut index = at + kind.len();
    if bytes.get(index) == Some(&b'<') {
        let mut depth = 0_i32;
        while index < bytes.len() {
            match bytes[index] {
                b'<' => depth += 1,
                b'>' if bytes[index - 1] != b'=' => {
                    depth -= 1;
                    if depth == 0 {
                        index += 1;
                        break;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }
    if bytes.get(index) != Some(&b'(') {
        return None;
    }
    let rest = text[index + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some((rest[..end].to_string(), &rest[end + 1..]))
}

/// One `/rpc` call with the owner session.
fn rpc_probe(method: &'static str, url: String, input: Value) -> Probe {
    Box::pin(async move {
        let fx = fixture().await;
        let (_, body) = rpc_common::call(&fx, method, &url, Some(OWNER_TOKEN), input).await;
        body.to_string()
    })
}

#[tokio::test]
async fn rpc_procedures_and_actions_not_ported() {
    let mut probes = Vec::new();
    for procedure in RPC_PROCEDURES {
        let mut input = json!({});
        if let Some(path) = procedure.workspace
            && let Some((last, parents)) = path.split_last()
        {
            let mut at = &mut input;
            for key in parents {
                at = at
                    .as_object_mut()
                    .expect("object")
                    .entry(*key)
                    .or_insert_with(|| json!({}));
            }
            at[*last] = json!(WS.to_string());
        }
        let method = procedure.method.unwrap_or("POST");
        let url = format!("/rpc/{}", procedure.path.replace('.', "/"));
        probes.push((procedure.path.to_string(), rpc_probe(method, url, input)));
    }
    let own_route = own_route_actions();
    let mut own_route_count = 0;
    for (name, input) in spa_actions() {
        if own_route.contains(&name) {
            own_route_count += 1;
            continue;
        }
        let url = format!("/rpc/actions/{name}");
        probes.push((format!("actions.{name}"), rpc_probe("POST", url, input)));
    }
    let mut tally = measure_all(probes).await;
    tally.ported += own_route_count;
    tally.check("rpc", include_str!("fixtures/not_ported_rpc.txt"));
}
