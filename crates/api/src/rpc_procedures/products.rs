//! The e-commerce screens: products, product imports, Meta catalog, product
//! categories, vendors, open hours, shop links (`features/{products,
//! product-categories,product-vendors,ecommerce-open-hours,webshop}`).
//!
//! The actions run the ported public `/v1` routes ([`public`]) where the
//! input is the same; the page data (server components of
//! `app/space/[workspaceId]/(e-commerce)/*` and `products/*`) are the page
//! queries `productsPage`, `productEditPage`, `productCategoriesPage`,
//! `productVendorsPage`, `ecommerceShareLinks` (`serverQuery`, P4 spec §5.1).

use chatbotx_business::product::{self, ProductListInput};
use chatbotx_business::product_category::{self, LevelInput, ListFilter};
use chatbotx_business::store_source::resolve_store_source;
use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::product as product_repo;
use chatbotx_db::repo::product_category::CategoryWithCount;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public, query_of};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "productsPage",
    "productEditPage",
    "productCategoriesPage",
    "productVendorsPage",
    "ecommerceShareLinks",
    "createProductAction",
    "updateProductAction",
    "deleteProductAction",
    "toggleProductActiveAction",
    "getProductDetailAction",
    "importProductsAction",
    "getMetaCatalogStateAction",
    "selectMetaCatalogAction",
    "listMetaCatalogBusinessesAction",
    "createMetaCatalogAction",
    "syncToMetaCatalogAction",
    "disconnectMetaCatalogAction",
    "createProductCategoryAction",
    "updateProductCategoryAction",
    "deleteProductCategoryAction",
    "toggleProductCategoryVisibleAction",
    "createProductVendorAction",
    "updateProductVendorAction",
    "deleteProductVendorAction",
    "toggleProductVendorActiveAction",
    "createOpenHoursAction",
    "updateOpenHoursAction",
    "deleteOpenHoursAction",
    "getWebshopLinksAction",
];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        // `productService.list` (the product picker).
        "productsAPI.listProductsAuthorizedAPI" => list_products(ctx, input).await,
        // `productService.listFormOptions`.
        "productsAPI.listProductFormOptionsAPI" => {
            let ws = ctx.workspace_id;
            tx::read(&ctx.api.db, move |conn| {
                let source = resolve_store_source(conn, ws)?;
                let (products, vendors) = product_repo::list_form_options(conn, ws, Some(&source))?;
                let pairs = |rows: Vec<(i64, String)>| -> Vec<Value> {
                    rows.into_iter()
                        .map(|(id, name)| json!({ "id": id.to_string(), "name": name }))
                        .collect()
                };
                Ok(json!({ "products": pairs(products), "vendors": pairs(vendors) }))
            })
            .await
            .map_err(fail)
        }
        // `productCategoryService.list` (the whole tree).
        "productCategoriesAPI.listProductCategoriesAPI" => categories(ctx).await.map(Value::Array),
        _ => return None,
    })
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// The id bound at `index` (`zodBigintAsString()`).
fn bound_id(args: &[Value], index: usize) -> Result<String, OrpcError> {
    In(&json!({ "id": arg(args, index) }))
        .id("id")
        .map(|id| id.to_string())
}

/// `""` → `null` for the form keys (`emptyToNull` of the action schemas).
fn empty_to_null(body: &Value, keys: &[&str]) -> Value {
    let mut out = body.clone();
    if let Some(object) = out.as_object_mut() {
        for key in keys {
            if object
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|text| text.trim().is_empty())
            {
                object.insert((*key).to_string(), Value::Null);
            }
        }
    }
    out
}

/// `body` with `extra` merged in.
fn with(body: &Value, extra: Value) -> Value {
    let mut out = body.as_object().cloned().unwrap_or_default();
    if let Value::Object(extra) = extra {
        out.extend(extra);
    }
    Value::Object(out)
}

