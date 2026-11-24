import { Injectable, computed, effect, signal } from '@angular/core';

export type ThemeMode = 'dark' | 'light';

const STORAGE_KEY = 'jt-theme';

/** Colores usados por los gráficos (ECharts), que no leen variables CSS. */
export interface ChartPalette {
  text: string;
  axis: string;
  axisLabel: string;
  axisLine: string;
  grid: string;
  tooltipBg: string;
  tooltipBorder: string;
  tooltipText: string;
  accent: string;
  accentText: string;
  accentRgb: string;
  pos: string;
  posRgb: string;
  neg: string;
  negRgb: string;
  neutralRgb: string;
  markerBorder: string;
  labelBg: string;
  exitLine: string;
}

const DARK: ChartPalette = {
  text: '#e5e9f2',
  axis: '#8a99b3',
  axisLabel: '#626c81',
  axisLine: '#1e2a3d',
  grid: 'rgba(255, 255, 255, 0.05)',
  tooltipBg: 'rgba(15, 21, 33, 0.96)',
  tooltipBorder: 'rgba(255, 255, 255, 0.12)',
  tooltipText: '#e5e9f2',
  accent: '#6d7cff',
  accentText: '#b8bfff',
  accentRgb: '109, 124, 255',
  pos: '#2dd4a0',
  posRgb: '45, 212, 160',
  neg: '#f2666f',
  negRgb: '242, 102, 111',
  neutralRgb: '139, 147, 167',
  markerBorder: '#0b0f17',
  labelBg: 'rgba(11, 15, 23, 0.84)',
  exitLine: '#8a99b3',
};

const LIGHT: ChartPalette = {
  text: '#141a26',
  axis: '#5b6478',
  axisLabel: '#8a93a6',
  axisLine: '#e1e6ef',
  grid: 'rgba(15, 23, 42, 0.06)',
  tooltipBg: 'rgba(255, 255, 255, 0.98)',
  tooltipBorder: '#e1e6ef',
  tooltipText: '#141a26',
  accent: '#5563f0',
  accentText: '#3f4bd1',
  accentRgb: '85, 99, 240',
  pos: '#0e9f74',
  posRgb: '14, 159, 116',
  neg: '#dc3d4a',
  negRgb: '220, 61, 74',
  neutralRgb: '91, 100, 120',
  markerBorder: '#ffffff',
  labelBg: 'rgba(255, 255, 255, 0.9)',
  exitLine: '#5b6478',
};

export function readStoredTheme(): ThemeMode {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

export function applyTheme(mode: ThemeMode): void {
  const root = document.documentElement;
  if (mode === 'light') root.dataset['theme'] = 'light';
  else delete root.dataset['theme'];
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute('content', mode === 'light' ? '#f3f5f9' : '#0b0f17');
}

@Injectable({ providedIn: 'root' })
export class ThemeService {
  readonly mode = signal<ThemeMode>(readStoredTheme());
  readonly isLight = computed(() => this.mode() === 'light');
  readonly chart = computed<ChartPalette>(() => (this.isLight() ? LIGHT : DARK));

  constructor() {
    effect(() => {
      const mode = this.mode();
      applyTheme(mode);
      try {
        localStorage.setItem(STORAGE_KEY, mode);
      } catch {
        // Sin almacenamiento disponible: el tema solo dura esta sesión.
      }
    });
  }

  set(mode: ThemeMode): void {
    this.mode.set(mode);
  }

  toggle(): void {
    this.mode.update((mode) => (mode === 'light' ? 'dark' : 'light'));
  }
}
