//! Integración con ATAS X.
//!
//! Hay dos fuentes de datos que se combinan:
//!
//! 1. La base de datos local de ATAS (`%APPDATA%\ATAS X\Database\*.cdb`). Son archivos de
//!    texto con cabecera `Columns:` y filas entre `StartData`/`EndData`. Contienen todas las
//!    ejecuciones (`MyTrade`), las operaciones emparejadas por ATAS con su PnL
//!    (`HistoryMyTrade`), las cuentas (`Portfolio`), las posiciones y los instrumentos.
//!    La app los relee cada pocos segundos cuando cambian; no hace falta tener ATAS abierto.
//! 2. El indicador `JournalTradingBridge` (opcional), que envía en tiempo real las
//!    ejecuciones de la sesión, el balance y la posición, y sirve velas bajo demanda.
//!
//! Las ejecuciones de ambas fuentes se fusionan por identificador y las operaciones se
//! reconstruyen siguiendo la posición neta (de plano a plano), igual que en NinjaTrader 8.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, Instant, SystemTime};

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use chrono::{Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use tauri::Emitter;
use tokio::sync::{Mutex, Notify};

use crate::db;
use crate::error::AppResult;
use crate::mt5::{
    self, ApiError, DealPayload, PROVIDER_ATAS, PositionPayload, SymbolPayload, SyncPayload,
};
use crate::state::AppState;
use crate::util::now_iso;

pub const ATAS_SERVER: &str = "ATAS X";
const POLL_SECONDS: u64 = 5;
const INDICATOR_TIMEOUT: StdDuration = StdDuration::from_secs(30);
const HISTORY_FROM_UTC: &str = "2000-01-01T00:00:00Z";
const EPS: f64 = 1e-9;

const DB_FILES: [&str; 5] = [
    "MyTrade.cdb",
    "HistoryMyTrade.cdb",
    "Portfolio.cdb",
    "Position.cdb",
    "Security.cdb",
];

// ---------------------------------------------------------------------------
// Estado en memoria
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct AtasRuntime {
    pub wake: Notify,
    inner: Mutex<AtasInner>,
}

#[derive(Default)]
struct AtasInner {
    stamps: HashMap<&'static str, (SystemTime, u64)>,
    db: AtasDb,
    live: HashMap<String, LiveAccount>,
    /// Huella del último payload enviado por cuenta, para no reingerir sin cambios.
    sent: HashMap<String, u64>,
    /// Último balance guardado por cuenta (para no llenar account_snapshots).
    balances: HashMap<String, (f64, f64)>,
    indicator_seen: Option<Instant>,
    indicator_version: Option<String>,
}

#[derive(Default)]
struct LiveAccount {
    trades: HashMap<String, AtasExec>,
    balance: Option<f64>,
    open_pnl: Option<f64>,
    currency: Option<String>,
    position: Option<AtasPosition>,
}

#[derive(Debug, Clone, Default)]
struct AtasDb {
    execs: Vec<AtasExec>,
    /// PnL de ATAS por (cuenta, id de la ejecución de salida).
    exit_pnl: HashMap<(String, String), f64>,
    portfolios: HashMap<String, AtasPortfolio>,
    positions: HashMap<(String, String), AtasPosition>,
    securities: HashMap<String, AtasSecurity>,
}

#[derive(Debug, Clone, PartialEq)]
struct AtasExec {
    id: String,
    account: String,
    security: String,
    buy: bool,
    price: f64,
    volume: f64,
    time_utc: NaiveDateTime,
    commission: f64,
}

#[derive(Debug, Clone, Default)]
struct AtasPortfolio {
    currency: String,
    balance: f64,
    open_pnl: f64,
}

#[derive(Debug, Clone, Default)]
struct AtasPosition {
    security: String,
    unrealized: f64,
}

#[derive(Debug, Clone, Default)]
struct AtasSecurity {
    instrument: String,
    tick_size: f64,
    tick_cost: f64,
}

impl AtasRuntime {
    /// Fuerza a reenviar todas las cuentas en la siguiente pasada (botón Sincronizar).
    pub async fn request_full_sync(&self) {
        let mut inner = self.inner.lock().await;
        inner.sent.clear();
        inner.stamps.clear();
        drop(inner);
        self.wake.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Rutas de ATAS
// ---------------------------------------------------------------------------

pub fn atas_db_dir() -> Option<PathBuf> {
    let appdata = PathBuf::from(std::env::var_os("APPDATA")?);
    ["ATAS X", "ATAS"]
        .iter()
        .map(|name| appdata.join(name).join("Database"))
        .find(|path| path.join("MyTrade.cdb").is_file())
}

pub fn atas_indicators_dir() -> Option<PathBuf> {
    let appdata = PathBuf::from(std::env::var_os("APPDATA")?);
    ["ATAS X", "ATAS"]
        .iter()
        .map(|name| appdata.join(name).join("Indicators"))
        .find(|path| path.is_dir())
}

// ---------------------------------------------------------------------------
// Bucle de lectura
// ---------------------------------------------------------------------------

pub fn spawn(state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = poll_once(&state).await {
                log::warn!("ATAS: {error}");
                set_status_error(&state, Some(error.to_string())).await;
            }
            tokio::select! {
                _ = tokio::time::sleep(StdDuration::from_secs(POLL_SECONDS)) => {}
                _ = state.atas.wake.notified() => {}
            }
        }
    });
}

