//! Copy trading.
//!
//! Flujo: el bridge de la cuenta principal envía cada acción a `/api/v1/copy/signal`.
//! El motor la convierte en una orden para cada secundaria del grupo (símbolo,
//! tamaño, protecciones) y la deja en `copy_orders`. Los bridges de las
//! secundarias recogen sus órdenes con `/api/v1/copy/poll` (long-polling) y
//! devuelven el resultado a `/api/v1/copy/result`.

pub mod commands;
pub mod model;
pub mod sizing;
pub mod symbols;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use tauri::Emitter;
use tokio::sync::{Mutex, watch};

use crate::atas;
use crate::db;
use crate::error::AppResult;
use crate::mt5::{self, ApiError, internal};
use crate::state::AppState;
use crate::util::{new_id, now_iso};

use model::{FollowerSettings, KellyStats, SymbolFilterMode, SymbolSpec};
use symbols::{MARKET_CFD, MARKET_FUTURES};

const ONLINE_SECONDS: u64 = 10;
const MAX_WAIT_MS: u64 = 25_000;
const SENT_TIMEOUT_MS: i64 = 60_000;
const RISK_CHECK_SECONDS: u64 = 5;
const EPS: f64 = 1e-9;

// ---------------------------------------------------------------------------
// Estado en memoria
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Presence {
    pub last_seen: Option<Instant>,
    pub last_seen_iso: Option<String>,
    pub bridge_version: Option<String>,
    pub specs: HashMap<String, SymbolSpec>,
    pub balance: Option<f64>,
    pub equity: Option<f64>,
    pub last_risk_check: Option<Instant>,
}

impl Presence {
    pub fn online(&self) -> bool {
        self.last_seen
            .is_some_and(|seen| seen.elapsed() < Duration::from_secs(ONLINE_SECONDS))
    }

    pub fn markets(&self) -> Vec<String> {
        let mut markets: Vec<String> = self
            .specs
            .values()
            .map(|spec| spec.market.clone())
            .filter(|market| !market.is_empty())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        markets.sort();
        markets
    }
}

pub struct CopyRuntime {
    pub presence: Mutex<HashMap<String, Presence>>,
    wake_tx: watch::Sender<u64>,
}

impl Default for CopyRuntime {
    fn default() -> Self {
        let (wake_tx, _) = watch::channel(0);
        Self {
            presence: Mutex::new(HashMap::new()),
            wake_tx,
        }
    }
}

impl CopyRuntime {
    fn wake(&self) {
        self.wake_tx.send_modify(|value| *value = value.wrapping_add(1));
    }
}

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(crate) fn emit_updated(state: &AppState) {
    let _ = state.app.emit("copy://updated", json!({}));
}

fn emit_activity(state: &AppState) {
    let _ = state.app.emit("copy://activity", json!({}));
}

// ---------------------------------------------------------------------------
// Payloads de los bridges
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct BridgeAccount {
    #[serde(default)]
    login: i64,
    #[serde(default)]
    server: String,
    /// Solo ATAS: AccountID de la cuenta.
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    balance: Option<f64>,
    #[serde(default)]
    equity: Option<f64>,
    #[serde(default)]
    specs: Vec<SymbolSpec>,
    /// Estado actual de lo copiado en la cuenta (por etiqueta).
    #[serde(default)]
    copies: Option<Vec<CopyState>>,
    /// Solo ATAS: instrumentos que puede operar este indicador (el de su gráfico).
    /// Si se indica, solo recibe y concilia órdenes de esos instrumentos.
    #[serde(default)]
    tradable: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopyState {
    tag: String,
    #[serde(default)]
    volume: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PollPayload {
    provider: String,
    #[serde(default)]
    bridge_version: Option<String>,
    #[serde(default)]
    wait_ms: u64,
    #[serde(default)]
    accounts: Vec<BridgeAccount>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Signal {
    pub kind: String,
    /// Referencia de la posición en la principal (ticket o símbolo en cuentas netting).
    #[serde(rename = "ref")]
    pub position_ref: String,
    /// Orden pendiente relacionada (alta, modificación o la que se acaba de ejecutar).
    #[serde(default)]
    pub order_ref: Option<String>,
    pub symbol: String,
    #[serde(default)]
    pub market: String,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub volume: f64,
    /// Volumen que queda en la posición después de la acción.
    #[serde(default)]
    pub position_volume: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub entry_price: Option<f64>,
    #[serde(default)]
    pub sl: Option<f64>,
    #[serde(default)]
    pub tp: Option<f64>,
    #[serde(default)]
    pub order_type: Option<String>,
    #[serde(default)]
    pub order_price: Option<f64>,
    #[serde(default)]
    pub stop_limit_price: Option<f64>,
    #[serde(default)]
    pub bid: Option<f64>,
    #[serde(default)]
    pub ask: Option<f64>,
    #[serde(default)]
    pub time_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SignalPayload {
    provider: String,
    #[serde(flatten)]
    account: BridgeAccount,
    #[serde(default)]
    signals: Vec<Signal>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OrderResult {
    order_id: String,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    status: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    price: Option<f64>,
    #[serde(default)]
    position_ref: Option<String>,
    #[serde(default)]
    order_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResultPayload {
    #[serde(default)]
    results: Vec<OrderResult>,
}

// ---------------------------------------------------------------------------
// Utilidades de cuentas y configuración
// ---------------------------------------------------------------------------

async fn resolve_account(
    pool: &SqlitePool,
    provider: &str,
    account: &BridgeAccount,
) -> AppResult<Option<(String, String)>> {
    let provider = match provider {
        "nt8" | "ninjatrader" => "ninjatrader",
        "atas" => "atas",
        _ => "mt5",
    };
    let (login, server) = if provider == "atas" {
        let login = if account.login > 0 {
            account.login
        } else {
            match account.account.as_deref().map(str::trim) {
                Some(value) if !value.is_empty() => atas::login_for(value),
                _ => return Ok(None),
            }
        };
        (login, mt5::normalize_server(provider, &account.server))
    } else {
        (account.login, mt5::normalize_server(provider, &account.server))
    };
    let row = sqlx::query("SELECT id, provider FROM accounts WHERE login = ? AND server = ?")
        .bind(login)
        .bind(&server)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|row| (row.get("id"), row.get("provider"))))
}

pub(crate) async fn global_enabled(pool: &SqlitePool) -> AppResult<bool> {
    Ok(db::get_setting(pool, "copy_enabled")
        .await?
        .is_some_and(|value| value == "1"))
}

pub(crate) fn parse_settings(raw: &str) -> FollowerSettings {
    serde_json::from_str(raw).unwrap_or_default()
}

pub(crate) fn make_tag(group_id: &str, master_ref: &str) -> String {
    let mut hash: u32 = 0x811c9dc5;
    for byte in group_id.bytes().chain([b':']).chain(master_ref.bytes()) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("JT{hash:08x}")
}

fn opposite(side: &str) -> &'static str {
    if side.eq_ignore_ascii_case("buy") {
        "sell"
    } else {
        "buy"
    }
}

fn normalize_side(side: Option<&str>) -> Option<&'static str> {
    match side.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) if value == "buy" || value == "long" => Some("buy"),
        Some(value) if value == "sell" || value == "short" => Some("sell"),
        _ => None,
    }
}

