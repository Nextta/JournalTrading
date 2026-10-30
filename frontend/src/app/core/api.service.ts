import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import {
  Account,
  AccountInput,
  AccountUpdate,
  AddGroupResult,
  AssetGroup,
  AssetGroupInput,
  Bootstrap,
  DayAsset,
  DayJournal,
  DayView,
  ExistingAssetAction,
  ImageInfo,
  ImageSaveInput,
  Mt5Status,
  PerformanceCalendar,
  PlanItem,
  SettingsResponse,
  SettingsUpdate,
  StatsFilter,
  StatsResponse,
  Strategy,
  StrategyInput,
  Trade,
  TradeFilter,
  TradePage,
  TradeSeries,
} from './models';

export interface SeriesUpdatedEvent {
  tradeId: string;
  timeframe: string;
  symbol: string;
  bars: number;
}

export interface Mt5Event {
  level: string;
  kind: string;
  message: string;
  accountId: string | null;
  receivedAt: string;
}

function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class ApiService {
  private async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    if (!isTauri()) {
      throw new Error(
        'Esta aplicación debe ejecutarse dentro de la app de escritorio (npm run dev).',
      );
    }
    return invoke<T>(command, args);
  }

  getBootstrap(): Promise<Bootstrap> {
    return this.call('get_bootstrap');
  }

  listAccounts(): Promise<Account[]> {
    return this.call('list_accounts');
  }

  createAccount(input: AccountInput): Promise<Account> {
    return this.call('create_account', { input });
  }

  updateAccount(id: string, input: AccountUpdate): Promise<Account> {
    return this.call('update_account', { id, input });
  }

  confirmAccount(id: string): Promise<Account> {
    return this.call('confirm_account', { id });
  }

  archiveAccount(id: string): Promise<void> {
    return this.call('archive_account', { id });
  }

  setActiveAccount(id: string): Promise<void> {
    return this.call('set_active_account', { id });
  }

  listStrategies(includeArchived = false): Promise<Strategy[]> {
    return this.call('list_strategies', { includeArchived });
  }

  getStrategy(id: string): Promise<Strategy> {
    return this.call('get_strategy', { id });
  }

  createStrategy(input: StrategyInput): Promise<Strategy> {
    return this.call('create_strategy', { input });
  }

  updateStrategy(id: string, input: StrategyInput): Promise<Strategy> {
    return this.call('update_strategy', { id, input });
  }

  archiveStrategy(id: string): Promise<void> {
    return this.call('archive_strategy', { id });
  }

  duplicateStrategy(id: string): Promise<Strategy> {
    return this.call('duplicate_strategy', { id });
  }

  listAssetGroups(): Promise<AssetGroup[]> {
    return this.call('list_asset_groups');
  }

  getAssetGroup(id: string): Promise<AssetGroup> {
    return this.call('get_asset_group', { id });
  }

  createAssetGroup(input: AssetGroupInput): Promise<AssetGroup> {
    return this.call('create_asset_group', { input });
  }

  updateAssetGroup(id: string, input: AssetGroupInput): Promise<AssetGroup> {
    return this.call('update_asset_group', { id, input });
  }

  deleteAssetGroup(id: string): Promise<void> {
    return this.call('delete_asset_group', { id });
  }

  addGroupToDay(
    accountId: string,
    date: string,
    groupId: string,
    existingAction: ExistingAssetAction,
    applyToTrades: boolean,
  ): Promise<AddGroupResult> {
    return this.call('add_group_to_day', {
      accountId,
      date,
      groupId,
      existingAction,
      applyToTrades,
    });
  }

  getDay(accountId: string, date: string): Promise<DayView> {
    return this.call('get_day', { accountId, date });
  }

  createDayAsset(
    accountId: string,
    date: string,
    symbol: string,
    strategyId: string | null,
  ): Promise<DayAsset> {
    return this.call('create_day_asset', { accountId, date, symbol, strategyId });
  }

  deleteDayAsset(assetId: string): Promise<void> {
    return this.call('delete_day_asset', { assetId });
  }

  selectAssetStrategy(
    assetId: string,
    strategyId: string | null,
    applyToTrades: boolean,
  ): Promise<DayAsset> {
    return this.call('select_asset_strategy', { assetId, strategyId, applyToTrades });
  }

  togglePlanItem(itemId: string, checked: boolean): Promise<PlanItem> {
    return this.call('toggle_plan_item', { itemId, checked });
  }

  addPlanItem(
    assetId: string,
    section: string,
    text: string,
    required: boolean,
  ): Promise<PlanItem> {
    return this.call('add_plan_item', { assetId, section, text, required });
  }

  deletePlanItem(itemId: string): Promise<void> {
    return this.call('delete_plan_item', { itemId });
  }

  saveDayJournal(journalId: string, html: string): Promise<DayJournal> {
    return this.call('save_day_journal', { journalId, html });
  }

  saveAssetJournal(assetId: string, html: string): Promise<DayAsset> {
    return this.call('save_asset_journal', { assetId, html });
  }

  saveImage(input: ImageSaveInput): Promise<ImageInfo> {
    return this.call('save_image', { input });
  }

  listImages(ownerType: string, ownerId: string): Promise<ImageInfo[]> {
    return this.call('list_images', { ownerType, ownerId });
  }

  deleteImage(id: string): Promise<void> {
    return this.call('delete_image', { id });
  }

  listTrades(filter: TradeFilter): Promise<TradePage> {
    return this.call('list_trades', { filter });
  }

  getTrade(id: string): Promise<Trade> {
    return this.call('get_trade', { id });
  }

  updateTradeNotes(id: string, notes: string): Promise<Trade> {
    return this.call('update_trade_notes', { id, notes });
  }

  assignTradeStrategy(id: string, strategyId: string | null): Promise<Trade> {
    return this.call('assign_trade_strategy', { id, strategyId });
  }

  assignTradeToAsset(id: string, dayAssetId: string | null): Promise<Trade> {
    return this.call('assign_trade_to_asset', { id, dayAssetId });
  }

  getTradeSeries(tradeId: string, timeframe: string): Promise<TradeSeries | null> {
    return this.call('get_trade_series', { tradeId, timeframe });
  }

  requestTradeSeries(tradeId: string, timeframe: string): Promise<string> {
    return this.call('request_trade_series', { tradeId, timeframe });
  }

  requestSync(accountId: string): Promise<string> {
    return this.call('request_sync', { accountId });
  }

  getStats(filter: StatsFilter): Promise<StatsResponse> {
    return this.call('get_stats', { filter });
  }

  getPerformanceCalendar(accountId: string): Promise<PerformanceCalendar> {
    return this.call('get_performance_calendar', { accountId });
  }

  getSettings(): Promise<SettingsResponse> {
    return this.call('get_settings');
  }

  updateSettings(input: SettingsUpdate): Promise<SettingsResponse> {
    return this.call('update_settings', { input });
  }

  regenerateApiKey(): Promise<string> {
    return this.call('regenerate_api_key');
  }

  getMt5Status(): Promise<Mt5Status> {
    return this.call('get_mt5_status');
  }

  restartMt5Server(): Promise<Mt5Status> {
    return this.call('restart_mt5_server');
  }

  openDataDir(): Promise<void> {
    return this.call('open_data_dir');
  }

  createBackup(): Promise<string> {
    return this.call('create_backup');
  }

  private listenEvent<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
    if (!isTauri()) {
      return Promise.resolve(() => undefined);
    }
    return listen<T>(event, (eventPayload) => handler(eventPayload.payload));
  }

  onMt5Status(handler: (status: Mt5Status) => void): Promise<UnlistenFn> {
    return this.listenEvent('mt5://status', handler);
  }

  onMt5Heartbeat(handler: (payload: Record<string, unknown>) => void): Promise<UnlistenFn> {
    return this.listenEvent('mt5://heartbeat', handler);
  }

  onAccountsUpdated(handler: (payload: { accountId: string }) => void): Promise<UnlistenFn> {
    return this.listenEvent('accounts://updated', handler);
  }

  onTradesUpdated(handler: (payload: Record<string, unknown>) => void): Promise<UnlistenFn> {
    return this.listenEvent('trades://updated', handler);
  }

  onJournalUpdated(handler: (payload: Record<string, unknown>) => void): Promise<UnlistenFn> {
    return this.listenEvent('journal://updated', handler);
  }

  onSeriesUpdated(handler: (payload: SeriesUpdatedEvent) => void): Promise<UnlistenFn> {
    return this.listenEvent('series://updated', handler);
  }

  onMt5Event(handler: (payload: Mt5Event) => void): Promise<UnlistenFn> {
    return this.listenEvent('mt5://event', handler);
  }
}
