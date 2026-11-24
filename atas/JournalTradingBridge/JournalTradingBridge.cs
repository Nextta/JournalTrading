#region Using declarations
using System.ComponentModel;
using System.ComponentModel.DataAnnotations;
using System.Globalization;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using ATAS.DataFeedsCore;
using ATAS.Indicators;
using Utils.Common.Logging;
#endregion

namespace ATAS.Indicators.Custom
{
    /// <summary>
    /// Puente entre ATAS X y JournalTrading.
    ///
    /// JournalTrading ya lee el historial completo de la base de datos local de ATAS. Este
    /// indicador es opcional y añade:
    ///   - Tiempo real: cada pocos segundos envía la cuenta del gráfico (balance, PnL abierto,
    ///     posición) y las ejecuciones de la sesión, sin esperar a que ATAS guarde su base de datos.
    ///   - Velas: atiende las peticiones de velas de la app (M1...W1) para cualquier instrumento
    ///     usando el proveedor de velas de ATAS.
    ///
    ///   - Copy trading: como cuenta principal envía sus operaciones a la app y como secundaria
    ///     ejecuta las copias (ver JournalTradingBridge.Copy.cs). Solo envía órdenes cuando la app
    ///     asigna a la cuenta el rol de secundaria. Solo se conecta a 127.0.0.1.
    /// </summary>
    [DisplayName("JournalTrading Bridge")]
    [Category("JournalTrading")]
    public partial class JournalTradingBridge : Indicator
    {
        private const string Version = "1.1.0";
        private const string ConfigFileName = "JournalTradingBridge.json";

        private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(60) };
        private static readonly JsonSerializerOptions JsonOptions = new() { WriteIndented = false };

        private readonly HashSet<string> runningCommands = new();
        private int busy;
        private bool timerSubscribed;
        private string lastLog = "";
        private DateTime lastLogAt = DateTime.MinValue;
        private long login;
        private string server = "ATAS X";

        #region Ajustes

        [Display(Name = "URL de JournalTrading", GroupName = "Conexión", Order = 10)]
        public string ApiUrl { get; set; } = "http://127.0.0.1:4477";

        [Display(Name = "API key (vacío = usar JournalTradingBridge.json)", GroupName = "Conexión", Order = 20)]
        public string ApiKey { get; set; } = "";

        [Display(Name = "Intervalo (segundos)", GroupName = "Conexión", Order = 30)]
        [Range(2, 300)]
        public int IntervalSeconds { get; set; } = 5;

        [Display(Name = "Logs detallados", GroupName = "Conexión", Order = 40)]
        public bool Verbose { get; set; } = true;

        #endregion

        public JournalTradingBridge()
            : base(true)
        {
            DenyToChangePanel = true;
            DataSeries[0].IsHidden = true;
        }

        protected override void OnInitialize()
        {
            if (!timerSubscribed)
            {
                SubscribeToTimer(TimeSpan.FromSeconds(Math.Max(2, IntervalSeconds)), OnTimer);
                timerSubscribed = true;
            }
            StartCopyTrading();
            Log("JournalTrading Bridge " + Version + " iniciado.");
        }

        protected override void OnCalculate(int bar, decimal value)
        {
        }

        protected override void OnDispose()
        {
            StopCopyTrading();
            if (timerSubscribed)
            {
                UnsubscribeFromTimer(TimeSpan.FromSeconds(Math.Max(2, IntervalSeconds)), OnTimer);
                timerSubscribed = false;
            }
            base.OnDispose();
        }

        // -------------------------------------------------------------------
        // Heartbeat
        // -------------------------------------------------------------------

        private void OnTimer()
        {
            if (Interlocked.CompareExchange(ref busy, 1, 0) != 0)
                return;

            JsonObject? payload;
            try
            {
                payload = BuildHeartbeat();
            }
            catch (Exception ex)
            {
                Interlocked.Exchange(ref busy, 0);
                Log("No se pudo leer la cuenta: " + ex.Message);
                return;
            }

            if (payload == null)
            {
                Interlocked.Exchange(ref busy, 0);
                return;
            }

            _ = Task.Run(async () =>
            {
                try
                {
                    JsonObject? response = await PostAsync("/api/v1/atas/heartbeat", payload);
                    if (response != null)
                        ProcessResponse(response);
                }
                catch (Exception ex)
                {
                    Log("Heartbeat: " + ex.Message);
                }
                finally
                {
                    Interlocked.Exchange(ref busy, 0);
                }
            });
        }

