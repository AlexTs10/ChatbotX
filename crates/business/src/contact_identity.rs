//! Scoped identity upkeep of a contact inbox (`contact-inbox/identity-rotation.ts`,
//! the identity half of `contactInboxService` and
//! `syncExistingContactIdentity`, `contactService.adoptPhoneNumberIfSafe`):
//! a returning contact's newly learned ids, WhatsApp `user_changed_user_id`
//! rotations and phone changes. Every function runs on the caller's write
//! transaction. The tracking cache tags of the original have no Rust cache.

use chatbotx_db::repo::contact_inbox::{
    IdentityFields, IdentityHistoryAppend, WithContact, find_with_contact, update_identity_guarded,
};
use chatbotx_db::repo::models::ContactInbox;
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{Model as _, iso, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};

use crate::{BusinessError, Result};

fn repo_err(error: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// The identity fields of an `IncomingContact`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncomingIdentity {
    pub source_id: String,
    pub source_user_id: Option<String>,
    pub source_parent_user_id: Option<String>,
    pub source_username: Option<String>,
}

/// `ContactInboxPhoneTransition`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneTransition {
    pub previous_phone: Option<String>,
    pub new_phone: String,
}

/// A contact phone write: `(before, after)`, for the contact-info events.
pub type PhoneAdoption = (Option<String>, String);

fn blank(value: Option<&str>) -> bool {
    value.is_none_or(|v| v.is_empty())
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// `isDistinctPrimaryIdentity`.
pub fn is_distinct_primary_identity(value: Option<&str>, scoped: &[Option<&str>]) -> bool {
    let Some(primary) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return false;
    };
    scoped.iter().all(|id| id.map(str::trim) != Some(primary))
}

/// `isSourceUserIdKeyedIdentity`.
fn is_user_id_keyed(source_id: &str, source_user_id: Option<&str>) -> bool {
    !blank(source_user_id) && Some(source_id) == source_user_id
}

/// `resolvePhoneTransition`.
pub fn resolve_phone_transition(
    previous_phone: Option<&str>,
    new_phone: Option<&str>,
    scoped: &[Option<&str>],
) -> Option<PhoneTransition> {
    let new_phone = new_phone.filter(|p| !p.is_empty())?;
    if !is_distinct_primary_identity(Some(new_phone), scoped) || Some(new_phone) == previous_phone {
        return None;
    }
    Some(PhoneTransition {
        previous_phone: previous_phone.map(str::to_owned),
        new_phone: new_phone.to_owned(),
    })
}

/// `ContactInboxIdentitySet` / guard with plain values (`None` = absent).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Identity {
    source_id: Option<String>,
    source_user_id: Option<String>,
    source_parent_user_id: Option<String>,
}

impl Identity {
    fn is_empty(&self) -> bool {
        self.source_id.is_none()
            && self.source_user_id.is_none()
            && self.source_parent_user_id.is_none()
    }

    fn fields(&self) -> IdentityFields {
        IdentityFields {
            source_id: self.source_id.clone().map(Some),
            source_user_id: self.source_user_id.clone().map(Some),
            source_parent_user_id: self.source_parent_user_id.clone().map(Some),
        }
    }
}

/// `resolveRotationSet`: the target fields that differ from the row.
fn rotation_set(row: &ContactInbox, target: &Identity) -> Identity {
    let differs = |target: &Option<String>, current: Option<&str>| {
        target.clone().filter(|t| Some(t.as_str()) != current)
    };
    Identity {
        source_id: differs(&target.source_id, Some(row.source_id.as_str())),
        source_user_id: differs(&target.source_user_id, row.source_user_id.as_deref()),
        source_parent_user_id: differs(
            &target.source_parent_user_id,
            row.source_parent_user_id.as_deref(),
        ),
    }
}