fn opt(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite() && v.abs() > EPS)
}

/// Equity al inicio del día (hora local) → PnL del día.
pub(crate) async fn daily_pnl(
    pool: &SqlitePool,
    account_id: &str,
    current_equity: f64,
) -> AppResult<Option<f64>> {
    let midnight = Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).single())
        .map(|value| {
            value
                .with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        });
    let Some(midnight) = midnight else {
        return Ok(None);
    };
    let start: Option<f64> = sqlx::query_scalar(
        "SELECT equity FROM account_snapshots WHERE account_id = ? AND taken_at >= ?
         ORDER BY taken_at ASC LIMIT 1",
    )
    .bind(account_id)
    .bind(&midnight)
    .fetch_optional(pool)
    .await?;
    let start = match start {
        Some(value) => Some(value),
        None => sqlx::query_scalar(
            "SELECT equity FROM account_snapshots WHERE account_id = ? AND taken_at < ?
             ORDER BY taken_at DESC LIMIT 1",
        )
        .bind(account_id)
        .bind(&midnight)
        .fetch_optional(pool)
        .await?,
    };
    Ok(start
        .filter(|value| *value > 0.0)
        .map(|value| current_equity - value))
}

async fn kelly_stats(pool: &SqlitePool, account_id: &str, lookback: u32) -> AppResult<KellyStats> {
    let rows: Vec<f64> = sqlx::query_scalar(
        "SELECT profit_net FROM trades WHERE account_id = ? AND state = 'closed'
         ORDER BY close_time_utc DESC LIMIT ?",
    )
    .bind(account_id)
    .bind(i64::from(lookback.clamp(5, 1000)))
    .fetch_all(pool)
    .await?;
    let wins: Vec<f64> = rows.iter().copied().filter(|v| *v > 0.0).collect();
    let losses: Vec<f64> = rows.iter().copied().filter(|v| *v < 0.0).map(f64::abs).collect();
    let trades = rows.len();
    let win_rate = if trades > 0 {
        wins.len() as f64 / trades as f64
    } else {
        0.0
    };
    let avg_win = if wins.is_empty() {
        0.0
    } else {
        wins.iter().sum::<f64>() / wins.len() as f64
    };
    let avg_loss = if losses.is_empty() {
        0.0
    } else {
        losses.iter().sum::<f64>() / losses.len() as f64
    };
    let payoff = if avg_loss > 0.0 {
        avg_win / avg_loss
    } else if avg_win > 0.0 {
        10.0
    } else {
        0.0
    };
    Ok(KellyStats {
        trades,
        win_rate,
        payoff,
    })
}

fn in_schedule(settings: &FollowerSettings) -> bool {
    if !settings.schedule_enabled {
        return true;
    }
    let now = Local::now();
    let weekday = now.weekday().number_from_monday();
    if !settings.schedule_days.is_empty() && !settings.schedule_days.contains(&weekday) {
        return false;
    }
    let parse = |value: &str| NaiveTime::parse_from_str(value.trim(), "%H:%M").ok();
    let (Some(start), Some(end)) = (parse(&settings.schedule_start), parse(&settings.schedule_end))
    else {
        return true;
    };
    let current = NaiveTime::from_hms_opt(now.hour(), now.minute(), 0).unwrap_or(start);
    if start <= end {
        current >= start && current <= end
    } else {
        current >= start || current <= end
    }
}

fn symbol_allowed(settings: &FollowerSettings, symbol: &str) -> bool {
    let matches = |pattern: &String| {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return false;
        }
        if symbol.eq_ignore_ascii_case(pattern) {
            return true;
        }
        // Una raíz de futuros (NQ) cubre todos sus vencimientos.
        symbols::parse_futures(symbol).is_some_and(|key| key.root.eq_ignore_ascii_case(pattern))
    };
    match settings.symbol_filter_mode {
        SymbolFilterMode::All => true,
        SymbolFilterMode::Include => settings.symbols.iter().any(matches),
        SymbolFilterMode::Exclude => !settings.symbols.iter().any(matches),
    }
}

// ---------------------------------------------------------------------------
// Motor
// ---------------------------------------------------------------------------

struct FollowerRow {
    id: String,
    group_id: String,
    account_id: String,
    enabled: bool,
    paused_reason: Option<String>,
    settings: FollowerSettings,
    provider: String,
    balance: f64,
    equity: f64,
}

async fn load_followers(pool: &SqlitePool, group_id: &str) -> AppResult<Vec<FollowerRow>> {
    let rows = sqlx::query(
        "SELECT f.id, f.group_id, f.account_id, f.enabled, f.paused_reason, f.settings_json,
                a.provider, a.balance, a.equity
         FROM copy_followers f JOIN accounts a ON a.id = f.account_id
         WHERE f.group_id = ? AND a.status != 'archived'",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| FollowerRow {
            id: row.get("id"),
            group_id: row.get("group_id"),
            account_id: row.get("account_id"),
            enabled: row.get::<i64, _>("enabled") != 0,
            paused_reason: row.get("paused_reason"),
            settings: parse_settings(&row.get::<String, _>("settings_json")),
            provider: row.get("provider"),
            balance: row.get("balance"),
            equity: row.get("equity"),
        })
        .collect())
}

struct CopiedPosition {
    tag: String,
    symbol: String,
    side: String,
    volume: f64,
    follower_ref: Option<String>,
}

async fn copied_position(
    pool: &SqlitePool,
    follower_id: &str,
    master_ref: &str,
) -> AppResult<Option<CopiedPosition>> {
    let row = sqlx::query(
        "SELECT tag, symbol, side, volume, follower_ref FROM copy_positions
         WHERE follower_id = ? AND master_ref = ?",
    )
    .bind(follower_id)
    .bind(master_ref)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| CopiedPosition {
        tag: row.get("tag"),
        symbol: row.get("symbol"),
        side: row.get("side"),
        volume: row.get("volume"),
        follower_ref: row.get("follower_ref"),
    }))
}

struct CopiedPending {
    tag: String,
    symbol: String,
    follower_order_ref: Option<String>,
}

async fn copied_pending(
    pool: &SqlitePool,
    follower_id: &str,
    master_order_ref: &str,
) -> AppResult<Option<CopiedPending>> {
    let row = sqlx::query(
        "SELECT tag, symbol, follower_order_ref FROM copy_pendings
         WHERE follower_id = ? AND master_order_ref = ?",
    )
    .bind(follower_id)
    .bind(master_order_ref)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| CopiedPending {
        tag: row.get("tag"),
        symbol: row.get("symbol"),
        follower_order_ref: row.get("follower_order_ref"),
    }))
}

/// Orden planificada para una secundaria (o el motivo por el que se salta).
pub(crate) struct Planned {
    pub action: String,
    pub master_ref: String,
    pub tag: String,
    pub symbol: String,
    pub side: Option<String>,
    pub volume: f64,
    pub payload: Value,
    pub skipped: Option<String>,
}

