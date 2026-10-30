import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';
import { MatTooltipModule } from '@angular/material/tooltip';

import { ApiService } from '../core/api.service';
import type { AssetGroup, AssetGroupInput, AssetGroupItemInput, Strategy } from '../core/models';
import { NotificationsService } from '../core/notifications.service';

interface GroupEditorData {
  group: AssetGroup | null;
  strategies: Strategy[];
}

interface DraftItem {
  symbol: string;
  selection: string;
}

@Component({
  selector: 'app-group-editor-dialog',
  imports: [
    FormsModule,
    MatButtonModule,
    MatDialogModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatSelectModule,
    MatTooltipModule,
  ],
  templateUrl: './group-editor.dialog.html',
  styleUrl: './group-editor.dialog.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GroupEditorDialogComponent {
  protected readonly data = inject<GroupEditorData>(MAT_DIALOG_DATA);
  private readonly api = inject(ApiService);
  private readonly ref = inject(MatDialogRef<GroupEditorDialogComponent, boolean>);
  private readonly notifications = inject(NotificationsService);

  protected readonly colors = [
    '#6d7cff',
    '#2dd4a0',
    '#f5b84b',
    '#f2666f',
    '#8b5cf6',
    '#38bdf8',
    '#fb923c',
    '#94a3b8',
  ];

  protected readonly inheritValue = '__group__';
  protected readonly noneValue = '__none__';

  protected name = this.data.group?.name ?? '';
  protected description = this.data.group?.description ?? '';
  protected color = this.data.group?.color ?? this.colors[0];
  protected defaultStrategyId = this.data.group?.defaultStrategyId ?? null;
  protected readonly saving = signal(false);
  protected readonly items = signal<DraftItem[]>(
    this.data.group?.items.map((item) => ({
      symbol: item.symbol,
      selection: item.strategyOverridden ? (item.strategyId ?? this.noneValue) : this.inheritValue,
    })) ?? [{ symbol: '', selection: this.inheritValue }],
  );

  protected defaultStrategyName(): string {
    if (!this.defaultStrategyId) return 'sin estrategia común';
    return (
      this.data.strategies.find((strategy) => strategy.id === this.defaultStrategyId)?.name ??
      'estrategia no disponible'
    );
  }

  protected setSymbol(index: number, value: string): void {
    this.items.update((items) =>
      items.map((item, current) =>
        current === index ? { ...item, symbol: value.toUpperCase() } : item,
      ),
    );
  }

  protected addItem(): void {
    this.items.update((items) => [...items, { symbol: '', selection: this.inheritValue }]);
  }

  protected removeItem(index: number): void {
    this.items.update((items) => items.filter((_, current) => current !== index));
  }

  protected async save(): Promise<void> {
    const name = this.name.trim();
    if (!name) {
      this.notifications.error('El nombre del grupo es obligatorio');
      return;
    }

    const items: AssetGroupItemInput[] = [];
    const seen = new Set<string>();
    for (const item of this.items()) {
      const symbol = item.symbol.trim().toUpperCase();
      if (!symbol) continue;
      if (seen.has(symbol)) {
        this.notifications.error(`El activo ${symbol} está repetido en el grupo`);
        return;
      }
      seen.add(symbol);

      if (item.selection === this.inheritValue) {
        items.push({ symbol, strategyId: null, strategyOverridden: false });
      } else if (item.selection === this.noneValue) {
        items.push({ symbol, strategyId: null, strategyOverridden: true });
      } else {
        items.push({
          symbol,
          strategyId: item.selection,
          strategyOverridden: true,
        });
      }
    }

    if (!items.length) {
      this.notifications.error('Añade al menos un activo al grupo');
      return;
    }

    const input: AssetGroupInput = {
      name,
      description: this.description.trim(),
      color: this.color,
      defaultStrategyId: this.defaultStrategyId,
      items,
    };

    this.saving.set(true);
    try {
      if (this.data.group) {
        await this.api.updateAssetGroup(this.data.group.id, input);
      } else {
        await this.api.createAssetGroup(input);
      }
      this.ref.close(true);
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.saving.set(false);
    }
  }

  protected close(): void {
    this.ref.close(false);
  }
}
