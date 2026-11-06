//+------------------------------------------------------------------+
//|                                     JournalTradingBridge.cs      |
//|              Puente entre NinjaTrader 8 y JournalTrading         |
//+------------------------------------------------------------------+
#region Using declarations
using System;
using System.Collections;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using NinjaTrader.Cbi;
using NinjaTrader.Data;
using NinjaTrader.NinjaScript;
#endregion

namespace NinjaTrader.NinjaScript.AddOns
{
    /// <summary>
    /// AddOn de solo lectura que sincroniza cuentas, ejecuciones y velas con
    /// JournalTrading a través de la API HTTP local (127.0.0.1).
    ///
    /// Configuración: Documents\NinjaTrader 8\bin\Custom\JournalTradingBridge.json
    /// La app puede generar ese archivo desde Ajustes > Puente NinjaTrader 8.
    /// </summary>
    public class JournalTradingBridge : AddOnBase
    {
        private const string AddOnVersion = "1.0.0";
        private const string ConfigFileName = "JournalTradingBridge.json";
        private const string DefaultApiUrl = "http://127.0.0.1:4477";

        private readonly HttpClient http = new HttpClient { Timeout = TimeSpan.FromSeconds(120) };
        private readonly Dictionary<long, SyncRequest> syncRequests = new Dictionary<long, SyncRequest>();
        private readonly HashSet<long> initializedAccounts = new HashSet<long>();
        private readonly List<BarsRequest> activeRequests = new List<BarsRequest>();

        private Timer timer;
        private BridgeConfig config = new BridgeConfig();
        private DateTime lastConfigRead = DateTime.MinValue;
        private DateTime lastHeartbeat = DateTime.MinValue;
        private DateTime lastSync = DateTime.MinValue;
        private DateTime lastSuccessfulSync = DateTime.MinValue;
        private DateTime lastLogAt = DateTime.MinValue;
        private string lastLogMessage = "";
        private bool configMissingLogged;
        private int busy;

        private static string ConfigPath
        {
            get
            {
                string dir = NinjaTrader.Core.Globals.UserDataDir;
                return Path.Combine(dir, "bin", "Custom", ConfigFileName);
            }
        }

        protected override void OnStateChange()
        {
            if (State == State.SetDefaults)
            {
                Name = "JournalTrading Bridge";
                Description = "Envía cuentas, ejecuciones y velas a JournalTrading.";
                IsVisible = false;
            }
            else if (State == State.Active)
            {
                if (timer != null)
                {
                    timer.Dispose();
                    timer = null;
                }

                ReloadConfig(true);
                Print("JournalTrading Bridge " + AddOnVersion + " iniciado.");
                timer = new Timer(OnTimer, null, 1000, 1000);
            }
            else if (State == State.Terminated || State == State.Finalized)
            {
                if (timer != null)
                {
                    timer.Dispose();
                    timer = null;
                }

                lock (activeRequests)
                {
                    foreach (BarsRequest request in activeRequests)
                    {
                        try { request.Dispose(); } catch { }
                    }
                    activeRequests.Clear();
                }

                try { http.Dispose(); } catch { }
            }
        }

        // -------------------------------------------------------------------
        // Temporizador
        // -------------------------------------------------------------------

        private void OnTimer(object state)
        {
            try
            {
                ReloadConfig(false);

                DateTime now = DateTime.UtcNow;
                bool heartbeatDue = (now - lastHeartbeat).TotalSeconds >= config.HeartbeatSeconds;
                bool syncDue = (now - lastSync).TotalSeconds >= config.SyncSeconds;
                if (!heartbeatDue && !syncDue)
                    return;

                if (Interlocked.CompareExchange(ref busy, 1, 0) != 0)
                    return;

                Task.Run(async () =>
                {
                    try
                    {
                        if (heartbeatDue)
                        {
                            lastHeartbeat = now;
                            await HeartbeatAllAsync();
                        }

                        if (syncDue)
                        {
                            lastSync = now;
                            await SyncAllAsync();
                        }
                    }
                    catch (Exception ex)
                    {
                        Log("Error en el ciclo: " + ex.Message);
                    }
                    finally
                    {
                        Interlocked.Exchange(ref busy, 0);
                    }
                });
            }
            catch (Exception ex)
            {
                Log("Error en el temporizador: " + ex.Message);
            }
        }

        // -------------------------------------------------------------------
        // Configuración
        // -------------------------------------------------------------------

        private void ReloadConfig(bool force)
        {
            try
            {
                string path = ConfigPath;
                if (!File.Exists(path))
                {
                    if (!configMissingLogged)
                    {
                        configMissingLogged = true;
                        Log("No existe " + path + ". Genera la configuración desde Ajustes en JournalTrading.");
                    }
                    return;
                }

                DateTime writeTime = File.GetLastWriteTimeUtc(path);
                if (!force && writeTime <= lastConfigRead)
                    return;

                lastConfigRead = writeTime;
                string text = File.ReadAllText(path);
                Dictionary<string, object> data = NewSerializer().Deserialize<Dictionary<string, object>>(text);
                config = BridgeConfig.From(data);
                configMissingLogged = false;
                Log("Configuración cargada: " + config.ApiUrl);
            }
            catch (Exception ex)
            {
                Log("No se pudo leer la configuración: " + ex.Message);
            }
        }

        // -------------------------------------------------------------------
        // Heartbeat
        // -------------------------------------------------------------------

