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
  CopyLogEntry,
  CopyOverview,
  CopyToggleOptions,
  FollowerSettings,
  SizePreview,
  SizePreviewInput,
  SymbolMapRow,
  DayAsset,
  DayJournal,
  DayView,
  ExistingAssetAction,
  ImageInfo,
  ImageSaveInput,
  InstallResult,
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
  provider?: string;
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

  getNt8Status(): Promise<Mt5Status> {
    return this.call('get_nt8_status');
  }

  restartMt5Server(): Promise<Mt5Status> {
    return this.call('restart_mt5_server');
  }

  restartBridgeServer(): Promise<Mt5Status> {
    return this.call('restart_bridge_server');
  }

  writeNt8Config(): Promise<string> {
    return this.call('write_nt8_config');
  }

  openNt8Folder(): Promise<void> {
    return this.call('open_nt8_folder');
  }

  getAtasStatus(): Promise<Mt5Status> {
    return this.call('get_atas_status');
  }

  writeAtasConfig(): Promise<string> {
    return this.call('write_atas_config');
  }

  openAtasIndicatorsFolder(): Promise<void> {
    return this.call('open_atas_indicators_folder');
  }

  installNt8Bridge(): Promise<InstallResult> {
    return this.call('install_nt8_bridge');
  }

  installAtasBridge(): Promise<InstallResult> {
    return this.call('install_atas_bridge');
  }

  openDataDir(): Promise<void> {
    return this.call('open_data_dir');
  }

  createBackup(): Promise<string> {
    return this.call('create_backup');
  }

  // --- Copy trading -------------------------------------------------------

  getCopyOverview(): Promise<CopyOverview> {
    return this.call('get_copy_overview');
  }

  createCopyGroup(name: string, masterAccountId: string): Promise<string> {
    return this.call('create_copy_group', { name, masterAccountId });
  }

  updateCopyGroup(id: string, name: string, masterAccountId: string): Promise<void> {
    return this.call('update_copy_group', { id, name, masterAccountId });
  }

  deleteCopyGroup(id: string): Promise<void> {
    return this.call('delete_copy_group', { id });
  }

  setCopyEnabled(enabled: boolean, options: CopyToggleOptions | null = null): Promise<void> {
    return this.call('set_copy_enabled', { enabled, options });
  }

  setCopyGroupEnabled(
    id: string,
    enabled: boolean,
    options: CopyToggleOptions | null = null,
  ): Promise<void> {
    return this.call('set_copy_group_enabled', { id, enabled, options });
  }

  addCopyFollower(groupId: string, accountId: string): Promise<string> {
    return this.call('add_copy_follower', { groupId, accountId });
  }

  removeCopyFollower(id: string): Promise<void> {
    return this.call('remove_copy_follower', { id });
  }

  updateCopyFollower(
    id: string,
    settings: FollowerSettings,
    symbolMap: SymbolMapRow[],
  ): Promise<void> {
    return this.call('update_copy_follower', { id, settings, symbolMap });
  }

  setCopyFollowerEnabled(
    id: string,
    enabled: boolean,
    options: CopyToggleOptions | null = null,
  ): Promise<void> {
    return this.call('set_copy_follower_enabled', { id, enabled, options });
  }

  resumeCopyFollower(id: string): Promise<void> {
    return this.call('resume_copy_follower', { id });
  }

  copyPanic(groupId: string | null = null): Promise<number> {
    return this.call('copy_panic', { groupId });
  }

  listCopyLog(groupId: string | null = null, limit = 100): Promise<CopyLogEntry[]> {
    return this.call('list_copy_log', { groupId, limit });
  }

  previewCopySize(input: SizePreviewInput): Promise<SizePreview> {
    return this.call('preview_copy_size', { input });
  }

  onCopyUpdated(handler: () => void): Promise<UnlistenFn> {
    return this.listenEvent('copy://updated', () => handler());
  }

  onCopyActivity(handler: () => void): Promise<UnlistenFn> {
    return this.listenEvent('copy://activity', () => handler());
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

  onNt8Status(handler: (status: Mt5Status) => void): Promise<UnlistenFn> {
    return this.listenEvent('nt8://status', handler);
  }

  onNt8Heartbeat(handler: (payload: Record<string, unknown>) => void): Promise<UnlistenFn> {
    return this.listenEvent('nt8://heartbeat', handler);
  }

  onAtasStatus(handler: (status: Mt5Status) => void): Promise<UnlistenFn> {
    return this.listenEvent('atas://status', handler);
  }

  onAtasHeartbeat(handler: (payload: Record<string, unknown>) => void): Promise<UnlistenFn> {
    return this.listenEvent('atas://heartbeat', handler);
  }

  onAtasEvent(handler: (payload: Mt5Event) => void): Promise<UnlistenFn> {
    return this.listenEvent('atas://event', handler);
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

  onNt8Event(handler: (payload: Mt5Event) => void): Promise<UnlistenFn> {
    return this.listenEvent('nt8://event', handler);
  }
}
