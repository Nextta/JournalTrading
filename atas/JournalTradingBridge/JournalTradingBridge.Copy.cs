#region Using declarations
using System.Collections.Concurrent;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using ATAS.DataFeedsCore;
#endregion

namespace ATAS.Indicators.Custom
{
    /// <summary>
    /// Copy trading del indicador.
    ///
    /// Principal: cada 100 ms compara la posición y las órdenes del gráfico (cuenta + instrumento)
    /// y envía los cambios a /api/v1/copy/signal.
    /// Secundaria: recoge sus órdenes con /api/v1/copy/poll (long-polling), las ejecuta con el
    /// TradingManager del gráfico y devuelve el resultado a /api/v1/copy/result.
    ///
    /// ATAS solo permite operar el instrumento y la cuenta del gráfico, así que en cada cuenta
    /// secundaria hace falta un gráfico con el indicador por cada instrumento que se quiera copiar.
    /// Lo copiado se marca con el comentario de la orden ("JT…:E", ":SL", ":TP", ":P") y se lleva la
    /// cuenta del volumen copiado por etiqueta: las operaciones manuales no se tocan.
    /// </summary>
    public partial class JournalTradingBridge
    {
        private const string CopyStateFileName = "JournalTradingCopyState.json";
        private static readonly object CopyLock = new();
        private static readonly Dictionary<string, CopyTrack> CopyTracks = LoadCopyState();
        private static readonly ConcurrentDictionary<string, Order> CopyOrders = new();

        private readonly Dictionary<Order, string> orderRefs = new(ReferenceEqualityComparer.Instance);
        private CancellationTokenSource? copyCts;
        private Timer? copyTimer;
        private int copyMasterBusy;
        private string copyRole = "none";
        private bool copyEnabled;
        private MasterSnapshot? masterSnapshot;
        private long orderRefSeq;

        [System.ComponentModel.DataAnnotations.Display(Name = "Copy trading (el rol se asigna en la app)", GroupName = "Copy trading", Order = 10)]
        public bool CopyTrading { get; set; } = true;

        private void StartCopyTrading()
        {
            StopCopyTrading();
            copyCts = new CancellationTokenSource();
            CancellationToken token = copyCts.Token;
            _ = Task.Run(() => CopyLoopAsync(token));
            copyTimer = new Timer(_ => OnCopyTimer(), null, 1000, 100);
        }

        private void StopCopyTrading()
        {
            copyTimer?.Dispose();
            copyTimer = null;
            try { copyCts?.Cancel(); } catch { }
            copyCts = null;
        }

        private string? CopyAccountId => TradingManager?.Portfolio?.AccountID;

        private string? CopySecurityId => TradingManager?.Security?.SecurityId;

        // Las etiquetas se repiten entre secundarias del mismo grupo: la clave incluye la cuenta.
        private string OrderKey(string? comment) => (CopyAccountId ?? "") + "|" + (comment ?? "");

        // --- Bucle de consulta ------------------------------------------------