/// The `ecommerce` permission of the e-commerce layouts (`notFound()`).
fn require_ecommerce(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    if ctx.has("ecommerce") {
        Ok(())
    } else {
        Err(fail(BusinessError::not_found("Page not found")))
    }
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let body = arg(args, args.len().saturating_sub(1).max(1));
    let call = |route: &'static str, params: Vec<(&'static str, String)>, body: Option<Value>| async move {
        public(ctx, route, &params, None, body.as_ref())
            .await
            .map_err(action_error)
    };
    match name {
        "productsPage" => products_page(ctx, arg(args, 1)).await,
        "productEditPage" => product_edit_page(ctx, arg(args, 1)).await,
        "productCategoriesPage" => categories_page(ctx, arg(args, 1)).await,
        "productVendorsPage" => vendors_page(ctx, arg(args, 1)).await,
        "ecommerceShareLinks" => {
            require_ecommerce(ctx)?;
            share_links(ctx)
        }
        // `productService.createFull` → `{ data: product }`.
        "createProductAction" => call("products.create", vec![], Some(body.clone()))
            .await
            .map(|data| json!({ "data": data })),
        "updateProductAction" => call(
            "products.update",
            vec![("id", bound_id(args, 1)?)],
            Some(arg(args, 2).clone()),
        )
        .await
        .map(|_| Value::Null),
        "deleteProductAction" => call("products.deleteMany", vec![], Some(body.clone()))
            .await
            .map(|_| Value::Null),
        "toggleProductActiveAction" => {
            let id = bound_id(args, 1)?;
            call(
                "products.setActive",
                vec![("id", id.clone())],
                Some(with(arg(args, 2), json!({ "id": id }))),
            )
            .await
            .map(|_| Value::Null)
        }
        "getProductDetailAction" => {
            let id = bound_id(args, 1)?.parse().unwrap_or(-1);
            product::find_by_id(&ctx.api.db, ctx.workspace_id, id)
                .await
                .map(|detail| detail_json(&detail))
                .map_err(action_fail)
        }
        // `{ fileId, format, meta: { columnMap, createMissingCategories } }`.
        "importProductsAction" => {
            let meta = body.get("meta").cloned().unwrap_or(Value::Null);
            let start = json!({
                "fileId": body.get("fileId"),
                "format": body.get("format"),
                "columnMap": meta.get("columnMap"),
                "createMissingCategories": meta.get("createMissingCategories").cloned().unwrap_or(json!(true)),
            });
            call("products.startImport", vec![], Some(start)).await
        }
        "getMetaCatalogStateAction" => call("products.getMetaCatalog", vec![], None).await,
        "selectMetaCatalogAction" => {
            call("products.selectMetaCatalog", vec![], Some(body.clone())).await
        }
        "listMetaCatalogBusinessesAction" => {
            call("products.listMetaCatalogBusinesses", vec![], None).await
        }
        "createMetaCatalogAction" => {
            call("products.createMetaCatalog", vec![], Some(body.clone())).await
        }
        "syncToMetaCatalogAction" => {
            call("products.syncMetaCatalog", vec![], Some(body.clone())).await
        }
        "disconnectMetaCatalogAction" => call("products.disconnectMetaCatalog", vec![], None)
            .await
            .map(|_| Value::Null),
        "createProductCategoryAction" => {
            call(
                "productCategories.create",
                vec![],
                Some(category_form(body)?),
            )
            .await
        }
        "updateProductCategoryAction" => {
            let id = bound_id(args, 1)?;
            let form = with(&category_form(arg(args, 2))?, json!({ "id": id }));
            update_category(ctx, &id, &form).await
        }
        "toggleProductCategoryVisibleAction" => {
            let id = bound_id(args, 1)?;
            call(
                "productCategories.setVisible",
                vec![("id", id.clone())],
                Some(with(arg(args, 2), json!({ "id": id }))),
            )
            .await
        }
        "deleteProductCategoryAction" => call(
            "productCategories.delete",
            vec![("id", bound_id(args, 1)?)],
            None,
        )
        .await
        .map(|_| Value::Null),
        "createProductVendorAction" => {
            call("productVendors.create", vec![], Some(vendor_form(body))).await
        }
        "updateProductVendorAction" | "toggleProductVendorActiveAction" => {
            let id = bound_id(args, 1)?;
            call(
                "productVendors.update",
                vec![("id", id.clone())],
                Some(with(&vendor_form(arg(args, 2)), json!({ "id": id }))),
            )
            .await
        }
        "deleteProductVendorAction" => call(
            "productVendors.delete",
            vec![("id", bound_id(args, 1)?)],
            None,
        )
        .await
        .map(|_| Value::Null),
        "createOpenHoursAction" => call("openHours.create", vec![], Some(body.clone())).await,
        "updateOpenHoursAction" => {
            let id = bound_id(args, 1)?;
            call(
                "openHours.update",
                vec![("id", id.clone())],
                Some(with(arg(args, 2), json!({ "id": id }))),
            )
            .await
        }
        "deleteOpenHoursAction" => call("openHours.delete", vec![("id", bound_id(args, 1)?)], None)
            .await
            .map(|_| Value::Null),
        _ => webshop_links(ctx, body).await,
    }
}

