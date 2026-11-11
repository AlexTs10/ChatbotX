//! Port of `packages/worker-config/__tests__/stored-image-optimization.test.ts` (spec 5.1). The bullmq `Queue.addBulk` mock is the
//! `FakeEnqueuer`; `STORED_IMAGE_OPTIMIZATION` is passed as config (no
//! global env mutation).

mod support;

use std::collections::BTreeMap;

use chatbotx_jobs::JobsError;
use chatbotx_jobs::QueueName;
use chatbotx_jobs::queues::heavy::HeavyJobAction;
use chatbotx_jobs::queues::stored_image::{
    OptimizeStoredImagePayload, StoredImageJob, StoredImageOptimizationConfig,
    enqueue_stored_image_optimizations, is_optimizable_storage_key,
};
use support::FakeEnqueuer;

fn attachment_job() -> StoredImageJob {
    StoredImageJob::Attachment {
        workspace_id: "ws-1".to_owned(),
        object_key: "public/messenger/ws-1/int-1/2026/10/13/att-key".to_owned(),
        attachment_id: "att-1".to_owned(),
        attachment_created_at: "2026-10-13T01:00:00.000Z".to_owned(),
        message_created_at: "2026-10-13T01:00:00.000Z".to_owned(),
    }
}

fn avatar_job_with_key(object_key: &str) -> StoredImageJob {
    StoredImageJob::Avatar {
        workspace_id: "ws-1".to_owned(),
        object_key: object_key.to_owned(),
        contact_id: "contact-1".to_owned(),
    }
}

fn avatar_job() -> StoredImageJob {
    avatar_job_with_key("public/space/public/ws/ws-1/2026/10/13/avatars/av-key")
}

