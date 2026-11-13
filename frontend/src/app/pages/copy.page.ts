import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatMenuModule } from '@angular/material/menu';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatTooltipModule } from '@angular/material/tooltip';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { firstValueFrom } from 'rxjs';

import { ApiService } from '../core/api.service';
import { formatDateTime, formatMoney, formatNumber, providerShort } from '../core/format';
import type {
  CopyAccount,
  CopyFollower,
  CopyGroup,
  CopyLogEntry,
  CopyOverview,
  CopyToggleOptions,
  SizingMode,
} from '../core/models';
import { NotificationsService } from '../core/notifications.service';
import {
  ConfirmDialogComponent,
  type ConfirmDialogResult,
} from '../shared/confirm-dialog.component';
import { CopyFollowerDialogComponent } from './copy-follower.dialog';
import { CopyGroupDialogComponent, type CopyGroupDialogResult } from './copy-group.dialog';
import { CopyPauseDialogComponent } from './copy-pause.dialog';

const SIZING_LABELS: Record<SizingMode, string> = {
  same: 'Mismo tamaño',
  fixed: 'Tamaño fijo',
  multiplier: 'Multiplicador',
  risk_balance: '% riesgo (balance)',
  risk_equity: '% riesgo (equity)',
  kelly: 'Kelly',
  notional: 'Fórmula nocional',
};

const ACTION_LABELS: Record<string, string> = {
  market: 'Mercado',
  close: 'Cierre',
  sltp: 'SL/TP',
  pending_place: 'Pendiente',
  pending_modify: 'Modificar pendiente',
  pending_cancel: 'Cancelar pendiente',
  pending_fill: 'Pendiente ejecutada',
  open: 'Apertura',
  increase: 'Ampliación',
  reduce: 'Cierre parcial',
};

const STATUS_LABELS: Record<string, string> = {
  pending: 'En cola',
  sent: 'Enviada',
  filled: 'Ejecutada',
  placed: 'Colocada',
  done: 'Hecha',
  rejected: 'Rechazada',
  skipped: 'Saltada',
};

