import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatRadioModule } from '@angular/material/radio';

import type { AssetGroup, AssetGroupItem, ExistingAssetAction, Strategy } from '../core/models';

export interface AddGroupDialogData {
  groups: AssetGroup[];
  strategies: Strategy[];
  existingSymbols: string[];
}

export interface AddGroupDialogResult {
  groupId: string;
  existingAction: ExistingAssetAction;
  applyToTrades: boolean;
}

@Component({
  selector: 'app-add-group-dialog',
  imports: [
    FormsModule,
    MatButtonModule,
    MatCheckboxModule,
    MatDialogModule,
    MatIconModule,
    MatRadioModule,
  ],
  templateUrl: './add-group.dialog.html',
  styleUrl: './add-group.dialog.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AddGroupDialogComponent {
  protected readonly data = inject<AddGroupDialogData>(MAT_DIALOG_DATA);
  private readonly ref = inject(MatDialogRef<AddGroupDialogComponent, AddGroupDialogResult>);

  private readonly existingSymbols = new Set(this.data.existingSymbols);
  private readonly strategiesById = new Map(
    this.data.strategies.map((strategy) => [strategy.id, strategy]),
  );

  protected readonly selectedId = signal(this.data.groups[0]?.id ?? '');
  protected readonly selectedGroup = computed(
    () => this.data.groups.find((group) => group.id === this.selectedId()) ?? null,
  );
  protected readonly conflicts = computed(() => {
    const group = this.selectedGroup();
    if (!group) return [];
    return group.items.filter((item) => this.existingSymbols.has(item.symbol));
  });
  protected readonly conflictSymbols = computed(() =>
    this.conflicts()
      .map((item) => item.symbol)
      .join(', '),
  );

  protected existingAction: ExistingAssetAction = 'skip';
  protected applyToTrades = true;

  protected select(groupId: string): void {
    this.selectedId.set(groupId);
    this.existingAction = 'skip';
    this.applyToTrades = true;
  }

  protected isExisting(symbol: string): boolean {
    return this.existingSymbols.has(symbol);
  }

  protected conflictCount(group: AssetGroup): number {
    return group.items.filter((item) => this.existingSymbols.has(item.symbol)).length;
  }

  protected defaultStrategyName(group: AssetGroup): string {
    if (!group.defaultStrategyId) return 'Sin estrategia común';
    return this.strategiesById.get(group.defaultStrategyId)?.name ?? 'Estrategia no disponible';
  }

  protected defaultStrategyColor(group: AssetGroup): string {
    if (!group.defaultStrategyId) return '#4b5a72';
    return this.strategiesById.get(group.defaultStrategyId)?.color ?? '#4b5a72';
  }

  protected effectiveStrategyId(group: AssetGroup, item: AssetGroupItem): string | null {
    return item.strategyOverridden ? item.strategyId : group.defaultStrategyId;
  }

  protected inheritsDefault(group: AssetGroup, item: AssetGroupItem): boolean {
    return !item.strategyOverridden && group.defaultStrategyId !== null;
  }

  protected strategyName(group: AssetGroup, item: AssetGroupItem): string {
    const strategyId = this.effectiveStrategyId(group, item);
    if (!strategyId) return 'Sin estrategia';
    return this.strategiesById.get(strategyId)?.name ?? 'Estrategia no disponible';
  }

  protected strategyColor(group: AssetGroup, item: AssetGroupItem): string {
    const strategyId = this.effectiveStrategyId(group, item);
    if (!strategyId) return '#4b5a72';
    return this.strategiesById.get(strategyId)?.color ?? '#4b5a72';
  }

  protected submit(): void {
    const group = this.selectedGroup();
    if (!group || !group.items.length) return;
    const hasConflicts = this.conflicts().length > 0;
    this.ref.close({
      groupId: group.id,
      existingAction: hasConflicts ? this.existingAction : 'skip',
      applyToTrades: hasConflicts && this.existingAction === 'update' && this.applyToTrades,
    });
  }

  protected close(): void {
    this.ref.close();
  }
}
