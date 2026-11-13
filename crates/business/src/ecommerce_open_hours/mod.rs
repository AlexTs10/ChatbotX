//! `packages/business/src/ecommerce-open-hours`: named open-hours schedules of
//! the store and the "is it open now" check.

pub mod is_open;
pub mod schedule;
pub mod service;

pub use is_open::is_within_open_hours;
pub use schedule::{OpenHoursRange, OpenHoursSchedule, parse_schedule};
