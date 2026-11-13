import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';

import { providerShort } from '../core/format';
import type { CopyAccount, CopyGroup, CopyOverview } from '../core/models';

interface CopyGroupDialogData {
  group: CopyGroup | null;
  overview: CopyOverview | null;
}

export interface CopyGroupDialogResult {
  name: string;
  masterAccountId: string;
}

@Component({
  selector: 'app-copy-group-dialog',
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
      <span class="material-symbols-outlined">content_copy</span>
      <span>{{ data.group ? 'Editar grupo de copia' : 'Nuevo grupo de copia' }}</span>
    </div>
    <mat-dialog-content class="dialog-body">
      <mat-form-field appearance="outline" subscriptSizing="dynamic" class="full">
        <mat-label>Nombre</mat-label>
        <input matInput [(ngModel)]="name" placeholder="Ej. Fondeo NQ" autofocus />
      </mat-form-field>

      <mat-form-field appearance="outline" subscriptSizing="dynamic" class="full">
        <mat-label>Cuenta principal (donde operas)</mat-label>
        <mat-select [(ngModel)]="masterAccountId">
          @for (account of candidates; track account.id) {
            <mat-option [value]="account.id">
              {{ account.displayName }} · {{ providerShort(account.provider) }}
              {{ account.provider === 'atas' ? '' : '· ' + account.login }}
            </mat-option>
          }
        </mat-select>
      </mat-form-field>

      <p class="hint">
        Las operaciones de esta cuenta se enviarán a las secundarias del grupo. Una cuenta no puede
        ser principal y secundaria a la vez. Solo aparecen las cuentas activas.
      </p>
    </mat-dialog-content>
    <div class="dialog-actions">
      <button mat-button (click)="ref.close()">Cancelar</button>
      <button mat-flat-button color="primary" [disabled]="!valid()" (click)="save()">
        {{ data.group ? 'Guardar' : 'Crear grupo' }}
      </button>
    </div>
  `,
  styles: `
    .full {
      width: 100%;
      margin-top: 12px;
    }
    .hint {
      margin-top: 12px;
      color: var(--muted);
      font-size: 12px;
      line-height: 1.55;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CopyGroupDialogComponent {
  protected readonly data = inject<CopyGroupDialogData>(MAT_DIALOG_DATA);
  protected readonly ref = inject(MatDialogRef<CopyGroupDialogComponent, CopyGroupDialogResult>);
  protected readonly providerShort = providerShort;

  protected name = this.data.group?.name ?? '';
  protected masterAccountId = this.data.group?.masterAccountId ?? '';
  protected readonly candidates: CopyAccount[] = this.buildCandidates();

  private buildCandidates(): CopyAccount[] {
    const overview = this.data.overview;
    if (!overview) return [];
    const followerIds = new Set(
      overview.groups.flatMap((group) => group.followers.map((follower) => follower.accountId)),
    );
    const otherMasters = new Set(
      overview.groups
        .filter((group) => group.id !== this.data.group?.id)
        .map((group) => group.masterAccountId),
    );
    return overview.accounts.filter(
      (account) =>
        (account.status === 'active' || account.id === this.data.group?.masterAccountId) &&
        !followerIds.has(account.id) &&
        !otherMasters.has(account.id),
    );
  }

  protected valid(): boolean {
    return this.name.trim().length > 0 && this.masterAccountId.length > 0;
  }

  protected save(): void {
    if (!this.valid()) return;
    this.ref.close({ name: this.name.trim(), masterAccountId: this.masterAccountId });
  }
}