        private async Task HeartbeatAllAsync()
        {
            if (string.IsNullOrWhiteSpace(config.ApiKey))
            {
                LogThrottled("Falta la API key. Configúrala en Ajustes > Puente NinjaTrader 8.");
                return;
            }

            List<Account> accounts = SnapshotAccounts();
            foreach (Account account in accounts)
            {
                try
                {
                    Dictionary<string, object> payload = BuildHeartbeat(account);
                    Dictionary<string, object> response = await PostAsync("/api/v1/nt8/heartbeat", payload);
                    if (response != null)
                        ProcessCommands(account, response);
                }
                catch (Exception ex)
                {
                    Log("Heartbeat " + account.Name + ": " + ex.Message);
                }
            }
        }

        private Dictionary<string, object> BuildHeartbeat(Account account)
        {
            Currency currency = account.Denomination;
            DateTime now = DateTime.Now;

            return new Dictionary<string, object>
            {
                { "login", account.Id },
                { "server", ConnectionName(account) },
                { "display_name", string.IsNullOrWhiteSpace(account.Name) ? account.DisplayName : account.Name },
                { "company", "NinjaTrader 8 · " + ConnectionName(account) },
                { "currency", CurrencyCode(currency) },
                { "leverage", null },
                { "broker_utc_offset_seconds", (int)TimeZoneInfo.Local.GetUtcOffset(now).TotalSeconds },
                { "balance", SafeAccountValue(account, AccountItem.CashValue, currency) },
                { "equity", SafeAccountValue(account, AccountItem.NetLiquidation, currency) },
                { "margin", SafeAccountValue(account, AccountItem.InitialMargin, currency) },
                { "free_margin", SafeAccountValue(account, AccountItem.ExcessInitialMargin, currency) },
                { "ea_version", AddOnVersion },
                { "terminal_build", 8 },
                { "terminal_data_path", NinjaTrader.Core.Globals.UserDataDir },
                { "server_time", IsoLocal(now) }
            };
        }

        private void ProcessCommands(Account account, Dictionary<string, object> response)
        {
            object commandsValue;
            if (!response.TryGetValue("commands", out commandsValue) || commandsValue == null)
                return;

            object[] commands = commandsValue as object[];
            if (commands == null)
            {
                ArrayList list = commandsValue as ArrayList;
                if (list != null)
                    commands = list.ToArray();
            }
            if (commands == null)
                return;

            foreach (object item in commands)
            {
                Dictionary<string, object> command = item as Dictionary<string, object>;
                if (command == null)
                    continue;

                string id = GetString(command, "id");
                string kind = GetString(command, "kind");
                Dictionary<string, object> payload = command.ContainsKey("payload")
                    ? command["payload"] as Dictionary<string, object>
                    : null;

                if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(kind))
                    continue;

                if (kind == "sync")
                {
                    bool allHistory = GetBool(payload, "all_history", false);
                    int fromDays = GetInt(payload, "from_days", 0);
                    lock (syncRequests)
                    {
                        syncRequests[account.Id] = new SyncRequest
                        {
                            ForceFull = true,
                            AllHistory = allHistory,
                            FromDays = fromDays
                        };
                    }
                    lastSync = DateTime.MinValue;
                    SendCommandResult(id, true, null);
                }
                else if (kind == "series")
                {
                    _ = Task.Run(() => HandleSeriesCommandAsync(account, id, payload));
                }
                else
                {
                    SendCommandResult(id, false, "Comando no soportado: " + kind);
                }
            }
        }

        // -------------------------------------------------------------------
        // Sincronización
        // -------------------------------------------------------------------

        private async Task SyncAllAsync()
        {
            if (string.IsNullOrWhiteSpace(config.ApiKey))
                return;

            List<Account> accounts = SnapshotAccounts();
            foreach (Account account in accounts)
            {
                try
                {
                    await SyncAccountAsync(account);
                }
                catch (Exception ex)
                {
                    Log("Sync " + account.Name + ": " + ex.Message);
                }
            }
        }

        private async Task SyncAccountAsync(Account account)
        {
            SyncRequest request;
            lock (syncRequests)
            {
                if (!syncRequests.TryGetValue(account.Id, out request))
                {
                    request = new SyncRequest
                    {
                        ForceFull = false,
                        AllHistory = false,
                        FromDays = config.HistoryDays
                    };
                }
            }

            bool firstSync;
            lock (syncRequests)
            {
                firstSync = !initializedAccounts.Contains(account.Id);
            }

            DateTime now = DateTime.Now;
            DateTime from;
            if (request.ForceFull || firstSync)
            {
                int days = request.FromDays > 0 ? request.FromDays : config.HistoryDays;
                from = request.AllHistory || days <= 0
                    ? new DateTime(2000, 1, 1)
                    : now.AddDays(-days);
            }
            else
            {
                from = lastSuccessfulSync == DateTime.MinValue
                    ? new DateTime(2000, 1, 1)
                    : lastSuccessfulSync.AddDays(-7);
            }

            DateTime to = now.AddDays(1);
            Dictionary<long, Execution> executions = new Dictionary<long, Execution>();

            try
            {
                Collection<Execution> historical = Execution.DbGet(account, from, to);
                if (historical != null)
                {
                    foreach (Execution execution in historical)
                        AddExecution(executions, execution);
                }
            }
            catch (Exception ex)
            {
                Log("Historial " + account.Name + ": " + ex.Message);
            }

            try
            {
                foreach (Execution execution in SnapshotExecutions(account))
                    AddExecution(executions, execution);
            }
            catch (Exception ex)
            {
                Log("Ejecuciones " + account.Name + ": " + ex.Message);
            }

            List<PositionGroup> groups = BuildPositionGroups(executions.Values);
            Dictionary<string, object> payload = BuildSyncPayload(account, groups);
            Dictionary<string, object> response = await PostAsync("/api/v1/nt8/sync", payload);
            if (response == null)
                return;

            lock (syncRequests)
            {
                syncRequests.Remove(account.Id);
                initializedAccounts.Add(account.Id);
            }
            lastSuccessfulSync = now;

            int openPositions = groups.Count(group => group.IsOpen);
            Log(string.Format(
                "Sync {0}: {1} ejecuciones, {2} posiciones abiertas.",
                account.Name,
                executions.Count,
                openPositions));
        }

