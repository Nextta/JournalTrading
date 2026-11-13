//! Cálculo del tamaño de cada orden copiada. Funciones puras, sin E/S.

use super::model::{KellyStats, SizingMode, SizingParams, SymbolSpec};

/// Mínimo de operaciones cerradas para aplicar Kelly.
pub const KELLY_MIN_TRADES: usize = 20;

#[derive(Debug, Clone)]
pub struct SizingInput<'a> {
    pub params: &'a SizingParams,
    pub is_futures: bool,
    pub master_volume: f64,
    /// Precio de entrada (mercado) o de la orden pendiente.
    pub entry_price: f64,
    pub stop_loss: Option<f64>,
    pub balance: f64,
    pub equity: f64,
    /// Ajuste mini ↔ micro o de la tabla de símbolos.
    pub volume_ratio: f64,
    pub kelly: Option<KellyStats>,
    /// Especificación del símbolo en la secundaria (si el bridge la envió).
    pub spec: Option<&'a SymbolSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SizingResult {
    pub volume: f64,
    pub mode_used: SizingMode,
    pub note: Option<String>,
}

/// Fracción de Kelly: f = W − (1 − W) / R.
pub fn kelly_fraction(stats: &KellyStats) -> f64 {
    if stats.payoff <= 0.0 {
        return -1.0;
    }
    stats.win_rate - (1.0 - stats.win_rate) / stats.payoff
}

fn risk_volume(risk_money: f64, input: &SizingInput<'_>) -> Result<f64, String> {
    let Some(sl) = input.stop_loss.filter(|value| *value > 0.0) else {
        return Err("la operación no tiene stop loss".into());
    };
    let Some(spec) = input.spec else {
        return Err("falta la especificación del símbolo en la secundaria".into());
    };
    if spec.tick_size <= 0.0 || spec.tick_value <= 0.0 {
        return Err("el símbolo no informa tick size / tick value".into());
    }
    let distance = (input.entry_price - sl).abs();
    if distance <= 0.0 {
        return Err("el stop loss está en el precio de entrada".into());
    }
    let loss_per_lot = distance / spec.tick_size * spec.tick_value;
    if loss_per_lot <= 0.0 {
        return Err("no se pudo calcular la pérdida por lote".into());
    }
    Ok(risk_money / loss_per_lot)
}

fn simple_volume(mode: SizingMode, input: &SizingInput<'_>) -> f64 {
    let params = input.params;
    match mode {
        SizingMode::Fixed => params.fixed_volume,
        SizingMode::Multiplier => input.master_volume * params.multiplier * input.volume_ratio,
        _ => input.master_volume * input.volume_ratio,
    }
}

fn mode_volume(mode: SizingMode, input: &SizingInput<'_>) -> Result<f64, String> {
    let params = input.params;
    match mode {
        SizingMode::Same | SizingMode::Fixed | SizingMode::Multiplier => {
            Ok(simple_volume(mode, input))
        }
        SizingMode::RiskBalance => {
            risk_volume(input.balance * params.risk_percent / 100.0, input)
        }
        SizingMode::RiskEquity => risk_volume(input.equity * params.risk_percent / 100.0, input),
        SizingMode::Kelly => {
            let Some(stats) = input.kelly else {
                return Err("sin historial de la principal".into());
            };
            if stats.trades < KELLY_MIN_TRADES {
                return Err(format!(
                    "Kelly necesita al menos {KELLY_MIN_TRADES} operaciones cerradas ({} disponibles)",
                    stats.trades
                ));
            }
            let fraction = kelly_fraction(&stats);
            if fraction <= 0.0 {
                return Err("Kelly es negativo: el historial no tiene ventaja".into());
            }
            let percent =
                (fraction * params.kelly_fraction * 100.0).min(params.kelly_max_risk_percent);
            risk_volume(input.balance * percent / 100.0, input)
        }
        SizingMode::Notional => {
            let Some(spec) = input.spec else {
                return Err("falta la especificación del símbolo en la secundaria".into());
            };
            if spec.contract_size <= 0.0 || input.entry_price <= 0.0 {
                return Err("faltan el precio o el contract size".into());
            }
            Ok(input.balance / (input.entry_price * spec.contract_size)
                * params.notional_multiplier)
        }
    }
}

