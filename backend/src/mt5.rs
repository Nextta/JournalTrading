use std::collections::HashSet;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use tauri::Emitter;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::models::{Account, CandleBar};
use crate::services;
use crate::state::AppState;
use crate::util::{broker_date_from_string, broker_time_pair, new_id, now_iso};

type ApiError = (StatusCode, Json<Value>);

fn api_error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (
        status,
        Json(json!({ "ok": false, "error": message.into() })),
    )
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    log::error!("Error en el puente MT5: {error}");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/mt5/heartbeat", post(heartbeat))
        .route("/api/v1/mt5/sync", post(sync))
        .route("/api/v1/mt5/series", post(series))
        .route("/api/v1/mt5/events", post(events))
        .route("/api/v1/mt5/commands/result", post(command_result))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(state)
}

async fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state.current_api_key().await;
    let from_header = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string());
    let from_bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|value| value.trim().to_string());
    let provided = from_header.or(from_bearer).unwrap_or_default();
    if expected.is_empty() || provided != expected {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "API key inválida. Revisa la configuración del EA.",
        ));
    }
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "app": "Plan de Trading",
        "version": env!("CARGO_PKG_VERSION"),
        "time": now_iso(),
    }))
}

// ---------------------------------------------------------------------------
// Payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct HeartbeatPayload {
    login: i64,
    server: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    leverage: Option<i64>,
    #[serde(default)]
    broker_utc_offset_seconds: Option<i64>,
    #[serde(default)]
    balance: f64,
    #[serde(default)]
    equity: f64,
    #[serde(default)]
    margin: f64,
    #[serde(default)]
    free_margin: f64,
    #[serde(default)]
    ea_version: Option<String>,
    #[serde(default)]
    terminal_build: Option<i64>,
    #[serde(default)]
    terminal_data_path: Option<String>,
    #[serde(default)]
    server_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SyncPayload {
    login: i64,
    server: String,
    #[serde(default)]
    broker_utc_offset_seconds: Option<i64>,
    #[serde(default)]
    symbols: Vec<SymbolPayload>,
    #[serde(default)]
    positions: Vec<PositionPayload>,
    #[serde(default)]
    deals: Vec<DealPayload>,
}

#[derive(Debug, Deserialize)]
struct SymbolPayload {
    symbol: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_digits")]
    digits: i64,
    #[serde(default)]
    tick_size: f64,
    #[serde(default)]
    tick_value: f64,
    #[serde(default)]
    contract_size: f64,
    #[serde(default)]
    currency_profit: String,
}

fn default_digits() -> i64 {
    5
}

#[derive(Debug, Deserialize)]
struct PositionPayload {
    ticket: i64,
    #[serde(default)]
    symbol: String,
    #[serde(rename = "type")]
    kind: i32,
    #[serde(default)]
    volume: f64,
    #[serde(default)]
    price_open: f64,
    #[serde(default)]
    sl: Option<f64>,
    #[serde(default)]
    tp: Option<f64>,
    #[serde(default)]
    profit: f64,
    #[serde(default)]
    swap: f64,
    #[serde(default)]
    commission: f64,
    #[serde(default)]
    magic: Option<i64>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    time_server: String,
    #[serde(default)]
    time: Option<i64>,
    #[serde(default)]
    time_utc: Option<String>,
    #[serde(default)]
    tick_size: Option<f64>,
    #[serde(default)]
    tick_value: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DealPayload {
    deal_ticket: i64,
    position_ticket: i64,
    #[serde(default)]
    symbol: String,
    #[serde(rename = "type")]
    kind: i32,
    entry: i64,
    #[serde(default)]
    volume: f64,
    #[serde(default)]
    price: f64,
    #[serde(default)]
    profit: f64,
    #[serde(default)]
    commission: f64,
    #[serde(default)]
    swap: f64,
    #[serde(default)]
    magic: Option<i64>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default = "default_deal_reason")]
    reason: i64,
    #[serde(default)]
    time_server: String,
    #[serde(default)]
    time: Option<i64>,
    #[serde(default)]
    time_utc: Option<String>,
}

fn default_deal_reason() -> i64 {
    -1
}

#[derive(Debug, Deserialize)]
struct SeriesPayload {
    login: i64,
    server: String,
    trade_ticket: i64,
    #[serde(default)]
    symbol: String,
    timeframe: String,
    #[serde(default)]
    from_time: i64,
    #[serde(default)]
    to_time: i64,
    #[serde(default)]
    bars: Vec<CandleBar>,
}

