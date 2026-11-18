//! `workingHoursAPI.listWorkingHoursOptionsWorkspaceAuthAPI` and the working
//! hours actions (`features/working-hours/actions`): the ported public
//! `workingHours.*` routes ([`public`]), the same service as the builder.

use chatbotx_business::working_hours;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, arg, fail, public, schema_errors};
use crate::handlers::id;
use crate::handlers::working_hours::{holiday_json, schedule_json};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "createWorkingHoursAction",
    "updateWorkingHoursAction",
    "deleteWorkingHoursAction",
    "replaceWorkingHoursMembersAction",
    "createHolidayAction",
    "updateHolidayAction",
    "deleteHolidayAction",
    // `settings/inbox-teams/working-hours/[workingHoursId]/page.tsx` data,
    // `args: [{ id }]`: `{ workingHours, timezone }` (404 when not in the
    // workspace).
    "workingHoursEditorPage",
    // The working-hours part of `settings/inbox-teams/page.tsx`:
    // `{ workingHours, members: [{ value, label }], holidays }`.
    "workingHoursListPage",
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
        "workingHoursListPage" => {
            let (db, ws) = (&ctx.api.db, ctx.workspace_id);
            let rows = working_hours::list(db, ws).await.map_err(fail)?;
            let holidays = working_hours::list_date_overrides(db, ws, None)
                .await
                .map_err(fail)?;
            let members = chatbotx_business::tx::read(db, move |conn| {
                let mut stmt = conn
                    .prepare(
                        r#"SELECT u."id", COALESCE(u."name", u."email") FROM "WorkspaceMember" m
                           JOIN "User" u ON u."id" = m."userId" WHERE m."workspaceId" = ?1 ORDER BY m."id""#,
                    )
                    .map_err(|e| chatbotx_business::BusinessError::Repo(e.into()))?;
                let rows = stmt
                    .query_map([ws], |r| {
                        Ok(json!({ "value": r.get::<_, i64>(0)?.to_string(), "label": r.get::<_, String>(1)? }))
                    })
                    .and_then(Iterator::collect::<Result<Vec<Value>, _>>)
                    .map_err(|e| chatbotx_business::BusinessError::Repo(e.into()))?;
                Ok(rows)
            })
            .await
            .map_err(fail)?;
            Ok(json!({
                "workingHours": rows.iter().map(|s| schedule_json(s, None)).collect::<Vec<_>>(),
                "members": members,
                "holidays": holidays.iter().map(holiday_json).collect::<Vec<_>>(),
            }))
        }
        "workingHoursEditorPage" => {
            let params = [("id", In(arg(args, 0)).id("id")?.to_string())];
            let resource = public(ctx, "workingHours.get", &params, None, None).await?;
            let ws = ctx.workspace_id;
            let timezone = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
                conn.query_row(
                    r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    [ws],
                    |r| r.get::<_, Option<String>>(0),
                )
                .map_err(|e| chatbotx_business::BusinessError::Repo(e.into()))
            })
            .await
            .map_err(fail)?;
            Ok(json!({ "workingHours": resource, "timezone": timezone }))
        }
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