        private List<PositionGroup> BuildPositionGroups(IEnumerable<Execution> executions)
        {
            Dictionary<long, PositionGroup> groups = new Dictionary<long, PositionGroup>();
            foreach (Execution execution in executions.OrderBy(item => item.Time))
            {
                long key = PositionKey(execution);
                PositionGroup group;
                if (!groups.TryGetValue(key, out group))
                {
                    group = new PositionGroup
                    {
                        Key = key,
                        Instrument = execution.Instrument
                    };
                    groups[key] = group;
                }
                group.Add(execution);
            }

            return groups.Values.Where(group => group.EntryQuantity > 0).ToList();
        }

        private Dictionary<string, object> BuildSyncPayload(Account account, List<PositionGroup> groups)
        {
            Dictionary<string, Dictionary<string, object>> symbols =
                new Dictionary<string, Dictionary<string, object>>();
            List<object> deals = new List<object>();
            List<object> positions = new List<object>();

            foreach (PositionGroup group in groups)
            {
                Instrument instrument = group.Instrument;
                if (instrument == null)
                    continue;

                symbols[instrument.FullName] = BuildSymbol(account, instrument);

                double tickSize = TickSize(instrument);
                double priceValue = PriceValuePerUnit(account, instrument);
                foreach (Execution execution in group.Executions)
                    deals.Add(BuildDeal(account, execution, group));

                if (group.IsOpen)
                    positions.Add(BuildPosition(account, group, tickSize, priceValue));
            }

            return new Dictionary<string, object>
            {
                { "login", account.Id },
                { "server", ConnectionName(account) },
                { "broker_utc_offset_seconds", (int)TimeZoneInfo.Local.GetUtcOffset(DateTime.Now).TotalSeconds },
                { "symbols", symbols.Values.ToList() },
                { "positions", positions },
                { "deals", deals },
                { "server_time", IsoLocal(DateTime.Now) }
            };
        }

        private Dictionary<string, object> BuildSymbol(Account account, Instrument instrument)
        {
            MasterInstrument master = instrument.MasterInstrument;
            double tickSize = TickSize(instrument);
            double tickValue = PriceValuePerUnit(account, instrument) * tickSize;

            return new Dictionary<string, object>
            {
                { "symbol", instrument.FullName },
                { "description", string.IsNullOrWhiteSpace(master.Description) ? instrument.FullName : master.Description },
                { "digits", DigitsFromTickSize(tickSize) },
                { "tick_size", tickSize },
                { "tick_value", tickValue },
                { "contract_size", 1.0 },
                { "currency_profit", CurrencyCode(master.Currency) }
            };
        }

        private Dictionary<string, object> BuildDeal(
            Account account,
            Execution execution,
            PositionGroup group)
        {
            int type = ExecutionType(execution, group);
            int entry = execution.IsEntry ? 0 : 1;
            double profit = 0.0;

            if (execution.IsExit && group.EntryQuantity > 0)
            {
                double direction = group.Side == MarketPosition.Long ? 1.0 : -1.0;
                double priceValue = PriceValuePerUnit(account, execution.Instrument);
                profit = (execution.Price - group.AverageEntryPrice) * priceValue * execution.Quantity * direction;
            }

            double commission = -Math.Abs(SafeDouble(execution.Commission) + SafeDouble(execution.Fee));

            return new Dictionary<string, object>
            {
                { "deal_ticket", ExecutionId(execution) },
                { "position_ticket", group.Key },
                { "symbol", execution.Instrument.FullName },
                { "type", type },
                { "entry", entry },
                { "volume", (double)execution.Quantity },
                { "price", SafeDouble(execution.Price) },
                { "profit", profit },
                { "commission", commission },
                { "swap", 0.0 },
                { "magic", 0 },
                { "comment", CommentFor(execution) },
                { "reason", ReasonFor(execution) },
                { "time_server", IsoLocal(execution.Time) },
                { "time", ToEpoch(execution.Time) },
                { "time_utc", IsoUtc(execution.Time) }
            };
        }