#[derive(Debug, Deserialize)]
struct EventPayload {
    #[serde(default)]
    login: Option<i64>,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct CommandResultPayload {
    command_id: String,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Time helpers
// ---------------------------------------------------------------------------

fn parse_naive(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    for format in [
        "%Y.%m.%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y.%m.%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, format) {
            return Some(parsed);
        }
    }
    None
}

fn naive_to_server(value: NaiveDateTime) -> String {
    value.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn naive_to_utc(value: NaiveDateTime) -> String {
    Utc.from_utc_datetime(&value)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_utc(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(
            parsed
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        );
    }
    parse_naive(value).map(naive_to_utc)
}

fn time_pair(
    time_server: &str,
    epoch: Option<i64>,
    time_utc: Option<&str>,
    offset_seconds: i64,
) -> (String, String) {
    let mut pair = if let Some(naive) = parse_naive(time_server) {
        let utc = naive - Duration::seconds(offset_seconds);
        (naive_to_server(naive), naive_to_utc(utc))
    } else if let Some(epoch) = epoch {
        broker_time_pair(epoch, offset_seconds)
    } else {
        let now = Utc::now();
        (
            now.naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string(),
            now.to_rfc3339_opts(SecondsFormat::Secs, true),
        )
    };
    if let Some(utc) = time_utc.and_then(parse_utc) {
        pair.1 = utc;
    }
    pair
}

fn date_of(value: &str) -> Option<String> {
    broker_date_from_string(value)
}

// ---------------------------------------------------------------------------
// Account and status helpers
// ---------------------------------------------------------------------------

async fn ensure_account(
    pool: &SqlitePool,
    login: i64,
    server: &str,
    display_name: Option<&str>,
    company: Option<&str>,
    currency: Option<&str>,
    leverage: Option<i64>,
) -> AppResult<(Account, bool)> {
    if login <= 0 {
        return Err(AppError::Validation("Login inválido".into()));
    }
    let server = server.trim();
    if server.is_empty() {
        return Err(AppError::Validation("Servidor vacío".into()));
    }

    if let Some(account) = sqlx::query_as::<_, Account>(
        "SELECT id, login, server, display_name, company, currency, leverage, status,
                broker_utc_offset_seconds, balance, equity, last_heartbeat_at, created_at, updated_at
         FROM accounts WHERE login = ? AND server = ?",
    )
    .bind(login)
    .bind(server)
    .fetch_optional(pool)
    .await?
    {
        let placeholder = format!("Cuenta {}", account.login);
        let display_name_value = match display_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value)
                if account.display_name.trim().is_empty() || account.display_name == placeholder =>
            {
                value.to_string()
            }
            _ => account.display_name.clone(),
        };
        let company_value = company
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(account.company.as_str());
        let currency_value = currency
            .map(str::trim)
            .filter(|value| value.len() == 3)
            .map(|value| value.to_uppercase())
            .unwrap_or_else(|| account.currency.clone());
        let leverage_value = leverage.or(account.leverage);
        sqlx::query(
            "UPDATE accounts SET display_name = ?, company = ?, currency = ?, leverage = ?, updated_at = ? WHERE id = ?",
        )
        .bind(display_name_value)
        .bind(company_value)
        .bind(currency_value)
        .bind(leverage_value)
        .bind(now_iso())
        .bind(&account.id)
        .execute(pool)
        .await?;
        return Ok((services::load_account(pool, &account.id).await?, false));
    }

    let id = new_id();
    let now = now_iso();
    let display_name = display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Cuenta {login}"));
    let company = company.unwrap_or("").trim().to_string();
    let currency = currency
        .map(str::trim)
        .filter(|value| value.len() == 3)
        .map(|value| value.to_uppercase())
        .unwrap_or_else(|| "USD".to_string());

