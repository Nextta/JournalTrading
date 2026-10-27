import type { DayView } from './models';

const VOLATILE_KEYS = new Set(['account', 'createdAt', 'updatedAt', 'lastHeartbeatAt']);

export function dayFingerprint(day: DayView | null): string {
  if (!day) return '';
  return JSON.stringify(day, (key, value) => (VOLATILE_KEYS.has(key) ? undefined : value));
}
