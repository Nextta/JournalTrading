use std::collections::{HashMap, HashSet};

use base64::Engine;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use tauri::{AppHandle, State};

use crate::db;
use crate::error::{AppError, AppResult};
use crate::models::{
    Account, AccountInput, AccountUpdate, AddGroupResult, AssetGroup, AssetGroupInput,
    AssetGroupItemInput, Bootstrap, DayAsset, DayJournal, DayView, ExistingAssetAction, ImageInfo,
    ImageSaveInput, PlanItem, SettingsResponse, SettingsUpdate, Strategy, StrategyInput, Trade,
    TradeFilter, TradePage, TradeSeries,
};
use crate::services;
use crate::state::AppState;
use crate::stats;
use crate::util::{new_id, now_iso, sanitize_html, validate_date};

async fn settings_response(state: &AppState) -> AppResult<SettingsResponse> {
    let api_key = state.current_api_key().await;
    let api_port = db::get_api_port(&state.db).await?;
    let active_account_id = services::active_account_id(&state.db).await?;
    let timezone_mode = db::get_setting(&state.db, "timezone_mode")
        .await?
        .unwrap_or_else(|| "broker".to_string());
    Ok(SettingsResponse {
        api_key,
        api_port,
        active_account_id,
        timezone_mode,
        data_dir: state.data_dir.to_string_lossy().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        mt5: state.status().await,
    })
}

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_bootstrap(state: State<'_, AppState>) -> AppResult<Bootstrap> {
    let accounts = load_accounts(&state.db).await?;
    let strategies = services::load_strategies(&state.db, true).await?;
    let settings = settings_response(&state).await?;
    Ok(Bootstrap {
        accounts,
        settings,
        strategies,
    })
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

async fn load_accounts(pool: &SqlitePool) -> AppResult<Vec<Account>> {
    Ok(sqlx::query_as::<_, Account>(
        "SELECT id, login, server, display_name, company, currency, leverage, status,
                broker_utc_offset_seconds, balance, equity, last_heartbeat_at, created_at, updated_at
         FROM accounts
         ORDER BY CASE status WHEN 'active' THEN 0 WHEN 'pending' THEN 1 ELSE 2 END, created_at",
    )
    .fetch_all(pool)
    .await?)
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> AppResult<Vec<Account>> {
    load_accounts(&state.db).await
}

#[tauri::command]
pub async fn create_account(input: AccountInput, state: State<'_, AppState>) -> AppResult<Account> {
    let server = input.server.trim();
    let display_name = input.display_name.trim();
    let currency = input.currency.trim().to_uppercase();
    if input.login <= 0 {
        return Err(AppError::Validation(
            "El login debe ser un número positivo".into(),
        ));
    }
    if server.is_empty() {
        return Err(AppError::Validation("El servidor es obligatorio".into()));
    }
    if display_name.is_empty() {
        return Err(AppError::Validation("El nombre es obligatorio".into()));
    }
    if currency.len() != 3 {
        return Err(AppError::Validation("La divisa debe tener 3 letras".into()));
    }

    let existing = sqlx::query("SELECT id FROM accounts WHERE login = ? AND server = ?")
        .bind(input.login)
        .bind(server)
        .fetch_optional(&state.db)
        .await?;
    if existing.is_some() {
        return Err(AppError::Validation("Esa cuenta ya existe".into()));
    }

    let id = new_id();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO accounts
         (id, login, server, display_name, company, currency, leverage, status,
          broker_utc_offset_seconds, balance, equity, created_at, updated_at)
         VALUES (?, ?, ?, ?, '', ?, NULL, 'active', 0, 0, 0, ?, ?)",
    )
    .bind(&id)
    .bind(input.login)
    .bind(server)
    .bind(display_name)
    .bind(&currency)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await?;

    if services::active_account_id(&state.db).await?.is_none() {
        db::set_setting(&state.db, "active_account_id", &id).await?;
    }

    services::load_account(&state.db, &id).await
}

#[tauri::command]
pub async fn update_account(
    id: String,
    input: AccountUpdate,
    state: State<'_, AppState>,
) -> AppResult<Account> {
    let display_name = input.display_name.trim();
    let currency = input.currency.trim().to_uppercase();
    if display_name.is_empty() || currency.len() != 3 {
        return Err(AppError::Validation(
            "Nombre y divisa son obligatorios".into(),
        ));
    }
    sqlx::query("UPDATE accounts SET display_name = ?, currency = ?, updated_at = ? WHERE id = ?")
        .bind(display_name)
        .bind(&currency)
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;
    services::load_account(&state.db, &id).await
}

#[tauri::command]
pub async fn confirm_account(id: String, state: State<'_, AppState>) -> AppResult<Account> {
    sqlx::query("UPDATE accounts SET status = 'active', updated_at = ? WHERE id = ?")
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;

    let active = services::active_account_id(&state.db).await?;
    if active.is_none() {
        db::set_setting(&state.db, "active_account_id", &id).await?;
    }
    services::load_account(&state.db, &id).await
}

#[tauri::command]
pub async fn archive_account(id: String, state: State<'_, AppState>) -> AppResult<()> {
    sqlx::query("UPDATE accounts SET status = 'archived', updated_at = ? WHERE id = ?")
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;

    if db::get_setting(&state.db, "active_account_id")
        .await?
        .as_deref()
        == Some(&id)
    {
        let next = sqlx::query(
            "SELECT id FROM accounts WHERE status = 'active' ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&state.db)
        .await?;
        let next_id = next
            .map(|row| row.get::<String, _>("id"))
            .unwrap_or_default();
        db::set_setting(&state.db, "active_account_id", &next_id).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_active_account(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let account = services::load_account(&state.db, &id).await?;
    if account.status == "archived" {
        return Err(AppError::Validation("La cuenta está archivada".into()));
    }
    db::set_setting(&state.db, "active_account_id", &id).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_strategies(
    include_archived: Option<bool>,
    state: State<'_, AppState>,
) -> AppResult<Vec<Strategy>> {
    services::load_strategies(&state.db, include_archived.unwrap_or(false)).await
}

#[tauri::command]
pub async fn get_strategy(id: String, state: State<'_, AppState>) -> AppResult<Strategy> {
    services::load_strategy(&state.db, &id).await
}

async fn insert_strategy_items(
    conn: &mut sqlx::SqliteConnection,
    strategy_id: &str,
    items: &[crate::models::StrategyItemInput],
) -> AppResult<()> {
    let now = now_iso();
    for (index, item) in items.iter().enumerate() {
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO strategy_items
             (id, strategy_id, section, text, sort_order, required, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(strategy_id)
        .bind(item.section.trim())
        .bind(text)
        .bind(index as i64)
        .bind(item.required)
        .bind(&now)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn create_strategy(
    input: StrategyInput,
    state: State<'_, AppState>,
) -> AppResult<Strategy> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "El nombre de la estrategia es obligatorio".into(),
        ));
    }
    let id = new_id();
    let now = now_iso();
    let sort_order: i64 =
        sqlx::query("SELECT COALESCE(MAX(sort_order), -1) + 1 AS next FROM strategies")
            .fetch_one(&state.db)
            .await?
            .get("next");

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO strategies
         (id, name, description, color, archived, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, 0, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(input.description.trim())
    .bind(&input.color)
    .bind(sort_order)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    insert_strategy_items(&mut tx, &id, &input.items).await?;
    tx.commit().await?;
    services::load_strategy(&state.db, &id).await
}

#[tauri::command]
pub async fn update_strategy(
    id: String,
    input: StrategyInput,
    state: State<'_, AppState>,
) -> AppResult<Strategy> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "El nombre de la estrategia es obligatorio".into(),
        ));
    }
    services::load_strategy(&state.db, &id).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE strategies SET name = ?, description = ?, color = ?, updated_at = ? WHERE id = ?",
    )
    .bind(name)
    .bind(input.description.trim())
    .bind(&input.color)
    .bind(now_iso())
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM strategy_items WHERE strategy_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    insert_strategy_items(&mut tx, &id, &input.items).await?;
    tx.commit().await?;
    services::load_strategy(&state.db, &id).await
}