impl Planned {
    fn skip(signal: &Signal, reason: impl Into<String>) -> Self {
        Self {
            action: signal.kind.clone(),
            master_ref: signal.position_ref.clone(),
            tag: String::new(),
            symbol: signal.symbol.clone(),
            side: signal.side.clone(),
            volume: 0.0,
            payload: json!({}),
            skipped: Some(reason.into()),
        }
    }
}

fn is_entry(kind: &str) -> bool {
    matches!(kind, "open" | "increase" | "pending_place")
}

/// Precio del lado contrario (para órdenes pendientes en CFD/Forex).
fn reference_price(signal: &Signal, side: &str) -> f64 {
    let quote = if side == "buy" { signal.ask } else { signal.bid };
    opt(quote).unwrap_or(signal.price)
}

#[allow(clippy::too_many_arguments)]
fn sltp_fields(
    settings: &FollowerSettings,
    is_futures: bool,
    base_price: f64,
    sl: Option<f64>,
    tp: Option<f64>,
    payload: &mut serde_json::Map<String, Value>,
) {
    if !settings.copy_sltp {
        return;
    }
    let (sl, tp) = if settings.invert { (tp, sl) } else { (sl, tp) };
    if is_futures {
        payload.insert("sl".into(), json!(opt(sl)));
        payload.insert("tp".into(), json!(opt(tp)));
    } else {
        payload.insert("sl_offset".into(), json!(opt(sl).map(|v| v - base_price)));
        payload.insert("tp_offset".into(), json!(opt(tp).map(|v| v - base_price)));
    }
}

/// Datos en vivo de la secundaria (los envía su bridge).
#[derive(Default)]
pub(crate) struct FollowerContext {
    pub specs: HashMap<String, SymbolSpec>,
    pub balance: f64,
    pub equity: f64,
}

async fn follower_context(state: &AppState, follower: &FollowerRow) -> FollowerContext {
    let presence = state.copy.presence.lock().await;
    match presence.get(&follower.account_id) {
        Some(value) => FollowerContext {
            specs: value.specs.clone(),
            balance: value.balance.unwrap_or(follower.balance),
            equity: value.equity.unwrap_or(follower.equity),
        },
        None => FollowerContext {
            specs: HashMap::new(),
            balance: follower.balance,
            equity: follower.equity,
        },
    }
}

/// Resultado de planificar una señal para una secundaria.
pub(crate) struct PlanOutcome {
    pub planned: Option<Planned>,
    /// Si se debe pausar la secundaria (límite de pérdida diaria).
    pub pause: Option<String>,
}

impl From<Option<Planned>> for PlanOutcome {
    fn from(planned: Option<Planned>) -> Self {
        Self {
            planned,
            pause: None,
        }
    }
}

async fn plan_for_follower(
    pool: &SqlitePool,
    context: &FollowerContext,
    master_provider: &str,
    master_account_id: &str,
    follower: &FollowerRow,
    signal: &Signal,
) -> AppResult<PlanOutcome> {
    let settings = &follower.settings;
    if follower.enabled && follower.paused_reason.is_none() && is_entry(&signal.kind) {
        if let Some(limit) = settings.daily_loss_limit.filter(|v| *v > 0.0) {
            if let Some(pnl) = daily_pnl(pool, &follower.account_id, context.equity).await? {
                if -pnl >= limit {
                    return Ok(PlanOutcome {
                        planned: Some(Planned::skip(signal, "Límite de pérdida diaria alcanzado")),
                        pause: Some(format!("Límite de pérdida diaria alcanzado ({pnl:.2})")),
                    });
                }
            }
        }
    }
    let planned =
        plan_inner(pool, context, master_provider, master_account_id, follower, signal).await?;
    Ok(planned.into())
}