    sqlx::query(
        "INSERT INTO accounts
         (id, login, server, display_name, company, currency, leverage, status,
          broker_utc_offset_seconds, balance, equity, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', 0, 0, 0, ?, ?)",
    )
    .bind(&id)
    .bind(login)
    .bind(server)
    .bind(&display_name)
    .bind(&company)
    .bind(&currency)
    .bind(leverage)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok((services::load_account(pool, &id).await?, true))
}

async fn mark_connected(
    state: &AppState,
    account: &Account,
    ea_version: Option<&str>,
    terminal_build: Option<i64>,
    terminal_data_path: Option<&str>,
) {
    let mut status = state.mt5.status.write().await;
    status.running = true;
    status.connected = true;
    status.error = None;
    status.last_heartbeat_at = Some(now_iso());
    status.login = Some(account.login);
    status.server = Some(account.server.clone());
    if let Some(value) = ea_version {
        status.ea_version = Some(value.to_string());
    }
    if let Some(value) = terminal_build {
        status.terminal_build = Some(value);
    }
    if let Some(value) = terminal_data_path {
        status.terminal_data_path = Some(value.to_string());
    }
    let snapshot = status.clone();
    drop(status);
    let _ = state.app.emit("mt5://status", snapshot);
}

async fn touch_sync(pool: &SqlitePool, account_id: &str, offset_seconds: i64) -> AppResult<()> {
    let now = now_iso();
    sqlx::query(
        "UPDATE accounts SET broker_utc_offset_seconds = ?, last_heartbeat_at = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(offset_seconds)
    .bind(&now)
    .bind(&now)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn take_commands(pool: &SqlitePool, account_id: &str) -> AppResult<Vec<Value>> {
    let cutoff = (Utc::now() - Duration::minutes(15)).to_rfc3339_opts(SecondsFormat::Secs, true);
    sqlx::query(
        "UPDATE mt5_commands SET status = 'failed', error = 'El EA no procesó el comando a tiempo',
         updated_at = ? WHERE account_id = ? AND status = 'sent' AND created_at < ?",
    )
    .bind(now_iso())
    .bind(account_id)
    .bind(&cutoff)
    .execute(pool)
    .await?;

    let rows = sqlx::query(
        "SELECT id, kind, payload_json FROM mt5_commands
         WHERE account_id = ? AND status IN ('pending', 'sent')
         ORDER BY created_at LIMIT 10",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    let mut commands = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let kind: String = row.get("kind");
        let raw: String = row.get("payload_json");
        let payload = serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw));
        commands.push(json!({ "id": id, "kind": kind, "payload": payload }));
        sqlx::query("UPDATE mt5_commands SET status = 'sent', updated_at = ? WHERE id = ?")
            .bind(now_iso())
            .bind(&id)
            .execute(pool)
            .await?;
    }
    Ok(commands)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<HeartbeatPayload>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers).await?;
    let (account, is_new) = ensure_account(
        &state.db,
        payload.login,
        &payload.server,
        payload.display_name.as_deref(),
        payload.company.as_deref(),
        payload.currency.as_deref(),
        payload.leverage,
    )
    .await
    .map_err(internal)?;

    let offset = payload
        .broker_utc_offset_seconds
        .unwrap_or(account.broker_utc_offset_seconds);
    db::touch_account_heartbeat(
        &state.db,
        &account.id,
        offset,
        payload.balance,
        payload.equity,
    )
    .await
    .map_err(internal)?;
    db::insert_snapshot(
        &state.db,
        &account.id,
        payload.balance,
        payload.equity,
        payload.margin,
        payload.free_margin,
    )
    .await
    .map_err(internal)?;

    mark_connected(
        &state,
        &account,
        payload.ea_version.as_deref(),
        payload.terminal_build,
        payload.terminal_data_path.as_deref(),
    )
    .await;

    let commands = take_commands(&state.db, &account.id)
        .await
        .map_err(internal)?;
    if is_new {
        let _ = state
            .app
            .emit("accounts://updated", json!({ "accountId": account.id }));
    }
    let _ = state.app.emit(
        "mt5://heartbeat",
        json!({
            "accountId": account.id,
            "login": account.login,
            "server": account.server,
            "serverTime": payload.server_time,
            "offset": offset,
            "balance": payload.balance,
            "equity": payload.equity,
        }),
    );

    Ok(Json(json!({
        "ok": true,
        "apiVersion": 1,
        "accountId": account.id,
        "accountStatus": account.status,
        "pendingConfirmation": account.status == "pending",
        "newAccount": is_new,
        "brokerUtcOffsetSeconds": offset,
        "commands": commands,
        "serverTime": now_iso(),
    })))
}

async fn sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SyncPayload>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers).await?;
    let (account, is_new) = ensure_account(
        &state.db,
        payload.login,
        &payload.server,
        None,
        None,
        None,
        None,
    )
    .await
    .map_err(internal)?;
    let offset = payload
        .broker_utc_offset_seconds
        .unwrap_or(account.broker_utc_offset_seconds);
    touch_sync(&state.db, &account.id, offset)
        .await
        .map_err(internal)?;
    mark_connected(&state, &account, None, None, None).await;

    let stats = ingest_sync(&state.db, &account.id, offset, &payload)
        .await
        .map_err(internal)?;

    let _ = state.app.emit(
        "trades://updated",
        json!({
            "accountId": account.id,
            "positions": stats.positions,
            "deals": stats.deals,
            "newTrades": stats.new_trades,
        }),
    );
    let _ = state
        .app
        .emit("journal://updated", json!({ "accountId": account.id }));
    if is_new {
        let _ = state
            .app
            .emit("accounts://updated", json!({ "accountId": account.id }));
    }

    Ok(Json(json!({
        "ok": true,
        "accountId": account.id,
        "accountStatus": account.status,
        "positions": stats.positions,
        "deals": stats.deals,
        "newTrades": stats.new_trades,
        "serverTime": now_iso(),
    })))
}