        private JsonObject? BuildHeartbeat()
        {
            Portfolio? portfolio = TradingManager?.Portfolio;
            if (portfolio == null || string.IsNullOrWhiteSpace(portfolio.AccountID))
                return null;

            JsonArray trades = new();
            IEnumerable<MyTrade>? myTrades = TradingManager?.MyTrades;
            if (myTrades != null)
            {
                foreach (MyTrade trade in myTrades.ToList())
                {
                    if (trade == null || trade.AccountID != portfolio.AccountID || string.IsNullOrEmpty(trade.Id))
                        continue;
                    trades.Add(new JsonObject
                    {
                        ["id"] = trade.Id,
                        ["security_id"] = trade.SecurityId,
                        ["direction"] = trade.OrderDirection == OrderDirections.Buy ? "Buy" : "Sell",
                        ["price"] = (double)trade.Price,
                        ["volume"] = (double)trade.Volume,
                        ["time_utc"] = IsoUtc(trade.Time),
                        ["commission"] = trade.Commission.HasValue ? (double)trade.Commission.Value : 0.0
                    });
                }
            }

            JsonObject result = new()
            {
                ["account_id"] = portfolio.AccountID,
                ["currency"] = portfolio.Currency?.ToString(),
                ["balance"] = (double)portfolio.Balance,
                ["open_pnl"] = (double)portfolio.OpenPnL,
                ["indicator_version"] = Version,
                ["my_trades"] = trades
            };

            Position? position = TradingManager?.Position;
            if (position != null && !string.IsNullOrEmpty(position.SecurityId))
            {
                result["position"] = new JsonObject
                {
                    ["security_id"] = position.SecurityId,
                    ["volume"] = (double)position.Volume,
                    ["average_price"] = (double)position.AveragePrice,
                    ["unrealized_pnl"] = (double)position.UnrealizedPnL
                };
            }
            return result;
        }

        private void ProcessResponse(JsonObject response)
        {
            if (response["skipped"]?.GetValue<bool>() == true)
            {
                LogThrottled(response["message"]?.GetValue<string>() ?? "Cuenta omitida.");
                return;
            }

            if (response["login"] is JsonValue loginValue && loginValue.TryGetValue(out long parsedLogin))
                login = parsedLogin;
            if (response["server"] is JsonValue serverValue && serverValue.TryGetValue(out string? parsedServer)
                && !string.IsNullOrWhiteSpace(parsedServer))
                server = parsedServer;

            if (response["commands"] is not JsonArray commands)
                return;

            foreach (JsonNode? node in commands)
            {
                if (node is not JsonObject command)
                    continue;
                string id = command["id"]?.GetValue<string>() ?? "";
                string kind = command["kind"]?.GetValue<string>() ?? "";
                if (string.IsNullOrEmpty(id))
                    continue;

                lock (runningCommands)
                {
                    if (!runningCommands.Add(id))
                        continue;
                }

                JsonObject commandPayload = command["payload"] as JsonObject ?? new JsonObject();
                _ = Task.Run(async () =>
                {
                    try
                    {
                        if (kind == "series")
                            await HandleSeriesAsync(id, commandPayload);
                        else if (kind == "sync")
                            await SendResultAsync(id, true, null);
                        else
                            await SendResultAsync(id, false, "Comando no soportado: " + kind);
                    }
                    catch (Exception ex)
                    {
                        await SendResultAsync(id, false, ex.Message);
                        Log("Comando " + kind + ": " + ex.Message);
                    }
                    finally
                    {
                        lock (runningCommands)
                            runningCommands.Remove(id);
                    }
                });
            }
        }

        // -------------------------------------------------------------------
        // Velas
        // -------------------------------------------------------------------

