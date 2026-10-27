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
import { MatTooltipModule } from '@angular/material/tooltip';
import { firstValueFrom } from 'rxjs';

import { ApiService } from '../core/api.service';
import {
  addDays,
  formatDateShort,
  formatDuration,
  formatMoney,
  formatPercent,
  formatPrice,
  formatR,
  formatSignedMoney,
  formatTime,
  sideLabel,
  todayIso,
} from '../core/format';
import type { Strategy, Trade, TradeFilter, TradePage as TradePageResult } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import { TradeDetailDialogComponent } from './trade-detail.dialog';

@Component({
  selector: 'app-trades-page',
  imports: [
    FormsModule,
    MatButtonModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatProgressBarModule,
    MatSelectModule,
    MatTooltipModule,
  ],
  templateUrl: './trades.page.html',
  styleUrl: './trades.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TradesPage {
  protected readonly store = inject(Store);
  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly today = todayIso();
  protected readonly loading = signal(false);
  protected readonly page = signal<TradePageResult>({ items: [], total: 0 });
  protected readonly from = signal(addDays(this.today, -30));
  protected readonly to = signal(this.today);
  protected readonly state = signal<'open' | 'closed' | null>(null);
  protected readonly strategyId = signal<string | null>(null);
  protected readonly appliedSearch = signal('');
  protected readonly offset = signal(0);
  protected readonly limit = 50;
  protected draftSearch = '';
  private requestId = 0;

  protected readonly netTotal = computed(() =>
    this.page().items.reduce((total, trade) => total + trade.profitNet, 0),
  );

  protected readonly winRate = computed(() => {
    const items = this.page().items.filter((trade) => trade.state === 'closed');
    if (!items.length) return null;
    return (items.filter((trade) => trade.profitNet > 0).length / items.length) * 100;
  });

  protected readonly pageNumber = computed(() => Math.floor(this.offset() / this.limit) + 1);
  protected readonly pageCount = computed(() => Math.max(1, Math.ceil(this.page().total / this.limit)));

  protected readonly formatDateShort = formatDateShort;
  protected readonly formatDuration = formatDuration;
  protected readonly formatPrice = formatPrice;
  protected readonly formatR = formatR;
  protected readonly formatTime = formatTime;
  protected readonly sideLabel = sideLabel;

  constructor() {
    effect(() => {
      this.store.activeAccountId();
      this.store.dataVersion();
      this.from();
      this.to();
      this.state();
      this.strategyId();
      this.appliedSearch();
      this.offset();
      void this.load();
    });
  }

  private async load(): Promise<void> {
    const accountId = this.store.activeAccountId();
    if (!accountId) {
      this.page.set({ items: [], total: 0 });
      return;
    }
    const requestId = ++this.requestId;
    this.loading.set(true);
    try {
      const filter: TradeFilter = {
        accountId,
        from: this.from() || null,
        to: this.to() || null,
        state: this.state(),
        strategyIds: this.strategyId() ? [this.strategyId()!] : [],
        search: this.appliedSearch() || null,
        limit: this.limit,
        offset: this.offset(),
      };
      const page = await this.api.listTrades(filter);
      if (requestId !== this.requestId) return;
      this.page.set(page);
    } catch (error) {
      if (requestId === this.requestId) this.notifications.error(error);
    } finally {
      if (requestId === this.requestId) this.loading.set(false);
    }
  }

  protected applySearch(): void {
    this.offset.set(0);
    this.appliedSearch.set(this.draftSearch.trim());
  }

  protected setRange(days: number | null): void {
    this.offset.set(0);
    if (days === null) {
      this.from.set('');
      this.to.set(this.today);
    } else {
      this.from.set(addDays(this.today, -days));
      this.to.set(this.today);
    }
  }

  protected resetFilters(): void {
    this.draftSearch = '';
    this.appliedSearch.set('');
    this.from.set(addDays(this.today, -30));
    this.to.set(this.today);
    this.state.set(null);
    this.strategyId.set(null);
    this.offset.set(0);
  }

  protected prevPage(): void {
    if (this.offset() === 0) return;
    this.offset.update((value) => Math.max(0, value - this.limit));
  }

  protected nextPage(): void {
    if (this.offset() + this.limit >= this.page().total) return;
    this.offset.update((value) => value + this.limit);
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

  protected strategyName(trade: Trade): string {
    return trade.strategyName || '—';
  }

  protected duration(trade: Trade): number {
    if (!trade.closeTimeUtc) return 0;
    const open = new Date(trade.openTimeUtc).getTime();
    const close = new Date(trade.closeTimeUtc).getTime();
    if (!Number.isFinite(open) || !Number.isFinite(close)) return 0;
    return Math.max(0, (close - open) / 60_000);
  }

  protected strategyColor(trade: Trade): string {
    return this.strategyColorById(trade.strategyId);
  }

  protected strategyColorById(id: string | null): string {
    if (!id) return 'var(--border-strong)';
    return this.store.strategies().find((strategy) => strategy.id === id)?.color ?? 'var(--border-strong)';
  }

  protected strategies(): Strategy[] {
    return this.store.strategies().filter((strategy) => !strategy.archived);
  }

  protected async openTrade(trade: Trade): Promise<void> {
    const account = this.store.activeAccount();
    if (!account) return;
    const ref = this.dialog.open<TradeDetailDialogComponent, unknown, boolean>(
      TradeDetailDialogComponent,
      {
        data: { tradeId: trade.id, accountId: account.id },
        width: '1080px',
        maxWidth: '96vw',
        maxHeight: '92vh',
      },
    );
    const changed = await firstValueFrom(ref.afterClosed());
    if (changed) await this.load();
  }
}