async fn series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SeriesPayload>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers).await?;
    let (account, _) = ensure_account(
        &state.db,
        payload.login,
        &payload.server,
        None,
        None,
        None,
        None,
    )
    .await
    .map_err(internal)?;
    let timeframe = payload.timeframe.trim().to_uppercase();
    if !crate::models::TIMEFRAMES.contains(&timeframe.as_str()) {
        return Err(api_error(StatusCode::BAD_REQUEST, "Timeframe no válido"));
    }
    if payload.bars.len() > 200_000 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "La serie de velas es demasiado grande",
        ));
    }

    let row = sqlx::query("SELECT id FROM trades WHERE account_id = ? AND position_ticket = ?")
        .bind(&account.id)
        .bind(payload.trade_ticket)
        .fetch_optional(&state.db)
        .await
        .map_err(internal)?;
    let Some(row) = row else {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "La operación no existe todavía en el diario",
        ));
    };
    let trade_id: String = row.get("id");
    let bars_json = serde_json::to_string(&payload.bars).map_err(internal)?;
    let now = now_iso();
    sqlx::query(
        "INSERT INTO trade_series
         (id, trade_id, timeframe, from_time, to_time, bar_count, bars_json, fetched_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(trade_id, timeframe) DO UPDATE SET
           from_time = excluded.from_time,
           to_time = excluded.to_time,
           bar_count = excluded.bar_count,
           bars_json = excluded.bars_json,
           fetched_at = excluded.fetched_at",
    )
    .bind(new_id())
    .bind(&trade_id)
    .bind(&timeframe)
    .bind(payload.from_time)
    .bind(payload.to_time)
    .bind(payload.bars.len() as i64)
    .bind(&bars_json)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(internal)?;

    let _ = state.app.emit(
        "series://updated",
        json!({
            "tradeId": trade_id,
            "timeframe": timeframe,
            "symbol": payload.symbol,
            "bars": payload.bars.len(),
        }),
    );

    Ok(Json(json!({
        "ok": true,
        "tradeId": trade_id,
        "timeframe": timeframe,
        "bars": payload.bars.len(),
    })))
}

async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<EventPayload>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers).await?;
    let mut account_id: Option<String> = None;
    if let (Some(login), Some(server)) = (payload.login, payload.server.as_deref())
        && login > 0
        && !server.trim().is_empty()
        && let Ok((account, _)) =
            ensure_account(&state.db, login, server, None, None, None, None).await
    {
        account_id = Some(account.id);
    }
    let level = payload.level.unwrap_or_else(|| "info".to_string());
    let kind = payload.kind.unwrap_or_else(|| "log".to_string());
    let now = now_iso();
    sqlx::query(
        "INSERT INTO mt5_event_log
         (id, account_id, event_type, status, payload_json, error, received_at)
         VALUES (?, ?, ?, ?, ?, NULL, ?)",
    )
    .bind(new_id())
    .bind(&account_id)
    .bind(&kind)
    .bind(&level)
    .bind(payload.data.as_ref().map(ToString::to_string))
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(internal)?;

    let _ = state.app.emit(
        "mt5://event",
        json!({
            "level": level,
            "kind": kind,
            "message": payload.message,
            "accountId": account_id,
            "receivedAt": now,
        }),
    );
    Ok(Json(json!({ "ok": true })))
}

