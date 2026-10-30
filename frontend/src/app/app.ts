import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatMenuModule } from '@angular/material/menu';
import { MatTooltipModule } from '@angular/material/tooltip';
import { NavigationEnd, Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { filter } from 'rxjs';

import { ApiService } from './core/api.service';
import { formatMoney, initials, todayIso, formatDateLong } from './core/format';
import { NotificationsService } from './core/notifications.service';
import { Store } from './core/store';

interface NavItem {
  label: string;
  icon: string;
  route: string;
}

@Component({
  selector: 'app-root',
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    MatButtonModule,
    MatIconModule,
    MatMenuModule,
    MatTooltipModule,
  ],
  templateUrl: './app.html',
  styleUrl: './app.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class App {
  protected readonly store = inject(Store);
  private readonly api = inject(ApiService);
  private readonly notifications = inject(NotificationsService);
  private readonly router = inject(Router);

  protected readonly navItems: NavItem[] = [
    { label: 'Dashboard', icon: 'space_dashboard', route: '/dashboard' },
    { label: 'Diario', icon: 'menu_book', route: '/diario' },
    { label: 'Operaciones', icon: 'receipt_long', route: '/operaciones' },
    { label: 'Estadísticas', icon: 'insights', route: '/estadisticas' },
    { label: 'Estrategias', icon: 'rule', route: '/estrategias' },
    { label: 'Grupos', icon: 'workspaces', route: '/grupos' },
    { label: 'Cuentas', icon: 'account_balance_wallet', route: '/cuentas' },
    { label: 'Ajustes', icon: 'tune', route: '/ajustes' },
  ];

  protected readonly collapsed = signal(false);
  protected readonly title = signal('Dashboard');
  protected readonly today = formatDateLong(todayIso());
  protected readonly syncing = signal(false);
  protected readonly accountInitials = computed(() => {
    const account = this.store.activeAccount();
    return account ? initials(account.displayName) : 'PT';
  });
  protected readonly hasPending = computed(() => this.store.pendingAccounts().length > 0);

  constructor() {
    void this.store.init();
    this.router.events
      .pipe(
        filter((event): event is NavigationEnd => event instanceof NavigationEnd),
        takeUntilDestroyed(inject(DestroyRef)),
      )
      .subscribe((event) => {
        let route = this.router.routerState.snapshot.root;
        while (route.firstChild) route = route.firstChild;
        this.title.set((route.data['title'] as string | undefined) ?? 'JournalTrading');
      });
  }

  protected toggleSidebar(): void {
    this.collapsed.update((value) => !value);
  }

  protected async activateAccount(accountId: string): Promise<void> {
    try {
      await this.store.setActiveAccount(accountId);
      this.store.dataVersion.update((value) => value + 1);
    } catch (error) {
      this.notifications.error(error);
    }
  }

  protected async sync(): Promise<void> {
    const account = this.store.activeAccount();
    if (!account || this.syncing()) return;
    this.syncing.set(true);
    try {
      await this.api.requestSync(account.id);
      this.notifications.info('Sincronización solicitada al EA');
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.syncing.set(false);
    }
  }

  protected money(value: number): string {
    return formatMoney(value, this.store.currency());
  }

  protected reload(): void {
    void this.store.reload();
  }
}
