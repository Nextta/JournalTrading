import { monthNumber, summarizePeriods } from '../calendar';
import type {
  Account,
  BreakdownRow,
  CalendarPeriod,
  PerformanceCalendar,
  StatsResponse,
  Trade,
} from '../models';

/** Secciones opcionales del reporte (todas activas por defecto). */
export type ReportSection = 'summary' | 'equity' | 'heatmap' | 'daily' | 'breakdowns' | 'timing';

export const REPORT_SECTIONS: ReadonlyArray<{ key: ReportSection; label: string }> = [
  { key: 'summary', label: 'Estadísticas principales' },
  { key: 'equity', label: 'Curva de retorno y drawdown' },
  { key: 'heatmap', label: 'Resultados por meses y años' },
  { key: 'daily', label: 'P&L diario y distribución de R' },
  { key: 'breakdowns', label: 'Desglose por símbolo y estrategia' },
  { key: 'timing', label: 'Rendimiento por día de la semana y hora' },
];

export interface ReportOptions {
  /** `null` = desde el inicio del historial. */
  from: string | null;
  /** `null` = hasta la última operación. */
  to: string | null;
  sections: ReportSection[];
  includeTrades: boolean;
  /** Oculta login, servidor, balances e importes en dinero. */
  hideSensitive: boolean;
}

export interface ReportPeriod extends CalendarPeriod {
  key: string;
}

export interface ReportHeatmapRow {
  year: number;
  cells: Array<CalendarPeriod | null>;
  total: CalendarPeriod;
}

export interface ReportHeatmap {
  rows: ReportHeatmapRow[];
  /** Agregado de cada mes del año a lo largo de todos los años del periodo. */
  monthTotals: Array<CalendarPeriod | null>;
  totals: CalendarPeriod;
  /** Hay operaciones con R en el periodo (se muestra la tabla en R). */
  hasR: boolean;
}

export interface ReportEquityPoint {
  time: string;
  date: string;
  cumulative: number;
  pct: number;
  balance: number;
  drawdownPct: number;
}

export interface ReportTradeRow {
  trade: Trade;
  pct: number;
}

export interface ReportData {
  account: Account;
  options: ReportOptions;
  currency: string;
  generatedAt: Date;
  /** Primera y última fecha efectivas del periodo (con operaciones o elegidas). */
  periodFrom: string | null;
  periodTo: string | null;
  startBalance: number;
  endBalance: number;
  returnPct: number;
  maxDrawdownBalancePct: number;
  bestTradePct: number;
  worstTradePct: number;
  stats: StatsResponse;
  equity: ReportEquityPoint[];
  days: ReportPeriod[];
  heatmap: ReportHeatmap;
  trades: ReportTradeRow[];
}

function emptyPeriod(): CalendarPeriod {
  return { net: 0, pct: 0, trades: 0, wins: 0, losses: 0, rTotal: 0, rTrades: 0 };
}

function pctOf(net: number, base: number): number {
  return Math.abs(base) > 0.01 ? (net / base) * 100 : 0;
}

function inRange(date: string, from: string | null, to: string | null): boolean {
  if (from && date < from) return false;
  if (to && date > to) return false;
  return true;
}

function tradeDate(trade: Trade): string {
  return (trade.closeTimeServer ?? trade.openTimeServer ?? '').slice(0, 10);
}

/**
 * Recalcula el calendario para un rango: el balance inicial es el de la cuenta
 * justo antes del primer día del rango y los % se componen igual que en el
 * backend (cada mes/año sobre el balance que había al empezar).
 */
