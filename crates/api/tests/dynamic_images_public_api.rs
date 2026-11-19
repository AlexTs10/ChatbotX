//! `apps/builder/__tests__/dynamic-images-public-api.test.ts` and
//! `packages/business/__tests__/dynamic-image-avatar-user.test.ts` (Q.D13b):
//! the routes run on a migrated database and an in-memory store; the
//! rendered backgrounds are decoded and checked pixel by pixel.

mod handlers_common;

use std::sync::Arc;

use chatbotx_files::{PutOptions, Uploader};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

const RED: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

async fn with_files() -> (Fixture, Arc<Uploader>) {
    let mut fx = fixture().await;
    let files = Arc::new(Uploader::in_memory());
    fx.state = fx.state.clone().with_files(Arc::clone(&files));
    (fx, files)
}

/// A solid 4x4 PNG at `public/<name>` and its public storage URL.
async fn solid(files: &Uploader, name: &str, rgba: [u8; 4]) -> String {
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba(rgba));
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    files
        .put_object(
            &format!("public/{name}"),
            bytes,
            PutOptions::default(),
            None,
        )
        .await
        .unwrap();
    format!("http://app/storage/public/{name}")
}

async fn background(files: &Uploader, out: &Value) -> image::RgbaImage {
    let url = out["backgroundUrl"].as_str().expect("backgroundUrl");
    let key = url
        .strip_prefix("http://app/storage/")
        .expect("storage URL");
    let bytes = files.get_object(key).await.expect("background object");
    image::load_from_memory(&bytes).unwrap().to_rgba8()
}

fn image_doc(url: &str) -> Value {
    json!({
        "width": 100,
        "height": 80,
        "elements": [{ "id": "img-1", "type": "image", "imageType": "url", "url": url,
                       "x": 10, "y": 10, "width": 40, "height": 40 }],
    })
}

#[tokio::test]
async fn create_renders_the_background_and_publishes_the_trigger_url() {
    let (fx, files) = with_files().await;
    let url = solid(&files, "logo.png", RED).await;
    let (status, out) = call(
        &fx,
        "dynamicImages.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Banner", "data": image_doc(&url) })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    let mut keys: Vec<&String> = out.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "backgroundUrl",
            "createdAt",
            "customFieldId",
            "data",
            "enabled",
            "id",
            "imageUrl",
            "name",
            "updatedAt"
        ]
    );
    let id = out["id"].as_str().unwrap();
    assert_eq!(
        out["imageUrl"],
        json!(format!(
            "http://app/dynamic-images?dynamicImageId={id}&userId={{{{user_id}}}}"
        ))
    );
    // zod defaults are stored.
    assert_eq!(out["data"]["elements"][0]["priority"], json!(false));
    assert_eq!(out["data"]["elements"][0]["imageStyle"], json!("square"));
    assert!(
        out["backgroundUrl"].as_str().unwrap().starts_with(&format!(
            "http://app/storage/public/space/{WS}/dynamic-images/{id}/background_"
        )),
        "{out}"
    );
    let png = background(&files, &out).await;
    assert_eq!(png.dimensions(), (200, 160));
    assert_eq!(png.get_pixel(60, 60).0, RED);
    assert_eq!(png.get_pixel(25, 25).0, RED);
    assert_eq!(png.get_pixel(5, 5).0, CLEAR);
    assert_eq!(png.get_pixel(150, 120).0, CLEAR);
}