        private Dictionary<string, object> BuildPosition(
            Account account,
            PositionGroup group,
            double tickSize,
            double priceValue)
        {
            Instrument instrument = group.Instrument;
            int side = group.Side == MarketPosition.Long ? 0 : 1;
            double stopPrice;
            double targetPrice;
            FindStopTarget(account, instrument, group.Side, group.AverageEntryPrice, out stopPrice, out targetPrice);

            double profit = 0.0;
            try
            {
                Position position = FindPosition(account, instrument, group.Side);
                if (position != null)
                {
                    double marketPrice = position.GetMarketPrice();
                    if (marketPrice > 0)
                        profit = position.GetUnrealizedProfitLoss(PerformanceUnit.Currency, marketPrice);
                }
            }
            catch { }

            return new Dictionary<string, object>
            {
                { "ticket", group.Key },
                { "symbol", instrument.FullName },
                { "type", side },
                { "volume", group.OpenQuantity },
                { "price_open", group.AverageEntryPrice },
                { "sl", stopPrice > 0 ? (object)stopPrice : null },
                { "tp", targetPrice > 0 ? (object)targetPrice : null },
                { "profit", profit },
                { "swap", 0.0 },
                { "commission", 0.0 },
                { "magic", 0 },
                { "comment", "" },
                { "time_server", IsoLocal(group.FirstEntryTime) },
                { "time", ToEpoch(group.FirstEntryTime) },
                { "time_utc", IsoUtc(group.FirstEntryTime) },
                { "tick_size", tickSize },
                { "tick_value", priceValue * tickSize }
            };
        }

        private static int ExecutionType(Execution execution, PositionGroup group)
        {
            try
            {
                if (execution.Order != null)
                {
                    OrderAction action = execution.Order.OrderAction;
                    return action == OrderAction.Buy || action == OrderAction.BuyToCover ? 0 : 1;
                }
            }
            catch { }

            if (execution.IsExit)
                return group.Side == MarketPosition.Long ? 1 : 0;

            return execution.MarketPosition == MarketPosition.Long ? 0 : 1;
        }

        private static long ExecutionId(Execution execution)
        {
            if (execution.Id > 0)
                return execution.Id;

            return StableHash(execution.ExecutionId ?? execution.ToString());
        }

        private static long PositionKey(Execution execution)
        {
            long baseKey;
            if (execution.Position > 0)
                baseKey = execution.Position;
            else if (execution.PositionStrategy > 0)
                baseKey = 1000000000L + execution.PositionStrategy;
            else
                baseKey = StableHash(execution.ExecutionId ?? execution.ToString());

            string instrument = execution.Instrument != null ? execution.Instrument.FullName : "?";
            return StableHash(instrument + "|" + baseKey.ToString(CultureInfo.InvariantCulture));
        }

        private static void AddExecution(Dictionary<long, Execution> map, Execution execution)
        {
            if (execution == null || execution.Instrument == null)
                return;
            if (execution.Quantity <= 0)
                return;
            if (!execution.IsEntry && !execution.IsExit)
                return;
            if (execution.IsSod)
                return;

            long id = ExecutionId(execution);
            if (id > 0)
                map[id] = execution;
        }

        private void FindStopTarget(
            Account account,
            Instrument instrument,
            MarketPosition side,
            double entry,
            out double stopPrice,
            out double targetPrice)
        {
            stopPrice = 0;
            targetPrice = 0;

            try
            {
                foreach (Order order in SnapshotOrders(account))
                {
                    if (order == null || order.Instrument != instrument || !IsWorking(order))
                        continue;

                    bool isStop = order.OrderType == OrderType.StopMarket
                        || order.OrderType == OrderType.StopLimit;
                    bool isLimit = order.OrderType == OrderType.Limit;

                    if (side == MarketPosition.Long)
                    {
                        if (isStop && order.StopPrice > 0 && order.StopPrice < entry
                            && (stopPrice == 0 || order.StopPrice > stopPrice))
                            stopPrice = order.StopPrice;

                        if (isLimit && order.LimitPrice > 0 && order.LimitPrice > entry
                            && (targetPrice == 0 || order.LimitPrice < targetPrice))
                            targetPrice = order.LimitPrice;
                    }
                    else
                    {
                        if (isStop && order.StopPrice > 0 && order.StopPrice > entry
                            && (stopPrice == 0 || order.StopPrice < stopPrice))
                            stopPrice = order.StopPrice;

                        if (isLimit && order.LimitPrice > 0 && order.LimitPrice < entry
                            && (targetPrice == 0 || order.LimitPrice > targetPrice))
                            targetPrice = order.LimitPrice;
                    }
                }
            }
            catch { }
        }

        private static bool IsWorking(Order order)
        {
            switch (order.OrderState)
            {
                case OrderState.Working:
                case OrderState.Accepted:
                case OrderState.Submitted:
                case OrderState.PartFilled:
                case OrderState.TriggerPending:
                case OrderState.ChangeSubmitted:
                case OrderState.ChangePending:
                    return true;
                default:
                    return false;
            }
        }

        private static string CommentFor(Execution execution)
        {
            try
            {
                if (execution.Order != null)
                {
                    if (!string.IsNullOrWhiteSpace(execution.Order.Name))
                        return execution.Order.Name;
                    if (!string.IsNullOrWhiteSpace(execution.Order.Text))
                        return execution.Order.Text;
                }
                return execution.Name ?? "";
            }
            catch
            {
                return "";
            }
        }

        private static int ReasonFor(Execution execution)
        {
            if (!execution.IsExit)
                return -1;

            try
            {
                if (execution.Order != null)
                {
                    if (execution.Order.OrderType == OrderType.StopMarket
                        || execution.Order.OrderType == OrderType.StopLimit)
                        return 4;
                    if (execution.Order.OrderType == OrderType.Limit)
                        return 5;
                }
            }
            catch { }

            string text = CommentFor(execution).ToLowerInvariant();
            if (text.Contains("stop") || text.Contains("sl"))
                return 4;
            if (text.Contains("target") || text.Contains("profit") || text.Contains("tp"))
                return 5;

            return -1;
        }

