/** Paleta clara para impresión, derivada de los colores de la app. */
export const PDF_COLORS = {
  ink: '#141b34',
  inkSoft: '#252e52',
  accent: '#4f5bd5',
  accentSoft: '#eceefc',
  text: '#1b2333',
  muted: '#6b7488',
  faint: '#9aa2b4',
  border: '#e3e7ef',
  grid: '#eef1f6',
  surface: '#f6f7fb',
  pos: '#12a27a',
  neg: '#d9434e',
  white: '#ffffff',
} as const;

export type Rgb = [number, number, number];

export function hexToRgb(hex: string): Rgb {
  const clean = hex.replace('#', '');
  return [
    parseInt(clean.slice(0, 2), 16),
    parseInt(clean.slice(2, 4), 16),
    parseInt(clean.slice(4, 6), 16),
  ];
}

/** Mezcla `hex` sobre blanco con la intensidad indicada (0..1). */
export function tint(hex: string, amount: number): Rgb {
  const [r, g, b] = hexToRgb(hex);
  const mix = (channel: number) => Math.round(255 + (channel - 255) * amount);
  return [mix(r), mix(g), mix(b)];
}