        private async Task HandleSeriesAsync(string commandId, JsonObject payload)
        {
            string symbol = payload["symbol"]?.GetValue<string>() ?? "";
            string timeframe = (payload["timeframe"]?.GetValue<string>() ?? "").ToUpperInvariant();
            long tradeTicket = GetLong(payload, "trade_ticket");
            long fromTime = GetLong(payload, "from_time");
            long toTime = GetLong(payload, "to_time");

            if (string.IsNullOrWhiteSpace(symbol))
            {
                await SendResultAsync(commandId, false, "Símbolo vacío.");
                return;
            }
            if (!TryTimeFrame(timeframe, out CandleSeriesTimeFrame frame, out TimeSpan span))
            {
                await SendResultAsync(commandId, false, "Timeframe no válido: " + timeframe);
                return;
            }
            if (CandlesDataProvider == null || !CandlesDataProvider.IsAvailable)
            {
                await SendResultAsync(commandId, false, "El proveedor de velas de ATAS no está disponible.");
                return;
            }

            // from_time/to_time son horas del bróker (hora local) expresadas como epoch.
            DateTime fromUtc = fromTime > 0 ? LocalEpochToUtc(fromTime) : DateTime.UtcNow.AddDays(-5);
            DateTime toUtc = toTime > 0 ? LocalEpochToUtc(toTime) : DateTime.UtcNow;
            if (toUtc < fromUtc)
                toUtc = fromUtc;
            TimeSpan margin = TimeSpan.FromTicks(Math.Max(span.Ticks * 150, TimeSpan.FromHours(6).Ticks));
            DateTime windowFrom = fromUtc - margin;
            DateTime windowTo = toUtc + margin;
            int days = (int)Math.Ceiling((DateTime.UtcNow - windowFrom).TotalDays) + 1;
            days = Math.Clamp(days, 1, 3650);

            CandleSeriesRequest request = new()
            {
                TimeFrame = frame,
                Instrument = BuildInstrument(symbol),
                DaysToLoad = days
            };

            using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(120));
            var candles = await CandlesDataProvider.GetCandlesAsync(request, timeout.Token);

            JsonArray bars = new();
            if (candles != null)
            {
                foreach (IndicatorCandle candle in candles)
                {
                    if (candle == null || candle.Time < windowFrom || candle.Time > windowTo)
                        continue;
                    bars.Add(new JsonObject
                    {
                        ["time"] = UtcToLocalEpoch(candle.Time),
                        ["open"] = (double)candle.Open,
                        ["high"] = (double)candle.High,
                        ["low"] = (double)candle.Low,
                        ["close"] = (double)candle.Close,
                        ["volume"] = (double)candle.Volume,
                        ["spread"] = 0
                    });
                }
            }

            if (bars.Count == 0)
            {
                await SendResultAsync(commandId, false, "ATAS no devolvió velas para " + symbol + " " + timeframe + ".");
                return;
            }

            JsonObject seriesPayload = new()
            {
                ["login"] = login,
                ["server"] = server,
                ["trade_ticket"] = tradeTicket,
                ["symbol"] = symbol,
                ["timeframe"] = timeframe,
                ["from_time"] = fromTime,
                ["to_time"] = toTime,
                ["bars"] = bars
            };

