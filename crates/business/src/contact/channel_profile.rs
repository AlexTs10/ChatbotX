//! The channel contact handlers `contact.getProfile`,
//! `getContactProfilePicUrl` and `getProfileSnapshot` (Messenger, Instagram,
//! Facebook-login Instagram, Zalo, Telegram) and `ensureContactAvatarMirrored`
//! / `resolveFreshContactAvatarUrl` of `channel-registry/media-hydration`.

use bytes::Bytes;
use chatbotx_core::utils::image_optimize::ImageOptimizeProfile;
use chatbotx_core::utils::image_optimize_settings::EncodeSlots;
use chatbotx_core::utils::no_avatar_sentinel::{
    NO_AVATAR_SENTINEL_KEY, build_no_avatar_sentinel, is_no_avatar_sentinel_fresh,
    parse_no_avatar_sentinel,
};
use chatbotx_core::utils::store_incoming_image::{
    IncomingImageUploader, PutObjectOptions, StoreIncomingImageInput, store_incoming_image,
};
use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_files::{Acl, PutOptions, Uploader, get_storage_prefix};
use serde_json::Value;

use super::profile_refresh::IncomingProfile;
use crate::channel_api::{self as api, ApiFailure, ChannelHttp};
use crate::{BusinessError, Result, tx};

/// What the handlers need: the database, the provider transport and the
/// avatar store (`None` = avatars are not stored).
#[derive(Clone, Copy)]
pub struct ProfileDeps<'a> {
    pub db: &'a Db,
    pub http: &'a dyn ChannelHttp,
    pub uploader: Option<&'a Uploader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Messenger,
    Instagram,
    InstagramFacebook,
    Zalo,
    Telegram,
}

/// The integration of a contact inbox with a profile handler.
struct ChannelIntegration {
    kind: Kind,
    auth: Value,
}

/// `getProfileSnapshot` (Instagram): the relationship fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileSnapshot {
    pub follows_business: Option<bool>,
    pub business_follows_contact: Option<bool>,
    pub account_verified: Option<bool>,
    pub follower_count: Option<i64>,
    pub username: Option<String>,
}