/// Volumen bruto (sin redondear) según el método de la secundaria.
pub fn compute(input: &SizingInput<'_>) -> Result<SizingResult, String> {
    let params = input.params;
    let mode = if input.is_futures && params.mode.cfd_only() {
        params.fallback_mode
    } else {
        params.mode
    };
    match mode_volume(mode, input) {
        Ok(volume) => Ok(SizingResult {
            volume,
            mode_used: mode,
            note: (mode != params.mode).then(|| {
                "Método solo disponible en CFD/Forex: se usa el de respaldo".to_string()
            }),
        }),
        Err(reason) if mode.cfd_only() => {
            let fallback = if params.fallback_mode.cfd_only() {
                SizingMode::Same
            } else {
                params.fallback_mode
            };
            Ok(SizingResult {
                volume: simple_volume(fallback, input),
                mode_used: fallback,
                note: Some(format!("{reason}: se usa el método de respaldo")),
            })
        }
        Err(reason) => Err(reason),
    }
}

/// Redondea hacia abajo al paso de volumen y aplica mínimos y máximos.
pub fn normalize(
    volume: f64,
    is_futures: bool,
    spec: Option<&SymbolSpec>,
    max_volume: Option<f64>,
) -> Result<f64, String> {
    if !volume.is_finite() || volume <= 0.0 {
        return Err("el tamaño calculado es 0".into());
    }
    let (step, min, max) = if is_futures {
        (1.0, 1.0, spec.map(|s| s.volume_max).filter(|v| *v > 0.0))
    } else {
        let step = spec.map(|s| s.volume_step).filter(|v| *v > 0.0).unwrap_or(0.01);
        let min = spec.map(|s| s.volume_min).filter(|v| *v > 0.0).unwrap_or(step);
        (step, min, spec.map(|s| s.volume_max).filter(|v| *v > 0.0))
    };
    let mut value = volume;
    if let Some(limit) = max_volume.filter(|v| *v > 0.0) {
        value = value.min(limit);
    }
    if let Some(limit) = max {
        value = value.min(limit);
    }
    let steps = (value / step + 1e-9).floor();
    let rounded = round_to(steps * step, step);
    if rounded + 1e-12 < min {
        return Err(format!(
            "el tamaño calculado ({}) está por debajo del mínimo ({})",
            trim(volume),
            trim(min)
        ));
    }
    Ok(rounded)
}

/// Volumen a cerrar en la secundaria cuando la principal cierra una parte.
pub fn reduce_volume(
    copied: f64,
    fraction: f64,
    is_futures: bool,
    spec: Option<&SymbolSpec>,
) -> Option<f64> {
    if copied <= 0.0 {
        return None;
    }
    if fraction >= 0.999 {
        return Some(copied);
    }
    let raw = copied * fraction.clamp(0.0, 1.0);
    match normalize(raw, is_futures, spec, None) {
        Ok(volume) if copied - volume < 1e-9 => Some(copied),
        Ok(volume) => {
            let remaining = copied - volume;
            let min = if is_futures {
                1.0
            } else {
                spec.map(|s| s.volume_min).filter(|v| *v > 0.0).unwrap_or(0.01)
            };
            // Si lo que queda no llega al mínimo, se cierra todo.
            if remaining + 1e-9 < min {
                Some(copied)
            } else {
                Some(volume)
            }
        }
        Err(_) => None,
    }
}

fn decimals(step: f64) -> i32 {
    let mut decimals = 0;
    let mut value = step;
    while decimals < 8 && (value - value.round()).abs() > 1e-9 {
        value *= 10.0;
        decimals += 1;
    }
    decimals
}

fn round_to(value: f64, step: f64) -> f64 {
    let factor = 10f64.powi(decimals(step));
    (value * factor).round() / factor
}

