use serde::{Deserialize, Serialize};

/// Método de cálculo del tamaño en una cuenta secundaria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SizingMode {
    /// Mismo tamaño que la principal.
    #[default]
    Same,
    /// Tamaño fijo indicado por el usuario.
    Fixed,
    /// Tamaño de la principal × multiplicador.
    Multiplier,
    /// % de riesgo sobre el balance según la distancia al SL.
    RiskBalance,
    /// % de riesgo sobre la equity según la distancia al SL.
    RiskEquity,
    /// Kelly fraccional con el historial de la principal.
    Kelly,
    /// Lotaje = (Balance / (Precio × contract size)) × multiplicador.
    Notional,
}

impl SizingMode {
    /// Métodos que solo tienen sentido en CFD/Forex.
    pub fn cfd_only(self) -> bool {
        matches!(
            self,
            Self::RiskBalance | Self::RiskEquity | Self::Kelly | Self::Notional
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SizingParams {
    pub mode: SizingMode,
    pub fixed_volume: f64,
    pub multiplier: f64,
    pub risk_percent: f64,
    pub kelly_fraction: f64,
    pub kelly_max_risk_percent: f64,
    pub kelly_lookback: u32,
    pub notional_multiplier: f64,
    /// Método cuando el elegido no se puede aplicar (sin SL, sin historial...).
    pub fallback_mode: SizingMode,
}

impl Default for SizingParams {
    fn default() -> Self {
        Self {
            mode: SizingMode::Same,
            fixed_volume: 1.0,
            multiplier: 1.0,
            risk_percent: 1.0,
            kelly_fraction: 0.5,
            kelly_max_risk_percent: 2.0,
            kelly_lookback: 50,
            notional_multiplier: 1.0,
            fallback_mode: SizingMode::Same,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SymbolFilterMode {
    #[default]
    All,
    Include,
    Exclude,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct FollowerSettings {
    pub sizing: SizingParams,
    /// Ajustar el tamaño entre mini, micro y nano por el valor del punto.
    pub family_scaling: bool,
    pub invert: bool,
    pub copy_sltp: bool,
    pub copy_pending: bool,
    pub max_volume_per_trade: Option<f64>,
    pub max_position_volume: Option<f64>,
    /// Pérdida diaria máxima en la divisa de la cuenta.
    pub daily_loss_limit: Option<f64>,
    pub close_on_daily_loss: bool,
    pub symbol_filter_mode: SymbolFilterMode,
    pub symbols: Vec<String>,
    pub schedule_enabled: bool,
    /// "HH:MM" en hora local del equipo.
    pub schedule_start: String,
    pub schedule_end: String,
    /// 1 = lunes … 7 = domingo.
    pub schedule_days: Vec<u32>,
    pub max_delay_seconds: u32,
    /// Ticks (futuros) o pips (CFD/Forex).
    pub max_deviation: f64,
}

impl Default for FollowerSettings {
    fn default() -> Self {
        Self {
            sizing: SizingParams::default(),
            family_scaling: true,
            invert: false,
            copy_sltp: true,
            copy_pending: true,
            max_volume_per_trade: None,
            max_position_volume: None,
            daily_loss_limit: None,
            close_on_daily_loss: false,
            symbol_filter_mode: SymbolFilterMode::All,
            symbols: Vec::new(),
            schedule_enabled: false,
            schedule_start: "00:00".into(),
            schedule_end: "23:59".into(),
            schedule_days: vec![1, 2, 3, 4, 5],
            max_delay_seconds: 5,
            max_deviation: 10.0,
        }
    }
}

/// Especificación de un símbolo tal como la ve la cuenta (la envían los bridges).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct SymbolSpec {
    pub symbol: String,
    pub market: String,
    pub digits: i64,
    pub tick_size: f64,
    pub tick_value: f64,
    pub contract_size: f64,
    pub volume_min: f64,
    pub volume_max: f64,
    pub volume_step: f64,
    pub bid: f64,
    pub ask: f64,
}

/// Estadísticas para Kelly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KellyStats {
    pub trades: usize,
    pub win_rate: f64,
    pub payoff: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolMapRow {
    pub master_symbol: String,
    pub follower_symbol: String,
    #[serde(default)]
    pub volume_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyAccountView {
    pub id: String,
    pub display_name: String,
    pub login: i64,
    pub server: String,
    pub provider: String,
    pub status: String,
    pub currency: String,
    pub balance: f64,
    pub equity: f64,
    pub online: bool,
    pub last_seen_at: Option<String>,
    pub bridge_version: Option<String>,
    pub role: String,
    pub markets: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyFollowerView {
    pub id: String,
    pub group_id: String,
    pub account_id: String,
    pub enabled: bool,
    pub paused_reason: Option<String>,
    pub paused_at: Option<String>,
    pub settings: FollowerSettings,
    pub symbol_map: Vec<SymbolMapRow>,
    pub open_positions: i64,
    pub open_volume: f64,
    pub pending_orders: i64,
    pub compatibility: String,
    pub daily_pnl: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyGroupView {
    pub id: String,
    pub name: String,
    pub master_account_id: String,
    pub enabled: bool,
    pub followers: Vec<CopyFollowerView>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyOverview {
    pub enabled: bool,
    pub groups: Vec<CopyGroupView>,
    pub accounts: Vec<CopyAccountView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyLogEntry {
    pub id: String,
    pub group_id: String,
    pub group_name: String,
    pub follower_id: String,
    pub account_id: String,
    pub account_name: String,
    pub signal_kind: Option<String>,
    pub action: String,
    pub symbol: String,
    pub side: Option<String>,
    pub volume: f64,
    pub master_volume: Option<f64>,
    pub master_price: Option<f64>,
    pub fill_price: Option<f64>,
    pub fill_volume: Option<f64>,
    pub status: String,
    pub error: Option<String>,
    pub latency_ms: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizePreview {
    pub ok: bool,
    pub volume: Option<f64>,
    pub symbol: Option<String>,
    pub message: String,
}
