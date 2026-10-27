export interface Account {
  id: string;
  login: number;
  server: string;
  displayName: string;
  company: string;
  currency: string;
  leverage: number | null;
  status: 'pending' | 'active' | 'archived';
  brokerUtcOffsetSeconds: number;
  balance: number;
  equity: number;
  lastHeartbeatAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface AccountInput {
  login: number;
  server: string;
  displayName: string;
  currency: string;
}

export interface AccountUpdate {
  displayName: string;
  currency: string;
}

export interface Mt5Status {
  running: boolean;
  port: number;
  error: string | null;
  startedAt: string | null;
  appVersion: string;
  connected: boolean;
  lastHeartbeatAt: string | null;
  login: number | null;
  server: string | null;
  eaVersion: string | null;
  terminalBuild: number | null;
  terminalDataPath: string | null;
}

export interface SettingsResponse {
  apiKey: string;
  apiPort: number;
  activeAccountId: string | null;
  timezoneMode: 'broker' | 'local';
  dataDir: string;
  version: string;
  mt5: Mt5Status;
}

export interface SettingsUpdate {
  apiPort: number;
  timezoneMode: 'broker' | 'local';
}

export interface Bootstrap {
  accounts: Account[];
  settings: SettingsResponse;
  strategies: Strategy[];
}

export interface StrategyItem {
  id: string;
  strategyId: string;
  section: string;
  text: string;
  sortOrder: number;
  required: boolean;
  createdAt: string;
}

export interface StrategyItemInput {
  id?: string | null;
  section: string;
  text: string;
  required: boolean;
}

export interface Strategy {
  id: string;
  name: string;
  description: string;
  color: string;
  archived: boolean;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
  items: StrategyItem[];
}

export interface StrategyInput {
  name: string;
  description: string;
  color: string;
  items: StrategyItemInput[];
}

export interface DayJournal {
  id: string;
  accountId: string;
  date: string;
  generalHtml: string;
  createdAt: string;
  updatedAt: string;
}

export interface DayAsset {
  id: string;
  journalId: string;
  symbol: string;
  strategyId: string | null;
  journalHtml: string;
  createdAt: string;
  updatedAt: string;
}

export interface PlanItem {
  id: string;
  dayAssetId: string;
  templateItemId: string | null;
  section: string;
  text: string;
  sortOrder: number;
  required: boolean;
  checked: boolean;
  checkedAt: string | null;
}

export interface Deal {
  id: string;
  tradeId: string;
  accountId: string;
  dealTicket: number;
  entry: number;
  type: number;
  volume: number;
  price: number;
  profit: number;
  commission: number;
  swap: number;
  timeServer: string;
  timeUtc: string;
  comment: string;
}

export interface ImageInfo {
  id: string;
  dayJournalId: string | null;
  dayAssetId: string | null;
  tradeId: string | null;
  fileName: string;
  relativePath: string;
  thumbPath: string | null;
  mime: string;
  width: number | null;
  height: number | null;
  bytes: number;
  caption: string;
  sortOrder: number;
  createdAt: string;
  path: string;
  thumbUrl: string | null;
}

export interface Trade {
  id: string;
  accountId: string;
  dayAssetId: string | null;
  positionTicket: number;
  symbol: string;
  side: 'buy' | 'sell';
  volumeInitial: number;
  volumeCurrent: number;
  openTimeServer: string;
  openTimeUtc: string;
  closeTimeServer: string | null;
  closeTimeUtc: string | null;
  openPrice: number;
  closePrice: number | null;
  sl: number | null;
  tp: number | null;
  profitGross: number;
  commission: number;
  swap: number;
  profitNet: number;
  magic: number | null;
  comment: string;
  strategyId: string | null;
  riskAmount: number | null;
  rMultiple: number | null;
  notes: string;
  state: 'open' | 'closed';
  createdAt: string;
  updatedAt: string;
  strategyName: string | null;
  dayAssetSymbol: string | null;
  deals: Deal[];
  images: ImageInfo[];
  seriesTimeframes: string[];
}

export interface TradeFilter {
  accountId: string;
  from?: string | null;
  to?: string | null;
  symbols?: string[];
  strategyIds?: string[];
  state?: 'open' | 'closed' | null;
  search?: string | null;
  limit?: number | null;
  offset?: number | null;
}

export interface TradePage {
  items: Trade[];
  total: number;
}

export interface CandleBar {
  time: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  spread: number;
}

export interface TradeSeries {
  tradeId: string;
  timeframe: string;
  fromTime: number;
  toTime: number;
  barCount: number;
  bars: CandleBar[];
  fetchedAt: string;
}

export interface ImageSaveInput {
  ownerType: 'day_journal' | 'day_asset' | 'trade';
  ownerId: string;
  fileName: string;
  caption: string;
  dataUrl: string;
}

export interface AssetDayStats {
  trades: number;
  netProfit: number;
  wins: number;
  losses: number;
  winRate: number;
}

export interface DayAssetView {
  asset: DayAsset;
  strategyName: string | null;
  planItems: PlanItem[];
  trades: Trade[];
  images: ImageInfo[];
  stats: AssetDayStats;
}

export interface DayView {
  account: Account;
  journal: DayJournal;
  assets: DayAssetView[];
  openTrades: Trade[];
  images: ImageInfo[];
}

export interface StatsFilter {
  accountId: string;
  from?: string | null;
  to?: string | null;
  symbols?: string[];
  strategyIds?: string[];
}

export interface StatsSummary {
  totalTrades: number;
  wins: number;
  losses: number;
  breakeven: number;
  netProfit: number;
  grossProfit: number;
  grossLoss: number;
  winRate: number;
  profitFactor: number | null;
  expectancy: number;
  avgWin: number;
  avgLoss: number;
  avgR: number | null;
  totalR: number;
  maxDrawdown: number;
  maxDrawdownPct: number;
  bestTrade: number;
  worstTrade: number;
  avgDurationMinutes: number;
  currentStreak: number;
  maxWinStreak: number;
  maxLossStreak: number;
}

export interface EquityPoint {
  index: number;
  time: string;
  date: string;
  cumulative: number;
  tradeNet: number;
  drawdown: number;
  symbol: string;
}

export interface DailyPnl {
  date: string;
  net: number;
  trades: number;
  wins: number;
  losses: number;
}

export interface BreakdownRow {
  key: string;
  label: string;
  trades: number;
  net: number;
  wins: number;
  losses: number;
  winRate: number;
  profitFactor: number | null;
  avgR: number | null;
}

export interface DistributionBucket {
  label: string;
  count: number;
}

export interface MonthlyRow {
  month: string;
  net: number;
  trades: number;
  winRate: number;
}

export interface StatsResponse {
  summary: StatsSummary;
  equity: EquityPoint[];
  dailyPnl: DailyPnl[];
  bySymbol: BreakdownRow[];
  byStrategy: BreakdownRow[];
  byWeekday: BreakdownRow[];
  byHour: BreakdownRow[];
  rDistribution: DistributionBucket[];
  monthly: MonthlyRow[];
}

export const TIMEFRAMES = ['M1', 'M5', 'M15', 'M30', 'H1', 'H4', 'D1', 'W1'] as const;
export type Timeframe = (typeof TIMEFRAMES)[number];
