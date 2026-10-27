import { describe, expect, it } from 'vitest';

import { dayFingerprint } from './journal-day';
import type { DayView, Trade } from './models';

function baseDay(overrides: Partial<DayView> = {}): DayView {
  return {
    account: {
      id: 'account-1',
      displayName: 'Demo',
      lastHeartbeatAt: '2026-10-27T10:00:00Z',
      updatedAt: '2026-10-27T10:00:00Z',
    },
    journal: {
      id: 'journal-1',
      accountId: 'account-1',
      date: '2026-10-27',
      generalHtml: '<p>Plan</p>',
      createdAt: '2026-10-27T08:00:00Z',
      updatedAt: '2026-10-27T08:00:00Z',
    },
    assets: [],
    openTrades: [],
    images: [],
    ...overrides,
  } as DayView;
}

function trade(overrides: Partial<Trade> = {}): Trade {
  return {
    id: 'trade-1',
    symbol: 'EURUSD',
    profitNet: 10,
    updatedAt: '2026-10-27T10:00:00Z',
    ...overrides,
  } as Trade;
}

describe('dayFingerprint', () => {
  it('ignores heartbeat and timestamp only changes', () => {
    const first = baseDay();
    const second = baseDay({
      account: {
        ...first.account,
        lastHeartbeatAt: '2026-10-27T10:00:05Z',
        updatedAt: '2026-10-27T10:00:05Z',
      },
      journal: { ...first.journal, updatedAt: '2026-10-27T10:00:05Z' },
    });

    expect(dayFingerprint(first)).toBe(dayFingerprint(second));
  });

  it('detects journal changes', () => {
    const first = baseDay();
    const second = baseDay({ journal: { ...first.journal, generalHtml: '<p>Actualizado</p>' } });

    expect(dayFingerprint(first)).not.toBe(dayFingerprint(second));
  });

  it('detects trade changes', () => {
    const first = baseDay({ openTrades: [trade()] });
    const second = baseDay({
      openTrades: [trade({ profitNet: 12, updatedAt: '2026-10-27T10:00:05Z' })],
    });

    expect(dayFingerprint(first)).not.toBe(dayFingerprint(second));
  });
});