export function buildRangeCalendar(
  calendar: PerformanceCalendar,
  from: string | null,
  to: string | null,
): { startBalance: number; days: ReportPeriod[]; heatmap: ReportHeatmap } {
  const sorted = [...calendar.days].sort((a, b) => a.date.localeCompare(b.date));
  let startBalance = calendar.startBalance;
  for (const day of sorted) {
    if (from && day.date < from) startBalance += day.net;
  }

  const days: ReportPeriod[] = [];
  const months = new Map<string, { period: CalendarPeriod; base: number }>();
  const years = new Map<number, { period: CalendarPeriod; base: number }>();
  const totals = emptyPeriod();
  let running = startBalance;

  const merge = (target: CalendarPeriod, source: CalendarPeriod) => {
    target.net += source.net;
    target.trades += source.trades;
    target.wins += source.wins;
    target.losses += source.losses;
    target.rTotal += source.rTotal;
    target.rTrades += source.rTrades;
  };

  for (const day of sorted) {
    if (!inRange(day.date, from, to)) continue;
    const period: CalendarPeriod = {
      net: day.net,
      pct: pctOf(day.net, running),
      trades: day.trades,
      wins: day.wins,
      losses: day.losses,
      rTotal: day.rTotal,
      rTrades: day.rTrades,
    };
    days.push({ key: day.date, ...period });

    const monthKey = day.date.slice(0, 7);
    const month = months.get(monthKey) ?? { period: emptyPeriod(), base: running };
    merge(month.period, period);
    months.set(monthKey, month);

    const yearKey = Number(day.date.slice(0, 4));
    const year = years.get(yearKey) ?? { period: emptyPeriod(), base: running };
    merge(year.period, period);
    years.set(yearKey, year);

    merge(totals, period);
    running += day.net;
  }

  const monthPeriods: ReportPeriod[] = [...months.entries()].map(([key, value]) => ({
    key,
    ...value.period,
    pct: pctOf(value.period.net, value.base),
  }));

  const rows: ReportHeatmapRow[] = [...years.entries()]
    .sort((a, b) => b[0] - a[0])
    .map(([year, value]) => ({
      year,
      cells: Array.from(
        { length: 12 },
        (_, index) =>
          monthPeriods.find(
            (month) => month.key.startsWith(`${year}-`) && monthNumber(month.key) === index + 1,
          ) ?? null,
      ),
      total: { ...value.period, pct: pctOf(value.period.net, value.base) },
    }));

  const monthTotals = Array.from({ length: 12 }, (_, index) =>
    summarizePeriods(monthPeriods.filter((month) => monthNumber(month.key) === index + 1)),
  );

  const totalPeriod = { ...totals, pct: pctOf(totals.net, startBalance) };

  return {
    startBalance,
    days,
    heatmap: {
      rows,
      monthTotals,
      totals: totalPeriod,
      hasR: totalPeriod.rTrades > 0,
    },
  };
}

/** Curva de capital en dinero y en %, con drawdown sobre el pico de balance. */
export function buildEquity(stats: StatsResponse, startBalance: number): ReportEquityPoint[] {
  let peak = startBalance;
  return stats.equity.map((point) => {
    const balance = startBalance + point.cumulative;
    peak = Math.max(peak, balance);
    return {
      time: point.time,
      date: point.date,
      cumulative: point.cumulative,
      pct: pctOf(point.cumulative, startBalance),
      balance,
      drawdownPct: peak > 0 ? ((balance - peak) / peak) * 100 : 0,
    };
  });
}

/** % de cada operación sobre el balance que había justo antes de cerrarla. */
export function buildTradeRows(trades: Trade[], startBalance: number): ReportTradeRow[] {
  const sorted = [...trades].sort((a, b) => {
    const left = a.closeTimeUtc ?? a.openTimeUtc;
    const right = b.closeTimeUtc ?? b.openTimeUtc;
    return left.localeCompare(right) || a.positionTicket - b.positionTicket;
  });
  let running = startBalance;
  return sorted.map((trade) => {
    const pct = pctOf(trade.profitNet, running);
    running += trade.profitNet;
    return { trade, pct };
  });
}

/** % del beneficio de un desglose sobre el balance inicial del periodo. */
export function breakdownPct(row: BreakdownRow, startBalance: number): number {
  return pctOf(row.net, startBalance);
}

export function buildReportData(input: {
  account: Account;
  options: ReportOptions;
  stats: StatsResponse;
  calendar: PerformanceCalendar;
  trades: Trade[];
  generatedAt?: Date;
}): ReportData {
  const { account, options, stats, calendar } = input;
  const range = buildRangeCalendar(calendar, options.from, options.to);
  const trades = input.trades.filter(
    (trade) => trade.state === 'closed' && inRange(tradeDate(trade), options.from, options.to),
  );
  const tradeRows = buildTradeRows(trades, range.startBalance);
  const equity = buildEquity(stats, range.startBalance);
  const net = stats.summary.netProfit;

  const tradePcts = tradeRows.map((row) => row.pct);
  const firstDay = range.days[0]?.key ?? null;
  const lastDay = range.days[range.days.length - 1]?.key ?? null;

  return {
    account,
    options,
    currency: calendar.currency || account.currency,
    generatedAt: input.generatedAt ?? new Date(),
    periodFrom: options.from ?? firstDay,
    periodTo: options.to ?? lastDay,
    startBalance: range.startBalance,
    endBalance: range.startBalance + net,
    returnPct: pctOf(net, range.startBalance),
    maxDrawdownBalancePct: equity.reduce((min, point) => Math.min(min, point.drawdownPct), 0),
    bestTradePct: tradePcts.length ? Math.max(...tradePcts) : 0,
    worstTradePct: tradePcts.length ? Math.min(...tradePcts) : 0,
    stats,
    equity,
    days: range.days,
    heatmap: range.heatmap,
    trades: tradeRows,
  };
}
