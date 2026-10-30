use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::{
    BreakdownRow, CalendarDay, CalendarMonth, CalendarPeriod, CalendarYear, DailyPnl,
    DistributionBucket, EquityPoint, MonthlyRow, PerformanceCalendar, StatsFilter, StatsResponse,
    StatsSummary, Trade,
};
use crate::util::round_money;

const WEEKDAYS: [&str; 7] = [
    "Lunes",
    "Martes",
    "Miércoles",
    "Jueves",
    "Viernes",
    "Sábado",
    "Domingo",
];

struct Bucket {
    trades: i64,
    wins: i64,
    losses: i64,
    net: f64,
    gross_profit: f64,
    gross_loss: f64,
    r_values: Vec<f64>,
}

impl Bucket {
    fn new() -> Self {
        Self {
            trades: 0,
            wins: 0,
            losses: 0,
            net: 0.0,
            gross_profit: 0.0,
            gross_loss: 0.0,
            r_values: Vec::new(),
        }
    }

    fn add(&mut self, trade: &Trade) {
        self.trades += 1;
        self.net += trade.profit_net;
        if trade.profit_net > 0.0 {
            self.wins += 1;
            self.gross_profit += trade.profit_net;
        } else if trade.profit_net < 0.0 {
            self.losses += 1;
            self.gross_loss += trade.profit_net.abs();
        }
        if let Some(r) = trade.r_multiple {
            self.r_values.push(r);
        }
    }

    fn to_row(&self, key: &str, label: &str) -> BreakdownRow {
        let avg_r = if self.r_values.is_empty() {
            None
        } else {
            Some(self.r_values.iter().sum::<f64>() / self.r_values.len() as f64)
        };
        BreakdownRow {
            key: key.to_string(),
            label: label.to_string(),
            trades: self.trades,
            net: round_money(self.net),
            wins: self.wins,
            losses: self.losses,
            win_rate: if self.trades > 0 {
                self.wins as f64 / self.trades as f64 * 100.0
            } else {
                0.0
            },
            profit_factor: if self.gross_loss > 0.0 {
                Some(round_money(self.gross_profit / self.gross_loss))
            } else {
                None
            },
            avg_r,
        }
    }
}

struct CalendarBucket {
    net: f64,
    trades: i64,
    wins: i64,
    losses: i64,
    r_total: f64,
    r_trades: i64,
}

impl CalendarBucket {
    fn new() -> Self {
        Self {
            net: 0.0,
            trades: 0,
            wins: 0,
            losses: 0,
            r_total: 0.0,
            r_trades: 0,
        }
    }

    fn add_trade(&mut self, trade: &Trade) {
        self.net += trade.profit_net;
        self.trades += 1;
        if trade.profit_net > 0.0 {
            self.wins += 1;
        } else if trade.profit_net < 0.0 {
            self.losses += 1;
        }
        if let Some(r) = trade.r_multiple {
            self.r_total += r;
            self.r_trades += 1;
        }
    }

    fn merge(&mut self, other: &CalendarBucket) {
        self.net += other.net;
        self.trades += other.trades;
        self.wins += other.wins;
        self.losses += other.losses;
        self.r_total += other.r_total;
        self.r_trades += other.r_trades;
    }

    fn to_period(&self, pct: f64) -> CalendarPeriod {
        CalendarPeriod {
            net: round_money(self.net),
            pct: round_pct(pct),
            trades: self.trades,
            wins: self.wins,
            losses: self.losses,
            r_total: round_r(self.r_total),
            r_trades: self.r_trades,
        }
    }
}

struct CalendarAccum {
    bucket: CalendarBucket,
    start_balance: f64,
}

