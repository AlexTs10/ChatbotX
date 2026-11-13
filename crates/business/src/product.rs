//! `product/service.ts` (`productService` + the variant-option, variant and
//! addon services). Template edit locks are not checked (DECISIONS, P3.6-D1).

use chatbotx_db::Db;
use chatbotx_db::repo::models::Product;
use serde_json::{Map, Value};

pub use chatbotx_db::repo::product::{ProductDetail, ProductListInput, ProductPage};

use crate::errors::Result;

/// `ProductImageInput` (`mode` is stored as the image `type`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInput {
    pub mode: String,
    pub url: String,
}

/// `ProductWriteData` (all optional: the patch form). `Some(None)` = null.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductData {
    pub name: Option<String>,
    pub short_description: Option<Option<String>>,
    pub long_description: Option<Option<String>>,
    pub price: Option<f64>,
    pub taxes: Option<f64>,
    pub discount: Option<f64>,
    pub currency: Option<String>,
    pub product_url: Option<Option<String>>,
    pub sku: Option<Option<String>>,
    pub inventory_policy: Option<String>,
    pub inventory_quantity: Option<i64>,
    pub allow_out_of_stock_purchase: Option<bool>,
    pub images: Option<Vec<ImageInput>>,
    pub tags: Option<Vec<String>>,
    pub vendor_id: Option<Option<i64>>,
    pub vendor: Option<Option<String>>,
    pub rank: Option<i64>,
    pub category_id: Option<Option<i64>>,
    pub subcategory_id: Option<Option<i64>>,
    pub status: Option<String>,
    pub is_searchable: Option<bool>,
    pub allow_special_request: Option<bool>,
    pub is_addon_only: Option<bool>,
}

/// `ProductVariantOptionInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantOptionInput {
    pub name: String,
    pub values: Vec<String>,
    pub position: f64,
}

/// `ProductVariantInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantInput {
    pub combination: Map<String, Value>,
    pub price: f64,
    pub is_enabled: bool,
}

/// `ProductAddonInput` (ids as sent: strings).
#[derive(Debug, Clone, PartialEq)]
pub struct AddonInput {
    pub name: String,
    pub max_selections: i64,
    pub addon_product_ids: Vec<String>,
}

/// `ProductFullWriteData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FullData {
    pub product: ProductData,
    pub variant_options: Vec<VariantOptionInput>,
    pub variants: Vec<VariantInput>,
    pub addons: Vec<AddonInput>,
}

/// `create`: checks the category references, then inserts.
pub async fn create(db: &Db, workspace_id: i64, data: ProductData) -> Result<Product> {
    let _ = (db, workspace_id, data);
    todo!("P3.6-D1")
}

/// `createFull`: product, variant options, variants and addons in one transaction.
pub async fn create_full(db: &Db, workspace_id: i64, data: FullData) -> Result<Product> {
    let _ = (db, workspace_id, data);
    todo!("P3.6-D1")
}

/// `update`: a patch (only the sent fields).
pub async fn update(db: &Db, workspace_id: i64, product_id: i64, data: ProductData) -> Result<()> {
    let _ = (db, workspace_id, product_id, data);
    todo!("P3.6-D1")
}

/// `updateFull`: the patch, then the child rows replaced.
pub async fn update_full(
    db: &Db,
    workspace_id: i64,
    product_id: i64,
    data: FullData,
) -> Result<()> {
    let _ = (db, workspace_id, product_id, data);
    todo!("P3.6-D1")
}

/// `delete`: drops the ids from discount codes, then deletes (other
/// workspaces' ids are ignored).
pub async fn delete(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D1")
}

/// `findById` (`Product does not exist.`).
pub async fn find_by_id(db: &Db, workspace_id: i64, id: i64) -> Result<ProductDetail> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D1")
}

/// `list`: the store source of the workspace unless given.
pub async fn list(db: &Db, input: ProductListInput) -> Result<ProductPage> {
    let _ = (db, input);
    todo!("P3.6-D1")
}
