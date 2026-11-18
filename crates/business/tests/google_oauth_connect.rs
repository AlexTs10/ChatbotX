//! Q.D9: the Google integration connect session is bound to the user who
//! started it (the original's unsigned JSON state was not): a callback
//! for another user, a signed-out browser or a non-member is refused and
//! leaves the session pending.

mod common;

use chatbotx_business::appointment::external::ExternalCalendars;
use chatbotx_business::connect_session::{self, NewSession};
use chatbotx_business::google_oauth;
use common::{WS, two_workspaces};
use serde_json::json;

#[tokio::test]
async fn callback_is_refused_for_another_user() {
    let db = two_workspaces().await;
    let session = connect_session::create(
        &db,
        NewSession {
            workspace_id: WS,
            provider: "googleCalendar".into(),
            purpose: "connect".into(),
            actor_user_id: Some(1001),
            ..NewSession::default()
        },
        |_, _| json!({ "type": "open_url", "url": "https://accounts.google.com/" }),
    )
    .await
    .expect("session");
    let google = ExternalCalendars::default();
    for user in [Some(1002), None] {
        let error = google_oauth::complete(&db, &google, "https://app.test", &session, user, "c")
            .await
            .expect_err("refused");
        assert_eq!(error.code(), Some("connectionStateMismatch"));
    }
    // 1001 owns WS but has no member row: not a member any more.
    let error = google_oauth::complete(&db, &google, "https://app.test", &session, Some(1001), "c")
        .await
        .expect_err("refused");
    assert_eq!(error.code(), Some("connectionStateMismatch"));
    let row = connect_session::find_for_workspace(&db, session.id, WS)
        .await
        .expect("read")
        .expect("row");
    assert_eq!(row.status, "pending");
}