fn enabled() -> StoredImageOptimizationConfig {
    StoredImageOptimizationConfig { enabled: true }
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:47 "isOptimizableStorageKey > accepts an extension-less key we own: public/messenger/ws-1/int-1/2026/10/13/abc"
#[test]
fn accepts_an_extension_less_key_we_own_public_messenger_ws_1_int_1_2026_10_13_abc() {
    assert!(is_optimizable_storage_key(
        Some("public/messenger/ws-1/int-1/2026/10/13/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:47 "isOptimizableStorageKey > accepts an extension-less key we own: public/space/ws-1/contacts/c-1/avatar/abc"
#[test]
fn accepts_an_extension_less_key_we_own_public_space_ws_1_contacts_c_1_avatar_abc() {
    assert!(is_optimizable_storage_key(
        Some("public/space/ws-1/contacts/c-1/avatar/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:47 "isOptimizableStorageKey > accepts an extension-less key we own: workspace/whatsapp/ws-1/int-1/2026/10/13/abc"
#[test]
fn accepts_an_extension_less_key_we_own_workspace_whatsapp_ws_1_int_1_2026_10_13_abc() {
    assert!(is_optimizable_storage_key(
        Some("workspace/whatsapp/ws-1/int-1/2026/10/13/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects null"
#[test]
fn rejects_null() {
    assert!(!is_optimizable_storage_key(None, "ws-1"));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects undefined"
#[test]
fn rejects_undefined() {
    assert!(!is_optimizable_storage_key(None, "ws-1"));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects "
#[test]
fn rejects_empty_string() {
    assert!(!is_optimizable_storage_key(Some(""), "ws-1"));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects https://scontent.xx.fbcdn.net/v/photo.jpg"
#[test]
fn rejects_https_scontent_xx_fbcdn_net_v_photo_jpg() {
    assert!(!is_optimizable_storage_key(
        Some("https://scontent.xx.fbcdn.net/v/photo.jpg"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects wa-media:1379511273954485//"
#[test]
fn rejects_wa_media_1379511273954485() {
    assert!(!is_optimizable_storage_key(
        Some("wa-media:1379511273954485//"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects failed:too-large"
#[test]
fn rejects_failed_too_large() {
    assert!(!is_optimizable_storage_key(
        Some("failed:too-large"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects public/img/no_avatar.jpg?time=1760000000000"
#[test]
fn rejects_public_img_no_avatar_jpg_time_1760000000000() {
    assert!(!is_optimizable_storage_key(
        Some("public/img/no_avatar.jpg?time=1760000000000"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects public/space/ws-1/logos/abc.jpg"
#[test]
fn rejects_public_space_ws_1_logos_abc_jpg() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-1/logos/abc.jpg"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects workspace/messenger/ws-1/int-1/2026/10/13/abc.png"
#[test]
fn rejects_workspace_messenger_ws_1_int_1_2026_10_13_abc_png() {
    assert!(!is_optimizable_storage_key(
        Some("workspace/messenger/ws-1/int-1/2026/10/13/abc.png"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects public/space/ws-1/avatars/abc.webp"
#[test]
fn rejects_public_space_ws_1_avatars_abc_webp() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-1/avatars/abc.webp"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:55 "isOptimizableStorageKey > rejects public/messenger/ws-1/"
#[test]
fn rejects_public_messenger_ws_1() {
    assert!(!is_optimizable_storage_key(
        Some("public/messenger/ws-1/"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: public/messenger/ws-2/int-1/2026/10/13/abc"
#[test]
fn rejects_a_key_outside_the_workspace_public_messenger_ws_2_int_1_2026_10_13_abc() {
    assert!(!is_optimizable_storage_key(
        Some("public/messenger/ws-2/int-1/2026/10/13/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: public/img/no_avatar"
#[test]
fn rejects_a_key_outside_the_workspace_public_img_no_avatar() {
    assert!(!is_optimizable_storage_key(
        Some("public/img/no_avatar"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: public/space/ws-10/contacts/c-1/avatar/abc"
#[test]
fn rejects_a_key_outside_the_workspace_public_space_ws_10_contacts_c_1_avatar_abc() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-10/contacts/c-1/avatar/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: public/space/xws-1/contacts/c-1/avatar/abc"
#[test]
fn rejects_a_key_outside_the_workspace_public_space_xws_1_contacts_c_1_avatar_abc() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/xws-1/contacts/c-1/avatar/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: public/space/ws-2/ws-1/image"
#[test]
fn rejects_a_key_outside_the_workspace_public_space_ws_2_ws_1_image() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-2/ws-1/image"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: public/messenger/ws-2/ws-1/2026/10/13/abc"
#[test]
fn rejects_a_key_outside_the_workspace_public_messenger_ws_2_ws_1_2026_10_13_abc() {
    assert!(!is_optimizable_storage_key(
        Some("public/messenger/ws-2/ws-1/2026/10/13/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: workspace/whatsapp/ws-2/int-1/ws-1/abc"
#[test]
fn rejects_a_key_outside_the_workspace_workspace_whatsapp_ws_2_int_1_ws_1_abc() {
    assert!(!is_optimizable_storage_key(
        Some("workspace/whatsapp/ws-2/int-1/ws-1/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: ws-1/abc"
#[test]
fn rejects_a_key_outside_the_workspace_ws_1_abc() {
    assert!(!is_optimizable_storage_key(Some("ws-1/abc"), "ws-1"));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:76 "isOptimizableStorageKey > rejects a key outside the workspace: public/ws-1/abc"
#[test]
fn rejects_a_key_outside_the_workspace_public_ws_1_abc() {
    assert!(!is_optimizable_storage_key(Some("public/ws-1/abc"), "ws-1"));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: public/space/ws-1/../../messenger/ws-2/int-1/2026/10/13/abc"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_dot_dot() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-1/../../messenger/ws-2/int-1/2026/10/13/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: public/space/ws-1/./abc"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_dot_segment() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-1/./abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: public//ws-1/abc"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_double_slash_after_public() {
    assert!(!is_optimizable_storage_key(
        Some("public//ws-1/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: /public/space/ws-1/abc"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_leading_slash() {
    assert!(!is_optimizable_storage_key(
        Some("/public/space/ws-1/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: public/space/ws-1//abc"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_double_slash_after_workspace() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-1//abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: public\space\ws-1\abc"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_backslashes() {
    assert!(!is_optimizable_storage_key(
        Some("public\\space\\ws-1\\abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: public/space/ws-1/%2e%2e/abc"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_percent_encoded_dot_dot() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-1/%2e%2e/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:97 "isOptimizableStorageKey > rejects a key with traversal or non-literal segments: public/space/ws-1/a b"
#[test]
fn rejects_a_key_with_traversal_or_non_literal_segments_space() {
    assert!(!is_optimizable_storage_key(
        Some("public/space/ws-1/a b"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:110 "isOptimizableStorageKey > accepts the workspace id in a known layout position: public/ws/ws-1/2026/10/13/avatars/abc"
#[test]
fn accepts_the_workspace_id_in_a_known_layout_position_public_ws_ws_1_2026_10_13_avatars_abc() {
    assert!(is_optimizable_storage_key(
        Some("public/ws/ws-1/2026/10/13/avatars/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:110 "isOptimizableStorageKey > accepts the workspace id in a known layout position: public/space/ws-1/avatars/abc"
#[test]
fn accepts_the_workspace_id_in_a_known_layout_position_public_space_ws_1_avatars_abc() {
    assert!(is_optimizable_storage_key(
        Some("public/space/ws-1/avatars/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:110 "isOptimizableStorageKey > accepts the workspace id in a known layout position: public/instagram/ws-1/int-1/2026/10/13/abc"
#[test]
fn accepts_the_workspace_id_in_a_known_layout_position_public_instagram_ws_1_int_1_2026_10_13_abc()
{
    assert!(is_optimizable_storage_key(
        Some("public/instagram/ws-1/int-1/2026/10/13/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:110 "isOptimizableStorageKey > accepts the workspace id in a known layout position: workspace/messenger/ws-1/int-1/2026/10/13/abc"
#[test]
fn accepts_the_workspace_id_in_a_known_layout_position_workspace_messenger_ws_1_int_1_2026_10_13_abc()
 {
    assert!(is_optimizable_storage_key(
        Some("workspace/messenger/ws-1/int-1/2026/10/13/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:110 "isOptimizableStorageKey > accepts the workspace id in a known layout position: public/space/public/ws/ws-1/2026/10/13/avatars/abc"
#[test]
fn accepts_the_workspace_id_in_a_known_layout_position_public_space_public_ws_ws_1_2026_10_13_avatars_abc()
 {
    assert!(is_optimizable_storage_key(
        Some("public/space/public/ws/ws-1/2026/10/13/avatars/abc"),
        "ws-1"
    ));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:133 "enqueueStoredImageOptimizations > queues low-priority jobs with deterministic, colon-free ids"
#[tokio::test]
async fn queues_low_priority_jobs_with_deterministic_colon_free_ids() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![attachment_job(), avatar_job()],
    )
    .await;

    let bulks = fake.bulks();
    assert_eq!(bulks.len(), 1);
    let (queue, jobs) = &bulks[0];
    assert_eq!(*queue, QueueName::Heavy);
    let ids: Vec<Option<&str>> = jobs.iter().map(|j| j.opts.job_id.as_deref()).collect();
    assert_eq!(
        ids,
        vec![
            Some("optimize-image-attachment-att-1"),
            Some("optimize-image-avatar-contact-1-av-key"),
        ]
    );
    for job in jobs {
        assert!(!job.opts.job_id.as_deref().unwrap().contains(':'));
        assert!(job.opts.priority.unwrap() > 0);
        assert_eq!(job.name, HeavyJobAction::OptimizeStoredImage.as_str());
        assert!(serde_json::from_value::<OptimizeStoredImagePayload>(job.data.clone()).is_ok());
    }
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:151 "enqueueStoredImageOptimizations > skips keys that are not stored objects"
#[tokio::test]
async fn skips_keys_that_are_not_stored_objects() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key("https://cdn.example.com/a.jpg")],
    )
    .await;

    assert!(fake.bulks().is_empty());
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:163 "enqueueStoredImageOptimizations > skips an avatar job whose key is not an avatar object: public/space/ws-1/logos/brand"
#[tokio::test]
async fn skips_an_avatar_job_whose_key_is_not_an_avatar_object_public_space_ws_1_logos_brand() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key("public/space/ws-1/logos/brand")],
    )
    .await;

    assert!(fake.bulks().is_empty());
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:163 "enqueueStoredImageOptimizations > skips an avatar job whose key is not an avatar object: public/space/ws-1/uploads/contract-scan"
#[tokio::test]
async fn skips_an_avatar_job_whose_key_is_not_an_avatar_object_public_space_ws_1_uploads_contract_scan()
 {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key(
            "public/space/ws-1/uploads/contract-scan",
        )],
    )
    .await;

    assert!(fake.bulks().is_empty());
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:163 "enqueueStoredImageOptimizations > skips an avatar job whose key is not an avatar object: public/ws/ws-1/2026/10/13/media-library-file"
#[tokio::test]
async fn skips_an_avatar_job_whose_key_is_not_an_avatar_object_public_ws_ws_1_2026_10_13_media_library_file()
 {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key(
            "public/ws/ws-1/2026/10/13/media-library-file",
        )],
    )
    .await;

    assert!(fake.bulks().is_empty());
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:163 "enqueueStoredImageOptimizations > skips an avatar job whose key is not an avatar object: public/messenger/ws-1/int-1/2026/10/13/att-key"
#[tokio::test]
async fn skips_an_avatar_job_whose_key_is_not_an_avatar_object_public_messenger_ws_1_int_1_2026_10_13_att_key()
 {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key(
            "public/messenger/ws-1/int-1/2026/10/13/att-key",
        )],
    )
    .await;

    assert!(fake.bulks().is_empty());
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:174 "enqueueStoredImageOptimizations > queues an avatar job for an avatar object: public/space/ws-1/avatars/abc"
#[tokio::test]
async fn queues_an_avatar_job_for_an_avatar_object_public_space_ws_1_avatars_abc() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key("public/space/ws-1/avatars/abc")],
    )
    .await;

    assert_eq!(fake.bulks().len(), 1);
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:174 "enqueueStoredImageOptimizations > queues an avatar job for an avatar object: public/space/ws-1/contacts/c-1/avatar/abc"
#[tokio::test]
async fn queues_an_avatar_job_for_an_avatar_object_public_space_ws_1_contacts_c_1_avatar_abc() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key(
            "public/space/ws-1/contacts/c-1/avatar/abc",
        )],
    )
    .await;

    assert_eq!(fake.bulks().len(), 1);
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:174 "enqueueStoredImageOptimizations > queues an avatar job for an avatar object: public/ws/ws-1/2026/10/13/avatars/abc"
#[tokio::test]
async fn queues_an_avatar_job_for_an_avatar_object_public_ws_ws_1_2026_10_13_avatars_abc() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key("public/ws/ws-1/2026/10/13/avatars/abc")],
    )
    .await;

    assert_eq!(fake.bulks().len(), 1);
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:174 "enqueueStoredImageOptimizations > queues an avatar job for an avatar object: public/space/public/ws/ws-1/2026/10/13/avatars/abc"
#[tokio::test]
async fn queues_an_avatar_job_for_an_avatar_object_public_space_public_ws_ws_1_2026_10_13_avatars_abc()
 {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key(
            "public/space/public/ws/ws-1/2026/10/13/avatars/abc",
        )],
    )
    .await;

    assert_eq!(fake.bulks().len(), 1);
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:185 "enqueueStoredImageOptimizations > skips a key that belongs to another workspace"
#[tokio::test]
async fn skips_a_key_that_belongs_to_another_workspace() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &enabled(),
        vec![avatar_job_with_key(
            "public/space/ws-2/contacts/c-9/avatar/stolen",
        )],
    )
    .await;

    assert!(fake.bulks().is_empty());
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:196 "enqueueStoredImageOptimizations > enqueues nothing when STORED_IMAGE_OPTIMIZATION=off"
#[tokio::test]
async fn enqueues_nothing_when_stored_image_optimization_off() {
    let env: BTreeMap<String, String> =
        [("STORED_IMAGE_OPTIMIZATION".to_owned(), "off".to_owned())]
            .into_iter()
            .collect();
    let config = StoredImageOptimizationConfig::from_env_map(&env);
    let fake = FakeEnqueuer::new();

    enqueue_stored_image_optimizations(
        fake.as_ref(),
        &config,
        vec![attachment_job(), avatar_job()],
    )
    .await;

    assert!(fake.bulks().is_empty());
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:204 "enqueueStoredImageOptimizations > delays each job so a caller-owned transaction can commit first"
#[tokio::test]
async fn delays_each_job_so_a_caller_owned_transaction_can_commit() {
    let fake = FakeEnqueuer::new();
    enqueue_stored_image_optimizations(fake.as_ref(), &enabled(), vec![avatar_job()]).await;

    let bulks = fake.bulks();
    assert_eq!(bulks[0].1[0].opts.delay_ms, Some(5000));
}

/// Port of: packages/worker-config/__tests__/stored-image-optimization.test.ts:213 "enqueueStoredImageOptimizations > never throws when the queue is unavailable"
#[tokio::test]
async fn never_throws_when_the_queue_is_unavailable() {
    let fake = FakeEnqueuer::failing(JobsError::Unavailable("redis down".to_owned()));

    // Returns `()`: there is no error to throw.
    let () =
        enqueue_stored_image_optimizations(fake.as_ref(), &enabled(), vec![attachment_job()]).await;
}