/// A route error as the action error: a `validation` exception with a field
/// is `validationErrors` (`returnValidationErrors`), as [`action_fail`].
fn action_error(mut error: OrpcError) -> OrpcError {
    if let Some(field) = error
        .data
        .as_ref()
        .and_then(|d| d.get("field"))
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        error.data = Some(json!({
            "validationErrors": {
                "_errors": ["Validation Exception"],
                field: { "_errors": [error.message] },
            }
        }));
    }
    error
}

/// `productCategoryFormSchema`: `""` image → null, `rank` coerced.
fn category_form(body: &Value) -> Result<Value, OrpcError> {
    let mut out = empty_to_null(body, &["imageUrl"]);
    if let Some(rank) = out.get("rank").and_then(Value::as_str) {
        let rank = rank.trim().parse::<i64>().map_err(|_| invalid())?;
        out["rank"] = json!(rank);
    }
    Ok(out)
}

/// `productCategoryService.update`: `parentId` left out keeps the parent.
async fn update_category(ctx: &Ctx<'_>, id: &str, form: &Value) -> Result<Value, OrpcError> {
    public(
        ctx,
        "productCategories.update",
        &[("id", id.to_string())],
        None,
        Some(form),
    )
    .await
    .map_err(action_error)
}

/// `productVendorFormSchema` (`emptyToNull` keys).
fn vendor_form(body: &Value) -> Value {
    empty_to_null(
        body,
        &[
            "shortDescription",
            "phone",
            "storeAddress",
            "imageUrl",
            "openHoursId",
            "categoryId",
            "subcategoryId",
        ],
    )
}

// ---- page data --------------------------------------------------------------

/// `listWithProductCount` rows.
fn category_json(row: &CategoryWithCount) -> Map<String, Value> {
    let Value::Object(out) = json!({
        "id": row.id.to_string(),
        "parentId": row.parent_id.map(|id| id.to_string()),
        "name": row.name,
        "imageUrl": row.image_url,
        "isVisible": row.is_visible,
        "rank": row.rank,
        "productCount": row.product_count,
    }) else {
        unreachable!("json! object")
    };
    out
}

async fn categories(ctx: &Ctx<'_>) -> Result<Vec<Value>, OrpcError> {
    product_category::list(&ctx.api.db, ctx.workspace_id, ListFilter::default())
        .await
        .map(|rows| {
            rows.iter()
                .map(|r| Value::Object(category_json(r)))
                .collect()
        })
        .map_err(fail)
}

/// `products/page.tsx`: products, categories, imports, Meta catalog state,
/// external store links (`null`: Shopify / WooCommerce are not ported).
/// A `productResource` list row.
fn list_row_json(row: &product_repo::ProductListRow) -> Value {
    let mut out = row.product.to_json();
    out["category"] = json!(row.category);
    out["subcategory"] = json!(row.subcategory);
    out["vendor"] = json!(row.vendor);
    out["variantCount"] = json!(row.variant_count);
    out["variantInventoryTotal"] = json!(row.variant_inventory_total);
    out
}

/// `listProductsAuthorizedAPI`: `listProductsRequest` + `ids` (1..=50).
async fn list_products(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input_in = In(input);
    let ids = match input_in.get("ids") {
        None => None,
        Some(_) => {
            let ids = input_in.ids("ids")?;
            if ids.is_empty() || ids.len() > PRODUCT_IDS_FILTER_MAX {
                return Err(invalid());
            }
            Some(ids)
        }
    };
    let list = ProductListInput {
        workspace_id: ctx.workspace_id,
        name: input_in.opt_str("name")?,
        category_id: input_in.opt_id("categoryId")?,
        ids,
        page: input_in.opt_positive("page")?,
        per_page: input_in.opt_positive("perPage")?,
        sort: input_in.sort(),
        ..ProductListInput::default()
    };
    let products = product::list(&ctx.api.db, list).await.map_err(fail)?;
    let data: Vec<Value> = products.data.iter().map(list_row_json).collect();
    Ok(json!({ "data": data, "pageCount": products.page_count }))
}