/// `findByInboxIdForWorkspace` of the channel's integration service:
/// `Ok(None)` = the channel has no profile handler; a missing row is an
/// error ("… integration not found").
async fn integration_for_inbox(
    db: &Db,
    channel: &str,
    workspace_id: i64,
    inbox_id: i64,
) -> std::result::Result<Option<ChannelIntegration>, String> {
    let (table, label, kind_sql) = match channel {
        "messenger" => ("IntegrationMessenger", "Messenger", "'messenger'"),
        "instagram" => ("IntegrationInstagram", "Instagram", "\"type\""),
        "zalo" => ("IntegrationZalo", "Zalo", "'zalo'"),
        "telegram" => ("IntegrationTelegram", "Telegram", "'telegram'"),
        _ => return Ok(None),
    };
    let sql = format!(
        r#"SELECT "auth", {kind_sql} FROM "{table}" WHERE "inboxId" = ?1 AND "workspaceId" = ?2 LIMIT 1"#
    );
    let row: Option<(Option<String>, Option<String>)> = tx::read(db, move |conn| {
        conn.query_row(&sql, params![inbox_id, workspace_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some((auth, kind)) = row else {
        return Err(format!("{label} integration not found"));
    };
    let kind = match (channel, kind.as_deref()) {
        ("messenger", _) => Kind::Messenger,
        ("instagram", Some("facebook")) => Kind::InstagramFacebook,
        ("instagram", _) => Kind::Instagram,
        ("zalo", _) => Kind::Zalo,
        _ => Kind::Telegram,
    };
    Ok(Some(ChannelIntegration {
        kind,
        auth: auth
            .and_then(|a| serde_json::from_str(&a).ok())
            .unwrap_or(Value::Null),
    }))
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn non_empty(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn token(auth: &Value) -> &str {
    auth["tokens"]["accessToken"].as_str().unwrap_or("")
}

fn version(auth: &Value, default: &'static str) -> String {
    non_empty(&auth["metadata"]["version"]).unwrap_or_else(|| default.to_owned())
}

/// `normalizeUtcOffset`: `5.5` → `+05:30`.
pub fn normalize_utc_offset(offset: &Value) -> Option<String> {
    let offset = offset.as_f64().filter(|o| o.is_finite())?;
    let sign = if offset < 0.0 { '-' } else { '+' };
    let abs = offset.abs();
    let hours = abs.trunc();
    let minutes = ((abs - hours) * 60.0).round();
    Some(format!("{sign}{:02}:{:02}", hours as i64, minutes as i64))
}

/// `normalizeGender`: `male` / `female` / `unknown` (any case).
pub fn normalize_gender(gender: Option<&str>) -> Option<String> {
    let lower = gender?.to_lowercase();
    matches!(lower.as_str(), "male" | "female" | "unknown").then_some(lower)
}

struct Putter<'a>(&'a Uploader);

impl IncomingImageUploader for Putter<'_> {
    async fn put_object(
        &self,
        path: &str,
        body: Bytes,
        options: PutObjectOptions,
    ) -> std::result::Result<(), String> {
        let put = PutOptions {
            acl: options.acl.map(|_| Acl::PublicRead),
            content_type: Some(options.content_type),
            content_length: Some(options.content_length as u64),
            ..PutOptions::default()
        };
        self.0
            .put_object(path, body, put, None)
            .await
            .map_err(|e| e.to_string())
    }
}

/// `fetchMediaWithLimits` + `storeIncomingImage` (`avatar`, public-read):
/// the stored key, `None` when the download fails softly or there is no
/// store.
async fn store_avatar(
    deps: ProfileDeps<'_>,
    url: &str,
    bearer: &str,
    key: String,
    fallback_mime: &str,
) -> std::result::Result<Option<String>, String> {
    let Some(uploader) = deps.uploader else {
        return Ok(None);
    };
    let Some((bytes, mime)) = deps
        .http
        .download(url.to_owned(), Some(bearer.to_owned()))
        .await?
    else {
        return Ok(None);
    };
    let mime = mime.unwrap_or_else(|| fallback_mime.to_owned());
    let putter = Putter(uploader);
    let stored = store_incoming_image(
        StoreIncomingImageInput {
            uploader: Some(&putter),
            key: &key,
            bytes,
            mime_type: &mime,
            profile: ImageOptimizeProfile::Avatar,
            acl: Some("public-read"),
        },
        &chatbotx_core::env::ProcessEnv,
        &EncodeSlots::global(),
    )
    .await?;
    Ok(Some(stored.origin_path))
}

/// A best-effort avatar (`getContactProfilePicture`: errors are logged).
async fn soft_avatar(
    deps: ProfileDeps<'_>,
    url: &str,
    bearer: &str,
    key: String,
) -> Option<String> {
    match store_avatar(deps, url, bearer, key, "application/octet-stream").await {
        Ok(avatar) => avatar,
        Err(error) => {
            tracing::error!(%error, "getContactProfilePicture error");
            None
        }
    }
}

fn avatar_key(workspace_id: i64, messenger: bool) -> String {
    let prefix = get_storage_prefix(&workspace_id.to_string());
    let id = chatbotx_core::utils::id::create_id();
    if messenger {
        format!("public/space/{prefix}/avatars/{id}")
    } else {
        format!("{prefix}/avatars/{id}")
    }
}

const MESSENGER_FIELDS: &str = "first_name,last_name,profile_pic,locale,timezone,gender";
const PUBLIC_PICTURE_FIELD: &str = "picture.height(480).width(480){url,is_silhouette}";

/// `fetchProfileWithPublicFallback`: the Messenger User Profile; when it
/// fails or has no `profile_pic`, the public profile fills in the name and
/// the picture (a silhouette is no picture).
async fn messenger_lookup(
    http: &dyn ChannelHttp,
    auth: &Value,
    psid: &str,
) -> std::result::Result<(Value, Option<String>), ApiFailure> {
    let path = format!("{}/{psid}", version(auth, api::META_DEFAULT_VERSION));
    let bearer = Some(token(auth));
    let messenger = api::graph_get(
        http,
        api::GRAPH_API_URL,
        &path,
        &[("fields", MESSENGER_FIELDS)],
        bearer,
    )
    .await;
    if let Ok(profile) = &messenger {
        if let Some(picture) = non_empty(&profile["profile_pic"]) {
            return Ok((profile.clone(), Some(picture)));
        }
    }
    let user_fields = format!("first_name,last_name,name,{PUBLIC_PICTURE_FIELD}");
    let mut public = api::graph_get(
        http,
        api::GRAPH_API_URL,
        &path,
        &[("fields", &user_fields)],
        bearer,
    )
    .await;
    if let Err(failure) = &public {
        if failure.code == Some(100) && failure.message.contains("node type (Page)") {
            let page_fields = format!("name,{PUBLIC_PICTURE_FIELD}");
            public = api::graph_get(
                http,
                api::GRAPH_API_URL,
                &path,
                &[("fields", &page_fields)],
                bearer,
            )
            .await;
        }
    }
    let public = match public {
        Ok(public) => public,
        Err(failure) => {
            tracing::warn!(err = %failure, source_id = psid, "fetchPublicUserProfile error");
            return messenger.map(|profile| (profile, None));
        }
    };
    let mut profile = messenger.unwrap_or_else(|_| Value::Object(Default::default()));
    let first = text(&profile["first_name"])
        .or_else(|| text(&public["first_name"]))
        .or_else(|| text(&public["name"]));
    let last = text(&profile["last_name"]).or_else(|| text(&public["last_name"]));
    profile["first_name"] = first.map_or(Value::Null, Value::String);
    profile["last_name"] = last.map_or(Value::Null, Value::String);
    let data = &public["picture"]["data"];
    let picture = if data["is_silhouette"].as_bool() == Some(true) {
        None
    } else {
        text(&data["url"])
    };
    Ok((profile, picture))
}

/// Facebook-login Instagram `fetchProfileFields`: `profile_pic` is dropped
/// on a "nonexisting field" error.
async fn instagram_facebook_lookup(
    http: &dyn ChannelHttp,
    auth: &Value,
    psid: &str,
    with_picture: bool,
) -> std::result::Result<Value, ApiFailure> {
    let path = format!("{}/{psid}", version(auth, api::META_DEFAULT_VERSION));
    let fields = if with_picture {
        "name,username,profile_pic"
    } else {
        "name,username"
    };
    let answer = api::graph_get(
        http,
        api::GRAPH_API_URL,
        &path,
        &[("fields", fields)],
        Some(token(auth)),
    )
    .await;
    match answer {
        Err(failure)
            if with_picture
                && failure.code == Some(100)
                && failure.message.contains("nonexisting field") =>
        {
            tracing::warn!(
                source_id = psid,
                "getUserProfile: profile_pic unavailable, retrying without it"
            );
            api::graph_get(
                http,
                api::GRAPH_API_URL,
                &path,
                &[("fields", "name,username")],
                Some(token(auth)),
            )
            .await
        }
        other => other,
    }
}

async fn instagram_lookup(
    http: &dyn ChannelHttp,
    auth: &Value,
    psid: &str,
) -> std::result::Result<Value, ApiFailure> {
    let path = format!("{}/{psid}", version(auth, api::INSTAGRAM_DEFAULT_VERSION));
    api::graph_get(
        http,
        api::INSTAGRAM_API_URL,
        &path,
        &[("fields", "id,name,username,profile_pic")],
        Some(token(auth)),
    )
    .await
}

fn zalo_gender(gender: &Value) -> Option<String> {
    match gender {
        Value::Number(n) if n.as_f64() == Some(1.0) => Some("male".into()),
        Value::Number(n) if n.as_f64() == Some(2.0) => Some("female".into()),
        Value::String(s) if s == "1" => Some("male".into()),
        Value::String(s) if s == "2" => Some("female".into()),
        _ => None,
    }
}

async fn telegram_avatar(
    http: &dyn ChannelHttp,
    bot: &str,
    psid: &str,
) -> std::result::Result<Option<String>, ApiFailure> {
    let photos = api::telegram_call(
        http,
        bot,
        "getUserProfilePhotos",
        &[("user_id", psid), ("limit", "1")],
    )
    .await;
    let Some(file_id) = photos
        .ok()
        .and_then(|p| text(&p["photos"][0][0]["file_id"]))
    else {
        return Ok(None);
    };
    match api::telegram_call(http, bot, "getFile", &[("file_id", &file_id)]).await {
        Ok(file) => Ok(non_empty(&file["file_path"])
            .map(|path| format!("{}/file/bot{bot}/{path}", api::TELEGRAM_API_URL))),
        Err(failure) if failure.status == Some(404) => Ok(None),
        Err(failure) => Err(failure),
    }
}

/// `contact.getProfile` of the contact inbox's channel. An unsupported
/// channel, a missing integration or a provider failure is an error.
pub async fn fetch_profile(
    deps: ProfileDeps<'_>,
    workspace_id: i64,
    channel: &str,
    inbox_id: i64,
    source_id: &str,
    include_avatar: bool,
) -> std::result::Result<IncomingProfile, String> {
    let Some(integration) = integration_for_inbox(deps.db, channel, workspace_id, inbox_id).await?
    else {
        return Err(format!(
            "Channel \"{channel}\" has no contact.getProfile handler"
        ));
    };
    let auth = &integration.auth;
    let mut profile = IncomingProfile {
        source_id: source_id.to_owned(),
        ..IncomingProfile::default()
    };
    match integration.kind {
        Kind::Messenger => {
            let (raw, picture) = messenger_lookup(deps.http, auth, source_id)
                .await
                .map_err(|f| f.message)?;
            profile.first_name = text(&raw["first_name"]);
            profile.last_name = text(&raw["last_name"]);
            profile.locale = text(&raw["locale"]);
            profile.timezone = normalize_utc_offset(&raw["timezone"]);
            profile.gender = normalize_gender(raw["gender"].as_str());
            if let (true, Some(picture)) = (include_avatar, picture) {
                profile.avatar =
                    soft_avatar(deps, &picture, token(auth), avatar_key(workspace_id, true)).await;
            }
        }
        Kind::Instagram | Kind::InstagramFacebook => {
            let raw = if integration.kind == Kind::Instagram {
                instagram_lookup(deps.http, auth, source_id).await
            } else {
                instagram_facebook_lookup(deps.http, auth, source_id, include_avatar).await
            }
            .map_err(|f| f.message)?;
            profile.first_name = text(&raw["name"]);
            if let (true, Some(picture)) = (include_avatar, non_empty(&raw["profile_pic"])) {
                profile.avatar =
                    soft_avatar(deps, &picture, token(auth), avatar_key(workspace_id, false)).await;
            }
        }
        Kind::Zalo => {
            let detail = api::zalo_user_detail(deps.http, token(auth), source_id)
                .await
                .map_err(|f| format!("Get user profile failed: {}", f.message))?;
            profile.first_name = Some(non_empty(&detail["display_name"]).unwrap_or_default());
            profile.gender = zalo_gender(&detail["user_gender"]);
            if let (true, Some(url)) = (include_avatar, non_empty(&detail["avatar"])) {
                let key = format!(
                    "{}/{}",
                    get_storage_prefix(&workspace_id.to_string()),
                    chatbotx_core::utils::id::create_id()
                );
                profile.avatar = store_avatar(deps, &url, token(auth), key, "image/png").await?;
            }
        }
        Kind::Telegram => {
            let bot = auth["secretText"].as_str().unwrap_or("");
            let chat = api::telegram_call(deps.http, bot, "getChat", &[("chat_id", source_id)])
                .await
                .map_err(|f| f.message)?;
            profile.first_name = text(&chat["first_name"]);
            profile.last_name = text(&chat["last_name"]);
            if chat["username"].as_str().is_some_and(|u| !u.is_empty()) {
                profile.avatar = telegram_avatar(deps.http, bot, source_id)
                    .await
                    .map_err(|f| f.message)?;
            }
        }
    }
    Ok(profile)
}

/// `contact.getContactProfilePicUrl` (Messenger, both Instagram logins):
/// `Ok(None)` for another channel or no picture.
pub async fn fetch_profile_pic_url(
    db: &Db,
    http: &dyn ChannelHttp,
    workspace_id: i64,
    channel: &str,
    inbox_id: i64,
    source_id: &str,
) -> std::result::Result<Option<String>, ProfileError> {
    let integration = integration_for_inbox(db, channel, workspace_id, inbox_id)
        .await
        .map_err(ProfileError::Integration)?;
    let Some(integration) = integration else {
        return Ok(None);
    };
    let auth = &integration.auth;
    let picture = match integration.kind {
        Kind::Messenger => messenger_lookup(http, auth, source_id)
            .await
            .map(|(_, p)| p),
        Kind::Instagram => instagram_lookup(http, auth, source_id)
            .await
            .map(|raw| text(&raw["profile_pic"])),
        Kind::InstagramFacebook => instagram_facebook_lookup(http, auth, source_id, true)
            .await
            .map(|raw| text(&raw["profile_pic"])),
        Kind::Zalo | Kind::Telegram => return Ok(None),
    };
    picture.map_err(ProfileError::Provider)
}

/// A handler failure: the integration could not be resolved, or the
/// provider call failed (an `IntegrationException`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    Integration(String),
    Provider(ApiFailure),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::Integration(message) => f.write_str(message),
            ProfileError::Provider(failure) => f.write_str(&failure.message),
        }
    }
}

/// `contact.getProfileSnapshot` (both Instagram logins): `Ok(None)` = the
/// channel has no snapshot handler.
pub async fn fetch_profile_snapshot(
    db: &Db,
    http: &dyn ChannelHttp,
    workspace_id: i64,
    channel: &str,
    inbox_id: i64,
    source_id: &str,
) -> std::result::Result<Option<ProfileSnapshot>, ProfileError> {
    if channel != "instagram" {
        return Ok(None);
    }
    let integration = integration_for_inbox(db, channel, workspace_id, inbox_id)
        .await
        .map_err(ProfileError::Integration)?
        .ok_or_else(|| ProfileError::Integration("Instagram integration not found".into()))?;
    let auth = &integration.auth;
    let (base, default) = if integration.kind == Kind::InstagramFacebook {
        (api::GRAPH_API_URL, api::META_DEFAULT_VERSION)
    } else {
        (api::INSTAGRAM_API_URL, api::INSTAGRAM_DEFAULT_VERSION)
    };
    let raw = api::graph_get(
        http,
        base,
        &format!("{}/{source_id}", version(auth, default)),
        &[(
            "fields",
            "username,follower_count,is_user_follow_business,is_business_follow_user,is_verified_user",
        )],
        Some(token(auth)),
    )
    .await
    .map_err(ProfileError::Provider)?;
    Ok(Some(ProfileSnapshot {
        follows_business: raw["is_user_follow_business"].as_bool(),
        business_follows_contact: raw["is_business_follow_user"].as_bool(),
        account_verified: raw["is_verified_user"].as_bool(),
        follower_count: raw["follower_count"].as_i64(),
        username: text(&raw["username"]),
    }))
}

/// A contact inbox row: `(contactId, channel, inboxId, sourceId)`.
async fn contact_inbox(
    db: &Db,
    contact_inbox_id: i64,
) -> Result<Option<(i64, String, i64, String)>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "contactId", "channel", "inboxId", "sourceId" FROM "ContactInbox" WHERE "id" = ?1"#,
            [contact_inbox_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
}

async fn contact_avatar(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<Option<String>>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "avatar" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![contact_id, workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
}

/// `contactService.setAvatarIfEmptyOrSentinel`: written only over no avatar
/// or a no-avatar sentinel.
// ponytail: the stored-image optimization job of the original is not
// enqueued (avatars are already encoded with the `avatar` profile on store).
pub async fn set_avatar_if_empty_or_sentinel(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    avatar: String,
) -> Result<bool> {
    let pattern = format!("{}?time=%", NO_AVATAR_SENTINEL_KEY.replace('_', "\\_"));
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "avatar" = ?3, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2
                 AND ("avatar" IS NULL OR "avatar" LIKE ?5 ESCAPE '\')"#,
            params![contact_id, workspace_id, avatar, repo::now_ms(), pattern],
        )
        .map(|n| n > 0)
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
}

/// `ensureContactAvatarMirrored`: a real avatar or a fresh sentinel is kept;
/// else the channel profile's avatar (or a new sentinel when it has none or
/// the fetch fails) is stored over an empty / sentinel avatar. `None` = no
/// contact inbox or contact.
pub async fn ensure_contact_avatar_mirrored(
    deps: ProfileDeps<'_>,
    workspace_id: i64,
    contact_inbox_id: i64,
) -> Option<String> {
    let now = repo::now_ms();
    let loaded = async {
        let Some((contact_id, channel, inbox_id, source_id)) =
            contact_inbox(deps.db, contact_inbox_id).await?
        else {
            return Ok(None);
        };
        Ok::<_, BusinessError>(
            contact_avatar(deps.db, workspace_id, contact_id)
                .await?
                .map(|avatar| (contact_id, avatar, channel, inbox_id, source_id)),
        )
    }
    .await;
    let (contact_id, avatar, channel, inbox_id, source_id) = match loaded {
        Ok(Some(found)) => found,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, contact_inbox_id, workspace_id, "Contact avatar mirror failed before persistence; returning no-avatar sentinel");
            return Some(build_no_avatar_sentinel(now));
        }
    };
    if let Some(avatar) = avatar {
        match parse_no_avatar_sentinel(&avatar) {
            None => return Some(avatar),
            Some(failed_at) if is_no_avatar_sentinel_fresh(failed_at, now) => return Some(avatar),
            Some(_) => {}
        }
    }
    let avatar = match fetch_profile(deps, workspace_id, &channel, inbox_id, &source_id, true).await
    {
        Ok(profile) => profile
            .avatar
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| build_no_avatar_sentinel(now)),
        Err(error) => {
            tracing::warn!(%error, contact_inbox_id, workspace_id, "Contact avatar mirror failed; storing no-avatar sentinel");
            build_no_avatar_sentinel(now)
        }
    };
    if let Err(error) =
        set_avatar_if_empty_or_sentinel(deps.db, workspace_id, contact_id, avatar.clone()).await
    {
        tracing::warn!(%error, contact_inbox_id, workspace_id, "Failed to persist mirrored avatar");
    }
    Some(avatar)
}

/// `resolveFreshContactAvatarUrl`: the channel's current picture URL;
/// `Ok(None)` without a contact inbox, picture or handler, or when the
/// provider call fails (logged). A missing integration is an error.
pub async fn resolve_fresh_contact_avatar_url(
    db: &Db,
    http: &dyn ChannelHttp,
    workspace_id: i64,
    contact_inbox_id: i64,
) -> std::result::Result<Option<String>, String> {
    let Some((_, channel, inbox_id, source_id)) = contact_inbox(db, contact_inbox_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    match fetch_profile_pic_url(db, http, workspace_id, &channel, inbox_id, &source_id).await {
        Ok(url) => Ok(url),
        Err(ProfileError::Provider(failure)) => {
            tracing::warn!(err = %failure, contact_inbox_id, "Failed to resolve fresh contact avatar URL");
            Ok(None)
        }
        Err(ProfileError::Integration(message)) => Err(message),
    }
}
