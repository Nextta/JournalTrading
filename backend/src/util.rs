use chrono::{DateTime, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Converts the broker server timestamp (seconds since epoch as if it were UTC)
/// into the two representations used by the app.
pub fn broker_time_pair(epoch: i64, offset_seconds: i64) -> (String, String) {
    let server = DateTime::from_timestamp(epoch, 0).unwrap_or_else(Utc::now);
    let utc = server - chrono::Duration::seconds(offset_seconds);
    (
        server.format("%Y-%m-%dT%H:%M:%S").to_string(),
        utc.to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}

pub fn broker_date_from_string(value: &str) -> Option<String> {
    if value.len() >= 10 {
        Some(value[0..10].to_string())
    } else {
        None
    }
}

pub fn parse_server_epoch(value: &str) -> Option<i64> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|naive| Utc.from_utc_datetime(&naive).timestamp())
}

pub fn validate_date(value: &str) -> AppResult<()> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| AppError::Validation("La fecha debe tener formato YYYY-MM-DD".into()))
}

pub fn sanitize_html(html: &str) -> String {
    ammonia::clean(html)
}

pub fn round_money(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
