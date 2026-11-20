//! `GET /integrations/whatsapp/callback`
//! (`apps/builder/src/app/integrations/whatsapp/callback/route.ts`): the
//! broker callback of the WhatsApp embedded signup. It relays Meta's `code`
//! to the tab that opened the popup (`window.opener.postMessage`) when the
//! `referer` of the state is an origin we control (`sanitizeReferer` of
//! `lib/oauth-referer.ts`), then closes the popup. No token exchange here.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use chatbotx_business::tx;
use chatbotx_db::rusqlite::OptionalExtension as _;

use crate::handlers::ApiState;
use crate::handlers::messenger_templates::broker_origin;

/// `FALLBACK_REDIRECT`.
pub const FALLBACK_REDIRECT: &str = "/manage";

/// `decodeOAuthState`: base64 JSON `{ referer, locale? }` (`atob`, so the
/// bytes are Latin-1 characters).
pub fn decode_state(raw: &str) -> Option<(String, Option<String>)> {
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let compact: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = engine.decode(compact).ok()?;
    let text: String = bytes.into_iter().map(char::from).collect();
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let referer = parsed.get("referer")?.as_str()?.to_owned();
    let locale = parsed
        .get("locale")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    Some((referer, locale))
}

/// `isAllowedOrigin`: the broker origin, the builder origin, or an active
/// custom domain.
pub async fn is_allowed_origin(state: &ApiState, target: &url::Url) -> bool {
    let origin = target.origin().ascii_serialization();
    let builder = url::Url::parse(&state.app_url)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    if origin == broker_origin(&state.app_url) || origin == builder {
        return true;
    }
    let Some(host) = target.host_str().map(str::to_owned) else {
        return false;
    };
    tx::read(&state.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT 1 FROM "CustomDomain" WHERE "domain" = ?1 AND "status" = 'active'"#,
                [host],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    })
    .await
    .unwrap_or_else(|error| {
        tracing::error!(%error, "custom domain lookup failed");
        false
    })
}

/// `sanitizeReferer`: the referer when it is an allowed origin, else
/// [`FALLBACK_REDIRECT`].
pub async fn sanitize_referer(state: &ApiState, referer: &str) -> String {
    match url::Url::parse(referer) {
        Ok(parsed) if is_allowed_origin(state, &parsed).await => referer.to_owned(),
        _ => FALLBACK_REDIRECT.to_owned(),
    }
}