fn trim(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forex_spec() -> SymbolSpec {
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
            bid: 1.1,
            ask: 1.1001,
        }
    }

    fn input<'a>(params: &'a SizingParams, spec: Option<&'a SymbolSpec>) -> SizingInput<'a> {
        SizingInput {
            params,
            is_futures: false,
            master_volume: 0.5,
            entry_price: 1.1,
            stop_loss: Some(1.098),
            balance: 10_000.0,
            equity: 9_000.0,
            volume_ratio: 1.0,
            kelly: None,
            spec,
        }
    }

    #[test]
    fn same_fixed_and_multiplier() {
        let spec = forex_spec();
        let mut params = SizingParams::default();
        assert_eq!(compute(&input(&params, Some(&spec))).unwrap().volume, 0.5);
        params.mode = SizingMode::Multiplier;
        params.multiplier = 2.0;
        assert_eq!(compute(&input(&params, Some(&spec))).unwrap().volume, 1.0);
        params.mode = SizingMode::Fixed;
        params.fixed_volume = 0.3;
        assert_eq!(compute(&input(&params, Some(&spec))).unwrap().volume, 0.3);
    }

    #[test]
    fn futures_multiplier_with_family_ratio() {
        let params = SizingParams {
            mode: SizingMode::Multiplier,
            multiplier: 2.0,
            ..SizingParams::default()
        };
        let mut data = input(&params, None);
        data.is_futures = true;
        data.master_volume = 1.0;
        data.volume_ratio = 10.0; // NQ → MNQ
        let volume = compute(&data).unwrap().volume;
        assert_eq!(normalize(volume, true, None, None).unwrap(), 20.0);
    }

    #[test]
    fn risk_percent_uses_stop_distance() {
        let spec = forex_spec();
        let params = SizingParams {
            mode: SizingMode::RiskBalance,
            risk_percent: 1.0,
            ..SizingParams::default()
        };
        // 1 % de 10.000 = 100 USD; SL a 20 pips = 200 ticks × 1 USD = 200 USD por lote.
        let volume = compute(&input(&params, Some(&spec))).unwrap().volume;
        assert!((volume - 0.5).abs() < 1e-9);

        let params = SizingParams {
            mode: SizingMode::RiskEquity,
            ..params
        };
        let volume = compute(&input(&params, Some(&spec))).unwrap().volume;
        assert!((volume - 0.45).abs() < 1e-9);
    }

    #[test]
    fn risk_without_stop_uses_fallback() {
        let spec = forex_spec();
        let params = SizingParams {
            mode: SizingMode::RiskBalance,
            fallback_mode: SizingMode::Fixed,
            fixed_volume: 0.1,
            ..SizingParams::default()
        };
        let mut data = input(&params, Some(&spec));
        data.stop_loss = None;
        let result = compute(&data).unwrap();
        assert_eq!(result.mode_used, SizingMode::Fixed);
        assert_eq!(result.volume, 0.1);
        assert!(result.note.is_some());
    }

    #[test]
    fn kelly_fraction_and_cap() {
        let stats = KellyStats {
            trades: 50,
            win_rate: 0.6,
            payoff: 1.5,
        };
        assert!((kelly_fraction(&stats) - (0.6 - 0.4 / 1.5)).abs() < 1e-12);

        let spec = forex_spec();
        let params = SizingParams {
            mode: SizingMode::Kelly,
            kelly_fraction: 0.5,
            kelly_max_risk_percent: 2.0,
            ..SizingParams::default()
        };
        let mut data = input(&params, Some(&spec));
        data.kelly = Some(stats);
        // f ≈ 0.333 → ½ Kelly ≈ 16.7 % → limitado al 2 % = 200 USD → 1 lote.
        let volume = compute(&data).unwrap().volume;
        assert!((volume - 1.0).abs() < 1e-9);
    }

    #[test]
    fn notional_formula() {
        let spec = forex_spec();
        let params = SizingParams {
            mode: SizingMode::Notional,
            notional_multiplier: 2.0,
            ..SizingParams::default()
        };
        // 10.000 / (1.1 × 100.000) × 2 ≈ 0.1818
        let volume = compute(&input(&params, Some(&spec))).unwrap().volume;
        assert_eq!(normalize(volume, false, Some(&spec), None).unwrap(), 0.18);
    }

    #[test]
    fn futures_ignore_cfd_only_modes() {
        let params = SizingParams {
            mode: SizingMode::Kelly,
            fallback_mode: SizingMode::Same,
            ..SizingParams::default()
        };
        let mut data = input(&params, None);
        data.is_futures = true;
        data.master_volume = 3.0;
        assert_eq!(compute(&data).unwrap().volume, 3.0);
    }

    #[test]
    fn normalization_rules() {
        let spec = forex_spec();
        assert_eq!(normalize(0.239, false, Some(&spec), None).unwrap(), 0.23);
        assert_eq!(normalize(5.0, false, Some(&spec), Some(1.5)).unwrap(), 1.5);
        assert!(normalize(0.004, false, Some(&spec), None).is_err());
        assert_eq!(normalize(2.7, true, None, None).unwrap(), 2.0);
        assert!(normalize(0.5, true, None, None).is_err());
    }

    #[test]
    fn partial_close_keeps_proportion() {
        let spec = forex_spec();
        assert_eq!(reduce_volume(1.0, 0.5, false, Some(&spec)), Some(0.5));
        assert_eq!(reduce_volume(0.02, 0.5, false, Some(&spec)), Some(0.01));
        assert_eq!(reduce_volume(0.02, 0.4, false, Some(&spec)), None);
        assert_eq!(reduce_volume(3.0, 0.5, true, None), Some(1.0));
        assert_eq!(reduce_volume(1.0, 0.5, true, None), None);
        assert_eq!(reduce_volume(2.0, 1.0, true, None), Some(2.0));
    }
}
