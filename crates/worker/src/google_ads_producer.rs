//! The two Google Ads conversion producers' shared part
//! (`google-ads/resolve-conversion-inputs.ts`, `google-ads-input-error.ts`):
//! the consent setting and the `{{variable}}` fields are resolved, then
//! `recordGoogleAdsConversion` runs. Used by the `sendGoogleAdsConversion`
//! flow step (`steps/mod.rs`) and trigger action (`trigger.rs`).

use chatbotx_business::google_ads::stored_consent;
use chatbotx_business::google_ads_conversion::{self as gac, RecordInput, Recorded};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::now_ms;
use serde_json::{Map, Value, json};

use crate::Deps;

/// The template fields of the step / action (`GoogleAdsConversionFieldTemplates`).
const FIELDS: &[&str] = &[
    "value",
    "currency",
    "dedupId",
    "conversionTime",
    "customerType",
    "customerValueBucket",
];
/// `WITHHELD_FIELDS`: customer data is never printed in the Error Log.
const WITHHELD: &[&str] = &["customerType", "customerValueBucket"];

/// One producer call.
pub(crate) struct Produce<'a> {
    pub deps: &'a Deps,
    pub conversation: &'a Conversation,
    /// The inbox the variables resolve against and the event is recorded on.
    pub contact_inbox: &'a ContactInbox,
    /// `flowStep` | `triggerAction`.
    pub source: &'static str,
    pub scope_id: String,
    /// The step / the parsed trigger action.
    pub fields: &'a Value,
    /// Set only for `dedupMode = "event"` with a durable key.
    pub occurrence_key: Option<String>,
    pub sent_by_user_id: Option<String>,
    /// Appended to the Error Log message (`\nTrigger: {id}`).
    pub report_suffix: String,
}

/// What the producer did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Produced {
    Queued,
    /// The `record` status / config failure name (`noClick`,
    /// `invalidConsentConfig`, ...): the error code is
    /// `googleAdsConversionErrorCodes[name]`.
    Refused(&'static str),
    /// `record` threw (`recordFailed`).
    Failed(String),
}

impl Produced {
    /// The step's `errorMessage` (`google_ads_*`).
    pub fn error_code(&self) -> Option<&'static str> {
        let name = match self {
            Produced::Queued => return None,
            Produced::Refused(name) => name,
            Produced::Failed(_) => "recordFailed",
        };
        chatbotx_core::utils::google_click::google_ads_conversion_error_code(name)
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// `describeGoogleAdsInputFailure(code, resolved, error)`: the code, one
/// `path: message` line per validation issue, then the allowlisted fields.
pub(crate) fn describe_input_failure(code: &str, resolved: &Value, issues: &[String]) -> String {
    let mut lines = vec![code.to_owned()];
    lines.extend(issues.iter().cloned());
    let entries: Vec<String> = FIELDS
        .iter()
        .filter_map(|field| {
            let value = text(resolved, field).filter(|v| !v.is_empty())?;
            let shown = if WITHHELD.contains(field) {
                "[withheld]"
            } else {
                value
            };
            Some(format!("{field}={}", Value::String(shown.to_owned())))
        })
        .collect();
    if !entries.is_empty() {
        lines.push(format!("Resolved: {}", entries.join(", ")));
    }
    lines.join("\n")
}

fn code_of(name: &str) -> &'static str {
    chatbotx_core::utils::google_click::google_ads_conversion_error_code(name).unwrap_or("")
}

/// `reportGoogleAdsInputFailure` (`provider: "google-ads"`, no HTTP code).
pub(crate) async fn report(
    deps: &Deps,
    workspace_id: i64,
    contact_id: i64,
    source_id: &str,
    message: String,
) {
    let input = crate::events::ProviderError {
        provider: "google-ads".to_owned(),
        workspace_id: workspace_id.to_string(),
        contact_id: Some(contact_id.to_string()),
        source_id: Some(source_id.to_owned()),
        error: json!({ "message": message }),
        http_code: Some(None),
        ..Default::default()
    };
    crate::events::log_provider_errors(deps.events.as_ref(), &[input]).await;
}