#[tokio::test]
async fn text_and_qr_layers_are_baked_and_dynamic_layers_are_not() {
    let (fx, files) = with_files().await;
    let (status, out) = call(
        &fx,
        "dynamicImages.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Text", "data": {
            "width": 120, "height": 90,
            "elements": [
                { "id": "t-1", "type": "text", "text": "Hello", "x": 0, "y": 0, "width": 120, "height": 30,
                  "fontSize": 20, "fontFamily": "roboto", "align": "center", "color": "#111111" },
                { "id": "q-1", "type": "qrCode", "text": "parity", "x": 30, "y": 30, "width": 64,
                  "height": 64, "size": 64, "color": "#000000" },
            ],
        }})),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(
        out["data"]["elements"][0],
        json!({ "id": "t-1", "type": "text", "text": "Hello", "x": 0, "y": 0, "width": 120, "height": 30,
                "fontSize": 20, "fontFamily": "roboto", "align": "center", "color": "#111111",
                "priority": false, "bold": false, "italic": false, "uppercase": false })
    );
    let png = background(&files, &out).await;
    assert_eq!(png.dimensions(), (240, 180));
    assert_eq!(png.get_pixel(62, 62).0, [255, 255, 255, 255]);
    assert_eq!(png.get_pixel(10, 150).0, CLEAR);
    assert!(
        (0..60).any(|y| (0..240).any(|x| png.get_pixel(x, y).0[3] > 0)),
        "the text is drawn"
    );

    let late = solid(&files, "late.png", RED).await;
    let (status, out) = call(
        &fx,
        "dynamicImages.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Dynamic", "data": {
            "width": 50, "height": 50,
            "elements": [
                { "id": "t-1", "type": "text", "text": "Hi {{first_name}}", "x": 0, "y": 0, "width": 50,
                  "height": 20, "fontSize": 12, "fontFamily": "arial", "align": "left", "color": "#000000" },
                { "id": "img-1", "type": "image", "imageType": "url", "url": late, "x": 0, "y": 0,
                  "width": 50, "height": 50 },
            ],
        }})),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    let png = background(&files, &out).await;
    assert_eq!(png.dimensions(), (100, 100));
    assert_eq!(png.get_pixel(50, 50).0, CLEAR);
}

#[tokio::test]
async fn invalid_templates_and_duplicate_names() {
    let (fx, _files) = with_files().await;
    let data = json!({ "width": 10, "height": 10, "elements": [] });
    let path_of = |out: &Value| out["data"]["issues"][0]["path"].clone();
    for (body, path) in [
        (json!({ "name": "", "data": data }), json!(["name"])),
        (
            json!({ "name": "x".repeat(101), "data": data }),
            json!(["name"]),
        ),
        (json!({ "name": "n" }), json!(["data"])),
        (
            json!({ "name": "n", "data": { "width": 0, "height": 10, "elements": [] } }),
            json!(["data", "width"]),
        ),
        (
            json!({ "name": "n", "data": { "width": 10, "height": 1.5, "elements": [] } }),
            json!(["data", "height"]),
        ),
        (
            json!({ "name": "n", "data": { "width": 10, "height": 10, "elements": [
                { "id": "x", "type": "video", "x": 0, "y": 0, "width": 1, "height": 1 }] } }),
            json!(["data", "elements", 0, "type"]),
        ),
        (
            json!({ "name": "n", "data": { "width": 10, "height": 10, "elements": [
                { "id": "q", "type": "qrCode", "text": "t", "x": 0, "y": 0, "width": 1, "height": 1,
                  "size": 63, "color": "#000" }] } }),
            json!(["data", "elements", 0, "size"]),
        ),
        (
            json!({ "name": "n", "customFieldId": "abc", "data": data }),
            json!(["customFieldId"]),
        ),
    ] {
        let (status, out) = call(&fx, "dynamicImages.create", WS, &[], None, Some(body)).await;
        assert_eq!((status, path_of(&out)), (422, path), "{out}");
    }
    let body = json!({ "name": "Same", "data": data });
    let (status, _) = call(
        &fx,
        "dynamicImages.create",
        WS,
        &[],
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, 201);
    let (status, out) = call(&fx, "dynamicImages.create", WS, &[], None, Some(body)).await;
    assert_eq!(
        (status, out["code"].clone()),
        (500, json!("INTERNAL_SERVER_ERROR"))
    );
}

