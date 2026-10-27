import {
  ChangeDetectionStrategy,
  Component,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import {
  MAT_DIALOG_DATA,
  MatDialogModule,
  MatDialogRef,
} from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatTooltipModule } from '@angular/material/tooltip';

import { ApiService } from '../core/api.service';
import type { Strategy, StrategyInput, StrategyItemInput } from '../core/models';
import { NotificationsService } from '../core/notifications.service';

interface StrategyEditorData {
  strategy: Strategy | null;
}

interface DraftItem {
  section: string;
  text: string;
  required: boolean;
}

@Component({
  selector: 'app-strategy-editor-dialog',
  imports: [
    FormsModule,
    MatButtonModule,
    MatCheckboxModule,
    MatDialogModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatTooltipModule,
  ],
  templateUrl: './strategy-editor.dialog.html',
  styleUrl: './strategy-editor.dialog.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class StrategyEditorDialogComponent {
  protected readonly data = inject<StrategyEditorData>(MAT_DIALOG_DATA);
  private readonly api = inject(ApiService);
  private readonly ref = inject(MatDialogRef<StrategyEditorDialogComponent, boolean>);
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

  protected name = this.data.strategy?.name ?? '';
  protected description = this.data.strategy?.description ?? '';
  protected color = this.data.strategy?.color ?? this.colors[0];
  protected readonly saving = signal(false);
  protected readonly items = signal<DraftItem[]>(
    this.data.strategy?.items.map((item) => ({
      section: item.section,
      text: item.text,
      required: item.required,
    })) ?? [{ section: 'General', text: '', required: false }],
  );

  protected addItem(): void {
    const items = this.items();
    const lastSection = items.length ? items[items.length - 1].section : 'General';
    this.items.set([...items, { section: lastSection || 'General', text: '', required: false }]);
  }

  protected removeItem(index: number): void {
    this.items.update((items) => items.filter((_, current) => current !== index));
  }

  protected async save(): Promise<void> {
    const name = this.name.trim();
    if (!name) {
      this.notifications.error('El nombre de la estrategia es obligatorio');
      return;
    }
    const items: StrategyItemInput[] = this.items()
      .map((item) => ({
        id: null,
        section: item.section.trim() || 'General',
        text: item.text.trim(),
        required: item.required,
      }))
      .filter((item) => item.text.length > 0);
    const input: StrategyInput = {
      name,
      description: this.description.trim(),
      color: this.color,
      items,
    };
    this.saving.set(true);
    try {
      if (this.data.strategy) {
        await this.api.updateStrategy(this.data.strategy.id, input);
      } else {
        await this.api.createStrategy(input);
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
