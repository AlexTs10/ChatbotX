//! `features/integration-smtp/api/public.ts`: `smtpIntegrations.*`, with
//! `createSmtpRequest` / `updateSmtpRequest` and `prepareSmtpAuth`.

use chatbotx_business::email_marketing::smtp::PROVIDERS;
use chatbotx_business::email_marketing::{is_email, smtp_service as smtp};
use serde_json::{Value, json};

use super::input::{Input, Num, Str, key};
use super::qr_codes::public_paging;
use super::{ApiState, Call, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "smtpIntegrations.list",
    "smtpIntegrations.get",
    "smtpIntegrations.create",
    "smtpIntegrations.update",
    "smtpIntegrations.delete",
];

/// `toPublicResource`: never the `auth` blob.
fn resource(row: &smtp::SmtpRow) -> Value {
    json!({ "id": row.id.to_string(), "name": row.name, "fromAddress": row.from_address })
}

/// `fromAddressSchema`: an email, or `Name <email>`.
fn valid_from(text: &str) -> bool {
    let email = text
        .strip_suffix('>')
        .and_then(|rest| rest.rsplit_once('<'))
        .map_or(text, |(_, email)| email.trim());
    is_email(email)
}

/// `createSmtpRequest` (`create`) / `updateSmtpRequest`.
fn parse(input: &mut Input, create: bool) -> Option<smtp::SmtpInput> {
    let provider = input.get("provider").cloned();
    let provider = input.enum_at(&key("provider"), provider.as_ref(), PROVIDERS, true);
    let host = input.string("host", Str::default());
    let port_value = input.get("port").cloned();
    let max = if create {
        65_535.0
    } else {
        9_007_199_254_740_991.0
    };
    let port = input.coerce_number_at(&key("port"), port_value.as_ref(), Num::int(1.0, max), true);
    let limit = if create { Some(255) } else { None };
    let spec = Str {
        min: Some(1),
        max: limit,
        ..Str::default()
    };
    let username = input.string("username", spec);
    let password = input.string("password", spec);
    let from_address = input.string(
        "fromAddress",
        Str {
            trim: true,
            min: Some(1),
            ..Str::default()
        },
    );
    if from_address.as_deref().is_some_and(|f| !valid_from(f)) {
        input.issue(
            &key("fromAddress"),
            "custom",
            "Invalid from address. Use an email or 'Name <email>' format.",
        );
        return None;
    }
    let (provider, host, port, username, password, from_address) =
        (provider?, host?, port?, username?, password?, from_address?);
    // `superRefine` (runs once the object parsed).
    if provider == "other" && host.trim().is_empty() {
        input.issue(
            &key("host"),
            "invalid_type",
            "Host is required when provider is 'other'",
        );
        return None;
    }
    Some(smtp::SmtpInput {
        provider,
        host,
        // ponytail: an update port past 65535 cannot connect; 0 = the verify fails.
        port: u16::try_from(port as i64).unwrap_or(0),
        username,
        password,
        from_address,
    })
}

pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "smtpIntegrations.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let rows = smtp::list_by_workspace(db, ws)
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            // `paginateInMemory`.
            let total = rows.len() as i64;
            let page_count = ((total + per_page - 1) / per_page).max(1);
            let data: Vec<Value> = rows
                .into_iter()
                .skip(((page - 1) * per_page) as usize)
                .take(per_page as usize)
                .collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "smtpIntegrations.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            smtp::find_by_id_for_workspace(db, id.unwrap_or_default(), ws)
                .await
                .map(|row| json(200, resource(&row)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "smtpIntegrations.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse(&mut input, true);
            input.finish()?;
            let Some(data) = data else {
                return Err(super::internal_error("smtp parse"));
            };
            let row = async {
                let (host, port) = smtp::prepare_auth(&data).await?;
                let id = smtp::connect(db, ws, None, &data, &host, port).await?;
                smtp::find_by_id_for_workspace(db, id, ws).await
            }
            .await
            .map_err(|e| business_error(e, super::CREATING))?;
            state
                .record_audit(
                    "connect",
                    &format!("connected a new SMTP channel (#{})", row.id),
                )
                .await;
            Ok(json(201, resource(&row)))
        }
        "smtpIntegrations.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let data = parse(&mut input, false);
            input.finish()?;
            let (Some(id), Some(data)) = (id, data) else {
                return Err(super::internal_error("smtp parse"));
            };
            let (row, changed) = async {
                let (host, port) = smtp::prepare_auth(&data).await?;
                let changed = smtp::update(db, ws, id, &data, &host, port).await?;
                Ok((smtp::find_by_id_for_workspace(db, id, ws).await?, changed))
            }
            .await
            .map_err(|e| business_error(e, super::MUTATING))?;
            if changed {
                state
                    .record_audit("update", "updated the SMTP channel configuration")
                    .await;
            }
            Ok(json(200, resource(&row)))
        }
        "smtpIntegrations.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            let id = id.unwrap_or_default();
            smtp::disconnect(db, ws, id)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            state
                .record_audit(
                    "disconnect",
                    &format!("disconnected the SMTP channel (#{id})"),
                )
                .await;
            Ok(no_content())
        }
        _ => Err(super::internal_error("unknown smtp route")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_address_formats() {
        assert!(valid_from("a@b.co"));
        assert!(valid_from("Ada <a@b.co>"));
        assert!(valid_from("\"Ada\" <a@b.co>"));
        assert!(!valid_from("Ada"));
        assert!(!valid_from("Ada <nope>"));
    }
}
