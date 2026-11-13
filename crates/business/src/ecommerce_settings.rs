//! `ecommerce-settings/service.ts` (get / update / dropPaymentMethod /
//! countProductsInOtherCurrency / resolveProFeatures) and the
//! `ecommerceSettingsSchema` checks of `database/partials/ecommerce.ts`.
//!
//! Settings are JSON objects with the four groups `general`, `payments`,
//! `shipping`, `checkout` (the TS `EcommerceSettings` shape, money in major
//! units); the row stores money x 10,000.

use chatbotx_db::Db;
use serde_json::Value;

use crate::Result;

/// `ECOMMERCE_SETTINGS_DEFAULTS`.
pub fn defaults() -> Value {
    todo!("P3.6-D2")
}

/// `resolveProFeatures` (community edition: every feature on).
pub fn pro_features() -> Value {
    todo!("P3.6-D2")
}

/// `updateEcommerceSettingsPublicRequest` + `toEcommerceSettingsPatch`: the
/// partial groups, normalized (codes upper-case, buttons ordered); `Err` =
/// the zod issues (paths like `["payments", "currency"]`).
pub fn parse_patch(_input: &Value) -> std::result::Result<Value, Vec<Value>> {
    todo!("P3.6-D2")
}

/// `ecommerceSettingsService.get` (no cache): the stored settings or the
/// defaults.
pub async fn get(_db: &Db, _workspace_id: i64) -> Result<Value> {
    todo!("P3.6-D2")
}

/// `ecommerceSettingsService.update`: in one transaction, insert the default
/// row when missing, merge the patch groups, check the merged settings
/// (schema, external store, coherence, online payment methods) and write
/// every column. Returns the merged settings.
pub async fn update(_db: &Db, _workspace_id: i64, _patch: Value) -> Result<Value> {
    todo!("P3.6-D2")
}

/// `ecommerceSettingsService.dropPaymentMethod`: remove `method`; an empty
/// list becomes `["cod"]`.
pub async fn drop_payment_method(_db: &Db, _workspace_id: i64, _method: &str) -> Result<()> {
    todo!("P3.6-D2")
}

/// `ecommerceSettingsRepository.countProductsInOtherCurrency`.
pub async fn count_products_in_other_currency(
    _db: &Db,
    _workspace_id: i64,
    _currency: &str,
) -> Result<i64> {
    todo!("P3.6-D2")
}
