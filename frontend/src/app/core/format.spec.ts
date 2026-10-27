import { describe, expect, it } from 'vitest';

import {
  addDays,
  clamp,
  formatDateNumeric,
  formatDuration,
  formatPercent,
  formatR,
  initials,
  monthLabel,
  pad,
  sideLabel,
} from './format';

describe('format helpers', () => {
  it('pads numbers to two digits', () => {
    expect(pad(3)).toBe('03');
    expect(pad(12)).toBe('12');
  });

  it('adds days across month boundaries', () => {
    expect(addDays('2026-10-31', 1)).toBe('2026-11-01');
    expect(addDays('2026-03-01', -1)).toBe('2026-02-28');
  });

  it('formats numeric dates and month labels', () => {
    expect(formatDateNumeric('2026-10-23T18:30:00')).toBe('23/10/2026');
    expect(formatDateNumeric('')).toBe('');
    expect(monthLabel('2026-10')).toBe('octubre 2026');
  });

  it('formats R multiples and percentages', () => {
    expect(formatR(1.85)).toBe('+1.85R');
    expect(formatR(-0.5)).toBe('-0.50R');
    expect(formatR(null)).toBe('—');
    expect(formatPercent(62.5)).toBe('62,5%');
  });

  it('formats durations', () => {
    expect(formatDuration(0)).toBe('—');
    expect(formatDuration(90)).toBe('1 h 30 min');
    expect(formatDuration(120)).toBe('2 h');
    expect(formatDuration(1500)).toBe('1 d 1 h');
  });

  it('builds initials and labels', () => {
    expect(initials('Jairo Pérez')).toBe('JP');
    expect(sideLabel('buy')).toBe('Compra');
    expect(sideLabel('sell')).toBe('Venta');
  });

  it('clamps values', () => {
    expect(clamp(5, 0, 3)).toBe(3);
    expect(clamp(-1, 0, 3)).toBe(0);
  });
});
