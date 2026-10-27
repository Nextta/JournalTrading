import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';

export interface ConfirmDialogData {
  title: string;
  message: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
  icon?: string;
  checkboxLabel?: string;
  checkboxChecked?: boolean;
}

export interface ConfirmDialogResult {
  confirmed: boolean;
  checked: boolean;
}

@Component({
  selector: 'app-confirm-dialog',
  imports: [MatDialogModule, MatButtonModule, MatCheckboxModule, FormsModule],
  template: `
    <div class="dialog-title">
      <span class="material-symbols-outlined">{{ data.icon ?? 'help' }}</span>
      <span>{{ data.title }}</span>
    </div>
    <div class="dialog-body">
      <p class="message">{{ data.message }}</p>
      @if (data.checkboxLabel) {
        <mat-checkbox [(ngModel)]="checked">{{ data.checkboxLabel }}</mat-checkbox>
      }
    </div>
    <div class="dialog-actions">
      <button mat-button (click)="close(false)">{{ data.cancelText ?? 'Cancelar' }}</button>
      <button
        mat-flat-button
        [color]="data.danger ? 'warn' : 'primary'"
        (click)="close(true)"
      >
        {{ data.confirmText ?? 'Confirmar' }}
      </button>
    </div>
  `,
  styles: `
    .message {
      color: var(--muted);
      font-size: 13px;
      line-height: 1.65;
      margin-bottom: 14px;
      white-space: pre-line;
    }
    mat-checkbox {
      font-size: 12.5px;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConfirmDialogComponent {
  protected readonly data = inject<ConfirmDialogData>(MAT_DIALOG_DATA);
  private readonly ref = inject(MatDialogRef<ConfirmDialogComponent, ConfirmDialogResult>);
  protected checked = this.data.checkboxChecked ?? true;

  protected close(confirmed: boolean): void {
    this.ref.close({ confirmed, checked: this.checked });
  }
}
