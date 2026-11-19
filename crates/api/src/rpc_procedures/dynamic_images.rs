//! `features/dynamic-images/{queries,actions}`: the dynamic images list and
//! editor of the SPA (`dynamicImageService`).

use chatbotx_business::dynamic_image::{self as service, Deps, ListInput, Upsert};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg};
use crate::handlers::dynamic_images::{files, is_duplicate_name, resource_json, upsert_fields};
use crate::handlers::input::Input;
use crate::orpc::OrpcError;

const ACTIONS: &[&str] = &[
    "listDynamicImages",
    "findDynamicImage",
    "createDynamicImageAction",
    "updateDynamicImageAction",
    "deleteDynamicImagesAction",
    "enableDynamicImageAction",
];

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// `returnValidationErrors(..., { name: "<feature> already exists" })`.
fn duplicate_name() -> OrpcError {
    let mut out = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    out.data = Some(json!({
        "validationErrors": { "name": { "_errors": ["Dynamic image already exists"] } }
    }));
    out
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let storage = crate::handlers::contacts::tenant_urls(ctx.api).storage_url;
    let deps = || -> Result<Deps<'_>, OrpcError> {
        Ok(Deps {
            db,
            files: files(ctx.api)?,
            storage_url: &storage,
        })
    };
    let bound = |index: usize| In(&json!({ "id": arg(args, index) })).id("id");
    match name {
        "listDynamicImages" => {
            let search = In(arg(args, 0));
            let page = service::list(
                db,
                ListInput {
                    workspace_id: ws,
                    page: search.opt_positive("page")?,
                    per_page: search.opt_positive("perPage")?,
                    name: search.opt_str("name")?,
                    sort: search.sort(),
                },
            )
            .await
            .map_err(action_fail)?;
            Ok(json!({
                "data": page.data.iter().map(resource_json).collect::<Vec<_>>(),
                "pageCount": page.page_count,
            }))
        }
        "findDynamicImage" => match service::find(db, ws, In(arg(args, 0)).id("id")?).await {
            Ok(row) => Ok(resource_json(&row)),
            Err(error) if error.code() == Some("notFound") => Ok(Value::Null),
            Err(error) => Err(action_fail(error)),
        },
        "createDynamicImageAction" | "updateDynamicImageAction" => {
            let create = name == "createDynamicImageAction";
            let body = arg(args, if create { 1 } else { 2 }).to_string();
            let mut input = Input::from_body(&[], body.as_bytes());
            let fields = upsert_fields(&mut input);
            input.finish().map_err(super::schema_errors)?;
            let (name, custom_field_id, data) = fields.ok_or_else(super::invalid)?;
            let upsert = Upsert {
                workspace_id: ws,
                name,
                custom_field_id,
                data,
            };
            let result = if create {
                service::create(deps()?, upsert).await
            } else {
                service::update(deps()?, bound(1)?, upsert).await
            };
            match result {
                Ok(row) if create => Ok(json!({ "id": row.id.to_string() })),
                Ok(_) => Ok(Value::Null),
                Err(error) if is_duplicate_name(&error) => Err(duplicate_name()),
                Err(error) => Err(action_fail(error)),
            }
        }
        "deleteDynamicImagesAction" => {
            for id in In(arg(args, 1)).ids("ids")? {
                service::delete(deps()?, ws, id)
                    .await
                    .map_err(action_fail)?;
            }
            Ok(Value::Null)
        }
        "enableDynamicImageAction" => {
            let enabled = In(arg(args, 2))
                .opt_bool("enabled")?
                .ok_or_else(super::invalid)?;
            service::set_enabled(db, ws, bound(1)?, enabled)
                .await
                .map(|_| Value::Null)
                .map_err(action_fail)
        }
        _ => Err(super::invalid()),
    }
}
