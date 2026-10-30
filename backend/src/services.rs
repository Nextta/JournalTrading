use std::collections::HashMap;
use std::path::Path;

use sqlx::{Row, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::{
    AddGroupResult, AssetDayStats, AssetGroup, AssetGroupItem, DayAsset, DayAssetView, DayJournal,
    DayView, Deal, ExistingAssetAction, ImageInfo, PlanItem, Strategy, StrategyItem, Trade,
};
use crate::util::{new_id, now_iso};

pub async fn load_account(pool: &SqlitePool, id: &str) -> AppResult<crate::models::Account> {
    sqlx::query_as::<_, crate::models::Account>(
        "SELECT id, login, server, display_name, company, currency, leverage, status,
                broker_utc_offset_seconds, balance, equity, last_heartbeat_at, created_at, updated_at
         FROM accounts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Cuenta no encontrada".into()))
}

pub async fn active_account_id(pool: &SqlitePool) -> AppResult<Option<String>> {
    let from_setting = crate::db::get_setting(pool, "active_account_id")
        .await?
        .filter(|value| !value.is_empty());
    if let Some(id) = from_setting {
        let exists = sqlx::query("SELECT 1 FROM accounts WHERE id = ? AND status != 'archived'")
            .bind(&id)
            .fetch_optional(pool)
            .await?;
        if exists.is_some() {
            return Ok(Some(id));
        }
    }

    let row = sqlx::query(
        "SELECT id FROM accounts WHERE status != 'archived'
         ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END, created_at LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| row.get::<String, _>("id")))
}

pub async fn load_strategy(pool: &SqlitePool, id: &str) -> AppResult<Strategy> {
    let mut strategy = sqlx::query_as::<_, Strategy>(
        "SELECT id, name, description, color, archived, sort_order, created_at, updated_at
         FROM strategies WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Estrategia no encontrada".into()))?;

    strategy.items = load_strategy_items(pool, &strategy.id).await?;
    Ok(strategy)
}

pub async fn load_strategy_items(
    pool: &SqlitePool,
    strategy_id: &str,
) -> AppResult<Vec<StrategyItem>> {
    Ok(sqlx::query_as::<_, StrategyItem>(
        "SELECT id, strategy_id, section, text, sort_order, required, created_at
         FROM strategy_items WHERE strategy_id = ? ORDER BY sort_order, created_at",
    )
    .bind(strategy_id)
    .fetch_all(pool)
    .await?)
}

pub async fn load_strategies(
    pool: &SqlitePool,
    include_archived: bool,
) -> AppResult<Vec<Strategy>> {
    let rows = if include_archived {
        sqlx::query_as::<_, Strategy>(
            "SELECT id, name, description, color, archived, sort_order, created_at, updated_at
             FROM strategies ORDER BY archived, sort_order, name",
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Strategy>(
            "SELECT id, name, description, color, archived, sort_order, created_at, updated_at
             FROM strategies WHERE archived = 0 ORDER BY sort_order, name",
        )
        .fetch_all(pool)
        .await?
    };

    let mut strategies = Vec::with_capacity(rows.len());
    for mut strategy in rows {
        strategy.items = load_strategy_items(pool, &strategy.id).await?;
        strategies.push(strategy);
    }
    Ok(strategies)
}

pub async fn load_asset_group(pool: &SqlitePool, id: &str) -> AppResult<AssetGroup> {
    let mut group = sqlx::query_as::<_, AssetGroup>(
        "SELECT id, name, description, color, default_strategy_id, sort_order, created_at,
                updated_at
         FROM asset_groups WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Grupo de activos no encontrado".into()))?;

    group.items = sqlx::query_as::<_, AssetGroupItem>(
        "SELECT id, group_id, symbol, strategy_id, strategy_overridden, sort_order, created_at
         FROM asset_group_items WHERE group_id = ? ORDER BY sort_order, symbol",
    )
    .bind(&group.id)
    .fetch_all(pool)
    .await?;
    Ok(group)
}

pub async fn load_asset_groups(pool: &SqlitePool) -> AppResult<Vec<AssetGroup>> {
    let mut groups = sqlx::query_as::<_, AssetGroup>(
        "SELECT id, name, description, color, default_strategy_id, sort_order, created_at,
                updated_at
         FROM asset_groups ORDER BY sort_order, name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;

    let items = sqlx::query_as::<_, AssetGroupItem>(
        "SELECT id, group_id, symbol, strategy_id, strategy_overridden, sort_order, created_at
         FROM asset_group_items ORDER BY group_id, sort_order, symbol",
    )
    .fetch_all(pool)
    .await?;

    let mut by_group: HashMap<String, Vec<AssetGroupItem>> = HashMap::new();
    for item in items {
        by_group
            .entry(item.group_id.clone())
            .or_default()
            .push(item);
    }
    for group in &mut groups {
        group.items = by_group.remove(&group.id).unwrap_or_default();
    }
    Ok(groups)
}

pub async fn add_asset_group_to_day(
    pool: &SqlitePool,
    account_id: &str,
    date: &str,
    group_id: &str,
    existing_action: ExistingAssetAction,
    apply_to_trades: bool,
) -> AppResult<AddGroupResult> {
    load_account(pool, account_id).await?;
    let group = load_asset_group(pool, group_id).await?;
    if group.items.is_empty() {
        return Err(AppError::Validation(
            "El grupo no tiene activos configurados".into(),
        ));
    }
    let journal = ensure_day_journal(pool, account_id, date).await?;
    let mut tx = pool.begin().await?;

    let existing_assets = sqlx::query_as::<_, DayAsset>(
        "SELECT id, journal_id, symbol, strategy_id, journal_html, created_at, updated_at
         FROM day_assets WHERE journal_id = ?",
    )
    .bind(&journal.id)
    .fetch_all(&mut *tx)
    .await?;

    let mut existing_by_symbol: HashMap<String, DayAsset> = HashMap::new();
    for asset in existing_assets {
        existing_by_symbol.insert(asset.symbol.clone(), asset);
    }

    let mut result = AddGroupResult::default();
    for item in &group.items {
        let symbol = item.symbol.clone();
        let effective_strategy_id = if item.strategy_overridden {
            item.strategy_id.as_deref()
        } else {
            group.default_strategy_id.as_deref()
        };

        if let Some(asset) = existing_by_symbol.get(&symbol) {
            if existing_action == ExistingAssetAction::Skip {
                result.skipped.push(symbol);
                continue;
            }

            let Some(strategy_id) = effective_strategy_id else {
                result.skipped.push(symbol);
                continue;
            };

            let strategy_changed = asset.strategy_id.as_deref() != Some(strategy_id);
            if strategy_changed {
                sqlx::query("UPDATE day_assets SET strategy_id = ?, updated_at = ? WHERE id = ?")
                    .bind(strategy_id)
                    .bind(now_iso())
                    .bind(&asset.id)
                    .execute(&mut *tx)
                    .await?;
                snapshot_strategy_items_conn(&mut tx, &asset.id, strategy_id).await?;
            }

            if apply_to_trades {
                sqlx::query(
                    "UPDATE trades SET strategy_id = ?, updated_at = ? WHERE day_asset_id = ?",
                )
                .bind(strategy_id)
                .bind(now_iso())
                .bind(&asset.id)
                .execute(&mut *tx)
                .await?;
            }

            if strategy_changed || apply_to_trades {
                result.updated.push(symbol);
            } else {
                result.skipped.push(symbol);
            }
            continue;
        }

        let id = new_id();
        let now = now_iso();
        sqlx::query(
            "INSERT INTO day_assets
             (id, journal_id, symbol, strategy_id, journal_html, created_at, updated_at)
             VALUES (?, ?, ?, ?, '', ?, ?)",
        )
        .bind(&id)
        .bind(&journal.id)
        .bind(&symbol)
        .bind(effective_strategy_id)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        if let Some(strategy_id) = effective_strategy_id {
            snapshot_strategy_items_conn(&mut tx, &id, strategy_id).await?;
        }
        result.added.push(symbol);
    }

    tx.commit().await?;
    Ok(result)
}

pub async fn ensure_day_journal(
    pool: &SqlitePool,
    account_id: &str,
    date: &str,
) -> AppResult<DayJournal> {
    if let Some(journal) = sqlx::query_as::<_, DayJournal>(
        "SELECT id, account_id, date, general_html, created_at, updated_at
         FROM day_journals WHERE account_id = ? AND date = ?",
    )
    .bind(account_id)
    .bind(date)
    .fetch_optional(pool)
    .await?
    {
        return Ok(journal);
    }

    let id = new_id();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO day_journals (id, account_id, date, general_html, created_at, updated_at)
         VALUES (?, ?, ?, '', ?, ?)",
    )
    .bind(&id)
    .bind(account_id)
    .bind(date)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, DayJournal>(
        "SELECT id, account_id, date, general_html, created_at, updated_at
         FROM day_journals WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(pool)
    .await
    .map_err(AppError::from)
}

pub async fn ensure_day_asset(
    pool: &SqlitePool,
    journal_id: &str,
    symbol: &str,
    strategy_id: Option<&str>,
) -> AppResult<DayAsset> {
    if let Some(asset) = sqlx::query_as::<_, DayAsset>(
        "SELECT id, journal_id, symbol, strategy_id, journal_html, created_at, updated_at
         FROM day_assets WHERE journal_id = ? AND symbol = ?",
    )
    .bind(journal_id)
    .bind(symbol)
    .fetch_optional(pool)
    .await?
    {
        if asset.strategy_id.is_none()
            && let Some(strategy_id) = strategy_id
        {
            sqlx::query("UPDATE day_assets SET strategy_id = ?, updated_at = ? WHERE id = ?")
                .bind(strategy_id)
                .bind(now_iso())
                .bind(&asset.id)
                .execute(pool)
                .await?;
            snapshot_strategy_items(pool, &asset.id, strategy_id).await?;
            return load_day_asset(pool, &asset.id).await;
        }
        return Ok(asset);
    }

    let id = new_id();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO day_assets
         (id, journal_id, symbol, strategy_id, journal_html, created_at, updated_at)
         VALUES (?, ?, ?, ?, '', ?, ?)",
    )
    .bind(&id)
    .bind(journal_id)
    .bind(symbol)
    .bind(strategy_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    if let Some(strategy_id) = strategy_id {
        snapshot_strategy_items(pool, &id, strategy_id).await?;
    }
    load_day_asset(pool, &id).await
}

pub async fn load_day_asset(pool: &SqlitePool, id: &str) -> AppResult<DayAsset> {
    sqlx::query_as::<_, DayAsset>(
        "SELECT id, journal_id, symbol, strategy_id, journal_html, created_at, updated_at
         FROM day_assets WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Activo del día no encontrado".into()))
}

pub async fn snapshot_strategy_items(
    pool: &SqlitePool,
    day_asset_id: &str,
    strategy_id: &str,
) -> AppResult<()> {
    let mut conn = pool.acquire().await?;
    snapshot_strategy_items_conn(&mut conn, day_asset_id, strategy_id).await
}

async fn snapshot_strategy_items_conn(
    conn: &mut sqlx::SqliteConnection,
    day_asset_id: &str,
    strategy_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM plan_items WHERE day_asset_id = ?")
        .bind(day_asset_id)
        .execute(&mut *conn)
        .await?;

    let items = sqlx::query_as::<_, StrategyItem>(
        "SELECT id, strategy_id, section, text, sort_order, required, created_at
         FROM strategy_items WHERE strategy_id = ? ORDER BY sort_order, created_at",
    )
    .bind(strategy_id)
    .fetch_all(&mut *conn)
    .await?;

    for item in items {
        sqlx::query(
            "INSERT INTO plan_items
             (id, day_asset_id, template_item_id, section, text, sort_order, required, checked, checked_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, NULL)",
        )
        .bind(new_id())
        .bind(day_asset_id)
        .bind(&item.id)
        .bind(&item.section)
        .bind(&item.text)
        .bind(item.sort_order)
        .bind(item.required)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub async fn load_plan_items(pool: &SqlitePool, asset_id: &str) -> AppResult<Vec<PlanItem>> {
    Ok(sqlx::query_as::<_, PlanItem>(
        "SELECT id, day_asset_id, template_item_id, section, text, sort_order, required,
                checked, checked_at
         FROM plan_items WHERE day_asset_id = ? ORDER BY sort_order, text",
    )
    .bind(asset_id)
    .fetch_all(pool)
    .await?)
}

pub async fn load_deals(pool: &SqlitePool, trade_id: &str) -> AppResult<Vec<Deal>> {
    Ok(sqlx::query_as::<_, Deal>(
        "SELECT id, trade_id, account_id, deal_ticket, entry, type, volume, price, profit,
                commission, swap, time_server, time_utc, comment, reason
         FROM deals WHERE trade_id = ? ORDER BY time_utc, deal_ticket",
    )
    .bind(trade_id)
    .fetch_all(pool)
    .await?)
}

pub async fn load_trade_images(
    pool: &SqlitePool,
    data_dir: &Path,
    trade_id: &str,
) -> AppResult<Vec<ImageInfo>> {
    load_images(pool, data_dir, "trade", trade_id).await
}

pub async fn load_images(
    pool: &SqlitePool,
    data_dir: &Path,
    owner_type: &str,
    owner_id: &str,
) -> AppResult<Vec<ImageInfo>> {
    let column = match owner_type {
        "day_journal" => "day_journal_id",
        "day_asset" => "day_asset_id",
        "trade" => "trade_id",
        _ => return Err(AppError::Validation("Tipo de propietario no válido".into())),
    };
    let sql = format!(
        "SELECT id, day_journal_id, day_asset_id, trade_id, file_name, relative_path,
                thumb_path, mime, width, height, bytes, caption, sort_order, created_at
         FROM images WHERE {column} = ? ORDER BY sort_order, created_at"
    );

    let mut images = sqlx::query_as::<_, ImageInfo>(sqlx::AssertSqlSafe(sql))
        .bind(owner_id)
        .fetch_all(pool)
        .await?;

    for image in &mut images {
        image.path = data_dir
            .join(&image.relative_path)
            .to_string_lossy()
            .to_string();
        image.thumb_url = image
            .thumb_path
            .as_ref()
            .map(|path| data_dir.join(path).to_string_lossy().to_string());
    }
    Ok(images)
}

pub async fn enrich_trade(pool: &SqlitePool, data_dir: &Path, trade: &mut Trade) -> AppResult<()> {
    trade.deals = load_deals(pool, &trade.id).await?;
    trade.images = load_trade_images(pool, data_dir, &trade.id).await?;
    trade.series_timeframes =
        sqlx::query("SELECT timeframe FROM trade_series WHERE trade_id = ? ORDER BY timeframe")
            .bind(&trade.id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| row.get::<String, _>("timeframe"))
            .collect();

    if let Some(strategy_id) = &trade.strategy_id {
        trade.strategy_name = sqlx::query("SELECT name FROM strategies WHERE id = ?")
            .bind(strategy_id)
            .fetch_optional(pool)
            .await?
            .map(|row| row.get::<String, _>("name"));
    }
    if let Some(day_asset_id) = &trade.day_asset_id {
        trade.day_asset_symbol = sqlx::query("SELECT symbol FROM day_assets WHERE id = ?")
            .bind(day_asset_id)
            .fetch_optional(pool)
            .await?
            .map(|row| row.get::<String, _>("symbol"));
    }
    Ok(())
}

pub async fn load_trade(pool: &SqlitePool, data_dir: &Path, trade_id: &str) -> AppResult<Trade> {
    let mut trade = sqlx::query_as::<_, Trade>(
        "SELECT id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
                volume_current, open_time_server, open_time_utc, close_time_server,
                close_time_utc, open_price, close_price, sl, tp, profit_gross, commission,
                swap, profit_net, magic, comment, strategy_id, risk_amount, r_multiple,
                notes, state, created_at, updated_at
         FROM trades WHERE id = ?",
    )
    .bind(trade_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Operación no encontrada".into()))?;

    enrich_trade(pool, data_dir, &mut trade).await?;
    Ok(trade)
}

pub async fn list_day_trades(
    pool: &SqlitePool,
    data_dir: &Path,
    account_id: &str,
    date: &str,
) -> AppResult<Vec<Trade>> {
    let mut trades = sqlx::query_as::<_, Trade>(
        "SELECT id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
                volume_current, open_time_server, open_time_utc, close_time_server,
                close_time_utc, open_price, close_price, sl, tp, profit_gross, commission,
                swap, profit_net, magic, comment, strategy_id, risk_amount, r_multiple,
                notes, state, created_at, updated_at
         FROM trades
         WHERE account_id = ? AND state = 'closed' AND substr(close_time_server, 1, 10) = ?
         ORDER BY close_time_server, position_ticket",
    )
    .bind(account_id)
    .bind(date)
    .fetch_all(pool)
    .await?;

    for trade in &mut trades {
        enrich_trade(pool, data_dir, trade).await?;
    }
    Ok(trades)
}

pub async fn load_open_trades(
    pool: &SqlitePool,
    data_dir: &Path,
    account_id: &str,
) -> AppResult<Vec<Trade>> {
    let mut trades = sqlx::query_as::<_, Trade>(
        "SELECT id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
                volume_current, open_time_server, open_time_utc, close_time_server,
                close_time_utc, open_price, close_price, sl, tp, profit_gross, commission,
                swap, profit_net, magic, comment, strategy_id, risk_amount, r_multiple,
                notes, state, created_at, updated_at
         FROM trades WHERE account_id = ? AND state = 'open'
         ORDER BY open_time_server DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    for trade in &mut trades {
        enrich_trade(pool, data_dir, trade).await?;
    }
    Ok(trades)
}

pub fn asset_stats(trades: &[Trade]) -> AssetDayStats {
    let mut stats = AssetDayStats::default();
    for trade in trades {
        if trade.state != "closed" {
            continue;
        }
        stats.trades += 1;
        stats.net_profit += trade.profit_net;
        if trade.profit_net > 0.0 {
            stats.wins += 1;
        } else if trade.profit_net < 0.0 {
            stats.losses += 1;
        }
    }
    if stats.trades > 0 {
        stats.win_rate = stats.wins as f64 / stats.trades as f64 * 100.0;
    }
    stats
}

pub async fn load_day_view(
    pool: &SqlitePool,
    data_dir: &Path,
    account_id: &str,
    date: &str,
) -> AppResult<DayView> {
    let account = load_account(pool, account_id).await?;
    let journal = ensure_day_journal(pool, account_id, date).await?;
    let assets = sqlx::query_as::<_, DayAsset>(
        "SELECT id, journal_id, symbol, strategy_id, journal_html, created_at, updated_at
         FROM day_assets WHERE journal_id = ? ORDER BY symbol",
    )
    .bind(&journal.id)
    .fetch_all(pool)
    .await?;

    let strategies = load_strategies(pool, true).await?;
    let strategy_names: HashMap<String, String> = strategies
        .iter()
        .map(|strategy| (strategy.id.clone(), strategy.name.clone()))
        .collect();

    let day_trades = list_day_trades(pool, data_dir, account_id, date).await?;
    let mut assets_view = Vec::with_capacity(assets.len());
    for asset in assets {
        let plan_items = load_plan_items(pool, &asset.id).await?;
        let images = load_images(pool, data_dir, "day_asset", &asset.id).await?;
        let trades: Vec<Trade> = day_trades
            .iter()
            .filter(|trade| {
                trade.day_asset_id.as_deref() == Some(asset.id.as_str())
                    || (trade.day_asset_id.is_none() && trade.symbol == asset.symbol)
            })
            .cloned()
            .collect();
        let stats = asset_stats(&trades);
        assets_view.push(DayAssetView {
            strategy_name: asset
                .strategy_id
                .as_ref()
                .and_then(|id| strategy_names.get(id).cloned()),
            asset,
            plan_items,
            trades,
            images,
            stats,
        });
    }

    let open_trades = load_open_trades(pool, data_dir, account_id).await?;
    let images = load_images(pool, data_dir, "day_journal", &journal.id).await?;

    Ok(DayView {
        account,
        journal,
        assets: assets_view,
        open_trades,
        images,
    })
}

#[cfg(test)]
mod tests {
    use sqlx::Row;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn seed_account_and_strategies(pool: &SqlitePool) -> (String, String, String) {
        let now = now_iso();
        let account_id = "account-test".to_string();
        sqlx::query(
            "INSERT INTO accounts
             (id, login, server, display_name, company, currency, status,
              broker_utc_offset_seconds, balance, equity, created_at, updated_at)
             VALUES (?, 1001, 'Test', 'Cuenta de prueba', '', 'USD', 'active', 0, 1000, 1000, ?, ?)",
        )
        .bind(&account_id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        for (id, name, color, text, sort_order) in [
            ("strategy-a", "Estrategia A", "#111111", "Punto A", 0i64),
            ("strategy-a", "Estrategia A", "#111111", "Punto A bis", 1i64),
            ("strategy-b", "Estrategia B", "#222222", "Punto B", 0i64),
        ] {
            sqlx::query(
                "INSERT OR IGNORE INTO strategies
                 (id, name, description, color, archived, sort_order, created_at, updated_at)
                 VALUES (?, ?, '', ?, 0, 0, ?, ?)",
            )
            .bind(id)
            .bind(name)
            .bind(color)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO strategy_items
                 (id, strategy_id, section, text, sort_order, required, created_at)
                 VALUES (?, ?, 'General', ?, ?, 1, ?)",
            )
            .bind(format!("{id}-item-{sort_order}"))
            .bind(id)
            .bind(text)
            .bind(sort_order)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        }

        (
            account_id,
            "strategy-a".to_string(),
            "strategy-b".to_string(),
        )
    }

    async fn seed_group(
        pool: &SqlitePool,
        group_id: &str,
        default_strategy_id: Option<&str>,
        items: &[(&str, Option<&str>, bool)],
    ) {
        let now = now_iso();
        sqlx::query(
            "INSERT INTO asset_groups
             (id, name, description, color, default_strategy_id, sort_order, created_at, updated_at)
             VALUES (?, 'Grupo de prueba', '', '#6d7cff', ?, 0, ?, ?)",
        )
        .bind(group_id)
        .bind(default_strategy_id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        for (index, (symbol, strategy_id, strategy_overridden)) in items.iter().enumerate() {
            sqlx::query(
                "INSERT INTO asset_group_items
                 (id, group_id, symbol, strategy_id, strategy_overridden, sort_order, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("{group_id}-item-{index}"))
            .bind(group_id)
            .bind(symbol)
            .bind(strategy_id)
            .bind(strategy_overridden)
            .bind(index as i64)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn add_group_creates_assets_with_per_asset_strategies() {
        let pool = test_pool().await;
        let (account_id, strategy_a, strategy_b) = seed_account_and_strategies(&pool).await;
        seed_group(
            &pool,
            "group-1",
            Some(strategy_a.as_str()),
            &[
                ("eurusd", Some(strategy_b.as_str()), true),
                ("gbpusd", None, false),
            ],
        )
        .await;

        let groups = load_asset_groups(&pool).await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].items.len(), 2);
        assert_eq!(groups[0].items[0].symbol, "eurusd");

        let result = add_asset_group_to_day(
            &pool,
            &account_id,
            "2026-10-30",
            "group-1",
            ExistingAssetAction::Skip,
            false,
        )
        .await
        .unwrap();

        assert_eq!(result.added, vec!["eurusd", "gbpusd"]);
        assert!(result.updated.is_empty());
        assert!(result.skipped.is_empty());

        let rows = sqlx::query("SELECT symbol, strategy_id FROM day_assets ORDER BY symbol")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get::<String, _>("symbol"), "eurusd");
        assert_eq!(
            rows[0].get::<Option<String>, _>("strategy_id"),
            Some(strategy_b)
        );
        assert_eq!(rows[1].get::<String, _>("symbol"), "gbpusd");
        assert_eq!(
            rows[1].get::<Option<String>, _>("strategy_id"),
            Some(strategy_a.clone())
        );

        let eurusd_id: String =
            sqlx::query_scalar("SELECT id FROM day_assets WHERE symbol = 'eurusd'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let eurusd_plan_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM plan_items WHERE day_asset_id = ?")
                .bind(&eurusd_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(eurusd_plan_count, 1);

        let gbpusd_id: String =
            sqlx::query_scalar("SELECT id FROM day_assets WHERE symbol = 'gbpusd'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let gbpusd_plan_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM plan_items WHERE day_asset_id = ?")
                .bind(&gbpusd_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(gbpusd_plan_count, 2);
    }

    #[tokio::test]
    async fn add_group_can_skip_or_update_existing_assets() {
        let pool = test_pool().await;
        let (account_id, strategy_a, strategy_b) = seed_account_and_strategies(&pool).await;
        seed_group(
            &pool,
            "group-1",
            Some(strategy_a.as_str()),
            &[("eurusd", None, false), ("gbpusd", None, false)],
        )
        .await;

        add_asset_group_to_day(
            &pool,
            &account_id,
            "2026-10-30",
            "group-1",
            ExistingAssetAction::Skip,
            false,
        )
        .await
        .unwrap();

        let skipped = add_asset_group_to_day(
            &pool,
            &account_id,
            "2026-10-30",
            "group-1",
            ExistingAssetAction::Skip,
            false,
        )
        .await
        .unwrap();
        assert!(skipped.added.is_empty());
        assert_eq!(skipped.skipped, vec!["eurusd", "gbpusd"]);

        let eurusd_id: String =
            sqlx::query_scalar("SELECT id FROM day_assets WHERE symbol = 'eurusd'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query("UPDATE day_assets SET strategy_id = ? WHERE id = ?")
            .bind(&strategy_b)
            .bind(&eurusd_id)
            .execute(&pool)
            .await
            .unwrap();
        snapshot_strategy_items(&pool, &eurusd_id, &strategy_b)
            .await
            .unwrap();

        let now = now_iso();
        sqlx::query(
            "INSERT INTO trades
             (id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
              volume_current, open_time_server, open_time_utc, open_price, profit_gross,
              commission, swap, profit_net, comment, strategy_id, notes, state, created_at, updated_at)
             VALUES ('trade-test', ?, ?, 1, 'EURUSD', 'buy', 1, 1, ?, ?, 1.1, 10, 0, 0, 10,
                     '', ?, '', 'closed', ?, ?)",
        )
        .bind(&account_id)
        .bind(&eurusd_id)
        .bind(&now)
        .bind(&now)
        .bind(&strategy_b)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

        let updated = add_asset_group_to_day(
            &pool,
            &account_id,
            "2026-10-30",
            "group-1",
            ExistingAssetAction::Update,
            true,
        )
        .await
        .unwrap();
        assert!(updated.updated.contains(&"eurusd".to_string()));

        let stored_strategy: Option<String> =
            sqlx::query_scalar("SELECT strategy_id FROM day_assets WHERE id = ?")
                .bind(&eurusd_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored_strategy, Some(strategy_a.clone()));

        let plan_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM plan_items WHERE day_asset_id = ?")
                .bind(&eurusd_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(plan_count, 2);

        let trade_strategy: Option<String> =
            sqlx::query_scalar("SELECT strategy_id FROM trades WHERE id = 'trade-test'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(trade_strategy, Some(strategy_a));
    }
}
