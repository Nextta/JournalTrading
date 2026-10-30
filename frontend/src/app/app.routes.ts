import { Routes } from '@angular/router';

export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'dashboard' },
  {
    path: 'dashboard',
    loadComponent: () => import('./pages/dashboard.page').then((m) => m.DashboardPage),
    data: { title: 'Dashboard' },
  },
  {
    path: 'diario',
    loadComponent: () => import('./pages/journal.page').then((m) => m.JournalPage),
    data: { title: 'Diario de trading' },
  },
  {
    path: 'operaciones',
    loadComponent: () => import('./pages/trades.page').then((m) => m.TradesPage),
    data: { title: 'Operaciones' },
  },
  {
    path: 'estadisticas',
    loadComponent: () => import('./pages/stats.page').then((m) => m.StatsPage),
    data: { title: 'Estadísticas' },
  },
  {
    path: 'estrategias',
    loadComponent: () => import('./pages/strategies.page').then((m) => m.StrategiesPage),
    data: { title: 'Estrategias' },
  },
  {
    path: 'grupos',
    loadComponent: () => import('./pages/groups.page').then((m) => m.GroupsPage),
    data: { title: 'Grupos de activos' },
  },
  {
    path: 'cuentas',
    loadComponent: () => import('./pages/accounts.page').then((m) => m.AccountsPage),
    data: { title: 'Cuentas' },
  },
  {
    path: 'ajustes',
    loadComponent: () => import('./pages/settings.page').then((m) => m.SettingsPage),
    data: { title: 'Ajustes' },
  },
  { path: '**', redirectTo: 'dashboard' },
];
