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
    /// AddOn que sincroniza cuentas, ejecuciones y velas con JournalTrading a
    /// través de la API HTTP local (127.0.0.1). Solo envía órdenes cuando la app
    /// asigna a una cuenta el rol de secundaria en un grupo de copy trading.
    ///
    /// Configuración: Documents\NinjaTrader 8\bin\Custom\JournalTradingBridge.json
    /// La app puede generar ese archivo desde Ajustes > Puente NinjaTrader 8.
    /// </summary>
    public class JournalTradingBridge : AddOnBase
    {
        private const string AddOnVersion = "1.2.0";
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
                StartCopyTrading();
            }
            else if (State == State.Terminated || State == State.Finalized)
            {
                StopCopyTrading();
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
        // Copy trading
        //
        // Principal: cada 100 ms compara posiciones y órdenes de las cuentas
        // con rol principal y envía los cambios a /api/v1/copy/signal.
        // Secundaria: recoge sus órdenes con /api/v1/copy/poll (long-polling),
        // las ejecuta y devuelve el resultado a /api/v1/copy/result.
        // Lo copiado se identifica por el nombre de la orden ("JT…:E", ":SL"…)
        // y se lleva la cuenta del volumen copiado por etiqueta, de modo que
        // las operaciones manuales no se tocan.
        // -------------------------------------------------------------------

        private const string CopyStateFileName = "JournalTradingCopyState.json";
        private readonly object copyLock = new object();
        private readonly Dictionary<string, CopyTrack> copyTracks = new Dictionary<string, CopyTrack>();
        private readonly Dictionary<long, string> copyRoles = new Dictionary<long, string>();
        private readonly Dictionary<long, bool> copyEnabled = new Dictionary<long, bool>();
        private readonly Dictionary<long, MasterState> masterStates = new Dictionary<long, MasterState>();
        private readonly Dictionary<Order, string> orderRefs = new Dictionary<Order, string>(new ReferenceComparer<Order>());
        private CancellationTokenSource copyCts;
        private Timer copyTimer;
        private int copyMasterBusy;
        private long orderRefSeq;

        private static string CopyStatePath
        {
            get { return Path.Combine(NinjaTrader.Core.Globals.UserDataDir, "bin", "Custom", CopyStateFileName); }
        }

        private void StartCopyTrading()
        {
            StopCopyTrading();
            LoadCopyState();
            copyCts = new CancellationTokenSource();
            CancellationToken token = copyCts.Token;
            Task.Run(() => CopyLoopAsync(token));
            copyTimer = new Timer(OnCopyTimer, null, 1000, 100);
        }

        private void StopCopyTrading()
        {
            if (copyTimer != null)
            {
                copyTimer.Dispose();
                copyTimer = null;
            }
            if (copyCts != null)
            {
                try { copyCts.Cancel(); } catch { }
                copyCts = null;
            }
        }

        private string RoleOf(Account account)
        {
            lock (copyLock)
            {
                string role;
                return copyRoles.TryGetValue(account.Id, out role) ? role : "none";
            }
        }

        private bool CopyActive(Account account)
        {
            lock (copyLock)
            {
                bool enabled;
                return copyEnabled.TryGetValue(account.Id, out enabled) && enabled;
            }
        }

        // --- Bucle de consulta ------------------------------------------------

        private async Task CopyLoopAsync(CancellationToken token)
        {
            while (!token.IsCancellationRequested)
            {
                try
                {
                    if (!config.CopyTrading || string.IsNullOrWhiteSpace(config.ApiKey))
                    {
                        await Task.Delay(2000, token).ConfigureAwait(false);
                        continue;
                    }

                    List<Account> accounts = SnapshotAccounts();
                    bool anyFollower = false;
                    List<object> items = new List<object>();
                    foreach (Account account in accounts)
                    {
                        Currency currency = account.Denomination;
                        Dictionary<string, object> item = new Dictionary<string, object>
                        {
                            { "login", account.Id },
                            { "server", ConnectionName(account) },
                            { "balance", SafeAccountValue(account, AccountItem.CashValue, currency) },
                            { "equity", SafeAccountValue(account, AccountItem.NetLiquidation, currency) }
                        };
                        if (RoleOf(account) == "follower")
                        {
                            anyFollower = true;
                            item["copies"] = BuildCopies(account);
                        }
                        items.Add(item);
                    }

                    Dictionary<string, object> payload = new Dictionary<string, object>
                    {
                        { "provider", "nt8" },
                        { "bridge_version", AddOnVersion },
                        { "wait_ms", anyFollower ? 1000 : 0 },
                        { "accounts", items }
                    };
                    Dictionary<string, object> response = await PostAsync("/api/v1/copy/poll", payload).ConfigureAwait(false);
                    if (response == null)
                    {
                        await Task.Delay(2000, token).ConfigureAwait(false);
                        continue;
                    }

                    List<Dictionary<string, object>> results = new List<Dictionary<string, object>>();
                    foreach (Dictionary<string, object> entry in GetList(response, "accounts"))
                    {
                        long login = GetLong(entry, "login", 0);
                        string role = GetString(entry, "role");
                        bool enabled = GetBool(entry, "enabled", false);
                        Account account = accounts.FirstOrDefault(a => a.Id == login);
                        if (account == null)
                            continue;
                        UpdateRole(account, string.IsNullOrEmpty(role) ? "none" : role, enabled);
                        if (role != "follower")
                            continue;
                        foreach (Dictionary<string, object> order in GetList(entry, "orders"))
                        {
                            Dictionary<string, object> result = await ExecuteCopyOrderAsync(account, order).ConfigureAwait(false);
                            if (result != null)
                                results.Add(result);
                        }
                    }

                    if (results.Count > 0)
                    {
                        await PostAsync("/api/v1/copy/result", new Dictionary<string, object> { { "results", results } })
                            .ConfigureAwait(false);
                        SaveCopyState();
                    }

                    if (!anyFollower)
                        await Task.Delay(1000, token).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
                catch (Exception ex)
                {
                    LogThrottled("Copy trading: " + ex.Message);
                    try { await Task.Delay(2000, token).ConfigureAwait(false); } catch { return; }
                }
            }
        }

        private void UpdateRole(Account account, string role, bool enabled)
        {
            bool changed;
            lock (copyLock)
            {
                string previousRole;
                bool previousEnabled;
                copyRoles.TryGetValue(account.Id, out previousRole);
                copyEnabled.TryGetValue(account.Id, out previousEnabled);
                changed = previousRole != role || previousEnabled != enabled;
                copyRoles[account.Id] = role;
                copyEnabled[account.Id] = enabled;
                if (changed && role == "master" && enabled)
                    masterStates.Remove(account.Id);
            }
            if (changed)
                Log("Copy trading · " + account.Name + " · rol " + role + (enabled ? " · activo" : " · pausado"));
        }

        // --- Principal ----------------------------------------------------------

        private void OnCopyTimer(object state)
        {
            if (!config.CopyTrading || Interlocked.CompareExchange(ref copyMasterBusy, 1, 0) != 0)
                return;
            Task.Run(async () =>
            {
                try
                {
                    foreach (Account account in SnapshotAccounts())
                    {
                        if (RoleOf(account) != "master" || !CopyActive(account))
                            continue;
                        List<Dictionary<string, object>> signals = DiffMaster(account);
                        if (signals.Count == 0)
                            continue;
                        Dictionary<string, object> payload = new Dictionary<string, object>
                        {
                            { "provider", "nt8" },
                            { "login", account.Id },
                            { "server", ConnectionName(account) },
                            { "signals", signals }
                        };
                        await PostAsync("/api/v1/copy/signal", payload).ConfigureAwait(false);
                    }
                }
                catch (Exception ex)
                {
                    LogThrottled("Copy trading (principal): " + ex.Message);
                }
                finally
                {
                    Interlocked.Exchange(ref copyMasterBusy, 0);
                }
            });
        }

        private static bool IsBuyAction(OrderAction action)
        {
            return action == OrderAction.Buy || action == OrderAction.BuyToCover;
        }

        private static string MarketOf(Instrument instrument)
        {
            try
            {
                switch (instrument.MasterInstrument.InstrumentType)
                {
                    case InstrumentType.Future:
                        return "futures";
                    case InstrumentType.Forex:
                    case InstrumentType.Cfd:
                        return "cfd_forex";
                }
            }
            catch { }
            return "other";
        }

        private string RefOf(Order order)
        {
            lock (copyLock)
            {
                string value;
                if (!orderRefs.TryGetValue(order, out value))
                {
                    value = "NT" + Interlocked.Increment(ref orderRefSeq).ToString(CultureInfo.InvariantCulture)
                        + "-" + DateTime.UtcNow.Ticks.ToString("x", CultureInfo.InvariantCulture);
                    orderRefs[order] = value;
                }
                return value;
            }
        }

        private MasterSnapshot TakeMasterSnapshot(Account account)
        {
            MasterSnapshot snapshot = new MasterSnapshot();
            Action action = () =>
            {
                foreach (Position position in account.Positions)
                {
                    if (position == null || position.Instrument == null || position.MarketPosition == MarketPosition.Flat)
                        continue;
                    snapshot.Positions[position.Instrument.FullName] = new PosSnap
                    {
                        Instrument = position.Instrument,
                        Side = position.MarketPosition == MarketPosition.Long ? 1 : -1,
                        Quantity = position.Quantity,
                        AveragePrice = position.AveragePrice
                    };
                }

                foreach (Order order in account.Orders)
                {
                    if (order == null || order.Instrument == null || !IsWorking(order) || order.OrderType == OrderType.Market)
                        continue;
                    string key = order.Instrument.FullName;
                    int remaining = order.Quantity - order.Filled;
                    bool buy = IsBuyAction(order.OrderAction);
                    PosSnap position;
                    bool protective = snapshot.Positions.TryGetValue(key, out position)
                        && buy != (position.Side > 0)
                        && remaining <= position.Quantity;
                    if (protective)
                    {
                        if (order.OrderType == OrderType.StopMarket || order.OrderType == OrderType.StopLimit)
                        {
                            double stop = order.StopPrice;
                            if (position.StopLoss == 0 || (position.Side > 0 ? stop > position.StopLoss : stop < position.StopLoss))
                                position.StopLoss = stop;
                        }
                        else if (order.OrderType == OrderType.Limit || order.OrderType == OrderType.MIT)
                        {
                            double target = order.OrderType == OrderType.Limit ? order.LimitPrice : order.StopPrice;
                            if (position.TakeProfit == 0 || (position.Side > 0 ? target < position.TakeProfit : target > position.TakeProfit))
                                position.TakeProfit = target;
                        }
                        continue;
                    }

                    string type;
                    double price;
                    double stopLimit = 0;
                    switch (order.OrderType)
                    {
                        case OrderType.Limit:
                            type = "limit";
                            price = order.LimitPrice;
                            break;
                        case OrderType.StopMarket:
                            type = "stop";
                            price = order.StopPrice;
                            break;
                        case OrderType.StopLimit:
                            type = "stop_limit";
                            price = order.StopPrice;
                            stopLimit = order.LimitPrice;
                            break;
                        default:
                            continue;
                    }
                    snapshot.Orders[RefOf(order)] = new OrdSnap
                    {
                        Order = order,
                        Instrument = order.Instrument,
                        Side = buy ? "buy" : "sell",
                        Type = type,
                        Quantity = remaining,
                        Price = price,
                        StopLimit = stopLimit
                    };
                }
            };
            RunOnMainThread(action);
            return snapshot;
        }

        private List<Dictionary<string, object>> DiffMaster(Account account)
        {
            List<Dictionary<string, object>> signals = new List<Dictionary<string, object>>();
            MasterSnapshot current = TakeMasterSnapshot(account);
            MasterSnapshot previous;
            lock (copyLock)
            {
                MasterState state;
                if (!masterStates.TryGetValue(account.Id, out state))
                {
                    // Lo que ya estaba abierto al activar la copia no se copia.
                    masterStates[account.Id] = new MasterState { Snapshot = current };
                    return signals;
                }
                previous = state.Snapshot;
                state.Snapshot = current;
            }

            // Pendientes que han desaparecido: ejecutadas o canceladas.
            Dictionary<string, string> filledByInstrument = new Dictionary<string, string>();
            foreach (KeyValuePair<string, OrdSnap> pair in previous.Orders)
            {
                if (current.Orders.ContainsKey(pair.Key))
                    continue;
                OrderState state = pair.Value.Order.OrderState;
                if (state == OrderState.Filled || state == OrderState.PartFilled)
                    filledByInstrument[pair.Value.Instrument.FullName] = pair.Key;
                else
                    signals.Add(OrderSignal("pending_cancel", pair.Key, pair.Value));
            }

            foreach (KeyValuePair<string, PosSnap> pair in current.Positions)
            {
                PosSnap now = pair.Value;
                PosSnap before;
                string orderRef;
                filledByInstrument.TryGetValue(pair.Key, out orderRef);
                if (!previous.Positions.TryGetValue(pair.Key, out before))
                {
                    signals.Add(PositionSignal("open", pair.Key, now, now.Quantity, now.Quantity, now.AveragePrice, orderRef));
                    continue;
                }
                if (before.Side != now.Side)
                {
                    signals.Add(PositionSignal("close", pair.Key, before, before.Quantity, 0, now.AveragePrice, null));
                    signals.Add(PositionSignal("open", pair.Key, now, now.Quantity, now.Quantity, now.AveragePrice, orderRef));
                    continue;
                }
                if (now.Quantity > before.Quantity)
                {
                    int added = now.Quantity - before.Quantity;
                    double fill = (now.AveragePrice * now.Quantity - before.AveragePrice * before.Quantity) / added;
                    signals.Add(PositionSignal("increase", pair.Key, now, added, now.Quantity, fill, orderRef));
                }
                else if (now.Quantity < before.Quantity)
                {
                    signals.Add(PositionSignal("reduce", pair.Key, now, before.Quantity - now.Quantity, now.Quantity, now.AveragePrice, null));
                }
                if (Math.Abs(now.StopLoss - before.StopLoss) > 1e-12 || Math.Abs(now.TakeProfit - before.TakeProfit) > 1e-12)
                    signals.Add(PositionSignal("sltp", pair.Key, now, 0, now.Quantity, now.AveragePrice, null));
            }

            foreach (KeyValuePair<string, PosSnap> pair in previous.Positions)
            {
                if (!current.Positions.ContainsKey(pair.Key))
                    signals.Add(PositionSignal("close", pair.Key, pair.Value, pair.Value.Quantity, 0, pair.Value.AveragePrice, null));
            }

            foreach (KeyValuePair<string, OrdSnap> pair in current.Orders)
            {
                OrdSnap before;
                if (!previous.Orders.TryGetValue(pair.Key, out before))
                {
                    signals.Add(OrderSignal("pending_place", pair.Key, pair.Value));
                    continue;
                }
                OrdSnap now = pair.Value;
                if (Math.Abs(now.Price - before.Price) > 1e-12 || Math.Abs(now.StopLimit - before.StopLimit) > 1e-12
                    || now.Quantity != before.Quantity)
                    signals.Add(OrderSignal("pending_modify", pair.Key, now));
            }
            return signals;
        }

        private static Dictionary<string, object> PositionSignal(
            string kind, string key, PosSnap position, int volume, int remaining, double price, string orderRef)
        {
            Dictionary<string, object> signal = new Dictionary<string, object>
            {
                { "kind", kind },
                { "ref", key },
                { "symbol", key },
                { "market", MarketOf(position.Instrument) },
                { "side", position.Side > 0 ? "buy" : "sell" },
                { "volume", volume },
                { "position_volume", remaining },
                { "price", price },
                { "entry_price", position.AveragePrice },
                { "sl", position.StopLoss > 0 ? (object)position.StopLoss : null },
                { "tp", position.TakeProfit > 0 ? (object)position.TakeProfit : null }
            };
            if (!string.IsNullOrEmpty(orderRef))
                signal["order_ref"] = orderRef;
            return signal;
        }

        private static Dictionary<string, object> OrderSignal(string kind, string orderRef, OrdSnap order)
        {
            return new Dictionary<string, object>
            {
                { "kind", kind },
                { "ref", orderRef },
                { "order_ref", orderRef },
                { "symbol", order.Instrument.FullName },
                { "market", MarketOf(order.Instrument) },
                { "side", order.Side },
                { "order_type", order.Type },
                { "volume", order.Quantity },
                { "price", order.Price },
                { "order_price", order.Price },
                { "stop_limit_price", order.StopLimit > 0 ? (object)order.StopLimit : null }
            };
        }

        // --- Secundaria -----------------------------------------------------------

        private static string TrackKey(Account account, string tag)
        {
            return account.Id.ToString(CultureInfo.InvariantCulture) + "|" + tag;
        }

        private CopyTrack GetTrack(Account account, string tag)
        {
            lock (copyLock)
            {
                CopyTrack track;
                return copyTracks.TryGetValue(TrackKey(account, tag), out track) ? track : null;
            }
        }

        private CopyTrack EnsureTrack(Account account, string tag, Instrument instrument, int side)
        {
            lock (copyLock)
            {
                string key = TrackKey(account, tag);
                CopyTrack track;
                if (!copyTracks.TryGetValue(key, out track))
                {
                    track = new CopyTrack { AccountId = account.Id, Tag = tag };
                    copyTracks[key] = track;
                }
                track.Instrument = instrument.FullName;
                track.Side = side;
                return track;
            }
        }

        private void RemoveTrack(Account account, string tag)
        {
            lock (copyLock)
            {
                copyTracks.Remove(TrackKey(account, tag));
            }
        }

        private static int NetQuantity(Account account, string instrumentName, int side)
        {
            int result = 0;
            Action action = () =>
            {
                foreach (Position position in account.Positions)
                {
                    if (position.Instrument == null || position.Instrument.FullName != instrumentName)
                        continue;
                    if (position.MarketPosition == MarketPosition.Long && side > 0)
                        result += position.Quantity;
                    else if (position.MarketPosition == MarketPosition.Short && side < 0)
                        result += position.Quantity;
                }
            };
            RunOnMainThread(action);
            return result;
        }

        private List<object> BuildCopies(Account account)
        {
            List<CopyTrack> tracks;
            lock (copyLock)
            {
                tracks = copyTracks.Values.Where(t => t.AccountId == account.Id && t.Quantity > 0).ToList();
            }
            List<object> copies = new List<object>();
            foreach (CopyTrack track in tracks)
            {
                // Si la posición se cerró (SL/TP, cierre manual) lo copiado no puede superar la posición real.
                int net = NetQuantity(account, track.Instrument, track.Side);
                int quantity = Math.Min(track.Quantity, net);
                if (quantity != track.Quantity)
                {
                    lock (copyLock)
                    {
                        track.Quantity = quantity;
                    }
                    SaveCopyState();
                }
                copies.Add(new Dictionary<string, object> { { "tag", track.Tag }, { "volume", quantity } });
            }
            return copies;
        }

        private static Instrument ResolveCopyInstrument(Dictionary<string, object> order)
        {
            foreach (object value in GetArray(order, "symbols"))
            {
                string name = Convert.ToString(value, CultureInfo.InvariantCulture);
                if (string.IsNullOrWhiteSpace(name))
                    continue;
                try
                {
                    Instrument instrument = Instrument.GetInstrument(name.Trim(), true);
                    if (instrument != null)
                        return instrument;
                }
                catch { }
            }
            return null;
        }

        private static Dictionary<string, object> CopyResult(
            string id, bool ok, string status, string error, Instrument instrument,
            double volume, double price, string orderRef)
        {
            Dictionary<string, object> result = new Dictionary<string, object>
            {
                { "order_id", id },
                { "ok", ok },
                { "status", status }
            };
            if (!string.IsNullOrEmpty(error))
                result["error"] = error;
            if (instrument != null)
                result["symbol"] = instrument.FullName;
            if (volume > 0)
                result["volume"] = volume;
            if (price > 0)
                result["price"] = price;
            if (!string.IsNullOrEmpty(orderRef))
                result["order_ref"] = orderRef;
            return result;
        }

        private async Task<Dictionary<string, object>> ExecuteCopyOrderAsync(Account account, Dictionary<string, object> order)
        {
            string id = GetString(order, "id");
            string action = GetString(order, "action");
            if (string.IsNullOrEmpty(id))
                return null;
            try
            {
                Instrument instrument = ResolveCopyInstrument(order);
                if (instrument == null)
                {
                    string names = string.Join(", ", GetArray(order, "symbols").Select(v => Convert.ToString(v, CultureInfo.InvariantCulture)));
                    return CopyResult(id, false, "rejected", "Instrumento no disponible en NinjaTrader: " + names, null, 0, 0, null);
                }

                switch (action)
                {
                    case "market":
                        return GetBool(order, "reduce_only", false)
                            ? await CopyReduceAsync(account, order, id, instrument, false).ConfigureAwait(false)
                            : await CopyEntryAsync(account, order, id, instrument, GetString(order, "tag")).ConfigureAwait(false);
                    case "close":
                        return await CopyReduceAsync(account, order, id, instrument, true).ConfigureAwait(false);
                    case "sltp":
                        return CopySlTp(account, order, id, instrument);
                    case "pending_place":
                        return await CopyPendingPlaceAsync(account, order, id, instrument).ConfigureAwait(false);
                    case "pending_modify":
                        return CopyPendingModify(account, order, id, instrument);
                    case "pending_cancel":
                        return CopyPendingCancel(account, order, id, instrument);
                    case "pending_fill":
                        return await CopyPendingFillAsync(account, order, id, instrument).ConfigureAwait(false);
                    default:
                        return CopyResult(id, false, "rejected", "Acción desconocida: " + action, instrument, 0, 0, null);
                }
            }
            catch (Exception ex)
            {
                return CopyResult(id, false, "rejected", ex.Message, null, 0, 0, null);
            }
        }

        private static async Task<Order> WaitForFinalAsync(Order order, int timeoutMs, bool workingIsFinal)
        {
            DateTime limit = DateTime.UtcNow.AddMilliseconds(timeoutMs);
            while (DateTime.UtcNow < limit)
            {
                OrderState state = order.OrderState;
                if (state == OrderState.Filled || state == OrderState.Rejected || state == OrderState.Cancelled)
                    return order;
                if (workingIsFinal && (state == OrderState.Working || state == OrderState.Accepted || state == OrderState.TriggerPending))
                    return order;
                await Task.Delay(25).ConfigureAwait(false);
            }
            return order;
        }

        private static Order SubmitOrder(
            Account account, Instrument instrument, OrderAction action, OrderType type, TimeInForce tif,
            int quantity, double limitPrice, double stopPrice, string oco, string name)
        {
            Order order = null;
            Action submit = () =>
            {
                order = account.CreateOrder(instrument, action, type, OrderEntry.Automated, tif, quantity,
                    limitPrice, stopPrice, oco ?? "", name, NinjaTrader.Core.Globals.MaxDate, null);
                account.Submit(new[] { order });
            };
            RunOnMainThread(submit);
            return order;
        }

        private static double RoundPrice(Instrument instrument, double price)
        {
            if (price <= 0)
                return 0;
            try
            {
                return instrument.MasterInstrument.RoundToTickSize(price);
            }
            catch
            {
                return price;
            }
        }

        private static void ResolveSlTp(Dictionary<string, object> order, Instrument instrument, double basePrice, out double sl, out double tp)
        {
            if (GetString(order, "price_mode") == "offset")
            {
                double slOffset = GetDouble(order, "sl_offset");
                double tpOffset = GetDouble(order, "tp_offset");
                sl = slOffset != 0 ? basePrice + slOffset : 0;
                tp = tpOffset != 0 ? basePrice + tpOffset : 0;
            }
            else
            {
                sl = GetDouble(order, "sl");
                tp = GetDouble(order, "tp");
            }
            sl = RoundPrice(instrument, sl);
            tp = RoundPrice(instrument, tp);
        }

        private static List<Order> FindOrders(Account account, string name, bool workingOnly)
        {
            List<Order> result = new List<Order>();
            Action action = () =>
            {
                foreach (Order order in account.Orders)
                {
                    if (order != null && order.Name == name && (!workingOnly || IsWorking(order)))
                        result.Add(order);
                }
            };
            RunOnMainThread(action);
            return result;
        }

        private static void CancelOrders(Account account, List<Order> orders)
        {
            if (orders.Count == 0)
                return;
            Action action = () => account.Cancel(orders);
            RunOnMainThread(action);
        }

        // Coloca (o sustituye) el stop y el objetivo de lo copiado como órdenes OCO.
        private void PlaceProtective(Account account, Instrument instrument, CopyTrack track, double sl, double tp)
        {
            List<Order> existing = FindOrders(account, track.Tag + ":SL", true);
            existing.AddRange(FindOrders(account, track.Tag + ":TP", true));
            CancelOrders(account, existing);
            lock (copyLock)
            {
                track.StopLoss = sl;
                track.TakeProfit = tp;
            }
            if (track.Quantity <= 0 || (sl <= 0 && tp <= 0))
                return;
            OrderAction exit = track.Side > 0 ? OrderAction.Sell : OrderAction.BuyToCover;
            string oco = sl > 0 && tp > 0 ? track.Tag + "-" + DateTime.UtcNow.Ticks.ToString("x", CultureInfo.InvariantCulture) : "";
            if (sl > 0)
                SubmitOrder(account, instrument, exit, OrderType.StopMarket, TimeInForce.Gtc, track.Quantity, 0, sl, oco, track.Tag + ":SL");
            if (tp > 0)
                SubmitOrder(account, instrument, exit, OrderType.Limit, TimeInForce.Gtc, track.Quantity, tp, 0, oco, track.Tag + ":TP");
        }

        private async Task<Dictionary<string, object>> CopyEntryAsync(
            Account account, Dictionary<string, object> order, string id, Instrument instrument, string tag)
        {
            string side = GetString(order, "side");
            int quantity = (int)Math.Floor(GetDouble(order, "volume") + 1e-9);
            if (quantity < 1)
                return CopyResult(id, false, "skipped", "Tamaño menor que 1 contrato", instrument, 0, 0, null);

            double maxDeviation = GetDouble(order, "max_deviation");
            double reference = GetDouble(order, "reference_price");
            double last = LastPrice(instrument);
            double tick = TickSize(instrument);
            if (maxDeviation > 0 && reference > 0 && last > 0 && tick > 0
                && Math.Abs(last - reference) / tick > maxDeviation + 1e-9)
                return CopyResult(id, false, "skipped",
                    "Precio desviado " + (Math.Abs(last - reference) / tick).ToString("0.#", CultureInfo.InvariantCulture)
                    + " ticks de la entrada de la principal", instrument, 0, 0, null);

            OrderAction entryAction = side == "buy" ? OrderAction.Buy : OrderAction.SellShort;
            Order submitted = SubmitOrder(account, instrument, entryAction, OrderType.Market, TimeInForce.Day, quantity, 0, 0, null, tag + ":E");
            await WaitForFinalAsync(submitted, 15000, false).ConfigureAwait(false);
            int filled = submitted.Filled;
            if (filled <= 0)
                return CopyResult(id, false, "rejected", "Orden " + submitted.OrderState, instrument, 0, 0, null);

            int sideSign = side == "buy" ? 1 : -1;
            CopyTrack track = EnsureTrack(account, tag, instrument, sideSign);
            lock (copyLock)
            {
                track.Quantity += filled;
            }
            double sl, tp;
            ResolveSlTp(order, instrument, submitted.AverageFillPrice, out sl, out tp);
            if (sl <= 0 && tp <= 0)
            {
                sl = track.StopLoss;
                tp = track.TakeProfit;
            }
            PlaceProtective(account, instrument, track, sl, tp);
            SaveCopyState();
            return CopyResult(id, true, "filled", null, instrument, filled, submitted.AverageFillPrice, null);
        }

        private async Task<Dictionary<string, object>> CopyReduceAsync(
            Account account, Dictionary<string, object> order, string id, Instrument instrument, bool closeAll)
        {
            string tag = GetString(order, "tag");
            CopyTrack track = GetTrack(account, tag);
            if (track == null || track.Quantity <= 0)
            {
                RemoveTrack(account, tag);
                return CopyResult(id, true, "done", "No hay nada copiado abierto", instrument, 0, 0, null);
            }
            int net = NetQuantity(account, track.Instrument, track.Side);
            int requested = closeAll ? track.Quantity : (int)Math.Floor(GetDouble(order, "volume") + 1e-9);
            int quantity = Math.Min(Math.Min(requested, track.Quantity), net);
            if (quantity <= 0)
            {
                lock (copyLock)
                {
                    track.Quantity = Math.Min(track.Quantity, net);
                }
                if (track.Quantity <= 0)
                {
                    PlaceProtective(account, instrument, track, 0, 0);
                    RemoveTrack(account, tag);
                }
                return CopyResult(id, true, "done", "La posición ya estaba cerrada", instrument, 0, 0, null);
            }

            OrderAction exit = track.Side > 0 ? OrderAction.Sell : OrderAction.BuyToCover;
            Order submitted = SubmitOrder(account, instrument, exit, OrderType.Market, TimeInForce.Day, quantity, 0, 0, null, tag + ":X");
            await WaitForFinalAsync(submitted, 15000, false).ConfigureAwait(false);
            int filled = submitted.Filled;
            if (filled <= 0)
                return CopyResult(id, false, "rejected", "Orden " + submitted.OrderState, instrument, 0, 0, null);

            lock (copyLock)
            {
                track.Quantity = Math.Max(0, track.Quantity - filled);
            }
            if (track.Quantity <= 0)
            {
                PlaceProtective(account, instrument, track, 0, 0);
                RemoveTrack(account, tag);
            }
            else
            {
                PlaceProtective(account, instrument, track, track.StopLoss, track.TakeProfit);
            }
            SaveCopyState();
            return CopyResult(id, true, "filled", null, instrument, filled, submitted.AverageFillPrice, null);
        }

        private Dictionary<string, object> CopySlTp(Account account, Dictionary<string, object> order, string id, Instrument instrument)
        {
            CopyTrack track = GetTrack(account, GetString(order, "tag"));
            if (track == null || track.Quantity <= 0)
                return CopyResult(id, true, "done", "No hay nada copiado abierto", instrument, 0, 0, null);
            double average = AveragePrice(account, instrument, track.Side);
            double sl, tp;
            ResolveSlTp(order, instrument, average, out sl, out tp);
            PlaceProtective(account, instrument, track, sl, tp);
            SaveCopyState();
            return CopyResult(id, true, "done", null, instrument, 0, 0, null);
        }

        private async Task<Dictionary<string, object>> CopyPendingPlaceAsync(
            Account account, Dictionary<string, object> order, string id, Instrument instrument)
        {
            if (GetString(order, "price_mode") == "offset")
                return CopyResult(id, false, "rejected", "NinjaTrader solo copia pendientes con precio absoluto (futuros)", instrument, 0, 0, null);
            string tag = GetString(order, "tag");
            string side = GetString(order, "side");
            int quantity = (int)Math.Floor(GetDouble(order, "volume") + 1e-9);
            if (quantity < 1)
                return CopyResult(id, false, "skipped", "Tamaño menor que 1 contrato", instrument, 0, 0, null);

            string type = GetString(order, "order_type");
            double price = RoundPrice(instrument, GetDouble(order, "price"));
            double stopLimit = RoundPrice(instrument, GetDouble(order, "stop_limit_price"));
            OrderType orderType;
            double limitPrice = 0;
            double stopPrice = 0;
            if (type == "stop")
            {
                orderType = OrderType.StopMarket;
                stopPrice = price;
            }
            else if (type == "stop_limit")
            {
                orderType = OrderType.StopLimit;
                stopPrice = price;
                limitPrice = stopLimit > 0 ? stopLimit : price;
            }
            else
            {
                orderType = OrderType.Limit;
                limitPrice = price;
            }

            OrderAction action = side == "buy" ? OrderAction.Buy : OrderAction.SellShort;
            Order submitted = SubmitOrder(account, instrument, action, orderType, TimeInForce.Gtc, quantity, limitPrice, stopPrice, null, tag + ":P");
            await WaitForFinalAsync(submitted, 5000, true).ConfigureAwait(false);
            if (submitted.OrderState == OrderState.Rejected || submitted.OrderState == OrderState.Cancelled)
                return CopyResult(id, false, "rejected", "Orden " + submitted.OrderState, instrument, 0, 0, null);

            CopyTrack track = EnsureTrack(account, tag, instrument, side == "buy" ? 1 : -1);
            double sl, tp;
            ResolveSlTp(order, instrument, price, out sl, out tp);
            lock (copyLock)
            {
                track.StopLoss = sl;
                track.TakeProfit = tp;
            }
            SaveCopyState();
            return CopyResult(id, true, "placed", null, instrument, quantity, price, submitted.OrderId);
        }

        private Dictionary<string, object> CopyPendingModify(Account account, Dictionary<string, object> order, string id, Instrument instrument)
        {
            string tag = GetString(order, "tag");
            List<Order> working = FindOrders(account, tag + ":P", true);
            if (working.Count == 0)
                return CopyResult(id, true, "done", "La orden pendiente ya no existe", instrument, 0, 0, null);
            double price = RoundPrice(instrument, GetDouble(order, "price"));
            double stopLimit = RoundPrice(instrument, GetDouble(order, "stop_limit_price"));
            Action change = () =>
            {
                foreach (Order item in working)
                {
                    if (item.OrderType == OrderType.Limit)
                        item.LimitPriceChanged = price;
                    else if (item.OrderType == OrderType.StopMarket)
                        item.StopPriceChanged = price;
                    else if (item.OrderType == OrderType.StopLimit)
                    {
                        item.StopPriceChanged = price;
                        item.LimitPriceChanged = stopLimit > 0 ? stopLimit : price;
                    }
                }
                account.Change(working);
            };
            RunOnMainThread(change);
            CopyTrack track = GetTrack(account, tag);
            if (track != null)
            {
                double sl, tp;
                ResolveSlTp(order, instrument, price, out sl, out tp);
                lock (copyLock)
                {
                    track.StopLoss = sl;
                    track.TakeProfit = tp;
                }
                SaveCopyState();
            }
            return CopyResult(id, true, "done", null, instrument, 0, price, null);
        }

        private Dictionary<string, object> CopyPendingCancel(Account account, Dictionary<string, object> order, string id, Instrument instrument)
        {
            string tag = GetString(order, "tag");
            CancelOrders(account, FindOrders(account, tag + ":P", true));
            CopyTrack track = GetTrack(account, tag);
            if (track != null && track.Quantity <= 0)
                RemoveTrack(account, tag);
            SaveCopyState();
            return CopyResult(id, true, "done", null, instrument, 0, 0, null);
        }

        // La pendiente de la principal se ejecutó.
        private async Task<Dictionary<string, object>> CopyPendingFillAsync(
            Account account, Dictionary<string, object> order, string id, Instrument instrument)
        {
            string tag = GetString(order, "tag");
            string pendingTag = GetString(order, "pending_tag");
            CopyTrack pendingTrack = GetTrack(account, pendingTag);
            List<Order> working = FindOrders(account, pendingTag + ":P", true);
            if (working.Count > 0)
            {
                CancelOrders(account, working);
                RemoveTrack(account, pendingTag);
                return await CopyEntryAsync(account, order, id, instrument, tag).ConfigureAwait(false);
            }

            int filled = 0;
            double price = 0;
            string sideName = GetString(order, "side");
            foreach (Order item in FindOrders(account, pendingTag + ":P", false))
            {
                if (item.Filled > 0)
                {
                    filled += item.Filled;
                    price = item.AverageFillPrice;
                    sideName = IsBuyAction(item.OrderAction) ? "buy" : "sell";
                }
            }
            if (filled <= 0)
            {
                RemoveTrack(account, pendingTag);
                return CopyResult(id, false, "skipped", "La orden pendiente copiada ya no existe", instrument, 0, 0, null);
            }

            CopyTrack track = EnsureTrack(account, tag, instrument, sideName == "buy" ? 1 : -1);
            lock (copyLock)
            {
                track.Quantity += filled;
            }
            double sl = pendingTrack != null ? pendingTrack.StopLoss : 0;
            double tp = pendingTrack != null ? pendingTrack.TakeProfit : 0;
            RemoveTrack(account, pendingTag);
            PlaceProtective(account, instrument, track, sl, tp);
            SaveCopyState();
            return CopyResult(id, true, "filled", null, instrument, filled, price, null);
        }

        private static double LastPrice(Instrument instrument)
        {
            try
            {
                if (instrument.MarketData != null && instrument.MarketData.Last != null)
                    return instrument.MarketData.Last.Price;
            }
            catch { }
            return 0;
        }

        private static double AveragePrice(Account account, Instrument instrument, int side)
        {
            double result = 0;
            Action action = () =>
            {
                foreach (Position position in account.Positions)
                {
                    if (position.Instrument != instrument)
                        continue;
                    if ((side > 0 && position.MarketPosition == MarketPosition.Long)
                        || (side < 0 && position.MarketPosition == MarketPosition.Short))
                        result = position.AveragePrice;
                }
            };
            RunOnMainThread(action);
            return result;
        }

        // --- Estado persistente -------------------------------------------------

        private void LoadCopyState()
        {
            try
            {
                if (!File.Exists(CopyStatePath))
                    return;
                Dictionary<string, object> data = NewSerializer().Deserialize<Dictionary<string, object>>(File.ReadAllText(CopyStatePath));
                lock (copyLock)
                {
                    copyTracks.Clear();
                    foreach (Dictionary<string, object> item in GetList(data, "tracks"))
                    {
                        CopyTrack track = new CopyTrack
                        {
                            AccountId = GetLong(item, "account", 0),
                            Tag = GetString(item, "tag"),
                            Instrument = GetString(item, "instrument"),
                            Side = GetInt(item, "side", 1),
                            Quantity = GetInt(item, "quantity", 0),
                            StopLoss = GetDouble(item, "sl"),
                            TakeProfit = GetDouble(item, "tp")
                        };
                        if (track.AccountId > 0 && !string.IsNullOrEmpty(track.Tag))
                            copyTracks[track.AccountId.ToString(CultureInfo.InvariantCulture) + "|" + track.Tag] = track;
                    }
                }
            }
            catch (Exception ex)
            {
                Log("No se pudo leer el estado de copy trading: " + ex.Message);
            }
        }

        private void SaveCopyState()
        {
            try
            {
                List<object> tracks;
                lock (copyLock)
                {
                    tracks = copyTracks.Values.Select(t => (object)new Dictionary<string, object>
                    {
                        { "account", t.AccountId },
                        { "tag", t.Tag },
                        { "instrument", t.Instrument },
                        { "side", t.Side },
                        { "quantity", t.Quantity },
                        { "sl", t.StopLoss },
                        { "tp", t.TakeProfit }
                    }).ToList();
                }
                string text = NewSerializer().Serialize(new Dictionary<string, object> { { "tracks", tracks } });
                File.WriteAllText(CopyStatePath, text);
            }
            catch (Exception ex)
            {
                LogThrottled("No se pudo guardar el estado de copy trading: " + ex.Message);
            }
        }

        private static IEnumerable<object> GetArray(Dictionary<string, object> data, string key)
        {
            object value;
            if (data == null || !data.TryGetValue(key, out value) || value == null)
                return new object[0];
            object[] array = value as object[];
            if (array != null)
                return array;
            ArrayList list = value as ArrayList;
            if (list != null)
                return list.ToArray();
            return new object[0];
        }

        private static List<Dictionary<string, object>> GetList(Dictionary<string, object> data, string key)
        {
            return GetArray(data, key).OfType<Dictionary<string, object>>().ToList();
        }

        private static double GetDouble(Dictionary<string, object> data, string key)
        {
            object value;
            if (data == null || !data.TryGetValue(key, out value) || value == null)
                return 0;
            try
            {
                return Convert.ToDouble(value, CultureInfo.InvariantCulture);
            }
            catch
            {
                return 0;
            }
        }

        private class CopyTrack
        {
            public long AccountId;
            public string Tag;
            public string Instrument;
            public int Side;
            public int Quantity;
            public double StopLoss;
            public double TakeProfit;
        }

        private class PosSnap
        {
            public Instrument Instrument;
            public int Side;
            public int Quantity;
            public double AveragePrice;
            public double StopLoss;
            public double TakeProfit;
        }

        private class OrdSnap
        {
            public Order Order;
            public Instrument Instrument;
            public string Side;
            public string Type;
            public int Quantity;
            public double Price;
            public double StopLimit;
        }

        private class MasterSnapshot
        {
            public readonly Dictionary<string, PosSnap> Positions = new Dictionary<string, PosSnap>();
            public readonly Dictionary<string, OrdSnap> Orders = new Dictionary<string, OrdSnap>();
        }

        private class MasterState
        {
            public MasterSnapshot Snapshot;
        }

        private class ReferenceComparer<T> : IEqualityComparer<T> where T : class
        {
            public bool Equals(T x, T y)
            {
                return ReferenceEquals(x, y);
            }

            public int GetHashCode(T obj)
            {
                return System.Runtime.CompilerServices.RuntimeHelpers.GetHashCode(obj);
            }
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
            public bool CopyTrading = true;

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
                result.CopyTrading = GetBool(data, "copyTrading", true);
                return result;
            }
        }
    }
}
