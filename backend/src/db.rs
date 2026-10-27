use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::AppResult;
use crate::util::now_iso;

pub async fn init_db(path: &Path) -> AppResult<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub async fn ensure_default_settings(pool: &SqlitePool) -> AppResult<()> {
    let api_key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let defaults = [
        ("api_key", api_key),
        ("api_port", "4477".to_string()),
        ("active_account_id", String::new()),
        ("timezone_mode", "broker".to_string()),
        ("ea_version", "1.0.0".to_string()),
    ];

    for (key, value) in defaults {
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn get_setting(pool: &SqlitePool, key: &str) -> AppResult<Option<String>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|row| row.get::<String, _>("value")))
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_api_port(pool: &SqlitePool) -> AppResult<u16> {
    Ok(get_setting(pool, "api_port")
        .await?
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4477))
}

pub async fn touch_account_heartbeat(
    pool: &SqlitePool,
    account_id: &str,
    offset: i64,
    balance: f64,
    equity: f64,
) -> AppResult<()> {
    let now = now_iso();
    sqlx::query(
        "UPDATE accounts SET broker_utc_offset_seconds = ?, balance = ?, equity = ?,
         last_heartbeat_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(offset)
    .bind(balance)
    .bind(equity)
    .bind(&now)
    .bind(&now)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_snapshot(
    pool: &SqlitePool,
    account_id: &str,
    balance: f64,
    equity: f64,
    margin: f64,
    free_margin: f64,
) -> AppResult<()> {
    let last = sqlx::query(
        "SELECT balance, equity FROM account_snapshots WHERE account_id = ?
         ORDER BY taken_at DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    let should_insert = match last {
        None => true,
        Some(row) => {
            let last_balance = row.get::<f64, _>("balance");
            let last_equity = row.get::<f64, _>("equity");
            (last_balance - balance).abs() > 0.001 || (last_equity - equity).abs() > 0.001
        }
    };

    if should_insert {
        sqlx::query(
            "INSERT INTO account_snapshots
             (id, account_id, taken_at, balance, equity, margin, free_margin)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(account_id)
        .bind(now_iso())
        .bind(balance)
        .bind(equity)
        .bind(margin)
        .bind(free_margin)
        .execute(pool)
        .await?;
    }
    Ok(())
}
