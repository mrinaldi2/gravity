//! H-167: one request's failure never ends the owner's connection. The
//! 0.17.0 home asked `projects_overview`; the redactor panicked on a `→` in
//! a decision's title, the panic ended the connection's task with the socket
//! still open, and the app waited forever on that request and every later
//! one: no projects, no bot opened.

mod common;

use common::tasks::project_with_bots;
use common::*;
use serde_json::json;

#[tokio::test]
async fn non_ascii_titles_on_the_home_and_the_connection_goes_on() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev"]).await;
    let project = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    bots[1]
        .call(
            "raise_decision",
            json!({"title": "Done → next: 日本語 😀 e\u{301}clair", "body": "Ship it?"}),
        )
        .await;
    let mut owner = WsClient::connect(&pair.d).await;
    let overview = owner.request(json!({"type": "projects_overview"})).await;
    assert_eq!(overview["type"], "projects_overview", "{overview}");
    let rows = owner
        .request(json!({"type": "attention_rows", "project_id": project}))
        .await;
    assert!(
        rows.to_string().contains("Done → next: 日本語 😀"),
        "{rows}"
    );
    // And the next request on the same connection answers.
    let listed = owner
        .request(json!({"type": "list_bots", "project_id": project}))
        .await;
    assert_eq!(listed["type"], "bots", "{listed}");
}
