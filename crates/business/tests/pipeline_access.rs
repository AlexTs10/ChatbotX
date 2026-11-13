//! Port of `packages/business/__tests__/pipeline-access.test.ts` (pure
//! functions; `test.each` = one test over its cases).

use chatbotx_business::pipeline::access::{
    FULL_PIPELINE_PERMISSIONS, NO_PIPELINE_PERMISSIONS, can_view_contact_email_and_phone,
    normalize_pipeline_permissions, permits_pipeline_action, resolve_pipeline_permissions,
};
use chatbotx_business::pipeline::{PipelineAccessScope, PipelineAction, PipelinePermissions};

fn member(is_workspace_admin: bool, can_view_contact_email_and_phone: bool) -> PipelineAccessScope {
    PipelineAccessScope::Member {
        workspace_member_id: 1,
        is_workspace_admin,
        can_view_contact_email_and_phone,
    }
}

const ROW: PipelinePermissions = PipelinePermissions {
    can_view: true,
    can_edit: false,
    can_delete: false,
    can_manage_settings: false,
};

#[test]
fn resolve_pipeline_permissions_bypasses_the_per_pipeline_acl_even_with_no_member_row() {
    for scope in [
        PipelineAccessScope::WorkspaceToken,
        PipelineAccessScope::Support,
        member(true, true),
    ] {
        assert_eq!(
            resolve_pipeline_permissions(&scope, None),
            FULL_PIPELINE_PERMISSIONS,
            "{scope:?}"
        );
    }
}

#[test]
fn resolve_pipeline_permissions_a_plain_member_without_a_row_has_no_access_at_all() {
    assert_eq!(
        resolve_pipeline_permissions(&member(false, true), None),
        NO_PIPELINE_PERMISSIONS
    );
}

#[test]
fn resolve_pipeline_permissions_a_plain_member_holds_exactly_what_their_row_grants() {
    assert_eq!(
        resolve_pipeline_permissions(&member(false, true), Some(ROW)),
        ROW
    );
}

#[test]
fn resolve_pipeline_permissions_an_admin_ignores_a_restrictive_member_row() {
    assert_eq!(
        resolve_pipeline_permissions(&member(true, true), Some(ROW)),
        FULL_PIPELINE_PERMISSIONS
    );
}

#[test]
fn normalize_pipeline_permissions_each_flag_implies_view() {
    let flags: [fn(&mut PipelinePermissions); 3] = [
        |p| p.can_edit = true,
        |p| p.can_delete = true,
        |p| p.can_manage_settings = true,
    ];
    for set in flags {
        let mut permissions = NO_PIPELINE_PERMISSIONS;
        set(&mut permissions);
        assert!(normalize_pipeline_permissions(permissions).can_view);
    }
}

#[test]
fn normalize_pipeline_permissions_leaves_a_view_only_grant_alone() {
    assert_eq!(normalize_pipeline_permissions(ROW), ROW);
}

#[test]
fn permits_pipeline_action_maps_each_action_to_its_own_flag() {
    let permissions = PipelinePermissions {
        can_view: true,
        can_edit: true,
        ..NO_PIPELINE_PERMISSIONS
    };
    assert!(permits_pipeline_action(&permissions, PipelineAction::View));
    assert!(permits_pipeline_action(&permissions, PipelineAction::Edit));
    assert!(!permits_pipeline_action(
        &permissions,
        PipelineAction::Delete
    ));
    assert!(!permits_pipeline_action(
        &permissions,
        PipelineAction::ManageSettings
    ));
}

#[test]
fn can_view_contact_email_and_phone_tokens_and_support_sessions_see_contact_email_and_phone() {
    assert!(can_view_contact_email_and_phone(
        &PipelineAccessScope::WorkspaceToken
    ));
    assert!(can_view_contact_email_and_phone(
        &PipelineAccessScope::Support
    ));
}

#[test]
fn can_view_contact_email_and_phone_a_member_follows_their_own_contact_permission() {
    assert!(can_view_contact_email_and_phone(&member(false, true)));
    assert!(!can_view_contact_email_and_phone(&member(false, false)));
    // Pipeline admin rights do not grant contact PII.
    assert!(!can_view_contact_email_and_phone(&member(true, false)));
}