/// `resolveGoogleAdsConversionInputs` + `record` + the config-refusal report.
pub(crate) async fn produce(p: Produce<'_>) -> Produced {
    let ws = p.conversation.workspace_id;
    let contact = p.conversation.contact_id;
    let source_id = p.contact_inbox.source_id.clone();
    let say = |message: String| report(p.deps, ws, contact, &source_id, message);
    let settings = match stored_consent(&p.deps.db, ws).await {
        Ok(Some(settings)) => settings,
        Ok(None) => {
            let code = code_of("invalidConsentConfig");
            say(format!("{code}{}", p.report_suffix)).await;
            return Produced::Refused("invalidConsentConfig");
        }
        Err(error) => return Produced::Failed(error.to_string()),
    };
    // `consentTemplatesOf` + the field templates, resolved in one pass.
    let mut templates = Map::new();
    for field in FIELDS {
        if let Some(value) = text(p.fields, field) {
            templates.insert((*field).to_owned(), json!(value));
        }
    }
    let mut consent_templates = Map::new();
    for setting in ["adUserData", "adPersonalization"] {
        if settings[setting]["type"] == "variable" {
            consent_templates.insert(setting.to_owned(), settings[setting]["template"].clone());
        }
    }
    templates.insert("consent".into(), Value::Object(consent_templates));
    let templates = Value::Object(templates);
    let resolved = match &p.deps.variables {
        Some(variables) => {
            match variables
                .resolve(
                    p.conversation,
                    p.contact_inbox,
                    &templates,
                    None,
                    p.sent_by_user_id.clone(),
                    false,
                )
                .await
            {
                Ok(resolved) => resolved,
                Err(error) => return Produced::Failed(error),
            }
        }
        None => templates,
    };
    let resolved_consent = resolved["consent"].as_object().cloned().unwrap_or_default();
    let consent = match gac::to_consent_input(&settings, &resolved_consent) {
        Ok(consent) => consent,
        Err(setting) => {
            let code = code_of("invalidConsentValue");
            say(format!("{code}\nSetting: {setting}{}", p.report_suffix)).await;
            return Produced::Refused("invalidConsentValue");
        }
    };
    let owned = |key: &str| text(&resolved, key).map(str::to_owned);
    let dedup_mode = text(p.fields, "dedupMode").unwrap_or("id").to_owned();
    let input = RecordInput {
        workspace_id: ws,
        contact_inbox_id: p.contact_inbox.id,
        source: p.source.to_owned(),
        scope_id: p.scope_id.clone(),
        conversion_action_id: text(p.fields, "conversionActionId")
            .unwrap_or_default()
            .to_owned(),
        value: owned("value"),
        currency: owned("currency"),
        recorded_at: now_ms(),
        occurrence_key: p.occurrence_key.clone().filter(|_| dedup_mode == "event"),
        dedup_mode,
        dedup_id: owned("dedupId"),
        conversion_time: owned("conversionTime"),
        consent,
        match_email: text(p.fields, "matchEmail").map(str::to_owned),
        match_phone: text(p.fields, "matchPhone").map(str::to_owned),
        customer_type: owned("customerType"),
        customer_value_bucket: owned("customerValueBucket"),
    };
    match gac::record(&p.deps.db, p.deps.jobs.as_ref(), input).await {
        Ok(Recorded::Queued(_)) => Produced::Queued,
        Ok(Recorded::Refused(status)) => {
            if matches!(
                status,
                "invalidValue" | "missingDedupId" | "invalidDedupId" | "invalidCustomerProperty"
            ) {
                let detail = describe_input_failure(code_of(status), &resolved, &[]);
                say(format!("{detail}{}", p.report_suffix)).await;
            }
            Produced::Refused(status)
        }
        Ok(Recorded::InvalidConversionTime(_)) => {
            let detail = describe_input_failure(code_of("invalidConversionTime"), &resolved, &[]);
            say(format!("{detail}{}", p.report_suffix)).await;
            Produced::Refused("invalidConversionTime")
        }
        Err(error) => Produced::Failed(error.to_string()),
    }
}

