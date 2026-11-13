//! Port of the parts of `partials/google-ads.ts` that the credential and settings
//! partials use.

use serde::{Deserialize, Serialize};

/// `googleAdsUploadMethodSchema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsUploadMethod {
    DataManager,
    Legacy,
}
