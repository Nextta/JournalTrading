//! Emparejamiento de símbolos entre plataformas.
//!
//! - Futuros: se normalizan a raíz + mes + año (`NQZ6@CME`, `NQ 12-26` y `NQZ26`
//!   son el mismo contrato) y se generan los nombres candidatos en el formato de
//!   la plataforma de destino. Mini, micro y nano de la misma familia se pueden
//!   copiar entre sí ajustando el tamaño por el valor del punto.
//! - CFD/Forex: mismo nombre o nombre con sufijo del bróker (`EURUSD` → `EURUSD.m`).

use chrono::Datelike;

pub const MARKET_FUTURES: &str = "futures";
pub const MARKET_CFD: &str = "cfd_forex";
#[allow(dead_code)]
pub const MARKET_OTHER: &str = "other";

/// Contratos conocidos: (raíz, familia, valor del punto en USD, mercado).
const FUTURES: &[(&str, &str, f64, &str)] = &[
    ("NQ", "NQ", 20.0, "CME"),
    ("MNQ", "NQ", 2.0, "CME"),
    ("ES", "ES", 50.0, "CME"),
    ("MES", "ES", 5.0, "CME"),
    ("YM", "YM", 5.0, "CBOT"),
    ("MYM", "YM", 0.5, "CBOT"),
    ("RTY", "RTY", 50.0, "CME"),
    ("M2K", "RTY", 5.0, "CME"),
    ("EMD", "EMD", 100.0, "CME"),
    ("NKD", "NKD", 5.0, "CME"),
    ("CL", "CL", 1000.0, "NYMEX"),
    ("QM", "CL", 500.0, "NYMEX"),
    ("MCL", "CL", 100.0, "NYMEX"),
    ("NG", "NG", 10000.0, "NYMEX"),
    ("QG", "NG", 2500.0, "NYMEX"),
    ("MNG", "NG", 1000.0, "NYMEX"),
    ("RB", "RB", 42000.0, "NYMEX"),
    ("HO", "HO", 42000.0, "NYMEX"),
    ("GC", "GC", 100.0, "COMEX"),
    ("MGC", "GC", 10.0, "COMEX"),
    ("1OZ", "GC", 1.0, "COMEX"),
    ("SI", "SI", 5000.0, "COMEX"),
    ("SIL", "SI", 1000.0, "COMEX"),
    ("HG", "HG", 25000.0, "COMEX"),
    ("MHG", "HG", 2500.0, "COMEX"),
    ("PL", "PL", 50.0, "NYMEX"),
    ("6E", "6E", 125000.0, "CME"),
    ("M6E", "6E", 12500.0, "CME"),
    ("6B", "6B", 62500.0, "CME"),
    ("M6B", "6B", 6250.0, "CME"),
    ("6A", "6A", 100000.0, "CME"),
    ("M6A", "6A", 10000.0, "CME"),
    ("6J", "6J", 12500000.0, "CME"),
    ("6C", "6C", 100000.0, "CME"),
    ("6S", "6S", 125000.0, "CME"),
    ("BTC", "BTC", 5.0, "CME"),
    ("MBT", "BTC", 0.1, "CME"),
    ("ETH", "ETH", 50.0, "CME"),
    ("MET", "ETH", 0.1, "CME"),
    ("ZB", "ZB", 1000.0, "CBOT"),
    ("UB", "UB", 1000.0, "CBOT"),
    ("ZN", "ZN", 1000.0, "CBOT"),
    ("ZF", "ZF", 1000.0, "CBOT"),
    ("ZT", "ZT", 2000.0, "CBOT"),
    ("ZC", "ZC", 50.0, "CBOT"),
    ("XC", "ZC", 10.0, "CBOT"),
    ("ZS", "ZS", 50.0, "CBOT"),
    ("XK", "ZS", 10.0, "CBOT"),
    ("ZW", "ZW", 50.0, "CBOT"),
    ("XW", "ZW", 10.0, "CBOT"),
    ("LE", "LE", 400.0, "CME"),
    ("HE", "HE", 400.0, "CME"),
];

const MONTH_CODES: [char; 12] = ['F', 'G', 'H', 'J', 'K', 'M', 'N', 'Q', 'U', 'V', 'X', 'Z'];