        // -------------------------------------------------------------------
        // Velas
        // -------------------------------------------------------------------

        private async Task HandleSeriesCommandAsync(
            Account account,
            string commandId,
            Dictionary<string, object> payload)
        {
            BarsRequest request = null;
            try
            {
                string symbol = GetString(payload, "symbol");
                string timeframe = GetString(payload, "timeframe");
                long tradeTicket = GetLong(payload, "trade_ticket", 0);
                long fromTime = GetLong(payload, "from_time", 0);
                long toTime = GetLong(payload, "to_time", 0);

                if (string.IsNullOrWhiteSpace(symbol) || string.IsNullOrWhiteSpace(timeframe))
                {
                    SendCommandResult(commandId, false, "Símbolo o timeframe vacío.");
                    return;
                }

                Instrument instrument = ResolveInstrument(symbol);
                if (instrument == null)
                {
                    SendCommandResult(commandId, false, "Instrumento no encontrado: " + symbol);
                    return;
                }

                BarsPeriod period = BuildBarsPeriod(timeframe);
                if (period == null)
                {
                    SendCommandResult(commandId, false, "Timeframe no válido: " + timeframe);
                    return;
                }

                DateTime from = fromTime > 0 ? FromEpoch(fromTime) : DateTime.Now.AddDays(-5);
                DateTime to = toTime > 0 ? FromEpoch(toTime) : DateTime.Now;
                if (from >= to)
                    from = to.AddDays(-5);

                request = await RequestBarsAsync(instrument, period, from, to);
                if (request == null)
                {
                    SendCommandResult(commandId, false, "No se pudieron cargar las velas.");
                    return;
                }

                List<Dictionary<string, object>> bars = CopyBars(request, period);
                Dictionary<string, object> seriesPayload = new Dictionary<string, object>
                {
                    { "login", account.Id },
                    { "server", ConnectionName(account) },
                    { "trade_ticket", tradeTicket },
                    { "symbol", symbol },
                    { "timeframe", timeframe.ToUpperInvariant() },
                    { "from_time", fromTime },
                    { "to_time", toTime },
                    { "bars", bars }
                };

                Dictionary<string, object> response = await PostAsync("/api/v1/nt8/series", seriesPayload);
                if (response == null)
                {
                    SendCommandResult(commandId, false, "Error al enviar las velas.");
                    return;
                }

                SendCommandResult(commandId, true, null);
                Log(string.Format("Velas {0} {1}: {2} barras.", timeframe, symbol, bars.Count));
            }
            catch (Exception ex)
            {
                SendCommandResult(commandId, false, ex.Message);
                Log("Velas: " + ex.Message);
            }
            finally
            {
                if (request != null)
                {
                    lock (activeRequests)
                        activeRequests.Remove(request);
                    try { request.Dispose(); } catch { }
                }
            }
        }

        private async Task<BarsRequest> RequestBarsAsync(
            Instrument instrument,
            BarsPeriod period,
            DateTime from,
            DateTime to)
        {
            TaskCompletionSource<BarsRequest> completion =
                new TaskCompletionSource<BarsRequest>();
            BarsRequest created = null;

            Action action = () =>
            {
                try
                {
                    TradingHours hours = instrument.MasterInstrument.TradingHours;
                    if (hours == null)
                        hours = TradingHours.Get(TradingHours.SystemDefault);

                    created = new BarsRequest(instrument, from, to)
                    {
                        BarsPeriod = period,
                        BarsBack = 0,
                        TradingHours = hours,
                        MergePolicy = MergePolicy.DoNotMerge
                    };
                    lock (activeRequests)
                        activeRequests.Add(created);

                    created.Request((bars, errorCode, errorMessage) =>
                    {
                        if (errorCode != ErrorCode.NoError)
                        {
                            NinjaTrader.Code.Output.Process(
                                "JournalTrading BarsRequest: " + errorCode + " " + errorMessage,
                                PrintTo.OutputTab1);
                            completion.TrySetResult(null);
                            return;
                        }
                        completion.TrySetResult(bars);
                    });
                }
                catch (Exception ex)
                {
                    completion.TrySetException(ex);
                }
            };

            System.Windows.Threading.Dispatcher dispatcher = NinjaTrader.Core.Globals.MainThreadDispatcher;
            if (dispatcher != null && !dispatcher.CheckAccess())
                _ = dispatcher.BeginInvoke(action);
            else
                action();

            Task finished = await Task.WhenAny(
                completion.Task,
                Task.Delay(TimeSpan.FromSeconds(90))).ConfigureAwait(false);

            if (finished != completion.Task)
            {
                if (created != null)
                {
                    lock (activeRequests)
                        activeRequests.Remove(created);
                    try { created.Dispose(); } catch { }
                }
                completion.TrySetResult(null);
                return null;
            }

            return await completion.Task.ConfigureAwait(false);
        }

