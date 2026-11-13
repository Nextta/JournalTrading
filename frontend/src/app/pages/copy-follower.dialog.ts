import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatTooltipModule } from '@angular/material/tooltip';

import { ApiService } from '../core/api.service';
import { formatNumber, providerShort } from '../core/format';
import type {
  CopyAccount,
  CopyFollower,
  FollowerSettings,
  SizePreview,
  SizingMode,
  SymbolMapRow,
} from '../core/models';
import { NotificationsService } from '../core/notifications.service';

interface CopyFollowerDialogData {
  follower: CopyFollower;
  account: CopyAccount | undefined;
  master: CopyAccount | undefined;
}

interface ModeOption {
  value: SizingMode;
  label: string;
  hint: string;
  cfdOnly: boolean;
}

const MODES: ModeOption[] = [
  { value: 'same', label: 'Mismo tamaño que la principal', hint: 'Copia los mismos lotes o contratos.', cfdOnly: false },
  { value: 'fixed', label: 'Tamaño fijo', hint: 'Siempre el mismo tamaño, sin mirar la principal.', cfdOnly: false },
  { value: 'multiplier', label: 'Multiplicador', hint: 'Tamaño de la principal × multiplicador (×2: 1 → 2).', cfdOnly: false },
  { value: 'risk_balance', label: '% de riesgo sobre el balance', hint: 'Arriesga ese % del balance según la distancia al SL.', cfdOnly: true },
  { value: 'risk_equity', label: '% de riesgo sobre la equity', hint: 'Arriesga ese % de la equity según la distancia al SL.', cfdOnly: true },
  { value: 'kelly', label: 'Criterio de Kelly', hint: 'Kelly fraccional con el historial de la cuenta principal en el diario.', cfdOnly: true },
  { value: 'notional', label: 'Fórmula nocional', hint: 'Lotaje = (Balance / (Precio × contract size)) × multiplicador.', cfdOnly: true },
];

const DAYS = [
  { value: 1, label: 'L' },
  { value: 2, label: 'M' },
  { value: 3, label: 'X' },
  { value: 4, label: 'J' },
  { value: 5, label: 'V' },
  { value: 6, label: 'S' },
  { value: 7, label: 'D' },
];

@Component({
  selector: 'app-copy-follower-dialog',
  imports: [
    FormsModule,
    MatButtonModule,
    MatCheckboxModule,
    MatDialogModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatSelectModule,
    MatSlideToggleModule,
    MatTooltipModule,
  ],
  templateUrl: './copy-follower.dialog.html',
  styleUrl: './copy-follower.dialog.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CopyFollowerDialogComponent {
  protected readonly data = inject<CopyFollowerDialogData>(MAT_DIALOG_DATA);
  private readonly api = inject(ApiService);
  private readonly ref = inject(MatDialogRef<CopyFollowerDialogComponent, boolean>);
  private readonly notifications = inject(NotificationsService);

  protected readonly modes = MODES;
  protected readonly days = DAYS;
  protected readonly providerShort = providerShort;
  protected readonly futuresOnly = this.data.account?.provider !== 'mt5';

  protected settings: FollowerSettings = structuredClone(this.data.follower.settings);
  protected symbolMap: SymbolMapRow[] = this.data.follower.symbolMap.map((row) => ({ ...row }));
  protected symbolsText = this.settings.symbols.join(', ');

  protected readonly saving = signal(false);
  protected readonly preview = signal<SizePreview | null>(null);
  protected previewSymbol = '';
  protected previewVolume = 1;
  protected previewPrice: number | null = null;
  protected previewStop: number | null = null;

  protected modeHint(): string {
    return MODES.find((mode) => mode.value === this.settings.sizing.mode)?.hint ?? '';
  }

  protected modeIsCfd(): boolean {
    return MODES.find((mode) => mode.value === this.settings.sizing.mode)?.cfdOnly ?? false;
  }

  protected simpleModes(): ModeOption[] {
    return MODES.filter((mode) => !mode.cfdOnly);
  }

  protected toggleDay(day: number): void {
    const days = new Set(this.settings.scheduleDays);
    if (days.has(day)) days.delete(day);
    else days.add(day);
    this.settings.scheduleDays = [...days].sort();
  }

  protected addMapRow(): void {
    this.symbolMap = [...this.symbolMap, { masterSymbol: '', followerSymbol: '', volumeRatio: null }];
  }

  protected removeMapRow(index: number): void {
    this.symbolMap = this.symbolMap.filter((_, i) => i !== index);
  }

  private nullable(value: number | null | undefined): number | null {
    return value === null || value === undefined || !Number.isFinite(value) || value <= 0 ? null : value;
  }

  private collect(): FollowerSettings {
    const settings = structuredClone(this.settings);
    settings.symbols = this.symbolsText
      .split(/[,;\s]+/)
      .map((value) => value.trim())
      .filter((value) => value.length > 0);
    settings.maxVolumePerTrade = this.nullable(settings.maxVolumePerTrade);
    settings.maxPositionVolume = this.nullable(settings.maxPositionVolume);
    settings.dailyLossLimit = this.nullable(settings.dailyLossLimit);
    settings.maxDelaySeconds = Math.max(1, Math.round(settings.maxDelaySeconds || 5));
    settings.maxDeviation = Math.max(0, settings.maxDeviation || 0);
    return settings;
  }

  private collectMap(): SymbolMapRow[] {
    return this.symbolMap
      .map((row) => ({
        masterSymbol: row.masterSymbol.trim(),
        followerSymbol: row.followerSymbol.trim(),
        volumeRatio: this.nullable(row.volumeRatio),
      }))
      .filter((row) => row.masterSymbol && row.followerSymbol);
  }

  protected async runPreview(): Promise<void> {
    if (!this.previewSymbol.trim() || !this.previewVolume || !this.previewPrice) {
      this.notifications.info('Indica símbolo, tamaño y precio de la operación de ejemplo');
      return;
    }
    try {
      this.preview.set(
        await this.api.previewCopySize({
          followerId: this.data.follower.id,
          settings: this.collect(),
          symbolMap: this.collectMap(),
          symbol: this.previewSymbol.trim(),
          volume: this.previewVolume,
          price: this.previewPrice,
          stopLoss: this.nullable(this.previewStop),
        }),
      );
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected previewText(preview: SizePreview): string {
    if (!preview.ok || preview.volume === null) return preview.message;
    const unit = this.futuresOnly ? 'contratos' : 'lotes';
    const symbol = preview.symbol ? ` de ${preview.symbol}` : '';
    return `Se abrirían ${formatNumber(preview.volume, this.futuresOnly ? 0 : 2)} ${unit}${symbol}.`;
  }

  protected async save(): Promise<void> {
    this.saving.set(true);
    try {
      await this.api.updateCopyFollower(this.data.follower.id, this.collect(), this.collectMap());
      this.notifications.success('Configuración guardada');
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