#[derive(Debug, Clone, PartialEq)]
pub struct FuturesKey {
    pub root: String,
    pub month: u32,
    pub year: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuturesInfo {
    pub family: &'static str,
    pub point_value: f64,
    pub exchange: &'static str,
}

pub fn futures_info(root: &str) -> Option<FuturesInfo> {
    FUTURES
        .iter()
        .find(|(r, ..)| r.eq_ignore_ascii_case(root))
        .map(|(_, family, point_value, exchange)| FuturesInfo {
            family,
            point_value: *point_value,
            exchange,
        })
}

fn month_from_code(code: char) -> Option<u32> {
    MONTH_CODES
        .iter()
        .position(|c| *c == code.to_ascii_uppercase())
        .map(|index| index as u32 + 1)
}

fn month_code(month: u32) -> char {
    MONTH_CODES[((month.clamp(1, 12)) - 1) as usize]
}

fn expand_year(digits: &str) -> Option<i32> {
    let value: i32 = digits.parse().ok()?;
    match digits.len() {
        1 => {
            let current = chrono::Utc::now().year();
            let mut year = current / 10 * 10 + value;
            if year < current - 1 {
                year += 10;
            }
            Some(year)
        }
        2 => Some(2000 + value),
        4 => Some(value),
        _ => None,
    }
}

/// Interpreta un símbolo de futuros de cualquier plataforma.
pub fn parse_futures(symbol: &str) -> Option<FuturesKey> {
    let symbol = symbol.trim().to_ascii_uppercase();
    let base = symbol.split('@').next().unwrap_or("").trim();
    if base.is_empty() {
        return None;
    }

    // NinjaTrader: "NQ 12-26"
    if let Some((root, expiry)) = base.split_once(' ') {
        let root = root.trim();
        if let Some((month, year)) = expiry.trim().split_once('-') {
            let month: u32 = month.trim().parse().ok()?;
            let year = expand_year(year.trim())?;
            if (1..=12).contains(&month) && !root.is_empty() {
                return Some(FuturesKey {
                    root: root.to_string(),
                    month,
                    year,
                });
            }
        }
        return None;
    }

    // Código de mes + año al final: "NQZ6", "NQZ26", "NQ_Z6", "NQ.Z26", "NQ-Z2026".
    let chars: Vec<char> = base.chars().collect();
    let mut digits_start = chars.len();
    while digits_start > 0 && chars[digits_start - 1].is_ascii_digit() {
        digits_start -= 1;
    }
    let digits: String = chars[digits_start..].iter().collect();
    if digits.is_empty() || digits.len() == 3 || digits.len() > 4 || digits_start == 0 {
        return None;
    }
    let code = chars[digits_start - 1];
    let month = month_from_code(code)?;
    let mut root: String = chars[..digits_start - 1].iter().collect();
    while root.ends_with(['_', '.', '-', ' ']) {
        root.pop();
    }
    if root.is_empty() || root.len() > 6 || !root.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let year = expand_year(&digits)?;
    Some(FuturesKey { root, month, year })
}

/// Nombres candidatos del contrato en la plataforma de destino, en orden de preferencia.
pub fn futures_candidates(key: &FuturesKey, provider: &str) -> Vec<String> {
    let root = &key.root;
    let code = month_code(key.month);
    let y1 = key.year.rem_euclid(10);
    let y2 = key.year.rem_euclid(100);
    let nt = format!("{root} {:02}-{:02}", key.month, y2);
    let mut list = match provider {
        "ninjatrader" => vec![nt],
        "atas" => {
            let exchange = futures_info(root).map(|info| info.exchange).unwrap_or("CME");
            vec![
                format!("{root}{code}{y1}@{exchange}"),
                format!("{root}{code}{y1}"),
                format!("{root}{code}{y2:02}"),
            ]
        }
        _ => vec![
            format!("{root}{code}{y2:02}"),
            format!("{root}{code}{y1}"),
            format!("{root}_{code}{y1}"),
            format!("{root}_{code}{y2:02}"),
            format!("{root}-{code}{y2:02}"),
            format!("{root}.{code}{y2:02}"),
            nt,
        ],
    };
    list.dedup();
    list
}

fn alnum_upper(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Busca entre los símbolos que conoce la secundaria uno equivalente
/// (mismo nombre, o con prefijo/sufijo del bróker).
pub fn match_cfd_symbol(master: &str, available: &[String]) -> Vec<String> {
    let core = alnum_upper(master);
    if core.is_empty() {
        return Vec::new();
    }
    let mut exact = Vec::new();
    let mut fuzzy = Vec::new();
    for symbol in available {
        let candidate = alnum_upper(symbol);
        if candidate == core {
            exact.push(symbol.clone());
        } else if candidate.len() <= core.len() + 6
            && (candidate.starts_with(&core) || candidate.ends_with(&core))
        {
            fuzzy.push(symbol.clone());
        }
    }
    fuzzy.sort_by_key(|value| value.len());
    exact.extend(fuzzy);
    exact
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    /// Nombres a probar en la secundaria, en orden.
    pub candidates: Vec<String>,
    /// Multiplicador del volumen por diferencia de tamaño de contrato (mini → micro).
    pub volume_ratio: f64,
}

pub struct MapEntry<'a> {
    pub master_symbol: &'a str,
    pub follower_symbol: &'a str,
    pub volume_ratio: Option<f64>,
}

/// Resuelve el símbolo de la secundaria para una señal de la principal.
pub fn resolve(
    master_symbol: &str,
    market: &str,
    follower_provider: &str,
    map: &[MapEntry<'_>],
    available: &[String],
    family_scaling: bool,
) -> Option<Resolution> {
    // 1. Equivalencia exacta de la tabla manual.
    if let Some(entry) = map
        .iter()
        .find(|entry| entry.master_symbol.trim().eq_ignore_ascii_case(master_symbol.trim()))
    {
        return Some(Resolution {
            candidates: vec![entry.follower_symbol.trim().to_string()],
            volume_ratio: entry.volume_ratio.filter(|v| *v > 0.0).unwrap_or(1.0),
        });
    }

    if market == MARKET_FUTURES {
        let key = parse_futures(master_symbol)?;
        // 2. Equivalencia por raíz (NQ → MNQ), conservando el vencimiento.
        let root_entry = map
            .iter()
            .find(|entry| entry.master_symbol.trim().eq_ignore_ascii_case(&key.root));
        let target_root = root_entry
            .map(|entry| entry.follower_symbol.trim().to_ascii_uppercase())
            .unwrap_or_else(|| key.root.clone());
        let target = FuturesKey {
            root: target_root.clone(),
            ..key.clone()
        };
        let ratio = match root_entry.and_then(|entry| entry.volume_ratio) {
            Some(value) if value > 0.0 => value,
            _ if family_scaling => match (futures_info(&key.root), futures_info(&target_root)) {
                (Some(a), Some(b)) if a.family == b.family && b.point_value > 0.0 => {
                    a.point_value / b.point_value
                }
                _ => 1.0,
            },
            _ => 1.0,
        };
        let mut candidates: Vec<String> = available
            .iter()
            .filter(|symbol| parse_futures(symbol).as_ref() == Some(&target))
            .cloned()
            .collect();
        for candidate in futures_candidates(&target, follower_provider) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        return Some(Resolution {
            candidates,
            volume_ratio: ratio,
        });
    }

    let mut candidates = match_cfd_symbol(master_symbol, available);
    if !candidates
        .iter()
        .any(|value| value.eq_ignore_ascii_case(master_symbol))
    {
        candidates.push(master_symbol.trim().to_string());
    }
    Some(Resolution {
        candidates,
        volume_ratio: 1.0,
    })
}

/// ¿Puede una secundaria de este proveedor operar este tipo de mercado?
pub fn compatible(market: &str, master_provider: &str, follower_provider: &str) -> bool {
    match market {
        MARKET_FUTURES => true,
        MARKET_CFD => follower_provider == "mt5",
        _ => master_provider == follower_provider,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_platform_format() {
        let expected = FuturesKey {
            root: "NQ".into(),
            month: 12,
            year: 2026,
        };
        assert_eq!(parse_futures("NQ 12-26"), Some(expected.clone()));
        assert_eq!(parse_futures("NQZ6@CME"), Some(expected.clone()));
        assert_eq!(parse_futures("NQZ26"), Some(expected.clone()));
        assert_eq!(parse_futures("nq_z6"), Some(expected.clone()));
        assert_eq!(parse_futures("NQ.Z2026"), Some(expected));
        assert_eq!(parse_futures("M2K H27").map(|k| k.root), None);
        assert_eq!(parse_futures("M2KH7").map(|k| (k.root, k.month)), Some(("M2K".into(), 3)));
        assert_eq!(parse_futures("EURUSD"), None);
        assert_eq!(parse_futures("US500"), None);
    }

    #[test]
    fn builds_candidates_per_platform() {
        let key = parse_futures("NQ 12-26").unwrap();
        assert_eq!(futures_candidates(&key, "ninjatrader"), vec!["NQ 12-26"]);
        assert_eq!(futures_candidates(&key, "atas")[0], "NQZ6@CME");
        assert_eq!(futures_candidates(&key, "mt5")[0], "NQZ26");
    }

    #[test]
    fn scales_between_mini_and_micro() {
        let map = [MapEntry {
            master_symbol: "NQ",
            follower_symbol: "MNQ",
            volume_ratio: None,
        }];
        let result = resolve("NQZ6@CME", MARKET_FUTURES, "ninjatrader", &map, &[], true).unwrap();
        assert_eq!(result.candidates, vec!["MNQ 12-26"]);
        assert!((result.volume_ratio - 10.0).abs() < 1e-9);

        let result = resolve("NQZ6@CME", MARKET_FUTURES, "ninjatrader", &map, &[], false).unwrap();
        assert!((result.volume_ratio - 1.0).abs() < 1e-9);
    }

    #[test]
    fn prefers_symbols_reported_by_the_follower() {
        let available = vec!["NQ_Z6".to_string(), "EURUSD.m".to_string(), "EURUSDx".to_string()];
        let result = resolve("NQ 12-26", MARKET_FUTURES, "mt5", &[], &available, true).unwrap();
        assert_eq!(result.candidates[0], "NQ_Z6");
        let result = resolve("EURUSD", MARKET_CFD, "mt5", &[], &available, true).unwrap();
        assert!(result.candidates.contains(&"EURUSD.m".to_string()));
        assert!(result.candidates.contains(&"EURUSDx".to_string()));
        assert_eq!(result.candidates.last().unwrap(), "EURUSD");
    }

    #[test]
    fn checks_market_compatibility() {
        assert!(compatible(MARKET_FUTURES, "mt5", "atas"));
        assert!(!compatible(MARKET_CFD, "mt5", "ninjatrader"));
        assert!(compatible(MARKET_CFD, "mt5", "mt5"));
        assert!(!compatible(MARKET_OTHER, "atas", "mt5"));
    }
}