        private static List<Dictionary<string, object>> CopyBars(BarsRequest request, BarsPeriod period)
        {
            List<Dictionary<string, object>> result = new List<Dictionary<string, object>>();
            Bars bars = request.Bars;
            if (bars == null)
                return result;

            int count = bars.Count;
            int start = Math.Max(0, count - 200000);
            for (int index = start; index < count; index++)
            {
                result.Add(new Dictionary<string, object>
                {
                    { "time", BarEpoch(bars, index, period) },
                    { "open", bars.GetOpen(index) },
                    { "high", bars.GetHigh(index) },
                    { "low", bars.GetLow(index) },
                    { "close", bars.GetClose(index) },
                    { "volume", (double)bars.GetVolume(index) },
                    { "spread", 0 }
                });
            }
            return result;
        }

        private static long BarEpoch(Bars bars, int index, BarsPeriod period)
        {
            DateTime time = bars.GetTime(index);
            switch (period.BarsPeriodType)
            {
                case BarsPeriodType.Minute:
                    return ToEpoch(time.AddMinutes(-period.Value));
                case BarsPeriodType.Day:
                    return ToEpoch(time.Date);
                case BarsPeriodType.Week:
                    int offset = ((int)time.DayOfWeek + 6) % 7;
                    return ToEpoch(time.Date.AddDays(-offset));
                default:
                    return ToEpoch(time);
            }
        }

        private static BarsPeriod BuildBarsPeriod(string timeframe)
        {
            BarsPeriod period = new BarsPeriod();
            switch ((timeframe ?? "").ToUpperInvariant())
            {
                case "M1":
                    period.BarsPeriodType = BarsPeriodType.Minute;
                    period.Value = 1;
                    break;
                case "M5":
                    period.BarsPeriodType = BarsPeriodType.Minute;
                    period.Value = 5;
                    break;
                case "M15":
                    period.BarsPeriodType = BarsPeriodType.Minute;
                    period.Value = 15;
                    break;
                case "M30":
                    period.BarsPeriodType = BarsPeriodType.Minute;
                    period.Value = 30;
                    break;
                case "H1":
                    period.BarsPeriodType = BarsPeriodType.Minute;
                    period.Value = 60;
                    break;
                case "H4":
                    period.BarsPeriodType = BarsPeriodType.Minute;
                    period.Value = 240;
                    break;
                case "D1":
                    period.BarsPeriodType = BarsPeriodType.Day;
                    period.Value = 1;
                    break;
                case "W1":
                    period.BarsPeriodType = BarsPeriodType.Week;
                    period.Value = 1;
                    break;
                default:
                    return null;
            }
            return period;
        }

        private static Instrument ResolveInstrument(string symbol)
        {
            try
            {
                Instrument instrument = Instrument.GetInstrument(symbol, true);
                if (instrument != null)
                    return instrument;
                return Instrument.GetInstrumentFuzzy(symbol);
            }
            catch
            {
                return null;
            }
        }

        // -------------------------------------------------------------------
        // HTTP
        // -------------------------------------------------------------------

        private async Task<Dictionary<string, object>> PostAsync(
            string path,
            Dictionary<string, object> payload)
        {
            if (string.IsNullOrWhiteSpace(config.ApiKey))
                return null;

            string url = config.ApiUrl.TrimEnd('/') + path;
            string body = NewSerializer().Serialize(payload);

            using (HttpRequestMessage request = new HttpRequestMessage(HttpMethod.Post, url))
            {
                request.Headers.TryAddWithoutValidation("X-Api-Key", config.ApiKey);
                request.Content = new StringContent(body, Encoding.UTF8, "application/json");

                using (HttpResponseMessage response = await http.SendAsync(request).ConfigureAwait(false))
                {
                    string text = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
                    if (!response.IsSuccessStatusCode)
                    {
                        LogThrottled(
                            "HTTP " + (int)response.StatusCode + " en " + path + ": " + text);
                        return null;
                    }

                    try
                    {
                        return NewSerializer().Deserialize<Dictionary<string, object>>(text);
                    }
                    catch
                    {
                        return new Dictionary<string, object>();
                    }
                }
            }
        }

        private void SendCommandResult(string commandId, bool ok, string error)
        {
            if (string.IsNullOrEmpty(commandId))
                return;

            Dictionary<string, object> payload = new Dictionary<string, object>
            {
                { "command_id", commandId },
                { "ok", ok },
                { "error", error }
            };

            Task.Run(() => PostAsync("/api/v1/nt8/commands/result", payload));
        }

        // -------------------------------------------------------------------
        // Snapshots de NT8 (siempre en el hilo principal)
        // -------------------------------------------------------------------

        private static List<Account> SnapshotAccounts()
        {
            List<Account> result = new List<Account>();
            Action action = () =>
            {
                foreach (Account account in Account.All)
                {
                    if (account == null || account.Id <= 0)
                        continue;
                    if (IsIgnoredAccount(account))
                        continue;
                    result.Add(account);
                }
            };
            RunOnMainThread(action);
            return result;
        }

        private static List<Execution> SnapshotExecutions(Account account)
        {
            List<Execution> result = new List<Execution>();
            Action action = () =>
            {
                foreach (Execution execution in account.Executions)
                {
                    if (execution != null)
                        result.Add(execution);
                }
            };
            RunOnMainThread(action);
            return result;
        }

        private static List<Order> SnapshotOrders(Account account)
        {
            List<Order> result = new List<Order>();
            Action action = () =>
            {
                foreach (Order order in account.Orders)
                {
                    if (order != null)
                        result.Add(order);
                }
            };
            RunOnMainThread(action);
            return result;
        }

