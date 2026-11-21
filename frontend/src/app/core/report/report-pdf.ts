import { jsPDF } from 'jspdf';
import { autoTable, type CellHookData, type RowInput } from 'jspdf-autotable';

import {
  formatDateNumeric,
  formatDuration,
  formatMoney,
  formatNumber,
  formatPrice,
  formatR,
  formatSignedMoney,
  providerShort,
  sideLabel,
} from '../format';
import type { BreakdownRow, CalendarPeriod } from '../models';
import { dailyChart, equityChart, rDistributionChart, renderChart } from './report-charts';
import { breakdownPct, type ReportData, type ReportSection } from './report-data';
import { PDF_COLORS, hexToRgb, tint } from './report-theme';

export const APP_NAME = 'JournalTrading';

const PAGE_W = 210;
const PAGE_H = 297;
const MARGIN = 14;
const CONTENT_W = PAGE_W - MARGIN * 2;
const CONTENT_TOP = 26;
const CONTENT_BOTTOM = PAGE_H - 20;
/** Píxeles por milímetro al rasterizar los gráficos. */
const PX_PER_MM = 4.2;

const MONTHS_SHORT = ['Ene', 'Feb', 'Mar', 'Abr', 'May', 'Jun', 'Jul', 'Ago', 'Sep', 'Oct', 'Nov', 'Dic'];

type Tone = 'pos' | 'neg' | 'neutral';

interface Kpi {
  label: string;
  value: string;
  meta?: string;
  tone?: Tone;
}

/** Los textos van con fuentes estándar (WinAnsi): normalizamos espacios raros de Intl. */
function clean(text: string): string {
  return text.replace(/[   ]/g, ' ').replace(/−/g, '-');
}

function tone(value: number | null | undefined): Tone {
  if (!value) return 'neutral';
  return value > 0 ? 'pos' : 'neg';
}

function toneColor(value: Tone | undefined): string {
  if (value === 'pos') return PDF_COLORS.pos;
  if (value === 'neg') return PDF_COLORS.neg;
  return PDF_COLORS.text;
}

function signedPct(value: number | null | undefined, decimals = 2): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—';
  return `${value > 0 ? '+' : ''}${formatNumber(value, decimals)}%`;
}

function compactNumber(value: number): string {
  const prefix = value > 0 ? '+' : '';
  const decimals = Math.abs(value) >= 1000 ? 0 : Math.abs(value) >= 100 ? 0 : 1;
  return `${prefix}${formatNumber(value, decimals)}`;
}

function profitFactor(value: number | null): string {
  return value === null ? 'Sin pérdidas' : formatNumber(value, 2);
}

function dateTime(value: string | null | undefined): string {
  if (!value) return '—';
  const normalized = value.replace('T', ' ');
  return `${formatDateNumeric(normalized.slice(0, 10))} ${normalized.slice(11, 16)}`.trim();
}