/// `PRODUCT_IDS_FILTER_MAX`: a picker never holds more products than this.
const PRODUCT_IDS_FILTER_MAX: usize = 50;

async fn products_page(ctx: &Ctx<'_>, search: &Value) -> Result<Value, OrpcError> {
    require_ecommerce(ctx)?;
    let search_in = In(search);
    let input = ProductListInput {
        workspace_id: ctx.workspace_id,
        name: search_in.opt_str("name")?,
        category_id: search_in.opt_id("categoryId")?,
        page: search_in.opt_positive("page")?,
        per_page: search_in.opt_positive("perPage")?,
        sort: search_in.sort(),
        ..ProductListInput::default()
    };
    let products = product::list(&ctx.api.db, input).await.map_err(fail)?;
    let data: Vec<Value> = products.data.iter().map(list_row_json).collect();
    let imports = public(
        ctx,
        "products.listImports",
        &[],
        Some(&query_of(
            &json!({ "page": 1, "perPage": 10 }),
            &["page", "perPage"],
        )),
        None,
    )
    .await?;
    let meta = public(ctx, "products.getMetaCatalog", &[], None, None).await?;
    Ok(json!({
        "products": { "data": data, "pageCount": products.page_count },
        "categories": categories(ctx).await?,
        "importHistory": imports,
        "metaCatalog": meta,
        "externalStoreLinks": chatbotx_business::external_store::store_links(
            &ctx.api.db,
            &ctx.api.app_url,
            ctx.workspace_id,
        )
        .await
        .map_err(super::fail)?,
    }))
}

/// `findDetail`: the row with category / subcategory / vendor names and the
/// variant options, variants and add-ons.
fn detail_json(detail: &product::ProductDetail) -> Value {
    let mut out = detail.product.to_json();
    out["category"] = json!(detail.category);
    out["subcategory"] = json!(detail.subcategory);
    out["vendor"] = json!(detail.vendor);
    out["variantOptions"] = detail.variant_options.iter().map(|r| r.to_json()).collect();
    out["variants"] = detail.variants.iter().map(|r| r.to_json()).collect();
    out["addons"] = detail.addons.iter().map(|r| r.to_json()).collect();
    out
}