async fn poll_once(state: &AppState) -> AppResult<()> {
    let Some(dir) = atas_db_dir() else {
        set_status_error(
            state,
            Some("No se encontró la base de datos de ATAS X (%APPDATA%\\ATAS X\\Database).".into()),
        )
        .await;
        return Ok(());
    };

    reload_changed_files(state, &dir).await;
    let skip = nt8_account_names(&state.db).await?;
    let payloads = {
        let inner = state.atas.inner.lock().await;
        build_payloads(&inner.db, &inner.live, &skip)
    };

    for account in payloads {
        ingest_account(state, account).await?;
    }

    mark_reader_alive(state, &dir).await;
    Ok(())
}

async fn reload_changed_files(state: &AppState, dir: &Path) {
    let mut changed: Vec<&'static str> = Vec::new();
    {
        let inner = state.atas.inner.lock().await;
        for name in DB_FILES {
            let stamp = std::fs::metadata(dir.join(name))
                .ok()
                .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
            if let Some(stamp) = stamp
                && inner.stamps.get(name) != Some(&stamp)
            {
                changed.push(name);
            }
        }
    }
    if changed.is_empty() {
        return;
    }

    // La lectura y el parseo (Security.cdb pesa varios MB) se hacen fuera del runtime async.
    let dir = dir.to_path_buf();
    let names = changed.clone();
    let parsed = tokio::task::spawn_blocking(move || {
        names
            .into_iter()
            .map(|name| {
                let path = dir.join(name);
                let stamp = std::fs::metadata(&path)
                    .ok()
                    .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
                let rows = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| parse_cdb(&String::from_utf8_lossy(&bytes)));
                (name, stamp, rows)
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();

    let mut inner = state.atas.inner.lock().await;
    for (name, stamp, rows) in parsed {
        // Si el archivo está a medio escribir (sin EndData) se reintenta en la siguiente pasada.
        let (Some(stamp), Some(rows)) = (stamp, rows) else {
            continue;
        };
        apply_table(&mut inner.db, name, &rows);
        inner.stamps.insert(name, stamp);
    }
}

fn apply_table(db: &mut AtasDb, name: &str, rows: &[HashMap<String, String>]) {
    match name {
        "MyTrade.cdb" => {
            db.execs = rows.iter().filter_map(exec_from_row).collect();
        }
        "HistoryMyTrade.cdb" => {
            let mut map = HashMap::new();
            for row in rows {
                let account = field(row, "AccountID");
                let exit = field(row, "ExitTradeId");
                if account.is_empty() || exit.is_empty() {
                    continue;
                }
                if let Some(pnl) = number(row, "PnL") {
                    *map.entry((account.to_string(), exit.to_string()))
                        .or_insert(0.0) += pnl;
                }
            }
            db.exit_pnl = map;
        }
        "Portfolio.cdb" => {
            db.portfolios = rows
                .iter()
                .filter(|row| !field(row, "AccountID").is_empty())
                .map(|row| {
                    (
                        field(row, "AccountID").to_string(),
                        AtasPortfolio {
                            currency: field(row, "Currency").to_string(),
                            balance: number(row, "Balance").unwrap_or(0.0),
                            open_pnl: number(row, "OpenPnL").unwrap_or(0.0),
                        },
                    )
                })
                .collect();
        }
        "Position.cdb" => {
            db.positions = rows
                .iter()
                .filter(|row| !field(row, "AccountID").is_empty())
                .map(|row| {
                    (
                        (
                            field(row, "AccountID").to_string(),
                            field(row, "SecurityId").to_string(),
                        ),
                        AtasPosition {
                            security: field(row, "SecurityId").to_string(),
                            unrealized: number(row, "UnrealizedPnL").unwrap_or(0.0),
                        },
                    )
                })
                .collect();
        }
        "Security.cdb" => {
            db.securities = rows
                .iter()
                .filter(|row| !field(row, "Id").is_empty())
                .map(|row| {
                    (
                        field(row, "Id").to_string(),
                        AtasSecurity {
                            instrument: field(row, "Instrument").replace("\\'", "'"),
                            tick_size: number(row, "TickSize").unwrap_or(0.0),
                            tick_cost: number(row, "TickCost").unwrap_or(0.0),
                        },
                    )
                })
                .collect();
        }
        _ => {}
    }
}

fn exec_from_row(row: &HashMap<String, String>) -> Option<AtasExec> {
    let id = field(row, "Id");
    let account = field(row, "AccountID");
    let security = field(row, "SecurityId");
    if id.is_empty() || account.is_empty() || security.is_empty() {
        return None;
    }
    let buy = match field(row, "OrderDirection") {
        "Buy" => true,
        "Sell" => false,
        _ => return None,
    };
    let volume = number(row, "Volume")?.abs();
    if volume <= EPS {
        return None;
    }
    Some(AtasExec {
        id: id.to_string(),
        account: account.to_string(),
        security: security.to_string(),
        buy,
        price: number(row, "Price")?,
        volume,
        time_utc: parse_atas_time(field(row, "Time"))?,
        commission: number(row, "Commission").unwrap_or(0.0),
    })
}

// ---------------------------------------------------------------------------
// Reconstrucción de operaciones
// ---------------------------------------------------------------------------

struct AccountPayload {
    account_id: String,
    currency: String,
    balance: f64,
    equity: f64,
    sync: SyncPayload,
}

fn is_ignored_account(account: &str) -> bool {
    let lower = account.to_lowercase();
    lower.contains("replay") || lower.contains("backtest") || lower.contains("playback")
}

/// Nombres de instrumento al estilo NinjaTrader ("NQ 12-26"): ATAS los usa cuando la
/// cuenta llega a través de su conector de NT8.
fn is_nt8_instrument(name: &str) -> bool {
    let mut parts = name.rsplitn(2, ' ');
    let (Some(expiry), Some(root)) = (parts.next(), parts.next()) else {
        return false;
    };
    let bytes = expiry.as_bytes();
    !root.is_empty()
        && bytes.len() == 5
        && bytes[2] == b'-'
        && bytes[..2].iter().all(u8::is_ascii_digit)
        && bytes[3..].iter().all(u8::is_ascii_digit)
}

async fn nt8_account_names(pool: &SqlitePool) -> AppResult<HashSet<String>> {
    let rows = sqlx::query("SELECT display_name FROM accounts WHERE provider = 'ninjatrader'")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| row.get::<String, _>("display_name").trim().to_string())
        .collect())
}

fn build_payloads(
    db: &AtasDb,
    live: &HashMap<String, LiveAccount>,
    nt8_accounts: &HashSet<String>,
) -> Vec<AccountPayload> {
    // Ejecuciones fusionadas por cuenta e id (las del indicador completan a las de la BD).
    let mut execs: BTreeMap<String, HashMap<String, AtasExec>> = BTreeMap::new();
    for exec in &db.execs {
        execs
            .entry(exec.account.clone())
            .or_default()
            .insert(exec.id.clone(), exec.clone());
    }
    for (account, data) in live {
        let entry = execs.entry(account.clone()).or_default();
        for (id, exec) in &data.trades {
            entry.entry(id.clone()).or_insert_with(|| exec.clone());
        }
    }

    let mut accounts: HashSet<String> = db.portfolios.keys().cloned().collect();
    accounts.extend(execs.keys().cloned());
    accounts.extend(live.keys().cloned());

    let mut result = Vec::new();
    let mut ordered: Vec<String> = accounts.into_iter().collect();
    ordered.sort();
    for account in ordered {
        if account.trim().is_empty() || is_ignored_account(&account) {
            continue;
        }
        if nt8_accounts.contains(account.trim()) {
            continue;
        }
        let account_execs: Vec<AtasExec> = execs
            .get(&account)
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default();
        let traded: HashSet<&str> = account_execs.iter().map(|e| e.security.as_str()).collect();
        if !traded.is_empty()
            && traded.iter().all(|security| {
                db.securities
                    .get(*security)
                    .map(|info| is_nt8_instrument(&info.instrument))
                    .unwrap_or(false)
            })
        {
            // Cuenta que ATAS recibe a través de NinjaTrader: ya la sincroniza el AddOn de NT8.
            continue;
        }

        let portfolio = db.portfolios.get(&account).cloned().unwrap_or_default();
        let live_account = live.get(&account);
        let currency = live_account
            .and_then(|data| data.currency.clone())
            .filter(|value| value.len() == 3)
            .unwrap_or_else(|| {
                if portfolio.currency.len() == 3 {
                    portfolio.currency.clone()
                } else {
                    "USD".to_string()
                }
            });
        let balance = live_account
            .and_then(|data| data.balance)
            .unwrap_or(portfolio.balance);
        let open_pnl = live_account
            .and_then(|data| data.open_pnl)
            .unwrap_or(portfolio.open_pnl);

        let sync = build_sync(&account, &currency, account_execs, db, live_account);
        result.push(AccountPayload {
            account_id: account,
            currency,
            balance,
            equity: balance + open_pnl,
            sync,
        });
    }
    result
}

struct Leg {
    exec: AtasExec,
    ticket: i64,
    entry: bool,
    volume: f64,
    profit: f64,
    commission: f64,
}

struct Trade {
    ticket: i64,
    security: String,
    long: bool,
    open_volume: f64,
    open_avg: f64,
    first_time: NaiveDateTime,
    legs: Vec<Leg>,
}

fn build_sync(
    account: &str,
    currency: &str,
    mut execs: Vec<AtasExec>,
    db: &AtasDb,
    live: Option<&LiveAccount>,
) -> SyncPayload {
    execs.sort_by(|a, b| {
        (a.security.as_str(), a.time_utc, a.id.as_str()).cmp(&(
            b.security.as_str(),
            b.time_utc,
            b.id.as_str(),
        ))
    });

    let mut trades: Vec<Trade> = Vec::new();
    let mut current: Option<usize> = None;
    let mut net = 0.0_f64;
    let mut last_security = String::new();

    for exec in execs {
        if exec.security != last_security {
            last_security = exec.security.clone();
            current = None;
            net = 0.0;
        }
        let sign = if exec.buy { 1.0 } else { -1.0 };
        let mut remaining = exec.volume;
        let mut leg_index = 0;
        let security = db.securities.get(&exec.security);

        if let Some(index) = current
            && net.abs() > EPS
            && net.signum() != sign
        {
            let close = remaining.min(net.abs());
            let trade = &mut trades[index];
            let direction = if trade.long { 1.0 } else { -1.0 };
            let profit = db
                .exit_pnl
                .get(&(account.to_string(), exec.id.clone()))
                .copied()
                .unwrap_or_else(|| {
                    let points = (exec.price - trade.open_avg) * direction * close;
                    match security {
                        Some(info) if info.tick_size > 0.0 && info.tick_cost > 0.0 => {
                            points / info.tick_size * info.tick_cost
                        }
                        _ => points,
                    }
                });
            trade.legs.push(Leg {
                ticket: ticket(&[account, &exec.id, &leg_index.to_string()]),
                exec: exec.clone(),
                entry: false,
                volume: close,
                profit,
                commission: exec.commission * close / exec.volume,
            });
            trade.open_volume = (trade.open_volume - close).max(0.0);
            leg_index += 1;
            net += sign * close;
            remaining -= close;
            if net.abs() <= EPS {
                net = 0.0;
                current = None;
            }
        }

        if remaining > EPS {
            let index = match current {
                Some(index) => index,
                None => {
                    trades.push(Trade {
                        ticket: ticket(&[
                            account,
                            &exec.security,
                            &exec.id,
                            &leg_index.to_string(),
                        ]),
                        security: exec.security.clone(),
                        long: sign > 0.0,
                        open_volume: 0.0,
                        open_avg: 0.0,
                        first_time: exec.time_utc,
                        legs: Vec::new(),
                    });
                    current = Some(trades.len() - 1);
                    trades.len() - 1
                }
            };
            let trade = &mut trades[index];
            trade.open_avg = (trade.open_avg * trade.open_volume + exec.price * remaining)
                / (trade.open_volume + remaining);
            trade.open_volume += remaining;
            trade.legs.push(Leg {
                ticket: ticket(&[account, &exec.id, &leg_index.to_string()]),
                exec: exec.clone(),
                entry: true,
                volume: remaining,
                profit: 0.0,
                commission: exec.commission * remaining / exec.volume,
            });
            net += sign * remaining;
        }
    }

    let mut symbols: BTreeMap<String, SymbolPayload> = BTreeMap::new();
    let mut deals = Vec::new();
    let mut positions = Vec::new();
    for trade in &trades {
        let info = db.securities.get(&trade.security);
        symbols
            .entry(trade.security.clone())
            .or_insert_with(|| symbol_payload(&trade.security, info, currency));
        for leg in &trade.legs {
            let (time_server, time_utc) = time_strings(leg.exec.time_utc);
            deals.push(DealPayload {
                deal_ticket: leg.ticket,
                position_ticket: trade.ticket,
                symbol: trade.security.clone(),
                kind: if leg.exec.buy { 0 } else { 1 },
                entry: if leg.entry { 0 } else { 1 },
                volume: leg.volume,
                price: leg.exec.price,
                profit: leg.profit,
                commission: -leg.commission.abs(),
                swap: 0.0,
                magic: Some(0),
                comment: None,
                reason: -1,
                time_server,
                time: None,
                time_utc: Some(time_utc),
            });
        }
        if trade.open_volume > EPS {
            let live_position = live
                .and_then(|data| data.position.as_ref())
                .filter(|position| position.security == trade.security);
            let stored = db
                .positions
                .get(&(account.to_string(), trade.security.clone()));
            let unrealized = live_position
                .or(stored)
                .map(|position| position.unrealized)
                .unwrap_or(0.0);
            let (time_server, time_utc) = time_strings(trade.first_time);
            positions.push(PositionPayload {
                ticket: trade.ticket,
                symbol: trade.security.clone(),
                kind: if trade.long { 0 } else { 1 },
                volume: trade.open_volume,
                price_open: trade.open_avg,
                sl: None,
                tp: None,
                profit: unrealized,
                swap: 0.0,
                commission: 0.0,
                magic: Some(0),
                comment: None,
                time_server,
                time: None,
                time_utc: Some(time_utc),
                tick_size: info.map(|value| value.tick_size).filter(|v| *v > 0.0),
                tick_value: info.map(|value| value.tick_cost).filter(|v| *v > 0.0),
            });
        }
    }

    SyncPayload {
        login: login_for(account),
        server: ATAS_SERVER.to_string(),
        broker_utc_offset_seconds: Some(local_offset_seconds()),
        symbols: symbols.into_values().collect(),
        positions,
        deals,
        history_from_utc: Some(HISTORY_FROM_UTC.to_string()),
    }
}

fn symbol_payload(security: &str, info: Option<&AtasSecurity>, currency: &str) -> SymbolPayload {
    let tick_size = info
        .map(|value| value.tick_size)
        .filter(|v| *v > 0.0)
        .unwrap_or(0.01);
    let tick_value = info
        .map(|value| value.tick_cost)
        .filter(|v| *v > 0.0)
        .unwrap_or(tick_size);
    SymbolPayload {
        symbol: security.to_string(),
        description: info
            .map(|value| value.instrument.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| security.to_string()),
        digits: digits_for(tick_size),
        tick_size,
        tick_value,
        contract_size: 1.0,
        currency_profit: currency.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Ingesta
// ---------------------------------------------------------------------------

async fn ingest_account(state: &AppState, account: AccountPayload) -> AppResult<()> {
    let fingerprint = payload_fingerprint(&account);
    let login = account.sync.login;
    {
        let inner = state.atas.inner.lock().await;
        if inner.sent.get(&account.account_id) == Some(&fingerprint) {
            return Ok(());
        }
    }

    let (record, is_new) = mt5::ensure_account(
        &state.db,
        PROVIDER_ATAS,
        login,
        ATAS_SERVER,
        Some(&account.account_id),
        Some(ATAS_SERVER),
        Some(&account.currency),
        None,
    )
    .await?;
    let offset = local_offset_seconds();
    db::touch_account_heartbeat(
        &state.db,
        &record.id,
        offset,
        account.balance,
        account.equity,
    )
    .await?;

    let balance_changed = {
        let inner = state.atas.inner.lock().await;
        inner.balances.get(&account.account_id) != Some(&(account.balance, account.equity))
    };
    if balance_changed {
        db::insert_snapshot(
            &state.db,
            &record.id,
            account.balance,
            account.equity,
            0.0,
            0.0,
        )
        .await?;
    }

    let stats = mt5::ingest_sync(&state.db, &record.id, offset, &account.sync).await?;
    mt5::reconcile_history(&state.db, &record.id, HISTORY_FROM_UTC, &account.sync).await?;

    {
        let mut inner = state.atas.inner.lock().await;
        inner.sent.insert(account.account_id.clone(), fingerprint);
        inner.balances.insert(
            account.account_id.clone(),
            (account.balance, account.equity),
        );
    }

    let _ = state.app.emit(
        "trades://updated",
        json!({
            "accountId": record.id,
            "positions": stats.positions,
            "deals": stats.deals,
            "newTrades": stats.new_trades,
        }),
    );
    let _ = state
        .app
        .emit("journal://updated", json!({ "accountId": record.id }));
    let _ = state.app.emit(
        "atas://heartbeat",
        json!({
            "accountId": record.id,
            "login": record.login,
            "server": record.server,
            "provider": PROVIDER_ATAS,
            "balance": account.balance,
            "equity": account.equity,
        }),
    );
    if is_new {
        let _ = state
            .app
            .emit("accounts://updated", json!({ "accountId": record.id }));
    }
    Ok(())
}

fn payload_fingerprint(account: &AccountPayload) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    account.balance.to_bits().hash(&mut hasher);
    account.equity.to_bits().hash(&mut hasher);
    account.currency.hash(&mut hasher);
    for deal in &account.sync.deals {
        deal.deal_ticket.hash(&mut hasher);
        deal.position_ticket.hash(&mut hasher);
        deal.volume.to_bits().hash(&mut hasher);
        deal.price.to_bits().hash(&mut hasher);
        deal.profit.to_bits().hash(&mut hasher);
        deal.commission.to_bits().hash(&mut hasher);
    }
    for position in &account.sync.positions {
        position.ticket.hash(&mut hasher);
        position.volume.to_bits().hash(&mut hasher);
        position.profit.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

async fn mark_reader_alive(state: &AppState, dir: &Path) {
    let (indicator_version, indicator_alive) = {
        let inner = state.atas.inner.lock().await;
        let alive = inner
            .indicator_seen
            .map(|seen| seen.elapsed() < INDICATOR_TIMEOUT)
            .unwrap_or(false);
        (inner.indicator_version.clone(), alive)
    };
    let snapshot = {
        let mut status = state.bridge.atas.write().await;
        status.running = true;
        status.connected = true;
        status.error = None;
        status.last_heartbeat_at = Some(now_iso());
        status.server = Some(ATAS_SERVER.to_string());
        status.terminal_data_path = Some(dir.to_string_lossy().to_string());
        status.ea_version = if indicator_alive {
            indicator_version
        } else {
            None
        };
        status.clone()
    };
    let _ = state.app.emit("atas://status", snapshot);
}

async fn set_status_error(state: &AppState, error: Option<String>) {
    let snapshot = {
        let mut status = state.bridge.atas.write().await;
        status.connected = false;
        status.error = error;
        status.clone()
    };
    let _ = state.app.emit("atas://status", snapshot);
}

// ---------------------------------------------------------------------------
// Indicador (tiempo real)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct IndicatorHeartbeat {
    account_id: String,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    balance: Option<f64>,
    #[serde(default)]
    open_pnl: Option<f64>,
    #[serde(default)]
    indicator_version: Option<String>,
    #[serde(default)]
    position: Option<IndicatorPosition>,
    #[serde(default)]
    my_trades: Vec<IndicatorTrade>,
}

#[derive(Debug, Deserialize)]
struct IndicatorPosition {
    security_id: String,
    #[serde(default)]
    unrealized_pnl: f64,
}

#[derive(Debug, Deserialize)]
struct IndicatorTrade {
    id: String,
    security_id: String,
    direction: String,
    price: f64,
    volume: f64,
    time_utc: String,
    #[serde(default)]
    commission: Option<f64>,
}

pub async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<IndicatorHeartbeat>,
) -> Result<Json<Value>, ApiError> {
    mt5::authorize(&state, &headers).await?;
    let account_id = payload.account_id.trim().to_string();
    if account_id.is_empty() {
        return Ok(Json(json!({ "ok": false, "error": "Cuenta vacía" })));
    }

    let skipped = is_ignored_account(&account_id)
        || nt8_account_names(&state.db)
            .await
            .map_err(mt5::internal)?
            .contains(&account_id);

    {
        let mut inner = state.atas.inner.lock().await;
        inner.indicator_seen = Some(Instant::now());
        inner.indicator_version = payload.indicator_version.clone();
        if !skipped {
            let live = inner.live.entry(account_id.clone()).or_default();
            live.balance = payload.balance;
            live.open_pnl = payload.open_pnl;
            live.currency = payload.currency.clone();
            live.position = payload.position.as_ref().map(|position| AtasPosition {
                security: position.security_id.clone(),
                unrealized: position.unrealized_pnl,
            });
            for trade in &payload.my_trades {
                let Some(time_utc) = parse_utc_naive(&trade.time_utc) else {
                    continue;
                };
                let buy = trade.direction.eq_ignore_ascii_case("buy");
                if trade.id.trim().is_empty() || trade.volume.abs() <= EPS {
                    continue;
                }
                live.trades.insert(
                    trade.id.clone(),
                    AtasExec {
                        id: trade.id.clone(),
                        account: account_id.clone(),
                        security: trade.security_id.clone(),
                        buy,
                        price: trade.price,
                        volume: trade.volume.abs(),
                        time_utc,
                        commission: trade.commission.unwrap_or(0.0),
                    },
                );
            }
        }
    }
    state.atas.wake.notify_one();

    if skipped {
        return Ok(Json(json!({
            "ok": true,
            "skipped": true,
            "message": "Esta cuenta se sincroniza desde NinjaTrader 8 o es de simulación.",
            "commands": [],
        })));
    }

    let (record, _) = mt5::ensure_account(
        &state.db,
        PROVIDER_ATAS,
        login_for(&account_id),
        ATAS_SERVER,
        Some(&account_id),
        Some(ATAS_SERVER),
        payload.currency.as_deref(),
        None,
    )
    .await
    .map_err(mt5::internal)?;
    let commands = mt5::take_commands(&state.db, &record.id)
        .await
        .map_err(mt5::internal)?;

    Ok(Json(json!({
        "ok": true,
        "accountId": record.id,
        "login": record.login,
        "server": record.server,
        "accountStatus": record.status,
        "commands": commands,
        "serverTime": now_iso(),
    })))
}

// ---------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------

/// Login numérico estable a partir del AccountID de ATAS (53 bits, > 0).
pub fn login_for(account: &str) -> i64 {
    ticket(&["atas-account", account])
}

fn ticket(parts: &[&str]) -> i64 {
    // FNV-1a de 64 bits recortado a 53 bits para que quepa exacto en JavaScript.
    let mut hash: u64 = 0xcbf29ce484222325;
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            hash ^= u64::from(b'|');
            hash = hash.wrapping_mul(0x100000001b3);
        }
        for byte in part.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    let value = (hash & 0x1F_FFFF_FFFF_FFFF) as i64;
    if value > 0 { value } else { 1 }
}

fn digits_for(tick_size: f64) -> i64 {
    let mut digits = 0;
    let mut value = tick_size;
    while digits < 10 && (value - value.round()).abs() > 1e-9 {
        value *= 10.0;
        digits += 1;
    }
    digits
}

fn local_offset_seconds() -> i64 {
    i64::from(Local::now().offset().local_minus_utc())
}

/// (hora del bróker = hora local del equipo, hora UTC).
fn time_strings(utc: NaiveDateTime) -> (String, String) {
    let utc = Utc.from_utc_datetime(&utc);
    (
        utc.with_timezone(&Local)
            .naive_local()
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string(),
        utc.to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}

/// ATAS guarda las horas en UTC con formato `MM/dd/yyyy HH:mm:ss`.
fn parse_atas_time(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    for format in [
        "%m/%d/%Y %H:%M:%S%.f",
        "%m/%d/%Y %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, format) {
            return Some(parsed);
        }
    }
    NaiveDate::parse_from_str(value, "%m/%d/%Y")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
}

fn parse_utc_naive(value: &str) -> Option<NaiveDateTime> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|parsed| parsed.naive_utc())
        .or_else(|| parse_atas_time(value))
}

