//! Port of the `listActivity` cases of
//! `apps/builder/__tests__/pipelines-activity-filter.test.ts` (the
//! `listItems` case is in `pipelines_query_public_api.rs`). Adapted:
//! `listActivityPublicRequest.parse(...).groups` is
//! `handlers::pipelines::activity_groups_query` (path params + raw query ->
//! the groups, `None` for every kind, or the 422 error).

use chatbotx_api::handlers::pipelines::activity_groups_query;
use chatbotx_api::orpc::OrpcError;

fn groups(query: &str) -> Result<Option<Vec<String>>, OrpcError> {
    let params = [("id", "1".to_string()), ("itemId", "2".to_string())];
    activity_groups_query(&params, Some(query))
}

#[test]
fn reads_a_comma_separated_query_list() {
    assert_eq!(
        groups("groups=stage%2C%20comments").expect("parsed"),
        Some(vec!["stage".to_string(), "comments".to_string()])
    );
}

#[test]
fn an_empty_or_missing_list_means_every_kind() {
    assert_eq!(groups("").expect("parsed"), None);
    assert_eq!(groups("groups=%20%2C%20").expect("parsed"), None);
}

#[test]
fn rejects_an_unknown_group() {
    assert!(groups("groups=stage%2Cbogus").is_err());
}
