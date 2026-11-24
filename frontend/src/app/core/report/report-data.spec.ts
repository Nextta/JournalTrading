import { describe, expect, it } from 'vitest';

import type { CalendarDay, PerformanceCalendar, StatsResponse, Trade } from '../models';
import { buildEquity, buildRangeCalendar, buildTradeRows } from './report-data';

function day(date: string, net: number, rTotal = 0, rTrades = 0): CalendarDay {
  return {
    date,
    net,
    pct: 0,
    trades: 1,
    wins: net > 0 ? 1 : 0,
    losses: net < 0 ? 1 : 0,
    rTotal,
    rTrades,
  };
}

function calendar(days: CalendarDay[], startBalance = 1000): PerformanceCalendar {
  const net = days.reduce((sum, item) => sum + item.net, 0);
  return {
    currency: 'USD',
    startBalance,
    endBalance: startBalance + net,
    days,
    months: [],
    years: [],
    totals: { net, pct: 0, trades: days.length, wins: 0, losses: 0, rTotal: 0, rTrades: 0 },
  };
}

const DAYS = [
  day('2025-12-31', 40, 1, 1),
  day('2026-01-02', 60, 1, 1),
  day('2026-01-15', -20),
  day('2026-02-03', 20, 0.5, 1),
];

describe('buildRangeCalendar', () => {
  it('usa todo el historial y compone los % igual que el backend', () => {
    const result = buildRangeCalendar(calendar(DAYS), null, null);

    expect(result.startBalance).toBe(1000);
    expect(result.days.map((item) => item.key)).toEqual(DAYS.map((item) => item.date));
    expect(result.days[1].pct).toBeCloseTo(5.7692, 3);
    expect(result.heatmap.rows.map((row) => row.year)).toEqual([2026, 2025]);

    const year2026 = result.heatmap.rows[0];
    expect(year2026.total.net).toBe(60);
    expect(year2026.total.pct).toBeCloseTo(5.7692, 3);
    expect(year2026.cells[0]?.pct).toBeCloseTo(3.8462, 3);
    expect(year2026.cells[1]?.net).toBe(20);
    expect(year2026.cells[2]).toBeNull();

    expect(result.heatmap.totals.net).toBe(100);
    expect(result.heatmap.totals.pct).toBeCloseTo(10, 6);
    expect(result.heatmap.hasR).toBe(true);
  });

  it('parte del balance que había antes del inicio del rango', () => {
    const result = buildRangeCalendar(calendar(DAYS), '2026-01-01', '2026-01-31');

    expect(result.startBalance).toBe(1040);
    expect(result.days).toHaveLength(2);
    expect(result.heatmap.rows).toHaveLength(1);
    expect(result.heatmap.totals.net).toBe(40);
    expect(result.heatmap.totals.pct).toBeCloseTo((40 / 1040) * 100, 6);
    expect(result.heatmap.hasR).toBe(true);
  });

  it('devuelve un heatmap vacío si no hay días en el rango', () => {
    const result = buildRangeCalendar(calendar(DAYS), '2027-01-01', null);

    expect(result.startBalance).toBe(1100);
    expect(result.days).toEqual([]);
    expect(result.heatmap.rows).toEqual([]);
    expect(result.heatmap.totals.trades).toBe(0);
    expect(result.heatmap.hasR).toBe(false);
  });
});

describe('buildEquity', () => {
  it('calcula el % acumulado y el drawdown sobre el pico de balance', () => {
    const stats = {
      equity: [
        { index: 0, time: 't1', date: '2026-01-01', cumulative: 100, tradeNet: 100, drawdown: 0, symbol: 'X' },
        { index: 1, time: 't2', date: '2026-01-02', cumulative: -10, tradeNet: -110, drawdown: 110, symbol: 'X' },
      ],
    } as unknown as StatsResponse;

    const points = buildEquity(stats, 1000);

    expect(points[0].pct).toBeCloseTo(10, 6);
    expect(points[0].drawdownPct).toBe(0);
    expect(points[1].balance).toBe(990);
    expect(points[1].drawdownPct).toBeCloseTo(-10, 6);
  });
});

describe('buildTradeRows', () => {
  it('ordena por cierre y calcula el % sobre el balance previo', () => {
    const trade = (id: string, close: string, profitNet: number) =>
      ({
        id,
        positionTicket: Number(id),
        openTimeUtc: close,
        closeTimeUtc: close,
        profitNet,
      }) as Trade;

    const rows = buildTradeRows(
      [trade('2', '2026-01-02T10:00:00Z', -50), trade('1', '2026-01-01T10:00:00Z', 100)],
      1000,
    );

    expect(rows.map((row) => row.trade.id)).toEqual(['1', '2']);
    expect(rows[0].pct).toBeCloseTo(10, 6);
    expect(rows[1].pct).toBeCloseTo((-50 / 1100) * 100, 6);
  });
});
