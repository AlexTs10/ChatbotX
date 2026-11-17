//! `workingHoursAPI.listWorkingHoursOptionsWorkspaceAuthAPI` and the working
//! hours actions (`features/working-hours/actions`): the ported public
//! `workingHours.*` routes ([`public`]), the same service as the builder.

use chatbotx_business::working_hours;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, arg, fail, public, schema_errors};
use crate::handlers::id;
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "createWorkingHoursAction",
    "updateWorkingHoursAction",
    "deleteWorkingHoursAction",
    "replaceWorkingHoursMembersAction",
    "createHolidayAction",
    "updateHolidayAction",
    "deleteHolidayAction",
];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, _input: &Value) -> Outcome {
    if path != "workingHoursAPI.listWorkingHoursOptionsWorkspaceAuthAPI" {
        return None;
    }
    Some(
        working_hours::list(&ctx.api.db, ctx.workspace_id)
            .await
            .map(|rows| {
                let data: Vec<Value> = rows
                    .iter()
                    .map(|s| json!({ "id": id(s.row.id), "name": s.row.name }))
                    .collect();
                json!({ "data": data })
            })
            .map_err(fail),
    )
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

fn bound(args: &[Value], index: usize) -> Result<String, OrpcError> {
    In(&json!({ "id": arg(args, index) }))
        .id("id")
        .map(|id| id.to_string())
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let with_id = |route: &'static str, body: Option<&'static str>| async move {
        let params = [("id", bound(args, 1)?)];
        let body = body.map(|_| arg(args, 2));
        public(ctx, route, &params, None, body)
            .await
            .map_err(schema_errors)
    };
    match name {
        "createWorkingHoursAction" => {
            public(ctx, "workingHours.create", &[], None, Some(arg(args, 1)))
                .await
                .map_err(schema_errors)
        }
        "createHolidayAction" => public(
            ctx,
            "workingHours.createHoliday",
            &[],
            None,
            Some(arg(args, 1)),
        )
        .await
        .map_err(schema_errors),
        "updateWorkingHoursAction" => {
            let mut row = with_id("workingHours.update", Some("body")).await?;
            // The action returns the resource without `dateOverrides`.
            if let Some(object) = row.as_object_mut() {
                object.remove("dateOverrides");
            }
            Ok(row)
        }
        "replaceWorkingHoursMembersAction" => {
            with_id("workingHours.setMembers", Some("body")).await
        }
        "updateHolidayAction" => with_id("workingHours.updateHoliday", Some("body")).await,
        "deleteWorkingHoursAction" => with_id("workingHours.delete", None)
            .await
            .map(|_| Value::Null),
        "deleteHolidayAction" => with_id("workingHours.deleteHoliday", None)
            .await
            .map(|_| Value::Null),
        _ => Ok(Value::Null),
    }
}