fn field<'a>(row: &'a HashMap<String, String>, key: &str) -> &'a str {
    row.get(key).map(|value| value.trim()).unwrap_or("")
}

fn number(row: &HashMap<String, String>, key: &str) -> Option<f64> {
    let value = field(row, key);
    if value.is_empty() {
        return None;
    }
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Parsea un archivo `.cdb` de ATAS. Devuelve `None` si está incompleto (sin `EndData`).
fn parse_cdb(text: &str) -> Option<Vec<HashMap<String, String>>> {
    let text = text.trim_start_matches('\u{feff}');
    let columns_start = text.find("Columns:")? + "Columns:".len();
    let columns_end = columns_start + text[columns_start..].find('\n')?;
    let columns: Vec<String> = parse_fields(&text[columns_start..columns_end])
        .into_iter()
        .flatten()
        .next()?
        .into_iter()
        .collect();
    let data_start = text.find("StartData")? + "StartData".len();
    let data_end = text.rfind("EndData")?;
    if data_end < data_start {
        return None;
    }
    let rows = parse_fields(&text[data_start..data_end])?;
    Some(
        rows.into_iter()
            .filter(|row| row.len() == columns.len())
            .map(|row| columns.iter().cloned().zip(row).collect())
            .collect(),
    )
}

/// Filas de campos separados por comas; los valores van entre comillas dobles con `\` como
/// escape y los vacíos sin comillas.
fn parse_fields(text: &str) -> Option<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut has_content = false;
    let mut chars = text.chars();

    while let Some(character) = chars.next() {
        if in_quotes {
            match character {
                '\\' => {
                    if let Some(next) = chars.next() {
                        match next {
                            '"' | '\\' => field.push(next),
                            other => {
                                field.push('\\');
                                field.push(other);
                            }
                        }
                    }
                }
                '"' => in_quotes = false,
                other => field.push(other),
            }
            continue;
        }
        match character {
            '"' => {
                in_quotes = true;
                has_content = true;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                has_content = true;
            }
            '\n' => {
                if has_content {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                has_content = false;
            }
            '\r' | ' ' | '\t' => {}
            other => {
                field.push(other);
                has_content = true;
            }
        }
    }
    if in_quotes {
        return None;
    }
    if has_content {
        row.push(field);
        rows.push(row);
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MY_TRADES: &str = "Version: \"1\"\r\nEntity: \"MyTrade\"\r\nColumns: \"Route\", \"AccountID\", \"SecurityId\", \"Id\", \"OrderId\", \"OrderDirection\", \"Price\", \"Time\", \"Volume\", \"OpenVolume\", \"Commission\", \"CommissionCurrency\"\r\nStartData\r\n, \"ACC1\", \"NQZ6@CME\", \"1\", \"10\", \"Sell\", \"29413.25\", \"09/16/2026 13:42:47\", \"1\", \"0\", \"0\", \r\n, \"ACC1\", \"NQZ6@CME\", \"2\", \"11\", \"Buy\", \"29422.5\", \"09/16/2026 13:45:51\", \"2\", \"0\", \"0\", \r\n, \"ACC1\", \"NQZ6@CME\", \"3\", \"12\", \"Sell\", \"29430\", \"09/16/2026 13:50:00\", \"1\", \"0\", \"0\", \r\nEndData\r\n";

    const HISTORY: &str = "Version: \"2\"\r\nEntity: \"HistoryMyTrade\"\r\nColumns: \"Id\", \"AccountID\", \"SecurityId\", \"PnL\", \"Comment\", \"EnterTradeId\", \"ExitTradeId\"\r\nStartData\r\n\"5\", \"ACC1\", \"NQZ6@CME\", \"-185.00\", , \"1\", \"2\"\r\n\"6\", \"ACC1\", \"NQZ6@CME\", \"150.00\", \"con \\\"comillas\\\"\", \"2\", \"3\"\r\nEndData\r\n";

    #[test]
    fn parses_cdb_rows_and_rejects_partial_files() {
        let rows = parse_cdb(MY_TRADES).expect("rows");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["Route"], "");
        assert_eq!(rows[0]["AccountID"], "ACC1");
        assert_eq!(rows[1]["OrderDirection"], "Buy");

        let history = parse_cdb(HISTORY).expect("history");
        assert_eq!(history[1]["Comment"], "con \"comillas\"");

        let partial = MY_TRADES.replace("EndData\r\n", "");
        assert!(parse_cdb(&partial).is_none());
    }

    #[test]
    fn rebuilds_trades_with_reversal_and_atas_pnl() {
        let mut db = AtasDb::default();
        apply_table(&mut db, "MyTrade.cdb", &parse_cdb(MY_TRADES).unwrap());
        apply_table(&mut db, "HistoryMyTrade.cdb", &parse_cdb(HISTORY).unwrap());
        db.securities.insert(
            "NQZ6@CME".into(),
            AtasSecurity {
                instrument: "E-mini Nasdaq-100 Index Futures, Dec-26".into(),
                tick_size: 0.25,
                tick_cost: 5.0,
            },
        );

        let payloads = build_payloads(&db, &HashMap::new(), &HashSet::new());
        assert_eq!(payloads.len(), 1);
        let sync = &payloads[0].sync;

        // Corto 1 -> compra 2 (cierra el corto y abre largo 1) -> venta 1 (cierra el largo).
        assert_eq!(sync.deals.len(), 4);
        let tickets: HashSet<i64> = sync.deals.iter().map(|d| d.position_ticket).collect();
        assert_eq!(tickets.len(), 2);
        assert!(sync.positions.is_empty());

        let first_exit = &sync.deals[1];
        assert_eq!(first_exit.entry, 1);
        assert!((first_exit.profit + 185.0).abs() < 1e-9);
        let second_exit = &sync.deals[3];
        assert!((second_exit.profit - 150.0).abs() < 1e-9);
        assert_eq!(sync.deals[2].entry, 0);
        assert!((sync.deals[2].volume - 1.0).abs() < 1e-9);
    }

    #[test]
    fn skips_simulation_and_ninjatrader_accounts() {
        let mut db = AtasDb::default();
        let text = MY_TRADES.replace("NQZ6@CME", "NQZ6");
        apply_table(&mut db, "MyTrade.cdb", &parse_cdb(&text).unwrap());
        db.securities.insert(
            "NQZ6".into(),
            AtasSecurity {
                instrument: "NQ 12-26".into(),
                tick_size: 0.25,
                tick_cost: 5.0,
            },
        );
        db.portfolios
            .insert("Replay".into(), AtasPortfolio::default());
        db.portfolios
            .insert("DEMO1".into(), AtasPortfolio::default());

        let payloads = build_payloads(&db, &HashMap::new(), &HashSet::new());
        let names: Vec<&str> = payloads.iter().map(|p| p.account_id.as_str()).collect();
        assert_eq!(names, vec!["DEMO1"]);

        let mut nt8 = HashSet::new();
        nt8.insert("DEMO1".to_string());
        assert!(build_payloads(&db, &HashMap::new(), &nt8).is_empty());
        assert!(is_nt8_instrument("NQ 12-26"));
        assert!(!is_nt8_instrument(
            "E-mini Nasdaq-100 Index Futures, Dec-26"
        ));
    }

    #[test]
    fn open_trade_produces_position() {
        let mut db = AtasDb::default();
        let text = MY_TRADES.replace(
            ", \"ACC1\", \"NQZ6@CME\", \"3\", \"12\", \"Sell\", \"29430\", \"09/16/2026 13:50:00\", \"1\", \"0\", \"0\", \r\n",
            "",
        );
        apply_table(&mut db, "MyTrade.cdb", &parse_cdb(&text).unwrap());
        let payloads = build_payloads(&db, &HashMap::new(), &HashSet::new());
        let sync = &payloads[0].sync;
        assert_eq!(sync.positions.len(), 1);
        assert_eq!(sync.positions[0].kind, 0);
        assert!((sync.positions[0].volume - 1.0).abs() < 1e-9);
        assert!((sync.positions[0].price_open - 29422.5).abs() < 1e-9);
    }
}
