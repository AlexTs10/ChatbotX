//! `pipeline/access.ts`: who may do what on a pipeline.

/// `PipelineAccessScope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineAccessScope {
    WorkspaceToken,
    System,
    Support,
    Member {
        workspace_member_id: i64,
        is_workspace_admin: bool,
        can_view_contact_email_and_phone: bool,
    },
}

/// `PipelinePermissions`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelinePermissions {
    pub can_view: bool,
    pub can_edit: bool,
    pub can_delete: bool,
    pub can_manage_settings: bool,
}

/// `PipelineAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineAction {
    View,
    Edit,
    Delete,
    ManageSettings,
}

/// `FULL_PIPELINE_PERMISSIONS`.
pub const FULL_PIPELINE_PERMISSIONS: PipelinePermissions = PipelinePermissions {
    can_view: true,
    can_edit: true,
    can_delete: true,
    can_manage_settings: true,
};

/// `NO_PIPELINE_PERMISSIONS`.
pub const NO_PIPELINE_PERMISSIONS: PipelinePermissions = PipelinePermissions {
    can_view: false,
    can_edit: false,
    can_delete: false,
    can_manage_settings: false,
};

/// `canViewContactEmailAndPhone`.
pub fn can_view_contact_email_and_phone(scope: &PipelineAccessScope) -> bool {
    match scope {
        PipelineAccessScope::Member {
            can_view_contact_email_and_phone,
            ..
        } => *can_view_contact_email_and_phone,
        _ => true,
    }
}

/// `bypassesPipelineAcl`.
fn bypasses_pipeline_acl(scope: &PipelineAccessScope) -> bool {
    match scope {
        PipelineAccessScope::Member {
            is_workspace_admin, ..
        } => *is_workspace_admin,
        _ => true,
    }
}

/// `resolvePipelinePermissions`: the member row as permissions (or none).
pub fn resolve_pipeline_permissions(
    scope: &PipelineAccessScope,
    member_row: Option<PipelinePermissions>,
) -> PipelinePermissions {
    if bypasses_pipeline_acl(scope) {
        return FULL_PIPELINE_PERMISSIONS;
    }
    member_row.map_or(NO_PIPELINE_PERMISSIONS, normalize_pipeline_permissions)
}

/// `normalizePipelinePermissions`: edit, delete and manage imply view.
pub fn normalize_pipeline_permissions(permissions: PipelinePermissions) -> PipelinePermissions {
    let needs_view =
        permissions.can_edit || permissions.can_delete || permissions.can_manage_settings;
    PipelinePermissions {
        can_view: permissions.can_view || needs_view,
        ..permissions
    }
}

/// `permitsPipelineAction`.
pub fn permits_pipeline_action(permissions: &PipelinePermissions, action: PipelineAction) -> bool {
    match action {
        PipelineAction::View => permissions.can_view,
        PipelineAction::Edit => permissions.can_edit,
        PipelineAction::Delete => permissions.can_delete,
        PipelineAction::ManageSettings => permissions.can_manage_settings,
    }
}

/// `PipelineGrantPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineGrantPolicy {
    Unlimited,
    Limited { self_member_id: i64 },
}

/// `pipelineGrantPolicy`.
pub fn pipeline_grant_policy(scope: &PipelineAccessScope) -> PipelineGrantPolicy {
    match scope {
        PipelineAccessScope::Member {
            workspace_member_id,
            is_workspace_admin: false,
            ..
        } => PipelineGrantPolicy::Limited {
            self_member_id: *workspace_member_id,
        },
        _ => PipelineGrantPolicy::Unlimited,
    }
}