/// `resolveRotationGuard`: the observed match, plus the current value of
/// every field the set rewrites (a null parent stays unguarded).
fn rotation_guard(row: &ContactInbox, field: &str, value: &str, set: &Identity) -> IdentityFields {
    let mut guard = IdentityFields::default();
    match field {
        "sourceId" => guard.source_id = Some(Some(value.to_owned())),
        "sourceUserId" => guard.source_user_id = Some(Some(value.to_owned())),
        _ => guard.source_parent_user_id = Some(Some(value.to_owned())),
    }
    if set.source_id.is_some() {
        guard.source_id = Some(Some(row.source_id.clone()));
    }
    if set.source_user_id.is_some() {
        guard.source_user_id = Some(row.source_user_id.clone());
    }
    if set.source_parent_user_id.is_some() && row.source_parent_user_id.is_some() {
        guard.source_parent_user_id = Some(row.source_parent_user_id.clone());
    }
    guard
}

/// `RotationPlan` without `stale`.
enum Plan {
    Apply {
        guard: IdentityFields,
        set: Identity,
        transition: Option<PhoneTransition>,
    },
    AlreadyApplied(Option<PhoneTransition>),
}

struct PhoneObservation<'a> {
    previous_phone: Option<&'a str>,
    new_phone: Option<&'a str>,
    scoped: Vec<Option<&'a str>>,
    use_new_phone_as_source_id: bool,
}

/// `resolveIdentityTargetStatePlan`.
fn target_state_plan(
    row: &ContactInbox,
    observed: (&str, &str),
    mut target: Identity,
    phone: PhoneObservation<'_>,
) -> Plan {
    let transition = resolve_phone_transition(phone.previous_phone, phone.new_phone, &phone.scoped);
    if let (Some(t), true) = (&transition, phone.use_new_phone_as_source_id) {
        target.source_id = Some(t.new_phone.clone());
    }
    let set = rotation_set(row, &target);
    if set.is_empty() {
        return Plan::AlreadyApplied(transition);
    }
    Plan::Apply {
        guard: rotation_guard(row, observed.0, observed.1, &set),
        set,
        transition,
    }
}

/// `resolveParentFallbackRotationPlan`.
fn parent_fallback_plan(row: &ContactInbox, incoming: &IncomingIdentity, user_id: &str) -> Plan {
    let keyed =
        row.source_id.is_empty() || is_user_id_keyed(&row.source_id, row.source_user_id.as_deref());
    target_state_plan(
        row,
        (
            "sourceParentUserId",
            row.source_parent_user_id.as_deref().unwrap_or(""),
        ),
        Identity {
            source_id: if keyed {
                Some(user_id.to_owned())
            } else {
                Some(incoming.source_id.clone()).filter(|s| !s.is_empty())
            },
            source_user_id: Some(user_id.to_owned()),
            source_parent_user_id: None,
        },
        PhoneObservation {
            previous_phone: (!keyed).then_some(row.source_id.as_str()),
            new_phone: Some(incoming.source_id.as_str()),
            scoped: vec![Some(user_id), row.source_user_id.as_deref()],
            use_new_phone_as_source_id: false,
        },
    )
}

/// The outcome of a guarded identity write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStatus {
    Applied,
    Stale,
    Conflict,
}

fn is_unique_violation(error: &chatbotx_db::repo::Error) -> bool {
    error.to_string().contains("UNIQUE constraint failed")
}

