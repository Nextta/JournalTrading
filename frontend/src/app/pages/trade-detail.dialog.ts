import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import {
  MAT_DIALOG_DATA,
  MatDialogModule,
  MatDialogRef,
} from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSelectModule } from '@angular/material/select';
import { MatTooltipModule } from '@angular/material/tooltip';
import type { EChartsCoreOption } from 'echarts/core';

import { ApiService } from '../core/api.service';
import {
  formatDateTime,
  formatDuration,
  formatMoney,
  formatPercent,
  formatPrice,
  formatR,
  formatSignedMoney,
  sideLabel,
} from '../core/format';
import type {
  DayAssetView,
  Strategy,
  Trade,
  TradeSeries,
} from '../core/models';
import { TIMEFRAMES } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import { barIndexAt, formatLots, parseServerTime, resolveExitReason } from '../core/trade-chart';
import { ChartComponent } from '../shared/chart.component';
import { ImageGalleryComponent } from '../shared/image-gallery.component';

interface TradeDetailData {
  tradeId: string;
  accountId: string;
}

@Component({
  selector: 'app-trade-detail-dialog',
  imports: [
    FormsModule,
    MatButtonModule,
    MatDialogModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatProgressBarModule,
    MatSelectModule,
    MatTooltipModule,
    ChartComponent,
    ImageGalleryComponent,
  ],
  templateUrl: './trade-detail.dialog.html',
  styleUrl: './trade-detail.dialog.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TradeDetailDialogComponent {
  protected readonly data = inject<TradeDetailData>(MAT_DIALOG_DATA);
  private readonly api = inject(ApiService);
  private readonly ref = inject(MatDialogRef<TradeDetailDialogComponent, boolean>);
  private readonly notifications = inject(NotificationsService);
  protected readonly store = inject(Store);

  protected readonly trade = signal<Trade | null>(null);
  protected readonly loading = signal(true);
  protected readonly saving = signal(false);
  protected readonly notes = signal('');
  protected readonly timeframe = signal<string>('M15');
  protected readonly series = signal<TradeSeries | null>(null);
  protected readonly seriesLoading = signal(false);
  protected readonly seriesRequested = signal(false);
  protected readonly assets = signal<DayAssetView[]>([]);
  protected readonly timeframes = TIMEFRAMES;
  protected readonly formatDateTime = formatDateTime;
  protected readonly formatDuration = formatDuration;
  protected readonly formatPrice = formatPrice;
  protected readonly formatR = formatR;
  protected readonly sideLabel = sideLabel;
  private saved = false;
  private loadedTimeframe: string | null = null;
  private readonly requestedSeries = new Set<string>();

  protected readonly strategies = computed<Strategy[]>(() =>
    this.store.strategies().filter((strategy) => !strategy.archived),
  );

  protected readonly chartOptions = computed<EChartsCoreOption | null>(() => {
    const trade = this.trade();
    const series = this.series();
    if (!trade || !series?.bars.length) return null;
    const bars = series.bars;
    const labels = bars.map((bar) =>
      new Date(bar.time * 1000).toLocaleString('es-ES', {
        day: '2-digit',
        month: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        timeZone: 'UTC',
      }),
    );
    const entryIndex = barIndexAt(bars, parseServerTime(trade.openTimeServer), series.timeframe);
    const exitIndex =
      trade.closePrice === null
        ? null
        : barIndexAt(bars, parseServerTime(trade.closeTimeServer), series.timeframe);
    const isBuy = trade.side === 'buy';
    const sideColor = isBuy ? '#2dd4a0' : '#f2666f';
    const entryText = isBuy ? 'entry buy' : 'entry sell';
    const priceValues = bars.flatMap((bar) => [bar.low, bar.high]);
    if (entryIndex !== null) priceValues.push(trade.openPrice);
    if (trade.closePrice !== null) priceValues.push(trade.closePrice);
    const lowest = priceValues.reduce((min, value) => Math.min(min, value), Number.POSITIVE_INFINITY);
    const highest = priceValues.reduce(
      (max, value) => Math.max(max, value),
      Number.NEGATIVE_INFINITY,
    );
    const rawRange = Math.max(highest - lowest, 1e-9);
    const pricePadding = Math.max(rawRange * 0.18, Math.abs(highest) * 0.0002, 1e-8);
    const axisDecimals = Math.max(2, Math.min(6, Math.ceil(-Math.log10(rawRange)) + 2));
    const axisStep = 10 ** axisDecimals;
    const axisMin = Math.floor((lowest - pricePadding) * axisStep) / axisStep;
    const axisMax = Math.ceil((highest + pricePadding) * axisStep) / axisStep;

    const lines: Record<string, unknown>[] = [
      {
        yAxis: trade.openPrice,
        label: { formatter: 'Entrada', color: '#b8bfff', position: 'insideEndTop' },
        lineStyle: { color: '#6d7cff', type: 'dashed', width: 1 },
      },
    ];
    if (trade.closePrice !== null) {
      lines.push({
        yAxis: trade.closePrice,
        label: { formatter: 'Salida', color: '#e5e9f2', position: 'insideEndBottom' },
        lineStyle: { color: '#8a99b3', type: 'dashed', width: 1 },
      });
    }

    const markers: Record<string, unknown>[] = [];
    if (entryIndex !== null) {
      markers.push({
        name: 'Entrada',
        coord: [entryIndex, trade.openPrice],
        symbol: 'arrow',
        symbolSize: 12,
        symbolRotate: isBuy ? 0 : 180,
        symbolOffset: isBuy ? [0, 8] : [0, -8],
        itemStyle: { color: sideColor, borderColor: '#0b0f17', borderWidth: 1 },
        label: {
          show: true,
          position: isBuy ? 'bottom' : 'top',
          distance: 2,
          formatter: `${formatLots(trade.volumeInitial)} · ${entryText}`,
          color: sideColor,
          fontSize: 9.5,
          fontWeight: 700,
          backgroundColor: 'rgba(11, 15, 23, 0.84)',
          borderColor: sideColor,
          borderWidth: 1,
          borderRadius: 3,
          padding: [2, 4],
        },
      });
    }
    if (exitIndex !== null && trade.closePrice !== null) {
      const reason = resolveExitReason(trade.deals, trade.closePrice, trade.sl, trade.tp);
      markers.push({
        name: 'Salida',
        coord: [exitIndex, trade.closePrice],
        symbol: 'arrow',
        symbolSize: 12,
        symbolRotate: isBuy ? 0 : 180,
        symbolOffset: isBuy ? [0, 8] : [0, -8],
        itemStyle: { color: sideColor, borderColor: '#0b0f17', borderWidth: 1 },
        label: {
          show: true,
          position: isBuy ? 'bottom' : 'top',
          distance: 2,
          formatter: reason,
          color: sideColor,
          fontSize: 9.5,
          fontWeight: 700,
          backgroundColor: 'rgba(11, 15, 23, 0.84)',
          borderColor: sideColor,
          borderWidth: 1,
          borderRadius: 3,
          padding: [2, 4],
        },
      });
    }

    const linkData = bars.map((_, index) => {
      if (index === entryIndex) return trade.openPrice;
      if (index === exitIndex) return trade.closePrice;
      return null;
    });
    const linkSeries: Record<string, unknown>[] =
      entryIndex !== null && exitIndex !== null && trade.closePrice !== null
        ? [
            {
              name: 'Entrada → salida',
              type: 'line',
              data: linkData,
              connectNulls: true,
              symbol: 'none',
              silent: true,
              lineStyle: { color: sideColor, width: 1.4, opacity: 0.8 },
              z: 3,
            },
          ]
        : [];

    return {
      animation: false,
      backgroundColor: 'transparent',
      grid: { left: 58, right: 20, top: 42, bottom: 58 },
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'cross' },
        backgroundColor: 'rgba(15, 21, 33, 0.96)',
        borderColor: 'rgba(255, 255, 255, 0.12)',
        textStyle: { color: '#e5e9f2', fontSize: 11 },
      },
      xAxis: {
        type: 'category',
        data: labels,
        boundaryGap: true,
        axisLine: { lineStyle: { color: 'rgba(255,255,255,0.12)' } },
        axisLabel: { color: '#8a99b3', fontSize: 9, hideOverlap: true },
        splitLine: { show: false },
      },
      yAxis: {
        min: axisMin,
        max: axisMax,
        axisLabel: {
          color: '#8a99b3',
          fontSize: 9,
          formatter: (value: number) => value.toFixed(axisDecimals),
        },
        splitLine: { lineStyle: { color: 'rgba(255,255,255,0.05)' } },
      },
      series: [
        {
          type: 'candlestick',
          data: bars.map((bar) => [bar.open, bar.close, bar.low, bar.high]),
          itemStyle: {
            color: '#2dd4a0',
            color0: '#f2666f',
            borderColor: '#2dd4a0',
            borderColor0: '#f2666f',
          },
          markLine: {
            symbol: 'none',
            silent: true,
            data: lines,
          },
          markPoint: {
            symbol: 'arrow',
            symbolSize: 13,
            data: markers,
          },
        },
        ...linkSeries,
      ],
    };
  });

  constructor() {
    void this.loadTrade();
    effect(() => {
      const trade = this.trade();
      this.store.seriesVersion();
      const timeframe = this.timeframe();
      if (trade) void this.loadSeries(trade, timeframe);
    });
  }

  private async loadTrade(): Promise<void> {
    this.loading.set(true);
    try {
      const trade = await this.api.getTrade(this.data.tradeId);
      this.trade.set(trade);
      this.notes.set(trade.notes);
      const preferred = trade.seriesTimeframes.includes('M15')
        ? 'M15'
        : trade.seriesTimeframes[0] ?? 'M15';
      this.timeframe.set(preferred);
      try {
        const day = await this.api.getDay(this.data.accountId, trade.openTimeServer.slice(0, 10));
        this.assets.set(day.assets);
      } catch {
        this.assets.set([]);
      }
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.loading.set(false);
    }
  }

  private async loadSeries(trade: Trade, timeframe: string): Promise<void> {
    const key = `${trade.id}:${timeframe}`;
    if (this.loadedTimeframe !== timeframe) {
      this.loadedTimeframe = timeframe;
      this.series.set(null);
    }
    this.seriesLoading.set(true);
    try {
      const series = await this.api.getTradeSeries(trade.id, timeframe);
      if (this.timeframe() !== timeframe) return;
      if (series) {
        this.series.set(series);
        this.seriesRequested.set(false);
      } else if (!this.requestedSeries.has(key)) {
        this.requestedSeries.add(key);
        await this.api.requestTradeSeries(trade.id, timeframe);
        this.seriesRequested.set(true);
      }
    } catch (error) {
      this.notifications.error(error);
    } finally {
      if (this.timeframe() === timeframe) this.seriesLoading.set(false);
    }
  }

  protected selectTimeframe(timeframe: string): void {
    this.timeframe.set(timeframe);
  }

  protected money(value: number | null | undefined): string {
    return formatMoney(value, this.store.currency());
  }

  protected signed(value: number): string {
    return formatSignedMoney(value, this.store.currency());
  }

  protected percent(value: number | null | undefined): string {
    return formatPercent(value);
  }

  protected duration(trade: Trade): number {
    if (!trade.closeTimeUtc) return 0;
    const open = new Date(trade.openTimeUtc).getTime();
    const close = new Date(trade.closeTimeUtc).getTime();
    if (!Number.isFinite(open) || !Number.isFinite(close)) return 0;
    return Math.max(0, (close - open) / 60_000);
  }

  protected strategyColor(id: string | null): string {
    if (!id) return 'var(--border-strong)';
    return this.store.strategies().find((strategy) => strategy.id === id)?.color ?? 'var(--border-strong)';
  }

  protected hasAsset(id: string): boolean {
    return this.assets().some((asset) => asset.asset.id === id);
  }

  protected async saveNotes(): Promise<void> {
    const trade = this.trade();
    if (!trade || this.saving()) return;
    this.saving.set(true);
    try {
      const updated = await this.api.updateTradeNotes(trade.id, this.notes());
      this.trade.set(updated);
      this.notes.set(updated.notes);
      this.saved = true;
      this.notifications.success('Notas guardadas');
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.saving.set(false);
    }
  }

  protected async changeStrategy(strategyId: string | null): Promise<void> {
    const trade = this.trade();
    if (!trade || strategyId === trade.strategyId) return;
    try {
      const updated = await this.api.assignTradeStrategy(trade.id, strategyId);
      this.trade.set(updated);
      this.saved = true;
      this.notifications.success('Estrategia actualizada');
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async changeAsset(dayAssetId: string | null): Promise<void> {
    const trade = this.trade();
    if (!trade || dayAssetId === trade.dayAssetId) return;
    try {
      const updated = await this.api.assignTradeToAsset(trade.id, dayAssetId);
      this.trade.set(updated);
      this.saved = true;
      this.notifications.success('Activo del diario actualizado');
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async imagesChanged(): Promise<void> {
    try {
      this.trade.set(await this.api.getTrade(this.data.tradeId));
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected close(): void {
    this.ref.close(this.saved);
  }
}
