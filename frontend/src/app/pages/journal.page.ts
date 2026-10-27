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
import { MatCheckboxModule } from '@angular/material/checkbox';
import {
  MAT_DIALOG_DATA,
  MatDialog,
  MatDialogModule,
  MatDialogRef,
} from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSelectModule } from '@angular/material/select';
import { MatTooltipModule } from '@angular/material/tooltip';
import { RouterLink } from '@angular/router';
import { firstValueFrom } from 'rxjs';

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
  todayIso,
} from '../core/format';
import { dayFingerprint } from '../core/journal-day';
import type { DayAssetView, DayView, PlanItem, Strategy } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import {
  ConfirmDialogComponent,
  type ConfirmDialogResult,
} from '../shared/confirm-dialog.component';
import { ImageGalleryComponent } from '../shared/image-gallery.component';
import { JournalEditorComponent } from '../shared/journal-editor.component';

interface PlanSection {
  name: string;
  items: PlanItem[];
}

interface AddAssetData {
  strategies: Strategy[];
  suggestions: string[];
}

interface AddAssetResult {
  symbol: string;
  strategyId: string | null;
}

@Component({
  selector: 'app-journal-page',
  imports: [
    FormsModule,
    MatButtonModule,
    MatCheckboxModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatProgressBarModule,
    MatSelectModule,
    MatTooltipModule,
    RouterLink,
    JournalEditorComponent,
    ImageGalleryComponent,
  ],
  templateUrl: './journal.page.html',
  styleUrl: './journal.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class JournalPage {
  protected readonly store = inject(Store);
  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly today = todayIso();
  protected readonly date = signal(this.today);
  protected readonly loading = signal(false);
  protected readonly day = signal<DayView | null>(null);
  protected readonly expanded = signal<Set<string>>(new Set());
  protected readonly savingJournal = signal(false);
  protected readonly savingAssets = signal<Set<string>>(new Set());
  protected readonly newItemAssetId = signal<string | null>(null);
  protected newItemSection = 'Personalizado';
  protected newItemText = '';
  protected newItemRequired = false;
  private requestId = 0;
  private loadedKey: string | null = null;
  private dayHash = '';

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

  protected readonly dayWinRate = computed(() => {
    const total = this.dayTrades().length;
    return total ? (this.dayWins() / total) * 100 : 0;
  });

  protected readonly checklist = computed(() => {
    const items = this.day()?.assets.flatMap((asset) => asset.planItems) ?? [];
    const checked = items.filter((item) => item.checked).length;
    return { total: items.length, checked, pct: items.length ? (checked / items.length) * 100 : 0 };
  });

  protected readonly formatDateLong = formatDateLong;
  protected readonly formatDateTime = formatDateTime;
  protected readonly formatPrice = formatPrice;
  protected readonly formatR = formatR;
  protected readonly sideLabel = sideLabel;

  constructor() {
    effect(() => {
      const accountId = this.store.activeAccountId();
      const date = this.date();
      this.store.dataVersion();
      void this.load(accountId, date);
    });
  }

  private async load(
    accountId: string | null = this.store.activeAccountId(),
    date: string = this.date(),
  ): Promise<void> {
    if (!accountId) {
      this.requestId += 1;
      this.loadedKey = null;
      this.dayHash = '';
      this.day.set(null);
      this.loading.set(false);
      return;
    }
    const key = `${accountId}:${date}`;
    const viewChanged = key !== this.loadedKey;
    const requestId = ++this.requestId;
    if (viewChanged) this.loading.set(true);
    try {
      const day = await this.api.getDay(accountId, date);
      if (requestId !== this.requestId) return;
      const hash = dayFingerprint(day);
      if (viewChanged || hash !== this.dayHash) {
        this.day.set(day);
        this.dayHash = hash;
      }
      if (viewChanged && !this.expanded().size && day.assets.length) {
        this.expanded.set(new Set([day.assets[0].asset.id]));
      }
      this.loadedKey = key;
    } catch (error) {
      if (requestId === this.requestId) this.notifications.error(error);
    } finally {
      if (requestId === this.requestId) this.loading.set(false);
    }
  }

  protected setDate(value: string): void {
    if (!value) return;
    this.date.set(value);
    this.expanded.set(new Set());
  }

  protected moveDate(days: number): void {
    this.setDate(addDays(this.date(), days));
  }

  protected goToday(): void {
    this.setDate(this.today);
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

  protected checkedCount(items: PlanItem[]): number {
    return items.filter((item) => item.checked).length;
  }

  protected assetProgress(asset: DayAssetView): number {
    if (!asset.planItems.length) return 0;
    return (this.checkedCount(asset.planItems) / asset.planItems.length) * 100;
  }

  protected sections(asset: DayAssetView): PlanSection[] {
    const map = new Map<string, PlanItem[]>();
    for (const item of asset.planItems) {
      const name = item.section.trim() || 'General';
      const list = map.get(name) ?? [];
      list.push(item);
      map.set(name, list);
    }
    return Array.from(map, ([name, items]) => ({ name, items }));
  }

  protected isExpanded(assetId: string): boolean {
    return this.expanded().has(assetId);
  }

  protected toggleExpanded(assetId: string): void {
    this.expanded.update((current) => {
      const next = new Set(current);
      if (next.has(assetId)) next.delete(assetId);
      else next.add(assetId);
      return next;
    });
  }

  protected strategyColor(asset: DayAssetView): string {
    return (
      this.store.strategies().find((strategy) => strategy.id === asset.asset.strategyId)?.color ??
      '#4b5a72'
    );
  }

  protected async addAsset(): Promise<void> {
    const day = this.day();
    if (!day) return;
    const suggestions = Array.from(
      new Set([
        ...day.openTrades.map((trade) => trade.symbol),
        ...day.assets.map((asset) => asset.asset.symbol),
      ]),
    );
    const ref = this.dialog.open<AddAssetDialogComponent, AddAssetData, AddAssetResult>(
      AddAssetDialogComponent,
      {
        data: { strategies: this.store.strategies().filter((s) => !s.archived), suggestions },
        width: '460px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result) return;
    const account = this.store.activeAccount();
    if (!account) return;
    try {
      const asset = await this.api.createDayAsset(
        account.id,
        this.date(),
        result.symbol,
        result.strategyId,
      );
      this.expanded.update((current) => new Set([...current, asset.id]));
      this.notifications.success(`Activo ${asset.symbol} añadido`);
      await this.load();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async deleteAsset(asset: DayAssetView): Promise<void> {
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: `Eliminar ${asset.asset.symbol}`,
          message:
            'Se eliminará el activo del diario, su checklist, sus notas y sus capturas. Las operaciones no se borran, solo se desvinculan.',
          confirmText: 'Eliminar activo',
          danger: true,
          icon: 'delete',
        },
        width: '460px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.deleteDayAsset(asset.asset.id);
      this.notifications.success('Activo eliminado');
      await this.load();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async changeStrategy(asset: DayAssetView, strategyId: string | null): Promise<void> {
    if (strategyId === asset.asset.strategyId) return;
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: 'Cambiar estrategia',
          message: strategyId
            ? 'Se copiará la checklist de la estrategia en este activo. Si ya había una checklist, se reemplazará.'
            : 'Se quitará la estrategia y su checklist de este activo.',
          confirmText: 'Aplicar',
          icon: 'rule',
          checkboxLabel: 'Aplicar también a las operaciones de este activo',
          checkboxChecked: true,
        },
        width: '480px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.selectAssetStrategy(asset.asset.id, strategyId, result.checked);
      this.notifications.success('Estrategia actualizada');
      await this.load();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async saveGeneralJournal(html: string): Promise<void> {
    const day = this.day();
    if (!day) return;
    this.savingJournal.set(true);
    try {
      const journal = await this.api.saveDayJournal(day.journal.id, html);
      this.day.update((current) =>
        current ? { ...current, journal: { ...current.journal, generalHtml: journal.generalHtml } } : current,
      );
      this.dayHash = dayFingerprint(this.day());
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.savingJournal.set(false);
    }
  }

  protected isAssetSaving(assetId: string): boolean {
    return this.savingAssets().has(assetId);
  }

  protected async saveAssetJournal(asset: DayAssetView, html: string): Promise<void> {
    if (this.isAssetSaving(asset.asset.id)) return;
    this.savingAssets.update((current) => new Set([...current, asset.asset.id]));
    try {
      const updated = await this.api.saveAssetJournal(asset.asset.id, html);
      this.patchAsset(asset.asset.id, (current) => ({
        ...current,
        asset: { ...current.asset, journalHtml: updated.journalHtml },
      }));
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.savingAssets.update((current) => {
        const next = new Set(current);
        next.delete(asset.asset.id);
        return next;
      });
    }
  }

  protected async toggleItem(asset: DayAssetView, item: PlanItem, checked: boolean): Promise<void> {
    try {
      const updated = await this.api.togglePlanItem(item.id, checked);
      this.patchAsset(asset.asset.id, (current) => ({
        ...current,
        planItems: current.planItems.map((entry) =>
          entry.id === item.id ? updated : entry,
        ),
      }));
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected startAddItem(assetId: string): void {
    this.newItemAssetId.set(assetId);
    this.newItemText = '';
    this.newItemSection = 'Personalizado';
    this.newItemRequired = false;
  }

  protected cancelAddItem(): void {
    this.newItemAssetId.set(null);
    this.newItemText = '';
  }

  protected async submitAddItem(asset: DayAssetView): Promise<void> {
    const text = this.newItemText.trim();
    if (!text) return;
    try {
      const item = await this.api.addPlanItem(
        asset.asset.id,
        this.newItemSection,
        text,
        this.newItemRequired,
      );
      this.patchAsset(asset.asset.id, (current) => ({
        ...current,
        planItems: [...current.planItems, item],
      }));
      this.cancelAddItem();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async deleteItem(asset: DayAssetView, item: PlanItem): Promise<void> {
    try {
      await this.api.deletePlanItem(item.id);
      this.patchAsset(asset.asset.id, (current) => ({
        ...current,
        planItems: current.planItems.filter((entry) => entry.id !== item.id),
      }));
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async imagesChanged(): Promise<void> {
    await this.load();
  }

  private patchAsset(assetId: string, update: (asset: DayAssetView) => DayAssetView): void {
    this.day.update((day) => {
      if (!day) return day;
      return {
        ...day,
        assets: day.assets.map((asset) => (asset.asset.id === assetId ? update(asset) : asset)),
      };
    });
    this.dayHash = dayFingerprint(this.day());
  }
}

@Component({
  selector: 'app-add-asset-dialog',
  imports: [
    FormsModule,
    MatButtonModule,
    MatDialogModule,
    MatFormFieldModule,
    MatInputModule,
    MatSelectModule,
  ],
  template: `
    <div class="dialog-title">
      <span class="material-symbols-outlined">add_chart</span>
      <span>Añadir activo al día</span>
    </div>
    <div class="dialog-body">
      <mat-form-field appearance="outline" class="full">
        <mat-label>Activo / símbolo</mat-label>
        <input
          matInput
          [(ngModel)]="symbol"
          (ngModelChange)="symbol = $event.toUpperCase()"
          placeholder="EURUSD"
          autofocus
        />
      </mat-form-field>
      @if (data.suggestions.length) {
        <div class="suggestions">
          <span class="text-xs muted">Sugerencias:</span>
          @for (suggestion of data.suggestions; track suggestion) {
            <button type="button" class="suggestion" (click)="symbol = suggestion">
              {{ suggestion }}
            </button>
          }
        </div>
      }
      <mat-form-field appearance="outline" class="full">
        <mat-label>Estrategia</mat-label>
        <mat-select [(ngModel)]="strategyId">
          <mat-option [value]="null">Sin estrategia</mat-option>
          @for (strategy of data.strategies; track strategy.id) {
            <mat-option [value]="strategy.id">{{ strategy.name }}</mat-option>
          }
        </mat-select>
      </mat-form-field>
      <p class="text-xs muted hint">
        Puedes añadir la estrategia después. La checklist se copiará automáticamente en este activo.
      </p>
    </div>
    <div class="dialog-actions">
      <button mat-button mat-dialog-close>Cancelar</button>
      <button mat-flat-button color="primary" [disabled]="!symbol.trim()" (click)="submit()">
        Añadir activo
      </button>
    </div>
  `,
  styles: `
    .suggestions {
      display: flex;
      align-items: center;
      flex-wrap: wrap;
      gap: 6px;
      margin: -4px 0 14px;
    }
    .suggestion {
      border: 1px solid var(--border-strong);
      background: var(--surface-2);
      color: var(--text);
      border-radius: 999px;
      padding: 3px 9px;
      font-size: 10.5px;
      cursor: pointer;
    }
    .suggestion:hover {
      border-color: var(--accent);
      color: var(--accent);
    }
    .hint {
      margin-top: 4px;
      line-height: 1.5;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AddAssetDialogComponent {
  protected readonly data = inject<AddAssetData>(MAT_DIALOG_DATA);
  private readonly ref = inject(MatDialogRef<AddAssetDialogComponent, AddAssetResult>);
  protected symbol = '';
  protected strategyId: string | null = null;

  protected submit(): void {
    const symbol = this.symbol.trim().toUpperCase();
    if (!symbol) return;
    this.ref.close({ symbol, strategyId: this.strategyId });
  }
}