fn find_row(conn: &Connection, id: i64) -> Result<Option<ContactInbox>> {
    conn.query_row(
        &format!(r#"{} WHERE "id" = ?1"#, ContactInbox::select_sql()),
        [id],
        ContactInbox::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// `applyGuardedIdentityUpdate`: the CAS write; a stale guard re-reads the
/// row, a unique conflict keeps it.
fn guarded_update(
    conn: &Connection,
    row: &ContactInbox,
    guard: &IdentityFields,
    set: &Identity,
    reason: &str,
    conflict_message: &str,
) -> Result<(ContactInbox, WriteStatus)> {
    let set = rotation_set(row, set);
    if set.is_empty() {
        return Ok((row.clone(), WriteStatus::Applied));
    }
    // `shouldAppendContactInboxIdentityHistory`.
    let append = [
        (Some(row.source_id.as_str()), &set.source_id),
        (row.source_user_id.as_deref(), &set.source_user_id),
        (
            row.source_parent_user_id.as_deref(),
            &set.source_parent_user_id,
        ),
    ]
    .iter()
    .any(|(prev, next)| !blank(*prev) && next.as_deref().is_some_and(|n| Some(n) != *prev))
    .then(|| IdentityHistoryAppend {
        changed_at: iso(now_ms()),
        reason: reason.to_owned(),
    });
    match update_identity_guarded(conn, row.id, guard, &set.fields(), append.as_ref()) {
        Ok(Some(updated)) => Ok((updated, WriteStatus::Applied)),
        Ok(None) => Ok((
            find_row(conn, row.id)?.unwrap_or_else(|| row.clone()),
            WriteStatus::Stale,
        )),
        Err(error) if is_unique_violation(&error) => {
            tracing::warn!(%error, contact_inbox_id = row.id, inbox_id = row.inbox_id, "{conflict_message}");
            Ok((row.clone(), WriteStatus::Conflict))
        }
        Err(error) => Err(repo_err(error)),
    }
}

/// `ScopedIdentitySyncResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncResult {
    pub contact_inbox: ContactInbox,
    pub learned_primary_identity: Option<String>,
    pub phone_transition: Option<PhoneTransition>,
}

/// `shouldAdvanceFromParentMatch`.
fn should_advance(row: &ContactInbox, incoming: &IncomingIdentity, matched_by: &str) -> bool {
    matched_by == "sourceParentUserId"
        && !blank(incoming.source_user_id.as_deref())
        && incoming.source_user_id != row.source_user_id
        && !blank(row.source_parent_user_id.as_deref())
}

/// `backfillScopedIdentity`: fills a missing scoped id (only while it is
/// still NULL) and a changed username; a conflicting id is dropped and the
/// rest retried.
fn backfill(
    conn: &Connection,
    row: &ContactInbox,
    incoming: &IncomingIdentity,
    mut pending: Vec<&'static str>,
    username: Option<&str>,
) -> Result<ContactInbox> {
    for _ in 0..=pending.len() {
        let mut sets = Vec::new();
        for field in &pending {
            let value = if *field == "sourceUserId" {
                &incoming.source_user_id
            } else {
                &incoming.source_parent_user_id
            };
            sets.push((*field, value.clone()));
        }
        if let Some(name) = username {
            sets.push(("sourceUsername", Some(name.to_owned())));
        }
        if sets.is_empty() {
            break;
        }
        let mut q = Sql::raw(r#"UPDATE "ContactInbox" SET "updatedAt" = "updatedAt""#);
        for (col, value) in &sets {
            q.push(&format!(r#", "{col}" = "#));
            q.push_bind(value.clone());
        }
        q.append(Sql::bind(r#" WHERE "id" = "#, row.id));
        for field in &pending {
            q.push(&format!(r#" AND "{field}" IS NULL"#));
        }
        q.push(&format!(" RETURNING {}", ContactInbox::columns_sql(None)));
        match q.model::<ContactInbox>(conn) {
            Ok(Some(updated)) => return Ok(updated),
            Ok(None) if username.is_some() && !pending.is_empty() => pending.clear(),
            Ok(None) => break,
            Err(error) if is_unique_violation(&error) => {
                let text = error.to_string();
                let Some(index) = pending
                    .iter()
                    .position(|f| text.contains(&format!("ContactInbox.{f}")))
                else {
                    return Err(repo_err(error));
                };
                tracing::warn!(
                    %error,
                    contact_inbox_id = row.id,
                    field = pending[index],
                    "ContactInbox scoped identity backfill skipped: identity already claimed by another row in this inbox"
                );
                pending.remove(index);
            }
            Err(error) => return Err(repo_err(error)),
        }
    }
    Ok(row.clone())
}

/// `contactInboxService.syncScopedIdentity`.
pub fn sync_scoped_identity(
    conn: &Connection,
    row: &ContactInbox,
    incoming: &IncomingIdentity,
    matched_by: &str,
) -> Result<SyncResult> {
    let advance = should_advance(row, incoming, matched_by);
    let mut current = row.clone();
    let mut phone_transition = None;
    if let (true, Some(user_id)) = (advance, incoming.source_user_id.as_deref()) {
        if let Plan::Apply {
            guard,
            set,
            transition,
        } = parent_fallback_plan(row, incoming, user_id)
        {
            let (updated, status) = guarded_update(
                conn,
                row,
                &guard,
                &set,
                "parentFallback",
                "ContactInbox parent-fallback identity rotation skipped: identity already claimed by another row in this inbox",
            )?;
            current = updated;
            phone_transition = transition.filter(|_| status == WriteStatus::Applied);
        }
    }
    // `resolveScopedIdentityBackfillPlan`.
    let mut pending = Vec::new();
    if !advance && !blank(incoming.source_user_id.as_deref()) && current.source_user_id.is_none() {
        pending.push("sourceUserId");
    }
    if !blank(incoming.source_parent_user_id.as_deref()) && current.source_parent_user_id.is_none()
    {
        pending.push("sourceParentUserId");
    }
    let username = incoming
        .source_username
        .as_deref()
        .filter(|u| !u.is_empty() && Some(*u) != current.source_username.as_deref());
    if !pending.is_empty() || username.is_some() {
        current = backfill(conn, &current, incoming, pending, username)?;
    }
    // `resolveLearnedPrimaryIdentity`.
    let learned_primary_identity = (!advance
        && is_user_id_keyed(&current.source_id, current.source_user_id.as_deref())
        && is_distinct_primary_identity(
            Some(&incoming.source_id),
            &[incoming.source_user_id.as_deref()],
        )
        && incoming.source_id != current.source_id)
        .then(|| incoming.source_id.clone());
    Ok(SyncResult {
        contact_inbox: current,
        learned_primary_identity,
        phone_transition,
    })
}

fn digits_only(value: &str) -> String {
    value.chars().filter(char::is_ascii_digit).collect()
}

fn contact_phone(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Option<String>>> {
    conn.query_row(
        r#"SELECT "phoneNumber" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![id, workspace_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(repo_err)
}

/// `contactService.adoptPhoneNumberIfSafeInTransaction`: the new phone lands
/// only when the stored one is empty or equals the previous one (digits); the
/// write guards the exact stored value. `None` = skipped.
pub fn adopt_phone_number_if_safe(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    previous_phone: Option<&str>,
    new_phone: &str,
) -> Result<Option<PhoneAdoption>> {
    let Some(stored) = contact_phone(conn, workspace_id, contact_id)? else {
        return Err(BusinessError::not_found("Contact not found"));
    };
    let can_adopt = stored.is_none()
        || previous_phone
            .is_some_and(|p| stored.as_deref().map(digits_only) == Some(digits_only(p)));
    if !can_adopt {
        return Ok(None);
    }
    // `preservePhoneStyle`.
    let phone = if stored.as_deref().is_some_and(|s| s.trim().starts_with('+')) {
        format!("+{}", digits_only(new_phone))
    } else {
        new_phone.to_owned()
    };
    let changed = conn
        .execute(
            r#"UPDATE "Contact" SET "phoneNumber" = ?1, "updatedAt" = ?2
               WHERE "id" = ?3 AND "workspaceId" = ?4 AND "phoneNumber" IS ?5"#,
            params![phone, now_ms(), contact_id, workspace_id, stored],
        )
        .map_err(repo_err)?;
    Ok((changed > 0).then_some((stored, phone)))
}

/// `syncExistingContactIdentity` + the learned primary identity backfill of
/// `buildExistingContactMatch`. Returns the synced row and the contact phone
/// write, if any.
pub fn sync_existing_contact_identity(
    conn: &Connection,
    workspace_id: i64,
    row: &ContactInbox,
    incoming: &IncomingIdentity,
    matched_by: &str,
) -> Result<(ContactInbox, Option<PhoneAdoption>)> {
    // The phone adoption shares the rotation's savepoint: a rejected adoption
    // undoes the rotation (`PhoneAdoptionRejected`).
    conn.execute_batch("SAVEPOINT identity_sync")
        .map_err(repo_err)?;
    let outcome = (|| {
        let sync = sync_scoped_identity(conn, row, incoming, matched_by)?;
        if let Some(t) = &sync.phone_transition {
            let adopted = adopt_phone_number_if_safe(
                conn,
                workspace_id,
                row.contact_id,
                t.previous_phone.as_deref(),
                &t.new_phone,
            )?;
            return Ok(adopted.map(|a| (sync.contact_inbox, Some(a))));
        }
        let mut phone = None;
        if let Some(value) = &sync.learned_primary_identity {
            let before = contact_phone(conn, workspace_id, row.contact_id)?.flatten();
            conn.execute(
                r#"UPDATE "Contact" SET "phoneNumber" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "workspaceId" = ?4"#,
                params![value, now_ms(), row.contact_id, workspace_id],
            )
            .map_err(repo_err)?;
            phone = Some((before, value.clone()));
        }
        Ok(Some((sync.contact_inbox, phone)))
    })();
    let release = |sql: &str| conn.execute_batch(sql).map_err(repo_err);
    match outcome {
        Ok(Some(done)) => {
            release("RELEASE identity_sync")?;
            Ok(done)
        }
        Ok(None) => {
            release("ROLLBACK TO identity_sync; RELEASE identity_sync")?;
            Ok((row.clone(), None))
        }
        Err(error) => {
            release("ROLLBACK TO identity_sync; RELEASE identity_sync")?;
            Err(error)
        }
    }
}

/// `ContactInboxIdentityChangeResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeResult {
    Invalid,
    NotFound,
    Done {
        /// `applied` | `alreadyApplied` | `stale` | `conflict`.
        status: &'static str,
        contact_inbox: Box<ContactInbox>,
        contact_id: i64,
        phone_transition: Option<PhoneTransition>,
    },
}

/// `findIdentityWithContact`: the first probe with a value that matches.
fn find_identity(
    conn: &Connection,
    inbox_id: i64,
    probes: &[(&'static str, Option<&str>)],
) -> Result<Option<(&'static str, String, WithContact)>> {
    for (field, value) in probes {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            continue;
        };
        let mut filter = Sql::bind(r#""inboxId" = "#, inbox_id);
        filter.append(Sql::bind(format!(r#" AND "{field}" = "#), value.to_owned()));
        if let Some(found) = find_with_contact(conn, &filter).map_err(repo_err)? {
            return Ok(Some((field, value.to_owned(), found)));
        }
    }
    Ok(None)
}

/// A WhatsApp `userIdChanged` change (`RotationChange`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RotationChange {
    pub previous_user_id: Option<String>,
    pub user_id: String,
    pub previous_parent_user_id: Option<String>,
    pub parent_user_id: Option<String>,
    pub previous_phone: Option<String>,
    pub new_phone: Option<String>,
}

fn done(
    status: &'static str,
    contact_inbox: ContactInbox,
    contact_id: i64,
    transition: Option<PhoneTransition>,
) -> ChangeResult {
    let phone_transition = transition.filter(|_| matches!(status, "applied" | "alreadyApplied"));
    ChangeResult::Done {
        status,
        contact_inbox: Box::new(contact_inbox),
        contact_id,
        phone_transition,
    }
}

fn status_name(status: WriteStatus) -> &'static str {
    match status {
        WriteStatus::Applied => "applied",
        WriteStatus::Stale => "stale",
        WriteStatus::Conflict => "conflict",
    }
}

/// `contactInboxService.rotateScopedUserId`.
pub fn rotate_scoped_user_id(
    conn: &Connection,
    inbox_id: i64,
    change: &RotationChange,
) -> Result<ChangeResult> {
    // `normalizeRotationChange`.
    let previous_user_id = trimmed(change.previous_user_id.as_deref());
    let user_id = change.user_id.trim().to_owned();
    let previous_phone = trimmed(change.previous_phone.as_deref());
    let c = RotationChange {
        new_phone: resolve_phone_transition(
            previous_phone.as_deref(),
            trimmed(change.new_phone.as_deref()).as_deref(),
            &[Some(&user_id), previous_user_id.as_deref()],
        )
        .map(|t| t.new_phone),
        previous_parent_user_id: trimmed(change.previous_parent_user_id.as_deref()),
        parent_user_id: trimmed(change.parent_user_id.as_deref()),
        previous_user_id,
        user_id,
        previous_phone,
    };
    // `isValidRotationChange`.
    let valid = !c.user_id.is_empty()
        && (c.previous_user_id.is_some()
            || c.previous_parent_user_id.is_some()
            || c.previous_phone.is_some())
        && (c.previous_user_id.as_deref() != Some(c.user_id.as_str()) || c.new_phone.is_some());
    if !valid {
        return Ok(ChangeResult::Invalid);
    }
    // `findRotationMatch`.
    let previous = find_identity(
        conn,
        inbox_id,
        &[
            ("sourceUserId", c.previous_user_id.as_deref()),
            ("sourceId", c.previous_user_id.as_deref()),
            ("sourceParentUserId", c.previous_parent_user_id.as_deref()),
            ("sourceId", c.previous_phone.as_deref()),
        ],
    )?;
    let (kind, found) = match previous {
        Some(found) => ("previous", found),
        None => match find_identity(
            conn,
            inbox_id,
            &[
                ("sourceUserId", Some(c.user_id.as_str())),
                ("sourceId", Some(c.user_id.as_str())),
                ("sourceParentUserId", c.parent_user_id.as_deref()),
                ("sourceId", c.new_phone.as_deref()),
            ],
        )? {
            Some(found) => ("current", found),
            None => return Ok(ChangeResult::NotFound),
        },
    };
    let (field, value, with) = found;
    let row = &with.contact_inbox;
    let contact_id = with.contact.id;
    // `resolveRotationPlan`.
    let unrelated = row
        .source_user_id
        .as_deref()
        .is_some_and(|u| u != c.user_id && Some(u) != c.previous_user_id.as_deref());
    let previous_key_without_user_id = kind == "previous"
        && c.previous_user_id.is_none()
        && ((field == "sourceId" && Some(value.as_str()) == c.previous_phone.as_deref())
            || (field == "sourceParentUserId"
                && Some(value.as_str()) == c.previous_parent_user_id.as_deref()));
    if unrelated && !previous_key_without_user_id {
        return Ok(done("stale", row.clone(), contact_id, None));
    }
    let keyed = row.source_id.is_empty()
        || is_user_id_keyed(&row.source_id, row.source_user_id.as_deref())
        || Some(row.source_id.as_str()) == c.previous_user_id.as_deref()
        || row.source_id == c.user_id;
    let plan = target_state_plan(
        row,
        (field, &value),
        Identity {
            source_id: Some(c.user_id.clone()),
            source_user_id: Some(c.user_id.clone()),
            source_parent_user_id: c.parent_user_id.clone(),
        },
        PhoneObservation {
            previous_phone: c.previous_phone.as_deref(),
            new_phone: c.new_phone.as_deref(),
            scoped: vec![
                Some(&c.user_id),
                c.previous_user_id.as_deref(),
                row.source_user_id.as_deref(),
            ],
            use_new_phone_as_source_id: !keyed,
        },
    );
    Ok(match plan {
        Plan::AlreadyApplied(t) => done("alreadyApplied", row.clone(), contact_id, t),
        Plan::Apply {
            guard,
            set,
            transition,
        } => {
            let (updated, status) = guarded_update(
                conn,
                row,
                &guard,
                &set,
                "userIdChanged",
                "ContactInbox scoped user id rotation skipped: identity already claimed by another row in this inbox",
            )?;
            done(status_name(status), updated, contact_id, transition)
        }
    })
}

/// `contactInboxService.changePrimaryPhone`.
pub fn change_primary_phone(
    conn: &Connection,
    inbox_id: i64,
    previous_phone: &str,
    new_phone: &str,
    user_id: Option<&str>,
) -> Result<ChangeResult> {
    let (previous_phone, new_phone) = (previous_phone.trim(), new_phone.trim());
    let user_id = user_id.map(str::trim).filter(|u| !u.is_empty());
    if previous_phone.is_empty()
        || new_phone == previous_phone
        || !is_distinct_primary_identity(Some(new_phone), &[user_id])
    {
        return Ok(ChangeResult::Invalid);
    }
    let (kind, (field, _, with)) = match find_identity(
        conn,
        inbox_id,
        &[
            ("sourceId", Some(previous_phone)),
            ("sourceUserId", user_id),
        ],
    )? {
        Some(found) => ("previous", found),
        None => match find_identity(conn, inbox_id, &[("sourceId", Some(new_phone))])? {
            Some(found) => ("current", found),
            None => return Ok(ChangeResult::NotFound),
        },
    };
    let row = &with.contact_inbox;
    let contact_id = with.contact.id;
    // `resolvePhoneChangePlan`.
    if user_id.is_some() && row.source_user_id.is_some() && row.source_user_id.as_deref() != user_id
    {
        return Ok(done("stale", row.clone(), contact_id, None));
    }
    let Some(transition) = resolve_phone_transition(
        Some(previous_phone),
        Some(new_phone),
        &[user_id, row.source_user_id.as_deref()],
    ) else {
        return Ok(ChangeResult::Invalid);
    };
    if kind == "current" || row.source_id == new_phone {
        return Ok(done(
            "alreadyApplied",
            row.clone(),
            contact_id,
            Some(transition),
        ));
    }
    if field == "sourceUserId" {
        let keyed = row.source_id.is_empty()
            || is_user_id_keyed(&row.source_id, row.source_user_id.as_deref());
        return Ok(if keyed {
            done("applied", row.clone(), contact_id, Some(transition))
        } else {
            done("stale", row.clone(), contact_id, None)
        });
    }
    let guard = IdentityFields {
        source_id: Some(Some(previous_phone.to_owned())),
        source_user_id: user_id.map(|_| row.source_user_id.clone()),
        source_parent_user_id: None,
    };
    let set = Identity {
        source_id: Some(new_phone.to_owned()),
        ..Identity::default()
    };
    let (updated, status) = guarded_update(
        conn,
        row,
        &guard,
        &set,
        "phoneChanged",
        "ContactInbox primary identity change skipped: identity already claimed by another row in this inbox",
    )?;
    Ok(done(
        status_name(status),
        updated,
        contact_id,
        Some(transition),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_transitions_skip_scoped_ids_and_repeats() {
        assert_eq!(
            resolve_phone_transition(Some("1"), Some("2"), &[Some("u")]),
            Some(PhoneTransition {
                previous_phone: Some("1".into()),
                new_phone: "2".into()
            })
        );
        assert_eq!(resolve_phone_transition(Some("1"), Some("1"), &[]), None);
        assert_eq!(
            resolve_phone_transition(None, Some("u"), &[Some("u")]),
            None
        );
        assert!(!is_distinct_primary_identity(Some(" "), &[]));
        assert!(is_user_id_keyed("u", Some("u")));
        assert!(!is_user_id_keyed("", None));
    }
}