#[tokio::test]
async fn get_list_update_enable_and_delete_stay_in_the_workspace() {
    let (fx, files) = with_files().await;
    let data = json!({ "width": 10, "height": 10, "elements": [] });
    let mut ids = Vec::new();
    for name in ["Alpha", "Beta"] {
        let (status, out) = call(
            &fx,
            "dynamicImages.create",
            WS,
            &[],
            None,
            Some(json!({ "name": name, "data": data })),
        )
        .await;
        assert_eq!(status, 201, "{out}");
        ids.push(out);
    }
    let a = ids[0].clone();
    let a_id = a["id"].as_str().unwrap().to_owned();
    let (status, got) = call(&fx, "dynamicImages.get", WS, &[("id", &a_id)], None, None).await;
    assert_eq!((status, &got), (200, &a));
    let (status, _) = call(
        &fx,
        "dynamicImages.get",
        OTHER_WS,
        &[("id", &a_id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(&fx, "dynamicImages.get", WS, &[("id", "abc")], None, None).await;
    assert_eq!(status, 422);

    let (_, listed) = call(&fx, "dynamicImages.list", WS, &[], Some("name=ALP"), None).await;
    assert_eq!(listed, json!({ "data": [a], "pageCount": 1 }));
    let (_, none) = call(&fx, "dynamicImages.list", WS, &[], Some("name=zzz"), None).await;
    assert_eq!(none, json!({ "data": [], "pageCount": 0 }));
    let (_, paged) = call(&fx, "dynamicImages.list", WS, &[], Some("perPage=1"), None).await;
    assert_eq!(paged["pageCount"], json!(2));
    let (status, _) = call(&fx, "dynamicImages.list", WS, &[], Some("perPage=51"), None).await;
    assert_eq!(status, 422);

    // Update: a new background, the old one is removed.
    let blue = solid(&files, "blue.png", [0, 0, 255, 255]).await;
    let (status, updated) = call(
        &fx,
        "dynamicImages.update",
        WS,
        &[("id", &a_id)],
        None,
        Some(json!({ "name": "Alpha 2", "data": {
            "width": 60, "height": 60,
            "elements": [{ "id": "i", "type": "image", "imageType": "url", "url": blue,
                           "x": 0, "y": 0, "width": 60, "height": 60 }],
        }})),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_ne!(updated["backgroundUrl"], a["backgroundUrl"]);
    assert_eq!(
        background(&files, &updated).await.get_pixel(30, 30).0,
        [0, 0, 255, 255]
    );
    let old_key = a["backgroundUrl"]
        .as_str()
        .unwrap()
        .strip_prefix("http://app/storage/")
        .unwrap()
        .to_owned();
    assert!(files.get_object(&old_key).await.is_err());
    let (status, _) = call(
        &fx,
        "dynamicImages.update",
        WS,
        &[("id", &a_id)],
        None,
        Some(json!({ "name": "x" })),
    )
    .await;
    assert_eq!(status, 422);

    let (status, disabled) = call(
        &fx,
        "dynamicImages.setEnabled",
        WS,
        &[("id", &a_id)],
        None,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!((status, disabled["enabled"].clone()), (200, json!(false)));
    let (status, _) = call(
        &fx,
        "dynamicImages.setEnabled",
        WS,
        &[("id", "1")],
        None,
        Some(json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, 404);

    let (status, _) = call(
        &fx,
        "dynamicImages.delete",
        OTHER_WS,
        &[("id", &a_id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &fx,
        "dynamicImages.delete",
        WS,
        &[("id", &a_id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let key = updated["backgroundUrl"]
        .as_str()
        .unwrap()
        .strip_prefix("http://app/storage/")
        .unwrap()
        .to_owned();
    assert!(files.get_object(&key).await.is_err());
    let (status, _) = call(&fx, "dynamicImages.get", WS, &[("id", &a_id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn avatar_layer_uses_the_stored_picture_or_the_placeholder() {
    use chatbotx_business::contact::avatar::TenantUrls;
    use chatbotx_business::dynamic_image::contact_avatar_url;
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            for (id, avatar) in [
                (1_i64, Some("public/ws/2001/avatars/abc")),
                (2, Some("public/img/no_avatar.jpg?time=1")),
                (3, None),
            ] {
                tx.execute(
                    r#"INSERT INTO "Contact" ("id", "workspaceId", "avatar") VALUES (?1, ?2, ?3)"#,
                    chatbotx_db::rusqlite::params![id, WS, avatar],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let urls = TenantUrls::from_app_url(
        "https://app.example.com",
        Some("https://storage.example.com"),
    );
    let at = |id| contact_avatar_url(&fx.state.db, WS, id, &urls);
    assert_eq!(
        at(1).await.unwrap().as_deref(),
        Some("https://storage.example.com/public/ws/2001/avatars/abc")
    );
    assert_eq!(
        at(2).await.unwrap().as_deref(),
        Some("https://app.example.com/media/default-avatar.svg")
    );
    assert_eq!(at(3).await.unwrap(), None);
    // Another workspace's contact has no picture here.
    assert_eq!(
        contact_avatar_url(&fx.state.db, OTHER_WS, 1, &urls)
            .await
            .unwrap(),
        None
    );
}