        private static Position FindPosition(Account account, Instrument instrument, MarketPosition side)
        {
            Position result = null;
            Action action = () =>
            {
                foreach (Position position in account.Positions)
                {
                    if (position.Instrument == instrument && position.MarketPosition == side)
                    {
                        result = position;
                        break;
                    }
                }
            };
            RunOnMainThread(action);
            return result;
        }

        private static void RunOnMainThread(Action action)
        {
            System.Windows.Threading.Dispatcher dispatcher = NinjaTrader.Core.Globals.MainThreadDispatcher;
            if (dispatcher != null && !dispatcher.CheckAccess())
                dispatcher.Invoke(action);
            else
                action();
        }

        private static bool IsIgnoredAccount(Account account)
        {
            string name = account.Name ?? "";
            if (name.IndexOf("backtest", StringComparison.OrdinalIgnoreCase) >= 0)
                return true;
            if (name.IndexOf("playback", StringComparison.OrdinalIgnoreCase) >= 0)
                return true;
            if (account.Provider == Provider.Playback)
                return true;
            return false;
        }

        private static string ConnectionName(Account account)
        {
            try
            {
                if (account.Connection != null
                    && account.Connection.Options != null
                    && !string.IsNullOrWhiteSpace(account.Connection.Options.Name))
                    return account.Connection.Options.Name;
            }
            catch { }
            return account.Provider.ToString();
        }

        private static double SafeAccountValue(Account account, AccountItem item, Currency currency)
        {
            try
            {
                return SafeDouble(account.Get(item, currency));
            }
            catch
            {
                return 0.0;
            }
        }

        // -------------------------------------------------------------------
        // Utilidades
        // -------------------------------------------------------------------

        private static JavaScriptSerializer NewSerializer()
        {
            return new JavaScriptSerializer { MaxJsonLength = int.MaxValue };
        }

        private static double SafeDouble(double value)
        {
            return double.IsNaN(value) || double.IsInfinity(value) ? 0.0 : value;
        }

        private static double TickSize(Instrument instrument)
        {
            try
            {
                double value = instrument.MasterInstrument.TickSize;
                return value > 0 ? value : 0.01;
            }
            catch
            {
                return 0.01;
            }
        }

        private static double PointValue(Instrument instrument)
        {
            try
            {
                double value = instrument.MasterInstrument.PointValue;
                return value > 0 ? value : 1.0;
            }
            catch
            {
                return 1.0;
            }
        }

        private static double LotSize(Account account, Instrument instrument)
        {
            try
            {
                if (instrument.MasterInstrument.InstrumentType == InstrumentType.Forex)
                {
                    int size = account.ForexLotSize;
                    return size > 0 ? size : 100000.0;
                }
                return 1.0;
            }
            catch
            {
                return 1.0;
            }
        }

        private static double PriceValuePerUnit(Account account, Instrument instrument)
        {
            return PointValue(instrument) * LotSize(account, instrument) * ConversionRate(account, instrument);
        }

        private static double ConversionRate(Account account, Instrument instrument)
        {
            try
            {
                bool ok;
                double rate = instrument.GetConversionRate(
                    MarketDataType.Last,
                    account.Denomination,
                    out ok);
                return ok && rate > 0 ? rate : 1.0;
            }
            catch
            {
                return 1.0;
            }
        }

        private static int DigitsFromTickSize(double tickSize)
        {
            if (tickSize <= 0)
                return 0;
            string text = tickSize.ToString("0.##########", CultureInfo.InvariantCulture);
            int dot = text.IndexOf('.');
            return dot < 0 ? 0 : text.Length - dot - 1;
        }

        private static long StableHash(string value)
        {
            unchecked
            {
                ulong hash = 14695981039346656037UL;
                foreach (char character in value ?? "")
                {
                    hash ^= character;
                    hash *= 1099511628211UL;
                }
                // 53 bits: cabe exacto en un número JavaScript.
                return (long)(hash & 0x1FFFFFFFFFFFFFUL);
            }
        }

        private static string IsoLocal(DateTime value)
        {
            return value.ToString("yyyy-MM-dd'T'HH:mm:ss", CultureInfo.InvariantCulture);
        }

