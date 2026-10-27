import {
  ChangeDetectionStrategy,
  Component,
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
import {
  formatDateTime,
  formatMoney,
  initials,
  timeAgo,
} from '../core/format';
import type { Account } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { Store } from '../core/store';
import {
  ConfirmDialogComponent,
  type ConfirmDialogResult,
} from '../shared/confirm-dialog.component';
import { AccountEditorDialogComponent } from './account-editor.dialog';

@Component({
  selector: 'app-accounts-page',
  imports: [MatButtonModule, MatIconModule, MatProgressBarModule, MatTooltipModule],
  templateUrl: './accounts.page.html',
  styleUrl: './accounts.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AccountsPage {
  protected readonly store = inject(Store);
  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly busy = signal(false);
  protected readonly restarting = signal(false);
  protected readonly showArchived = signal(false);

  protected readonly formatDateTime = formatDateTime;
  protected readonly timeAgo = timeAgo;

  protected money(value: number | null | undefined, currency: string): string {
    return formatMoney(value, currency);
  }

  protected accountInitials(account: Account): string {
    return initials(account.displayName || account.server);
  }

  protected toggleArchived(): void {
    this.showArchived.update((value) => !value);
  }

  protected async create(): Promise<void> {
    const ref = this.dialog.open<AccountEditorDialogComponent, unknown, boolean>(
      AccountEditorDialogComponent,
      { data: { account: null }, width: '520px' },
    );
    if (!(await firstValueFrom(ref.afterClosed()))) return;
    try {
      await Promise.all([this.store.refreshAccounts(), this.store.refreshSettings()]);
      this.notifications.success('Cuenta creada');
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async edit(account: Account): Promise<void> {
    const ref = this.dialog.open<AccountEditorDialogComponent, unknown, boolean>(
      AccountEditorDialogComponent,
      { data: { account }, width: '520px' },
    );
    if (!(await firstValueFrom(ref.afterClosed()))) return;
    await this.store.refreshAccounts();
  }

  protected async use(account: Account): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    try {
      await this.store.setActiveAccount(account.id);
      this.notifications.success(`Cuenta ${account.displayName} activada`);
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async confirm(account: Account): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    try {
      await this.api.confirmAccount(account.id);
      await Promise.all([this.store.refreshAccounts(), this.store.refreshSettings()]);
      this.notifications.success(`Cuenta ${account.login} confirmada`);
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async archive(account: Account): Promise<void> {
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: `Archivar ${account.displayName}`,
          message:
            'La cuenta dejará de aparecer en el selector y no se actualizará desde MetaTrader 5. Sus datos históricos se conservan.',
          confirmText: 'Archivar cuenta',
          danger: true,
          icon: 'archive',
        },
        width: '460px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.archiveAccount(account.id);
      await Promise.all([this.store.refreshAccounts(), this.store.refreshSettings()]);
      this.notifications.success('Cuenta archivada');
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async restartServer(): Promise<void> {
    if (this.restarting()) return;
    this.restarting.set(true);
    try {
      await this.api.restartMt5Server();
      await this.store.refreshSettings();
      this.notifications.success('Servidor MT5 reiniciado');
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.restarting.set(false);
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
