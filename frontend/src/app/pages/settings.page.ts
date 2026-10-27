import {
  ChangeDetectionStrategy,
  Component,
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
import type { SettingsResponse } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import {
  ConfirmDialogComponent,
  type ConfirmDialogResult,
} from '../shared/confirm-dialog.component';

@Component({
  selector: 'app-settings-page',
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
  templateUrl: './settings.page.html',
  styleUrl: './settings.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SettingsPage {
  protected readonly store = inject(Store);
  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly settings = signal<SettingsResponse | null>(null);
  protected readonly saving = signal(false);
  protected readonly regenerating = signal(false);
  protected readonly backingUp = signal(false);
  protected readonly showKey = signal(false);
  protected port = 4477;
  protected timezoneMode: 'broker' | 'local' = 'broker';

  constructor() {
    effect(() => {
      const settings = this.store.settings();
      if (!settings) return;
      this.settings.set(settings);
      this.port = settings.apiPort;
      this.timezoneMode = settings.timezoneMode;
    });
  }

  protected maskedKey(key: string): string {
    if (this.showKey()) return key;
    if (key.length <= 8) return '••••••••';
    return `${key.slice(0, 4)}••••••••${key.slice(-4)}`;
  }

  protected endpoint(): string {
    return `http://127.0.0.1:${this.settings()?.apiPort ?? this.port}/api/v1`;
  }

  protected async copy(value: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
      this.notifications.success('Copiado al portapapeles');
    } catch {
      this.notifications.error('No se pudo copiar al portapapeles');
    }
  }

  protected async save(): Promise<void> {
    if (this.saving()) return;
    if (!Number.isFinite(this.port) || this.port < 1024 || this.port > 65535) {
      this.notifications.error('El puerto debe estar entre 1024 y 65535');
      return;
    }
    this.saving.set(true);
    try {
      const updated = await this.api.updateSettings({
        apiPort: Math.round(this.port),
        timezoneMode: this.timezoneMode,
      });
      this.settings.set(updated);
      await this.store.refreshSettings();
      this.notifications.success('Ajustes guardados');
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.saving.set(false);
    }
  }

  protected async regenerate(): Promise<void> {
    if (this.regenerating()) return;
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: 'Regenerar API key',
          message:
            'La clave actual dejará de funcionar inmediatamente. Tendrás que actualizarla en el Expert Advisor de MetaTrader 5.',
          confirmText: 'Regenerar clave',
          danger: true,
          icon: 'key',
        },
        width: '460px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    this.regenerating.set(true);
    try {
      const key = await this.api.regenerateApiKey();
      this.settings.update((current) => (current ? { ...current, apiKey: key } : current));
      await this.store.refreshSettings();
      this.notifications.success('Nueva API key generada');
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.regenerating.set(false);
    }
  }

  protected async backup(): Promise<void> {
    if (this.backingUp()) return;
    this.backingUp.set(true);
    try {
      const path = await this.api.createBackup();
      this.notifications.success(`Copia creada: ${path}`);
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.backingUp.set(false);
    }
  }

  protected async openDataDir(): Promise<void> {
    try {
      await this.api.openDataDir();
    } catch (error) {
      this.notifications.error(error);
    }
  }
}
