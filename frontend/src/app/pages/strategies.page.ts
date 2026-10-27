import {
  ChangeDetectionStrategy,
  Component,
  effect,
  inject,
  signal,
} from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { firstValueFrom } from 'rxjs';

import { ApiService } from '../core/api.service';
import type { Strategy } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import {
  ConfirmDialogComponent,
  type ConfirmDialogResult,
} from '../shared/confirm-dialog.component';
import { StrategyEditorDialogComponent } from './strategy-editor.dialog';

@Component({
  selector: 'app-strategies-page',
  imports: [
    MatButtonModule,
    MatIconModule,
    MatProgressBarModule,
    MatTooltipModule,
  ],
  templateUrl: './strategies.page.html',
  styleUrl: './strategies.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class StrategiesPage {
  protected readonly store = inject(Store);
  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly loading = signal(false);
  protected readonly strategies = signal<Strategy[]>([]);
  protected readonly includeArchived = signal(false);

  constructor() {
    effect(() => {
      this.includeArchived();
      void this.load();
    });
  }

  private async load(): Promise<void> {
    this.loading.set(true);
    try {
      this.strategies.set(await this.api.listStrategies(this.includeArchived()));
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.loading.set(false);
    }
  }

  protected toggleArchived(): void {
    this.includeArchived.update((value) => !value);
  }

  protected sectionsCount(strategy: Strategy): number {
    return new Set(strategy.items.map((item) => item.section.trim() || 'General')).size;
  }

  protected previewItems(strategy: Strategy): Strategy['items'] {
    return strategy.items.slice(0, 4);
  }

  protected async create(): Promise<void> {
    const ref = this.dialog.open<StrategyEditorDialogComponent, unknown, boolean>(
      StrategyEditorDialogComponent,
      { data: { strategy: null }, width: '720px', maxWidth: '94vw' },
    );
    if (await firstValueFrom(ref.afterClosed())) await this.refresh();
  }

  protected async edit(strategy: Strategy): Promise<void> {
    const ref = this.dialog.open<StrategyEditorDialogComponent, unknown, boolean>(
      StrategyEditorDialogComponent,
      { data: { strategy }, width: '720px', maxWidth: '94vw' },
    );
    if (await firstValueFrom(ref.afterClosed())) await this.refresh();
  }

  protected async duplicate(strategy: Strategy): Promise<void> {
    try {
      await this.api.duplicateStrategy(strategy.id);
      this.notifications.success('Estrategia duplicada');
      await this.refresh();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async archive(strategy: Strategy): Promise<void> {
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: `Archivar ${strategy.name}`,
          message:
            'La estrategia dejará de estar disponible para nuevos días. El historial y las operaciones existentes no se modifican.',
          confirmText: 'Archivar',
          danger: true,
          icon: 'archive',
        },
        width: '460px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.archiveStrategy(strategy.id);
      this.notifications.success('Estrategia archivada');
      await this.refresh();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  private async refresh(): Promise<void> {
    await Promise.all([this.load(), this.store.refreshStrategies()]);
  }
}
