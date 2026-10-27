import { DecimalPipe } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { RouterLink } from '@angular/router';
import type { EChartsCoreOption } from 'echarts/core';

import { ApiService } from '../core/api.service';
import {
  addDays,
  formatDateLong,
  formatDateTime,
  formatMoney,
  formatPercent,
  formatPrice,
  formatR,
  formatSignedMoney,
  sideLabel,
  startOfMonth,
  todayIso,
} from '../core/format';
import type { DayView, PlanItem, StatsResponse, Trade, TradePage } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import { ChartComponent } from '../shared/chart.component';

type Period = 'month' | '30d' | 'year';

@Component({
  selector: 'app-dashboard-page',
  imports: [
    DecimalPipe,
    RouterLink,
    MatButtonModule,
    MatIconModule,
    MatProgressBarModule,
    MatTooltipModule,
    ChartComponent,
  ],
  templateUrl: './dashboard.page.html',
  styleUrl: './dashboard.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class DashboardPage {
  protected readonly store = inject(Store);
  private readonly api = inject(ApiService);
  private readonly notifications = inject(NotificationsService);

  protected readonly today = todayIso();
  protected readonly todayLabel = formatDateLong(this.today);
  protected readonly greeting = this.buildGreeting();
  protected readonly period = signal<Period>('month');
  protected readonly loading = signal(false);
  protected readonly day = signal<DayView | null>(null);
  protected readonly stats = signal<StatsResponse | null>(null);
  protected readonly recent = signal<Trade[]>([]);
  private requestId = 0;

  protected readonly dayTrades = computed(() => {
    const trades = this.day()?.assets.flatMap((asset) => asset.trades) ?? [];
    const seen = new Set<string>();
    return trades.filter((trade) => {
      if (seen.has(trade.id)) return false;
      seen.add(trade.id);
      return true;
    });
  });

  protected readonly dayNet = computed(() =>
    this.dayTrades().reduce((total, trade) => total + trade.profitNet, 0),
  );

  protected readonly dayWins = computed(
    () => this.dayTrades().filter((trade) => trade.profitNet > 0).length,
  );

  protected readonly openTrades = computed(() => this.day()?.openTrades ?? []);

  protected readonly openPnl = computed(() =>
    this.openTrades().reduce((total, trade) => total + trade.profitNet, 0),
  );

  protected readonly checklist = computed(() => {
    const items = this.day()?.assets.flatMap((asset) => asset.planItems) ?? [];
    const checked = items.filter((item) => item.checked).length;
    return { total: items.length, checked, pct: items.length ? (checked / items.length) * 100 : 0 };
  });

  protected readonly periodLabel = computed(() => {
    if (this.period() === '30d') return 'últimos 30 días';
    if (this.period() === 'year') return 'año en curso';
    return 'mes en curso';
  });

  protected readonly equityOptions = computed<EChartsCoreOption>(() => {
    const points = this.stats()?.equity ?? [];
    return {
      backgroundColor: 'transparent',
      animationDuration: 500,
      grid: { left: 56, right: 18, top: 18, bottom: 28 },
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#172132',
        borderColor: '#2a3950',
        textStyle: { color: '#e7ecf5', fontSize: 11 },
        formatter: (params: unknown) => {
          const list = params as Array<{ axisValue: string; value: number }>;
          if (!list.length) return '';
          return `${list[0].axisValue}<br/><b>${formatMoney(list[0].value, this.store.currency())}</b>`;
        },
      },
      xAxis: {
        type: 'category',
        data: points.map((point) => point.date),
        boundaryGap: false,
        axisLine: { lineStyle: { color: '#1e2a3d' } },
        axisTick: { show: false },
        axisLabel: { color: '#626c81', fontSize: 10, hideOverlap: true },
      },
      yAxis: {
        type: 'value',
        scale: true,
        axisLabel: { color: '#626c81', fontSize: 10 },
        splitLine: { lineStyle: { color: 'rgba(30, 42, 61, 0.65)' } },
      },
      series: [
        {
          type: 'line',
          data: points.map((point) => point.cumulative),
          smooth: true,
          symbol: 'none',
          lineStyle: { color: '#6d7cff', width: 2.2 },
          areaStyle: {
            color: {
              type: 'linear',
              x: 0,
              y: 0,
              x2: 0,
              y2: 1,
              colorStops: [
                { offset: 0, color: 'rgba(109, 124, 255, 0.34)' },
                { offset: 1, color: 'rgba(109, 124, 255, 0)' },
              ],
            },
          },
        },
      ],
    };
  });

  protected readonly formatDateTime = formatDateTime;
  protected readonly formatPrice = formatPrice;
  protected readonly formatR = formatR;
  protected readonly sideLabel = sideLabel;

  constructor() {
    effect(() => {
      this.store.activeAccount()?.id;
      this.store.dataVersion();
      this.period();
      void this.load();
    });
  }

  private async load(): Promise<void> {
    const account = this.store.activeAccount();
    if (!account) {
      this.day.set(null);
      this.stats.set(null);
      this.recent.set([]);
      return;
    }
    const requestId = ++this.requestId;
    this.loading.set(true);
    try {
      const [day, stats, page] = await Promise.all([
        this.api.getDay(account.id, this.today),
        this.api.getStats({
          accountId: account.id,
          from: this.rangeFrom(),
          to: this.today,
        }),
        this.api.listTrades({
          accountId: account.id,
          state: 'closed',
          limit: 6,
          offset: 0,
        }),
      ]);
      if (requestId !== this.requestId) return;
      this.day.set(day);
      this.stats.set(stats);
      this.recent.set((page as TradePage).items);
    } catch (error) {
      if (requestId === this.requestId) this.notifications.error(error);
    } finally {
      if (requestId === this.requestId) this.loading.set(false);
    }
  }

  private rangeFrom(): string {
    if (this.period() === '30d') return addDays(this.today, -29);
    if (this.period() === 'year') return `${this.today.slice(0, 4)}-01-01`;
    return startOfMonth();
  }

  protected setPeriod(period: Period): void {
    this.period.set(period);
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

  protected accountBalance(): number {
    return this.store.activeAccount()?.balance ?? 0;
  }

  protected accountEquity(): number {
    return this.store.activeAccount()?.equity ?? 0;
  }

  protected checkedCount(items: PlanItem[]): number {
    return items.filter((item) => item.checked).length;
  }

  protected profitFactorText(): string {
    const value = this.stats()?.summary.profitFactor;
    return value === null || value === undefined ? '—' : value.toFixed(2);
  }

  protected avgRText(): string {
    const value = this.stats()?.summary.avgR;
    return value === null || value === undefined ? '—' : formatR(value);
  }

  private buildGreeting(): string {
    const hour = new Date().getHours();
    if (hour < 12) return 'Buenos días';
    if (hour < 20) return 'Buenas tardes';
    return 'Buenas noches';
  }
}