@Component({
  selector: 'app-copy-page',
  imports: [
    FormsModule,
    MatButtonModule,
    MatIconModule,
    MatMenuModule,
    MatProgressBarModule,
    MatSlideToggleModule,
    MatTooltipModule,
  ],
  templateUrl: './copy.page.html',
  styleUrl: './copy.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CopyPage {
  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly loading = signal(false);
  protected readonly overview = signal<CopyOverview | null>(null);
  protected readonly log = signal<CopyLogEntry[]>([]);
  protected readonly logFilter = signal<string | null>(null);

  protected readonly accounts = computed(() => this.overview()?.accounts ?? []);
  protected readonly groups = computed(() => this.overview()?.groups ?? []);
  protected readonly enabled = computed(() => this.overview()?.enabled ?? false);
  protected readonly onlineCount = computed(() => this.accounts().filter((a) => a.online).length);
  protected readonly followerCount = computed(() =>
    this.groups().reduce((total, group) => total + group.followers.length, 0),
  );
  protected readonly logFilterName = computed(() => {
    const id = this.logFilter();
    if (!id) return 'Todos los grupos';
    return this.groups().find((group) => group.id === id)?.name ?? 'Grupo eliminado';
  });
  protected readonly openCopies = computed(() =>
    this.groups().reduce(
      (total, group) => total + group.followers.reduce((sum, f) => sum + f.openPositions, 0),
      0,
    ),
  );

  protected readonly formatMoney = formatMoney;
  protected readonly formatNumber = formatNumber;
  protected readonly formatDateTime = formatDateTime;
  protected readonly providerShort = providerShort;

  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private logTimer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    void this.load();
    const unlisten: Array<Promise<UnlistenFn>> = [
      this.api.onCopyUpdated(() => this.scheduleRefresh()),
      this.api.onCopyActivity(() => this.scheduleLog()),
    ];
    const poll = setInterval(() => void this.refreshOverview(), 5000);
    inject(DestroyRef).onDestroy(() => {
      clearInterval(poll);
      if (this.refreshTimer) clearTimeout(this.refreshTimer);
      if (this.logTimer) clearTimeout(this.logTimer);
      for (const promise of unlisten) void promise.then((fn) => fn()).catch(() => undefined);
    });
  }

  private async load(): Promise<void> {
    this.loading.set(true);
    try {
      await Promise.all([this.refreshOverview(), this.refreshLog()]);
    } finally {
      this.loading.set(false);
    }
  }

  private scheduleRefresh(): void {
    if (this.refreshTimer) return;
    this.refreshTimer = setTimeout(() => {
      this.refreshTimer = null;
      void this.refreshOverview();
    }, 250);
  }

  private scheduleLog(): void {
    if (this.logTimer) return;
    this.logTimer = setTimeout(() => {
      this.logTimer = null;
      void this.refreshLog();
      void this.refreshOverview();
    }, 400);
  }

  private async refreshOverview(): Promise<void> {
    try {
      this.overview.set(await this.api.getCopyOverview());
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async refreshLog(): Promise<void> {
    try {
      this.log.set(await this.api.listCopyLog(this.logFilter(), 150));
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected setLogFilter(groupId: string | null): void {
    this.logFilter.set(groupId);
    void this.refreshLog();
  }

  // --- Datos derivados -----------------------------------------------------

  protected account(id: string): CopyAccount | undefined {
    return this.accounts().find((account) => account.id === id);
  }

  protected accountName(id: string): string {
    return this.account(id)?.displayName ?? 'Cuenta eliminada';
  }

  protected accountLine(account: CopyAccount | undefined): string {
    if (!account) return '';
    const login = account.provider === 'atas' ? '' : ` · ${account.login}`;
    return `${providerShort(account.provider)}${login} · ${account.server}`;
  }

  protected availableFollowers(group: CopyGroup): CopyAccount[] {
    const masters = new Set(this.groups().map((g) => g.masterAccountId));
    const inGroup = new Set(group.followers.map((f) => f.accountId));
    return this.accounts().filter(
      (account) =>
        account.status === 'active' &&
        account.id !== group.masterAccountId &&
        !masters.has(account.id) &&
        !inGroup.has(account.id),
    );
  }

  protected sizingSummary(follower: CopyFollower): string {
    const sizing = follower.settings.sizing;
    const label = SIZING_LABELS[sizing.mode];
    switch (sizing.mode) {
      case 'fixed':
        return `${label} · ${formatNumber(sizing.fixedVolume, 2)}`;
      case 'multiplier':
        return `${label} · ×${formatNumber(sizing.multiplier, 2)}`;
      case 'risk_balance':
      case 'risk_equity':
        return `${label} · ${formatNumber(sizing.riskPercent, 2)} %`;
      case 'kelly':
        return `${label} · ${formatNumber(sizing.kellyFraction, 2)} (máx. ${formatNumber(sizing.kellyMaxRiskPercent, 1)} %)`;
      case 'notional':
        return `${label} · ×${formatNumber(sizing.notionalMultiplier, 2)}`;
      default:
        return label;
    }
  }

  protected compatibilityText(follower: CopyFollower): string {
    if (follower.compatibility === 'futures_only')
      return 'Solo se copian futuros: los CFD/Forex de la principal se saltan';
    if (follower.compatibility === 'broker_futures')
      return 'Solo si el bróker de MT5 ofrece el mismo futuro';
    return '';
  }

  protected followerState(follower: CopyFollower): { label: string; tone: string } {
    if (!follower.enabled) return { label: 'Desactivada', tone: 'off' };
    if (follower.pausedReason) return { label: 'Pausada', tone: 'warn' };
    return { label: 'Activa', tone: 'on' };
  }

  protected actionLabel(entry: CopyLogEntry): string {
    const action = ACTION_LABELS[entry.action] ?? entry.action;
    if (entry.action === 'market' && entry.signalKind) {
      return ACTION_LABELS[entry.signalKind] ?? action;
    }
    return action;
  }

  protected statusLabel(status: string): string {
    return STATUS_LABELS[status] ?? status;
  }

  protected statusTone(status: string): string {
    if (status === 'filled' || status === 'placed' || status === 'done') return 'ok';
    if (status === 'rejected') return 'bad';
    if (status === 'skipped') return 'warn';
    return 'wait';
  }

  protected slippage(entry: CopyLogEntry): string {
    if (entry.fillPrice === null || entry.masterPrice === null || entry.action !== 'market') return '';
    const diff = entry.fillPrice - entry.masterPrice;
    if (Math.abs(diff) < 1e-12) return '0';
    return (diff > 0 ? '+' : '') + formatNumber(diff, 5).replace(/0+$/, '').replace(/,$/, '');
  }

  // --- Acciones --------------------------------------------------------------

  private async askPause(title: string, message: string): Promise<CopyToggleOptions | null> {
    const ref = this.dialog.open<CopyPauseDialogComponent, unknown, CopyToggleOptions | null>(
      CopyPauseDialogComponent,
      { data: { title, message }, width: '480px', maxWidth: '94vw' },
    );
    return (await firstValueFrom(ref.afterClosed())) ?? null;
  }

  protected async toggleGlobal(enabled: boolean): Promise<void> {
    try {
      if (enabled) {
        await this.api.setCopyEnabled(true);
        this.notifications.success('Copy trading activado');
      } else {
        const options = await this.askPause(
          'Pausar el copy trading',
          'No se copiará ninguna operación nueva. Lo que ya está copiado se queda abierto salvo que marques lo contrario.',
        );
        if (!options) {
          await this.refreshOverview();
          return;
        }
        await this.api.setCopyEnabled(false, options);
        this.notifications.info('Copy trading pausado');
      }
      await this.refreshOverview();
    } catch (error) {
      this.notifications.error(error);
      await this.refreshOverview();
    }
  }

  protected async toggleGroup(group: CopyGroup, enabled: boolean): Promise<void> {
    try {
      if (enabled) {
        await this.api.setCopyGroupEnabled(group.id, true);
      } else {
        const options = await this.askPause(
          `Pausar ${group.name}`,
          'Las secundarias de este grupo dejarán de copiar operaciones nuevas.',
        );
        if (!options) {
          await this.refreshOverview();
          return;
        }
        await this.api.setCopyGroupEnabled(group.id, false, options);
      }
      await this.refreshOverview();
    } catch (error) {
      this.notifications.error(error);
      await this.refreshOverview();
    }
  }

  protected async toggleFollower(follower: CopyFollower, enabled: boolean): Promise<void> {
    try {
      if (enabled) {
        await this.api.setCopyFollowerEnabled(follower.id, true);
      } else {
        const options = await this.askPause(
          `Desactivar ${this.accountName(follower.accountId)}`,
          'Esta cuenta dejará de copiar operaciones.',
        );
        if (!options) {
          await this.refreshOverview();
          return;
        }
        await this.api.setCopyFollowerEnabled(follower.id, false, options);
      }
      await this.refreshOverview();
    } catch (error) {
      this.notifications.error(error);
      await this.refreshOverview();
    }
  }

  protected async resume(follower: CopyFollower): Promise<void> {
    try {
      await this.api.resumeCopyFollower(follower.id);
      this.notifications.success('Copia reanudada');
      await this.refreshOverview();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async panic(group: CopyGroup | null): Promise<void> {
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: group ? `Pánico en ${group.name}` : 'Botón de pánico',
          message: group
            ? 'Se cerrarán todas las posiciones copiadas, se cancelarán las órdenes pendientes copiadas de este grupo y se pausará el grupo.\n\nLas operaciones manuales y las de la cuenta principal no se tocan.'
            : 'Se cerrarán todas las posiciones copiadas, se cancelarán todas las órdenes pendientes copiadas y se pausará el copy trading.\n\nLas operaciones manuales y las de las cuentas principales no se tocan.',
          confirmText: 'Cerrar todo y pausar',
          danger: true,
          icon: 'emergency',
        },
        width: '480px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      const count = await this.api.copyPanic(group?.id ?? null);
      this.notifications.info(
        count ? `${count} órdenes de cierre/cancelación enviadas` : 'No había nada copiado abierto',
      );
      await this.load();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async createGroup(): Promise<void> {
    await this.openGroupDialog(null);
  }

  protected async editGroup(group: CopyGroup): Promise<void> {
    await this.openGroupDialog(group);
  }

  private async openGroupDialog(group: CopyGroup | null): Promise<void> {
    const ref = this.dialog.open<CopyGroupDialogComponent, unknown, CopyGroupDialogResult>(
      CopyGroupDialogComponent,
      {
        data: { group, overview: this.overview() },
        width: '520px',
        maxWidth: '94vw',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result) return;
    try {
      if (group) {
        await this.api.updateCopyGroup(group.id, result.name, result.masterAccountId);
        this.notifications.success('Grupo actualizado');
      } else {
        await this.api.createCopyGroup(result.name, result.masterAccountId);
        this.notifications.success('Grupo creado. Añade las cuentas secundarias.');
      }
      await this.refreshOverview();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async deleteGroup(group: CopyGroup): Promise<void> {
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: `Eliminar ${group.name}`,
          message:
            'Se borra la configuración del grupo y su registro. Las posiciones ya copiadas siguen abiertas en las secundarias y dejarán de gestionarse.',
          confirmText: 'Eliminar grupo',
          danger: true,
          icon: 'delete',
        },
        width: '470px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.deleteCopyGroup(group.id);
      this.notifications.success('Grupo eliminado');
      await this.load();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async addFollower(group: CopyGroup, account: CopyAccount): Promise<void> {
    try {
      await this.api.addCopyFollower(group.id, account.id);
      this.notifications.success(`${account.displayName} copiará a ${this.accountName(group.masterAccountId)}`);
      await this.refreshOverview();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async configure(group: CopyGroup, follower: CopyFollower): Promise<void> {
    const ref = this.dialog.open<CopyFollowerDialogComponent, unknown, boolean>(
      CopyFollowerDialogComponent,
      {
        data: {
          follower,
          account: this.account(follower.accountId),
          master: this.account(group.masterAccountId),
        },
        width: '780px',
        maxWidth: '96vw',
        maxHeight: '92vh',
      },
    );
    if (await firstValueFrom(ref.afterClosed())) await this.refreshOverview();
  }

  protected async removeFollower(follower: CopyFollower): Promise<void> {
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: `Quitar ${this.accountName(follower.accountId)}`,
          message:
            'La cuenta deja de copiar este grupo. Las posiciones que ya estén copiadas siguen abiertas y dejarán de gestionarse.',
          confirmText: 'Quitar del grupo',
          danger: true,
          icon: 'person_remove',
        },
        width: '470px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.removeCopyFollower(follower.id);
      await this.refreshOverview();
    } catch (error) {
      this.notifications.error(error);
    }
  }
}