#[tauri::command]
pub async fn archive_strategy(id: String, state: State<'_, AppState>) -> AppResult<()> {
    sqlx::query("UPDATE strategies SET archived = 1, updated_at = ? WHERE id = ?")
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn duplicate_strategy(id: String, state: State<'_, AppState>) -> AppResult<Strategy> {
    let original = services::load_strategy(&state.db, &id).await?;
    let input = StrategyInput {
        name: format!("{} (copia)", original.name),
        description: original.description,
        color: original.color,
        items: original
            .items
            .into_iter()
            .map(|item| crate::models::StrategyItemInput {
                id: None,
                section: item.section,
                text: item.text,
                required: item.required,
            })
            .collect(),
    };
    create_strategy(input, state).await
}

// ---------------------------------------------------------------------------
// Asset groups
// ---------------------------------------------------------------------------

async fn normalize_asset_group_items(
    pool: &SqlitePool,
    items: &[AssetGroupItemInput],
) -> AppResult<Vec<(String, Option<String>, bool)>> {
    let mut normalized = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();

    for item in items {
        let symbol = item.symbol.trim().to_uppercase();
        if symbol.is_empty() {
            continue;
        }
        if !seen.insert(symbol.clone()) {
            return Err(AppError::Validation(format!(
                "El activo {symbol} está repetido en el grupo"
            )));
        }
        let strategy_id = if item.strategy_overridden {
            item.strategy_id.clone()
        } else {
            None
        };
        if let Some(strategy_id) = strategy_id.as_deref() {
            services::load_strategy(pool, strategy_id).await?;
        }
        normalized.push((symbol, strategy_id, item.strategy_overridden));
    }

    if normalized.is_empty() {
        return Err(AppError::Validation(
            "Añade al menos un activo al grupo".into(),
        ));
    }
    Ok(normalized)
}

async fn validate_default_group_strategy(
    pool: &SqlitePool,
    strategy_id: Option<&str>,
) -> AppResult<()> {
    if let Some(strategy_id) = strategy_id {
        services::load_strategy(pool, strategy_id).await?;
    }
    Ok(())
}

async fn insert_asset_group_items(
    conn: &mut sqlx::SqliteConnection,
    group_id: &str,
    items: &[(String, Option<String>, bool)],
) -> AppResult<()> {
    let now = now_iso();
    for (index, (symbol, strategy_id, strategy_overridden)) in items.iter().enumerate() {
        sqlx::query(
            "INSERT INTO asset_group_items
             (id, group_id, symbol, strategy_id, strategy_overridden, sort_order, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(group_id)
        .bind(symbol)
        .bind(strategy_id.as_deref())
        .bind(strategy_overridden)
        .bind(index as i64)
        .bind(&now)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_asset_groups(state: State<'_, AppState>) -> AppResult<Vec<AssetGroup>> {
    services::load_asset_groups(&state.db).await
}

#[tauri::command]
pub async fn get_asset_group(id: String, state: State<'_, AppState>) -> AppResult<AssetGroup> {
    services::load_asset_group(&state.db, &id).await
}

#[tauri::command]
pub async fn create_asset_group(
    input: AssetGroupInput,
    state: State<'_, AppState>,
) -> AppResult<AssetGroup> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "El nombre del grupo es obligatorio".into(),
        ));
    }
    validate_default_group_strategy(&state.db, input.default_strategy_id.as_deref()).await?;
    let items = normalize_asset_group_items(&state.db, &input.items).await?;
    let color = if input.color.trim().is_empty() {
        "#6d7cff".to_string()
    } else {
        input.color.trim().to_string()
    };

    let id = new_id();
    let now = now_iso();
    let sort_order: i64 =
        sqlx::query("SELECT COALESCE(MAX(sort_order), -1) + 1 AS next FROM asset_groups")
            .fetch_one(&state.db)
            .await?
            .get("next");

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO asset_groups
         (id, name, description, color, default_strategy_id, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(input.description.trim())
    .bind(&color)
    .bind(input.default_strategy_id.as_deref())
    .bind(sort_order)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    insert_asset_group_items(&mut tx, &id, &items).await?;
    tx.commit().await?;

    services::load_asset_group(&state.db, &id).await
}

#[tauri::command]
pub async fn update_asset_group(
    id: String,
    input: AssetGroupInput,
    state: State<'_, AppState>,
) -> AppResult<AssetGroup> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "El nombre del grupo es obligatorio".into(),
        ));
    }
    services::load_asset_group(&state.db, &id).await?;
    validate_default_group_strategy(&state.db, input.default_strategy_id.as_deref()).await?;
    let items = normalize_asset_group_items(&state.db, &input.items).await?;
    let color = if input.color.trim().is_empty() {
        "#6d7cff".to_string()
    } else {
        input.color.trim().to_string()
    };

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE asset_groups
         SET name = ?, description = ?, color = ?, default_strategy_id = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(name)
    .bind(input.description.trim())
    .bind(&color)
    .bind(input.default_strategy_id.as_deref())
    .bind(now_iso())
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM asset_group_items WHERE group_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    insert_asset_group_items(&mut tx, &id, &items).await?;
    tx.commit().await?;

    services::load_asset_group(&state.db, &id).await
}

