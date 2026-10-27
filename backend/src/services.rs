use std::collections::HashMap;
use std::path::Path;

use sqlx::{Row, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::{
    AssetDayStats, DayAsset, DayAssetView, DayJournal, DayView, Deal, ImageInfo, PlanItem,
    Strategy, StrategyItem, Trade,
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
    sqlx::query("DELETE FROM plan_items WHERE day_asset_id = ?")
        .bind(day_asset_id)
        .execute(pool)
        .await?;

    let items = load_strategy_items(pool, strategy_id).await?;
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
        .execute(pool)
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
                commission, swap, time_server, time_utc, comment
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