async fn plan_inner(
    pool: &SqlitePool,
    context: &FollowerContext,
    master_provider: &str,
    master_account_id: &str,
    follower: &FollowerRow,
    signal: &Signal,
) -> AppResult<Option<Planned>> {
    let settings = &follower.settings;
    let market = if signal.market.is_empty() {
        if master_provider == "mt5" {
            MARKET_CFD
        } else {
            MARKET_FUTURES
        }
    } else {
        signal.market.as_str()
    };
    let is_futures = market == MARKET_FUTURES;
    let entry = is_entry(&signal.kind);

    if !follower.enabled {
        return Ok(None);
    }

    if entry {
        if let Some(reason) = &follower.paused_reason {
            return Ok(Some(Planned::skip(signal, format!("Copia pausada: {reason}"))));
        }
        if !symbols::compatible(market, master_provider, &follower.provider) {
            return Ok(Some(Planned::skip(
                signal,
                "Mercado incompatible con la plataforma de la secundaria",
            )));
        }
        if !symbol_allowed(settings, &signal.symbol) {
            return Ok(Some(Planned::skip(signal, "Símbolo excluido por el filtro")));
        }
        if !in_schedule(settings) {
            return Ok(Some(Planned::skip(signal, "Fuera del horario de copia")));
        }
        if signal.kind == "pending_place" && !settings.copy_pending {
            return Ok(Some(Planned::skip(
                signal,
                "La copia de órdenes pendientes está desactivada",
            )));
        }
    }

    let presence_specs = &context.specs;
    let balance = context.balance;
    let equity = context.equity;

    let map_rows = sqlx::query(
        "SELECT master_symbol, follower_symbol, volume_ratio FROM copy_symbol_map WHERE follower_id = ?",
    )
    .bind(&follower.id)
    .fetch_all(pool)
    .await?;
    let map_data: Vec<(String, String, Option<f64>)> = map_rows
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("master_symbol"),
                row.get::<String, _>("follower_symbol"),
                row.get::<Option<f64>, _>("volume_ratio"),
            )
        })
        .collect();
    let map: Vec<symbols::MapEntry<'_>> = map_data
        .iter()
        .map(|(master, follower, ratio)| symbols::MapEntry {
            master_symbol: master,
            follower_symbol: follower,
            volume_ratio: *ratio,
        })
        .collect();
    let available: Vec<String> = presence_specs.keys().cloned().collect();

    let base = |action: &str, tag: &str, symbols: Vec<String>, side: Option<&str>, volume: f64| {
        let mut payload = serde_json::Map::new();
        payload.insert("action".into(), json!(action));
        payload.insert("tag".into(), json!(tag));
        payload.insert("symbols".into(), json!(symbols));
        payload.insert("market".into(), json!(market));
        payload.insert("side".into(), json!(side));
        payload.insert("volume".into(), json!(volume));
        payload.insert("master_ref".into(), json!(signal.position_ref));
        payload.insert(
            "price_mode".into(),
            json!(if is_futures { "absolute" } else { "offset" }),
        );
        payload.insert("max_deviation".into(), json!(settings.max_deviation));
        payload
    };

    let planned = |action: &str,
                   master_ref: &str,
                   tag: &str,
                   symbol: &str,
                   side: Option<&str>,
                   volume: f64,
                   payload: serde_json::Map<String, Value>| Planned {
        action: action.to_string(),
        master_ref: master_ref.to_string(),
        tag: tag.to_string(),
        symbol: symbol.to_string(),
        side: side.map(ToString::to_string),
        volume,
        payload: Value::Object(payload),
        skipped: None,
    };

    match signal.kind.as_str() {
        "open" | "increase" | "pending_place" => {
            let Some(side) = normalize_side(signal.side.as_deref()) else {
                return Ok(Some(Planned::skip(signal, "La señal no indica la dirección")));
            };
            let Some(resolution) = symbols::resolve(
                &signal.symbol,
                market,
                &follower.provider,
                &map,
                &available,
                settings.family_scaling,
            ) else {
                return Ok(Some(Planned::skip(
                    signal,
                    "No se pudo emparejar el símbolo (añádelo a la tabla de equivalencias)",
                )));
            };
            let spec = resolution
                .candidates
                .iter()
                .find_map(|candidate| presence_specs.get(candidate));
            let pending = signal.kind == "pending_place";
            let entry_price = if pending {
                opt(signal.order_price).unwrap_or(signal.price)
            } else {
                signal.price
            };
            let kelly = if settings.sizing.mode == model::SizingMode::Kelly {
                Some(kelly_stats(pool, master_account_id, settings.sizing.kelly_lookback).await?)
            } else {
                None
            };
            let raw = match sizing::compute(&sizing::SizingInput {
                params: &settings.sizing,
                is_futures,
                master_volume: signal.volume,
                entry_price,
                stop_loss: opt(signal.sl),
                balance,
                equity,
                volume_ratio: resolution.volume_ratio,
                kelly,
                spec,
            }) {
                Ok(value) => value,
                Err(reason) => return Ok(Some(Planned::skip(signal, reason))),
            };

            let existing = if pending {
                None
            } else {
                copied_position(pool, &follower.id, &signal.position_ref).await?
            };
            let mut cap = settings.max_volume_per_trade.filter(|v| *v > 0.0);
            if let Some(max_position) = settings.max_position_volume.filter(|v| *v > 0.0) {
                let allowed = max_position - existing.as_ref().map(|p| p.volume).unwrap_or(0.0);
                if allowed <= EPS {
                    return Ok(Some(Planned::skip(
                        signal,
                        "Se alcanzó el tamaño máximo de posición",
                    )));
                }
                cap = Some(cap.map_or(allowed, |value| value.min(allowed)));
            }
            let volume = match sizing::normalize(raw.volume, is_futures, spec, cap) {
                Ok(value) => value,
                Err(reason) => return Ok(Some(Planned::skip(signal, reason))),
            };
            let follower_side = if settings.invert { opposite(side) } else { side };
            let first = resolution.candidates.first().cloned().unwrap_or_default();
            let expires_at = signal.time_ms.unwrap_or_else(now_ms)
                + i64::from(settings.max_delay_seconds.max(1)) * 1000;

            if pending {
                let order_ref = signal
                    .order_ref
                    .clone()
                    .unwrap_or_else(|| signal.position_ref.clone());
                let tag = make_tag(&follower.group_id, &order_ref);
                let order_type = signal.order_type.as_deref().unwrap_or("limit");
                let follower_type = if settings.invert {
                    match order_type {
                        "limit" => "stop",
                        "stop" => "limit",
                        _ => {
                            return Ok(Some(Planned::skip(
                                signal,
                                "Una orden stop-limit no se puede invertir",
                            )));
                        }
                    }
                } else {
                    order_type
                };
                let mut payload = base(
                    "pending_place",
                    &tag,
                    resolution.candidates.clone(),
                    Some(follower_side),
                    volume,
                );
                payload.insert("master_order_ref".into(), json!(order_ref));
                payload.insert("order_type".into(), json!(follower_type));
                payload.insert("entry".into(), json!(true));
                payload.insert("expires_at_ms".into(), json!(expires_at));
                payload.insert("reference_price".into(), json!(signal.price));
                if is_futures {
                    payload.insert("price".into(), json!(entry_price));
                    payload.insert("stop_limit_price".into(), json!(opt(signal.stop_limit_price)));
                } else {
                    let reference = reference_price(signal, side);
                    payload.insert("price_offset".into(), json!(entry_price - reference));
                    payload.insert(
                        "stop_limit_offset".into(),
                        json!(opt(signal.stop_limit_price).map(|v| v - reference)),
                    );
                }
                sltp_fields(settings, is_futures, entry_price, signal.sl, signal.tp, &mut payload);
                if let Some(note) = raw.note {
                    payload.insert("note".into(), json!(note));
                }
                return Ok(Some(planned(
                    "pending_place",
                    &order_ref,
                    &tag,
                    &first,
                    Some(follower_side),
                    volume,
                    payload,
                )));
            }

            let tag = existing
                .as_ref()
                .map(|p| p.tag.clone())
                .unwrap_or_else(|| make_tag(&follower.group_id, &signal.position_ref));
            let filled_pending = match signal.order_ref.as_deref() {
                Some(order_ref) => copied_pending(pool, &follower.id, order_ref)
                    .await?
                    .map(|p| (order_ref.to_string(), p)),
                None => None,
            };
            let action = if filled_pending.is_some() {
                "pending_fill"
            } else {
                "market"
            };
            let symbols = match (&existing, &filled_pending) {
                (Some(position), _) => vec![position.symbol.clone()],
                (None, Some((_, pending))) => vec![pending.symbol.clone()],
                _ => resolution.candidates.clone(),
            };
            let symbol = symbols.first().cloned().unwrap_or(first);
            let mut payload = base(action, &tag, symbols, Some(follower_side), volume);
            payload.insert("entry".into(), json!(true));
            payload.insert("reduce_only".into(), json!(false));
            payload.insert("expires_at_ms".into(), json!(expires_at));
            payload.insert("reference_price".into(), json!(signal.price));
            if let Some(position) = &existing {
                payload.insert("follower_ref".into(), json!(position.follower_ref));
            }
            if let Some((order_ref, pending)) = &filled_pending {
                payload.insert("master_order_ref".into(), json!(order_ref));
                payload.insert("follower_order_ref".into(), json!(pending.follower_order_ref));
                payload.insert("pending_tag".into(), json!(pending.tag));
            }
            sltp_fields(settings, is_futures, signal.price, signal.sl, signal.tp, &mut payload);
            if let Some(note) = raw.note {
                payload.insert("note".into(), json!(note));
            }
            Ok(Some(planned(
                action,
                &signal.position_ref,
                &tag,
                &symbol,
                Some(follower_side),
                volume,
                payload,
            )))
        }
        "reduce" | "close" => {
            let Some(position) = copied_position(pool, &follower.id, &signal.position_ref).await?
            else {
                return Ok(None);
            };
            let spec = presence_specs.get(&position.symbol);
            let full = signal.kind == "close" || signal.position_volume <= EPS;
            let volume = if full {
                position.volume
            } else {
                let total = signal.volume + signal.position_volume;
                let fraction = if total > EPS { signal.volume / total } else { 1.0 };
                match sizing::reduce_volume(position.volume, fraction, is_futures, spec) {
                    Some(value) => value,
                    None => {
                        return Ok(Some(Planned::skip(
                            signal,
                            "El cierre parcial queda por debajo del volumen mínimo",
                        )));
                    }
                }
            };
            let action = if full || (position.volume - volume).abs() < EPS {
                "close"
            } else {
                "market"
            };
            let close_side = opposite(&position.side);
            let mut payload = base(
                action,
                &position.tag,
                vec![position.symbol.clone()],
                Some(close_side),
                volume,
            );
            payload.insert("reduce_only".into(), json!(true));
            payload.insert("entry".into(), json!(false));
            payload.insert("position_side".into(), json!(position.side));
            payload.insert("follower_ref".into(), json!(position.follower_ref));
            Ok(Some(planned(
                action,
                &signal.position_ref,
                &position.tag,
                &position.symbol,
                Some(close_side),
                volume,
                payload,
            )))
        }
        "sltp" => {
            if !settings.copy_sltp {
                return Ok(None);
            }
            let Some(position) = copied_position(pool, &follower.id, &signal.position_ref).await?
            else {
                return Ok(None);
            };
            let entry_price = opt(signal.entry_price).unwrap_or(signal.price);
            let mut payload = base(
                "sltp",
                &position.tag,
                vec![position.symbol.clone()],
                Some(&position.side),
                position.volume,
            );
            payload.insert("follower_ref".into(), json!(position.follower_ref));
            payload.insert("position_side".into(), json!(position.side));
            payload.insert("entry".into(), json!(false));
            sltp_fields(settings, is_futures, entry_price, signal.sl, signal.tp, &mut payload);
            Ok(Some(planned(
                "sltp",
                &signal.position_ref,
                &position.tag,
                &position.symbol,
                Some(&position.side),
                position.volume,
                payload,
            )))
        }
        "pending_modify" | "pending_cancel" => {
            let order_ref = signal
                .order_ref
                .clone()
                .unwrap_or_else(|| signal.position_ref.clone());
            let Some(pending) = copied_pending(pool, &follower.id, &order_ref).await? else {
                return Ok(None);
            };
            let mut payload = base(
                &signal.kind,
                &pending.tag,
                vec![pending.symbol.clone()],
                None,
                0.0,
            );
            payload.insert("master_order_ref".into(), json!(order_ref));
            payload.insert("follower_order_ref".into(), json!(pending.follower_order_ref));
            payload.insert("entry".into(), json!(false));
            if signal.kind == "pending_modify" {
                let side = normalize_side(signal.side.as_deref()).unwrap_or("buy");
                let price = opt(signal.order_price).unwrap_or(signal.price);
                if is_futures {
                    payload.insert("price".into(), json!(price));
                    payload.insert("stop_limit_price".into(), json!(opt(signal.stop_limit_price)));
                } else {
                    let reference = reference_price(signal, side);
                    payload.insert("price_offset".into(), json!(price - reference));
                    payload.insert(
                        "stop_limit_offset".into(),
                        json!(opt(signal.stop_limit_price).map(|v| v - reference)),
                    );
                }
                let follower_side = if settings.invert { opposite(side) } else { side };
                payload.insert("side".into(), json!(follower_side));
                sltp_fields(settings, is_futures, price, signal.sl, signal.tp, &mut payload);
            }
            Ok(Some(planned(
                &signal.kind,
                &order_ref,
                &pending.tag,
                &pending.symbol,
                None,
                0.0,
                payload,
            )))
        }
        other => Ok(Some(Planned::skip(signal, format!("Tipo de señal desconocido: {other}")))),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_order(
    pool: &SqlitePool,
    signal_id: Option<&str>,
    group_id: &str,
    follower_id: &str,
    account_id: &str,
    planned: Planned,
) -> AppResult<()> {
    let id = new_id();
    let mut payload = planned.payload;
    if let Value::Object(map) = &mut payload {
        map.insert("id".into(), json!(id));
    }
    let status = if planned.skipped.is_some() {
        "skipped"
    } else {
        "pending"
    };
    sqlx::query(
        "INSERT INTO copy_orders
         (id, signal_id, group_id, follower_id, account_id, master_ref, tag, action, symbol, side,
          volume, payload_json, status, error, created_at, created_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(signal_id)
    .bind(group_id)
    .bind(follower_id)
    .bind(account_id)
    .bind(&planned.master_ref)
    .bind(&planned.tag)
    .bind(&planned.action)
    .bind(&planned.symbol)
    .bind(&planned.side)
    .bind(planned.volume)
    .bind(payload.to_string())
    .bind(status)
    .bind(&planned.skipped)
    .bind(now_iso())
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

/// Cierra lo copiado y/o cancela las pendientes de una secundaria.
pub(crate) async fn flatten_follower(
    state: &AppState,
    follower_id: &str,
    close_positions: bool,
    cancel_pending: bool,
) -> AppResult<usize> {
    let pool = &state.db;
    let Some(row) = sqlx::query("SELECT group_id, account_id FROM copy_followers WHERE id = ?")
        .bind(follower_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(0);
    };
    let group_id: String = row.get("group_id");
    let account_id: String = row.get("account_id");
    let mut count = 0;

    if close_positions {
        let positions = sqlx::query(
            "SELECT master_ref, tag, symbol, side, volume, follower_ref FROM copy_positions
             WHERE follower_id = ?",
        )
        .bind(follower_id)
        .fetch_all(pool)
        .await?;
        for position in positions {
            let side: String = position.get("side");
            let symbol: String = position.get("symbol");
            let tag: String = position.get("tag");
            let volume: f64 = position.get("volume");
            let master_ref: String = position.get("master_ref");
            let follower_ref: Option<String> = position.get("follower_ref");
            let close_side = opposite(&side);
            let payload = json!({
                "action": "close",
                "tag": tag,
                "symbols": [symbol],
                "side": close_side,
                "position_side": side,
                "volume": volume,
                "reduce_only": true,
                "entry": false,
                "master_ref": master_ref,
                "follower_ref": follower_ref,
            });
            insert_order(
                pool,
                None,
                &group_id,
                follower_id,
                &account_id,
                Planned {
                    action: "close".into(),
                    master_ref,
                    tag,
                    symbol,
                    side: Some(close_side.to_string()),
                    volume,
                    payload,
                    skipped: None,
                },
            )
            .await?;
            count += 1;
        }
    }

    if cancel_pending {
        let pendings = sqlx::query(
            "SELECT master_order_ref, tag, symbol, follower_order_ref FROM copy_pendings
             WHERE follower_id = ?",
        )
        .bind(follower_id)
        .fetch_all(pool)
        .await?;
        for pending in pendings {
            let tag: String = pending.get("tag");
            let symbol: String = pending.get("symbol");
            let master_ref: String = pending.get("master_order_ref");
            let follower_ref: Option<String> = pending.get("follower_order_ref");
            let payload = json!({
                "action": "pending_cancel",
                "tag": tag,
                "symbols": [symbol],
                "entry": false,
                "master_order_ref": master_ref,
                "follower_order_ref": follower_ref,
            });
            insert_order(
                pool,
                None,
                &group_id,
                follower_id,
                &account_id,
                Planned {
                    action: "pending_cancel".into(),
                    master_ref,
                    tag,
                    symbol,
                    side: None,
                    volume: 0.0,
                    payload,
                    skipped: None,
                },
            )
            .await?;
            count += 1;
        }
    }

    if count > 0 {
        state.copy.wake();
        emit_activity(state);
    }
    Ok(count)
}

pub(crate) async fn pause_follower(
    state: &AppState,
    follower_id: &str,
    reason: &str,
    close_positions: bool,
) -> AppResult<()> {
    let already: Option<Option<String>> =
        sqlx::query_scalar("SELECT paused_reason FROM copy_followers WHERE id = ?")
            .bind(follower_id)
            .fetch_optional(&state.db)
            .await?;
    if matches!(already, Some(Some(_))) {
        return Ok(());
    }
    let now = now_iso();
    sqlx::query(
        "UPDATE copy_followers SET paused_reason = ?, paused_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(reason)
    .bind(&now)
    .bind(&now)
    .bind(follower_id)
    .execute(&state.db)
    .await?;
    if close_positions {
        flatten_follower(state, follower_id, true, true).await?;
    }
    log::warn!("Copy trading: secundaria {follower_id} pausada: {reason}");
    emit_updated(state);
    Ok(())
}

pub(crate) fn wake(state: &AppState) {
    state.copy.wake();
}

// ---------------------------------------------------------------------------
// Handlers HTTP
// ---------------------------------------------------------------------------

async fn account_role(pool: &SqlitePool, account_id: &str, global: bool) -> AppResult<(String, bool)> {
    if let Some(enabled) = sqlx::query_scalar::<_, i64>(
        "SELECT enabled FROM copy_groups WHERE master_account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?
    {
        return Ok(("master".into(), global && enabled != 0));
    }
    let rows = sqlx::query(
        "SELECT g.enabled AS group_enabled, f.enabled AS follower_enabled, f.paused_reason
         FROM copy_followers f JOIN copy_groups g ON g.id = f.group_id
         WHERE f.account_id = ?",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(("none".into(), false));
    }
    let enabled = rows.iter().any(|row| {
        row.get::<i64, _>("group_enabled") != 0
            && row.get::<i64, _>("follower_enabled") != 0
            && row.get::<Option<String>, _>("paused_reason").is_none()
    });
    Ok(("follower".into(), global && enabled))
}

fn order_matches(payload: &Value, tradable: Option<&[String]>) -> bool {
    let Some(tradable) = tradable else {
        return true;
    };
    payload
        .get("symbols")
        .and_then(Value::as_array)
        .is_some_and(|symbols| {
            symbols.iter().filter_map(Value::as_str).any(|symbol| {
                tradable
                    .iter()
                    .any(|value| value.trim().eq_ignore_ascii_case(symbol.trim()))
            })
        })
}

async fn take_orders(
    pool: &SqlitePool,
    account_id: &str,
    tradable: Option<&[String]>,
) -> AppResult<Vec<Value>> {
    let now = now_ms();
    sqlx::query(
        "UPDATE copy_orders SET status = 'rejected', error = 'El bridge no respondió a tiempo',
         done_at = ? WHERE account_id = ? AND status = 'sent' AND created_ms < ?",
    )
    .bind(now_iso())
    .bind(account_id)
    .bind(now - SENT_TIMEOUT_MS)
    .execute(pool)
    .await?;

    let rows = sqlx::query(
        "SELECT id, payload_json FROM copy_orders WHERE account_id = ? AND status = 'pending'
         ORDER BY created_ms ASC, rowid ASC LIMIT 50",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    let mut orders = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let payload: Value =
            serde_json::from_str(&row.get::<String, _>("payload_json")).unwrap_or(Value::Null);
        let entry = payload.get("entry").and_then(Value::as_bool).unwrap_or(false);
        let expires = payload.get("expires_at_ms").and_then(Value::as_i64);
        let matches = order_matches(&payload, tradable);
        if entry && expires.is_some_and(|value| now > value) {
            sqlx::query(
                "UPDATE copy_orders SET status = 'skipped', done_at = ?,
                 error = 'Entrada tardía: la secundaria no estaba conectada a tiempo' WHERE id = ?",
            )
            .bind(now_iso())
            .bind(&id)
            .execute(pool)
            .await?;
            continue;
        }
        if !matches {
            continue;
        }
        sqlx::query("UPDATE copy_orders SET status = 'sent', sent_at = ? WHERE id = ?")
            .bind(now_iso())
            .bind(&id)
            .execute(pool)
            .await?;
        orders.push(payload);
    }
    Ok(orders)
}

/// Ajusta el estado guardado con lo que informa el bridge (cierres por SL/TP, cierres manuales...).
async fn reconcile_copies(
    pool: &SqlitePool,
    account_id: &str,
    copies: &[CopyState],
    tradable: Option<&[String]>,
) -> AppResult<bool> {
    let in_flight: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM copy_orders WHERE account_id = ? AND status IN ('pending', 'sent')",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    if in_flight > 0 {
        return Ok(false);
    }
    let reported: HashMap<&str, f64> = copies
        .iter()
        .map(|copy| (copy.tag.as_str(), copy.volume.abs()))
        .collect();
    let rows = sqlx::query(
        "SELECT p.follower_id, p.master_ref, p.tag, p.volume, p.symbol FROM copy_positions p
         JOIN copy_followers f ON f.id = p.follower_id WHERE f.account_id = ?",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    let mut changed = false;
    for row in rows {
        let follower_id: String = row.get("follower_id");
        let master_ref: String = row.get("master_ref");
        let tag: String = row.get("tag");
        let volume: f64 = row.get("volume");
        let symbol: String = row.get("symbol");
        if let Some(tradable) = tradable {
            if !tradable.iter().any(|value| value.trim().eq_ignore_ascii_case(symbol.trim())) {
                continue;
            }
        }
        match reported.get(tag.as_str()) {
            Some(current) if *current > EPS => {
                if (current - volume).abs() > EPS {
                    sqlx::query(
                        "UPDATE copy_positions SET volume = ?, updated_at = ?
                         WHERE follower_id = ? AND master_ref = ?",
                    )
                    .bind(current)
                    .bind(now_iso())
                    .bind(&follower_id)
                    .bind(&master_ref)
                    .execute(pool)
                    .await?;
                    changed = true;
                }
            }
            _ => {
                sqlx::query("DELETE FROM copy_positions WHERE follower_id = ? AND master_ref = ?")
                    .bind(&follower_id)
                    .bind(&master_ref)
                    .execute(pool)
                    .await?;
                changed = true;
            }
        }
    }
    Ok(changed)
}

async fn check_daily_loss(state: &AppState, account_id: &str, equity: f64) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT id, settings_json FROM copy_followers WHERE account_id = ? AND paused_reason IS NULL
         AND enabled = 1",
    )
    .bind(account_id)
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let pnl = daily_pnl(&state.db, account_id, equity).await?;
    let Some(pnl) = pnl else {
        return Ok(());
    };
    for row in rows {
        let settings = parse_settings(&row.get::<String, _>("settings_json"));
        if let Some(limit) = settings.daily_loss_limit.filter(|v| *v > 0.0) {
            if -pnl >= limit {
                let id: String = row.get("id");
                pause_follower(
                    state,
                    &id,
                    &format!("Límite de pérdida diaria alcanzado ({pnl:.2})"),
                    settings.close_on_daily_loss,
                )
                .await?;
            }
        }
    }
    Ok(())
}

struct PollAccount {
    tradable: Option<Vec<String>>,
    login: i64,
    server: String,
    account_id: Option<String>,
    role: String,
    enabled: bool,
}

pub(crate) async fn poll(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PollPayload>,
) -> Result<Json<Value>, ApiError> {
    mt5::authorize(&state, &headers).await?;
    let pool = &state.db;
    let global = global_enabled(pool).await.map_err(internal)?;
    let mut wake_rx = state.copy.wake_tx.subscribe();
    let mut accounts = Vec::with_capacity(payload.accounts.len());
    let mut presence_changed = false;

    for account in &payload.accounts {
        let resolved = resolve_account(pool, &payload.provider, account)
            .await
            .map_err(internal)?;
        let Some((account_id, _provider)) = resolved else {
            accounts.push(PollAccount {
                tradable: None,
                login: account.login,
                server: account.server.clone(),
                account_id: None,
                role: "none".into(),
                enabled: false,
            });
            continue;
        };

        let risk_due = {
            let mut presence = state.copy.presence.lock().await;
            let entry = presence.entry(account_id.clone()).or_default();
            if !entry.online() {
                presence_changed = true;
            }
            entry.last_seen = Some(Instant::now());
            entry.last_seen_iso = Some(now_iso());
            entry.bridge_version = payload.bridge_version.clone();
            if account.balance.is_some() {
                entry.balance = account.balance;
            }
            if account.equity.is_some() {
                entry.equity = account.equity;
            }
            for spec in &account.specs {
                if !spec.symbol.is_empty() {
                    entry.specs.insert(spec.symbol.clone(), spec.clone());
                }
            }
            let due = entry
                .last_risk_check
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(RISK_CHECK_SECONDS));
            if due {
                entry.last_risk_check = Some(Instant::now());
            }
            due
        };

        if let Some(copies) = &account.copies {
            if reconcile_copies(pool, &account_id, copies, account.tradable.as_deref())
                .await
                .map_err(internal)?
            {
                emit_activity(&state);
            }
        }

        let (role, enabled) = account_role(pool, &account_id, global)
            .await
            .map_err(internal)?;
        if role == "follower" && risk_due {
            if let Some(equity) = account.equity {
                check_daily_loss(&state, &account_id, equity)
                    .await
                    .map_err(internal)?;
            }
        }
        accounts.push(PollAccount {
            tradable: account.tradable.clone(),
            login: account.login,
            server: account.server.clone(),
            account_id: Some(account_id),
            role,
            enabled,
        });
    }
    if presence_changed {
        emit_updated(&state);
    }

    let wait_ms = payload.wait_ms.min(MAX_WAIT_MS);
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    let followers: Vec<usize> = accounts
        .iter()
        .enumerate()
        .filter(|(_, account)| account.role == "follower" && account.account_id.is_some())
        .map(|(index, _)| index)
        .collect();

    let mut orders: HashMap<usize, Vec<Value>> = HashMap::new();
    loop {
        wake_rx.mark_unchanged();
        let mut total = 0;
        for index in &followers {
            let account_id = accounts[*index].account_id.as_deref().unwrap_or_default();
            let taken = take_orders(pool, account_id, accounts[*index].tradable.as_deref())
                .await
                .map_err(internal)?;
            total += taken.len();
            orders.entry(*index).or_default().extend(taken);
        }
        if total > 0 || followers.is_empty() {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if tokio::time::timeout(deadline - now, wake_rx.changed())
            .await
            .is_err()
        {
            break;
        }
    }
    if orders.values().any(|list| !list.is_empty()) {
        emit_activity(&state);
    }

    let accounts_json: Vec<Value> = accounts
        .into_iter()
        .enumerate()
        .map(|(index, account)| {
            json!({
                "login": account.login,
                "server": account.server,
                "accountId": account.account_id,
                "role": account.role,
                "enabled": account.enabled,
                "orders": orders.remove(&index).unwrap_or_default(),
            })
        })
        .collect();

    Ok(Json(json!({
        "ok": true,
        "serverTimeMs": now_ms(),
        "accounts": accounts_json,
    })))
}

pub(crate) async fn signal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SignalPayload>,
) -> Result<Json<Value>, ApiError> {
    mt5::authorize(&state, &headers).await?;
    let pool = &state.db;
    let Some((account_id, provider)) = resolve_account(pool, &payload.provider, &payload.account)
        .await
        .map_err(internal)?
    else {
        return Ok(Json(json!({ "ok": false, "error": "Cuenta desconocida" })));
    };

    {
        let mut presence = state.copy.presence.lock().await;
        let entry = presence.entry(account_id.clone()).or_default();
        entry.last_seen = Some(Instant::now());
        entry.last_seen_iso = Some(now_iso());
        for spec in &payload.account.specs {
            if !spec.symbol.is_empty() {
                entry.specs.insert(spec.symbol.clone(), spec.clone());
            }
        }
    }

    let Some(group) = sqlx::query("SELECT id, enabled FROM copy_groups WHERE master_account_id = ?")
        .bind(&account_id)
        .fetch_optional(pool)
        .await
        .map_err(internal)?
    else {
        return Ok(Json(json!({ "ok": true, "ignored": "La cuenta no es principal de ningún grupo" })));
    };
    let group_id: String = group.get("id");
    let enabled = group.get::<i64, _>("enabled") != 0;
    if !enabled || !global_enabled(pool).await.map_err(internal)? {
        return Ok(Json(json!({ "ok": true, "ignored": "Copia desactivada" })));
    }

    let followers = load_followers(pool, &group_id).await.map_err(internal)?;
    let mut planned_count = 0;
    let mut signal_ids = Vec::new();
    for signal in &payload.signals {
        if signal.position_ref.trim().is_empty() || signal.symbol.trim().is_empty() {
            continue;
        }
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM copy_signals WHERE group_id = ?",
        )
        .bind(&group_id)
        .fetch_one(pool)
        .await
        .map_err(internal)?;
        let signal_id = new_id();
        let raw = serde_json::to_string(&json!({
            "kind": signal.kind,
            "ref": signal.position_ref,
            "order_ref": signal.order_ref,
            "symbol": signal.symbol,
            "market": signal.market,
            "side": signal.side,
            "volume": signal.volume,
            "position_volume": signal.position_volume,
            "price": signal.price,
            "sl": signal.sl,
            "tp": signal.tp,
            "order_type": signal.order_type,
            "order_price": signal.order_price,
        }))
        .unwrap_or_default();
        sqlx::query(
            "INSERT INTO copy_signals
             (id, group_id, seq, kind, master_ref, symbol, side, volume, price, payload_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&signal_id)
        .bind(&group_id)
        .bind(seq)
        .bind(&signal.kind)
        .bind(&signal.position_ref)
        .bind(&signal.symbol)
        .bind(&signal.side)
        .bind(signal.volume)
        .bind(signal.price)
        .bind(raw)
        .bind(now_iso())
        .execute(pool)
        .await
        .map_err(internal)?;
        signal_ids.push(signal_id.clone());

        for follower in &followers {
            let context = follower_context(&state, follower).await;
            let outcome = plan_for_follower(pool, &context, &provider, &account_id, follower, signal)
                .await
                .map_err(internal)?;
            if let Some(reason) = &outcome.pause {
                pause_follower(&state, &follower.id, reason, follower.settings.close_on_daily_loss)
                    .await
                    .map_err(internal)?;
            }
            if let Some(planned) = outcome.planned {
                if planned.skipped.is_none() {
                    planned_count += 1;
                }
                insert_order(
                    pool,
                    Some(&signal_id),
                    &group_id,
                    &follower.id,
                    &follower.account_id,
                    planned,
                )
                .await
                .map_err(internal)?;
            }
        }
    }

    if planned_count > 0 {
        state.copy.wake();
    }
    emit_activity(&state);
    Ok(Json(json!({
        "ok": true,
        "signals": signal_ids,
        "orders": planned_count,
    })))
}

pub(crate) async fn result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ResultPayload>,
) -> Result<Json<Value>, ApiError> {
    mt5::authorize(&state, &headers).await?;
    for item in &payload.results {
        apply_result(&state.db, item).await.map_err(internal)?;
    }
    emit_activity(&state);
    Ok(Json(json!({ "ok": true })))
}

async fn apply_result(pool: &SqlitePool, item: &OrderResult) -> AppResult<()> {
    let Some(row) = sqlx::query(
        "SELECT follower_id, master_ref, tag, action, symbol, side, payload_json, created_ms, status
         FROM copy_orders WHERE id = ?",
    )
    .bind(&item.order_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(());
    };
    let follower_id: String = row.get("follower_id");
    let master_ref: String = row.get("master_ref");
    let tag: String = row.get("tag");
    let action: String = row.get("action");
    let planned_symbol: String = row.get("symbol");
    let side: Option<String> = row.get("side");
    let created_ms: i64 = row.get("created_ms");
    let payload: Value =
        serde_json::from_str(&row.get::<String, _>("payload_json")).unwrap_or(Value::Null);

    let status = match item.status.as_str() {
        "filled" | "placed" | "done" | "rejected" | "skipped" => item.status.clone(),
        _ if item.ok => "done".to_string(),
        _ => "rejected".to_string(),
    };
    let symbol = item
        .symbol
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or(planned_symbol);
    sqlx::query(
        "UPDATE copy_orders SET status = ?, error = ?, fill_price = ?, fill_volume = ?,
         latency_ms = ?, symbol = ?, done_at = ? WHERE id = ?",
    )
    .bind(&status)
    .bind(&item.error)
    .bind(item.price)
    .bind(item.volume)
    .bind(now_ms() - created_ms)
    .bind(&symbol)
    .bind(now_iso())
    .bind(&item.order_id)
    .execute(pool)
    .await?;

    let ok = item.ok && matches!(status.as_str(), "filled" | "placed" | "done");
    let reduce_only = payload
        .get("reduce_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let master_order_ref = payload
        .get("master_order_ref")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let now = now_iso();

    match action.as_str() {
        "market" | "pending_fill" if !reduce_only => {
            if action == "pending_fill" {
                if let Some(order_ref) = &master_order_ref {
                    sqlx::query(
                        "DELETE FROM copy_pendings WHERE follower_id = ? AND master_order_ref = ?",
                    )
                    .bind(&follower_id)
                    .bind(order_ref)
                    .execute(pool)
                    .await?;
                }
            }
            let filled = item.volume.unwrap_or(0.0);
            if ok && filled > EPS {
                let side = side.unwrap_or_else(|| "buy".into());
                sqlx::query(
                    "INSERT INTO copy_positions
                     (follower_id, master_ref, tag, symbol, side, volume, follower_ref, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                     ON CONFLICT(follower_id, master_ref) DO UPDATE SET
                       volume = copy_positions.volume + excluded.volume,
                       follower_ref = COALESCE(excluded.follower_ref, copy_positions.follower_ref),
                       updated_at = excluded.updated_at",
                )
                .bind(&follower_id)
                .bind(&master_ref)
                .bind(&tag)
                .bind(&symbol)
                .bind(&side)
                .bind(filled)
                .bind(&item.position_ref)
                .bind(&now)
                .execute(pool)
                .await?;
            }
        }
        "market" | "close" => {
            if ok {
                let filled = item.volume.unwrap_or(0.0);
                if action == "close" {
                    sqlx::query("DELETE FROM copy_positions WHERE follower_id = ? AND master_ref = ?")
                        .bind(&follower_id)
                        .bind(&master_ref)
                        .execute(pool)
                        .await?;
                } else {
                    sqlx::query(
                        "UPDATE copy_positions SET volume = MAX(volume - ?, 0), updated_at = ?
                         WHERE follower_id = ? AND master_ref = ?",
                    )
                    .bind(filled)
                    .bind(&now)
                    .bind(&follower_id)
                    .bind(&master_ref)
                    .execute(pool)
                    .await?;
                    sqlx::query(
                        "DELETE FROM copy_positions WHERE follower_id = ? AND master_ref = ? AND volume <= 0.0000001",
                    )
                    .bind(&follower_id)
                    .bind(&master_ref)
                    .execute(pool)
                    .await?;
                }
            }
        }
        "pending_place" => {
            if ok {
                sqlx::query(
                    "INSERT INTO copy_pendings
                     (follower_id, master_order_ref, tag, symbol, follower_order_ref, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT(follower_id, master_order_ref) DO UPDATE SET
                       follower_order_ref = excluded.follower_order_ref,
                       symbol = excluded.symbol, updated_at = excluded.updated_at",
                )
                .bind(&follower_id)
                .bind(master_order_ref.as_deref().unwrap_or(&master_ref))
                .bind(&tag)
                .bind(&symbol)
                .bind(&item.order_ref)
                .bind(&now)
                .execute(pool)
                .await?;
            }
        }
        "pending_cancel" => {
            // Cancelada o ya inexistente: en ambos casos deja de estar copiada.
            sqlx::query("DELETE FROM copy_pendings WHERE follower_id = ? AND master_order_ref = ?")
                .bind(&follower_id)
                .bind(master_order_ref.as_deref().unwrap_or(&master_ref))
                .execute(pool)
                .await?;
        }
        _ => {}
    }
    Ok(())
}
