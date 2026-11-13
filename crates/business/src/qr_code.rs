//! `qr-code/qr-code.service.ts` (`qrCodeService`): `"Reflink"` rows of type
//! `qrCode`, the name stored with a `qr_` prefix and the size in `qrStyles`.
//! No read cache and no template locks in the Rust port.

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::{Flow, Reflink};

use crate::errors::Result;

/// `CreateQrCodeData` / `UpdateQrCodeData`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QrCodeData {
    pub name: Option<String>,
    pub flow_id: Option<i64>,
    pub custom_field_id: Option<Option<i64>>,
    pub size: Option<i64>,
}

/// `ListQrCodesInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQrCodes {
    pub workspace_id: i64,
    pub page: Option<i64>,
    pub per_page: i64,
    pub sort: Option<Vec<SortItem>>,
    pub keyword: Option<String>,
}

/// One listed QR code with its flow.
#[derive(Debug, Clone, PartialEq)]
pub struct QrCodeRow {
    pub qr_code: Reflink,
    pub flow: Flow,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct QrCodePage {
    pub data: Vec<QrCodeRow>,
    pub page_count: i64,
}

/// `find`.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<Reflink>> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D1")
}

/// `findOrFail` (`QR Code not found`).
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Reflink> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D1")
}

/// `create`: the new id.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    data: QrCodeData,
    duplicate_name_message: &str,
) -> Result<i64> {
    let _ = (db, workspace_id, data, duplicate_name_message);
    todo!("P3.6-D1")
}

/// `update`.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    data: QrCodeData,
    duplicate_name_message: &str,
) -> Result<Reflink> {
    let _ = (db, workspace_id, id, data, duplicate_name_message);
    todo!("P3.6-D1")
}

/// `deleteMany` (QR codes only).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D1")
}

/// `list`: QR codes with their flow, paged.
pub async fn list(db: &Db, input: ListQrCodes) -> Result<QrCodePage> {
    let _ = (db, input);
    todo!("P3.6-D1")
}