/// `products/[id]/edit/page.tsx`: the product, its Meta catalog link and
/// its shop button link.
async fn product_edit_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    require_ecommerce(ctx)?;
    let id = In(input).id("id")?;
    let detail = product::find_by_id(&ctx.api.db, ctx.workspace_id, id)
        .await
        .map_err(fail)?;
    // `metaCatalogItemRepository.findLatestByProductId`.
    let catalog_id = tx::read(&ctx.api.db, move |conn| {
        conn.query_row(
            r#"SELECT "catalogId" FROM "MetaCatalogItem" WHERE "productId" = ?1
               ORDER BY "lastSyncedAt" IS NULL, "lastSyncedAt" DESC, "createdAt" DESC LIMIT 1"#,
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
    .map_err(fail)?;
    let ws = ctx.workspace_id.to_string();
    Ok(json!({
        "product": detail_json(&detail),
        "metaCatalogId": catalog_id,
        "publicUrl": shop_url(&ctx.api.app_url, &ws, &format!("/products/{id}"), SHOP_TOKEN, &[]),
    }))
}

/// `product-categories/page.tsx`: one level of the tree.
async fn categories_page(ctx: &Ctx<'_>, search: &Value) -> Result<Value, OrpcError> {
    require_ecommerce(ctx)?;
    let search_in = In(search);
    let level = product_category::list_level(
        &ctx.api.db,
        LevelInput {
            workspace_id: ctx.workspace_id,
            parent_id: search_in.opt_id("parentId")?,
            name: search_in.opt_str("name")?,
            page: search_in.opt_positive("page")?,
            per_page: search_in.opt_positive("perPage")?,
        },
    )
    .await
    .map_err(fail)?;
    let data: Vec<Value> = level
        .data
        .iter()
        .map(|row| {
            let mut out = category_json(&row.category);
            out.insert("childCount".into(), json!(row.child_count));
            Value::Object(out)
        })
        .collect();
    Ok(json!({
        "data": data,
        "pageCount": level.page_count,
        "parent": level.parent.as_ref().map(|p| json!({ "id": p.id.to_string(), "name": p.name })),
    }))
}

/// `product-vendors/page.tsx`: vendors, categories, open hours.
async fn vendors_page(ctx: &Ctx<'_>, search: &Value) -> Result<Value, OrpcError> {
    require_ecommerce(ctx)?;
    let query = query_of(search, &["name", "page", "perPage"]);
    let vendors = public(ctx, "productVendors.list", &[], Some(&query), None).await?;
    let open_hours = public(ctx, "openHours.list", &[], None, None).await?;
    Ok(json!({
        "vendors": vendors,
        "categories": categories(ctx).await?,
        "openHours": open_hours.get("data").cloned().unwrap_or(open_hours),
    }))
}

// ---- shop links ---------------------------------------------------------------

/// The token placeholder a bot button carries (`buildWebshopButtonUrl`).
const SHOP_TOKEN: &str = "{{shop_token}}";

/// `buildWebshopUrl`: `<appUrl>/shop/<ws><page>?token=…[&…]`.
fn shop_url(app_url: &str, ws: &str, page: &str, token: &str, query: &[(&str, &str)]) -> String {
    let mut url = format!(
        "{}/shop/{}{}?token={}",
        app_url.trim_end_matches('/'),
        ws,
        page,
        token
    );
    for (key, value) in query {
        url.push_str(&format!("&{key}={value}"));
    }
    url
}

fn preview_token(ws: &str) -> Result<String, OrpcError> {
    chatbotx_business::meta_catalog::process_encryption()
        .sign_shop_preview_token(ws)
        .map_err(|e| crate::handlers::internal_error(&e.to_string()))
}

/// `createWebshopShareLinks`: the preview link and the bot button links.
// ponytail: the platform app URL; tenant custom domains are not ported.
fn share_links(ctx: &Ctx<'_>) -> Result<Value, OrpcError> {
    let ws = ctx.workspace_id.to_string();
    let app = &ctx.api.app_url;
    Ok(json!({
        "previewUrl": shop_url(app, &ws, "", &preview_token(&ws)?, &[]),
        "buttonUrls": {
            "shop": shop_url(app, &ws, "", SHOP_TOKEN, &[]),
            "cart": shop_url(app, &ws, "/cart", SHOP_TOKEN, &[]),
            "categories": shop_url(app, &ws, "/categories", SHOP_TOKEN, &[]),
        },
    }))
}

/// `getWebshopLinksAction`: `createWebshopLink` without a contact.
async fn webshop_links(ctx: &Ctx<'_>, body: &Value) -> Result<Value, OrpcError> {
    let input = In(body);
    let product_id = input.opt_id("productId")?;
    let vendor_id = input.opt_id("vendorId")?;
    let category_id = input.opt_id("categoryId")?;
    let ws = ctx.workspace_id;
    tx::read(&ctx.api.db, move |conn| {
        let missing = |sql: &str, id: Option<i64>| -> Result<bool, BusinessError> {
            Ok(match id {
                Some(id) => conn
                    .query_row(sql, params![id, ws], |_| Ok(()))
                    .optional()
                    .map_err(|e| BusinessError::Repo(e.into()))?
                    .is_none(),
                None => false,
            })
        };
        if missing(
            r#"SELECT 1 FROM "Vendor" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            vendor_id,
        )? {
            return Err(BusinessError::not_found("Vendor does not exist."));
        }
        if missing(
            r#"SELECT 1 FROM "ProductCategory" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            category_id,
        )? {
            return Err(BusinessError::not_found("Product category does not exist."));
        }
        if missing(
            r#"SELECT 1 FROM "Product" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            product_id,
        )? {
            return Err(BusinessError::not_found("Product does not exist."));
        }
        Ok(())
    })
    .await
    .map_err(action_fail)?;
    let ws = ws.to_string();
    let page = product_id
        .map(|id| format!("/products/{id}"))
        .unwrap_or_default();
    let (vendor, category) = (
        vendor_id.map(|id| id.to_string()),
        category_id.map(|id| id.to_string()),
    );
    let mut query = Vec::new();
    if let Some(vendor) = vendor.as_deref() {
        query.push(("vendor", vendor));
    }
    if let Some(category) = category.as_deref() {
        query.push(("category", category));
    }
    let app = &ctx.api.app_url;
    Ok(json!({
        "previewUrl": shop_url(app, &ws, &page, &preview_token(&ws)?, &query),
        "buttonUrl": shop_url(app, &ws, &page, SHOP_TOKEN, &query),
    }))
}
