import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MAT_DIALOG_DATA, MatDialog, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';

import { addDays, todayIso } from '../core/format';
import type { Account } from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import { REPORT_SECTIONS, type ReportSection } from '../core/report/report-data';
import { ReportService } from '../core/report/report.service';

export interface ReportDownloadDialogData {
  account: Account;
}

type RangeMode = 'all' | 'custom';

/** Opciones del reporte PDF: rango, secciones y privacidad. */
@Component({
  selector: 'app-report-download-dialog',
  imports: [
    FormsModule,
    MatButtonModule,
    MatCheckboxModule,
    MatDialogModule,
    MatProgressBarModule,
    MatSlideToggleModule,
  ],
  template: `
    <div class="dialog-title">
      <span class="material-symbols-outlined">picture_as_pdf</span>
      <span>Descargar reporte</span>
    </div>
    <div class="dialog-body">
      <p class="lead">
        Reporte en PDF de <strong>{{ data.account.displayName }}</strong> para compartir con otras
        personas.
      </p>

      <div class="block">
        <div class="block-title">Periodo</div>
        <div class="segmented" role="group" aria-label="Periodo del reporte">
          <button type="button" [class.active]="mode() === 'all'" (click)="mode.set('all')">
            Todo el historial
          </button>
          <button type="button" [class.active]="mode() === 'custom'" (click)="mode.set('custom')">
            Rango personalizado
          </button>
        </div>
        @if (mode() === 'custom') {
          <div class="range">
            <label class="date-field">
              <span>Desde</span>
              <input type="date" [max]="to()" [value]="from()" (change)="from.set($any($event.target).value)" />
            </label>
            <label class="date-field">
              <span>Hasta</span>
              <input type="date" [min]="from()" [value]="to()" (change)="to.set($any($event.target).value)" />
            </label>
          </div>
          <div class="chips">
            <button type="button" class="range-chip" (click)="setDays(30)">30 días</button>
            <button type="button" class="range-chip" (click)="setDays(90)">90 días</button>
            <button type="button" class="range-chip" (click)="setYear(0)">Este año</button>
            <button type="button" class="range-chip" (click)="setYear(-1)">Año anterior</button>
          </div>
          @if (!rangeValid()) {
            <p class="error">La fecha "Desde" debe ser anterior o igual a "Hasta".</p>
          }
        }
      </div>

      <div class="block">
        <div class="block-title">Secciones</div>
        <div class="sections">
          @for (section of sectionList; track section.key) {
            <mat-checkbox
              [checked]="sections().includes(section.key)"
              (change)="toggleSection(section.key, $event.checked)"
            >
              {{ section.label }}
            </mat-checkbox>
          }
          <mat-checkbox [(ngModel)]="includeTrades">
            Listado de operaciones
            <span class="hint">(puede ocupar varias páginas)</span>
          </mat-checkbox>
        </div>
      </div>

      <div class="block privacy">
        <mat-slide-toggle [(ngModel)]="hideSensitive">Ocultar datos sensibles</mat-slide-toggle>
        <p class="hint">
          Oculta login, servidor, balances, importes en dinero y volumen. Solo se muestran %, R y
          ratios.
        </p>
      </div>

      @if (busy()) {
        <mat-progress-bar mode="indeterminate" />
      }
    </div>
    <div class="dialog-actions">
      <button mat-button [disabled]="busy()" (click)="ref.close(null)">Cancelar</button>
      <button mat-flat-button color="primary" [disabled]="!canDownload()" (click)="download()">
        <span class="material-symbols-outlined">download</span>
        {{ busy() ? 'Generando…' : 'Descargar PDF' }}
      </button>
    </div>
  `,
  styles: `
    .lead {
      margin: 0 0 14px;
      color: var(--muted);
      font-size: 13px;
      line-height: 1.5;
      strong {
        color: var(--text);
      }
    }
    .block {
      margin-bottom: 16px;
    }
    .block-title {
      margin-bottom: 8px;
      color: var(--muted-2);
      font-size: 10px;
      font-weight: 750;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }
    .segmented {
      display: inline-flex;
      padding: 3px;
      border: 1px solid var(--border);
      border-radius: 9px;
      background: var(--surface-2);
      button {
        border: 0;
        border-radius: 7px;
        padding: 6px 12px;
        background: transparent;
        color: var(--muted);
        font: inherit;
        font-size: 12px;
        font-weight: 650;
        cursor: pointer;
        &.active {
          background: var(--accent-soft);
          color: var(--text);
        }
      }
    }
    .range {
      display: flex;
      gap: 10px;
      margin-top: 10px;
    }
    .date-field {
      display: flex;
      flex: 1;
      flex-direction: column;
      gap: 3px;
      padding: 5px 9px;
      border: 1px solid var(--border-strong);
      border-radius: 7px;
      background: var(--surface-2);
      span {
        color: var(--muted-2);
        font-size: 8.5px;
        font-weight: 750;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      input {
        border: 0;
        background: transparent;
        color: var(--text);
        font: inherit;
        font-size: 12px;
        outline: none;
        color-scheme: dark;
        cursor: pointer;
      }
    }
    .chips {
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
      margin-top: 8px;
    }
    .range-chip {
      border: 1px solid var(--border);
      background: transparent;
      color: var(--muted);
      border-radius: 999px;
      padding: 3px 10px;
      font-size: 11px;
      font-weight: 650;
      cursor: pointer;
      &:hover {
        color: var(--text);
        border-color: var(--border-strong);
        background: var(--surface-2);
      }
    }
    .sections {
      display: grid;
      grid-template-columns: 1fr;
      mat-checkbox {
        font-size: 12.5px;
      }
    }
    .hint {
      color: var(--muted-2);
      font-size: 11.5px;
      line-height: 1.5;
    }
    .privacy .hint {
      margin: 4px 0 0;
    }
    .error {
      margin: 8px 0 0;
      color: var(--neg);
      font-size: 12px;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ReportDownloadDialogComponent {
  protected readonly data = inject<ReportDownloadDialogData>(MAT_DIALOG_DATA);
  protected readonly ref = inject(MatDialogRef<ReportDownloadDialogComponent, string | null>);
  private readonly reports = inject(ReportService);
  private readonly notifications = inject(NotificationsService);

  protected readonly sectionList = REPORT_SECTIONS;
  protected readonly mode = signal<RangeMode>('all');
  protected readonly from = signal(addDays(todayIso(), -90));
  protected readonly to = signal(todayIso());
  protected readonly sections = signal<ReportSection[]>(REPORT_SECTIONS.map((section) => section.key));
  protected readonly busy = signal(false);
  protected includeTrades = false;
  protected hideSensitive = false;

  protected readonly rangeValid = computed(
    () => this.mode() === 'all' || (!!this.from() && !!this.to() && this.from() <= this.to()),
  );
  protected readonly canDownload = computed(() => !this.busy() && this.rangeValid());

  protected setDays(days: number): void {
    const today = todayIso();
    this.from.set(addDays(today, -days));
    this.to.set(today);
  }

  protected setYear(offset: number): void {
    const year = new Date().getFullYear() + offset;
    this.from.set(`${year}-01-01`);
    this.to.set(offset === 0 ? todayIso() : `${year}-12-31`);
  }

  protected toggleSection(key: ReportSection, checked: boolean): void {
    this.sections.update((current) =>
      checked ? [...new Set([...current, key])] : current.filter((item) => item !== key),
    );
  }

  protected async download(): Promise<void> {
    if (!this.canDownload()) return;
    this.busy.set(true);
    this.ref.disableClose = true;
    try {
      const custom = this.mode() === 'custom';
      const path = await this.reports.download(this.data.account, {
        from: custom ? this.from() : null,
        to: custom ? this.to() : null,
        sections: REPORT_SECTIONS.map((section) => section.key).filter((key) =>
          this.sections().includes(key),
        ),
        includeTrades: this.includeTrades,
        hideSensitive: this.hideSensitive,
      });
      if (path) {
        this.notifications.success(`Reporte guardado en ${path}`);
        this.ref.close(path);
      }
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.busy.set(false);
      this.ref.disableClose = false;
    }
  }
}

export function openReportDialog(dialog: MatDialog, account: Account): void {
  dialog.open<ReportDownloadDialogComponent, ReportDownloadDialogData, string | null>(
    ReportDownloadDialogComponent,
    { data: { account }, width: '520px', maxWidth: '94vw', autoFocus: false },
  );
}
