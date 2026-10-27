import { describe, expect, it } from 'vitest';

import type { CandleBar, Deal } from './models';
import { barIndexAt, formatLots, parseServerTime, resolveExitReason } from './trade-chart';

const bars: CandleBar[] = [
  {
    time: Date.UTC(2026, 9, 23, 10, 0) / 1000,
    open: 1.085,
    high: 1.086,
    low: 1.084,
    close: 1.0855,
    volume: 100,
    spread: 10,
  },
  {
    time: Date.UTC(2026, 9, 23, 10, 15) / 1000,
    open: 1.0855,
    high: 1.087,
    low: 1.085,
    close: 1.0865,
    volume: 120,
    spread: 10,
  },
];

function deal(overrides: Partial<Deal> = {}): Deal {
  return {
    id: 'deal',
    tradeId: 'trade',
    accountId: 'account',
    dealTicket: 1,
    entry: 1,
    type: 0,
    volume: 0.1,
    price: 1.087,
    profit: 20,
    commission: 0,
    swap: 0,
    timeServer: '2026-10-23T12:30:00',
    timeUtc: '2026-10-23T10:30:00Z',
    comment: '',
    reason: -1,
    ...overrides,
  };
}

describe('trade chart helpers', () => {
  it('parses broker server times as UTC', () => {
    expect(parseServerTime('2026-10-23T10:07:00')).toBe(Date.UTC(2026, 9, 23, 10, 7));
    expect(parseServerTime('')).toBeNull();
    expect(parseServerTime(null)).toBeNull();
  });

  it('finds the candle containing the trade time', () => {
    expect(barIndexAt(bars, Date.UTC(2026, 9, 23, 10, 7), 'M15')).toBe(0);
    expect(barIndexAt(bars, Date.UTC(2026, 9, 23, 10, 15), 'M15')).toBe(1);
    expect(barIndexAt(bars, Date.UTC(2026, 9, 23, 11, 0), 'M15')).toBe(1);
  });

  it('uses the MT5 deal reason for SL and TP exits', () => {
    expect(resolveExitReason([deal({ reason: 4 })], 1.084, 1.084, 1.1)).toBe('SL');
    expect(resolveExitReason([deal({ reason: 5 })], 1.1, 1.084, 1.1)).toBe('TP');
    expect(resolveExitReason([deal({ reason: 3 })], 1.09, 1.084, 1.1)).toBe('Cierre');
  });

  it('falls back to SL/TP prices when the deal reason is missing', () => {
    expect(resolveExitReason([deal()], 1.084, 1.084, 1.1)).toBe('SL');
    expect(resolveExitReason([deal()], 1.1, 1.084, 1.1)).toBe('TP');
    expect(resolveExitReason([], 1.09, 1.084, 1.1)).toBe('Cierre');
  });

  it('formats lot sizes with two decimals', () => {
    expect(formatLots(0.1)).toBe('0.10');
    expect(formatLots(1.5)).toBe('1.50');
  });
});
