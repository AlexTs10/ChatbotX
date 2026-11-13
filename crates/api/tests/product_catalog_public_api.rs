//! Ports of `apps/builder/__tests__/product-categories-public-api.test.ts`,
//! `product-categories-public-scope.test.ts`, `product-vendors-public-api.test.ts`,
//! `products-public-api.test.ts` (the catalog routes) and
//! `products-public-scope.test.ts`. Adapted: the captured oRPC handlers become
//! `handlers::dispatch` calls over a real migrated database; "the service was
//! called with" becomes the stored rows / the response. The scope tests check
//! the route table (the auth middleware itself is P3.5).

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id, scopes_of_module};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn post(f: &Fixture, route: &str, ws: i64, body: Value) -> Value {
    let (status, out) = call(f, route, ws, &[], None, Some(body)).await;
    assert_eq!(status, 200, "{route}: {out}");
    out
}

fn id_of(value: &Value) -> String {
    value["id"].as_str().expect("id").to_string()
}

async fn new_category(f: &Fixture, ws: i64, name: &str, parent: Option<&str>) -> String {
    let mut body = json!({ "name": name });
    if let Some(parent) = parent {
        body["parentId"] = json!(parent);
    }
    id_of(&post(f, "productCategories.create", ws, body).await)
}

async fn new_product(f: &Fixture, ws: i64, body: Value) -> Value {
    let mut full = json!({ "name": "Product", "price": 10 });
    for (key, value) in body.as_object().expect("body").clone() {
        full[key] = value;
    }
    post(f, "products.create", ws, full).await
}

// --- product categories -----------------------------------------------------

#[tokio::test]
async fn registers_the_product_categories_public_router_under_the_ecommerce_scope() {
    assert_eq!(
        scopes_of_module("features/product-categories/api/public"),
        vec!["ecommerce"]
    );
}

#[tokio::test]
async fn product_categories_public_scope_a_contacts_scoped_token_is_denied_the_real_get_v1_product_categories_route_with_forbidden()
 {
    let route = route_by_id("productCategories.list").expect("route");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["ecommerce"]));
}

#[tokio::test]
async fn product_categories_public_scope_null_scopes_unrestricted_passes_the_real_get_v1_product_categories_route()
 {
    let f = fixture().await;
    let (status, body) = call(&f, "productCategories.list", WS, &[], None, None).await;
    assert_eq!((status, body), (200, json!({ "data": [] })));
}

#[tokio::test]
async fn product_categories_public_api_lists_categories_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let own = new_category(&f, WS, "Own", None).await;
    new_category(&f, OTHER_WS, "Other", None).await;
    let (status, body) = call(&f, "productCategories.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "data": [{ "id": own, "parentId": null, "name": "Own", "imageUrl": null, "isVisible": true, "rank": 10, "productCount": 0 }] })
    );
}

#[tokio::test]
async fn product_categories_public_api_passes_the_parent_and_name_filters_to_the_service() {
    let f = fixture().await;
    let parent = new_category(&f, WS, "Parent", None).await;
    let shirts = new_category(&f, WS, "Shirts", Some(&parent)).await;
    new_category(&f, WS, "Shoes", Some(&parent)).await;
    new_category(&f, WS, "Shirts top", None).await;
    let query = format!("parentId={parent}&name=shi");
    let (status, body) = call(&f, "productCategories.list", WS, &[], Some(&query), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["data"]
            .as_array()
            .map(|d| d.iter().map(id_of).collect::<Vec<_>>()),
        Some(vec![shirts])
    );
}

#[tokio::test]
async fn product_categories_public_api_creates_a_top_level_category_when_parent_id_is_omitted() {
    let f = fixture().await;
    let created = post(&f, "productCategories.create", WS, json!({ "name": "Top" })).await;
    assert_eq!(
        (&created["parentId"], &created["rank"]),
        (&Value::Null, &json!(10))
    );
    assert!(created.get("productCount").is_none());
}

#[tokio::test]
async fn product_categories_public_api_passes_the_image_and_visibility_through() {
    let f = fixture().await;
    let created = post(
        &f,
        "productCategories.create",
        WS,
        json!({ "name": "Img", "imageUrl": "https://example.com/c.png", "isVisible": false }),
    )
    .await;
    assert_eq!(
        (&created["imageUrl"], &created["isVisible"]),
        (&json!("https://example.com/c.png"), &json!(false))
    );
}