            JsonObject? response = await PostAsync("/api/v1/atas/series", seriesPayload);
            await SendResultAsync(commandId, response != null, response == null ? "Error al enviar las velas." : null);
            Log("Velas " + timeframe + " " + symbol + ": " + bars.Count + " barras.");
        }

        private static CandleSeriesInstrument BuildInstrument(string securityId)
        {
            string code = securityId;
            string exchange = "";
            int at = securityId.IndexOf('@');
            if (at > 0)
            {
                code = securityId.Substring(0, at);
                exchange = securityId.Substring(at + 1);
            }
            return new CandleSeriesInstrument
            {
                SecurityId = securityId,
                Code = code,
                Exchange = exchange
            };
        }

        private static bool TryTimeFrame(string timeframe, out CandleSeriesTimeFrame frame, out TimeSpan span)
        {
            switch (timeframe)
            {
                case "M1": frame = CandleSeriesTimeFrame.Minutes(1); span = TimeSpan.FromMinutes(1); return true;
                case "M5": frame = CandleSeriesTimeFrame.Minutes(5); span = TimeSpan.FromMinutes(5); return true;
                case "M15": frame = CandleSeriesTimeFrame.Minutes(15); span = TimeSpan.FromMinutes(15); return true;
                case "M30": frame = CandleSeriesTimeFrame.Minutes(30); span = TimeSpan.FromMinutes(30); return true;
                case "H1": frame = CandleSeriesTimeFrame.Hours(1); span = TimeSpan.FromHours(1); return true;
                case "H4": frame = CandleSeriesTimeFrame.Hours(4); span = TimeSpan.FromHours(4); return true;
                case "D1": frame = CandleSeriesTimeFrame.Daily(); span = TimeSpan.FromDays(1); return true;
                case "W1": frame = CandleSeriesTimeFrame.Weekly(); span = TimeSpan.FromDays(7); return true;
                default: frame = default; span = TimeSpan.Zero; return false;
            }
        }

        // -------------------------------------------------------------------
        // HTTP y configuración
        // -------------------------------------------------------------------

        private (string url, string key) ResolveConnection()
        {
            string url = string.IsNullOrWhiteSpace(ApiUrl) ? "http://127.0.0.1:4477" : ApiUrl.Trim();
            string key = (ApiKey ?? "").Trim();
            if (key.Length > 0)
                return (url, key);

            try
            {
                string appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
                foreach (string folder in new[] { "ATAS X", "ATAS" })
                {
                    string path = Path.Combine(appData, folder, ConfigFileName);
                    if (!File.Exists(path))
                        continue;
                    JsonObject? config = JsonNode.Parse(File.ReadAllText(path)) as JsonObject;
                    string? fileKey = config?["apiKey"]?.GetValue<string>();
                    string? fileUrl = config?["apiUrl"]?.GetValue<string>();
                    if (!string.IsNullOrWhiteSpace(fileUrl) && ApiUrl == "http://127.0.0.1:4477")
                        url = fileUrl.Trim();
                    if (!string.IsNullOrWhiteSpace(fileKey))
                        return (url, fileKey.Trim());
                }
            }
            catch (Exception ex)
            {
                LogThrottled("No se pudo leer " + ConfigFileName + ": " + ex.Message);
            }
            return (url, "");
        }

        private async Task<JsonObject?> PostAsync(string path, JsonObject payload)
        {
            (string url, string key) = ResolveConnection();
            if (key.Length == 0)
            {
                LogThrottled("Falta la API key: escríbela en los ajustes del indicador o genera la configuración desde JournalTrading.");
                return null;
            }

            using HttpRequestMessage request = new(HttpMethod.Post, url.TrimEnd('/') + path);
            request.Headers.TryAddWithoutValidation("X-Api-Key", key);
            request.Content = new StringContent(payload.ToJsonString(JsonOptions), Encoding.UTF8, "application/json");
            using HttpResponseMessage response = await Http.SendAsync(request).ConfigureAwait(false);
            string text = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
            if (!response.IsSuccessStatusCode)
            {
                LogThrottled("HTTP " + (int)response.StatusCode + " en " + path + ": " + text);
                return null;
            }
            try
            {
                return JsonNode.Parse(text) as JsonObject ?? new JsonObject();
            }
            catch
            {
                return new JsonObject();
            }
        }

        private async Task SendResultAsync(string commandId, bool ok, string? error)
        {
            JsonObject payload = new()
            {
                ["command_id"] = commandId,
                ["ok"] = ok,
                ["error"] = error
            };
            try
            {
                await PostAsync("/api/v1/atas/commands/result", payload);
            }
            catch (Exception ex)
            {
                Log("Resultado de comando: " + ex.Message);
            }
        }

        // -------------------------------------------------------------------
        // Utilidades
        // -------------------------------------------------------------------

        private static long GetLong(JsonObject data, string key)
        {
            try
            {
                JsonNode? node = data[key];
                return node == null ? 0 : node.GetValue<long>();
            }
            catch
            {
                return 0;
            }
        }

        /// <summary>ATAS trabaja en UTC; si la hora llega sin zona se asume UTC.</summary>
        private static string IsoUtc(DateTime value)
        {
            DateTime utc = value.Kind == DateTimeKind.Local ? value.ToUniversalTime() : DateTime.SpecifyKind(value, DateTimeKind.Utc);
            return utc.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture);
        }

        private static DateTime LocalEpochToUtc(long seconds)
        {
            DateTime local = DateTime.SpecifyKind(DateTime.UnixEpoch.AddSeconds(seconds), DateTimeKind.Unspecified);
            return TimeZoneInfo.ConvertTimeToUtc(local, TimeZoneInfo.Local);
        }

        private static long UtcToLocalEpoch(DateTime value)
        {
            DateTime utc = value.Kind == DateTimeKind.Local ? value.ToUniversalTime() : DateTime.SpecifyKind(value, DateTimeKind.Utc);
            DateTime local = TimeZoneInfo.ConvertTimeFromUtc(utc, TimeZoneInfo.Local);
            return (long)(local - DateTime.UnixEpoch).TotalSeconds;
        }

        private void Log(string message)
        {
            if (!Verbose)
                return;
            try
            {
                this.LogInfo("JournalTrading: {0}", message);
            }
            catch
            {
            }
        }

        private void LogThrottled(string message)
        {
            DateTime now = DateTime.UtcNow;
            if (message == lastLog && (now - lastLogAt).TotalSeconds < 60)
                return;
            lastLog = message;
            lastLogAt = now;
            Log(message);
        }
    }
}