#[tauri::command]
pub async fn delete_asset_group(id: String, state: State<'_, AppState>) -> AppResult<()> {
    services::load_asset_group(&state.db, &id).await?;
    sqlx::query("DELETE FROM asset_groups WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn add_group_to_day(
    account_id: String,
    date: String,
    group_id: String,
    existing_action: ExistingAssetAction,
    apply_to_trades: bool,
    state: State<'_, AppState>,
) -> AppResult<AddGroupResult> {
    validate_date(&date)?;
    services::add_asset_group_to_day(
        &state.db,
        &account_id,
        &date,
        &group_id,
        existing_action,
        apply_to_trades,
    )
    .await
}

// ---------------------------------------------------------------------------
// Day journal, assets and checklists
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_day(
    account_id: String,
    date: String,
    state: State<'_, AppState>,
) -> AppResult<DayView> {
    validate_date(&date)?;
    services::load_day_view(&state.db, &state.data_dir, &account_id, &date).await
}

#[tauri::command]
pub async fn create_day_asset(
    account_id: String,
    date: String,
    symbol: String,
    strategy_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<DayAsset> {
    validate_date(&date)?;
    let symbol = symbol.trim().to_uppercase();
    if symbol.is_empty() {
        return Err(AppError::Validation("El activo es obligatorio".into()));
    }
    if let Some(id) = &strategy_id {
        services::load_strategy(&state.db, id).await?;
    }
    let journal = services::ensure_day_journal(&state.db, &account_id, &date).await?;
    services::ensure_day_asset(&state.db, &journal.id, &symbol, strategy_id.as_deref()).await
}

#[tauri::command]
pub async fn delete_day_asset(asset_id: String, state: State<'_, AppState>) -> AppResult<()> {
    sqlx::query("UPDATE trades SET day_asset_id = NULL WHERE day_asset_id = ?")
        .bind(&asset_id)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM day_assets WHERE id = ?")
        .bind(&asset_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn select_asset_strategy(
    asset_id: String,
    strategy_id: Option<String>,
    apply_to_trades: bool,
    state: State<'_, AppState>,
) -> AppResult<DayAsset> {
    if let Some(id) = &strategy_id {
        services::load_strategy(&state.db, id).await?;
    }
    let asset = services::load_day_asset(&state.db, &asset_id).await?;
    sqlx::query("UPDATE day_assets SET strategy_id = ?, updated_at = ? WHERE id = ?")
        .bind(strategy_id.as_deref())
        .bind(now_iso())
        .bind(&asset_id)
        .execute(&state.db)
        .await?;

    if let Some(strategy_id) = &strategy_id {
        services::snapshot_strategy_items(&state.db, &asset_id, strategy_id).await?;
    } else {
        sqlx::query("DELETE FROM plan_items WHERE day_asset_id = ?")
            .bind(&asset_id)
            .execute(&state.db)
            .await?;
    }

    if apply_to_trades {
        sqlx::query("UPDATE trades SET strategy_id = ?, updated_at = ? WHERE day_asset_id = ?")
            .bind(strategy_id.as_deref())
            .bind(now_iso())
            .bind(&asset.id)
            .execute(&state.db)
            .await?;
    }

    let _ = asset;
    services::load_day_asset(&state.db, &asset_id).await
}

#[tauri::command]
pub async fn toggle_plan_item(
    item_id: String,
    checked: bool,
    state: State<'_, AppState>,
) -> AppResult<PlanItem> {
    let checked_at = if checked { Some(now_iso()) } else { None };
    sqlx::query("UPDATE plan_items SET checked = ?, checked_at = ? WHERE id = ?")
        .bind(checked)
        .bind(&checked_at)
        .bind(&item_id)
        .execute(&state.db)
        .await?;
    sqlx::query_as::<_, PlanItem>(
        "SELECT id, day_asset_id, template_item_id, section, text, sort_order, required,
                checked, checked_at FROM plan_items WHERE id = ?",
    )
    .bind(&item_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Punto del checklist no encontrado".into()))
}

#[tauri::command]
pub async fn add_plan_item(
    asset_id: String,
    section: String,
    text: String,
    required: bool,
    state: State<'_, AppState>,
) -> AppResult<PlanItem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Validation("El texto es obligatorio".into()));
    }
    let next: i64 = sqlx::query(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 AS next FROM plan_items WHERE day_asset_id = ?",
    )
    .bind(&asset_id)
    .fetch_one(&state.db)
    .await?
    .get("next");
    let id = new_id();
    sqlx::query(
        "INSERT INTO plan_items
         (id, day_asset_id, template_item_id, section, text, sort_order, required, checked, checked_at)
         VALUES (?, ?, NULL, ?, ?, ?, ?, 0, NULL)",
    )
    .bind(&id)
    .bind(&asset_id)
    .bind(section.trim())
    .bind(text)
    .bind(next)
    .bind(required)
    .execute(&state.db)
    .await?;
    sqlx::query_as::<_, PlanItem>(
        "SELECT id, day_asset_id, template_item_id, section, text, sort_order, required,
                checked, checked_at FROM plan_items WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::from)
}

#[tauri::command]
pub async fn delete_plan_item(item_id: String, state: State<'_, AppState>) -> AppResult<()> {
    sqlx::query("DELETE FROM plan_items WHERE id = ?")
        .bind(&item_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn save_day_journal(
    journal_id: String,
    html: String,
    state: State<'_, AppState>,
) -> AppResult<DayJournal> {
    sqlx::query("UPDATE day_journals SET general_html = ?, updated_at = ? WHERE id = ?")
        .bind(sanitize_html(&html))
        .bind(now_iso())
        .bind(&journal_id)
        .execute(&state.db)
        .await?;
    sqlx::query_as::<_, DayJournal>(
        "SELECT id, account_id, date, general_html, created_at, updated_at
         FROM day_journals WHERE id = ?",
    )
    .bind(&journal_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Diario no encontrado".into()))
}

#[tauri::command]
pub async fn save_asset_journal(
    asset_id: String,
    html: String,
    state: State<'_, AppState>,
) -> AppResult<DayAsset> {
    sqlx::query("UPDATE day_assets SET journal_html = ?, updated_at = ? WHERE id = ?")
        .bind(sanitize_html(&html))
        .bind(now_iso())
        .bind(&asset_id)
        .execute(&state.db)
        .await?;
    services::load_day_asset(&state.db, &asset_id).await
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

fn image_format(bytes: &[u8]) -> AppResult<(image::ImageFormat, &'static str, &'static str)> {
    let format = image::guess_format(bytes)
        .map_err(|_| AppError::Validation("El archivo no es una imagen válida".into()))?;
    match format {
        image::ImageFormat::Png => Ok((format, "png", "image/png")),
        image::ImageFormat::Jpeg => Ok((format, "jpg", "image/jpeg")),
        image::ImageFormat::WebP => Ok((format, "webp", "image/webp")),
        image::ImageFormat::Gif => Ok((format, "gif", "image/gif")),
        _ => Err(AppError::Validation(
            "Formato no soportado. Usa PNG, JPG, WEBP o GIF.".into(),
        )),
    }
}

async fn validate_image_owner(
    pool: &sqlx::SqlitePool,
    owner_type: &str,
    owner_id: &str,
) -> AppResult<&'static str> {
    let (column, table) = match owner_type {
        "day_journal" => ("id", "day_journals"),
        "day_asset" => ("id", "day_assets"),
        "trade" => ("id", "trades"),
        _ => return Err(AppError::Validation("Tipo de propietario no válido".into())),
    };
    let query = format!("SELECT 1 FROM {table} WHERE {column} = ?");
    let exists = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(
            "El propietario de la imagen no existe".into(),
        ));
    }
    Ok(match owner_type {
        "day_journal" => "day_journal_id",
        "day_asset" => "day_asset_id",
        _ => "trade_id",
    })
}

#[tauri::command]
pub async fn save_image(input: ImageSaveInput, state: State<'_, AppState>) -> AppResult<ImageInfo> {
    let owner_column = validate_image_owner(&state.db, &input.owner_type, &input.owner_id).await?;
    let payload = input
        .data_url
        .split_once(',')
        .map(|(_, payload)| payload)
        .ok_or_else(|| AppError::Validation("Imagen inválida".into()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|_| AppError::Validation("No se pudo decodificar la imagen".into()))?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(AppError::Validation(
            "La imagen supera el límite de 20 MB".into(),
        ));
    }
    let (format, extension, mime) = image_format(&bytes)?;
    let decoded = image::load_from_memory_with_format(&bytes, format)
        .map_err(|_| AppError::Validation("No se pudo leer la imagen".into()))?;
    let width = decoded.width() as i64;
    let height = decoded.height() as i64;

    let id = new_id();
    let file_name = format!("{id}.{extension}");
    let relative_path = format!("media/{file_name}");
    let absolute_path = state.data_dir.join(&relative_path);
    tokio::fs::write(&absolute_path, &bytes).await?;

    let thumb = decoded.thumbnail(720, 540);
    let thumb_name = format!("{id}.png");
    let thumb_relative = format!("media/thumbs/{thumb_name}");
    let thumb_absolute = state.data_dir.join(&thumb_relative);
    thumb
        .save_with_format(&thumb_absolute, image::ImageFormat::Png)
        .map_err(|error| AppError::Internal(error.to_string()))?;

    let sort_order: i64 = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 AS next FROM images WHERE {owner_column} = ?"
    )))
    .bind(&input.owner_id)
    .fetch_one(&state.db)
    .await?
    .get("next");

    let now = now_iso();
    sqlx::query(
        "INSERT INTO images
         (id, day_journal_id, day_asset_id, trade_id, file_name, relative_path, thumb_path,
          mime, width, height, bytes, caption, sort_order, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(if input.owner_type == "day_journal" {
        Some(input.owner_id.as_str())
    } else {
        None
    })
    .bind(if input.owner_type == "day_asset" {
        Some(input.owner_id.as_str())
    } else {
        None
    })
    .bind(if input.owner_type == "trade" {
        Some(input.owner_id.as_str())
    } else {
        None
    })
    .bind(&input.file_name)
    .bind(&relative_path)
    .bind(&thumb_relative)
    .bind(mime)
    .bind(width)
    .bind(height)
    .bind(bytes.len() as i64)
    .bind(input.caption.trim())
    .bind(sort_order)
    .bind(&now)
    .execute(&state.db)
    .await?;

    let mut image = sqlx::query_as::<_, ImageInfo>(
        "SELECT id, day_journal_id, day_asset_id, trade_id, file_name, relative_path,
                thumb_path, mime, width, height, bytes, caption, sort_order, created_at
         FROM images WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    image.path = absolute_path.to_string_lossy().to_string();
    image.thumb_url = Some(thumb_absolute.to_string_lossy().to_string());
    Ok(image)
}

#[tauri::command]
pub async fn list_images(
    owner_type: String,
    owner_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<ImageInfo>> {
    services::load_images(&state.db, &state.data_dir, &owner_type, &owner_id).await
}

#[tauri::command]
pub async fn delete_image(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let row = sqlx::query("SELECT relative_path, thumb_path FROM images WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?;
    if let Some(row) = row {
        let relative: String = row.get("relative_path");
        let thumb: Option<String> = row.get("thumb_path");
        let _ = tokio::fs::remove_file(state.data_dir.join(relative)).await;
        if let Some(thumb) = thumb {
            let _ = tokio::fs::remove_file(state.data_dir.join(thumb)).await;
        }
    }
    sqlx::query("DELETE FROM images WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Trades
// ---------------------------------------------------------------------------

fn push_trade_filters(builder: &mut QueryBuilder<Sqlite>, filter: &TradeFilter) {
    builder.push(" WHERE account_id = ");
    builder.push_bind(&filter.account_id);
    if let Some(state) = &filter.state
        && (state == "open" || state == "closed")
    {
        builder.push(" AND state = ");
        builder.push_bind(state);
    }
    if let Some(from) = &filter.from {
        builder.push(" AND substr(COALESCE(close_time_server, open_time_server), 1, 10) >= ");
        builder.push_bind(from);
    }
    if let Some(to) = &filter.to {
        builder.push(" AND substr(COALESCE(close_time_server, open_time_server), 1, 10) <= ");
        builder.push_bind(to);
    }
    if !filter.symbols.is_empty() {
        builder.push(" AND symbol IN (");
        let mut separated = builder.separated(", ");
        for symbol in &filter.symbols {
            separated.push_bind(symbol);
        }
        separated.push_unseparated(")");
    }
    if !filter.strategy_ids.is_empty() {
        builder.push(" AND strategy_id IN (");
        let mut separated = builder.separated(", ");
        for strategy_id in &filter.strategy_ids {
            separated.push_bind(strategy_id);
        }
        separated.push_unseparated(")");
    }
    if let Some(search) = &filter.search
        && !search.trim().is_empty()
    {
        let pattern = format!("%{}%", search.trim());
        builder.push(" AND (symbol LIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" OR comment LIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" OR notes LIKE ");
        builder.push_bind(pattern);
        builder.push(")");
    }
}

#[tauri::command]
pub async fn list_trades(filter: TradeFilter, state: State<'_, AppState>) -> AppResult<TradePage> {
    let limit = filter.limit.unwrap_or(100).clamp(1, 500);
    let offset = filter.offset.unwrap_or(0).max(0);

    let mut count_builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS count FROM trades");
    push_trade_filters(&mut count_builder, &filter);
    let total: i64 = count_builder
        .build_query_scalar()
        .fetch_one(&state.db)
        .await?;

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
                volume_current, open_time_server, open_time_utc, close_time_server,
                close_time_utc, open_price, close_price, sl, tp, profit_gross, commission,
                swap, profit_net, magic, comment, strategy_id, risk_amount, r_multiple,
                notes, state, created_at, updated_at FROM trades",
    );
    push_trade_filters(&mut builder, &filter);
    builder.push(
        " ORDER BY CASE state WHEN 'open' THEN 0 ELSE 1 END,
          COALESCE(close_time_utc, open_time_utc) DESC, position_ticket DESC LIMIT ",
    );
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    let mut items = builder
        .build_query_as::<Trade>()
        .fetch_all(&state.db)
        .await?;
    let strategy_rows = sqlx::query("SELECT id, name FROM strategies")
        .fetch_all(&state.db)
        .await?;
    let strategy_names: HashMap<String, String> = strategy_rows
        .into_iter()
        .map(|row| (row.get::<String, _>("id"), row.get::<String, _>("name")))
        .collect();
    for trade in &mut items {
        if let Some(strategy_id) = &trade.strategy_id {
            trade.strategy_name = strategy_names.get(strategy_id).cloned();
        }
        trade.series_timeframes = Vec::new();
    }
    Ok(TradePage { items, total })
}

#[tauri::command]
pub async fn get_trade(id: String, state: State<'_, AppState>) -> AppResult<Trade> {
    services::load_trade(&state.db, &state.data_dir, &id).await
}

#[tauri::command]
pub async fn update_trade_notes(
    id: String,
    notes: String,
    state: State<'_, AppState>,
) -> AppResult<Trade> {
    sqlx::query("UPDATE trades SET notes = ?, updated_at = ? WHERE id = ?")
        .bind(&notes)
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;
    services::load_trade(&state.db, &state.data_dir, &id).await
}

#[tauri::command]
pub async fn assign_trade_strategy(
    id: String,
    strategy_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Trade> {
    if let Some(strategy_id) = &strategy_id {
        services::load_strategy(&state.db, strategy_id).await?;
    }
    sqlx::query("UPDATE trades SET strategy_id = ?, updated_at = ? WHERE id = ?")
        .bind(strategy_id.as_deref())
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;
    services::load_trade(&state.db, &state.data_dir, &id).await
}

#[tauri::command]
pub async fn assign_trade_to_asset(
    id: String,
    day_asset_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Trade> {
    if let Some(asset_id) = &day_asset_id {
        services::load_day_asset(&state.db, asset_id).await?;
    }
    sqlx::query("UPDATE trades SET day_asset_id = ?, updated_at = ? WHERE id = ?")
        .bind(day_asset_id.as_deref())
        .bind(now_iso())
        .bind(&id)
        .execute(&state.db)
        .await?;
    services::load_trade(&state.db, &state.data_dir, &id).await
}

#[tauri::command]
pub async fn get_trade_series(
    trade_id: String,
    timeframe: String,
    state: State<'_, AppState>,
) -> AppResult<Option<TradeSeries>> {
    let timeframe = timeframe.to_uppercase();
    if !crate::models::TIMEFRAMES.contains(&timeframe.as_str()) {
        return Err(AppError::Validation("Timeframe no válido".into()));
    }
    let row = sqlx::query(
        "SELECT from_time, to_time, bar_count, bars_json, fetched_at
         FROM trade_series WHERE trade_id = ? AND timeframe = ?",
    )
    .bind(&trade_id)
    .bind(&timeframe)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let bars_json: String = row.get("bars_json");
    let bars =
        serde_json::from_str(&bars_json).map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(Some(TradeSeries {
        trade_id,
        timeframe,
        from_time: row.get("from_time"),
        to_time: row.get("to_time"),
        bar_count: row.get("bar_count"),
        bars,
        fetched_at: row.get("fetched_at"),
    }))
}

#[tauri::command]
pub async fn request_trade_series(
    trade_id: String,
    timeframe: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let timeframe = timeframe.to_uppercase();
    if !crate::models::TIMEFRAMES.contains(&timeframe.as_str()) {
        return Err(AppError::Validation("Timeframe no válido".into()));
    }
    let trade = services::load_trade(&state.db, &state.data_dir, &trade_id).await?;
    let from_time = crate::util::parse_server_epoch(&trade.open_time_server).unwrap_or(0);
    let to_time = trade
        .close_time_server
        .as_deref()
        .and_then(crate::util::parse_server_epoch)
        .unwrap_or(0);
    let payload = serde_json::json!({
        "trade_ticket": trade.position_ticket,
        "symbol": trade.symbol,
        "timeframe": timeframe,
        "from_time": from_time,
        "to_time": to_time,
    });
    let id = new_id();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO mt5_commands
         (id, account_id, kind, payload_json, status, created_at, updated_at)
         VALUES (?, ?, 'series', ?, 'pending', ?, ?)",
    )
    .bind(&id)
    .bind(&trade.account_id)
    .bind(payload.to_string())
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await?;
    Ok(id)
}

#[tauri::command]
pub async fn request_sync(account_id: String, state: State<'_, AppState>) -> AppResult<String> {
    services::load_account(&state.db, &account_id).await?;
    let id = new_id();
    let now = now_iso();
    let payload = serde_json::json!({ "from_days": 90 });
    sqlx::query(
        "INSERT INTO mt5_commands
         (id, account_id, kind, payload_json, status, created_at, updated_at)
         VALUES (?, ?, 'sync', ?, 'pending', ?, ?)",
    )
    .bind(&id)
    .bind(&account_id)
    .bind(payload.to_string())
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// Statistics and settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_stats(
    filter: crate::models::StatsFilter,
    state: State<'_, AppState>,
) -> AppResult<crate::models::StatsResponse> {
    stats::compute_stats(&state.db, &filter).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> AppResult<SettingsResponse> {
    settings_response(&state).await
}

#[tauri::command]
pub async fn update_settings(
    input: SettingsUpdate,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<SettingsResponse> {
    if input.api_port < 1024 {
        return Err(AppError::Validation(
            "Usa un puerto local entre 1024 y 65535".into(),
        ));
    }
    if input.timezone_mode != "broker" && input.timezone_mode != "local" {
        return Err(AppError::Validation("Zona horaria no válida".into()));
    }
    let old_port = db::get_api_port(&state.db).await?;
    db::set_setting(&state.db, "api_port", &input.api_port.to_string()).await?;
    db::set_setting(&state.db, "timezone_mode", &input.timezone_mode).await?;
    if old_port != input.api_port {
        state.start_server(app).await?;
    }
    settings_response(&state).await
}

#[tauri::command]
pub async fn regenerate_api_key(state: State<'_, AppState>) -> AppResult<String> {
    let key = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    db::set_setting(&state.db, "api_key", &key).await?;
    state.set_api_key(key.clone()).await;
    Ok(key)
}

#[tauri::command]
pub async fn get_mt5_status(state: State<'_, AppState>) -> AppResult<crate::models::Mt5Status> {
    Ok(state.status().await)
}

#[tauri::command]
pub async fn restart_mt5_server(
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<crate::models::Mt5Status> {
    state.restart_server(app).await
}

#[tauri::command]
pub async fn open_data_dir(state: State<'_, AppState>) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&state.data_dir)
            .spawn()
            .map_err(AppError::from)?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = &state;
    }
    Ok(())
}

#[tauri::command]
pub async fn create_backup(state: State<'_, AppState>) -> AppResult<String> {
    let backups_dir = state.data_dir.join("backups");
    tokio::fs::create_dir_all(&backups_dir).await?;
    let name = format!(
        "trading-journal-{}.db",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let path = backups_dir.join(name);
    let query = format!(
        "VACUUM INTO '{}'",
        path.to_string_lossy().replace('\'', "''")
    );
    sqlx::query(sqlx::AssertSqlSafe(query))
        .execute(&state.db)
        .await?;
    Ok(path.to_string_lossy().to_string())
}