        private static string IsoUtc(DateTime value)
        {
            try
            {
                DateTime local = DateTime.SpecifyKind(value, DateTimeKind.Unspecified);
                DateTime utc = TimeZoneInfo.ConvertTimeToUtc(local, TimeZoneInfo.Local);
                return utc.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture);
            }
            catch
            {
                return DateTime.SpecifyKind(value, DateTimeKind.Utc)
                    .ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture);
            }
        }

        private static long ToEpoch(DateTime value)
        {
            DateTime epoch = new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Unspecified);
            return (long)(value - epoch).TotalSeconds;
        }

        private static DateTime FromEpoch(long seconds)
        {
            return new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Unspecified).AddSeconds(seconds);
        }

        private static string CurrencyCode(Currency currency)
        {
            switch (currency)
            {
                case Currency.UsDollar: return "USD";
                case Currency.Euro: return "EUR";
                case Currency.BritishPound: return "GBP";
                case Currency.JapaneseYen: return "JPY";
                case Currency.SwissFranc: return "CHF";
                case Currency.CanadianDollar: return "CAD";
                case Currency.AustralianDollar: return "AUD";
                case Currency.NewZealandDollar: return "NZD";
                case Currency.HongKongDollar: return "HKD";
                case Currency.SingaporeDollar: return "SGD";
                case Currency.MexicanPeso: return "MXN";
                case Currency.BrazilianReal: return "BRL";
                case Currency.SouthAfricanRand: return "ZAR";
                case Currency.SwedishKrona: return "SEK";
                case Currency.NorwayKrone: return "NOK";
                case Currency.DenmarkKrone: return "DKK";
                case Currency.PolandZloty: return "PLN";
                case Currency.CzechRepublicKoruna: return "CZK";
                case Currency.HungaryForint: return "HUF";
                case Currency.TurkeyLira: return "TRY";
                case Currency.IsraeliShekel: return "ILS";
                case Currency.RussiaRuble: return "RUB";
                case Currency.ChinaYuan: return "CNY";
                case Currency.KoreanWon: return "KRW";
                case Currency.IndianRupee: return "INR";
                case Currency.TaiwanNewDollar: return "TWD";
                case Currency.ThailandBaht: return "THB";
                case Currency.MalaysiaRinggit: return "MYR";
                case Currency.SaudiRiyal: return "SAR";
                default: return "USD";
            }
        }

        private static string GetString(Dictionary<string, object> data, string key)
        {
            if (data == null)
                return "";
            object value;
            return data.TryGetValue(key, out value) && value != null
                ? Convert.ToString(value, CultureInfo.InvariantCulture)
                : "";
        }

        private static int GetInt(Dictionary<string, object> data, string key, int fallback)
        {
            if (data == null)
                return fallback;
            object value;
            if (!data.TryGetValue(key, out value) || value == null)
                return fallback;
            try
            {
                return Convert.ToInt32(value, CultureInfo.InvariantCulture);
            }
            catch
            {
                return fallback;
            }
        }

        private static long GetLong(Dictionary<string, object> data, string key, long fallback)
        {
            if (data == null)
                return fallback;
            object value;
            if (!data.TryGetValue(key, out value) || value == null)
                return fallback;
            try
            {
                return Convert.ToInt64(value, CultureInfo.InvariantCulture);
            }
            catch
            {
                return fallback;
            }
        }

        private static bool GetBool(Dictionary<string, object> data, string key, bool fallback)
        {
            if (data == null)
                return fallback;
            object value;
            if (!data.TryGetValue(key, out value) || value == null)
                return fallback;
            try
            {
                return Convert.ToBoolean(value, CultureInfo.InvariantCulture);
            }
            catch
            {
                return fallback;
            }
        }

        private void Log(string message)
        {
            if (!config.Verbose)
                return;
            try
            {
                NinjaTrader.Code.Output.Process(
                    "JournalTrading: " + message,
                    PrintTo.OutputTab1);
            }
            catch { }
        }

        private void LogThrottled(string message)
        {
            DateTime now = DateTime.UtcNow;
            if (message == lastLogMessage && (now - lastLogAt).TotalSeconds < 30)
                return;
            lastLogAt = now;
            lastLogMessage = message;
            Log(message);
        }

        // -------------------------------------------------------------------
        // Tipos internos
        // -------------------------------------------------------------------

        private class SyncRequest
        {
            public bool ForceFull;
            public bool AllHistory;
            public int FromDays;
        }

        private class PositionGroup
        {
            public long Key;
            public Instrument Instrument;
            public MarketPosition Side;
            public double EntryQuantity;
            public double EntryValue;
            public double ExitQuantity;
            public DateTime FirstEntryTime;
            public readonly List<Execution> Executions = new List<Execution>();

            public double AverageEntryPrice
            {
                get { return EntryQuantity > 0 ? EntryValue / EntryQuantity : 0; }
            }

            public double OpenQuantity
            {
                get { return EntryQuantity - ExitQuantity; }
            }

            public bool IsOpen
            {
                get { return OpenQuantity > 0.0000001; }
            }

            public void Add(Execution execution)
            {
                Executions.Add(execution);
                if (execution.IsEntry)
                {
                    if (EntryQuantity <= 0)
                    {
                        Side = execution.MarketPosition;
                        FirstEntryTime = execution.Time;
                    }
                    EntryQuantity += execution.Quantity;
                    EntryValue += execution.Price * execution.Quantity;
                }
                else if (execution.IsExit)
                {
                    ExitQuantity += execution.Quantity;
                }
            }
        }

        private class BridgeConfig
        {
            public string ApiUrl = DefaultApiUrl;
            public string ApiKey = "";
            public int HeartbeatSeconds = 5;
            public int SyncSeconds = 15;
            public int HistoryDays;
            public bool Verbose = true;

            public static BridgeConfig From(Dictionary<string, object> data)
            {
                BridgeConfig result = new BridgeConfig();
                if (data == null)
                    return result;

                result.ApiUrl = GetString(data, "apiUrl");
                if (string.IsNullOrWhiteSpace(result.ApiUrl))
                    result.ApiUrl = DefaultApiUrl;

                result.ApiKey = GetString(data, "apiKey");
                result.HeartbeatSeconds = Math.Max(1, GetInt(data, "heartbeatSeconds", 5));
                result.SyncSeconds = Math.Max(5, GetInt(data, "syncSeconds", 15));
                result.HistoryDays = Math.Max(0, GetInt(data, "historyDays", 0));
                result.Verbose = GetBool(data, "verbose", true);
                return result;
            }
        }
    }
}
