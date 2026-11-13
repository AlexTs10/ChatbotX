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
    let _ = scope;
    todo!("P3.6-D1p")
}

/// `resolvePipelinePermissions`: the member row as permissions (or none).
pub fn resolve_pipeline_permissions(
    scope: &PipelineAccessScope,
    member_row: Option<PipelinePermissions>,
) -> PipelinePermissions {
    let _ = (scope, member_row);
    todo!("P3.6-D1p")
}

/// `normalizePipelinePermissions`: edit, delete and manage imply view.
pub fn normalize_pipeline_permissions(permissions: PipelinePermissions) -> PipelinePermissions {
    let _ = permissions;
    todo!("P3.6-D1p")
}

/// `permitsPipelineAction`.
pub fn permits_pipeline_action(permissions: &PipelinePermissions, action: PipelineAction) -> bool {
    let _ = (permissions, action);
    todo!("P3.6-D1p")
}

/// `PipelineGrantPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineGrantPolicy {
    Unlimited,
    Limited { self_member_id: i64 },
}

/// `pipelineGrantPolicy`.
pub fn pipeline_grant_policy(scope: &PipelineAccessScope) -> PipelineGrantPolicy {
    let _ = scope;
    todo!("P3.6-D1p")
}