async fn command_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CommandResultPayload>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers).await?;
    let status = if payload.ok { "done" } else { "failed" };
    sqlx::query("UPDATE mt5_commands SET status = ?, error = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(payload.error)
        .bind(now_iso())
        .bind(&payload.command_id)
        .execute(&state.db)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Ingestion
// ---------------------------------------------------------------------------

#[derive(Default)]
struct IngestStats {
    positions: i64,
    deals: i64,
    new_trades: i64,
}

async fn ingest_sync(
    pool: &SqlitePool,
    account_id: &str,
    offset_seconds: i64,
    payload: &SyncPayload,
) -> AppResult<IngestStats> {
    let mut stats = IngestStats::default();

    for symbol in &payload.symbols {
        if symbol.symbol.trim().is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO symbols
             (account_id, symbol, description, digits, tick_size, tick_value, contract_size,
              currency_profit, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, symbol) DO UPDATE SET
               description = excluded.description,
               digits = excluded.digits,
               tick_size = excluded.tick_size,
               tick_value = excluded.tick_value,
               contract_size = excluded.contract_size,
               currency_profit = excluded.currency_profit,
               updated_at = excluded.updated_at",
        )
        .bind(account_id)
        .bind(symbol.symbol.trim())
        .bind(symbol.description.trim())
        .bind(symbol.digits)
        .bind(symbol.tick_size)
        .bind(symbol.tick_value)
        .bind(symbol.contract_size)
        .bind(symbol.currency_profit.trim())
        .bind(now_iso())
        .execute(pool)
        .await?;
    }

    let open_tickets: HashSet<i64> = payload
        .positions
        .iter()
        .map(|position| position.ticket)
        .collect();
    let mut affected: HashSet<i64> = HashSet::new();
    let mut created_trades: HashSet<String> = HashSet::new();

    for deal in &payload.deals {
        if let Some((trade_id, created)) =
            upsert_deal(pool, account_id, offset_seconds, deal).await?
        {
            stats.deals += 1;
            affected.insert(deal.position_ticket);
            if created {
                created_trades.insert(trade_id);
            }
        }
    }

    for ticket in &affected {
        let row = sqlx::query("SELECT id FROM trades WHERE account_id = ? AND position_ticket = ?")
            .bind(account_id)
            .bind(ticket)
            .fetch_optional(pool)
            .await?;
        if let Some(row) = row {
            let trade_id: String = row.get("id");
            rebuild_trade_from_deals(pool, &trade_id, open_tickets.contains(ticket)).await?;
        }
    }

    for position in &payload.positions {
        if upsert_position(pool, account_id, offset_seconds, position).await? {
            created_trades.insert(
                sqlx::query("SELECT id FROM trades WHERE account_id = ? AND position_ticket = ?")
                    .bind(account_id)
                    .bind(position.ticket)
                    .fetch_one(pool)
                    .await?
                    .get("id"),
            );
        }
        stats.positions += 1;
    }

    stats.new_trades = created_trades.len() as i64;
    Ok(stats)
}

async fn find_trade_id(
    pool: &SqlitePool,
    account_id: &str,
    position_ticket: i64,
) -> AppResult<Option<String>> {
    let row = sqlx::query("SELECT id FROM trades WHERE account_id = ? AND position_ticket = ?")
        .bind(account_id)
        .bind(position_ticket)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|row| row.get::<String, _>("id")))
}

async fn upsert_deal(
    pool: &SqlitePool,
    account_id: &str,
    offset_seconds: i64,
    deal: &DealPayload,
) -> AppResult<Option<(String, bool)>> {
    if deal.position_ticket <= 0 || deal.deal_ticket <= 0 {
        return Ok(None);
    }
    if deal.kind != 0 && deal.kind != 1 {
        return Ok(None);
    }
    let (time_server, time_utc) = time_pair(
        &deal.time_server,
        deal.time,
        deal.time_utc.as_deref(),
        offset_seconds,
    );

    let (trade_id, created) = if let Some(trade_id) =
        find_trade_id(pool, account_id, deal.position_ticket).await?
    {
        (trade_id, false)
    } else {
        let symbol = if deal.symbol.trim().is_empty() {
            "UNKNOWN"
        } else {
            deal.symbol.trim()
        };
        let side = if deal.entry == 0 {
            if deal.kind == 0 { "buy" } else { "sell" }
        } else if deal.kind == 0 {
            "sell"
        } else {
            "buy"
        };
        let id = new_id();
        let now = now_iso();
        sqlx::query(
            "INSERT INTO trades
             (id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
              volume_current, open_time_server, open_time_utc, open_price, sl, tp,
              profit_gross, commission, swap, profit_net, magic, comment, notes, state,
              created_at, updated_at)
             VALUES (?, ?, NULL, ?, ?, ?, ?, 0, ?, ?, ?, NULL, NULL, 0, 0, 0, 0, ?, ?, '', 'open', ?, ?)",
        )
        .bind(&id)
        .bind(account_id)
        .bind(deal.position_ticket)
        .bind(symbol)
        .bind(side)
        .bind(deal.volume.abs())
        .bind(&time_server)
        .bind(&time_utc)
        .bind(deal.price)
        .bind(deal.magic)
        .bind(deal.comment.as_deref().unwrap_or(""))
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
        (id, true)
    };

    sqlx::query(
        "INSERT INTO deals
         (id, trade_id, account_id, deal_ticket, entry, type, volume, price, profit,
          commission, swap, time_server, time_utc, comment, reason)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id, deal_ticket) DO UPDATE SET
           trade_id = excluded.trade_id,
           entry = excluded.entry,
           type = excluded.type,
           volume = excluded.volume,
           price = excluded.price,
           profit = excluded.profit,
           commission = excluded.commission,
           swap = excluded.swap,
           time_server = excluded.time_server,
           time_utc = excluded.time_utc,
           comment = excluded.comment,
           reason = excluded.reason",
    )
    .bind(new_id())
    .bind(&trade_id)
    .bind(account_id)
    .bind(deal.deal_ticket)
    .bind(deal.entry)
    .bind(deal.kind)
    .bind(deal.volume.abs())
    .bind(deal.price)
    .bind(deal.profit)
    .bind(deal.commission)
    .bind(deal.swap)
    .bind(&time_server)
    .bind(&time_utc)
    .bind(deal.comment.as_deref().unwrap_or(""))
    .bind(deal.reason)
    .execute(pool)
    .await?;

    Ok(Some((trade_id, created)))
}