#[tokio::test]
async fn product_categories_public_api_updates_a_category_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let foreign = new_category(&f, OTHER_WS, "Foreign", None).await;
    let (status, body) = call(
        &f,
        "productCategories.update",
        WS,
        &[("id", &foreign)],
        None,
        Some(json!({ "name": "Mine" })),
    )
    .await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")), "{body}");
}

#[tokio::test]
async fn product_categories_public_api_omits_parent_id_entirely_when_the_caller_did_not_send_it() {
    let f = fixture().await;
    let parent = new_category(&f, WS, "Parent", None).await;
    let child = new_category(&f, WS, "Child", Some(&parent)).await;
    let (status, body) = call(
        &f,
        "productCategories.update",
        WS,
        &[("id", &child)],
        None,
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (&body["name"], &body["parentId"]),
        (&json!("Renamed"), &json!(parent))
    );
}

#[tokio::test]
async fn product_categories_public_api_files_the_category_under_a_parent_when_parent_id_is_an_id() {
    let f = fixture().await;
    let parent = new_category(&f, WS, "Parent", None).await;
    let other = new_category(&f, WS, "Other", None).await;
    let (status, body) = call(
        &f,
        "productCategories.update",
        WS,
        &[("id", &other)],
        None,
        Some(json!({ "name": "Other", "parentId": parent })),
    )
    .await;
    assert_eq!((status, &body["parentId"]), (200, &json!(parent)), "{body}");
}

#[tokio::test]
async fn product_categories_public_api_updates_the_image_rank_and_visibility_it_is_given() {
    let f = fixture().await;
    let id = new_category(&f, WS, "Cat", None).await;
    let (status, body) = call(
        &f,
        "productCategories.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Cat", "imageUrl": "https://example.com/i.png", "rank": 3, "isVisible": false })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (&body["imageUrl"], &body["rank"], &body["isVisible"]),
        (
            &json!("https://example.com/i.png"),
            &json!(3),
            &json!(false)
        )
    );
}

#[tokio::test]
async fn product_categories_public_api_changes_only_the_visibility() {
    let f = fixture().await;
    let created = post(
        &f,
        "productCategories.create",
        WS,
        json!({ "name": "Vis", "rank": 4 }),
    )
    .await;
    let id = id_of(&created);
    let (status, body) = call(
        &f,
        "productCategories.setVisible",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "isVisible": false })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let mut expected = created.clone();
    expected["isVisible"] = json!(false);
    assert_eq!(body, expected);
}