        private async Task CopyLoopAsync(CancellationToken token)
        {
            while (!token.IsCancellationRequested)
            {
                try
                {
                    string? account = CopyAccountId;
                    string? security = CopySecurityId;
                    if (!CopyTrading || string.IsNullOrEmpty(account) || string.IsNullOrEmpty(security))
                    {
                        await Task.Delay(2000, token).ConfigureAwait(false);
                        continue;
                    }

                    bool follower = copyRole == "follower";
                    Portfolio? portfolio = TradingManager?.Portfolio;
                    JsonObject item = new()
                    {
                        ["account"] = account,
                        ["server"] = server,
                        ["balance"] = portfolio == null ? null : (double)portfolio.Balance,
                        ["equity"] = portfolio == null ? null : (double)(portfolio.Balance + portfolio.OpenPnL),
                        ["tradable"] = new JsonArray(security),
                    };
                    if (follower)
                        item["copies"] = BuildCopies(account, security);

                    JsonObject payload = new()
                    {
                        ["provider"] = "atas",
                        ["bridge_version"] = Version,
                        ["wait_ms"] = follower ? 1000 : 0,
                        ["accounts"] = new JsonArray(item),
                    };
                    JsonObject? response = await PostAsync("/api/v1/copy/poll", payload).ConfigureAwait(false);
                    if (response == null)
                    {
                        await Task.Delay(2000, token).ConfigureAwait(false);
                        continue;
                    }

                    JsonArray results = new();
                    if (response["accounts"] is JsonArray accounts && accounts.Count > 0 && accounts[0] is JsonObject entry)
                    {
                        string role = entry["role"]?.GetValue<string>() ?? "none";
                        bool enabled = entry["enabled"]?.GetValue<bool>() ?? false;
                        if (role != copyRole || enabled != copyEnabled)
                        {
                            if (role == "master" && enabled)
                                masterSnapshot = null;
                            copyRole = role;
                            copyEnabled = enabled;
                            Log("Copy trading · " + account + " · " + security + " · rol " + role + (enabled ? " · activo" : " · pausado"));
                        }
                        if (role == "follower" && entry["orders"] is JsonArray orders)
                        {
                            foreach (JsonNode? node in orders)
                            {
                                if (node is JsonObject order)
                                    results.Add(await ExecuteCopyOrderAsync(order).ConfigureAwait(false));
                            }
                        }
                    }

                    if (results.Count > 0)
                    {
                        await PostAsync("/api/v1/copy/result", new JsonObject { ["results"] = results }).ConfigureAwait(false);
                        SaveCopyState();
                    }

                    if (!follower)
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

        // --- Principal ----------------------------------------------------------

        private void OnCopyTimer()
        {
            if (!CopyTrading || copyRole != "master" || !copyEnabled)
                return;
            if (Interlocked.CompareExchange(ref copyMasterBusy, 1, 0) != 0)
                return;
            _ = Task.Run(async () =>
            {
                try
                {
                    JsonArray signals = DiffMaster();
                    string? account = CopyAccountId;
                    if (signals.Count == 0 || string.IsNullOrEmpty(account))
                        return;
                    JsonObject payload = new()
                    {
                        ["provider"] = "atas",
                        ["account"] = account,
                        ["server"] = server,
                        ["signals"] = signals,
                    };
                    await PostAsync("/api/v1/copy/signal", payload).ConfigureAwait(false);
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

        private static string MarketOf(Security? security)
        {
            if (security == null)
                return "futures";
            return security.Type switch
            {
                SecType.Future or SecType.CryptoFutures => "futures",
                SecType.Forex or SecType.Cfd => "cfd_forex",
                _ => "other",
            };
        }

        private static bool IsWorking(Order order) => order.State == OrderStates.Active && !order.Canceled;

        private string RefOf(Order order)
        {
            lock (orderRefs)
            {
                if (!orderRefs.TryGetValue(order, out string? value))
                {
                    value = order.ExtId != 0
                        ? "AT" + order.ExtId.ToString(CultureInfo.InvariantCulture)
                        : "AT" + Interlocked.Increment(ref orderRefSeq).ToString(CultureInfo.InvariantCulture) + "-" + DateTime.UtcNow.Ticks.ToString("x", CultureInfo.InvariantCulture);
                    orderRefs[order] = value;
                }
                return value;
            }
        }

        private MasterSnapshot TakeMasterSnapshot()
        {
            MasterSnapshot snapshot = new();
            ITradingManager? manager = TradingManager;
            Security? security = manager?.Security;
            if (manager == null || security == null)
                return snapshot;

            Position? position = manager.Position;
            if (position != null && position.Volume != 0)
            {
                snapshot.Position = new PosSnap
                {
                    Side = position.Volume > 0 ? 1 : -1,
                    Quantity = Math.Abs(position.Volume),
                    AveragePrice = position.AveragePrice,
                };
            }

            foreach (Order order in manager.Orders ?? Enumerable.Empty<Order>())
            {
                if (order == null || !IsWorking(order) || order.Type == OrderTypes.Market)
                    continue;
                if (order.SecurityId != security.SecurityId)
                    continue;
                string? accountId = manager.Portfolio?.AccountID;
                if (!string.IsNullOrEmpty(accountId) && !string.IsNullOrEmpty(order.AccountID) && order.AccountID != accountId)
                    continue;
                bool buy = order.Direction == OrderDirections.Buy;
                decimal remaining = order.Unfilled;
                PosSnap? pos = snapshot.Position;
                if (pos != null && buy != (pos.Side > 0) && remaining <= pos.Quantity)
                {
                    if (order.Type is OrderTypes.Stop or OrderTypes.StopLimit)
                    {
                        decimal stop = order.TriggerPrice;
                        if (pos.StopLoss == 0 || (pos.Side > 0 ? stop > pos.StopLoss : stop < pos.StopLoss))
                            pos.StopLoss = stop;
                    }
                    else if (order.Type == OrderTypes.Limit)
                    {
                        if (pos.TakeProfit == 0 || (pos.Side > 0 ? order.Price < pos.TakeProfit : order.Price > pos.TakeProfit))
                            pos.TakeProfit = order.Price;
                    }
                    continue;
                }

                (string type, decimal price, decimal stopLimit) = order.Type switch
                {
                    OrderTypes.Limit => ("limit", order.Price, 0m),
                    OrderTypes.Stop => ("stop", order.TriggerPrice, 0m),
                    OrderTypes.StopLimit => ("stop_limit", order.TriggerPrice, order.Price),
                    _ => ("", 0m, 0m),
                };
                if (type.Length == 0)
                    continue;
                snapshot.Orders[RefOf(order)] = new OrdSnap
                {
                    Order = order,
                    Side = buy ? "buy" : "sell",
                    Type = type,
                    Quantity = remaining,
                    Price = price,
                    StopLimit = stopLimit,
                };
            }
            return snapshot;
        }

        private JsonArray DiffMaster()
        {
            JsonArray signals = new();
            string? securityId = CopySecurityId;
            if (string.IsNullOrEmpty(securityId))
                return signals;
            Security? security = TradingManager?.Security;
            MasterSnapshot current = TakeMasterSnapshot();
            MasterSnapshot? previous = masterSnapshot;
            masterSnapshot = current;
            if (previous == null)
                return signals; // Lo que ya estaba abierto al activar la copia no se copia.

            string? filledRef = null;
            foreach ((string key, OrdSnap order) in previous.Orders)
            {
                if (current.Orders.ContainsKey(key))
                    continue;
                bool filled = !order.Order.Canceled && order.Order.State == OrderStates.Done && order.Order.Unfilled < order.Quantity;
                if (filled)
                    filledRef = key;
                else
                    signals.Add(OrderSignal("pending_cancel", key, order, securityId, security));
            }

            PosSnap? now = current.Position;
            PosSnap? before = previous.Position;
            if (now != null && before == null)
            {
                signals.Add(PositionSignal("open", securityId, security, now, now.Quantity, now.Quantity, now.AveragePrice, filledRef));
            }
            else if (now == null && before != null)
            {
                signals.Add(PositionSignal("close", securityId, security, before, before.Quantity, 0, before.AveragePrice, null));
            }
            else if (now != null && before != null)
            {
                if (now.Side != before.Side)
                {
                    signals.Add(PositionSignal("close", securityId, security, before, before.Quantity, 0, now.AveragePrice, null));
                    signals.Add(PositionSignal("open", securityId, security, now, now.Quantity, now.Quantity, now.AveragePrice, filledRef));
                }
                else
                {
                    if (now.Quantity > before.Quantity)
                    {
                        decimal added = now.Quantity - before.Quantity;
                        decimal fill = (now.AveragePrice * now.Quantity - before.AveragePrice * before.Quantity) / added;
                        signals.Add(PositionSignal("increase", securityId, security, now, added, now.Quantity, fill, filledRef));
                    }
                    else if (now.Quantity < before.Quantity)
                    {
                        signals.Add(PositionSignal("reduce", securityId, security, now, before.Quantity - now.Quantity, now.Quantity, now.AveragePrice, null));
                    }
                    if (now.StopLoss != before.StopLoss || now.TakeProfit != before.TakeProfit)
                        signals.Add(PositionSignal("sltp", securityId, security, now, 0, now.Quantity, now.AveragePrice, null));
                }
            }

            foreach ((string key, OrdSnap order) in current.Orders)
            {
                if (!previous.Orders.TryGetValue(key, out OrdSnap? old))
                {
                    signals.Add(OrderSignal("pending_place", key, order, securityId, security));
                    continue;
                }
                if (old.Price != order.Price || old.StopLimit != order.StopLimit || old.Quantity != order.Quantity)
                    signals.Add(OrderSignal("pending_modify", key, order, securityId, security));
            }
            return signals;
        }

        private static JsonObject PositionSignal(
            string kind, string securityId, Security? security, PosSnap position,
            decimal volume, decimal remaining, decimal price, string? orderRef)
        {
            JsonObject signal = new()
            {
                ["kind"] = kind,
                ["ref"] = securityId,
                ["symbol"] = securityId,
                ["market"] = MarketOf(security),
                ["side"] = position.Side > 0 ? "buy" : "sell",
                ["volume"] = (double)volume,
                ["position_volume"] = (double)remaining,
                ["price"] = (double)price,
                ["entry_price"] = (double)position.AveragePrice,
                ["sl"] = position.StopLoss > 0 ? (double)position.StopLoss : null,
                ["tp"] = position.TakeProfit > 0 ? (double)position.TakeProfit : null,
            };
            if (!string.IsNullOrEmpty(orderRef))
                signal["order_ref"] = orderRef;
            return signal;
        }

        private static JsonObject OrderSignal(string kind, string orderRef, OrdSnap order, string securityId, Security? security)
        {
            return new JsonObject
            {
                ["kind"] = kind,
                ["ref"] = orderRef,
                ["order_ref"] = orderRef,
                ["symbol"] = securityId,
                ["market"] = MarketOf(security),
                ["side"] = order.Side,
                ["order_type"] = order.Type,
                ["volume"] = (double)order.Quantity,
                ["price"] = (double)order.Price,
                ["order_price"] = (double)order.Price,
                ["stop_limit_price"] = order.StopLimit > 0 ? (double)order.StopLimit : null,
            };
        }

        // --- Secundaria -----------------------------------------------------------

        private static string TrackKey(string account, string tag) => account + "|" + tag;

        private static CopyTrack? GetTrack(string account, string tag)
        {
            lock (CopyLock)
                return CopyTracks.TryGetValue(TrackKey(account, tag), out CopyTrack? track) ? track : null;
        }

        private static CopyTrack EnsureTrack(string account, string tag, string security, int side)
        {
            lock (CopyLock)
            {
                string key = TrackKey(account, tag);
                if (!CopyTracks.TryGetValue(key, out CopyTrack? track))
                {
                    track = new CopyTrack { Account = account, Tag = tag };
                    CopyTracks[key] = track;
                }
                track.Security = security;
                track.Side = side;
                return track;
            }
        }

        private static void RemoveTrack(string account, string tag)
        {
            lock (CopyLock)
                CopyTracks.Remove(TrackKey(account, tag));
        }

        private decimal NetQuantity(int side)
        {
            Position? position = TradingManager?.Position;
            if (position == null || position.Volume == 0)
                return 0;
            if ((position.Volume > 0) != (side > 0))
                return 0;
            return Math.Abs(position.Volume);
        }

        private JsonArray BuildCopies(string account, string security)
        {
            List<CopyTrack> tracks;
            lock (CopyLock)
                tracks = CopyTracks.Values.Where(t => t.Account == account && t.Security == security && t.Quantity > 0).ToList();
            JsonArray copies = new();
            bool changed = false;
            foreach (CopyTrack track in tracks)
            {
                // Si la posición se cerró (SL/TP, cierre manual), lo copiado no puede superar la posición real.
                decimal quantity = Math.Min(track.Quantity, NetQuantity(track.Side));
                if (quantity != track.Quantity)
                {
                    lock (CopyLock)
                        track.Quantity = quantity;
                    changed = true;
                }
                copies.Add(new JsonObject { ["tag"] = track.Tag, ["volume"] = (double)quantity });
            }
            if (changed)
                SaveCopyState();
            return copies;
        }

        private static JsonObject CopyResult(string id, bool ok, string status, string? error, string? symbol,
            decimal volume = 0, decimal price = 0, string? orderRef = null)
        {
            JsonObject result = new() { ["order_id"] = id, ["ok"] = ok, ["status"] = status };
            if (!string.IsNullOrEmpty(error))
                result["error"] = error;
            if (!string.IsNullOrEmpty(symbol))
                result["symbol"] = symbol;
            if (volume > 0)
                result["volume"] = (double)volume;
            if (price > 0)
                result["price"] = (double)price;
            if (!string.IsNullOrEmpty(orderRef))
                result["order_ref"] = orderRef;
            return result;
        }

        private static string Str(JsonObject data, string key)
        {
            try { return data[key]?.GetValue<string>() ?? ""; } catch { return ""; }
        }

        private static decimal Dec(JsonObject data, string key)
        {
            try
            {
                JsonNode? node = data[key];
                return node == null ? 0m : (decimal)node.GetValue<double>();
            }
            catch
            {
                return 0m;
            }
        }

        private static bool Bool(JsonObject data, string key)
        {
            try { return data[key]?.GetValue<bool>() ?? false; } catch { return false; }
        }

        private async Task<JsonObject> ExecuteCopyOrderAsync(JsonObject order)
        {
            string id = Str(order, "id");
            string action = Str(order, "action");
            try
            {
                ITradingManager? manager = TradingManager;
                string? account = CopyAccountId;
                Security? security = manager?.Security;
                if (manager == null || security == null || string.IsNullOrEmpty(account))
                    return CopyResult(id, false, "rejected", "El indicador no tiene cuenta o instrumento", null);

                bool matches = order["symbols"] is JsonArray symbols
                    && symbols.Any(node => string.Equals(node?.GetValue<string>(), security.SecurityId, StringComparison.OrdinalIgnoreCase));
                if (!matches)
                    return CopyResult(id, false, "rejected", "Este gráfico opera " + security.SecurityId + ", no el instrumento de la orden", null);

                return action switch
                {
                    "market" when Bool(order, "reduce_only") => await CopyReduceAsync(order, id, account, security, false).ConfigureAwait(false),
                    "market" => await CopyEntryAsync(order, id, account, security, Str(order, "tag")).ConfigureAwait(false),
                    "close" => await CopyReduceAsync(order, id, account, security, true).ConfigureAwait(false),
                    "sltp" => await CopySlTpAsync(order, id, account, security).ConfigureAwait(false),
                    "pending_place" => await CopyPendingPlaceAsync(order, id, account, security).ConfigureAwait(false),
                    "pending_modify" => await CopyPendingModifyAsync(order, id, security).ConfigureAwait(false),
                    "pending_cancel" => await CopyPendingCancelAsync(order, id, account, security).ConfigureAwait(false),
                    "pending_fill" => await CopyPendingFillAsync(order, id, account, security).ConfigureAwait(false),
                    _ => CopyResult(id, false, "rejected", "Acción desconocida: " + action, security.SecurityId),
                };
            }
            catch (Exception ex)
            {
                return CopyResult(id, false, "rejected", ex.Message, null);
            }
        }

        private static decimal RoundPrice(Security security, decimal price)
        {
            if (price <= 0 || security.TickSize <= 0)
                return Math.Max(price, 0);
            return Math.Round(price / security.TickSize) * security.TickSize;
        }

        private static void ResolveSlTp(JsonObject order, Security security, decimal basePrice, out decimal sl, out decimal tp)
        {
            if (Str(order, "price_mode") == "offset")
            {
                decimal slOffset = Dec(order, "sl_offset");
                decimal tpOffset = Dec(order, "tp_offset");
                sl = slOffset != 0 ? basePrice + slOffset : 0;
                tp = tpOffset != 0 ? basePrice + tpOffset : 0;
            }
            else
            {
                sl = Dec(order, "sl");
                tp = Dec(order, "tp");
            }
            sl = RoundPrice(security, sl);
            tp = RoundPrice(security, tp);
        }

        private Order NewOrder(Security security, OrderDirections direction, OrderTypes type, decimal quantity,
            decimal price, decimal trigger, string comment, string? oco = null)
        {
            Order order = new()
            {
                Portfolio = TradingManager!.Portfolio,
                Security = security,
                Direction = direction,
                Type = type,
                QuantityToFill = quantity,
                Comment = comment,
            };
            if (price > 0)
                order.Price = price;
            if (trigger > 0)
                order.TriggerPrice = trigger;
            if (!string.IsNullOrEmpty(oco))
                order.OCOGroup = oco;
            return order;
        }

        private static async Task WaitForFinalAsync(Order order, int timeoutMs, bool activeIsFinal)
        {
            DateTime limit = DateTime.UtcNow.AddMilliseconds(timeoutMs);
            while (DateTime.UtcNow < limit)
            {
                if (order.State is OrderStates.Done or OrderStates.Failed)
                    return;
                if (activeIsFinal && order.State == OrderStates.Active)
                    return;
                await Task.Delay(25).ConfigureAwait(false);
            }
        }

        private decimal FillPrice(Order order, Security security)
        {
            try
            {
                List<MyTrade> trades = (TradingManager?.MyTrades ?? Enumerable.Empty<MyTrade>())
                    .Where(t => t.OrderId == order.Id && t.Volume > 0).ToList();
                decimal volume = trades.Sum(t => t.Volume);
                if (volume > 0)
                    return trades.Sum(t => t.Price * t.Volume) / volume;
            }
            catch { }
            return order.Direction == OrderDirections.Buy ? security.BestAskPrice : security.BestBidPrice;
        }

        private async Task CancelTagged(string comment)
        {
            ITradingManager? manager = TradingManager;
            if (manager == null)
                return;
            List<Order> targets = (manager.Orders ?? Enumerable.Empty<Order>())
                .Where(o => o != null && IsWorking(o) && o.Comment == comment).ToList();
            if (CopyOrders.TryRemove(OrderKey(comment), out Order? known) && IsWorking(known) && !targets.Contains(known))
                targets.Add(known);
            foreach (Order order in targets)
                await manager.CancelOrderAsync(order, false, false).ConfigureAwait(false);
        }

        // Coloca (o sustituye) el stop y el objetivo de lo copiado como órdenes OCO.
        private async Task PlaceProtectiveAsync(Security security, CopyTrack track, decimal sl, decimal tp)
        {
            await CancelTagged(track.Tag + ":SL").ConfigureAwait(false);
            await CancelTagged(track.Tag + ":TP").ConfigureAwait(false);
            lock (CopyLock)
            {
                track.StopLoss = sl;
                track.TakeProfit = tp;
            }
            if (track.Quantity <= 0 || (sl <= 0 && tp <= 0))
                return;
            OrderDirections exit = track.Side > 0 ? OrderDirections.Sell : OrderDirections.Buy;
            string? oco = sl > 0 && tp > 0 ? track.Tag + "-" + DateTime.UtcNow.Ticks.ToString("x", CultureInfo.InvariantCulture) : null;
            if (sl > 0)
            {
                Order stop = NewOrder(security, exit, OrderTypes.Stop, track.Quantity, 0, sl, track.Tag + ":SL", oco);
                CopyOrders[OrderKey(stop.Comment)] = stop;
                await TradingManager!.OpenOrderAsync(stop, false, false, false).ConfigureAwait(false);
            }
            if (tp > 0)
            {
                Order limit = NewOrder(security, exit, OrderTypes.Limit, track.Quantity, tp, 0, track.Tag + ":TP", oco);
                CopyOrders[OrderKey(limit.Comment)] = limit;
                await TradingManager!.OpenOrderAsync(limit, false, false, false).ConfigureAwait(false);
            }
        }

        private async Task<JsonObject> CopyEntryAsync(JsonObject order, string id, string account, Security security, string tag)
        {
            string side = Str(order, "side");
            decimal quantity = Math.Floor(Dec(order, "volume") + 0.000000001m);
            if (quantity < 1)
                return CopyResult(id, false, "skipped", "Tamaño menor que 1 contrato", security.SecurityId);

            decimal maxDeviation = Dec(order, "max_deviation");
            decimal reference = Dec(order, "reference_price");
            decimal current = side == "buy" ? security.BestAskPrice : security.BestBidPrice;
            if (maxDeviation > 0 && reference > 0 && current > 0 && security.TickSize > 0
                && Math.Abs(current - reference) / security.TickSize > maxDeviation)
                return CopyResult(id, false, "skipped",
                    "Precio desviado " + (Math.Abs(current - reference) / security.TickSize).ToString("0.#", CultureInfo.InvariantCulture)
                    + " ticks de la entrada de la principal", security.SecurityId);

            Order entry = NewOrder(security, side == "buy" ? OrderDirections.Buy : OrderDirections.Sell, OrderTypes.Market, quantity, 0, 0, tag + ":E");
            await TradingManager!.OpenOrderAsync(entry, false, false, false).ConfigureAwait(false);
            await WaitForFinalAsync(entry, 15000, false).ConfigureAwait(false);
            decimal filled = entry.QuantityToFill - entry.Unfilled;
            if (filled <= 0)
                return CopyResult(id, false, "rejected", "Orden " + entry.State, security.SecurityId);

            CopyTrack track = EnsureTrack(account, tag, security.SecurityId, side == "buy" ? 1 : -1);
            lock (CopyLock)
                track.Quantity += filled;
            decimal price = FillPrice(entry, security);
            ResolveSlTp(order, security, price, out decimal sl, out decimal tp);
            if (sl <= 0 && tp <= 0)
            {
                sl = track.StopLoss;
                tp = track.TakeProfit;
            }
            await PlaceProtectiveAsync(security, track, sl, tp).ConfigureAwait(false);
            SaveCopyState();
            return CopyResult(id, true, "filled", null, security.SecurityId, filled, price);
        }

        private async Task<JsonObject> CopyReduceAsync(JsonObject order, string id, string account, Security security, bool closeAll)
        {
            string tag = Str(order, "tag");
            CopyTrack? track = GetTrack(account, tag);
            if (track == null || track.Quantity <= 0)
            {
                RemoveTrack(account, tag);
                return CopyResult(id, true, "done", "No hay nada copiado abierto", security.SecurityId);
            }
            decimal net = NetQuantity(track.Side);
            decimal requested = closeAll ? track.Quantity : Math.Floor(Dec(order, "volume") + 0.000000001m);
            decimal quantity = Math.Min(Math.Min(requested, track.Quantity), net);
            if (quantity <= 0)
            {
                lock (CopyLock)
                    track.Quantity = Math.Min(track.Quantity, net);
                if (track.Quantity <= 0)
                {
                    await PlaceProtectiveAsync(security, track, 0, 0).ConfigureAwait(false);
                    RemoveTrack(account, tag);
                }
                return CopyResult(id, true, "done", "La posición ya estaba cerrada", security.SecurityId);
            }

            OrderDirections exit = track.Side > 0 ? OrderDirections.Sell : OrderDirections.Buy;
            Order close = NewOrder(security, exit, OrderTypes.Market, quantity, 0, 0, tag + ":X");
            await TradingManager!.OpenOrderAsync(close, false, false, false).ConfigureAwait(false);
            await WaitForFinalAsync(close, 15000, false).ConfigureAwait(false);
            decimal filled = close.QuantityToFill - close.Unfilled;
            if (filled <= 0)
                return CopyResult(id, false, "rejected", "Orden " + close.State, security.SecurityId);

            lock (CopyLock)
                track.Quantity = Math.Max(0, track.Quantity - filled);
            if (track.Quantity <= 0)
            {
                await PlaceProtectiveAsync(security, track, 0, 0).ConfigureAwait(false);
                RemoveTrack(account, tag);
            }
            else
            {
                await PlaceProtectiveAsync(security, track, track.StopLoss, track.TakeProfit).ConfigureAwait(false);
            }
            SaveCopyState();
            return CopyResult(id, true, "filled", null, security.SecurityId, filled, FillPrice(close, security));
        }

        private async Task<JsonObject> CopySlTpAsync(JsonObject order, string id, string account, Security security)
        {
            CopyTrack? track = GetTrack(account, Str(order, "tag"));
            if (track == null || track.Quantity <= 0)
                return CopyResult(id, true, "done", "No hay nada copiado abierto", security.SecurityId);
            decimal average = TradingManager?.Position?.AveragePrice ?? 0;
            ResolveSlTp(order, security, average, out decimal sl, out decimal tp);
            await PlaceProtectiveAsync(security, track, sl, tp).ConfigureAwait(false);
            SaveCopyState();
            return CopyResult(id, true, "done", null, security.SecurityId);
        }

        private async Task<JsonObject> CopyPendingPlaceAsync(JsonObject order, string id, string account, Security security)
        {
            if (Str(order, "price_mode") == "offset")
                return CopyResult(id, false, "rejected", "ATAS solo copia pendientes con precio absoluto (futuros)", security.SecurityId);
            string tag = Str(order, "tag");
            string side = Str(order, "side");
            decimal quantity = Math.Floor(Dec(order, "volume") + 0.000000001m);
            if (quantity < 1)
                return CopyResult(id, false, "skipped", "Tamaño menor que 1 contrato", security.SecurityId);

            decimal price = RoundPrice(security, Dec(order, "price"));
            decimal stopLimit = RoundPrice(security, Dec(order, "stop_limit_price"));
            (OrderTypes type, decimal limit, decimal trigger) = Str(order, "order_type") switch
            {
                "stop" => (OrderTypes.Stop, 0m, price),
                "stop_limit" => (OrderTypes.StopLimit, stopLimit > 0 ? stopLimit : price, price),
                _ => (OrderTypes.Limit, price, 0m),
            };
            Order pending = NewOrder(security, side == "buy" ? OrderDirections.Buy : OrderDirections.Sell, type, quantity, limit, trigger, tag + ":P");
            CopyOrders[OrderKey(pending.Comment)] = pending;
            await TradingManager!.OpenOrderAsync(pending, false, false, false).ConfigureAwait(false);
            await WaitForFinalAsync(pending, 5000, true).ConfigureAwait(false);
            if (pending.State == OrderStates.Failed || pending.Canceled)
                return CopyResult(id, false, "rejected", "Orden " + pending.State, security.SecurityId);

            CopyTrack track = EnsureTrack(account, tag, security.SecurityId, side == "buy" ? 1 : -1);
            ResolveSlTp(order, security, price, out decimal sl, out decimal tp);
            lock (CopyLock)
            {
                track.StopLoss = sl;
                track.TakeProfit = tp;
            }
            SaveCopyState();
            return CopyResult(id, true, "placed", null, security.SecurityId, quantity, price, pending.Id);
        }

        private Order? FindPending(string tag)
        {
            string comment = tag + ":P";
            if (CopyOrders.TryGetValue(OrderKey(comment), out Order? known) && IsWorking(known))
                return known;
            return (TradingManager?.Orders ?? Enumerable.Empty<Order>())
                .FirstOrDefault(o => o != null && IsWorking(o) && o.Comment == comment);
        }

        private async Task<JsonObject> CopyPendingModifyAsync(JsonObject order, string id, Security security)
        {
            string tag = Str(order, "tag");
            Order? pending = FindPending(tag);
            if (pending == null)
                return CopyResult(id, true, "done", "La orden pendiente ya no existe", security.SecurityId);
            decimal price = RoundPrice(security, Dec(order, "price"));
            decimal stopLimit = RoundPrice(security, Dec(order, "stop_limit_price"));
            Order changed = pending.Clone();
            switch (pending.Type)
            {
                case OrderTypes.Limit:
                    changed.Price = price;
                    break;
                case OrderTypes.Stop:
                    changed.TriggerPrice = price;
                    break;
                case OrderTypes.StopLimit:
                    changed.TriggerPrice = price;
                    changed.Price = stopLimit > 0 ? stopLimit : price;
                    break;
            }
            await TradingManager!.ModifyOrderAsync(pending, changed, false, false).ConfigureAwait(false);
            string? account = CopyAccountId;
            CopyTrack? track = account == null ? null : GetTrack(account, tag);
            if (track != null)
            {
                ResolveSlTp(order, security, price, out decimal sl, out decimal tp);
                lock (CopyLock)
                {
                    track.StopLoss = sl;
                    track.TakeProfit = tp;
                }
                SaveCopyState();
            }
            return CopyResult(id, true, "done", null, security.SecurityId, 0, price);
        }

        private async Task<JsonObject> CopyPendingCancelAsync(JsonObject order, string id, string account, Security security)
        {
            string tag = Str(order, "tag");
            Order? pending = FindPending(tag);
            if (pending != null)
                await TradingManager!.CancelOrderAsync(pending, false, false).ConfigureAwait(false);
            CopyOrders.TryRemove(OrderKey(tag + ":P"), out _);
            CopyTrack? track = GetTrack(account, tag);
            if (track != null && track.Quantity <= 0)
                RemoveTrack(account, tag);
            SaveCopyState();
            return CopyResult(id, true, "done", null, security.SecurityId);
        }

        // La pendiente de la principal se ejecutó.
        private async Task<JsonObject> CopyPendingFillAsync(JsonObject order, string id, string account, Security security)
        {
            string tag = Str(order, "tag");
            string pendingTag = Str(order, "pending_tag");
            CopyTrack? pendingTrack = GetTrack(account, pendingTag);
            Order? working = FindPending(pendingTag);
            if (working != null)
            {
                await TradingManager!.CancelOrderAsync(working, false, false).ConfigureAwait(false);
                CopyOrders.TryRemove(OrderKey(pendingTag + ":P"), out _);
                RemoveTrack(account, pendingTag);
                return await CopyEntryAsync(order, id, account, security, tag).ConfigureAwait(false);
            }

            CopyOrders.TryRemove(OrderKey(pendingTag + ":P"), out Order? known);
            decimal filled = known == null ? 0 : known.QuantityToFill - known.Unfilled;
            if (filled <= 0)
            {
                RemoveTrack(account, pendingTag);
                return CopyResult(id, false, "skipped", "La orden pendiente copiada ya no existe", security.SecurityId);
            }
            int side = known!.Direction == OrderDirections.Buy ? 1 : -1;
            CopyTrack track = EnsureTrack(account, tag, security.SecurityId, side);
            lock (CopyLock)
                track.Quantity += filled;
            RemoveTrack(account, pendingTag);
            await PlaceProtectiveAsync(security, track, pendingTrack?.StopLoss ?? 0, pendingTrack?.TakeProfit ?? 0).ConfigureAwait(false);
            SaveCopyState();
            return CopyResult(id, true, "filled", null, security.SecurityId, filled, FillPrice(known, security));
        }

        // --- Estado persistente -------------------------------------------------

        private static string CopyStatePath()
        {
            string appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
            return Path.Combine(appData, "ATAS X", CopyStateFileName);
        }

        private static Dictionary<string, CopyTrack> LoadCopyState()
        {
            Dictionary<string, CopyTrack> result = new();
            try
            {
                string path = CopyStatePath();
                if (!File.Exists(path))
                    return result;
                List<CopyTrack>? tracks = JsonSerializer.Deserialize<List<CopyTrack>>(File.ReadAllText(path));
                foreach (CopyTrack track in tracks ?? new List<CopyTrack>())
                {
                    if (!string.IsNullOrEmpty(track.Account) && !string.IsNullOrEmpty(track.Tag))
                        result[TrackKey(track.Account, track.Tag)] = track;
                }
            }
            catch
            {
            }
            return result;
        }

        private void SaveCopyState()
        {
            try
            {
                string text;
                lock (CopyLock)
                    text = JsonSerializer.Serialize(CopyTracks.Values.ToList());
                string path = CopyStatePath();
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                File.WriteAllText(path, text);
            }
            catch (Exception ex)
            {
                LogThrottled("No se pudo guardar el estado de copy trading: " + ex.Message);
            }
        }

        public sealed class CopyTrack
        {
            public string Account { get; set; } = "";
            public string Tag { get; set; } = "";
            public string Security { get; set; } = "";
            public int Side { get; set; }
            public decimal Quantity { get; set; }
            public decimal StopLoss { get; set; }
            public decimal TakeProfit { get; set; }
        }

        private sealed class PosSnap
        {
            public int Side;
            public decimal Quantity;
            public decimal AveragePrice;
            public decimal StopLoss;
            public decimal TakeProfit;
        }

        private sealed class OrdSnap
        {
            public Order Order = null!;
            public string Side = "";
            public string Type = "";
            public decimal Quantity;
            public decimal Price;
            public decimal StopLimit;
        }

        private sealed class MasterSnapshot
        {
            public PosSnap? Position;
            public readonly Dictionary<string, OrdSnap> Orders = new();
        }
    }
}
