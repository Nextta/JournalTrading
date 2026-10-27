import {
  ChangeDetectionStrategy,
  Component,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import {
  MAT_DIALOG_DATA,
  MatDialogModule,
  MatDialogRef,
} from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';

import { ApiService } from '../core/api.service';
import type { Account } from '../core/models';
import { NotificationsService } from '../core/notifications.service';

interface AccountEditorData {
  account: Account | null;
}

@Component({
  selector: 'app-account-editor-dialog',
  imports: [FormsModule, MatButtonModule, MatDialogModule, MatFormFieldModule, MatInputModule],
  template: `
    <div class="dialog-title">
      <span class="material-symbols-outlined">account_balance_wallet</span>
      <span>{{ data.account ? 'Editar cuenta' : 'Añadir cuenta' }}</span>
    </div>
    <mat-dialog-content class="dialog-body">
      @if (data.account) {
        <p class="text-xs muted note">
          El login y el servidor no se pueden cambiar. La cuenta ya está vinculada a su historial.
        </p>
      }
      <div class="form-grid">
        <mat-form-field appearance="outline" subscriptSizing="dynamic">
          <mat-label>Login</mat-label>
          <input
            matInput
            type="number"
            [(ngModel)]="login"
            [disabled]="!!data.account"
            placeholder="12345678"
          />
        </mat-form-field>
        <mat-form-field appearance="outline" subscriptSizing="dynamic">
          <mat-label>Servidor</mat-label>
          <input
            matInput
            [(ngModel)]="server"
            [disabled]="!!data.account"
            placeholder="Broker-Demo"
          />
        </mat-form-field>
        <mat-form-field appearance="outline" subscriptSizing="dynamic" class="full">
          <mat-label>Nombre visible</mat-label>
          <input matInput [(ngModel)]="displayName" placeholder="Cuenta principal" />
        </mat-form-field>
        <mat-form-field appearance="outline" subscriptSizing="dynamic">
          <mat-label>Divisa</mat-label>
          <input matInput [(ngModel)]="currency" placeholder="USD" maxlength="8" />
        </mat-form-field>
      </div>
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button type="button" (click)="close()">Cancelar</button>
      <button
        mat-flat-button
        color="primary"
        type="button"
        [disabled]="saving() || !valid()"
        (click)="save()"
      >
        {{ saving() ? 'Guardando…' : data.account ? 'Guardar cambios' : 'Añadir cuenta' }}
      </button>
    </mat-dialog-actions>
  `,
  styles: `
    .note {
      margin-bottom: 12px;
      line-height: 1.5;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AccountEditorDialogComponent {
  protected readonly data = inject<AccountEditorData>(MAT_DIALOG_DATA);
  private readonly api = inject(ApiService);
  private readonly ref = inject(MatDialogRef<AccountEditorDialogComponent, boolean>);
  private readonly notifications = inject(NotificationsService);

  protected login = this.data.account?.login ?? 0;
  protected server = this.data.account?.server ?? '';
  protected displayName = this.data.account?.displayName ?? '';
  protected currency = this.data.account?.currency ?? 'USD';
  protected readonly saving = signal(false);

  protected valid(): boolean {
    if (!this.server.trim() || !this.displayName.trim() || !this.currency.trim()) return false;
    return this.data.account ? true : this.login > 0;
  }

  protected async save(): Promise<void> {
    if (!this.valid() || this.saving()) return;
    this.saving.set(true);
    try {
      if (this.data.account) {
        await this.api.updateAccount(this.data.account.id, {
          displayName: this.displayName.trim(),
          currency: this.currency.trim().toUpperCase(),
        });
        this.notifications.success('Cuenta actualizada');
      } else {
        await this.api.createAccount({
          login: this.login,
          server: this.server.trim(),
          displayName: this.displayName.trim(),
          currency: this.currency.trim().toUpperCase(),
        });
        this.notifications.success('Cuenta añadida');
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
