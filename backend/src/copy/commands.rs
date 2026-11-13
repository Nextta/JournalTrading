//! Comandos de Tauri para configurar y controlar el copy trading desde la app.

use std::collections::HashMap;

use serde::Deserialize;
use sqlx::Row;
use tauri::State;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::util::{new_id, now_iso};

use super::model::{
    CopyAccountView, CopyFollowerView, CopyGroupView, CopyLogEntry, CopyOverview,
    FollowerSettings, SizePreview, SymbolMapRow,
};
use super::{daily_pnl, emit_updated, flatten_follower, global_enabled, parse_settings, sizing, symbols, wake};

fn compatibility(master_provider: &str, follower_provider: &str) -> String {
    match (master_provider, follower_provider) {
        (a, b) if a == b => "compatible".into(),
        ("mt5", _) => "futures_only".into(),
        (_, "mt5") => "broker_futures".into(),
        _ => "compatible".into(),
    }
}

async fn ensure_follower_exists(state: &AppState, id: &str) -> AppResult<()> {
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM copy_followers WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    exists
        .map(|_| ())
        .ok_or_else(|| AppError::NotFound("Cuenta secundaria no encontrada".into()))
}

/// Secundarias de un grupo con el resumen de lo copiado. `0.0` y `CAST`: con `0` a secas SQLite
/// devuelve un entero cuando no hay nada copiado y no se puede leer como `f64`.
pub(crate) const FOLLOWER_ROWS_SQL: &str = "SELECT f.id, f.account_id, f.enabled, f.paused_reason, f.paused_at, f.settings_json,
        (SELECT COUNT(*) FROM copy_positions p WHERE p.follower_id = f.id) AS open_positions,
        (SELECT CAST(COALESCE(SUM(volume), 0.0) AS REAL) FROM copy_positions p WHERE p.follower_id = f.id) AS open_volume,
        (SELECT COUNT(*) FROM copy_pendings p WHERE p.follower_id = f.id) AS pending_orders
 FROM copy_followers f WHERE f.group_id = ? ORDER BY f.created_at ASC";

