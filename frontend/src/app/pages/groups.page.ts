import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { firstValueFrom } from 'rxjs';

import { ApiService } from '../core/api.service';
import type { AssetGroup, AssetGroupItem, Strategy } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import {
  ConfirmDialogComponent,
  type ConfirmDialogResult,
} from '../shared/confirm-dialog.component';
import { GroupEditorDialogComponent } from './group-editor.dialog';

@Component({
  selector: 'app-groups-page',
  imports: [MatButtonModule, MatIconModule, MatProgressBarModule, MatTooltipModule],
  templateUrl: './groups.page.html',
  styleUrl: './groups.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GroupsPage {
  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly loading = signal(false);
  protected readonly groups = signal<AssetGroup[]>([]);
  protected readonly strategies = signal<Strategy[]>([]);

  constructor() {
    void this.load();
  }

  private async load(): Promise<void> {
    this.loading.set(true);
    try {
      const [groups, strategies] = await Promise.all([
        this.api.listAssetGroups(),
        this.api.listStrategies(true),
      ]);
      this.groups.set(groups);
      this.strategies.set(strategies);
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.loading.set(false);
    }
  }

  private strategyForId(strategyId: string | null): Strategy | undefined {
    if (!strategyId) return undefined;
    return this.strategies().find((strategy) => strategy.id === strategyId);
  }

  protected defaultStrategyName(group: AssetGroup): string {
    return this.strategyForId(group.defaultStrategyId)?.name ?? 'Sin estrategia común';
  }

  protected defaultStrategyColor(group: AssetGroup): string {
    return this.strategyForId(group.defaultStrategyId)?.color ?? '#4b5a72';
  }

  protected effectiveStrategyId(group: AssetGroup, item: AssetGroupItem): string | null {
    return item.strategyOverridden ? item.strategyId : group.defaultStrategyId;
  }

  protected strategyName(group: AssetGroup, item: AssetGroupItem): string {
    return this.strategyForId(this.effectiveStrategyId(group, item))?.name ?? 'Sin estrategia';
  }

  protected strategyColor(group: AssetGroup, item: AssetGroupItem): string {
    return this.strategyForId(this.effectiveStrategyId(group, item))?.color ?? '#4b5a72';
  }

  protected inheritedStrategy(group: AssetGroup, item: AssetGroupItem): boolean {
    return !item.strategyOverridden && group.defaultStrategyId !== null;
  }

  protected withStrategyCount(group: AssetGroup): number {
    return group.items.filter((item) => this.effectiveStrategyId(group, item) !== null).length;
  }

  protected previewItems(group: AssetGroup): AssetGroupItem[] {
    return group.items.slice(0, 5);
  }

  protected async create(): Promise<void> {
    await this.openEditor(null);
  }

  protected async edit(group: AssetGroup): Promise<void> {
    await this.openEditor(group);
  }

  private async openEditor(group: AssetGroup | null): Promise<void> {
    const ref = this.dialog.open<GroupEditorDialogComponent, unknown, boolean>(
      GroupEditorDialogComponent,
      {
        data: { group, strategies: this.strategies() },
        width: '700px',
        maxWidth: '94vw',
      },
    );
    if (await firstValueFrom(ref.afterClosed())) await this.load();
  }

  protected async remove(group: AssetGroup): Promise<void> {
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: `Eliminar ${group.name}`,
          message:
            'El grupo dejará de estar disponible para añadirlo al diario. Los activos que ya hayas añadido a tus días no se modifican.',
          confirmText: 'Eliminar grupo',
          danger: true,
          icon: 'delete',
        },
        width: '470px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.deleteAssetGroup(group.id);
      this.notifications.success('Grupo eliminado');
      await this.load();
    } catch (error) {
      this.notifications.error(error);
    }
  }
}
