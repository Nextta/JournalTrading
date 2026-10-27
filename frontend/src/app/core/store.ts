import { computed, inject, Injectable, signal } from '@angular/core';

import { ApiService, type SeriesUpdatedEvent } from './api.service';
import type {
  Account,
  Bootstrap,
  Mt5Status,
  SettingsResponse,
  Strategy,
} from './models';

@Injectable({ providedIn: 'root' })
export class Store {
  private readonly api = inject(ApiService);
  private initialized = false;

  readonly loading = signal(true);
  readonly error = signal<string | null>(null);
  readonly accounts = signal<Account[]>([]);
  readonly strategies = signal<Strategy[]>([]);
  readonly settings = signal<SettingsResponse | null>(null);
  readonly dataVersion = signal(0);
  readonly seriesVersion = signal(0);
  readonly lastMt5Event = signal<string | null>(null);

  readonly activeAccount = computed(() => {
    const settings = this.settings();
    const accounts = this.accounts();
    return (
      accounts.find((account) => account.id === settings?.activeAccountId) ??
      accounts.find((account) => account.status === 'active') ??
      null
    );
  });

  readonly activeAccounts = computed(() =>
    this.accounts().filter((account) => account.status === 'active'),
  );

  readonly pendingAccounts = computed(() =>
    this.accounts().filter((account) => account.status === 'pending'),
  );

  readonly archivedAccounts = computed(() =>
    this.accounts().filter((account) => account.status === 'archived'),
  );

  readonly mt5 = computed<Mt5Status | null>(() => this.settings()?.mt5 ?? null);

  readonly mt5Connected = computed(() => {
    const status = this.mt5();
    if (!status?.connected || !status.lastHeartbeatAt) return false;
    const parsed = new Date(status.lastHeartbeatAt).getTime();
    return Number.isFinite(parsed) && Date.now() - parsed < 20_000;
  });

  readonly currency = computed(() => this.activeAccount()?.currency ?? 'USD');

  async init(): Promise<void> {
    if (this.initialized) return;
    this.initialized = true;
    this.loading.set(true);
    this.error.set(null);
    try {
      const bootstrap = await this.api.getBootstrap();
      this.applyBootstrap(bootstrap);
      await this.bindEvents();
    } catch (error) {
      this.error.set(this.message(error));
    } finally {
      this.loading.set(false);
    }
  }

  private applyBootstrap(bootstrap: Bootstrap): void {
    this.accounts.set(bootstrap.accounts);
    this.strategies.set(bootstrap.strategies);
    this.settings.set(bootstrap.settings);
  }

  private async bindEvents(): Promise<void> {
    await this.api.onMt5Status((status) => {
      this.settings.update((current) =>
        current ? { ...current, mt5: status } : current,
      );
    });
    await this.api.onMt5Heartbeat((payload) => {
      const accountId = payload['accountId'] as string | undefined;
      const balance = payload['balance'] as number | undefined;
      const equity = payload['equity'] as number | undefined;
      if (accountId) {
        this.accounts.update((accounts) =>
          accounts.map((account) =>
            account.id === accountId
              ? {
                  ...account,
                  balance: balance ?? account.balance,
                  equity: equity ?? account.equity,
                  lastHeartbeatAt: new Date().toISOString(),
                }
              : account,
          ),
        );
      }
      this.settings.update((current) =>
        current && current.mt5
          ? {
              ...current,
              mt5: {
                ...current.mt5,
                connected: true,
                lastHeartbeatAt: new Date().toISOString(),
              },
            }
          : current,
      );
    });
    await this.api.onAccountsUpdated(() => {
      void this.refreshAccounts();
      void this.refreshSettings();
    });
    await this.api.onTradesUpdated(() => {
      this.dataVersion.update((value) => value + 1);
    });
    await this.api.onJournalUpdated(() => {
      this.dataVersion.update((value) => value + 1);
    });
    await this.api.onSeriesUpdated((event: SeriesUpdatedEvent) => {
      this.seriesVersion.update((value) => value + 1);
      this.lastMt5Event.set(`Velas ${event.timeframe} recibidas para ${event.symbol}`);
    });
    await this.api.onMt5Event((event) => {
      this.lastMt5Event.set(event.message);
    });
  }

  async refreshAccounts(): Promise<void> {
    try {
      this.accounts.set(await this.api.listAccounts());
    } catch (error) {
      this.error.set(this.message(error));
    }
  }

  async refreshSettings(): Promise<void> {
    try {
      this.settings.set(await this.api.getSettings());
    } catch (error) {
      this.error.set(this.message(error));
    }
  }

  async refreshStrategies(includeArchived = false): Promise<void> {
    try {
      this.strategies.set(await this.api.listStrategies(includeArchived));
    } catch (error) {
      this.error.set(this.message(error));
    }
  }

  async setActiveAccount(accountId: string): Promise<void> {
    await this.api.setActiveAccount(accountId);
    this.settings.update((current) =>
      current ? { ...current, activeAccountId: accountId } : current,
    );
    await this.refreshAccounts();
  }

  async reload(): Promise<void> {
    this.loading.set(true);
    this.error.set(null);
    try {
      this.applyBootstrap(await this.api.getBootstrap());
    } catch (error) {
      this.error.set(this.message(error));
    } finally {
      this.loading.set(false);
    }
  }

  message(error: unknown): string {
    if (typeof error === 'string') return error;
    if (error instanceof Error) return error.message;
    return 'Ha ocurrido un error inesperado';
  }
}
