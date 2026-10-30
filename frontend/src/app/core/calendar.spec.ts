import { describe, expect, it } from 'vitest';

import { addMonths, buildMonthWeeks, hasFullR, summarizePeriods } from './calendar';
import type { CalendarDay, CalendarPeriod } from './models';

function day(date: string, net: number, pct: number, trades = 1): CalendarDay {
  return {
    date,
    net,
    pct,
    trades,
    wins: net > 0 ? 1 : 0,
    losses: net < 0 ? 1 : 0,
    rTotal: 0,
    rTrades: 0,
  };
}

function period(pct: number, trades = 1): CalendarPeriod {
  return {
    net: pct,
    pct,
    trades,
    wins: pct > 0 ? 1 : 0,
    losses: pct < 0 ? 1 : 0,
    rTotal: 0,
    rTrades: 0,
  };
}

describe('calendar helpers', () => {
  it('moves between months across year boundaries', () => {
    expect(addMonths('2026-01', -1)).toBe('2025-12');
    expect(addMonths('2026-12', 1)).toBe('2027-01');
    expect(addMonths('2026-10', 3)).toBe('2027-01');
  });

  it('builds Monday-to-Sunday weeks including neighbouring days', () => {
    const weeks = buildMonthWeeks('2026-10', [day('2026-09-30', 50, 1), day('2026-10-01', 20, 1)]);

    expect(weeks).toHaveLength(5);
    expect(weeks[0].start).toBe('2026-09-28');
    expect(weeks[0].end).toBe('2026-10-04');
    expect(weeks[0].days[0].date).toBe('2026-09-28');
    expect(weeks[0].days[2].inMonth).toBe(false);
    expect(weeks[0].days[3].inMonth).toBe(true);
    expect(weeks[0].total?.net).toBe(70);
    expect(weeks[0].total?.trades).toBe(2);
    expect(weeks[4].end).toBe('2026-11-01');
  });

  it('compounds percentages and ignores empty periods', () => {
    const total = summarizePeriods([period(10), period(-10), period(0, 0)]);
    expect(total).not.toBeNull();
    expect(total?.pct).toBeCloseTo(-1, 10);
    expect(total?.trades).toBe(2);
    expect(summarizePeriods([])).toBeNull();
    expect(summarizePeriods([period(0, 0)])).toBeNull();
  });

  it('detects full R coverage', () => {
    expect(hasFullR(null)).toBe(false);
    expect(hasFullR({ ...period(1), rTrades: 1 })).toBe(true);
    expect(hasFullR({ ...period(1, 2), rTrades: 1 })).toBe(false);
    expect(hasFullR({ ...period(0, 0), rTrades: 0 })).toBe(false);
  });
});
