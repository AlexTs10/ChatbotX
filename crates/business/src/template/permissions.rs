//! `template/permissions.ts`: who may manage, read and install templates.

use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::workspace_member::has_permission;

/// `templateCategories`, in display order (resource categories, then the
/// manifest-only ones).
pub const CATEGORIES: &[&str] = &[
    "flows",
    "products",
    "aiFunctions",
    "aiAgents",
    "calendars",
    "webchats",
    "keywords",
    "entryPointLinks",
    "triggers",
    "fbCommentAutomations",
    "settings",
    "sequences",
    "aiMcpServers",
    "aiFiles",
    "questionnaires",
    "couponTopics",
    "pipelines",
    "customFields",
    "tags",
    "productCategories",
];

/// `TEMPLATE_CATEGORY_PERMISSION`.
pub fn category_permission(category: &str) -> &'static str {
    match category {
        "products" | "couponTopics" | "productCategories" => "ecommerce",
        "sequences" => "broadcast",
        _ => "flows",
    }
}

/// `canViewTemplateInstalls`: any category permission.
pub fn can_view_installs(actor: &Actor) -> bool {
    CATEGORIES.iter().any(|c| actor.has(category_permission(c)))
}

/// `TEMPLATE_MANAGE_PERMISSION`.
const MANAGE_PERMISSION: &str = "flows";

/// `TemplateActor`.
#[derive(Debug, Clone, PartialEq)]
pub enum Actor {
    /// A workspace member with their stored permissions.
    Member(Value),
    /// A workspace API token; `true` = `full`, `false` = `read_only`.
    ApiToken { full: bool },
}

impl Actor {
    fn has(&self, key: &str) -> bool {
        match self {
            Actor::Member(permissions) => has_permission(permissions, key),
            Actor::ApiToken { full } => *full,
        }
    }
}

fn manage_denied() -> BusinessError {
    BusinessError::exception(
        "You don't have permission to manage templates",
        "templateManagePermissionDenied",
        403,
    )
}

/// `templateCategoryPermissionDeniedException`.
pub fn category_denied() -> BusinessError {
    BusinessError::exception(
        "You don't have permission to use this template's contents",
        "templateCategoryPermissionDenied",
        403,
    )
}

/// `canManageTemplates`.
pub fn can_manage(actor: &Actor) -> bool {
    actor.has(MANAGE_PERMISSION)
}

/// `assertCanManageTemplates`.
pub fn assert_can_manage(actor: &Actor) -> Result<()> {
    if can_manage(actor) {
        Ok(())
    } else {
        Err(manage_denied())
    }
}

/// `assertCanReadTemplates`: any token, or a member who may manage.
pub fn assert_can_read(actor: &Actor) -> Result<()> {
    if matches!(actor, Actor::ApiToken { .. }) || can_manage(actor) {
        Ok(())
    } else {
        Err(BusinessError::exception(
            "You don't have permission to view templates",
            "templateManagePermissionDenied",
            403,
        ))
    }
}

/// `assertCanWriteTemplateInstalls`: a `read_only` token may not change copies.
pub fn assert_can_write_installs(actor: &Actor) -> Result<()> {
    if *actor == (Actor::ApiToken { full: false }) {
        return Err(BusinessError::exception(
            "A read-only token can not change template installations",
            "templateManagePermissionDenied",
            403,
        ));
    }
    Ok(())
}

/// `assertCanUseTemplateCategories` over the categories with a non-zero count.
pub fn assert_can_use_categories<'a>(
    actor: &Actor,
    categories: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    for category in categories {
        if !actor.has(category_permission(category)) {
            return Err(category_denied());
        }
    }
    Ok(())
}

/// `visibleInstallCategories`: `None` = every category.
pub fn visible_install_categories(actor: &Actor) -> Option<Vec<&'static str>> {
    if matches!(actor, Actor::ApiToken { .. }) {
        return None;
    }
    let visible: Vec<_> = CATEGORIES
        .iter()
        .copied()
        .filter(|category| actor.has(category_permission(category)))
        .collect();
    (visible.len() != CATEGORIES.len()).then_some(visible)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tokens_and_members() {
        let read_only = Actor::ApiToken { full: false };
        assert!(assert_can_manage(&read_only).is_err());
        assert!(assert_can_read(&read_only).is_ok());
        assert!(assert_can_write_installs(&read_only).is_err());
        assert!(assert_can_write_installs(&Actor::ApiToken { full: true }).is_ok());
        let shop = Actor::Member(json!({ "ecommerce": true }));
        assert!(assert_can_read(&shop).is_err());
        assert!(assert_can_use_categories(&shop, ["products"]).is_ok());
        assert!(assert_can_use_categories(&shop, ["flows"]).is_err());
        assert_eq!(
            visible_install_categories(&shop),
            Some(vec!["products", "couponTopics", "productCategories"])
        );
        assert_eq!(
            visible_install_categories(&Actor::Member(json!({ "superAdmin": true }))),
            None
        );
    }
}