function generatedLabel(date: Date): string {
  const pad = (value: number) => value.toString().padStart(2, '0');
  return `${pad(date.getDate())}/${pad(date.getMonth() + 1)}/${date.getFullYear()} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function periodLabel(data: ReportData): string {
  const from = data.periodFrom ? formatDateNumeric(data.periodFrom) : null;
  const to = data.periodTo ? formatDateNumeric(data.periodTo) : null;
  const range = from && to ? `${from} – ${to}` : from ? `desde ${from}` : to ? `hasta ${to}` : '';
  if (!data.options.from && !data.options.to) {
    return range ? `Todo el historial (${range})` : 'Todo el historial';
  }
  return range;
}

class ReportWriter {
  readonly doc = new jsPDF({ unit: 'mm', format: 'a4', orientation: 'portrait', compress: true });
  private y = CONTENT_TOP;
  private readonly hide: boolean;
  private readonly money: (value: number) => string;
  private readonly signedMoney: (value: number) => string;

  constructor(
    private readonly data: ReportData,
    private readonly logo: string | null,
  ) {
    this.hide = data.options.hideSensitive;
    this.money = (value) => clean(formatMoney(value, data.currency));
    this.signedMoney = (value) => clean(formatSignedMoney(value, data.currency));
    this.doc.setProperties({
      title: `Reporte de rendimiento · ${data.account.displayName}`,
      subject: periodLabel(data),
      creator: APP_NAME,
      author: APP_NAME,
    });
  }

  private has(section: ReportSection): boolean {
    return this.data.options.sections.includes(section);
  }

  build(): jsPDF {
    this.cover();
    if (this.has('summary')) this.summary();
    if (this.has('equity')) this.equity();
    if (this.has('heatmap')) this.heatmaps();
    if (this.has('daily')) this.daily();
    if (this.has('breakdowns')) this.breakdowns();
    if (this.has('timing')) this.timing();
    if (this.data.options.includeTrades) this.tradeList();
    this.decoratePages();
    return this.doc;
  }

  // -------------------------------------------------------------------------
  // Utilidades de maquetación
  // -------------------------------------------------------------------------

  private newPage(): void {
    this.doc.addPage();
    this.y = CONTENT_TOP;
  }

  private ensure(height: number): void {
    if (this.y + height > CONTENT_BOTTOM) this.newPage();
  }

  private font(size: number, style: 'normal' | 'bold' = 'normal', color: string = PDF_COLORS.text) {
    this.doc.setFont('helvetica', style);
    this.doc.setFontSize(size);
    this.doc.setTextColor(color);
  }

  private text(
    value: string,
    x: number,
    y: number,
    options: { align?: 'left' | 'right' | 'center'; maxWidth?: number } = {},
  ): void {
    this.doc.text(clean(value), x, y, { baseline: 'alphabetic', ...options });
  }

  private sectionTitle(title: string, subtitle?: string, minBody = 40): void {
    this.ensure(14 + minBody);
    if (this.y > CONTENT_TOP) this.y += 4;
    this.doc.setFillColor(PDF_COLORS.accent);
    this.doc.roundedRect(MARGIN, this.y - 4.6, 1.4, 6, 0.7, 0.7, 'F');
    this.font(13, 'bold', PDF_COLORS.ink);
    this.text(title, MARGIN + 4, this.y);
    if (subtitle) {
      this.font(8, 'normal', PDF_COLORS.muted);
      this.text(subtitle, PAGE_W - MARGIN, this.y, { align: 'right' });
    }
    this.y += 7;
  }

  private note(message: string): void {
    this.ensure(14);
    this.doc.setFillColor(PDF_COLORS.surface);
    this.doc.roundedRect(MARGIN, this.y, CONTENT_W, 11, 2, 2, 'F');
    this.font(8.5, 'normal', PDF_COLORS.muted);
    this.text(message, PAGE_W / 2, this.y + 6.8, { align: 'center' });
    this.y += 15;
  }

  private kpiGrid(items: Kpi[], columns = 4, height = 19): void {
    const gap = 3;
    const width = (CONTENT_W - gap * (columns - 1)) / columns;
    for (let index = 0; index < items.length; index += columns) {
      this.ensure(height + gap);
      items.slice(index, index + columns).forEach((item, column) => {
        const x = MARGIN + column * (width + gap);
        this.doc.setFillColor(PDF_COLORS.surface);
        this.doc.setDrawColor(PDF_COLORS.border);
        this.doc.setLineWidth(0.2);
        this.doc.roundedRect(x, this.y, width, height, 2, 2, 'FD');
        this.font(6.8, 'bold', PDF_COLORS.muted);
        this.text(item.label.toUpperCase(), x + 3.5, this.y + 5.4, { maxWidth: width - 7 });
        this.font(12.5, 'bold', toneColor(item.tone));
        this.text(item.value, x + 3.5, this.y + 12.2, { maxWidth: width - 7 });
        if (item.meta) {
          this.font(6.8, 'normal', PDF_COLORS.muted);
          this.text(item.meta, x + 3.5, this.y + 16.4, { maxWidth: width - 7 });
        }
      });
      this.y += height + gap;
    }
    this.y += 2;
  }

  private chart(image: string, heightMm: number): void {
    this.ensure(heightMm + 4);
    this.doc.setDrawColor(PDF_COLORS.border);
    this.doc.setLineWidth(0.2);
    this.doc.roundedRect(MARGIN, this.y, CONTENT_W, heightMm + 4, 2, 2, 'S');
    this.doc.addImage(image, 'PNG', MARGIN + 2, this.y + 2, CONTENT_W - 4, heightMm, undefined, 'FAST');
    this.y += heightMm + 8;
  }

  private chartImage(option: Parameters<typeof renderChart>[0], heightMm: number): string {
    return renderChart(option, Math.round((CONTENT_W - 4) * PX_PER_MM), Math.round(heightMm * PX_PER_MM));
  }

  private lastTableY(): number {
    const table = (this.doc as unknown as { lastAutoTable?: { finalY?: number } }).lastAutoTable;
    return table?.finalY ?? this.y;
  }

  private table(options: {
    head: string[];
    body: RowInput[];
    columnStyles?: Record<number, Record<string, unknown>>;
    fontSize?: number;
    left?: number;
    width?: number;
    didParseCell?: (data: CellHookData) => void;
  }): number {
    const left = options.left ?? MARGIN;
    const width = options.width ?? CONTENT_W;
    autoTable(this.doc, {
      startY: this.y,
      head: [options.head],
      body: options.body,
      theme: 'plain',
      margin: { left, right: PAGE_W - left - width, top: CONTENT_TOP, bottom: PAGE_H - CONTENT_BOTTOM },
      tableWidth: width,
      showHead: 'everyPage',
      styles: {
        font: 'helvetica',
        fontSize: options.fontSize ?? 8,
        textColor: hexToRgb(PDF_COLORS.text),
        cellPadding: { top: 1.8, bottom: 1.8, left: 2.2, right: 2.2 },
        lineColor: hexToRgb(PDF_COLORS.border),
        overflow: 'ellipsize',
      },
      headStyles: {
        fillColor: hexToRgb(PDF_COLORS.ink),
        textColor: hexToRgb(PDF_COLORS.white),
        fontStyle: 'bold',
        fontSize: (options.fontSize ?? 8) - 0.5,
      },
      alternateRowStyles: { fillColor: hexToRgb(PDF_COLORS.surface) },
      bodyStyles: { lineWidth: { bottom: 0.15 } },
      columnStyles: options.columnStyles as never,
      didParseCell: (cell) => {
        if (typeof cell.cell.raw === 'string') cell.cell.text = [clean(cell.cell.raw)];
        options.didParseCell?.(cell);
      },
    });
    return this.lastTableY();
  }

  /** Colorea una celda numérica según su signo (verde/rojo). */
  private static colorBySign(cell: CellHookData, values: Array<number | null>): void {
    if (cell.section !== 'body') return;
    const value = values[cell.row.index];
    if (value === null || value === undefined || value === 0) return;
    cell.cell.styles.textColor = hexToRgb(value > 0 ? PDF_COLORS.pos : PDF_COLORS.neg);
    cell.cell.styles.fontStyle = 'bold';
  }

  // -------------------------------------------------------------------------
  // Secciones
  // -------------------------------------------------------------------------

  private cover(): void {
    const { doc, data } = this;
    const account = data.account;
    const summary = data.stats.summary;

    doc.setFillColor(PDF_COLORS.ink);
    doc.rect(0, 0, PAGE_W, 78, 'F');
    doc.setFillColor(PDF_COLORS.inkSoft);
    doc.circle(PAGE_W - 18, 6, 46, 'F');
    doc.setFillColor(PDF_COLORS.accent);
    doc.rect(0, 78, PAGE_W, 1.6, 'F');

    if (this.logo) doc.addImage(this.logo, 'PNG', MARGIN, 13, 11, 11);
    this.font(11, 'bold', PDF_COLORS.white);
    this.text(APP_NAME, MARGIN + (this.logo ? 14 : 0), 20.2);
    this.font(7.5, 'bold', '#aeb6ff');
    this.text('REPORTE DE RENDIMIENTO', PAGE_W - MARGIN, 20.2, { align: 'right' });

    this.font(24, 'bold', PDF_COLORS.white);
    this.text(account.displayName || account.server, MARGIN, 45, { maxWidth: CONTENT_W });
    this.font(10.5, 'normal', '#c9cfe8');
    this.text(periodLabel(data), MARGIN, 54);
    this.font(8, 'normal', '#9099c0');
    this.text(`Generado el ${generatedLabel(data.generatedAt)}`, MARGIN, 61);

    // Ficha de la cuenta.
    const info: Array<[string, string]> = this.hide
      ? [
          ['Plataforma', providerShort(account.provider)],
          ['Broker', account.company || '—'],
          ['Divisa', data.currency],
          ['Periodo', periodLabel(data)],
          ['Operaciones', `${summary.totalTrades}`],
          ['Generado', generatedLabel(data.generatedAt)],
        ]
      : [
          ['Plataforma', providerShort(account.provider)],
          ['Broker', account.company || '—'],
          ['Divisa', data.currency],
          ['Login', account.provider === 'atas' ? '—' : `${account.login}`],
          ['Servidor', account.server || '—'],
          ['Operaciones', `${summary.totalTrades}`],
          ['Balance inicial', this.money(data.startBalance)],
          ['Balance final', this.money(data.endBalance)],
          ['Apalancamiento', account.leverage ? `1:${account.leverage}` : '—'],
        ];

    const top = 90;
    const rows = Math.ceil(info.length / 3);
    const cardH = rows * 12 + 6;
    doc.setFillColor(PDF_COLORS.white);
    doc.setDrawColor(PDF_COLORS.border);
    doc.setLineWidth(0.25);
    doc.roundedRect(MARGIN, top, CONTENT_W, cardH, 2.5, 2.5, 'FD');
    const colW = CONTENT_W / 3;
    info.forEach(([label, value], index) => {
      const x = MARGIN + 5 + (index % 3) * colW;
      const y = top + 8 + Math.floor(index / 3) * 12;
      this.font(6.8, 'bold', PDF_COLORS.faint);
      this.text(label.toUpperCase(), x, y);
      this.font(9.5, 'bold', PDF_COLORS.text);
      this.text(value, x, y + 5, { maxWidth: colW - 8 });
    });

    // Cifras destacadas.
    this.y = top + cardH + 8;
    this.font(8, 'bold', PDF_COLORS.muted);
    this.text('RESUMEN DEL PERIODO', MARGIN, this.y);
    this.y += 4;

    const highlights: Kpi[] = [
      {
        label: 'Retorno',
        value: signedPct(data.returnPct),
        meta: this.hide ? `${summary.totalTrades} operaciones` : `${this.money(data.startBalance)} – ${this.money(data.endBalance)}`,
        tone: tone(data.returnPct),
      },
      this.hide
        ? { label: 'R total', value: formatR(summary.totalR), meta: `R medio ${formatR(summary.avgR)}`, tone: tone(summary.totalR) }
        : { label: 'Beneficio neto', value: this.signedMoney(summary.netProfit), meta: `${summary.totalTrades} operaciones`, tone: tone(summary.netProfit) },
      {
        label: 'Win rate',
        value: `${formatNumber(summary.winRate, 1)}%`,
        meta: `${summary.wins} G · ${summary.losses} P · ${summary.breakeven} BE`,
      },
      {
        label: 'Máx. drawdown',
        value: signedPct(data.maxDrawdownBalancePct),
        meta: this.hide ? 'sobre el pico de balance' : this.money(-summary.maxDrawdown),
        tone: data.maxDrawdownBalancePct < 0 ? 'neg' : 'neutral',
      },
    ];
    const gap = 3;
    const width = (CONTENT_W - gap * 3) / 4;
    highlights.forEach((item, index) => {
      const x = MARGIN + index * (width + gap);
      doc.setFillColor(index === 0 ? PDF_COLORS.accentSoft : PDF_COLORS.surface);
      doc.setDrawColor(index === 0 ? '#cfd3f6' : PDF_COLORS.border);
      doc.roundedRect(x, this.y, width, 26, 2.5, 2.5, 'FD');
      this.font(7, 'bold', PDF_COLORS.muted);
      this.text(item.label.toUpperCase(), x + 4, this.y + 6.5);
      this.font(16, 'bold', toneColor(item.tone));
      this.text(item.value, x + 4, this.y + 15.5, { maxWidth: width - 8 });
      this.font(6.8, 'normal', PDF_COLORS.muted);
      this.text(item.meta ?? '', x + 4, this.y + 21.5, { maxWidth: width - 8 });
    });
    this.y += 34;
  }

  private summary(): void {
    const s = this.data.stats.summary;
    const ratio = s.avgLoss ? s.avgWin / Math.abs(s.avgLoss) : null;
    const items: Kpi[] = this.hide
      ? [
          { label: 'Retorno', value: signedPct(this.data.returnPct), tone: tone(this.data.returnPct), meta: 'sobre el balance inicial' },
          { label: 'Operaciones', value: `${s.totalTrades}`, meta: `${s.wins} G · ${s.losses} P · ${s.breakeven} BE` },
          { label: 'Win rate', value: `${formatNumber(s.winRate, 1)}%`, tone: s.winRate >= 50 ? 'pos' : 'neg' },
          { label: 'Profit factor', value: profitFactor(s.profitFactor), tone: (s.profitFactor ?? 2) >= 1 ? 'pos' : 'neg' },
          { label: 'Esperanza (R)', value: formatR(s.avgR), tone: tone(s.avgR), meta: 'por operación' },
          { label: 'R total', value: formatR(s.totalR), tone: tone(s.totalR) },
          { label: 'Ratio G/P medio', value: ratio === null ? '—' : formatNumber(ratio, 2), meta: 'ganancia media / pérdida media' },
          { label: 'Máx. drawdown', value: signedPct(this.data.maxDrawdownBalancePct), tone: 'neg', meta: 'sobre el pico de balance' },
          { label: 'Mejor operación', value: signedPct(this.data.bestTradePct), tone: 'pos' },
          { label: 'Peor operación', value: signedPct(this.data.worstTradePct), tone: 'neg' },
          { label: 'Rachas máximas', value: `${s.maxWinStreak} G / ${s.maxLossStreak} P`, meta: `actual ${s.currentStreak > 0 ? '+' : ''}${s.currentStreak}` },
          { label: 'Duración media', value: formatDuration(s.avgDurationMinutes), meta: 'por operación' },
        ]
      : [
          { label: 'Beneficio neto', value: this.signedMoney(s.netProfit), tone: tone(s.netProfit), meta: `${s.totalTrades} operaciones` },
          { label: 'Retorno', value: signedPct(this.data.returnPct), tone: tone(this.data.returnPct), meta: 'sobre el balance inicial' },
          { label: 'Win rate', value: `${formatNumber(s.winRate, 1)}%`, tone: s.winRate >= 50 ? 'pos' : 'neg', meta: `${s.wins} G · ${s.losses} P · ${s.breakeven} BE` },
          { label: 'Profit factor', value: profitFactor(s.profitFactor), tone: (s.profitFactor ?? 2) >= 1 ? 'pos' : 'neg', meta: `${this.money(s.grossProfit)} / ${this.money(-s.grossLoss)}` },
          { label: 'Esperanza', value: this.signedMoney(s.expectancy), tone: tone(s.expectancy), meta: 'por operación' },
          { label: 'Ganancia media', value: this.signedMoney(s.avgWin), tone: 'pos' },
          { label: 'Pérdida media', value: this.signedMoney(s.avgLoss), tone: 'neg' },
          { label: 'Ratio G/P medio', value: ratio === null ? '—' : formatNumber(ratio, 2) },
          { label: 'R medio', value: formatR(s.avgR), tone: tone(s.avgR), meta: 'por operación' },
          { label: 'R total', value: formatR(s.totalR), tone: tone(s.totalR) },
          { label: 'Máx. drawdown', value: this.money(-s.maxDrawdown), tone: 'neg', meta: `${signedPct(this.data.maxDrawdownBalancePct)} sobre el pico de balance` },
          { label: 'Duración media', value: formatDuration(s.avgDurationMinutes), meta: 'por operación' },
          { label: 'Mejor operación', value: this.signedMoney(s.bestTrade), tone: 'pos', meta: signedPct(this.data.bestTradePct) },
          { label: 'Peor operación', value: this.signedMoney(s.worstTrade), tone: 'neg', meta: signedPct(this.data.worstTradePct) },
          { label: 'Racha máx. ganadora', value: `${s.maxWinStreak}`, meta: 'operaciones seguidas' },
          { label: 'Racha máx. perdedora', value: `${s.maxLossStreak}`, meta: `actual ${s.currentStreak > 0 ? '+' : ''}${s.currentStreak}` },
        ];
    this.sectionTitle('Estadísticas', `${s.totalTrades} operaciones cerradas`, 45);
    this.kpiGrid(items);
  }

  private equity(): void {
    const subtitle = this.hide ? 'Retorno acumulado (%) y drawdown' : `Retorno acumulado en % y en ${this.data.currency} · drawdown`;
    this.sectionTitle('Curva de retorno', subtitle, 100);
    if (this.data.equity.length < 1) {
      this.note('No hay operaciones cerradas en el periodo.');
      return;
    }
    this.chart(this.chartImage(equityChart(this.data), 96), 96);
    this.font(7, 'normal', PDF_COLORS.faint);
    this.text(
      'Retorno sobre el balance al inicio del periodo. El área inferior muestra la caída desde el máximo de balance.',
      MARGIN,
      this.y - 1,
    );
    this.y += 4;
  }

  private heatmaps(): void {
    const heatmap = this.data.heatmap;
    if (!heatmap.rows.length) {
      this.sectionTitle('Resultados por meses y años', undefined, 20);
      this.note('No hay operaciones cerradas en el periodo.');
      return;
    }
    this.heatmapTable('pct', 'Resultados mensuales (%)', 'Retorno compuesto sobre el balance de cada periodo');
    if (!this.hide) {
      this.heatmapTable('money', `Resultados mensuales (${this.data.currency})`, 'Beneficio neto por mes y año');
    }
    if (heatmap.hasR) {
      this.heatmapTable('r', 'Resultados mensuales (R)', 'R total por mes y año');
    }
  }

  private heatmapTable(mode: 'pct' | 'money' | 'r', title: string, subtitle: string): void {
    const heatmap = this.data.heatmap;
    const value = (period: CalendarPeriod | null): number | null => {
      if (!period || period.trades === 0) return null;
      if (mode === 'pct') return period.pct;
      if (mode === 'money') return period.net;
      return period.rTrades > 0 ? period.rTotal : null;
    };
    const label = (period: CalendarPeriod | null): string => {
      const v = value(period);
      if (v === null) return '';
      if (mode === 'pct') return signedPct(v, Math.abs(v) >= 100 ? 0 : 1);
      if (mode === 'money') return compactNumber(v);
      return `${v > 0 ? '+' : ''}${formatNumber(v, 1)}R`;
    };

    const matrix: Array<Array<CalendarPeriod | null>> = [
      ...heatmap.rows.map((row) => [...row.cells, row.total]),
      [...heatmap.monthTotals, heatmap.totals],
    ];
    const maxAbs = Math.max(
      0.0001,
      ...matrix.flatMap((row) => row.slice(0, 12).map((cell) => Math.abs(value(cell) ?? 0))),
    );
    const maxTotalAbs = Math.max(0.0001, ...matrix.map((row) => Math.abs(value(row[12]) ?? 0)));

    const body: RowInput[] = matrix.map((row, index) => [
      index < heatmap.rows.length ? `${heatmap.rows[index].year}` : 'Total',
      ...row.map((cell) => label(cell)),
    ]);

    this.sectionTitle(title, subtitle, Math.min(80, 12 + matrix.length * 7));
    const monthW = (CONTENT_W - 15 - 19) / 12;
    const columnStyles: Record<number, Record<string, unknown>> = {
      0: { cellWidth: 15, fontStyle: 'bold', halign: 'left' },
      13: { cellWidth: 19, fontStyle: 'bold' },
    };
    for (let month = 1; month <= 12; month += 1) columnStyles[month] = { cellWidth: monthW };

    this.y = this.table({
      head: ['Año', ...MONTHS_SHORT, 'Total'],
      body,
      fontSize: 7,
      columnStyles,
      didParseCell: (cell) => {
        cell.cell.styles.halign = cell.column.index === 0 ? 'left' : 'center';
        cell.cell.styles.cellPadding = { top: 2.4, bottom: 2.4, left: 1, right: 1 };
        if (cell.section !== 'body') return;
        const isTotalRow = cell.row.index === matrix.length - 1;
        if (isTotalRow) {
          cell.cell.styles.fontStyle = 'bold';
          cell.cell.styles.lineWidth = { top: 0.35 };
          cell.cell.styles.lineColor = hexToRgb(PDF_COLORS.ink);
        }
        if (cell.column.index === 0) {
          cell.cell.styles.fillColor = hexToRgb(isTotalRow ? PDF_COLORS.accentSoft : PDF_COLORS.surface);
          return;
        }
        const period = matrix[cell.row.index][cell.column.index - 1];
        const v = value(period);
        if (v === null || v === 0) {
          cell.cell.styles.fillColor = hexToRgb(PDF_COLORS.white);
          return;
        }
        const max = cell.column.index === 13 ? maxTotalAbs : maxAbs;
        const intensity = Math.min(1, Math.abs(v) / max);
        const color = v > 0 ? PDF_COLORS.pos : PDF_COLORS.neg;
        cell.cell.styles.fillColor = tint(color, 0.1 + intensity * 0.55);
        cell.cell.styles.textColor = hexToRgb(intensity > 0.75 ? PDF_COLORS.white : PDF_COLORS.text);
      },
    });
    this.y += 6;
  }

  private daily(): void {
    const subtitle = this.hide ? 'Retorno diario (%)' : `Beneficio neto diario (${this.data.currency})`;
    this.sectionTitle('P&L diario', subtitle, 64);
    if (!this.data.days.length) {
      this.note('No hay operaciones cerradas en el periodo.');
    } else {
      this.chart(this.chartImage(dailyChart(this.data), 58), 58);
    }

    const buckets = this.data.stats.rDistribution;
    const total = buckets.reduce((sum, bucket) => sum + bucket.count, 0);
    this.sectionTitle('Distribución de R', `${total} operaciones con R`, 58);
    if (!total) {
      this.note('Ninguna operación del periodo tiene R calculado (falta stop loss o riesgo).');
    } else {
      this.chart(this.chartImage(rDistributionChart(buckets), 54), 54);
    }
  }

  private breakdownTable(title: string, labelHeader: string, rows: BreakdownRow[], subtitle: string): void {
    this.sectionTitle(title, subtitle, Math.min(120, 12 + rows.length * 7));
    if (!rows.length) {
      this.note('Sin operaciones en el periodo.');
      return;
    }
    const sorted = [...rows].sort((a, b) => b.net - a.net);
    const last = this.hide ? 'Retorno' : 'Neto';
    const values = sorted.map((row) => (this.hide ? breakdownPct(row, this.data.startBalance) : row.net));
    const rValues = sorted.map((row) => row.avgR);
    this.y = this.table({
      head: [labelHeader, 'Ops', 'Ganadas', 'Perdidas', 'Win rate', 'Profit factor', 'R medio', last],
      body: sorted.map((row, index) => [
        row.label,
        `${row.trades}`,
        `${row.wins}`,
        `${row.losses}`,
        `${formatNumber(row.winRate, 1)}%`,
        profitFactor(row.profitFactor),
        formatR(row.avgR),
        this.hide ? signedPct(values[index]) : this.signedMoney(row.net),
      ]),
      columnStyles: {
        0: { cellWidth: 46, fontStyle: 'bold' },
        1: { halign: 'right' },
        2: { halign: 'right' },
        3: { halign: 'right' },
        4: { halign: 'right' },
        5: { halign: 'right' },
        6: { halign: 'right' },
        7: { halign: 'right', cellWidth: 30 },
      },
      didParseCell: (cell) => {
        if (cell.section === 'head' && cell.column.index > 0) cell.cell.styles.halign = 'right';
        if (cell.column.index === 7) ReportWriter.colorBySign(cell, values);
        if (cell.column.index === 6) ReportWriter.colorBySign(cell, rValues);
      },
    });
    this.y += 6;
  }

  private breakdowns(): void {
    const subtitle = this.hide ? 'Retorno sobre el balance inicial' : 'Ordenado por beneficio neto';
    this.breakdownTable('Por símbolo', 'Símbolo', this.data.stats.bySymbol, subtitle);
    this.breakdownTable('Por estrategia', 'Estrategia', this.data.stats.byStrategy, subtitle);
  }

  private timing(): void {
    const weekdays = [...this.data.stats.byWeekday].filter((row) => row.trades > 0);
    const hours = [...this.data.stats.byHour].filter((row) => row.trades > 0);
    const needed = 14 + Math.max(weekdays.length, hours.length) * 6.2;
    this.sectionTitle('Rendimiento por día y hora', 'Según la hora de cierre del broker', Math.min(needed, 200));
    if (!weekdays.length && !hours.length) {
      this.note('Sin operaciones en el periodo.');
      return;
    }

    const gap = 5;
    const half = (CONTENT_W - gap) / 2;
    const start = this.y;
    const startPage = this.doc.getNumberOfPages();
    const render = (rows: BreakdownRow[], header: string, label: (row: BreakdownRow) => string, left: number) => {
      this.y = start;
      this.doc.setPage(startPage);
      const values = rows.map((row) => (this.hide ? breakdownPct(row, this.data.startBalance) : row.net));
      return this.table({
        head: [header, 'Ops', 'Win rate', this.hide ? 'Retorno' : 'Neto'],
        body: rows.map((row, index) => [
          label(row),
          `${row.trades}`,
          `${formatNumber(row.winRate, 1)}%`,
          this.hide ? signedPct(values[index]) : this.signedMoney(row.net),
        ]),
        left,
        width: half,
        columnStyles: {
          0: { fontStyle: 'bold' },
          1: { halign: 'right', cellWidth: 12 },
          2: { halign: 'right', cellWidth: 20 },
          3: { halign: 'right', cellWidth: 28 },
        },
        didParseCell: (cell) => {
          if (cell.section === 'head' && cell.column.index > 0) cell.cell.styles.halign = 'right';
          if (cell.column.index === 3) ReportWriter.colorBySign(cell, values);
        },
      });
    };

    const weekdayLabelOf = (row: BreakdownRow) => row.label.charAt(0).toUpperCase() + row.label.slice(1);
    const firstEnd = render(weekdays, 'Día', weekdayLabelOf, MARGIN);
    const firstPage = this.doc.getNumberOfPages();
    const secondEnd = render(hours, 'Hora', (row) => row.label, MARGIN + half + gap);
    const lastPage = this.doc.getNumberOfPages();
    this.doc.setPage(lastPage);
    this.y = (lastPage > firstPage ? secondEnd : Math.max(firstEnd, secondEnd)) + 6;
  }

  private tradeList(): void {
    const rows = [...this.data.trades].reverse();
    this.sectionTitle('Listado de operaciones', `${rows.length} operaciones cerradas · más recientes primero`, 30);
    if (!rows.length) {
      this.note('Sin operaciones en el periodo.');
      return;
    }
    const head = this.hide
      ? ['Cierre', 'Símbolo', 'Lado', 'Entrada', 'Salida', 'R', 'Retorno']
      : ['Cierre', 'Símbolo', 'Lado', 'Volumen', 'Entrada', 'Salida', 'R', 'Neto'];
    const resultValues = rows.map((row) => (this.hide ? row.pct : row.trade.profitNet));
    const rValues = rows.map((row) => row.trade.rMultiple);
    const body: RowInput[] = rows.map(({ trade, pct }) => {
      const base = [
        dateTime(trade.closeTimeServer ?? trade.openTimeServer),
        trade.symbol,
        sideLabel(trade.side),
      ];
      const prices = [formatPrice(trade.openPrice), formatPrice(trade.closePrice), formatR(trade.rMultiple)];
      return this.hide
        ? [...base, ...prices, signedPct(pct)]
        : [...base, formatNumber(trade.volumeInitial, 2), ...prices, this.signedMoney(trade.profitNet)];
    });
    const resultColumn = head.length - 1;
    const rColumn = head.length - 2;
    const numericFrom = 3;
    const columnStyles: Record<number, Record<string, unknown>> = {
      0: { cellWidth: 27 },
      1: { fontStyle: 'bold' },
      2: { cellWidth: 15 },
    };
    for (let column = numericFrom; column < head.length; column += 1) {
      columnStyles[column] = { halign: 'right' };
    }
    columnStyles[resultColumn] = { halign: 'right', cellWidth: 27 };

    this.y = this.table({
      head,
      body,
      fontSize: 7.5,
      columnStyles,
      didParseCell: (cell) => {
        if (cell.section === 'head' && cell.column.index >= numericFrom) cell.cell.styles.halign = 'right';
        if (cell.column.index === resultColumn) ReportWriter.colorBySign(cell, resultValues);
        if (cell.column.index === rColumn) ReportWriter.colorBySign(cell, rValues);
        if (cell.section === 'body' && cell.column.index === 2) {
          cell.cell.styles.textColor = hexToRgb(
            rows[cell.row.index].trade.side === 'buy' ? PDF_COLORS.accent : '#b7791f',
          );
        }
      },
    });
  }

  /** Cabecera (desde la página 2) y pie con numeración en todas las páginas. */
  private decoratePages(): void {
    const { doc, data } = this;
    const total = doc.getNumberOfPages();
    const generated = generatedLabel(data.generatedAt);
    for (let page = 1; page <= total; page += 1) {
      doc.setPage(page);
      if (page > 1) {
        if (this.logo) doc.addImage(this.logo, 'PNG', MARGIN, 8.6, 5.2, 5.2);
        this.font(8.5, 'bold', PDF_COLORS.ink);
        this.text(APP_NAME, MARGIN + (this.logo ? 7 : 0), 12.6);
        this.font(7.5, 'normal', PDF_COLORS.muted);
        this.text(`${data.account.displayName} · ${periodLabel(data)}`, PAGE_W - MARGIN, 12.6, {
          align: 'right',
        });
        doc.setDrawColor(PDF_COLORS.border);
        doc.setLineWidth(0.3);
        doc.line(MARGIN, 16.5, PAGE_W - MARGIN, 16.5);
      }
      doc.setDrawColor(PDF_COLORS.border);
      doc.setLineWidth(0.3);
      doc.line(MARGIN, PAGE_H - 13, PAGE_W - MARGIN, PAGE_H - 13);
      this.font(7, 'normal', PDF_COLORS.faint);
      this.text(`${APP_NAME} · Reporte generado el ${generated}`, MARGIN, PAGE_H - 8.5);
      this.text(`Página ${page} de ${total}`, PAGE_W - MARGIN, PAGE_H - 8.5, { align: 'right' });
    }
  }
}

export function buildReportPdf(data: ReportData, logo: string | null): jsPDF {
  return new ReportWriter(data, logo).build();
}