/// The relay result and the message index (done, cancelled, failed) of
/// `whatsappCallbackOutcome`: only `access_denied` is a cancel.
pub fn outcome(code: Option<&str>, error: Option<&str>) -> (String, usize) {
    if error == Some("access_denied") {
        return (
            r#"{"type":"WA_OAUTH_RESULT","status":"cancelled"}"#.into(),
            1,
        );
    }
    match code.filter(|c| !c.is_empty()) {
        Some(code) if error.is_none_or(str::is_empty) => (
            format!(
                r#"{{"type":"WA_OAUTH_RESULT","status":"success","code":{}}}"#,
                serde_json::Value::from(code)
            ),
            0,
        ),
        _ => (r#"{"type":"WA_OAUTH_RESULT","status":"error"}"#.into(), 2),
    }
}

/// `safeJson`: no `<`, U+2028 or U+2029 inside the inline script.
pub fn safe_json(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// `escapeHtml`.
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `whatsapp.embeddedSignup{Done,Cancelled,Failed}` of `apps/builder/messages`.
const MESSAGES: &[(&str, [&str; 3])] = &[
    (
        "ar",
        [
            "اكتمل التسجيل — يمكنك إغلاق علامة التبويب هذه.",
            "تم إلغاء التسجيل — يمكنك إغلاق علامة التبويب هذه.",
            "لم يكتمل التسجيل — أغلق علامة التبويب هذه وحاول مرة أخرى.",
        ],
    ),
    (
        "az",
        [
            "Qeydiyyat tamamlandı. Bu vərəqi bağlaya bilərsiniz.",
            "Qeydiyyat ləğv edildi — bu tabı bağlaya bilərsiniz.",
            "Qeydiyyat tamamlanmadı — bu tabı bağlayın və yenidən cəhd edin.",
        ],
    ),
    (
        "da",
        [
            "Signup complete — du kan close denne tab.",
            "Tilmeldingen blev annulleret — du kan lukke denne fane.",
            "Tilmeldingen blev ikke gennemført — luk denne fane, og prøv igen.",
        ],
    ),
    (
        "de",
        [
            "Registrierung abgeschlossen — Sie können diesen Tab schließen.",
            "Die Registrierung wurde abgebrochen – du kannst diesen Tab schließen.",
            "Die Registrierung wurde nicht abgeschlossen – schließe diesen Tab und versuche es erneut.",
        ],
    ),
    (
        "en",
        [
            "Signup complete — you can close this tab.",
            "Signup was cancelled — you can close this tab.",
            "Signup didn't finish — close this tab and try again.",
        ],
    ),
    (
        "es",
        [
            "Signup complete — tú puede Cerrar este tab.",
            "Se canceló el registro: puedes cerrar esta pestaña.",
            "El registro no se completó: cierra esta pestaña e inténtalo de nuevo.",
        ],
    ),
    (
        "fi",
        [
            "Rekisteröinti valmis — voit sulkea tämän välilehden.",
            "Rekisteröityminen peruttiin – voit sulkea tämän välilehden.",
            "Rekisteröityminen ei valmistunut – sulje tämä välilehti ja yritä uudelleen.",
        ],
    ),
    (
        "fr",
        [
            "Inscription terminée — vous pouvez fermer cet onglet.",
            "L'inscription a été annulée — vous pouvez fermer cet onglet.",
            "L'inscription n'a pas abouti — fermez cet onglet et réessayez.",
        ],
    ),
    (
        "he",
        [
            "ההרשמה הושלמה — ניתן לסגור כרטיסייה זו.",
            "ההרשמה בוטלה — אפשר לסגור את הכרטיסייה.",
            "ההרשמה לא הושלמה — סגרו את הכרטיסייה ונסו שוב.",
        ],
    ),
    (
        "id",
        [
            "Pendaftaran selesai — Anda dapat menutup tab ini.",
            "Pendaftaran dibatalkan — Anda dapat menutup tab ini.",
            "Pendaftaran belum selesai — tutup tab ini dan coba lagi.",
        ],
    ),
    (
        "it",
        [
            "Registrazione completata: puoi chiudere questa scheda.",
            "La registrazione è stata annullata: puoi chiudere questa scheda.",
            "La registrazione non è stata completata: chiudi questa scheda e riprova.",
        ],
    ),
    (
        "ja",
        [
            "登録が完了しました。このタブを閉じてください。",
            "登録はキャンセルされました。このタブを閉じてください。",
            "登録が完了しませんでした。このタブを閉じて、もう一度お試しください。",
        ],
    ),
    (
        "nl",
        [
            "Registratie voltooid — je kunt dit tabblad sluiten.",
            "De registratie is geannuleerd — je kunt dit tabblad sluiten.",
            "De registratie is niet voltooid — sluit dit tabblad en probeer het opnieuw.",
        ],
    ),
    (
        "pt-BR",
        [
            "Cadastro concluído — você pode fechar esta aba.",
            "O cadastro foi cancelado — você pode fechar esta aba.",
            "O cadastro não foi concluído — feche esta aba e tente novamente.",
        ],
    ),
    (
        "pt-PT",
        [
            "Registo concluído — pode fechar este separador.",
            "O registo foi cancelado — pode fechar este separador.",
            "O registo não foi concluído — feche este separador e tente novamente.",
        ],
    ),
    (
        "ro",
        [
            "Înregistrare completă — poți închide această filă.",
            "Înregistrarea a fost anulată — poți închide această filă.",
            "Înregistrarea nu s-a finalizat — închide această filă și încearcă din nou.",
        ],
    ),
    (
        "sv",
        [
            "Registreringen är klar – du kan stänga den här fliken.",
            "Registreringen avbröts – du kan stänga den här fliken.",
            "Registreringen slutfördes inte – stäng den här fliken och försök igen.",
        ],
    ),
    (
        "tr",
        [
            "Kayıt tamamlandı — bu sekmeyi kapatabilirsiniz.",
            "Kayıt iptal edildi — bu sekmeyi kapatabilirsiniz.",
            "Kayıt tamamlanmadı — bu sekmeyi kapatıp tekrar deneyin.",
        ],
    ),
    (
        "vi",
        [
            "Đã hoàn tất đăng ký — bạn có thể đóng tab này.",
            "Đã huỷ đăng ký — bạn có thể đóng tab này.",
            "Đăng ký chưa hoàn tất — hãy đóng tab này và thử lại.",
        ],
    ),
    (
        "zh-CN",
        [
            "注册完成 — 您可以关闭此标签。",
            "注册已取消——你可以关闭此标签页。",
            "注册未完成——请关闭此标签页并重试。",
        ],
    ),
    (
        "zh-TW",
        [
            "註冊完成 — 您可以關閉此標籤。",
            "註冊已取消——你可以關閉此分頁。",
            "註冊未完成——請關閉此分頁並再試一次。",
        ],
    ),
];

fn message(locale: Option<&str>, index: usize) -> &'static str {
    let pick = |l: &str| {
        MESSAGES
            .iter()
            .find(|(k, _)| *k == l)
            .map(|(_, m)| m[index])
    };
    locale
        .and_then(pick)
        .or_else(|| pick("en"))
        .unwrap_or_default()
}

/// `relayHtml`.
pub fn relay_html(result: &str, target_origin: &str, message: &str) -> String {
    let result = safe_json(result);
    let target = safe_json(&serde_json::Value::from(target_origin).to_string());
    let message = escape_html(message);
    format!(
        r#"<!doctype html>
<html lang="en">
  <head><meta charset="utf-8" /><title>WhatsApp</title></head>
  <body style="font-family: system-ui, sans-serif; padding: 24px; text-align: center;">
    <p>{message}</p>
    <script>
      (function () {{
        try {{
          if (window.opener) {{
            window.opener.postMessage({result}, {target});
          }}
        }} catch (e) {{}}
        window.setTimeout(function () {{ window.close(); }}, 300);
      }})();
    </script>
  </body>
</html>"#
    )
}

fn invalid_request() -> Response {
    (StatusCode::BAD_REQUEST, "Invalid request").into_response()
}

/// The route handler.
pub async fn handle(State(state): State<Arc<ApiState>>, uri: Uri) -> Response {
    let mut code = None;
    let mut error = None;
    let mut raw_state = None;
    for (key, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        let slot = match key.as_ref() {
            "code" => &mut code,
            "error" => &mut error,
            "state" => &mut raw_state,
            _ => continue,
        };
        // `searchParams.get`: the first value.
        if slot.is_none() {
            *slot = Some(value.into_owned());
        }
    }
    let Some((referer, locale)) = decode_state(raw_state.as_deref().unwrap_or_default()) else {
        tracing::warn!("[wa-oauth-callback] missing or invalid state");
        return invalid_request();
    };
    let safe = sanitize_referer(&state, &referer).await;
    if !safe.starts_with("http") {
        tracing::warn!(%referer, "[wa-oauth-callback] rejected relay target");
        return invalid_request();
    }
    let target_origin = url::Url::parse(&safe)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    let (result, index) = outcome(code.as_deref(), error.as_deref());
    let html = relay_html(&result, &target_origin, message(locale.as_deref(), index));
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-ancestors 'none'",
            ),
        ],
        html,
    )
        .into_response()
}