#[tokio::test]
async fn product_categories_public_api_deletes_a_category_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let foreign = new_category(&f, OTHER_WS, "Foreign", None).await;
    let (status, _) = call(
        &f,
        "productCategories.delete",
        WS,
        &[("id", &foreign)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let own = new_category(&f, WS, "Own", None).await;
    let (status, body) = call(
        &f,
        "productCategories.delete",
        WS,
        &[("id", &own)],
        None,
        None,
    )
    .await;
    assert_eq!((status, body), (204, Value::Null));
}

// --- product vendors --------------------------------------------------------

#[tokio::test]
async fn registers_the_product_vendors_public_router_under_the_ecommerce_scope() {
    assert_eq!(
        scopes_of_module("features/product-vendors/api/public"),
        vec!["ecommerce"]
    );
}

#[tokio::test]
async fn product_vendors_public_api_get_v1_product_vendors_passes_the_name_filter_and_paging() {
    let f = fixture().await;
    for name in ["Acme 1", "Acme 2", "Acme 3", "Globex"] {
        post(&f, "productVendors.create", WS, json!({ "name": name })).await;
    }
    let (status, body) = call(
        &f,
        "productVendors.list",
        WS,
        &[],
        Some("name=acme&page=2&perPage=2"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let names: Vec<&str> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|v| v["name"].as_str().expect("name"))
        .collect();
    assert_eq!((names, &body["pageCount"]), (vec!["Acme 3"], &json!(2)));
}

#[tokio::test]
async fn product_vendors_public_api_get_v1_product_vendors_without_paging_lists_every_vendor() {
    let f = fixture().await;
    for index in 0..3 {
        post(
            &f,
            "productVendors.create",
            WS,
            json!({ "name": format!("Vendor {index}") }),
        )
        .await;
    }
    let (status, body) = call(&f, "productVendors.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["data"].as_array().map(Vec::len), &body["pageCount"]),
        (Some(3), &json!(1))
    );
}

#[tokio::test]
async fn product_vendors_public_api_post_v1_product_vendors_creates_with_the_vendor_s_rules() {
    let f = fixture().await;
    let created = post(
        &f,
        "productVendors.create",
        WS,
        json!({ "name": "Acme", "discountPercent": 20, "minimumOrderValue": 50 }),
    )
    .await;
    assert_eq!(
        (
            &created["name"],
            &created["discountPercent"],
            &created["minimumOrderValue"]
        ),
        (&json!("Acme"), &json!(20), &json!(50))
    );
}

#[tokio::test]
async fn product_vendors_public_api_patch_v1_product_vendors_id_can_switch_a_vendor_off_alone() {
    let f = fixture().await;
    let created = post(
        &f,
        "productVendors.create",
        WS,
        json!({ "name": "Acme", "phone": "1" }),
    )
    .await;
    let id = id_of(&created);
    let (status, body) = call(
        &f,
        "productVendors.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "isActive": false })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let mut expected = created.clone();
    expected["isActive"] = json!(false);
    assert_eq!(body, expected);
}

#[tokio::test]
async fn product_vendors_public_api_delete_v1_product_vendors_id_deletes_in_the_token_s_workspace()
{
    let f = fixture().await;
    let foreign = id_of(
        &post(
            &f,
            "productVendors.create",
            OTHER_WS,
            json!({ "name": "Foreign" }),
        )
        .await,
    );
    let (status, _) = call(
        &f,
        "productVendors.delete",
        WS,
        &[("id", &foreign)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let own = id_of(&post(&f, "productVendors.create", WS, json!({ "name": "Own" })).await);
    let (status, _) = call(&f, "productVendors.delete", WS, &[("id", &own)], None, None).await;
    assert_eq!(status, 204);
}

// --- products ----------------------------------------------------------------

#[tokio::test]
async fn registers_the_products_public_router_under_the_ecommerce_scope() {
    assert_eq!(
        scopes_of_module("features/products/api/public"),
        vec!["ecommerce"]
    );
}

#[tokio::test]
async fn products_public_scope_a_contacts_scoped_token_is_denied_the_real_get_v1_products_route_with_forbidden()
 {
    let route = route_by_id("products.list").expect("route");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["ecommerce"]));
}

#[tokio::test]
async fn products_public_scope_null_scopes_unrestricted_passes_the_real_get_v1_products_route() {
    let f = fixture().await;
    let (status, body) = call(&f, "products.list", WS, &[], None, None).await;
    assert_eq!((status, body), (200, json!({ "data": [], "pageCount": 0 })));
}

#[tokio::test]
async fn products_public_api_lists_products_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let own = new_product(&f, WS, json!({ "name": "Own" })).await;
    new_product(&f, OTHER_WS, json!({ "name": "Other" })).await;
    let (status, body) = call(&f, "products.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (
            body["data"]
                .as_array()
                .map(|d| d.iter().map(id_of).collect::<Vec<_>>()),
            &body["pageCount"]
        ),
        (Some(vec![id_of(&own)]), &json!(1))
    );
    assert_eq!(body["data"][0]["isActive"], json!(true));
}

#[tokio::test]
async fn products_public_api_lists_sendable_products_for_the_token_s_workspace_and_the_requested_channel()
 {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "products.listSendable",
        WS,
        &[],
        Some("channel=whatsapp&keyword=shoe"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "ready": false, "reason": "catalogNotConnected", "data": [], "pageCount": 0 })
    );
}

#[tokio::test]
async fn products_public_api_a_workspace_id_in_the_input_can_never_override_the_token_s_workspace()
{
    let f = fixture().await;
    new_product(&f, OTHER_WS, json!({ "name": "Other" })).await;
    let query = format!("workspaceId={OTHER_WS}");
    let (status, body) = call(&f, "products.list", WS, &[], Some(&query), None).await;
    assert_eq!(
        (status, body["data"].as_array().map(Vec::len)),
        (200, Some(0)),
        "{body}"
    );
}

