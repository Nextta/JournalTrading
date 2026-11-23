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
import { MatDialog } from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSelectModule } from '@angular/material/select';
import type { EChartsCoreOption } from 'echarts/core';

import { ApiService } from '../core/api.service';
import {
  addDays,
  formatDateNumeric,
  formatMoney,
  formatNumber,
  formatPercent,
  formatR,
  formatSignedMoney,
  hourLabel,
  monthLabel,
  todayIso,
  weekdayLabel,
} from '../core/format';
import type {
  BreakdownRow,
  DistributionBucket,
  StatsFilter,
  StatsResponse,
} from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import { ThemeService } from '../core/theme.service';
import { ChartComponent } from '../shared/chart.component';
import { openReportDialog } from './report-download.dialog';


@Component({
  selector: 'app-stats-page',
  imports: [
    FormsModule,
    MatButtonModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatProgressBarModule,
    MatSelectModule,
    ChartComponent,
  ],
  templateUrl: './stats.page.html',
  styleUrl: './stats.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class StatsPage {
  protected readonly store = inject(Store);
  private readonly theme = inject(ThemeService);
  private readonly api = inject(ApiService);
  private readonly notifications = inject(NotificationsService);
  private readonly dialog = inject(MatDialog);

  protected readonly today = todayIso();
  protected readonly loading = signal(false);
  protected readonly stats = signal<StatsResponse | null>(null);
  protected readonly from = signal(addDays(this.today, -90));
  protected readonly to = signal(this.today);
  protected readonly appliedSymbols = signal('');
  protected readonly strategyIds = signal<string[]>([]);
  protected draftSymbols = '';
  private requestId = 0;

  protected readonly symbols = computed(() =>
    this.appliedSymbols()
      .split(',')
      .map((symbol) => symbol.trim().toUpperCase())
      .filter(Boolean),
  );

  protected readonly equityOptions = computed<EChartsCoreOption>(() => {
    const stats = this.stats();
    const c = this.theme.chart();
    const points = stats?.equity ?? [];
    return {
      animation: false,
      backgroundColor: 'transparent',
      grid: { left: 64, right: 20, top: 20, bottom: 34 },
      tooltip: {
        trigger: 'axis',
        backgroundColor: c.tooltipBg,
        borderColor: c.tooltipBorder,
        textStyle: { color: c.tooltipText, fontSize: 11 },
      },
      xAxis: {
        type: 'category',
        data: points.map((point) => point.date),
        axisLine: { lineStyle: { color: c.axisLine } },
        axisLabel: { color: c.axis, fontSize: 9, hideOverlap: true },
        splitLine: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { color: c.axis, fontSize: 9 },
        splitLine: { lineStyle: { color: c.grid } },
      },
      series: [
        {
          type: 'line',
          data: points.map((point) => point.cumulative),
          smooth: 0.2,
          showSymbol: false,
          lineStyle: { color: c.accent, width: 1.8 },
          areaStyle: {
            color: {
              type: 'linear',
              x: 0,
              y: 0,
              x2: 0,
              y2: 1,
              colorStops: [
                { offset: 0, color: `rgba(${c.accentRgb}, 0.28)` },
                { offset: 1, color: `rgba(${c.accentRgb}, 0.01)` },
              ],
            },
          },
        },
      ],
    };
  });

  protected readonly dailyOptions = computed<EChartsCoreOption>(() => {
    const stats = this.stats();
    const c = this.theme.chart();
    const days = stats?.dailyPnl ?? [];
    return {
      animation: false,
      backgroundColor: 'transparent',
      grid: { left: 58, right: 16, top: 18, bottom: 30 },
      tooltip: {
        trigger: 'axis',
        backgroundColor: c.tooltipBg,
        borderColor: c.tooltipBorder,
        textStyle: { color: c.tooltipText, fontSize: 11 },
      },
      xAxis: {
        type: 'category',
        data: days.map((day) => day.date),
        axisLine: { lineStyle: { color: c.axisLine } },
        axisLabel: { color: c.axis, fontSize: 9, hideOverlap: true },
        splitLine: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { color: c.axis, fontSize: 9 },
        splitLine: { lineStyle: { color: c.grid } },
      },
      series: [
        {
          type: 'bar',
          data: days.map((day) => ({
            value: day.net,
            itemStyle: { color: day.net >= 0 ? c.pos : c.neg, opacity: 0.82 },
          })),
          barMaxWidth: 14,
        },
      ],
    };
  });

  protected readonly rOptions = computed<EChartsCoreOption>(() => {
    const stats = this.stats();
    const c = this.theme.chart();
    const buckets = stats?.rDistribution ?? [];
    return {
      animation: false,
      backgroundColor: 'transparent',
      grid: { left: 42, right: 16, top: 18, bottom: 30 },
      tooltip: {
        trigger: 'axis',
        backgroundColor: c.tooltipBg,
        borderColor: c.tooltipBorder,
        textStyle: { color: c.tooltipText, fontSize: 11 },
      },
      xAxis: {
        type: 'category',
        data: buckets.map((bucket) => bucket.label),
        axisLine: { lineStyle: { color: c.axisLine } },
        axisLabel: { color: c.axis, fontSize: 9 },
        splitLine: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { color: c.axis, fontSize: 9 },
        splitLine: { lineStyle: { color: c.grid } },
      },
      series: [
        {
          type: 'bar',
          data: buckets.map((bucket) => bucket.count),
          barMaxWidth: 26,
          itemStyle: { color: c.accent, opacity: 0.8, borderRadius: [3, 3, 0, 0] },
        },
      ],
    };
  });

  protected readonly formatDateNumeric = formatDateNumeric;
  protected readonly formatNumber = formatNumber;
  protected readonly formatR = formatR;

  protected openReport(): void {
    const account = this.store.activeAccount();
    if (account) openReportDialog(this.dialog, account);
  }

  constructor() {
    effect(() => {
      this.store.activeAccountId();
      this.store.dataVersion();
      this.from();
      this.to();
      this.symbols();
      this.strategyIds();
      void this.load();
    });
  }

  private async load(): Promise<void> {
    const accountId = this.store.activeAccountId();
    if (!accountId) {
      this.stats.set(null);
      return;
    }
    const requestId = ++this.requestId;
    this.loading.set(true);
    try {
      const filter: StatsFilter = {
        accountId,
        from: this.from() || null,
        to: this.to() || null,
        symbols: this.symbols(),
        strategyIds: this.strategyIds(),
      };
      const stats = await this.api.getStats(filter);
      if (requestId !== this.requestId) return;
      this.stats.set(stats);
    } catch (error) {
      if (requestId === this.requestId) this.notifications.error(error);
    } finally {
      if (requestId === this.requestId) this.loading.set(false);
    }
  }

  protected applySymbols(): void {
    this.appliedSymbols.set(this.draftSymbols);
  }

  protected setRange(days: number | null): void {
    if (days === null) {
      this.from.set('');
      this.to.set(this.today);
    } else {
      this.from.set(addDays(this.today, -days));
      this.to.set(this.today);
    }
  }

  protected reset(): void {
    this.draftSymbols = '';
    this.appliedSymbols.set('');
    this.strategyIds.set([]);
    this.from.set(addDays(this.today, -90));
    this.to.set(this.today);
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

  protected profitFactor(value: number | null | undefined): string {
    return value === null || value === undefined ? '—' : formatNumber(value, 2);
  }

  protected strategyName(id: string): string {
    return this.store.strategies().find((strategy) => strategy.id === id)?.name ?? id;
  }

  protected breakdownLabel(row: BreakdownRow): string {
    if (row.key.startsWith('weekday:')) return weekdayLabel(Number(row.key.split(':')[1]));
    if (row.key.startsWith('hour:')) return hourLabel(row.key.split(':')[1]);
    if (row.key.startsWith('strategy:')) return this.strategyName(row.key.slice(9));
    return row.label || row.key;
  }

  protected month(value: string): string {
    return monthLabel(value);
  }

  protected monthRows(): { month: string; net: number; trades: number; winRate: number }[] {
    return this.stats()?.monthly ?? [];
  }

  protected distributionTotal(buckets: DistributionBucket[]): number {
    return buckets.reduce((total, bucket) => total + bucket.count, 0);
  }
}