async fn rebuild_trade_from_deals(
    pool: &SqlitePool,
    trade_id: &str,
    is_open: bool,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT entry, type, volume, price, profit, commission, swap, time_server, time_utc
         FROM deals WHERE trade_id = ? ORDER BY time_server, deal_ticket",
    )
    .bind(trade_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }

    let trade = sqlx::query(
        "SELECT account_id, symbol, side, volume_initial, volume_current, open_time_server,
                open_time_utc, open_price, close_price, sl, risk_amount, state,
                close_time_server, close_time_utc
         FROM trades WHERE id = ?",
    )
    .bind(trade_id)
    .fetch_one(pool)
    .await?;

    let mut side: Option<String> = None;
    let mut open_volume = 0.0;
    let mut close_volume = 0.0;
    let mut open_value = 0.0;
    let mut close_value = 0.0;
    let mut profit_gross = 0.0;
    let mut commission = 0.0;
    let mut swap = 0.0;
    let mut first_open: Option<(String, String)> = None;
    let mut last_close: Option<(String, String)> = None;

    for row in rows {
        let entry: i64 = row.get("entry");
        let kind: i64 = row.get("type");
        let volume: f64 = row.get("volume");
        let price: f64 = row.get("price");
        profit_gross += row.get::<f64, _>("profit");
        commission += row.get::<f64, _>("commission");
        swap += row.get::<f64, _>("swap");
        let time_server: String = row.get("time_server");
        let time_utc: String = row.get("time_utc");

        if entry == 0 {
            open_volume += volume;
            open_value += price * volume;
            if side.is_none() {
                side = Some(if kind == 0 { "buy" } else { "sell" }.to_string());
            }
            if first_open
                .as_ref()
                .map(|current| time_server < current.0)
                .unwrap_or(true)
            {
                first_open = Some((time_server.clone(), time_utc.clone()));
            }
        } else if entry == 1 || entry == 3 {
            close_volume += volume;
            close_value += price * volume;
            if side.is_none() {
                side = Some(if kind == 0 { "sell" } else { "buy" }.to_string());
            }
            if last_close
                .as_ref()
                .map(|current| time_server > current.0)
                .unwrap_or(true)
            {
                last_close = Some((time_server, time_utc));
            }
        }
    }

    let open_volume_existing: f64 = trade.get("volume_initial");
    let open_price_existing: f64 = trade.get("open_price");
    let sl: Option<f64> = trade.get("sl");
    let account_id: String = trade.get("account_id");
    let symbol: String = trade.get("symbol");
    let side = side
        .or_else(|| trade.try_get::<String, _>("side").ok())
        .unwrap_or_else(|| "buy".to_string());
    let volume_initial = if open_volume > 0.0 {
        open_volume
    } else {
        open_volume_existing
    };
    let open_price = if open_volume > 0.0 {
        open_value / open_volume
    } else {
        open_price_existing
    };
    let close_price = if close_volume > 0.0 {
        Some(close_value / close_volume)
    } else {
        trade
            .try_get::<Option<f64>, _>("close_price")
            .ok()
            .flatten()
    };
    let profit_net = profit_gross + commission + swap;
    let state = if is_open { "open" } else { "closed" };

    let (open_time_server, open_time_utc) = first_open.unwrap_or_else(|| {
        (
            trade.get::<String, _>("open_time_server"),
            trade.get::<String, _>("open_time_utc"),
        )
    });
    let (close_time_server, close_time_utc): (Option<String>, Option<String>) = if is_open {
        (
            trade
                .try_get::<Option<String>, _>("close_time_server")
                .ok()
                .flatten(),
            trade
                .try_get::<Option<String>, _>("close_time_utc")
                .ok()
                .flatten(),
        )
    } else if let Some((server, utc)) = last_close {
        (Some(server), Some(utc))
    } else {
        (
            trade
                .try_get::<Option<String>, _>("close_time_server")
                .ok()
                .flatten(),
            trade
                .try_get::<Option<String>, _>("close_time_utc")
                .ok()
                .flatten(),
        )
    };
    let close_time_server = close_time_server.filter(|value| !value.is_empty());
    let close_time_utc = close_time_utc.filter(|value| !value.is_empty());

    let existing_risk: Option<f64> = trade
        .try_get::<Option<f64>, _>("risk_amount")
        .ok()
        .flatten();
    let risk = match existing_risk {
        Some(value) => Some(value),
        None => {
            risk_for(
                pool,
                &account_id,
                &symbol,
                None,
                None,
                volume_initial,
                open_price,
                sl,
            )
            .await?
        }
    };
    let r_multiple = risk
        .filter(|value| *value > 0.0)
        .map(|value| profit_net / value);

    sqlx::query(
        "UPDATE trades SET side = ?, volume_initial = ?, volume_current = ?,
         open_time_server = ?, open_time_utc = ?, close_time_server = ?, close_time_utc = ?,
         open_price = ?, close_price = ?, profit_gross = ?, commission = ?, swap = ?,
         profit_net = ?, risk_amount = ?, r_multiple = ?, state = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&side)
    .bind(volume_initial)
    .bind(if is_open {
        trade.get::<f64, _>("volume_current")
    } else {
        0.0
    })
    .bind(&open_time_server)
    .bind(&open_time_utc)
    .bind(close_time_server.as_deref())
    .bind(close_time_utc.as_deref())
    .bind(open_price)
    .bind(close_price)
    .bind(profit_gross)
    .bind(commission)
    .bind(swap)
    .bind(profit_net)
    .bind(risk)
    .bind(r_multiple)
    .bind(state)
    .bind(now_iso())
    .bind(trade_id)
    .execute(pool)
    .await?;

    let date: Option<String> = if is_open {
        date_of(&open_time_server)
    } else {
        close_time_server
            .as_deref()
            .and_then(date_of)
            .or_else(|| date_of(&open_time_server))
    };
    if let Some(date) = date {
        link_trade_day(pool, trade_id, &account_id, &symbol, &date).await?;
    }
    Ok(())
}