#[tauri::command]
pub async fn get_copy_overview(state: State<'_, AppState>) -> AppResult<CopyOverview> {
    let pool = &state.db;
    let enabled = global_enabled(pool).await?;
    let presence = state.copy.presence.lock().await;

    let master_ids: Vec<String> = sqlx::query_scalar("SELECT master_account_id FROM copy_groups")
        .fetch_all(pool)
        .await?;
    let follower_ids: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT account_id FROM copy_followers")
            .fetch_all(pool)
            .await?;

    let account_rows = sqlx::query(
        "SELECT id, display_name, login, server, provider, status, currency, balance, equity
         FROM accounts WHERE status != 'archived' ORDER BY display_name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    let mut providers = HashMap::new();
    let mut accounts = Vec::with_capacity(account_rows.len());
    for row in account_rows {
        let id: String = row.get("id");
        let provider: String = row.get("provider");
        providers.insert(id.clone(), provider.clone());
        let live = presence.get(&id);
        let role = if master_ids.contains(&id) {
            "master"
        } else if follower_ids.contains(&id) {
            "follower"
        } else {
            "none"
        };
        accounts.push(CopyAccountView {
            display_name: row.get("display_name"),
            login: row.get("login"),
            server: row.get("server"),
            provider,
            status: row.get("status"),
            currency: row.get("currency"),
            balance: live
                .and_then(|p| p.balance)
                .unwrap_or_else(|| row.get("balance")),
            equity: live
                .and_then(|p| p.equity)
                .unwrap_or_else(|| row.get("equity")),
            online: live.is_some_and(|p| p.online()),
            last_seen_at: live.and_then(|p| p.last_seen_iso.clone()),
            bridge_version: live.and_then(|p| p.bridge_version.clone()),
            role: role.into(),
            markets: live.map(|p| p.markets()).unwrap_or_default(),
            id,
        });
    }
    let equities: HashMap<String, f64> = accounts
        .iter()
        .map(|account| (account.id.clone(), account.equity))
        .collect();
    drop(presence);

    let group_rows = sqlx::query(
        "SELECT id, name, master_account_id, enabled, created_at FROM copy_groups
         ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    let mut groups = Vec::with_capacity(group_rows.len());
    for group in group_rows {
        let group_id: String = group.get("id");
        let master_account_id: String = group.get("master_account_id");
        let master_provider = providers
            .get(&master_account_id)
            .cloned()
            .unwrap_or_default();
        let follower_rows = sqlx::query(FOLLOWER_ROWS_SQL)
        .bind(&group_id)
        .fetch_all(pool)
        .await?;
        let mut followers = Vec::with_capacity(follower_rows.len());
        for row in follower_rows {
            let id: String = row.get("id");
            let account_id: String = row.get("account_id");
            let map = sqlx::query(
                "SELECT master_symbol, follower_symbol, volume_ratio FROM copy_symbol_map
                 WHERE follower_id = ? ORDER BY master_symbol",
            )
            .bind(&id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|map| SymbolMapRow {
                master_symbol: map.get("master_symbol"),
                follower_symbol: map.get("follower_symbol"),
                volume_ratio: map.get("volume_ratio"),
            })
            .collect();
            let follower_provider = providers.get(&account_id).cloned().unwrap_or_default();
            let equity = equities.get(&account_id).copied().unwrap_or(0.0);
            followers.push(CopyFollowerView {
                group_id: group_id.clone(),
                enabled: row.get::<i64, _>("enabled") != 0,
                paused_reason: row.get("paused_reason"),
                paused_at: row.get("paused_at"),
                settings: parse_settings(&row.get::<String, _>("settings_json")),
                symbol_map: map,
                open_positions: row.get("open_positions"),
                open_volume: row.get("open_volume"),
                pending_orders: row.get("pending_orders"),
                compatibility: compatibility(&master_provider, &follower_provider),
                daily_pnl: daily_pnl(pool, &account_id, equity).await?,
                account_id,
                id,
            });
        }
        groups.push(CopyGroupView {
            id: group_id,
            name: group.get("name"),
            master_account_id,
            enabled: group.get::<i64, _>("enabled") != 0,
            followers,
            created_at: group.get("created_at"),
        });
    }

    Ok(CopyOverview {
        enabled,
        groups,
        accounts,
    })
}

async fn validate_master(state: &AppState, master_account_id: &str, group_id: Option<&str>) -> AppResult<()> {
    let pool = &state.db;
    let exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM accounts WHERE id = ? AND status != 'archived'")
            .bind(master_account_id)
            .fetch_optional(pool)
            .await?;
    if exists.is_none() {
        return Err(AppError::NotFound("Cuenta principal no encontrada".into()));
    }
    let other: Option<String> = sqlx::query_scalar(
        "SELECT name FROM copy_groups WHERE master_account_id = ? AND id != ?",
    )
    .bind(master_account_id)
    .bind(group_id.unwrap_or(""))
    .fetch_optional(pool)
    .await?;
    if let Some(name) = other {
        return Err(AppError::Validation(format!(
            "La cuenta ya es la principal del grupo «{name}»"
        )));
    }
    let follower: Option<String> = sqlx::query_scalar(
        "SELECT g.name FROM copy_followers f JOIN copy_groups g ON g.id = f.group_id
         WHERE f.account_id = ? LIMIT 1",
    )
    .bind(master_account_id)
    .fetch_optional(pool)
    .await?;
    if let Some(name) = follower {
        return Err(AppError::Validation(format!(
            "La cuenta es secundaria en el grupo «{name}». Una cuenta no puede ser principal y secundaria a la vez."
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn create_copy_group(
    name: String,
    master_account_id: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("El grupo necesita un nombre".into()));
    }
    validate_master(&state, &master_account_id, None).await?;
    let id = new_id();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO copy_groups (id, name, master_account_id, enabled, created_at, updated_at)
         VALUES (?, ?, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(&master_account_id)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await?;
    emit_updated(&state);
    Ok(id)
}

#[tauri::command]
pub async fn update_copy_group(
    id: String,
    name: String,
    master_account_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("El grupo necesita un nombre".into()));
    }
    validate_master(&state, &master_account_id, Some(&id)).await?;
    let in_followers: Option<String> = sqlx::query_scalar(
        "SELECT id FROM copy_followers WHERE group_id = ? AND account_id = ?",
    )
    .bind(&id)
    .bind(&master_account_id)
    .fetch_optional(&state.db)
    .await?;
    if in_followers.is_some() {
        return Err(AppError::Validation(
            "La cuenta principal no puede ser también secundaria del grupo".into(),
        ));
    }
    sqlx::query("UPDATE copy_groups SET name = ?, master_account_id = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(&master_account_id)
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;
    emit_updated(&state);
    Ok(())
}

#[tauri::command]
pub async fn delete_copy_group(id: String, state: State<'_, AppState>) -> AppResult<()> {
    sqlx::query("DELETE FROM copy_groups WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    emit_updated(&state);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleOptions {
    #[serde(default)]
    pub cancel_pending: bool,
    #[serde(default)]
    pub close_positions: bool,
}

async fn flatten_followers(
    state: &AppState,
    follower_ids: Vec<String>,
    options: &ToggleOptions,
) -> AppResult<usize> {
    let mut total = 0;
    if options.cancel_pending || options.close_positions {
        for id in follower_ids {
            total +=
                flatten_follower(state, &id, options.close_positions, options.cancel_pending).await?;
        }
    }
    Ok(total)
}

#[tauri::command]
pub async fn set_copy_enabled(
    enabled: bool,
    options: Option<ToggleOptions>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    db::set_setting(&state.db, "copy_enabled", if enabled { "1" } else { "0" }).await?;
    if !enabled {
        if let Some(options) = options {
            let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM copy_followers")
                .fetch_all(&state.db)
                .await?;
            flatten_followers(&state, ids, &options).await?;
        }
    }
    wake(&state);
    emit_updated(&state);
    Ok(())
}

#[tauri::command]
pub async fn set_copy_group_enabled(
    id: String,
    enabled: bool,
    options: Option<ToggleOptions>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    sqlx::query("UPDATE copy_groups SET enabled = ?, updated_at = ? WHERE id = ?")
        .bind(i64::from(enabled))
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;
    if !enabled {
        if let Some(options) = options {
            let ids: Vec<String> =
                sqlx::query_scalar("SELECT id FROM copy_followers WHERE group_id = ?")
                    .bind(&id)
                    .fetch_all(&state.db)
                    .await?;
            flatten_followers(&state, ids, &options).await?;
        }
    }
    wake(&state);
    emit_updated(&state);
    Ok(())
}

#[tauri::command]
pub async fn add_copy_follower(
    group_id: String,
    account_id: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = &state.db;
    let Some(master) =
        sqlx::query_scalar::<_, String>("SELECT master_account_id FROM copy_groups WHERE id = ?")
            .bind(&group_id)
            .fetch_optional(pool)
            .await?
    else {
        return Err(AppError::NotFound("Grupo no encontrado".into()));
    };
    if master == account_id {
        return Err(AppError::Validation(
            "La cuenta principal no puede copiarse a sí misma".into(),
        ));
    }
    let is_master: Option<String> =
        sqlx::query_scalar("SELECT name FROM copy_groups WHERE master_account_id = ?")
            .bind(&account_id)
            .fetch_optional(pool)
            .await?;
    if let Some(name) = is_master {
        return Err(AppError::Validation(format!(
            "La cuenta es la principal del grupo «{name}». Una cuenta no puede ser principal y secundaria a la vez."
        )));
    }
    let id = new_id();
    let now = now_iso();
    let settings = serde_json::to_string(&FollowerSettings::default())
        .map_err(|error| AppError::Internal(error.to_string()))?;
    sqlx::query(
        "INSERT INTO copy_followers
         (id, group_id, account_id, enabled, settings_json, created_at, updated_at)
         VALUES (?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&group_id)
    .bind(&account_id)
    .bind(settings)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::Validation("La cuenta ya es secundaria de este grupo".into())
        }
        other => AppError::Db(other),
    })?;
    emit_updated(&state);
    Ok(id)
}

#[tauri::command]
pub async fn remove_copy_follower(id: String, state: State<'_, AppState>) -> AppResult<()> {
    sqlx::query("DELETE FROM copy_followers WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    emit_updated(&state);
    Ok(())
}

fn validate_settings(settings: &FollowerSettings) -> AppResult<()> {
    let sizing = &settings.sizing;
    let positive = |value: f64, label: &str| {
        if value.is_finite() && value > 0.0 {
            Ok(())
        } else {
            Err(AppError::Validation(format!("{label} debe ser mayor que 0")))
        }
    };
    positive(sizing.fixed_volume, "El tamaño fijo")?;
    positive(sizing.multiplier, "El multiplicador")?;
    positive(sizing.risk_percent, "El % de riesgo")?;
    positive(sizing.kelly_fraction, "La fracción de Kelly")?;
    positive(sizing.kelly_max_risk_percent, "El riesgo máximo de Kelly")?;
    positive(sizing.notional_multiplier, "El multiplicador de la fórmula")?;
    if sizing.risk_percent > 100.0 || sizing.kelly_max_risk_percent > 100.0 {
        return Err(AppError::Validation("El % de riesgo no puede superar el 100 %".into()));
    }
    if sizing.fallback_mode.cfd_only() {
        return Err(AppError::Validation(
            "El método de respaldo debe ser mismo tamaño, fijo o multiplicador".into(),
        ));
    }
    for (value, label) in [
        (settings.max_volume_per_trade, "El tamaño máximo por operación"),
        (settings.max_position_volume, "El tamaño máximo de posición"),
        (settings.daily_loss_limit, "La pérdida diaria máxima"),
    ] {
        if let Some(value) = value {
            positive(value, label)?;
        }
    }
    if settings.schedule_enabled {
        for value in [&settings.schedule_start, &settings.schedule_end] {
            chrono::NaiveTime::parse_from_str(value.trim(), "%H:%M")
                .map_err(|_| AppError::Validation("El horario debe tener formato HH:MM".into()))?;
        }
    }
    if settings.max_deviation < 0.0 {
        return Err(AppError::Validation("La desviación máxima no puede ser negativa".into()));
    }
    Ok(())
}

#[tauri::command]
pub async fn update_copy_follower(
    id: String,
    settings: FollowerSettings,
    symbol_map: Vec<SymbolMapRow>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    ensure_follower_exists(&state, &id).await?;
    validate_settings(&settings)?;
    let raw = serde_json::to_string(&settings).map_err(|error| AppError::Internal(error.to_string()))?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE copy_followers SET settings_json = ?, updated_at = ? WHERE id = ?")
        .bind(raw)
        .bind(now_iso())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM copy_symbol_map WHERE follower_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let mut seen = std::collections::HashSet::new();
    for row in symbol_map {
        let master = row.master_symbol.trim().to_string();
        let follower = row.follower_symbol.trim().to_string();
        if master.is_empty() || follower.is_empty() {
            continue;
        }
        if !seen.insert(master.to_ascii_uppercase()) {
            return Err(AppError::Validation(format!(
                "El símbolo {master} aparece dos veces en la tabla de equivalencias"
            )));
        }
        let ratio = row.volume_ratio.filter(|value| value.is_finite() && *value > 0.0);
        sqlx::query(
            "INSERT INTO copy_symbol_map (id, follower_id, master_symbol, follower_symbol, volume_ratio)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&id)
        .bind(&master)
        .bind(&follower)
        .bind(ratio)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    emit_updated(&state);
    Ok(())
}

#[tauri::command]
pub async fn set_copy_follower_enabled(
    id: String,
    enabled: bool,
    options: Option<ToggleOptions>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    ensure_follower_exists(&state, &id).await?;
    sqlx::query(
        "UPDATE copy_followers SET enabled = ?, paused_reason = CASE WHEN ? THEN NULL ELSE paused_reason END,
         paused_at = CASE WHEN ? THEN NULL ELSE paused_at END, updated_at = ? WHERE id = ?",
    )
    .bind(i64::from(enabled))
    .bind(enabled)
    .bind(enabled)
    .bind(now_iso())
    .bind(&id)
    .execute(&state.db)
    .await?;
    if !enabled {
        if let Some(options) = options {
            flatten_followers(&state, vec![id], &options).await?;
        }
    }
    wake(&state);
    emit_updated(&state);
    Ok(())
}

#[tauri::command]
pub async fn resume_copy_follower(id: String, state: State<'_, AppState>) -> AppResult<()> {
    ensure_follower_exists(&state, &id).await?;
    sqlx::query(
        "UPDATE copy_followers SET paused_reason = NULL, paused_at = NULL, enabled = 1, updated_at = ?
         WHERE id = ?",
    )
    .bind(now_iso())
    .bind(&id)
    .execute(&state.db)
    .await?;
    wake(&state);
    emit_updated(&state);
    Ok(())
}

/// Botón de pánico: cierra todo lo copiado, cancela las pendientes y pausa la copia.
#[tauri::command]
pub async fn copy_panic(group_id: Option<String>, state: State<'_, AppState>) -> AppResult<usize> {
    let ids: Vec<String> = match &group_id {
        Some(group) => {
            sqlx::query("UPDATE copy_groups SET enabled = 0, updated_at = ? WHERE id = ?")
                .bind(now_iso())
                .bind(group)
                .execute(&state.db)
                .await?;
            sqlx::query_scalar("SELECT id FROM copy_followers WHERE group_id = ?")
                .bind(group)
                .fetch_all(&state.db)
                .await?
        }
        None => {
            db::set_setting(&state.db, "copy_enabled", "0").await?;
            sqlx::query_scalar("SELECT id FROM copy_followers")
                .fetch_all(&state.db)
                .await?
        }
    };
    let options = ToggleOptions {
        cancel_pending: true,
        close_positions: true,
    };
    let total = flatten_followers(&state, ids, &options).await?;
    wake(&state);
    emit_updated(&state);
    Ok(total)
}

#[tauri::command]
pub async fn list_copy_log(
    group_id: Option<String>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> AppResult<Vec<CopyLogEntry>> {
    let limit = limit.unwrap_or(100).clamp(1, 1000);
    let rows = sqlx::query(
        "SELECT o.id, o.group_id, g.name AS group_name, o.follower_id, o.account_id,
                a.display_name AS account_name, s.kind AS signal_kind, o.action, o.symbol, o.side,
                o.volume, s.volume AS master_volume, s.price AS master_price, o.fill_price,
                o.fill_volume, o.status, o.error, o.latency_ms, o.created_at
         FROM copy_orders o
         JOIN copy_groups g ON g.id = o.group_id
         JOIN accounts a ON a.id = o.account_id
         LEFT JOIN copy_signals s ON s.id = o.signal_id
         WHERE (? IS NULL OR o.group_id = ?)
         ORDER BY o.created_ms DESC LIMIT ?",
    )
    .bind(&group_id)
    .bind(&group_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| CopyLogEntry {
            id: row.get("id"),
            group_id: row.get("group_id"),
            group_name: row.get("group_name"),
            follower_id: row.get("follower_id"),
            account_id: row.get("account_id"),
            account_name: row.get("account_name"),
            signal_kind: row.get("signal_kind"),
            action: row.get("action"),
            symbol: row.get("symbol"),
            side: row.get("side"),
            volume: row.get("volume"),
            master_volume: row.get("master_volume"),
            master_price: row.get("master_price"),
            fill_price: row.get("fill_price"),
            fill_volume: row.get("fill_volume"),
            status: row.get("status"),
            error: row.get("error"),
            latency_ms: row.get("latency_ms"),
            created_at: row.get("created_at"),
        })
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewInput {
    pub follower_id: String,
    pub settings: FollowerSettings,
    #[serde(default)]
    pub symbol_map: Vec<SymbolMapRow>,
    pub symbol: String,
    pub volume: f64,
    pub price: f64,
    #[serde(default)]
    pub stop_loss: Option<f64>,
}

/// Calcula, sin enviar nada, qué tamaño abriría la secundaria con una operación de ejemplo.
#[tauri::command]
pub async fn preview_copy_size(input: PreviewInput, state: State<'_, AppState>) -> AppResult<SizePreview> {
    let pool = &state.db;
    let Some(row) = sqlx::query(
        "SELECT f.account_id, f.group_id, a.provider, a.balance, a.equity, g.master_account_id,
                m.provider AS master_provider
         FROM copy_followers f JOIN accounts a ON a.id = f.account_id
         JOIN copy_groups g ON g.id = f.group_id JOIN accounts m ON m.id = g.master_account_id
         WHERE f.id = ?",
    )
    .bind(&input.follower_id)
    .fetch_optional(pool)
    .await?
    else {
        return Err(AppError::NotFound("Cuenta secundaria no encontrada".into()));
    };
    let account_id: String = row.get("account_id");
    let provider: String = row.get("provider");
    let master_provider: String = row.get("master_provider");
    let master_account_id: String = row.get("master_account_id");

    let (specs, master_specs, balance, equity) = {
        let presence = state.copy.presence.lock().await;
        let follower = presence.get(&account_id);
        (
            follower.map(|p| p.specs.clone()).unwrap_or_default(),
            presence
                .get(&master_account_id)
                .map(|p| p.specs.clone())
                .unwrap_or_default(),
            follower
                .and_then(|p| p.balance)
                .unwrap_or_else(|| row.get("balance")),
            follower
                .and_then(|p| p.equity)
                .unwrap_or_else(|| row.get("equity")),
        )
    };

    let market = master_specs
        .get(input.symbol.trim())
        .map(|spec| spec.market.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if master_provider != "mt5" || symbols::parse_futures(&input.symbol).is_some() {
                symbols::MARKET_FUTURES.to_string()
            } else {
                symbols::MARKET_CFD.to_string()
            }
        });
    if !symbols::compatible(&market, &master_provider, &provider) {
        return Ok(SizePreview {
            ok: false,
            volume: None,
            symbol: None,
            message: "Mercado incompatible: esta secundaria no puede operar ese instrumento".into(),
        });
    }
    let map: Vec<symbols::MapEntry<'_>> = input
        .symbol_map
        .iter()
        .map(|row| symbols::MapEntry {
            master_symbol: &row.master_symbol,
            follower_symbol: &row.follower_symbol,
            volume_ratio: row.volume_ratio,
        })
        .collect();
    let available: Vec<String> = specs.keys().cloned().collect();
    let Some(resolution) = symbols::resolve(
        &input.symbol,
        &market,
        &provider,
        &map,
        &available,
        input.settings.family_scaling,
    ) else {
        return Ok(SizePreview {
            ok: false,
            volume: None,
            symbol: None,
            message: "No se pudo emparejar el símbolo".into(),
        });
    };
    let spec = resolution
        .candidates
        .iter()
        .find_map(|candidate| specs.get(candidate));
    let is_futures = market == symbols::MARKET_FUTURES;
    let kelly = if input.settings.sizing.mode == super::model::SizingMode::Kelly {
        Some(super::kelly_stats(pool, &master_account_id, input.settings.sizing.kelly_lookback).await?)
    } else {
        None
    };
    let symbol = spec
        .map(|spec| spec.symbol.clone())
        .or_else(|| resolution.candidates.first().cloned());
    let raw = match sizing::compute(&sizing::SizingInput {
        params: &input.settings.sizing,
        is_futures,
        master_volume: input.volume,
        entry_price: input.price,
        stop_loss: input.stop_loss.filter(|v| *v > 0.0),
        balance,
        equity,
        volume_ratio: resolution.volume_ratio,
        kelly,
        spec,
    }) {
        Ok(value) => value,
        Err(reason) => {
            return Ok(SizePreview {
                ok: false,
                volume: None,
                symbol,
                message: reason,
            });
        }
    };
    let cap = match (
        input.settings.max_volume_per_trade.filter(|v| *v > 0.0),
        input.settings.max_position_volume.filter(|v| *v > 0.0),
    ) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    match sizing::normalize(raw.volume, is_futures, spec, cap) {
        Ok(volume) => {
            let mut message = if spec.is_none() && !is_futures {
                "Sin especificación del símbolo en la secundaria: se asume paso 0,01".to_string()
            } else {
                String::new()
            };
            if let Some(note) = raw.note {
                if !message.is_empty() {
                    message.push_str(". ");
                }
                message.push_str(&note);
            }
            Ok(SizePreview {
                ok: true,
                volume: Some(volume),
                symbol,
                message,
            })
        }
        Err(reason) => Ok(SizePreview {
            ok: false,
            volume: None,
            symbol,
            message: reason,
        }),
    }
}
