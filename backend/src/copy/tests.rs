//! Pruebas del motor de copia sobre una base de datos SQLite temporal.

use serde_json::Value;
use sqlx::{Row, SqlitePool};

use super::model::{FollowerSettings, SizingMode, SymbolSpec};
use super::*;

async fn setup() -> (SqlitePool, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("jt-copy-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = crate::db::init_db(&dir.join("test.db")).await.unwrap();
    for (id, login, server, provider) in [
        ("master", 1001_i64, "Broker-Demo", "mt5"),
        ("mt5f", 1002, "Broker-Demo", "mt5"),
        ("nt8f", 1003, "NinjaTrader 8 · Sim", "ninjatrader"),
    ] {
        sqlx::query(
            "INSERT INTO accounts (id, login, server, provider, display_name, status, balance, equity,
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'active', 10000, 10000, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(login)
        .bind(server)
        .bind(provider)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO copy_groups (id, name, master_account_id, enabled, created_at, updated_at)
         VALUES ('g1', 'Grupo', 'master', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut mt5 = FollowerSettings::default();
    mt5.sizing.mode = SizingMode::Multiplier;
    mt5.sizing.multiplier = 2.0;
    for (id, account, settings) in [
        ("f-mt5", "mt5f", mt5),
        ("f-nt8", "nt8f", FollowerSettings::default()),
    ] {
        sqlx::query(
            "INSERT INTO copy_followers (id, group_id, account_id, enabled, settings_json, created_at, updated_at)
             VALUES (?, 'g1', ?, 1, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(account)
        .bind(serde_json::to_string(&settings).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    }
    (pool, dir)
}

fn signal(kind: &str, symbol: &str, market: &str) -> Signal {
    Signal {
        kind: kind.into(),
        position_ref: "500".into(),
        order_ref: None,
        symbol: symbol.into(),
        market: market.into(),
        side: Some("buy".into()),
        volume: 0.5,
        position_volume: 0.5,
        price: 1.1,
        entry_price: Some(1.1),
        sl: Some(1.098),
        tp: Some(1.105),
        order_type: None,
        order_price: None,
        stop_limit_price: None,
        bid: Some(1.0999),
        ask: Some(1.1),
        time_ms: None,
    }
}

fn eurusd() -> SymbolSpec {
    SymbolSpec {
        symbol: "EURUSD".into(),
        market: "cfd_forex".into(),
        digits: 5,
        tick_size: 0.00001,
        tick_value: 1.0,
        contract_size: 100_000.0,
        volume_min: 0.01,
        volume_max: 100.0,
        volume_step: 0.01,
        bid: 1.0999,
        ask: 1.1,
    }
}

fn context() -> FollowerContext {
    let mut specs = HashMap::new();
    specs.insert("EURUSD".to_string(), eurusd());
    FollowerContext {
        specs,
        balance: 10_000.0,
        equity: 10_000.0,
    }
}

/// Planifica la señal para todas las secundarias del grupo y guarda las órdenes.
async fn dispatch(pool: &SqlitePool, signal: &Signal) {
    let followers = load_followers(pool, "g1").await.unwrap();
    for follower in &followers {
        let ctx = if follower.provider == "mt5" {
            context()
        } else {
            FollowerContext::default()
        };
        let outcome = plan_for_follower(pool, &ctx, "mt5", "master", follower, signal)
            .await
            .unwrap();
        if let Some(planned) = outcome.planned {
            insert_order(pool, None, "g1", &follower.id, &follower.account_id, planned)
                .await
                .unwrap();
        }
    }
}

fn result(order: &Value, status: &str, volume: f64, position_ref: Option<&str>, order_ref: Option<&str>) -> OrderResult {
    OrderResult {
        order_id: order["id"].as_str().unwrap().to_string(),
        ok: status != "rejected",
        status: status.into(),
        error: None,
        symbol: None,
        volume: Some(volume),
        price: Some(1.1001),
        position_ref: position_ref.map(ToString::to_string),
        order_ref: order_ref.map(ToString::to_string),
    }
}

async fn copied_volume(pool: &SqlitePool, follower: &str) -> Option<f64> {
    sqlx::query_scalar("SELECT volume FROM copy_positions WHERE follower_id = ?")
        .bind(follower)
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn copies_open_partial_close_and_close() {
    let (pool, dir) = setup().await;

    // Apertura: la secundaria MT5 copia ×2 con SL/TP como distancia; NT8 no puede operar Forex.
    dispatch(&pool, &signal("open", "EURUSD", "cfd_forex")).await;
    let skipped: String = sqlx::query_scalar("SELECT error FROM copy_orders WHERE follower_id = 'f-nt8'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(skipped.contains("incompatible"));

    let orders = take_orders(&pool, "mt5f", None).await.unwrap();
    assert_eq!(orders.len(), 1);
    let order = &orders[0];
    assert_eq!(order["action"], "market");
    assert_eq!(order["side"], "buy");
    assert_eq!(order["volume"].as_f64().unwrap(), 1.0);
    assert_eq!(order["price_mode"], "offset");
    assert!((order["sl_offset"].as_f64().unwrap() + 0.002).abs() < 1e-9);
    assert!((order["tp_offset"].as_f64().unwrap() - 0.005).abs() < 1e-9);
    assert!(take_orders(&pool, "mt5f", None).await.unwrap().is_empty());

    apply_result(&pool, &result(order, "filled", 1.0, Some("9001"), None)).await.unwrap();
    assert_eq!(copied_volume(&pool, "f-mt5").await, Some(1.0));

    // Cierre parcial del 50 % en la principal → 50 % de lo copiado.
    let mut reduce = signal("reduce", "EURUSD", "cfd_forex");
    reduce.volume = 0.25;
    reduce.position_volume = 0.25;
    dispatch(&pool, &reduce).await;
    let orders = take_orders(&pool, "mt5f", None).await.unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0]["reduce_only"], true);
    assert_eq!(orders[0]["side"], "sell");
    assert_eq!(orders[0]["volume"].as_f64().unwrap(), 0.5);
    assert_eq!(orders[0]["follower_ref"], "9001");
    apply_result(&pool, &result(&orders[0], "filled", 0.5, None, None)).await.unwrap();
    assert_eq!(copied_volume(&pool, "f-mt5").await, Some(0.5));

    // Cierre total.
    let mut close = signal("close", "EURUSD", "cfd_forex");
    close.volume = 0.25;
    close.position_volume = 0.0;
    dispatch(&pool, &close).await;
    let orders = take_orders(&pool, "mt5f", None).await.unwrap();
    assert_eq!(orders[0]["action"], "close");
    assert_eq!(orders[0]["volume"].as_f64().unwrap(), 0.5);
    apply_result(&pool, &result(&orders[0], "filled", 0.5, None, None)).await.unwrap();
    assert_eq!(copied_volume(&pool, "f-mt5").await, None);

    pool.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn copies_futures_across_platforms() {
    let (pool, dir) = setup().await;
    let mut open = signal("open", "NQZ26", "futures");
    open.volume = 2.0;
    open.position_volume = 2.0;
    open.price = 21000.0;
    open.sl = Some(20950.0);
    open.tp = Some(21100.0);
    dispatch(&pool, &open).await;

    let orders = take_orders(&pool, "nt8f", None).await.unwrap();
    assert_eq!(orders.len(), 1);
    let order = &orders[0];
    assert_eq!(order["symbols"][0], "NQ 12-26");
    assert_eq!(order["volume"].as_f64().unwrap(), 2.0);
    assert_eq!(order["price_mode"], "absolute");
    assert_eq!(order["sl"].as_f64().unwrap(), 20950.0);

    // La secundaria MT5 (×2) también puede copiar futuros si su bróker los tiene.
    let orders = take_orders(&pool, "mt5f", None).await.unwrap();
    assert_eq!(orders[0]["volume"].as_f64().unwrap(), 4.0);
    assert_eq!(orders[0]["symbols"][0], "NQZ26");

    pool.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn pending_orders_place_and_fill() {
    let (pool, dir) = setup().await;
    let mut place = signal("pending_place", "EURUSD", "cfd_forex");
    place.position_ref = "777".into();
    place.order_ref = Some("777".into());
    place.order_type = Some("limit".into());
    place.order_price = Some(1.095);
    dispatch(&pool, &place).await;

    let orders = take_orders(&pool, "mt5f", None).await.unwrap();
    assert_eq!(orders.len(), 1);
    let order = &orders[0];
    assert_eq!(order["action"], "pending_place");
    assert_eq!(order["order_type"], "limit");
    // Distancia al precio actual (ask 1.1000): -0.005.
    assert!((order["price_offset"].as_f64().unwrap() + 0.005).abs() < 1e-9);
    apply_result(&pool, &result(order, "placed", 1.0, None, Some("4242"))).await.unwrap();

    // La pendiente de la principal se ejecuta: la secundaria recibe "pending_fill".
    let mut fill = signal("open", "EURUSD", "cfd_forex");
    fill.position_ref = "777".into();
    fill.order_ref = Some("777".into());
    fill.price = 1.095;
    dispatch(&pool, &fill).await;
    let orders = take_orders(&pool, "mt5f", None).await.unwrap();
    assert_eq!(orders[0]["action"], "pending_fill");
    assert_eq!(orders[0]["follower_order_ref"], "4242");
    apply_result(&pool, &result(&orders[0], "filled", 1.0, Some("4242"), None)).await.unwrap();
    let pendings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM copy_pendings").fetch_one(&pool).await.unwrap();
    assert_eq!(pendings, 0);
    assert_eq!(copied_volume(&pool, "f-mt5").await, Some(1.0));

    pool.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn late_entries_are_skipped_and_reconcile_closes_positions() {
    let (pool, dir) = setup().await;
    let mut open = signal("open", "EURUSD", "cfd_forex");
    open.time_ms = Some(now_ms() - 60_000);
    dispatch(&pool, &open).await;
    assert!(take_orders(&pool, "mt5f", None).await.unwrap().is_empty());
    let status: String = sqlx::query_scalar("SELECT status FROM copy_orders WHERE follower_id = 'f-mt5'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "skipped");

    // Una posición copiada que el bridge ya no informa (cerrada por SL) se elimina.
    sqlx::query(
        "INSERT INTO copy_positions (follower_id, master_ref, tag, symbol, side, volume, updated_at)
         VALUES ('f-mt5', '1', 'JTabcdef01', 'EURUSD', 'buy', 1.0, '2026-01-01')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let changed = reconcile_copies(&pool, "mt5f", &[], None).await.unwrap();
    assert!(changed);
    assert_eq!(copied_volume(&pool, "f-mt5").await, None);

    pool.close().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn tradable_filter_only_delivers_matching_orders() {
    let (pool, dir) = setup().await;
    let mut open = signal("open", "NQZ26", "futures");
    open.volume = 1.0;
    open.price = 21000.0;
    dispatch(&pool, &open).await;
    let other = vec!["ESZ6@CME".to_string()];
    assert!(take_orders(&pool, "nt8f", Some(&other)).await.unwrap().is_empty());
    let nq = vec!["nq 12-26".to_string()];
    assert_eq!(take_orders(&pool, "nt8f", Some(&nq)).await.unwrap().len(), 1);

    let row = sqlx::query("SELECT status FROM copy_orders WHERE follower_id = 'f-nt8'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("status"), "sent");

    pool.close().await;
    let _ = std::fs::remove_dir_all(dir);
}
