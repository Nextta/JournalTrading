import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';

import type { CopyToggleOptions } from '../core/models';

interface CopyPauseDialogData {
  title: string;
  message: string;
}

/** Pausar la copia: por defecto lo copiado se deja como está. */
@Component({
  selector: 'app-copy-pause-dialog',
  imports: [FormsModule, MatButtonModule, MatCheckboxModule, MatDialogModule],
  template: `
    <div class="dialog-title">
      <span class="material-symbols-outlined">pause_circle</span>
      <span>{{ data.title }}</span>
    </div>
    <div class="dialog-body">
      <p class="message">{{ data.message }}</p>
      <mat-checkbox [(ngModel)]="cancelPending">Cancelar también las órdenes pendientes copiadas</mat-checkbox>
      <mat-checkbox [(ngModel)]="closePositions">Cerrar también las posiciones copiadas</mat-checkbox>
    </div>
    <div class="dialog-actions">
      <button mat-button (click)="ref.close(null)">Cancelar</button>
      <button mat-flat-button color="primary" (click)="confirm()">Pausar</button>
    </div>
  `,
  styles: `
    .message {
      margin-bottom: 12px;
      color: var(--muted);
      font-size: 13px;
      line-height: 1.6;
    }
    mat-checkbox {
      display: block;
      font-size: 12.5px;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CopyPauseDialogComponent {
  protected readonly data = inject<CopyPauseDialogData>(MAT_DIALOG_DATA);
  protected readonly ref = inject(MatDialogRef<CopyPauseDialogComponent, CopyToggleOptions | null>);
  protected cancelPending = false;
  protected closePositions = false;

  protected confirm(): void {
    this.ref.close({ cancelPending: this.cancelPending, closePositions: this.closePositions });
  }
}
