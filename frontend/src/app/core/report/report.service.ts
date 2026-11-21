import { Injectable, inject } from '@angular/core';
import { save } from '@tauri-apps/plugin-dialog';

import { ApiService } from '../api.service';
import type { Account, Trade } from '../models';
import { buildReportData, type ReportOptions } from './report-data';

const TRADES_PAGE = 500;

function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

function slug(value: string): string {
  return (
    value
      .normalize('NFD')
      .replace(/[̀-ͯ]/g, '')
      .replace(/[^a-zA-Z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 40) || 'cuenta'
  );
}

export function reportFileName(account: Account, options: ReportOptions): string {
  const range =
    options.from || options.to
      ? `${options.from ?? 'inicio'}_${options.to ?? 'hoy'}`
      : 'historial-completo';
  return `Reporte-${slug(account.displayName || account.server)}-${range}.pdf`;
}

async function loadLogo(): Promise<string | null> {
  try {
    const response = await fetch('report-logo.png');
    if (!response.ok) return null;
    const blob = await response.blob();
    return await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });
  } catch {
    return null;
  }
}

/** Genera el reporte PDF de una cuenta y lo guarda donde elija el usuario. */
@Injectable({ providedIn: 'root' })
export class ReportService {
  private readonly api = inject(ApiService);

  private async loadTrades(account: Account, options: ReportOptions): Promise<Trade[]> {
    const trades: Trade[] = [];
    for (let offset = 0; ; offset += TRADES_PAGE) {
      const page = await this.api.listTrades({
        accountId: account.id,
        from: options.from,
        to: options.to,
        state: 'closed',
        limit: TRADES_PAGE,
        offset,
      });
      trades.push(...page.items);
      if (!page.items.length || trades.length >= page.total) break;
    }
    return trades;
  }

  /** Construye el PDF en memoria. */
  async build(account: Account, options: ReportOptions): Promise<Uint8Array> {
    const [stats, calendar, trades, logo, pdf] = await Promise.all([
      this.api.getStats({ accountId: account.id, from: options.from, to: options.to }),
      this.api.getPerformanceCalendar(account.id),
      this.loadTrades(account, options),
      loadLogo(),
      import('./report-pdf'),
    ]);
    const data = buildReportData({ account, options, stats, calendar, trades });
    const doc = pdf.buildReportPdf(data, logo);
    return new Uint8Array(doc.output('arraybuffer'));
  }

  /**
   * Pide la ruta con el diálogo nativo, genera el PDF y lo escribe.
   * Devuelve la ruta guardada o `null` si el usuario canceló.
   */
  async download(account: Account, options: ReportOptions): Promise<string | null> {
    const path = await save({
      title: 'Guardar reporte PDF',
      defaultPath: reportFileName(account, options),
      filters: [{ name: 'PDF', extensions: ['pdf'] }],
    });
    if (!path) return null;
    const target = /\.pdf$/i.test(path) ? path : `${path}.pdf`;
    const bytes = await this.build(account, options);
    return this.api.saveReportPdf(target, bytesToBase64(bytes));
  }
}