fn round_pct(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn round_r(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn period_pct(net: f64, start_balance: f64) -> f64 {
    if start_balance.abs() > 0.01 {
        net / start_balance * 100.0
    } else {
        0.0
    }
}

async fn load_trades(pool: &SqlitePool, filter: &StatsFilter) -> AppResult<Vec<Trade>> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, account_id, day_asset_id, position_ticket, symbol, side, volume_initial,
                volume_current, open_time_server, open_time_utc, close_time_server,
                close_time_utc, open_price, close_price, sl, tp, profit_gross, commission,
                swap, profit_net, magic, comment, strategy_id, risk_amount, r_multiple,
                notes, state, created_at, updated_at
         FROM trades WHERE account_id = ",
    );
    builder.push_bind(&filter.account_id);
    builder.push(" AND state = 'closed'");

    if let Some(from) = &filter.from {
        builder.push(" AND substr(close_time_server, 1, 10) >= ");
        builder.push_bind(from);
    }
    if let Some(to) = &filter.to {
        builder.push(" AND substr(close_time_server, 1, 10) <= ");
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
        for id in &filter.strategy_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }

    builder.push(" ORDER BY close_time_utc, position_ticket");
    Ok(builder.build_query_as::<Trade>().fetch_all(pool).await?)
}

pub async fn compute_calendar(
    pool: &SqlitePool,
    account_id: &str,
) -> AppResult<PerformanceCalendar> {
    let account: Option<(String, f64)> =
        sqlx::query_as("SELECT currency, balance FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(pool)
            .await?;
    let (currency, balance) =
        account.ok_or_else(|| AppError::NotFound("Cuenta no encontrada".into()))?;

    let filter = StatsFilter {
        account_id: account_id.to_string(),
        ..StatsFilter::default()
    };
    let trades = load_trades(pool, &filter).await?;

    let mut daily: BTreeMap<String, CalendarBucket> = BTreeMap::new();
    let mut total_bucket = CalendarBucket::new();
    for trade in &trades {
        let server_time = trade
            .close_time_server
            .as_deref()
            .unwrap_or(&trade.open_time_server);
        let date = server_time.get(0..10).unwrap_or("");
        if date.len() != 10 {
            continue;
        }
        daily
            .entry(date.to_string())
            .or_insert_with(CalendarBucket::new)
            .add_trade(trade);
        total_bucket.add_trade(trade);
    }

    let start_balance = balance - total_bucket.net;
    let mut running_balance = start_balance;
    let mut days = Vec::with_capacity(daily.len());
    let mut months: BTreeMap<String, CalendarAccum> = BTreeMap::new();
    let mut years: BTreeMap<i32, CalendarAccum> = BTreeMap::new();

    for (date, bucket) in daily {
        let day_pct = period_pct(bucket.net, running_balance);
        days.push(CalendarDay {
            date: date.clone(),
            period: bucket.to_period(day_pct),
        });

        if date.len() >= 7 {
            months
                .entry(date[0..7].to_string())
                .and_modify(|accum| accum.bucket.merge(&bucket))
                .or_insert_with(|| CalendarAccum {
                    bucket: {
                        let mut initial = CalendarBucket::new();
                        initial.merge(&bucket);
                        initial
                    },
                    start_balance: running_balance,
                });
        }
        if let Ok(year) = date[0..4].parse::<i32>() {
            years
                .entry(year)
                .and_modify(|accum| accum.bucket.merge(&bucket))
                .or_insert_with(|| CalendarAccum {
                    bucket: {
                        let mut initial = CalendarBucket::new();
                        initial.merge(&bucket);
                        initial
                    },
                    start_balance: running_balance,
                });
        }

        running_balance += bucket.net;
    }

    let month_rows: Vec<CalendarMonth> = months
        .into_iter()
        .map(|(month, accum)| CalendarMonth {
            month,
            period: accum
                .bucket
                .to_period(period_pct(accum.bucket.net, accum.start_balance)),
        })
        .collect();

    let year_rows: Vec<CalendarYear> = years
        .into_iter()
        .map(|(year, accum)| {
            let prefix = format!("{year}-");
            let months = month_rows
                .iter()
                .filter(|month| month.month.starts_with(&prefix))
                .cloned()
                .collect();
            CalendarYear {
                year,
                months,
                period: accum
                    .bucket
                    .to_period(period_pct(accum.bucket.net, accum.start_balance)),
            }
        })
        .collect();

    let totals = total_bucket.to_period(period_pct(total_bucket.net, start_balance));

    Ok(PerformanceCalendar {
        currency,
        start_balance: round_money(start_balance),
        end_balance: round_money(balance),
        days,
        months: month_rows,
        years: year_rows,
        totals,
    })
}

pub async fn compute_stats(pool: &SqlitePool, filter: &StatsFilter) -> AppResult<StatsResponse> {
    let trades = load_trades(pool, filter).await?;
    let strategy_rows = sqlx::query("SELECT id, name FROM strategies")
        .fetch_all(pool)
        .await?;
    let strategy_names: HashMap<String, String> = strategy_rows
        .into_iter()
        .map(|row| {
            (
                sqlx::Row::get::<String, _>(&row, "id"),
                sqlx::Row::get::<String, _>(&row, "name"),
            )
        })
        .collect();

    let mut summary = StatsSummary {
        best_trade: f64::NEG_INFINITY,
        worst_trade: f64::INFINITY,
        ..StatsSummary::default()
    };
    let mut equity = Vec::with_capacity(trades.len());
    let mut daily: BTreeMap<String, DailyPnl> = BTreeMap::new();
    let mut monthly: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut by_symbol: HashMap<String, Bucket> = HashMap::new();
    let mut by_strategy: HashMap<String, Bucket> = HashMap::new();
    let mut by_weekday: Vec<Bucket> = (0..7).map(|_| Bucket::new()).collect();
    let mut by_hour: Vec<Bucket> = (0..24).map(|_| Bucket::new()).collect();
    let mut r_counts = [0_i64; 8];

    let mut cumulative: f64 = 0.0;
    let mut peak: f64 = 0.0;
    let mut max_drawdown: f64 = 0.0;
    let mut max_drawdown_pct: f64 = 0.0;
    let mut current_streak = 0_i64;
    let mut max_win_streak = 0_i64;
    let mut max_loss_streak = 0_i64;
    let mut durations = Vec::new();
    let mut r_values = Vec::new();

    for (index, trade) in trades.iter().enumerate() {
        summary.total_trades += 1;
        summary.net_profit += trade.profit_net;

        if trade.profit_net > 0.0 {
            summary.wins += 1;
            summary.gross_profit += trade.profit_net;
            if current_streak >= 0 {
                current_streak += 1;
            } else {
                current_streak = 1;
            }
            max_win_streak = max_win_streak.max(current_streak);
        } else if trade.profit_net < 0.0 {
            summary.losses += 1;
            summary.gross_loss += trade.profit_net.abs();
            if current_streak <= 0 {
                current_streak -= 1;
            } else {
                current_streak = -1;
            }
            max_loss_streak = max_loss_streak.max(-current_streak);
        } else {
            summary.breakeven += 1;
            current_streak = 0;
        }

        summary.best_trade = summary.best_trade.max(trade.profit_net);
        summary.worst_trade = summary.worst_trade.min(trade.profit_net);

        if let Some(r) = trade.r_multiple {
            r_values.push(r);
            summary.total_r += r;
            let bucket = if r < -2.0 {
                0
            } else if r < -1.0 {
                1
            } else if r < 0.0 {
                2
            } else if r < 1.0 {
                3
            } else if r < 2.0 {
                4
            } else if r < 3.0 {
                5
            } else if r < 5.0 {
                6
            } else {
                7
            };
            r_counts[bucket] += 1;
        }

        if let (open, Some(close)) = (
            trade.open_time_utc.as_str(),
            trade.close_time_utc.as_deref(),
        ) && let (Ok(open), Ok(close)) = (
            DateTime::parse_from_rfc3339(open),
            DateTime::parse_from_rfc3339(close),
        ) {
            durations.push((close - open).num_minutes());
        }

        cumulative += trade.profit_net;
        peak = peak.max(cumulative);
        let drawdown = peak - cumulative;
        max_drawdown = max_drawdown.max(drawdown);
        if peak > 0.0 {
            max_drawdown_pct = max_drawdown_pct.max(drawdown / peak * 100.0);
        }

        let server_time = trade
            .close_time_server
            .as_deref()
            .unwrap_or(&trade.open_time_server);
        let date = server_time.get(0..10).unwrap_or("").to_string();

        equity.push(EquityPoint {
            index: index as i64,
            time: trade.close_time_utc.clone().unwrap_or_default(),
            date: date.clone(),
            cumulative: round_money(cumulative),
            trade_net: round_money(trade.profit_net),
            drawdown: round_money(drawdown),
            symbol: trade.symbol.clone(),
        });

        let daily_entry = daily.entry(date.clone()).or_insert_with(|| DailyPnl {
            date: date.clone(),
            net: 0.0,
            trades: 0,
            wins: 0,
            losses: 0,
        });
        daily_entry.net += trade.profit_net;
        daily_entry.trades += 1;
        if trade.profit_net > 0.0 {
            daily_entry.wins += 1;
        } else if trade.profit_net < 0.0 {
            daily_entry.losses += 1;
        }

        if date.len() >= 7 {
            monthly
                .entry(date[0..7].to_string())
                .or_insert_with(Bucket::new)
                .add(trade);
        }
        by_symbol
            .entry(trade.symbol.clone())
            .or_insert_with(Bucket::new)
            .add(trade);
        let strategy_key = trade
            .strategy_id
            .clone()
            .unwrap_or_else(|| "_none".to_string());
        by_strategy
            .entry(strategy_key)
            .or_insert_with(Bucket::new)
            .add(trade);

        if let Ok(naive) = NaiveDateTime::parse_from_str(server_time, "%Y-%m-%dT%H:%M:%S") {
            let weekday = naive.weekday().num_days_from_monday() as usize;
            by_weekday[weekday].add(trade);
            by_hour[naive.hour() as usize].add(trade);
        }
    }

    if trades.is_empty() {
        summary.best_trade = 0.0;
        summary.worst_trade = 0.0;
    } else {
        summary.best_trade = round_money(summary.best_trade);
        summary.worst_trade = round_money(summary.worst_trade);
    }

    summary.net_profit = round_money(summary.net_profit);
    summary.gross_profit = round_money(summary.gross_profit);
    summary.gross_loss = round_money(summary.gross_loss);
    summary.win_rate = if summary.total_trades > 0 {
        summary.wins as f64 / summary.total_trades as f64 * 100.0
    } else {
        0.0
    };
    summary.profit_factor = if summary.gross_loss > 0.0 {
        Some(round_money(summary.gross_profit / summary.gross_loss))
    } else {
        None
    };
    summary.expectancy = if summary.total_trades > 0 {
        round_money(summary.net_profit / summary.total_trades as f64)
    } else {
        0.0
    };
    summary.avg_win = if summary.wins > 0 {
        round_money(summary.gross_profit / summary.wins as f64)
    } else {
        0.0
    };
    summary.avg_loss = if summary.losses > 0 {
        round_money(-summary.gross_loss / summary.losses as f64)
    } else {
        0.0
    };
    summary.avg_r = if r_values.is_empty() {
        None
    } else {
        Some(r_values.iter().sum::<f64>() / r_values.len() as f64)
    };
    summary.total_r = r_values.iter().sum();
    summary.max_drawdown = round_money(max_drawdown);
    summary.max_drawdown_pct = max_drawdown_pct;
    summary.avg_duration_minutes = if durations.is_empty() {
        0
    } else {
        durations.iter().sum::<i64>() / durations.len() as i64
    };
    summary.current_streak = current_streak;
    summary.max_win_streak = max_win_streak;
    summary.max_loss_streak = max_loss_streak;

    let mut daily_pnl: Vec<DailyPnl> = daily
        .into_values()
        .map(|mut row| {
            row.net = round_money(row.net);
            row
        })
        .collect();
    daily_pnl.sort_by(|a, b| a.date.cmp(&b.date));

    let mut monthly_rows: Vec<MonthlyRow> = monthly
        .into_iter()
        .map(|(month, bucket)| MonthlyRow {
            month,
            net: round_money(bucket.net),
            trades: bucket.trades,
            win_rate: if bucket.trades > 0 {
                bucket.wins as f64 / bucket.trades as f64 * 100.0
            } else {
                0.0
            },
        })
        .collect();
    monthly_rows.sort_by(|a, b| a.month.cmp(&b.month));

    let mut symbol_rows: Vec<BreakdownRow> = by_symbol
        .into_iter()
        .map(|(key, bucket)| bucket.to_row(&key, &key))
        .collect();
    symbol_rows.sort_by(|a, b| {
        b.net
            .partial_cmp(&a.net)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut strategy_rows: Vec<BreakdownRow> = by_strategy
        .into_iter()
        .map(|(key, bucket)| {
            let label = if key == "_none" {
                "Sin estrategia".to_string()
            } else {
                strategy_names
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "Estrategia".to_string())
            };
            bucket.to_row(&key, &label)
        })
        .collect();
    strategy_rows.sort_by(|a, b| {
        b.net
            .partial_cmp(&a.net)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let weekday_rows: Vec<BreakdownRow> = by_weekday
        .iter()
        .enumerate()
        .filter(|(_, bucket)| bucket.trades > 0)
        .map(|(index, bucket)| bucket.to_row(&index.to_string(), WEEKDAYS[index]))
        .collect();

    let hour_rows: Vec<BreakdownRow> = by_hour
        .iter()
        .enumerate()
        .filter(|(_, bucket)| bucket.trades > 0)
        .map(|(index, bucket)| bucket.to_row(&index.to_string(), &format!("{index:02}:00")))
        .collect();

    let r_labels = [
        "< -2R",
        "-2R a -1R",
        "-1R a 0",
        "0 a 1R",
        "1R a 2R",
        "2R a 3R",
        "3R a 5R",
        "> 5R",
    ];
    let r_distribution = r_labels
        .iter()
        .enumerate()
        .map(|(index, label)| DistributionBucket {
            label: (*label).to_string(),
            count: r_counts[index],
        })
        .collect();

    Ok(StatsResponse {
        summary,
        equity,
        daily_pnl,
        by_symbol: symbol_rows,
        by_strategy: strategy_rows,
        by_weekday: weekday_rows,
        by_hour: hour_rows,
        r_distribution,
        monthly: monthly_rows,
    })
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;
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

    async fn seed_account(pool: &SqlitePool, id: &str, balance: f64) {
        sqlx::query(
            "INSERT INTO accounts
             (id, login, server, display_name, company, currency, status,
              broker_utc_offset_seconds, balance, equity, created_at, updated_at)
             VALUES (?, 1001, 'Test', 'Cuenta de prueba', '', 'USD', 'active', 0, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(balance)
        .bind(balance)
        .bind("2026-02-10T00:00:00Z")
        .bind("2026-02-10T00:00:00Z")
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_trade(
        pool: &SqlitePool,
        account_id: &str,
        ticket: i64,
        date: &str,
        net: f64,
        r_multiple: Option<f64>,
    ) {
        sqlx::query(
            "INSERT INTO trades
             (id, account_id, position_ticket, symbol, side, volume_initial, volume_current,
              open_time_server, open_time_utc, close_time_server, close_time_utc, open_price,
              close_price, profit_gross, commission, swap, profit_net, comment, notes, state,
              created_at, updated_at)
             VALUES (?, ?, ?, 'EURUSD', 'buy', 0.10, 0.10, ?, ?, ?, ?, 1.0, 1.1, ?, 0, 0, ?, '', '', 'closed', ?, ?)",
        )
        .bind(format!("trade-{ticket}"))
        .bind(account_id)
        .bind(ticket)
        .bind(format!("{date}T10:00:00"))
        .bind(format!("{date}T10:00:00Z"))
        .bind(format!("{date}T12:00:00"))
        .bind(format!("{date}T12:00:00Z"))
        .bind(net)
        .bind(net)
        .bind("2026-02-10T00:00:00Z")
        .bind("2026-02-10T00:00:00Z")
        .execute(pool)
        .await
        .unwrap();

        if let Some(r) = r_multiple {
            sqlx::query(
                "UPDATE trades SET r_multiple = ?, risk_amount = 10 WHERE position_ticket = ?",
            )
            .bind(r)
            .bind(ticket)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn calendar_aggregates_compounded_returns_and_r_coverage() {
        let pool = test_pool().await;
        let account_id = "account-calendar";
        seed_account(&pool, account_id, 1100.0).await;

        insert_trade(&pool, account_id, 1, "2025-12-31", 40.0, Some(2.0)).await;
        insert_trade(&pool, account_id, 2, "2026-01-02", 60.0, None).await;
        insert_trade(&pool, account_id, 3, "2026-01-05", -20.0, Some(-1.0)).await;
        insert_trade(&pool, account_id, 4, "2026-02-02", 20.0, Some(1.0)).await;

        let calendar = compute_calendar(&pool, account_id).await.unwrap();

        assert_eq!(calendar.currency, "USD");
        assert_eq!(calendar.start_balance, 1000.0);
        assert_eq!(calendar.end_balance, 1100.0);

        assert_eq!(calendar.days.len(), 4);
        assert_eq!(calendar.days[0].date, "2025-12-31");
        assert_eq!(calendar.days[0].period.net, 40.0);
        assert!((calendar.days[0].period.pct - 4.0).abs() < 1e-9);
        assert_eq!(calendar.days[1].date, "2026-01-02");
        assert!((calendar.days[1].period.pct - 5.7692).abs() < 1e-9);
        assert!((calendar.days[2].period.pct - (-1.8182)).abs() < 1e-9);
        assert!((calendar.days[3].period.pct - 1.8519).abs() < 1e-9);

        let january = calendar
            .months
            .iter()
            .find(|month| month.month == "2026-01")
            .unwrap();
        assert_eq!(january.period.net, 40.0);
        assert!((january.period.pct - 3.8462).abs() < 1e-9);
        assert_eq!(january.period.trades, 2);
        assert_eq!(january.period.wins, 1);
        assert_eq!(january.period.losses, 1);
        assert_eq!(january.period.r_total, -1.0);
        assert_eq!(january.period.r_trades, 1);

        let year_2026 = calendar
            .years
            .iter()
            .find(|year| year.year == 2026)
            .unwrap();
        assert_eq!(year_2026.months.len(), 2);
        assert_eq!(year_2026.period.net, 60.0);
        assert!((year_2026.period.pct - 5.7692).abs() < 1e-9);

        assert_eq!(calendar.totals.net, 100.0);
        assert!((calendar.totals.pct - 10.0).abs() < 1e-9);
        assert_eq!(calendar.totals.trades, 4);
        assert_eq!(calendar.totals.wins, 3);
        assert_eq!(calendar.totals.losses, 1);
        assert_eq!(calendar.totals.r_total, 2.0);
        assert_eq!(calendar.totals.r_trades, 3);
    }

    #[tokio::test]
    async fn calendar_without_trades_keeps_balance_and_empty_periods() {
        let pool = test_pool().await;
        let account_id = "account-empty";
        seed_account(&pool, account_id, 500.0).await;

        let calendar = compute_calendar(&pool, account_id).await.unwrap();

        assert_eq!(calendar.start_balance, 500.0);
        assert_eq!(calendar.end_balance, 500.0);
        assert!(calendar.days.is_empty());
        assert!(calendar.months.is_empty());
        assert!(calendar.years.is_empty());
        assert_eq!(calendar.totals.trades, 0);
        assert_eq!(calendar.totals.net, 0.0);
        assert_eq!(calendar.totals.pct, 0.0);
    }

    #[tokio::test]
    async fn calendar_rejects_unknown_account() {
        let pool = test_pool().await;
        let error = compute_calendar(&pool, "missing").await.unwrap_err();
        assert!(matches!(error, AppError::NotFound(_)));
    }
}