#[tokio::test]
async fn products_public_api_fetches_a_product_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let foreign = id_of(&new_product(&f, OTHER_WS, json!({})).await);
    let (status, body) = call(&f, "products.get", WS, &[("id", &foreign)], None, None).await;
    assert_eq!(
        (status, &body["message"]),
        (404, &json!("Product does not exist.")),
        "{body}"
    );
    let own = new_product(&f, WS, json!({})).await;
    let (status, body) = call(&f, "products.get", WS, &[("id", &id_of(&own))], None, None).await;
    assert_eq!(status, 200, "{body}");
    let mut expected = own.clone();
    for key in ["variantOptions", "variants", "addons"] {
        expected[key] = json!([]);
    }
    assert_eq!(body, expected);
}

#[tokio::test]
async fn products_public_api_creates_a_product_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let created = new_product(&f, WS, json!({ "name": "  Mug  ", "price": 3 })).await;
    assert_eq!(
        (
            &created["workspaceId"],
            &created["name"],
            &created["price"],
            &created["status"]
        ),
        (
            &json!(WS.to_string()),
            &json!("Mug"),
            &json!(3),
            &json!("active")
        )
    );
}

#[tokio::test]
async fn products_public_api_replaces_a_product_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let id = id_of(&new_product(&f, WS, json!({ "name": "Old", "sku": "OLD" })).await);
    let (status, body) = call(
        &f,
        "products.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "New" })),
    )
    .await;
    assert_eq!((status, body), (204, Value::Null));
    let (_, after) = call(&f, "products.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(
        (&after["name"], &after["sku"], &after["price"]),
        (&json!("New"), &json!(""), &json!(0))
    );
}

#[tokio::test]
async fn products_public_api_deletes_a_product_scoped_to_the_token_s_workspace_after_confirming_it_exists()
 {
    let f = fixture().await;
    let id = id_of(&new_product(&f, WS, json!({})).await);
    let (status, _) = call(&f, "products.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&f, "products.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn products_public_api_404s_does_not_call_delete_when_the_product_does_not_exist() {
    let f = fixture().await;
    let foreign = id_of(&new_product(&f, OTHER_WS, json!({})).await);
    let (status, body) = call(&f, "products.delete", WS, &[("id", &foreign)], None, None).await;
    assert_eq!(
        (status, &body["message"]),
        (404, &json!("Product does not exist."))
    );
    let (status, _) = call(
        &f,
        "products.get",
        OTHER_WS,
        &[("id", &foreign)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn products_public_api_sets_only_the_status_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let id = id_of(&new_product(&f, WS, json!({ "sku": "KEEP" })).await);
    let (status, _) = call(
        &f,
        "products.setActive",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "isActive": false })),
    )
    .await;
    assert_eq!(status, 204);
    let (_, after) = call(&f, "products.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(
        (&after["status"], &after["isActive"], &after["sku"]),
        (&json!("draft"), &json!(false), &json!("KEEP"))
    );
}

#[tokio::test]
async fn products_public_api_404s_does_not_update_when_the_product_does_not_exist() {
    let f = fixture().await;
    let foreign = id_of(&new_product(&f, OTHER_WS, json!({})).await);
    let (status, _) = call(
        &f,
        "products.setActive",
        WS,
        &[("id", &foreign)],
        None,
        Some(json!({ "isActive": false })),
    )
    .await;
    assert_eq!(status, 404);
    let (_, after) = call(
        &f,
        "products.get",
        OTHER_WS,
        &[("id", &foreign)],
        None,
        None,
    )
    .await;
    assert_eq!(after["status"], json!("active"));
}

#[tokio::test]
async fn products_public_api_deletes_the_given_ids_in_the_token_s_workspace() {
    let f = fixture().await;
    let own = id_of(&new_product(&f, WS, json!({})).await);
    let kept = id_of(&new_product(&f, WS, json!({})).await);
    let foreign = id_of(&new_product(&f, OTHER_WS, json!({})).await);
    let (status, _) = call(
        &f,
        "products.deleteMany",
        WS,
        &[],
        None,
        Some(json!({ "ids": [own, foreign] })),
    )
    .await;
    assert_eq!(status, 204);
    let (_, list) = call(&f, "products.list", WS, &[], None, None).await;
    assert_eq!(
        list["data"]
            .as_array()
            .map(|d| d.iter().map(id_of).collect::<Vec<_>>()),
        Some(vec![kept])
    );
    let (status, _) = call(
        &f,
        "products.get",
        OTHER_WS,
        &[("id", &foreign)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200);
}