async fn upsert_position(
    pool: &SqlitePool,
    account_id: &str,
    offset_seconds: i64,
    position: &PositionPayload,
) -> AppResult<bool> {
    if position.ticket <= 0 || position.volume <= 0.0 || (position.kind != 0 && position.kind != 1)
    {
        return Ok(false);
    }
    let symbol = position.symbol.trim();
    if symbol.is_empty() {
        return Ok(false);
    }
    let side = if position.kind == 0 { "buy" } else { "sell" };
    let sl = position.sl.filter(|value| *value > 0.0);
    let tp = position.tp.filter(|value| *value > 0.0);
    let (time_server, time_utc) = time_pair(
        &position.time_server,
        position.time,
        position.time_utc.as_deref(),
        offset_seconds,
    );
    let risk = risk_for(
        pool,
        account_id,
        symbol,
        position.tick_size,
        position.tick_value,
        position.volume,
        position.price_open,
        sl,
    )
    .await?;
    let commission = position.commission;
    let swap = position.swap;
    let profit_net = position.profit + commission + swap;
    let now = now_iso();

    if let Some(row) = sqlx::query(
        "SELECT id, state, risk_amount FROM trades WHERE account_id = ? AND position_ticket = ?",
    )
    .bind(account_id)
    .bind(position.ticket)
    .fetch_optional(pool)
    .await?
    {
        let trade_id: String = row.get("id");
        let state: String = row.get("state");
        if state == "closed" {
            return Ok(false);
        }
        let existing_risk: Option<f64> =
            row.try_get::<Option<f64>, _>("risk_amount").ok().flatten();
        let risk_amount = existing_risk.or(risk);
        let r_multiple = risk_amount
            .filter(|value| *value > 0.0)
            .map(|value| profit_net / value);
        sqlx::query(
            "UPDATE trades SET symbol = ?, side = ?, volume_initial = ?,
             volume_current = ?, open_time_server = ?, open_time_utc = ?, open_price = ?,
             sl = ?, tp = ?, profit_gross = ?, commission = ?, swap = ?, profit_net = ?,
             magic = ?, comment = ?, risk_amount = ?, r_multiple = ?, state = 'open',
             close_time_server = NULL, close_time_utc = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(symbol)
        .bind(side)
        .bind(position.volume)
        .bind(position.volume)
        .bind(&time_server)
        .bind(&time_utc)
        .bind(position.price_open)
        .bind(sl)
        .bind(tp)
        .bind(position.profit)
        .bind(commission)
        .bind(swap)
        .bind(profit_net)
        .bind(position.magic)
        .bind(position.comment.as_deref().unwrap_or(""))
        .bind(risk_amount)
        .bind(r_multiple)
        .bind(&now)
        .bind(&trade_id)
        .execute(pool)
        .await?;

        if let Some(date) = date_of(&time_server) {
            link_trade_day(pool, &trade_id, account_id, symbol, &date).await?;
        }
        return Ok(false);
    }

    let id = new_id();
    sqlx::query(
        "INSERT INTO trades
         (id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
          volume_current, open_time_server, open_time_utc, open_price, sl, tp,
          profit_gross, commission, swap, profit_net, magic, comment, risk_amount,
          r_multiple, notes, state, created_at, updated_at)
         VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '', 'open', ?, ?)",
    )
    .bind(&id)
    .bind(account_id)
    .bind(position.ticket)
    .bind(symbol)
    .bind(side)
    .bind(position.volume)
    .bind(position.volume)
    .bind(&time_server)
    .bind(&time_utc)
    .bind(position.price_open)
    .bind(sl)
    .bind(tp)
    .bind(position.profit)
    .bind(commission)
    .bind(swap)
    .bind(profit_net)
    .bind(position.magic)
    .bind(position.comment.as_deref().unwrap_or(""))
    .bind(risk)
    .bind(risk.filter(|value| *value > 0.0).map(|value| profit_net / value))
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    if let Some(date) = date_of(&time_server) {
        link_trade_day(pool, &id, account_id, symbol, &date).await?;
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn risk_for(
    pool: &SqlitePool,
    account_id: &str,
    symbol: &str,
    tick_size: Option<f64>,
    tick_value: Option<f64>,
    volume: f64,
    open_price: f64,
    sl: Option<f64>,
) -> AppResult<Option<f64>> {
    let Some(sl) = sl else {
        return Ok(None);
    };
    let distance = (open_price - sl).abs();
    if distance <= 0.0 || volume <= 0.0 {
        return Ok(None);
    }

    let (tick_size, tick_value) = match (
        tick_size.filter(|value| *value > 0.0),
        tick_value.filter(|value| *value > 0.0),
    ) {
        (Some(size), Some(value)) => (size, value),
        _ => {
            let row = sqlx::query(
                "SELECT tick_size, tick_value FROM symbols WHERE account_id = ? AND symbol = ?",
            )
            .bind(account_id)
            .bind(symbol)
            .fetch_optional(pool)
            .await?;
            match row {
                Some(row) => (
                    row.get::<f64, _>("tick_size"),
                    row.get::<f64, _>("tick_value"),
                ),
                None => (0.0, 0.0),
            }
        }
    };

    if tick_size <= 0.0 || tick_value <= 0.0 {
        return Ok(None);
    }
    let value = distance / tick_size * tick_value * volume;
    if value.is_finite() && value > 0.0 {
        Ok(Some((value * 100.0).round() / 100.0))
    } else {
        Ok(None)
    }
}

async fn link_trade_day(
    pool: &SqlitePool,
    trade_id: &str,
    account_id: &str,
    symbol: &str,
    date: &str,
) -> AppResult<()> {
    if date.len() != 10 {
        return Ok(());
    }
    let journal = services::ensure_day_journal(pool, account_id, date).await?;
    let asset = services::ensure_day_asset(pool, &journal.id, symbol, None).await?;

    let strategy = sqlx::query("SELECT strategy_id FROM trades WHERE id = ?")
        .bind(trade_id)
        .fetch_optional(pool)
        .await?;
    if let Some(row) = strategy {
        let trade_strategy: Option<String> = row
            .try_get::<Option<String>, _>("strategy_id")
            .ok()
            .flatten();
        if trade_strategy.is_none()
            && let Some(asset_strategy) = &asset.strategy_id
        {
            sqlx::query("UPDATE trades SET strategy_id = ?, updated_at = ? WHERE id = ?")
                .bind(asset_strategy)
                .bind(now_iso())
                .bind(trade_id)
                .execute(pool)
                .await?;
        }
    }

    sqlx::query(
        "UPDATE trades SET day_asset_id = ?, updated_at = ? WHERE id = ? AND day_asset_id IS NULL",
    )
    .bind(&asset.id)
    .bind(now_iso())
    .bind(trade_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AppState>();
}
