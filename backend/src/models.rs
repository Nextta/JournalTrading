use serde::{Deserialize, Serialize};

pub const TIMEFRAMES: [&str; 8] = ["M1", "M5", "M15", "M30", "H1", "H4", "D1", "W1"];

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub login: i64,
    pub server: String,
    pub display_name: String,
    pub company: String,
    pub currency: String,
    pub leverage: Option<i64>,
    pub status: String,
    pub broker_utc_offset_seconds: i64,
    pub balance: f64,
    pub equity: f64,
    pub last_heartbeat_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    pub login: i64,
    pub server: String,
    pub display_name: String,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    pub display_name: String,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    pub api_key: String,
    pub api_port: u16,
    pub active_account_id: Option<String>,
    pub timezone_mode: String,
    pub data_dir: String,
    pub version: String,
    pub mt5: Mt5Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdate {
    pub api_port: u16,
    pub timezone_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Mt5Status {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub app_version: String,
    pub connected: bool,
    pub last_heartbeat_at: Option<String>,
    pub login: Option<i64>,
    pub server: Option<String>,
    pub ea_version: Option<String>,
    pub terminal_build: Option<i64>,
    pub terminal_data_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub accounts: Vec<Account>,
    pub settings: SettingsResponse,
    pub strategies: Vec<Strategy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Strategy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub color: String,
    pub archived: bool,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    #[sqlx(skip)]
    #[serde(default)]
    pub items: Vec<StrategyItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StrategyItem {
    pub id: String,
    pub strategy_id: String,
    pub section: String,
    pub text: String,
    pub sort_order: i64,
    pub required: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyItemInput {
    pub id: Option<String>,
    #[serde(default)]
    pub section: String,
    pub text: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub items: Vec<StrategyItemInput>,
}

fn default_color() -> String {
    "#6d7cff".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AssetGroupItem {
    pub id: String,
    pub group_id: String,
    pub symbol: String,
    pub strategy_id: Option<String>,
    pub strategy_overridden: bool,
    pub sort_order: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AssetGroup {
    pub id: String,
    pub name: String,
    pub description: String,
    pub color: String,
    pub default_strategy_id: Option<String>,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    #[sqlx(skip)]
    #[serde(default)]
    pub items: Vec<AssetGroupItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetGroupItemInput {
    #[serde(default)]
    pub symbol: String,
    pub strategy_id: Option<String>,
    #[serde(default)]
    pub strategy_overridden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetGroupInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_color")]
    pub color: String,
    pub default_strategy_id: Option<String>,
    #[serde(default)]
    pub items: Vec<AssetGroupItemInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingAssetAction {
    Skip,
    Update,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddGroupResult {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DayJournal {
    pub id: String,
    pub account_id: String,
    pub date: String,
    pub general_html: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DayAsset {
    pub id: String,
    pub journal_id: String,
    pub symbol: String,
    pub strategy_id: Option<String>,
    pub journal_html: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub id: String,
    pub day_asset_id: String,
    pub template_item_id: Option<String>,
    pub section: String,
    pub text: String,
    pub sort_order: i64,
    pub required: bool,
    pub checked: bool,
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Deal {
    pub id: String,
    pub trade_id: String,
    pub account_id: String,
    pub deal_ticket: i64,
    pub entry: i64,
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub deal_type: i64,
    pub volume: f64,
    pub price: f64,
    pub profit: f64,
    pub commission: f64,
    pub swap: f64,
    pub time_server: String,
    pub time_utc: String,
    pub comment: String,
    pub reason: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    pub id: String,
    pub account_id: String,
    pub day_asset_id: Option<String>,
    pub position_ticket: i64,
    pub symbol: String,
    pub side: String,
    pub volume_initial: f64,
    pub volume_current: f64,
    pub open_time_server: String,
    pub open_time_utc: String,
    pub close_time_server: Option<String>,
    pub close_time_utc: Option<String>,
    pub open_price: f64,
    pub close_price: Option<f64>,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
    pub profit_gross: f64,
    pub commission: f64,
    pub swap: f64,
    pub profit_net: f64,
    pub magic: Option<i64>,
    pub comment: String,
    pub strategy_id: Option<String>,
    pub risk_amount: Option<f64>,
    pub r_multiple: Option<f64>,
    pub notes: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    #[sqlx(skip)]
    #[serde(default)]
    pub strategy_name: Option<String>,
    #[sqlx(skip)]
    #[serde(default)]
    pub day_asset_symbol: Option<String>,
    #[sqlx(skip)]
    #[serde(default)]
    pub deals: Vec<Deal>,
    #[sqlx(skip)]
    #[serde(default)]
    pub images: Vec<ImageInfo>,
    #[sqlx(skip)]
    #[serde(default)]
    pub series_timeframes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeFilter {
    pub account_id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub strategy_ids: Vec<String>,
    pub state: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradePage {
    pub items: Vec<Trade>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandleBar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(default)]
    pub volume: f64,
    #[serde(default)]
    pub spread: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeSeries {
    pub trade_id: String,
    pub timeframe: String,
    pub from_time: i64,
    pub to_time: i64,
    pub bar_count: i64,
    pub bars: Vec<CandleBar>,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub id: String,
    pub day_journal_id: Option<String>,
    pub day_asset_id: Option<String>,
    pub trade_id: Option<String>,
    pub file_name: String,
    pub relative_path: String,
    pub thumb_path: Option<String>,
    pub mime: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bytes: i64,
    pub caption: String,
    pub sort_order: i64,
    pub created_at: String,
    #[sqlx(skip)]
    #[serde(default)]
    pub path: String,
    #[sqlx(skip)]
    #[serde(default)]
    pub thumb_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSaveInput {
    pub owner_type: String,
    pub owner_id: String,
    pub file_name: String,
    #[serde(default)]
    pub caption: String,
    pub data_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetDayStats {
    pub trades: i64,
    pub net_profit: f64,
    pub wins: i64,
    pub losses: i64,
    pub win_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayAssetView {
    pub asset: DayAsset,
    pub strategy_name: Option<String>,
    pub plan_items: Vec<PlanItem>,
    pub trades: Vec<Trade>,
    pub images: Vec<ImageInfo>,
    pub stats: AssetDayStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayView {
    pub account: Account,
    pub journal: DayJournal,
    pub assets: Vec<DayAssetView>,
    pub open_trades: Vec<Trade>,
    pub images: Vec<ImageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StatsFilter {
    pub account_id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub strategy_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StatsSummary {
    pub total_trades: i64,
    pub wins: i64,
    pub losses: i64,
    pub breakeven: i64,
    pub net_profit: f64,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub win_rate: f64,
    pub profit_factor: Option<f64>,
    pub expectancy: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub avg_r: Option<f64>,
    pub total_r: f64,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub best_trade: f64,
    pub worst_trade: f64,
    pub avg_duration_minutes: i64,
    pub current_streak: i64,
    pub max_win_streak: i64,
    pub max_loss_streak: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquityPoint {
    pub index: i64,
    pub time: String,
    pub date: String,
    pub cumulative: f64,
    pub trade_net: f64,
    pub drawdown: f64,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPnl {
    pub date: String,
    pub net: f64,
    pub trades: i64,
    pub wins: i64,
    pub losses: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownRow {
    pub key: String,
    pub label: String,
    pub trades: i64,
    pub net: f64,
    pub wins: i64,
    pub losses: i64,
    pub win_rate: f64,
    pub profit_factor: Option<f64>,
    pub avg_r: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionBucket {
    pub label: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyRow {
    pub month: String,
    pub net: f64,
    pub trades: i64,
    pub win_rate: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarPeriod {
    pub net: f64,
    pub pct: f64,
    pub trades: i64,
    pub wins: i64,
    pub losses: i64,
    pub r_total: f64,
    pub r_trades: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarDay {
    pub date: String,
    #[serde(flatten)]
    pub period: CalendarPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarMonth {
    pub month: String,
    #[serde(flatten)]
    pub period: CalendarPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarYear {
    pub year: i32,
    pub months: Vec<CalendarMonth>,
    #[serde(flatten)]
    pub period: CalendarPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceCalendar {
    pub currency: String,
    pub start_balance: f64,
    pub end_balance: f64,
    pub days: Vec<CalendarDay>,
    pub months: Vec<CalendarMonth>,
    pub years: Vec<CalendarYear>,
    pub totals: CalendarPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsResponse {
    pub summary: StatsSummary,
    pub equity: Vec<EquityPoint>,
    pub daily_pnl: Vec<DailyPnl>,
    pub by_symbol: Vec<BreakdownRow>,
    pub by_strategy: Vec<BreakdownRow>,
    pub by_weekday: Vec<BreakdownRow>,
    pub by_hour: Vec<BreakdownRow>,
    pub r_distribution: Vec<DistributionBucket>,
    pub monthly: Vec<MonthlyRow>,
}