/// `handleSendGoogleAdsConversionStep`: `success`, else `error` with the
/// `google_ads_*` code.
pub(crate) async fn step(ctx: &crate::steps::StepCtx<'_, '_>) -> crate::flow::StepStatus {
    let run = ctx.run;
    let step_id = ctx.str("id").unwrap_or_default().to_owned();
    let key = run.flow_execution_key.as_str();
    // `stepOccurrenceKey`: only a durable execution key identifies the run.
    let occurrence_key = crate::integration::is_durable_flow_execution_key(Some(key))
        .then(|| format!("flow:{key}:{}:{step_id}", run.contact_inbox.id));
    let produced = produce(Produce {
        deps: run.deps,
        conversation: &run.conversation,
        contact_inbox: &run.contact_inbox,
        source: "flowStep",
        scope_id: step_id.clone(),
        fields: ctx.step,
        occurrence_key,
        sent_by_user_id: crate::str_of(run.data, "sentByUserId").map(str::to_owned),
        report_suffix: String::new(),
    })
    .await;
    if let Produced::Failed(reason) = &produced {
        tracing::warn!(
            reason,
            workspace_id = run.conversation.workspace_id,
            conversation_id = run.conversation.id,
            contact_inbox_id = run.contact_inbox.id,
            step_id,
            "Failed to record Google Ads conversion flow step"
        );
    }
    match produced.error_code() {
        None => crate::flow::StepStatus::Success,
        Some(code) => crate::steps::step_error(ctx, code),
    }
}

#[cfg(test)]
mod tests {
    //! Port of `apps/worker/__tests__/google-ads-input-error.test.ts`.
    use super::*;

    const KEY: &str = "googleAds.conversionFields.validation.dedupIdRequired";

    /// Port of: "prints each zod issue as a plain `path: message` line under the code"
    #[test]
    fn prints_each_issue_as_a_plain_line_under_the_code() {
        let issues = [format!("a.dedupId: {KEY}"), "whole object".to_owned()];
        let text = describe_input_failure(
            "google_ads_invalid_input",
            &json!({ "value": "5" }),
            &issues,
        );
        assert_eq!(
            text.split('\n').collect::<Vec<_>>(),
            [
                "google_ads_invalid_input",
                &format!("a.dedupId: {KEY}"),
                "whole object",
                "Resolved: value=\"5\"",
            ]
        );
        assert!(!text.contains('✖'));
    }

    /// Port of: "prints only the allowlisted fields, never an extra runtime key"
    #[test]
    fn prints_only_the_allowlisted_fields() {
        let resolved = json!({
            "value": "5", "currency": "USD", "dedupId": "O-1",
            "conversionTime": "2026-10-08T14:30:00+07:00",
            "emailAddress": "customer@example.com", "phoneNumber": "+84901234567",
        });
        let text = describe_input_failure("google_ads_invalid_input", &resolved, &[]);
        assert!(text.contains("value=\"5\""));
        assert!(text.contains("conversionTime=\"2026-10-08T14:30:00+07:00\""));
        assert!(!text.contains("customer@example.com"));
        assert!(!text.contains("+84901234567"));
    }

    /// Port of: "never prints the resolved customer properties" (+ "withholds a customer property that looks like personal data: %s")
    #[test]
    fn never_prints_the_resolved_customer_properties() {
        let long = "a".repeat(64);
        for leaked in [
            "Jane Doe",
            "GOLD",
            "jane@example.com",
            "+84901234567",
            &long,
            "84901234567",
        ] {
            let resolved = json!({ "customerType": leaked, "customerValueBucket": leaked });
            let text =
                describe_input_failure("google_ads_invalid_customer_property", &resolved, &[]);
            assert_eq!(
                text.split('\n').next(),
                Some("google_ads_invalid_customer_property")
            );
            assert!(!text.contains(leaked), "{leaked}");
            assert!(text.contains("customerType=\"[withheld]\""));
            assert!(text.contains("customerValueBucket=\"[withheld]\""));
        }
    }

    #[test]
    fn maps_outcomes_to_error_codes() {
        assert_eq!(Produced::Queued.error_code(), None);
        assert_eq!(
            Produced::Refused("noClick").error_code(),
            Some("google_ads_no_click")
        );
        assert_eq!(
            Produced::Failed("boom".into()).error_code(),
            Some("google_ads_record_failed")
        );
    }
}
