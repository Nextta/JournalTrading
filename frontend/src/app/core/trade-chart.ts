import type { CandleBar, Deal } from './models';

const TIMEFRAME_SECONDS: Record<string, number> = {
  M1: 60,
  M5: 300,
  M15: 900,
  M30: 1800,
  H1: 3600,
  H4: 14400,
  D1: 86400,
  W1: 604800,
};

const DEAL_REASON_SL = 4;
const DEAL_REASON_TP = 5;

export function parseServerTime(value: string | null | undefined): number | null {
  if (!value) return null;
  const text = value.trim();
  if (!text) return null;
  const normalized = text.includes('T') ? text : text.replace(' ', 'T');
  const withZone = /(?:Z|[+-]\d{2}:?\d{2})$/.test(normalized) ? normalized : `${normalized}Z`;
  const parsed = Date.parse(withZone);
  return Number.isFinite(parsed) ? parsed : null;
}

export function barIndexAt(
  bars: CandleBar[],
  timeMs: number | null,
  timeframe: string,
): number | null {
  if (!bars.length || timeMs === null) return null;
  const step = (TIMEFRAME_SECONDS[timeframe] ?? 0) * 1000;
  if (step > 0) {
    for (let index = 0; index < bars.length; index += 1) {
      const start = bars[index].time * 1000;
      if (timeMs >= start && timeMs < start + step) return index;
    }
  }
  let nearest = 0;
  let distance = Number.POSITIVE_INFINITY;
  for (let index = 0; index < bars.length; index += 1) {
    const current = Math.abs(bars[index].time * 1000 - timeMs);
    if (current < distance) {
      distance = current;
      nearest = index;
    }
  }
  return nearest;
}

export function resolveExitReason(
  deals: Deal[],
  closePrice: number | null,
  sl: number | null,
  tp: number | null,
): string {
  const closingDeals = deals.filter((deal) => deal.entry === 1 || deal.entry === 3);
  const lastClose = closingDeals[closingDeals.length - 1];
  if (lastClose?.reason === DEAL_REASON_SL) return 'SL';
  if (lastClose?.reason === DEAL_REASON_TP) return 'TP';
  if (closePrice !== null) {
    if (sl !== null && sl > 0 && samePrice(closePrice, sl)) return 'SL';
    if (tp !== null && tp > 0 && samePrice(closePrice, tp)) return 'TP';
  }
  return 'Cierre';
}

export function formatLots(value: number): string {
  return value.toFixed(2);
}

function samePrice(a: number, b: number): boolean {
  return Math.abs(a - b) <= Math.max(1e-8, Math.abs(b) * 1e-5);
}
