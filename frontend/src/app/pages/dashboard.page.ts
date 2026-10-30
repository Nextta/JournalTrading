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
  addMonths,
  buildMonthWeeks,
  hasFullR,
  monthNumber,
  summarizePeriods,
  type CalendarWeek,
} from '../core/calendar';
import {
  addDays,
  formatDateLong,
  formatDateShort,
  formatDateTime,
  formatMoney,
  formatNumber,
  formatPercent,
  formatPrice,
  formatR,
  formatSignedMoney,
  monthLabel,
  sideLabel,
  startOfMonth,
  todayIso,
} from '../core/format';
import type {
  CalendarDay,
  CalendarMonth,
  CalendarPeriod,
  CalendarYear,
  DayView,
  PerformanceCalendar,
  PlanItem,
  StatsResponse,
  Trade,
  TradePage,
} from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import { ChartComponent } from '../shared/chart.component';

type Period = 'month' | '30d' | 'year';
type ValueMode = 'money' | 'percent' | 'r';

interface HeatmapRow {
  year: number;
  cells: Array<CalendarMonth | null>;
  total: CalendarYear;
}

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
  protected readonly calendar = signal<PerformanceCalendar | null>(null);
  protected readonly calendarLoading = signal(false);
  protected readonly calendarMonth = signal(this.today.slice(0, 7));
  protected readonly calendarMode = signal<ValueMode>('money');
  protected readonly heatmapMode = signal<ValueMode>('money');
  protected readonly weekdayColumns = ['Lun', 'Mar', 'Mié', 'Jue', 'Vie', 'Sáb', 'Dom'];
  protected readonly monthColumns = [
    'Ene',
    'Feb',
    'Mar',
    'Abr',
    'May',
    'Jun',
    'Jul',
    'Ago',
    'Sep',
    'Oct',
    'Nov',
    'Dic',
  ];
  private requestId = 0;
  private calendarRequestId = 0;

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

  protected readonly calendarMonthLabel = computed(() => monthLabel(this.calendarMonth()));

  protected readonly calendarWeeks = computed(() =>
    buildMonthWeeks(this.calendarMonth(), this.calendar()?.days ?? []),
  );

  protected readonly selectedMonth = computed(() => {
    const month = this.calendarMonth();
    return this.calendar()?.months.find((entry) => entry.month === month) ?? null;
  });

  protected readonly calendarRVisible = computed(() => hasFullR(this.selectedMonth()));

  protected readonly heatmapRVisible = computed(() =>
    (this.calendar()?.months ?? []).some((month) => hasFullR(month)),
  );

  protected readonly calendarMonthValue = computed(() =>
    this.periodValue(this.selectedMonth(), this.calendarMode()),
  );

  protected readonly calendarMaxAbs = computed(() => {
    const mode = this.calendarMode();
    const month = this.calendarMonth();
    let max = 0;
    for (const day of this.calendar()?.days ?? []) {
      if (!day.date.startsWith(`${month}-`)) continue;
      const value = this.periodValue(day, mode);
      if (value !== null) max = Math.max(max, Math.abs(value));
    }
    return max;
  });

  protected readonly heatmapRows = computed<HeatmapRow[]>(() => {
    const years = this.calendar()?.years ?? [];
    return [...years]
      .sort((left, right) => right.year - left.year)
      .map((year) => ({
        year: year.year,
        cells: Array.from(
          { length: 12 },
          (_, index) => year.months.find((month) => monthNumber(month.month) === index + 1) ?? null,
        ),
        total: year,
      }));
  });

  protected readonly heatmapHistory = computed<Array<CalendarPeriod | null>>(() => {
    const months = this.calendar()?.months ?? [];
    return Array.from({ length: 12 }, (_, index) =>
      summarizePeriods(months.filter((month) => monthNumber(month.month) === index + 1)),
    );
  });

  protected readonly heatmapTotals = computed(() => this.calendar()?.totals ?? null);

  protected readonly heatmapMaxAbs = computed(() => {
    const mode = this.heatmapMode();
    let max = 0;
    const consider = (period: CalendarPeriod | null) => {
      const value = this.periodValue(period, mode);
      if (value !== null) max = Math.max(max, Math.abs(value));
    };
    for (const row of this.heatmapRows()) {
      for (const cell of row.cells) consider(cell);
      consider(row.total);
    }
    for (const cell of this.heatmapHistory()) consider(cell);
    consider(this.heatmapTotals());
    return max;
  });

  protected readonly calendarSubtitle = computed(() => {
    const mode = this.calendarMode();
    if (mode === 'percent') return 'Retornos compuestos diarios · totales semanales';
    if (mode === 'r') return 'Múltiplos de riesgo · totales semanales';
    return `Resultados diarios en ${this.store.currency()} · totales semanales`;
  });

  protected readonly heatmapSubtitle = computed(() => {
    const mode = this.heatmapMode();
    if (mode === 'percent') return 'Retorno compuesto por mes y año';
    if (mode === 'r') return 'R por mes y año · los meses sin cobertura muestran —';
    return `Resultados mensuales y anuales en ${this.store.currency()}`;
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
      this.store.activeAccountId();
      this.store.dataVersion();
      this.period();
      void this.load();
    });

    effect(() => {
      const accountId = this.store.activeAccountId();
      this.store.dataVersion();
      void this.loadCalendar(accountId);
    });

    effect(() => {
      if (this.calendarMode() === 'r' && !this.calendarRVisible()) {
        this.calendarMode.set('money');
      }
    });

    effect(() => {
      if (this.heatmapMode() === 'r' && !this.heatmapRVisible()) {
        this.heatmapMode.set('money');
      }
    });
  }

  private async load(): Promise<void> {
    const accountId = this.store.activeAccountId();
    if (!accountId) {
      this.day.set(null);
      this.stats.set(null);
      this.recent.set([]);
      return;
    }
    const requestId = ++this.requestId;
    this.loading.set(true);
    try {
      const [day, stats, page] = await Promise.all([
        this.api.getDay(accountId, this.today),
        this.api.getStats({
          accountId,
          from: this.rangeFrom(),
          to: this.today,
        }),
        this.api.listTrades({
          accountId,
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

  private async loadCalendar(accountId: string | null): Promise<void> {
    if (!accountId) {
      this.calendarRequestId += 1;
      this.calendar.set(null);
      this.calendarLoading.set(false);
      return;
    }
    const requestId = ++this.calendarRequestId;
    this.calendarLoading.set(true);
    try {
      const calendar = await this.api.getPerformanceCalendar(accountId);
      if (requestId !== this.calendarRequestId) return;
      this.calendar.set(calendar);
    } catch (error) {
      if (requestId === this.calendarRequestId) this.notifications.error(error);
    } finally {
      if (requestId === this.calendarRequestId) this.calendarLoading.set(false);
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

  protected setCalendarMode(mode: ValueMode): void {
    if (mode === 'r' && !this.calendarRVisible()) return;
    this.calendarMode.set(mode);
  }

  protected setHeatmapMode(mode: ValueMode): void {
    if (mode === 'r' && !this.heatmapRVisible()) return;
    this.heatmapMode.set(mode);
  }

  protected moveCalendar(delta: number): void {
    this.calendarMonth.update((month) => addMonths(month, delta));
  }

  protected resetCalendarMonth(): void {
    this.calendarMonth.set(this.today.slice(0, 7));
  }

  protected dayNumber(date: string): number {
    return Number(date.slice(8, 10));
  }

  protected calendarDayText(day: CalendarDay | null): string {
    const value = this.periodValue(day, this.calendarMode());
    if (value !== null) return this.formatPeriodValue(value, this.calendarMode(), true);
    return day && day.trades > 0 ? '—' : '';
  }

  protected calendarWeekText(week: CalendarWeek): string {
    const value = this.periodValue(week.total, this.calendarMode());
    if (value !== null) return this.formatPeriodValue(value, this.calendarMode(), true);
    return week.total && week.total.trades > 0 ? '—' : '';
  }

  protected heatmapCellText(period: CalendarPeriod | null): string {
    const value = this.periodValue(period, this.heatmapMode());
    if (value !== null) return this.formatPeriodValue(value, this.heatmapMode(), true);
    return period && period.trades > 0 ? '—' : '';
  }

  protected heatmapTotalText(period: CalendarPeriod | null): string {
    const value = this.periodValue(period, this.heatmapMode());
    return value === null ? '—' : this.formatPeriodValue(value, this.heatmapMode(), true);
  }

  protected calendarDayBackground(day: CalendarDay | null): string {
    return this.periodBackground(day, this.calendarMode(), this.calendarMaxAbs());
  }

  protected heatmapCellBackground(period: CalendarPeriod | null): string {
    return this.periodBackground(period, this.heatmapMode(), this.heatmapMaxAbs());
  }

  protected calendarMonthTotalText(): string {
    const value = this.calendarMonthValue();
    return value === null ? '—' : this.formatPeriodValue(value, this.calendarMode(), false);
  }

  protected calendarDayTooltip(day: CalendarDay | null, date: string): string {
    const title = formatDateLong(date);
    if (!day || day.trades === 0) {
      return `${title}\nSin operaciones cerradas\nClic para abrir el diario`;
    }
    return `${this.periodTooltip(day, title)}\nClic para abrir el diario`;
  }

  protected weekTooltip(week: CalendarWeek): string {
    const title = `Semana del ${formatDateShort(week.start)} al ${formatDateShort(week.end)}`;
    return this.periodTooltip(week.total, title);
  }

  protected heatmapCellTooltip(
    period: CalendarPeriod | null,
    year: number,
    monthIndex: number,
  ): string {
    const label = monthLabel(`${year}-${String(monthIndex + 1).padStart(2, '0')}`);
    return this.periodTooltip(period, label);
  }

  protected heatmapYearTooltip(year: CalendarYear): string {
    return this.periodTooltip(year, `Año ${year.year}`);
  }

  protected heatmapHistoryTooltip(period: CalendarPeriod | null, monthIndex: number): string {
    return this.periodTooltip(period, `Total histórico · ${this.monthColumns[monthIndex]}`);
  }

  protected heatmapTotalTooltip(): string {
    return this.periodTooltip(this.heatmapTotals(), 'Total histórico');
  }

  private periodValue(period: CalendarPeriod | null | undefined, mode: ValueMode): number | null {
    if (!period || period.trades === 0) return null;
    if (mode === 'r') return hasFullR(period) ? period.rTotal : null;
    return mode === 'percent' ? period.pct : period.net;
  }

  private formatPeriodValue(value: number, mode: ValueMode, compact: boolean): string {
    if (mode === 'r') return formatR(value);
    if (mode === 'percent') return formatPercent(value, 2);
    return compact ? this.compactMoney(value) : formatSignedMoney(value, this.store.currency());
  }

  private compactMoney(value: number): string {
    const digits = Math.abs(value) >= 1000 ? 0 : 2;
    return `${value > 0 ? '+' : ''}${formatNumber(value, digits)}`;
  }

  private periodBackground(period: CalendarPeriod | null, mode: ValueMode, maxAbs: number): string {
    const value = this.periodValue(period, mode);
    if (value === null) {
      return period && period.trades > 0 ? 'rgba(139, 147, 167, 0.08)' : 'transparent';
    }
    if (value === 0) return 'rgba(139, 147, 167, 0.08)';
    const ratio = maxAbs > 0 ? Math.min(1, Math.abs(value) / maxAbs) : 0;
    const alpha = (0.08 + ratio * 0.3).toFixed(3);
    return value > 0 ? `rgba(45, 212, 160, ${alpha})` : `rgba(242, 102, 111, ${alpha})`;
  }

  private periodTooltip(period: CalendarPeriod | null, title: string): string {
    if (!period || period.trades === 0) return `${title}\nSin operaciones cerradas`;
    const lines = [
      title,
      `${period.trades} ${period.trades === 1 ? 'operación' : 'operaciones'} · ${period.wins}G / ${period.losses}P`,
      `Neto: ${formatSignedMoney(period.net, this.store.currency())}`,
      `Retorno: ${formatPercent(period.pct, 2)}`,
    ];
    if (hasFullR(period)) {
      lines.push(`R: ${formatR(period.rTotal)}`);
    } else if (period.rTrades > 0) {
      lines.push(`R: ${period.rTrades}/${period.trades} operaciones con dato`);
    }
    return lines.join('\n');
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
