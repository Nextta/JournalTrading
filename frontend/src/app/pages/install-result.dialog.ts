import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MAT_DIALOG_DATA, MatDialogModule } from '@angular/material/dialog';

import type { InstallResult } from '../core/models';

interface InstallResultData {
  title: string;
  result: InstallResult;
}

/** Resultado de un instalador: mensaje, ruta de la configuración y salida completa. */
@Component({
  selector: 'app-install-result-dialog',
  imports: [MatButtonModule, MatDialogModule],
  template: `
    <div class="dialog-title">
      <span class="material-symbols-outlined" [class.ok]="data.result.ok" [class.bad]="!data.result.ok">
        {{ data.result.ok ? 'check_circle' : 'error' }}
      </span>
      <span>{{ data.title }}</span>
    </div>
    <div class="dialog-body">
      <p class="message">{{ data.result.message }}</p>
      @if (data.result.configPath) {
        <p class="path">Configuración escrita en <code>{{ data.result.configPath }}</code></p>
      }
      @if (data.result.log) {
        <button mat-button class="toggle" (click)="showLog.set(!showLog())">
          {{ showLog() ? 'Ocultar' : 'Ver' }} la salida del instalador
        </button>
        @if (showLog()) {
          <pre class="log">{{ data.result.log }}</pre>
        }
      }
    </div>
    <div class="dialog-actions">
      <button mat-flat-button color="primary" mat-dialog-close>Cerrar</button>
    </div>
  `,
  styles: `
    .ok {
      color: var(--pos);
    }
    .bad {
      color: var(--neg);
    }
    .message {
      color: var(--text);
      font-size: 13px;
      line-height: 1.6;
    }
    .path {
      margin-top: 8px;
      color: var(--muted);
      font-size: 12px;
      word-break: break-all;
    }
    .toggle {
      margin-top: 10px;
    }
    .log {
      max-height: 280px;
      margin-top: 8px;
      padding: 10px 12px;
      overflow: auto;
      border: 1px solid var(--border);
      border-radius: 10px;
      background: var(--surface-1);
      color: var(--muted);
      font-size: 11px;
      line-height: 1.5;
      white-space: pre-wrap;
      word-break: break-word;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class InstallResultDialogComponent {
  protected readonly data = inject<InstallResultData>(MAT_DIALOG_DATA);
  protected readonly showLog = signal(!this.data.result.ok);
}
