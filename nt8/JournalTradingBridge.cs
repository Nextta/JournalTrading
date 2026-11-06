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
        private const string AddOnVersion = "1.1.0";
        private const int IncrementalDays = 7;
        private static readonly DateTime HistoryEpoch = new DateTime(2000, 1, 1);
        private const string ConfigFileName = "JournalTradingBridge.json";
        private const string DefaultApiUrl = "http://127.0.0.1:4477";

        private readonly HttpClient http = new HttpClient { Timeout = TimeSpan.FromSeconds(120) };
        private readonly Dictionary<long, SyncRequest> syncRequests = new Dictionary<long, SyncRequest>();
        private readonly Dictionary<long, AccountHistory> histories = new Dictionary<long, AccountHistory>();
        private readonly List<BarsRequest> activeRequests = new List<BarsRequest>();

        private Timer timer;
        private BridgeConfig config = new BridgeConfig();
        private DateTime lastConfigRead = DateTime.MinValue;
        private DateTime lastHeartbeat = DateTime.MinValue;
        private DateTime lastSync = DateTime.MinValue;
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
                { "broker_utc_offset_seconds", UtcOffsetSeconds(now) },
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
                syncRequests.TryGetValue(account.Id, out request);
            }

            AccountHistory history;
            lock (histories)
            {
                if (!histories.TryGetValue(account.Id, out history))
                {
                    history = new AccountHistory();
                    histories[account.Id] = history;
                }
            }

            // Una sincronización completa relee todo el historial de NT8 para poder reconstruir
            // las operaciones desde una posición plana; historyDays solo limita lo que se envía.
            bool full = request != null || !history.Initialized;
            DateTime now = DateTime.Now;
            int days = request != null && request.FromDays > 0 ? request.FromDays : config.HistoryDays;
            bool allHistory = (request != null && request.AllHistory) || days <= 0;
            DateTime windowStart = allHistory ? HistoryEpoch : now.AddDays(-days);

            Dictionary<string, Execution> executions = full
                ? new Dictionary<string, Execution>(StringComparer.Ordinal)
                : history.Executions;
            DateTime loadFrom = full ? HistoryEpoch : history.LastLoad.AddDays(-2);
            bool historyOk = true;

            try
            {
                Collection<Execution> historical = Execution.DbGet(account, loadFrom, now.AddDays(1));
                if (historical != null)
                {
                    foreach (Execution execution in historical)
                        AddExecution(executions, execution);
                }
            }
            catch (Exception ex)
            {
                historyOk = false;
                Log("Historial " + account.Name + ": " + ex.Message);
                if (full && history.Initialized)
                    executions = history.Executions;
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

            List<TradeGroup> trades = BuildTrades(account, executions.Values);
            DateTime sendFrom = full ? windowStart : now.AddDays(-IncrementalDays);
            List<TradeGroup> toSend = trades
                .Where(trade => trade.IsOpen || trade.LastTime >= sendFrom)
                .ToList();

            DateTime? reconcileFrom = full && historyOk ? (DateTime?)windowStart : null;
            Dictionary<string, object> payload = BuildSyncPayload(account, toSend, reconcileFrom);
            Dictionary<string, object> response = await PostAsync("/api/v1/nt8/sync", payload);
            if (response == null)
                return;

            history.Executions = executions;
            history.Initialized = true;
            history.LastLoad = now;
            lock (syncRequests)
            {
                SyncRequest current;
                if (request != null
                    && syncRequests.TryGetValue(account.Id, out current)
                    && ReferenceEquals(current, request))
                    syncRequests.Remove(account.Id);
            }

            int deals = toSend.Sum(trade => trade.Legs.Count);
            int openPositions = toSend.Count(trade => trade.IsOpen);
            Log(string.Format(
                "Sync {0}{1}: {2} ejecuciones en NT8, {3} operaciones y {4} deals enviados, {5} posiciones abiertas.",
                account.Name,
                full ? " (completa)" : "",
                executions.Count,
                toSend.Count,
                deals,
                openPositions));
        }

        /// <summary>
        /// Reconstruye las operaciones recorriendo las ejecuciones de cada instrumento en orden
        /// cronológico y acumulando la posición neta: una operación empieza cuando la posición
        /// sale de cero y termina cuando vuelve a cero. Las ejecuciones que invierten la posición
        /// se dividen en una salida y una nueva entrada.
        ///
        /// No se usa Execution.Position: en NT8 es el tamaño de la posición tras la ejecución,
        /// no un identificador de posición.
        /// </summary>
        private static List<TradeGroup> BuildTrades(Account account, IEnumerable<Execution> executions)
        {
            List<TradeGroup> result = new List<TradeGroup>();

            foreach (IGrouping<string, Execution> byInstrument in executions.GroupBy(item => item.Instrument.FullName))
            {
                string instrumentName = byInstrument.Key;
                double priceValue = 0;
                bool priceValueReady = false;
                TradeGroup current = null;
                int net = 0;

                IEnumerable<Execution> ordered = byInstrument
                    .OrderBy(item => item.Time)
                    .ThenBy(item => item.Id)
                    .ThenBy(item => item.ExecutionId ?? "", StringComparer.Ordinal);

                foreach (Execution execution in ordered)
                {
                    int sign = DirectionOf(execution);
                    if (sign == 0 || execution.Quantity <= 0)
                        continue;

                    if (!priceValueReady)
                    {
                        priceValue = PriceValuePerUnit(account, execution.Instrument);
                        priceValueReady = true;
                    }

                    int quantity = execution.Quantity;
                    int remaining = quantity;
                    double costs = SafeDouble(execution.Commission) + SafeDouble(execution.Fee);
                    string executionKey = ExecutionKey(execution);
                    int leg = 0;

                    if (current != null && net != 0 && Math.Sign(net) != sign)
                    {
                        int closeQuantity = Math.Min(remaining, Math.Abs(net));
                        current.AddExit(
                            execution,
                            DealTicket(account, executionKey, leg),
                            closeQuantity,
                            costs * closeQuantity / quantity,
                            priceValue);
                        leg++;
                        net += sign * closeQuantity;
                        remaining -= closeQuantity;
                        if (net == 0)
                            current = null;
                    }

                    if (remaining > 0)
                    {
                        if (current == null)
                        {
                            current = new TradeGroup
                            {
                                Key = PositiveHash(account.Id.ToString(CultureInfo.InvariantCulture)
                                    + "|" + instrumentName + "|" + executionKey + "|" + leg.ToString(CultureInfo.InvariantCulture)),
                                Instrument = execution.Instrument,
                                Side = sign > 0 ? MarketPosition.Long : MarketPosition.Short,
                                FirstEntryTime = execution.Time
                            };
                            result.Add(current);
                        }

                        current.AddEntry(
                            execution,
                            DealTicket(account, executionKey, leg),
                            remaining,
                            costs * remaining / quantity);
                        net += sign * remaining;
                    }
                }
            }

            return result;
        }

        private Dictionary<string, object> BuildSyncPayload(
            Account account,
            List<TradeGroup> trades,
            DateTime? reconcileFrom)
        {
            Dictionary<string, Dictionary<string, object>> symbols =
                new Dictionary<string, Dictionary<string, object>>();
            List<object> deals = new List<object>();
            List<object> positions = new List<object>();

            foreach (TradeGroup trade in trades)
            {
                Instrument instrument = trade.Instrument;
                if (instrument == null)
                    continue;

                if (!symbols.ContainsKey(instrument.FullName))
                    symbols[instrument.FullName] = BuildSymbol(account, instrument);

                foreach (TradeLeg leg in trade.Legs)
                    deals.Add(BuildDeal(trade, leg));

                if (trade.IsOpen)
                {
                    double tickSize = TickSize(instrument);
                    double priceValue = PriceValuePerUnit(account, instrument);
                    positions.Add(BuildPosition(account, trade, tickSize, priceValue));
                }
            }

            Dictionary<string, object> payload = new Dictionary<string, object>
            {
                { "login", account.Id },
                { "server", ConnectionName(account) },
                { "broker_utc_offset_seconds", UtcOffsetSeconds(DateTime.Now) },
                { "symbols", symbols.Values.ToList() },
                { "positions", positions },
                { "deals", deals },
                { "server_time", IsoLocal(DateTime.Now) }
            };
            if (reconcileFrom.HasValue)
                payload["history_from_utc"] = IsoUtc(reconcileFrom.Value);
            return payload;
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

        private static Dictionary<string, object> BuildDeal(TradeGroup trade, TradeLeg leg)
        {
            Execution execution = leg.Execution;
            return new Dictionary<string, object>
            {
                { "deal_ticket", leg.DealTicket },
                { "position_ticket", trade.Key },
                { "symbol", execution.Instrument.FullName },
                { "type", DirectionOf(execution) > 0 ? 0 : 1 },
                { "entry", leg.IsEntry ? 0 : 1 },
                { "volume", (double)leg.Quantity },
                { "price", SafeDouble(execution.Price) },
                { "profit", leg.Profit },
                { "commission", -Math.Abs(leg.Commission) },
                { "swap", 0.0 },
                { "magic", 0 },
                { "comment", CommentFor(execution) },
                { "reason", leg.IsEntry ? -1 : ReasonFor(execution) },
                { "time_server", IsoLocal(execution.Time) },
                { "time", ToEpoch(execution.Time) },
                { "time_utc", IsoUtc(execution.Time) }
            };
        }

        private Dictionary<string, object> BuildPosition(
            Account account,
            TradeGroup trade,
            double tickSize,
            double priceValue)
        {
            Instrument instrument = trade.Instrument;
            int side = trade.Side == MarketPosition.Long ? 0 : 1;
            double stopPrice;
            double targetPrice;
            FindStopTarget(account, instrument, trade.Side, trade.OpenAveragePrice, out stopPrice, out targetPrice);

            double profit = 0.0;
            try
            {
                Position position = FindPosition(account, instrument, trade.Side);
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
                { "ticket", trade.Key },
                { "symbol", instrument.FullName },
                { "type", side },
                { "volume", (double)trade.OpenQuantity },
                { "price_open", trade.OpenAveragePrice },
                { "sl", stopPrice > 0 ? (object)stopPrice : null },
                { "tp", targetPrice > 0 ? (object)targetPrice : null },
                { "profit", profit },
                { "swap", 0.0 },
                { "commission", 0.0 },
                { "magic", 0 },
                { "comment", "" },
                { "time_server", IsoLocal(trade.FirstEntryTime) },
                { "time", ToEpoch(trade.FirstEntryTime) },
                { "time_utc", IsoUtc(trade.FirstEntryTime) },
                { "tick_size", tickSize },
                { "tick_value", priceValue * tickSize }
            };
        }

        /// <summary>+1 compra, -1 venta, 0 desconocido. En una ejecución de NT8,
        /// MarketPosition indica el sentido de la ejecución (Long = compra).</summary>
        private static int DirectionOf(Execution execution)
        {
            if (execution.MarketPosition == MarketPosition.Long)
                return 1;
            if (execution.MarketPosition == MarketPosition.Short)
                return -1;

            try
            {
                if (execution.Order != null)
                {
                    OrderAction action = execution.Order.OrderAction;
                    return action == OrderAction.Buy || action == OrderAction.BuyToCover ? 1 : -1;
                }
            }
            catch { }
            return 0;
        }

        private static string ExecutionKey(Execution execution)
        {
            if (!string.IsNullOrWhiteSpace(execution.ExecutionId))
                return execution.ExecutionId;
            return "db:" + execution.Id.ToString(CultureInfo.InvariantCulture);
        }

        private static long DealTicket(Account account, string executionKey, int leg)
        {
            return PositiveHash(account.Id.ToString(CultureInfo.InvariantCulture)
                + "|" + executionKey + "|" + leg.ToString(CultureInfo.InvariantCulture));
        }

        private static long PositiveHash(string value)
        {
            long hash = StableHash(value);
            return hash > 0 ? hash : 1;
        }

        private static void AddExecution(Dictionary<string, Execution> map, Execution execution)
        {
            if (execution == null || execution.Instrument == null)
                return;
            if (execution.Quantity <= 0)
                return;
            if (execution.IsSod)
                return;

            // Se deduplica por ExecutionId: la misma ejecución puede llegar desde la base de
            // datos de NT8 y desde Account.Executions con distinto Id interno.
            map[ExecutionKey(execution)] = execution;
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

            // Las ejecuciones del historial no traen Order; NT8 guarda el nombre de la orden
            // (Stop1, Target1, Close, Exit...) en Execution.Name.
            string text = CommentFor(execution).Trim().ToLowerInvariant();
            if (text.StartsWith("stop") || text == "sl" || text.StartsWith("sl ") || text.Contains("stop loss"))
                return 4;
            if (text.StartsWith("target") || text.StartsWith("profit") || text == "tp" || text.StartsWith("tp "))
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

        /// <summary>Zona horaria configurada en NT8 (Herramientas > Opciones > General).
        /// Las horas de las ejecuciones vienen expresadas en ella.</summary>
        private static TimeZoneInfo NtTimeZone()
        {
            try
            {
                TimeZoneInfo zone = NinjaTrader.Core.Globals.GeneralOptions.TimeZoneInfo;
                if (zone != null)
                    return zone;
            }
            catch { }
            return TimeZoneInfo.Local;
        }

        private static int UtcOffsetSeconds(DateTime value)
        {
            try
            {
                DateTime local = DateTime.SpecifyKind(value, DateTimeKind.Unspecified);
                return (int)NtTimeZone().GetUtcOffset(local).TotalSeconds;
            }
            catch
            {
                return (int)TimeZoneInfo.Local.GetUtcOffset(DateTime.Now).TotalSeconds;
            }
        }

        private static string IsoUtc(DateTime value)
        {
            try
            {
                DateTime local = DateTime.SpecifyKind(value, DateTimeKind.Unspecified);
                DateTime utc = TimeZoneInfo.ConvertTimeToUtc(local, NtTimeZone());
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

        private class AccountHistory
        {
            public Dictionary<string, Execution> Executions =
                new Dictionary<string, Execution>(StringComparer.Ordinal);
            public DateTime LastLoad = DateTime.MinValue;
            public bool Initialized;
        }

        private class TradeLeg
        {
            public Execution Execution;
            public long DealTicket;
            public int Quantity;
            public bool IsEntry;
            public double Profit;
            public double Commission;
        }

        private class TradeGroup
        {
            public long Key;
            public Instrument Instrument;
            public MarketPosition Side;
            public int EntryQuantity;
            public double EntryValue;
            public int OpenQuantity;
            public double OpenAveragePrice;
            public DateTime FirstEntryTime;
            public DateTime LastTime;
            public readonly List<TradeLeg> Legs = new List<TradeLeg>();

            public bool IsOpen
            {
                get { return OpenQuantity > 0; }
            }

            public double AverageEntryPrice
            {
                get { return EntryQuantity > 0 ? EntryValue / EntryQuantity : 0; }
            }

            public void AddEntry(Execution execution, long dealTicket, int quantity, double commission)
            {
                double price = SafeDouble(execution.Price);
                OpenAveragePrice = OpenQuantity + quantity > 0
                    ? (OpenAveragePrice * OpenQuantity + price * quantity) / (OpenQuantity + quantity)
                    : price;
                OpenQuantity += quantity;
                EntryQuantity += quantity;
                EntryValue += price * quantity;
                LastTime = execution.Time;
                Legs.Add(new TradeLeg
                {
                    Execution = execution,
                    DealTicket = dealTicket,
                    Quantity = quantity,
                    IsEntry = true,
                    Profit = 0.0,
                    Commission = commission
                });
            }

            public void AddExit(
                Execution execution,
                long dealTicket,
                int quantity,
                double commission,
                double priceValue)
            {
                double direction = Side == MarketPosition.Long ? 1.0 : -1.0;
                double price = SafeDouble(execution.Price);
                double profit = (price - OpenAveragePrice) * priceValue * quantity * direction;
                OpenQuantity = Math.Max(0, OpenQuantity - quantity);
                LastTime = execution.Time;
                Legs.Add(new TradeLeg
                {
                    Execution = execution,
                    DealTicket = dealTicket,
                    Quantity = quantity,
                    IsEntry = false,
                    Profit = profit,
                    Commission = commission
                });
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
