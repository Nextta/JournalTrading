const MONTHS = [
  'enero',
  'febrero',
  'marzo',
  'abril',
  'mayo',
  'junio',
  'julio',
  'agosto',
  'septiembre',
  'octubre',
  'noviembre',
  'diciembre',
];

const WEEKDAYS = ['domingo', 'lunes', 'martes', 'miércoles', 'jueves', 'viernes', 'sábado'];
const WEEKDAYS_SHORT = ['dom', 'lun', 'mar', 'mié', 'jue', 'vie', 'sáb'];

export function pad(value: number): string {
  return value.toString().padStart(2, '0');
}

export function todayIso(): string {
  return dateToIso(new Date());
}

export function dateToIso(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function parseIsoDate(value: string): Date {
  const [year, month, day] = value.split('-').map(Number);
  return new Date(year, (month || 1) - 1, day || 1, 12, 0, 0);
}

export function addDays(value: string, days: number): string {
  const date = parseIsoDate(value);
  date.setDate(date.getDate() + days);
  return dateToIso(date);
}

export function startOfMonth(value = new Date()): string {
  return dateToIso(new Date(value.getFullYear(), value.getMonth(), 1));
}

export function monthsAgo(months: number): string {
  const date = new Date();
  date.setMonth(date.getMonth() - months);
  return dateToIso(date);
}

export function formatDateLong(value: string): string {
  if (!value) return '';
  const date = parseIsoDate(value);
  return `${WEEKDAYS[date.getDay()]}, ${date.getDate()} de ${MONTHS[date.getMonth()]} de ${date.getFullYear()}`;
}

export function formatDateShort(value: string): string {
  if (!value) return '';
  const date = parseIsoDate(value);
  return `${pad(date.getDate())} ${MONTHS[date.getMonth()].slice(0, 3)} ${date.getFullYear()}`;
}

export function formatDateNumeric(value: string): string {
  if (!value) return '';
  return value.slice(0, 10).split('-').reverse().join('/');
}

export function formatDateTime(value: string | null | undefined): string {
  if (!value) return '—';
  const clean = value.replace('T', ' ').replace('Z', '');
  return clean.slice(0, 16);
}

export function formatTime(value: string | null | undefined): string {
  if (!value) return '—';
  const clean = value.replace('T', ' ');
  return clean.slice(11, 16);
}

export function formatMoney(value: number | null | undefined, currency = 'USD'): string {
  const amount = value ?? 0;
  try {
    return new Intl.NumberFormat('es-ES', {
      style: 'currency',
      currency: currency || 'USD',
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }).format(amount);
  } catch {
    return `${amount.toFixed(2)} ${currency}`;
  }
}

export function formatSignedMoney(value: number, currency = 'USD'): string {
  const prefix = value > 0 ? '+' : '';
  return `${prefix}${formatMoney(value, currency)}`;
}

export function formatNumber(value: number | null | undefined, decimals = 2): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—';
  return new Intl.NumberFormat('es-ES', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  }).format(value);
}

export function formatPrice(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  const decimals = Math.abs(value) >= 100 ? 2 : Math.abs(value) >= 10 ? 3 : 5;
  return new Intl.NumberFormat('es-ES', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  }).format(value);
}

export function formatPercent(value: number | null | undefined, decimals = 1): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—';
  return `${formatNumber(value, decimals)}%`;
}

export function formatR(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—';
  const prefix = value > 0 ? '+' : '';
  return `${prefix}${value.toFixed(2)}R`;
}

export function formatDuration(minutes: number): string {
  if (!minutes || minutes <= 0) return '—';
  if (minutes < 60) return `${Math.round(minutes)} min`;
  const hours = Math.floor(minutes / 60);
  const rest = Math.round(minutes % 60);
  if (hours < 24) return rest ? `${hours} h ${rest} min` : `${hours} h`;
  const days = Math.floor(hours / 24);
  return `${days} d ${hours % 24} h`;
}

export function timeAgo(value: string | null | undefined): string {
  if (!value) return 'sin conexión';
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return '—';
  const seconds = Math.max(0, (Date.now() - parsed.getTime()) / 1000);
  if (seconds < 10) return 'ahora';
  if (seconds < 60) return `hace ${Math.floor(seconds)} s`;
  if (seconds < 3600) return `hace ${Math.floor(seconds / 60)} min`;
  if (seconds < 86400) return `hace ${Math.floor(seconds / 3600)} h`;
  return `hace ${Math.floor(seconds / 86400)} d`;
}

export function monthLabel(value: string): string {
  const [year, month] = value.split('-').map(Number);
  return `${MONTHS[(month || 1) - 1]} ${year}`;
}

export function weekdayLabel(index: number): string {
  return WEEKDAYS[index] ?? '';
}

export function weekdayShort(index: number): string {
  return WEEKDAYS_SHORT[index] ?? '';
}

export function hourLabel(value: string): string {
  const hour = Number(value.replace(/[^0-9]/g, ''));
  if (!Number.isFinite(hour)) return value;
  return `${pad(hour)}:00`;
}

export function sideLabel(side: string): string {
  return side === 'buy' ? 'Compra' : 'Venta';
}

export function entryLabel(entry: number): string {
  if (entry === 0) return 'Entrada';
  if (entry === 1) return 'Salida';
  if (entry === 2) return 'Entrada/Salida';
  return 'Salida parcial';
}

export function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? '')
    .join('');
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function percentChange(current: number, previous: number): number {
  if (!previous) return 0;
  return ((current - previous) / Math.abs(previous)) * 100;
}
