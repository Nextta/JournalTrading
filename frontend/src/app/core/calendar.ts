import { dateToIso } from './format';
import type { CalendarDay, CalendarPeriod } from './models';

export interface CalendarCell {
  date: string;
  day: CalendarDay | null;
  inMonth: boolean;
}

export interface CalendarWeek {
  start: string;
  end: string;
  days: CalendarCell[];
  total: CalendarPeriod | null;
}

export function addMonths(month: string, delta: number): string {
  const [year, monthNumber] = month.split('-').map(Number);
  const date = new Date(year, (monthNumber || 1) - 1 + delta, 1, 12, 0, 0);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`;
}

export function buildMonthWeeks(month: string, days: CalendarDay[]): CalendarWeek[] {
  const [year, monthNumber] = month.split('-').map(Number);
  if (!year || !monthNumber) return [];

  const first = new Date(year, monthNumber - 1, 1, 12, 0, 0);
  const last = new Date(year, monthNumber, 0, 12, 0, 0);
  const offset = (first.getDay() + 6) % 7;
  const start = new Date(first);
  start.setDate(first.getDate() - offset);
  const end = new Date(last);
  end.setDate(last.getDate() + (6 - ((last.getDay() + 6) % 7)));

  const byDate = new Map(days.map((day) => [day.date, day]));
  const weeks: CalendarWeek[] = [];
  const cursor = new Date(start);

  while (cursor <= end) {
    const cells: CalendarCell[] = [];
    for (let index = 0; index < 7; index += 1) {
      const date = dateToIso(cursor);
      cells.push({
        date,
        day: byDate.get(date) ?? null,
        inMonth: date.slice(0, 7) === month,
      });
      cursor.setDate(cursor.getDate() + 1);
    }
    weeks.push({
      start: cells[0].date,
      end: cells[6].date,
      days: cells,
      total: summarizePeriods(
        cells.map((cell) => cell.day).filter((day): day is CalendarDay => day !== null),
      ),
    });
  }

  return weeks;
}

export function summarizePeriods(periods: CalendarPeriod[]): CalendarPeriod | null {
  const active = periods.filter((period) => period.trades > 0);
  if (!active.length) return null;

  let factor = 1;
  for (const period of active) factor *= 1 + period.pct / 100;

  return {
    net: active.reduce((total, period) => total + period.net, 0),
    pct: (factor - 1) * 100,
    trades: active.reduce((total, period) => total + period.trades, 0),
    wins: active.reduce((total, period) => total + period.wins, 0),
    losses: active.reduce((total, period) => total + period.losses, 0),
    rTotal: active.reduce((total, period) => total + period.rTotal, 0),
    rTrades: active.reduce((total, period) => total + period.rTrades, 0),
  };
}

export function hasFullR(period: CalendarPeriod | null | undefined): boolean {
  return !!period && period.trades > 0 && period.rTrades === period.trades;
}

export function monthNumber(value: string): number {
  return Number(value.slice(5, 7));
}
