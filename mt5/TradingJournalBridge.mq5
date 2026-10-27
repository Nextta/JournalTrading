//+------------------------------------------------------------------+
//|                                         TradingJournalBridge.mq5 |
//|                    Puente entre MetaTrader 5 y Plan de Trading   |
//+------------------------------------------------------------------+
#property copyright "Plan de Trading"
#property version   "1.10"
#property description "Envía heartbeat, operaciones, deals y velas a la app local."

input string InpApiUrl           = "http://127.0.0.1:4477"; // URL base de la app
input string InpApiKey           = "";                       // API key (Ajustes > Seguridad)
input int    InpHeartbeatSeconds = 5;                        // Heartbeat (segundos)
input int    InpSyncSeconds      = 15;                       // Sincronización (segundos)
input int    InpHistoryDays      = 90;                       // Días de historial inicial
input bool   InpVerbose          = true;                     // Logs detallados

#define EA_VERSION "1.1.0"
#define MAX_SERIES_BARS 20000

string   g_apiBase = "";
datetime g_lastHeartbeat = 0;
datetime g_lastHeartbeatAttempt = 0;
datetime g_lastSync = 0;
datetime g_lastSyncAttempt = 0;
bool     g_initialSyncDone = false;
bool     g_forceHeartbeat = true;
bool     g_forceSync = true;
int      g_brokerOffset = 0;

//+------------------------------------------------------------------+
//| Utilidades de texto                                              |
//+------------------------------------------------------------------+
string Esc(string value)
{
   StringReplace(value, "\\", "\\\\");
   StringReplace(value, "\"", "\\\"");
   StringReplace(value, "\r", "\\r");
   StringReplace(value, "\n", "\\n");
   StringReplace(value, "\t", "\\t");
   return value;
}

string Num(double value)
{
   return DoubleToString(value, 8);
}

string OptNum(double value)
{
   return value == 0.0 ? "null" : Num(value);
}

string ServerTime(datetime value)
{
   return TimeToString(value, TIME_DATE | TIME_SECONDS);
}

string IsoTime(datetime value)
{
   string text = TimeToString(value, TIME_DATE | TIME_SECONDS);
   StringReplace(text, ".", "-");
   StringReplace(text, " ", "T");
   return text;
}

int BrokerOffsetSeconds()
{
   long offset = (long)TimeCurrent() - (long)TimeGMT();
   return (int)offset;
}

//+------------------------------------------------------------------+
//| HTTP                                                             |
//+------------------------------------------------------------------+
bool HttpPost(const string path, const string body, string &response, int &status)
{
   char data[];
   char result[];
   string resultHeaders = "";
   int length = StringToCharArray(body, data, 0, WHOLE_ARRAY, CP_UTF8);
   if(length > 0 && data[length - 1] == 0)
      ArrayResize(data, length - 1);

   string headers = "Content-Type: application/json\r\n";
   headers += "X-Api-Key: " + InpApiKey + "\r\n";

   ResetLastError();
   status = WebRequest("POST", g_apiBase + path, headers, 5000, data, result, resultHeaders);
   if(status == -1)
   {
      int error = GetLastError();
      if(error == 4014)
         Print("WebRequest bloqueado. Añade ", g_apiBase,
               " en Herramientas > Opciones > Asesores expertos > Permitir WebRequest.");
      else
         PrintFormat("WebRequest falló (%d) al llamar %s", error, path);
      return false;
   }

   response = CharArrayToString(result, 0, WHOLE_ARRAY, CP_UTF8);
   if(status < 200 || status >= 300)
   {
      PrintFormat("HTTP %d en %s: %s", status, path, response);
      return false;
   }
   return true;
}

//+------------------------------------------------------------------+
//| JSON mínimo                                                      |
//+------------------------------------------------------------------+
bool IsJsonSpace(ushort character)
{
   return character == ' ' || character == '\r' || character == '\n' || character == '\t';
}

int ValueStart(const string source, const string key)
{
   string needle = "\"" + key + "\"";
   int keyPos = StringFind(source, needle);
   if(keyPos < 0)
      return -1;
   int colon = StringFind(source, ":", keyPos + StringLen(needle));
   if(colon < 0)
      return -1;
   int pos = colon + 1;
   int length = StringLen(source);
   while(pos < length && IsJsonSpace(StringGetCharacter(source, pos)))
      pos++;
   return pos < length ? pos : -1;
}

int FindMatching(const string source, const int start)
{
   int length = StringLen(source);
   if(start < 0 || start >= length)
      return -1;
   ushort open = StringGetCharacter(source, start);
   ushort close = open == '{' ? '}' : ']';
   if(open != '{' && open != '[')
      return -1;

   int depth = 0;
   bool inString = false;
   bool escaped = false;
   for(int i = start; i < length; i++)
   {
      ushort character = StringGetCharacter(source, i);
      if(inString)
      {
         if(escaped)
            escaped = false;
         else if(character == '\\')
            escaped = true;
         else if(character == '"')
            inString = false;
         continue;
      }
      if(character == '"')
      {
         inString = true;
         continue;
      }
      if(character == open)
         depth++;
      else if(character == close)
      {
         depth--;
         if(depth == 0)
            return i;
      }
   }
   return -1;
}

string JsonBlock(const string key, const string source)
{
   int pos = ValueStart(source, key);
   if(pos < 0)
      return "";
   ushort character = StringGetCharacter(source, pos);
   if(character != '{' && character != '[')
      return "";
   int end = FindMatching(source, pos);
   if(end < 0)
      return "";
   return StringSubstr(source, pos, end - pos + 1);
}

string JsonString(const string key, const string source)
{
   int pos = ValueStart(source, key);
   if(pos < 0 || StringGetCharacter(source, pos) != '"')
      return "";
   pos++;
   string result = "";
   bool escaped = false;
   int length = StringLen(source);
   for(int i = pos; i < length; i++)
   {
      ushort character = StringGetCharacter(source, i);
      if(escaped)
      {
         if(character == 'n')
            result += "\n";
         else if(character == 'r')
            result += "\r";
         else if(character == 't')
            result += "\t";
         else
            result += ShortToString(character);
         escaped = false;
         continue;
      }
      if(character == '\\')
      {
         escaped = true;
         continue;
      }
      if(character == '"')
         break;
      result += ShortToString(character);
   }
   return result;
}

long JsonLong(const string key, const string source)
{
   int pos = ValueStart(source, key);
   if(pos < 0)
      return 0;
   int length = StringLen(source);
   int end = pos;
   while(end < length)
   {
      ushort character = StringGetCharacter(source, end);
      if(character == ',' || character == '}' || character == ']' || IsJsonSpace(character))
         break;
      end++;
   }
   return StringToInteger(StringSubstr(source, pos, end - pos));
}

double JsonDouble(const string key, const string source)
{
   int pos = ValueStart(source, key);
   if(pos < 0)
      return 0.0;
   int length = StringLen(source);
   int end = pos;
   while(end < length)
   {
      ushort character = StringGetCharacter(source, end);
      if(character == ',' || character == '}' || character == ']' || IsJsonSpace(character))
         break;
      end++;
   }
   return StringToDouble(StringSubstr(source, pos, end - pos));
}

bool JsonBool(const string key, const string source)
{
   int pos = ValueStart(source, key);
   if(pos < 0)
      return false;
   return StringFind(source, "true", pos) == pos;
}

//+------------------------------------------------------------------+
//| Símbolos                                                         |
//+------------------------------------------------------------------+
void AddSymbol(const string symbol, string &symbols[])
{
   if(symbol == "")
      return;
   int count = ArraySize(symbols);
   for(int i = 0; i < count; i++)
   {
      if(StringCompare(symbols[i], symbol, false) == 0)
         return;
   }
   ArrayResize(symbols, count + 1);
   symbols[count] = symbol;
}

string BuildSymbols(string &symbols[])
{
   string json = "";
   int count = ArraySize(symbols);
   for(int i = 0; i < count; i++)
   {
      string symbol = symbols[i];
      if(!SymbolSelect(symbol, true))
         continue;
      if(json != "")
         json += ",";
      json += "{";
      json += "\"symbol\":\"" + Esc(symbol) + "\",";
      json += "\"description\":\"" + Esc(SymbolInfoString(symbol, SYMBOL_DESCRIPTION)) + "\",";
      json += "\"digits\":" + IntegerToString(SymbolInfoInteger(symbol, SYMBOL_DIGITS)) + ",";
      json += "\"tick_size\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_SIZE)) + ",";
      json += "\"tick_value\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_VALUE)) + ",";
      json += "\"contract_size\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_CONTRACT_SIZE)) + ",";
      json += "\"currency_profit\":\"" + Esc(SymbolInfoString(symbol, SYMBOL_CURRENCY_PROFIT)) + "\"";
      json += "}";
   }
   return json;
}

//+------------------------------------------------------------------+
//| Payloads                                                         |
//+------------------------------------------------------------------+
string BuildHeartbeat()
{
   long login = AccountInfoInteger(ACCOUNT_LOGIN);
   string server = AccountInfoString(ACCOUNT_SERVER);
   g_brokerOffset = BrokerOffsetSeconds();

   string body = "{";
   body += "\"login\":" + IntegerToString(login) + ",";
   body += "\"server\":\"" + Esc(server) + "\",";
   body += "\"display_name\":\"" + Esc(AccountInfoString(ACCOUNT_NAME)) + "\",";
   body += "\"company\":\"" + Esc(AccountInfoString(ACCOUNT_COMPANY)) + "\",";
   body += "\"currency\":\"" + Esc(AccountInfoString(ACCOUNT_CURRENCY)) + "\",";
   body += "\"leverage\":" + IntegerToString(AccountInfoInteger(ACCOUNT_LEVERAGE)) + ",";
   body += "\"broker_utc_offset_seconds\":" + IntegerToString(g_brokerOffset) + ",";
   body += "\"balance\":" + Num(AccountInfoDouble(ACCOUNT_BALANCE)) + ",";
   body += "\"equity\":" + Num(AccountInfoDouble(ACCOUNT_EQUITY)) + ",";
   body += "\"margin\":" + Num(AccountInfoDouble(ACCOUNT_MARGIN)) + ",";
   body += "\"free_margin\":" + Num(AccountInfoDouble(ACCOUNT_MARGIN_FREE)) + ",";
   body += "\"ea_version\":\"" + EA_VERSION + "\",";
   body += "\"terminal_build\":" + IntegerToString(TerminalInfoInteger(TERMINAL_BUILD)) + ",";
   body += "\"terminal_data_path\":\"" + Esc(TerminalInfoString(TERMINAL_DATA_PATH)) + "\",";
   body += "\"server_time\":\"" + IsoTime(TimeCurrent()) + "\"";
   body += "}";
   return body;
}

string BuildDealJson(const ulong ticket)
{
   long position = HistoryDealGetInteger(ticket, DEAL_POSITION_ID);
   if(position <= 0)
      return "";
   string symbol = HistoryDealGetString(ticket, DEAL_SYMBOL);
   if(symbol == "")
      return "";

   datetime time = (datetime)HistoryDealGetInteger(ticket, DEAL_TIME);
   string json = "{";
   json += "\"deal_ticket\":" + IntegerToString((long)ticket) + ",";
   json += "\"position_ticket\":" + IntegerToString(position) + ",";
   json += "\"symbol\":\"" + Esc(symbol) + "\",";
   json += "\"type\":" + IntegerToString(HistoryDealGetInteger(ticket, DEAL_TYPE)) + ",";
   json += "\"entry\":" + IntegerToString(HistoryDealGetInteger(ticket, DEAL_ENTRY)) + ",";
   json += "\"volume\":" + Num(HistoryDealGetDouble(ticket, DEAL_VOLUME)) + ",";
   json += "\"price\":" + Num(HistoryDealGetDouble(ticket, DEAL_PRICE)) + ",";
   json += "\"profit\":" + Num(HistoryDealGetDouble(ticket, DEAL_PROFIT)) + ",";
   json += "\"commission\":" + Num(HistoryDealGetDouble(ticket, DEAL_COMMISSION)) + ",";
   json += "\"swap\":" + Num(HistoryDealGetDouble(ticket, DEAL_SWAP)) + ",";
   json += "\"magic\":" + IntegerToString(HistoryDealGetInteger(ticket, DEAL_MAGIC)) + ",";
   json += "\"comment\":\"" + Esc(HistoryDealGetString(ticket, DEAL_COMMENT)) + "\",";
   json += "\"reason\":" + IntegerToString(HistoryDealGetInteger(ticket, DEAL_REASON)) + ",";
   json += "\"time_server\":\"" + ServerTime(time) + "\",";
   json += "\"time\":" + IntegerToString((long)time);
   json += "}";
   return json;
}

string BuildPositionJson(const ulong ticket)
{
   string symbol = PositionGetString(POSITION_SYMBOL);
   if(symbol == "")
      return "";
   datetime time = (datetime)PositionGetInteger(POSITION_TIME);

   string json = "{";
   json += "\"ticket\":" + IntegerToString((long)ticket) + ",";
   json += "\"symbol\":\"" + Esc(symbol) + "\",";
   json += "\"type\":" + IntegerToString(PositionGetInteger(POSITION_TYPE)) + ",";
   json += "\"volume\":" + Num(PositionGetDouble(POSITION_VOLUME)) + ",";
   json += "\"price_open\":" + Num(PositionGetDouble(POSITION_PRICE_OPEN)) + ",";
   json += "\"sl\":" + OptNum(PositionGetDouble(POSITION_SL)) + ",";
   json += "\"tp\":" + OptNum(PositionGetDouble(POSITION_TP)) + ",";
   json += "\"profit\":" + Num(PositionGetDouble(POSITION_PROFIT)) + ",";
   json += "\"swap\":" + Num(PositionGetDouble(POSITION_SWAP)) + ",";
   json += "\"commission\":0.0,";
   json += "\"magic\":" + IntegerToString(PositionGetInteger(POSITION_MAGIC)) + ",";
   json += "\"comment\":\"" + Esc(PositionGetString(POSITION_COMMENT)) + "\",";
   json += "\"time_server\":\"" + ServerTime(time) + "\",";
   json += "\"time\":" + IntegerToString((long)time) + ",";
   json += "\"tick_size\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_SIZE)) + ",";
   json += "\"tick_value\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_VALUE));
   json += "}";
   return json;
}

//+------------------------------------------------------------------+
//| Sincronización                                                   |
//+------------------------------------------------------------------+
bool SyncData(const bool forceFull, const int requestedDays)
{
   if(InpApiKey == "")
      return false;

   datetime now = TimeCurrent();
   datetime from = 0;
   datetime to = now + 86400;

   if(forceFull || !g_initialSyncDone)
   {
      int days = requestedDays > 0 ? requestedDays : InpHistoryDays;
      if(days < 1)
         days = 1;
      from = now - (datetime)(days * 86400);
   }
   else
   {
      from = g_lastSync - 86400;
   }
   if(from < 0)
      from = 0;
   if(from >= to)
      from = to - 86400;

   HistorySelect(from, to);

   string symbols[];
   string deals = "";
   int dealCount = HistoryDealsTotal();
   for(int i = 0; i < dealCount; i++)
   {
      ulong ticket = HistoryDealGetTicket(i);
      if(ticket == 0)
         continue;
      string symbol = HistoryDealGetString(ticket, DEAL_SYMBOL);
      if(symbol == "")
         continue;
      AddSymbol(symbol, symbols);
      string json = BuildDealJson(ticket);
      if(json == "")
         continue;
      if(deals != "")
         deals += ",";
      deals += json;
   }

   string positions = "";
   int positionCount = PositionsTotal();
   for(int i = positionCount - 1; i >= 0; i--)
   {
      ulong ticket = PositionGetTicket(i);
      if(ticket == 0)
         continue;
      string symbol = PositionGetString(POSITION_SYMBOL);
      if(symbol == "")
         continue;
      AddSymbol(symbol, symbols);
      string json = BuildPositionJson(ticket);
      if(json == "")
         continue;
      if(positions != "")
         positions += ",";
      positions += json;
   }

   string body = "{";
   body += "\"login\":" + IntegerToString(AccountInfoInteger(ACCOUNT_LOGIN)) + ",";
   body += "\"server\":\"" + Esc(AccountInfoString(ACCOUNT_SERVER)) + "\",";
   body += "\"broker_utc_offset_seconds\":" + IntegerToString(BrokerOffsetSeconds()) + ",";
   body += "\"symbols\":[" + BuildSymbols(symbols) + "],";
   body += "\"positions\":[" + positions + "],";
   body += "\"deals\":[" + deals + "],";
   body += "\"server_time\":\"" + IsoTime(now) + "\"";
   body += "}";

   string response = "";
   int status = 0;
   if(!HttpPost("/api/v1/mt5/sync", body, response, status))
      return false;

   g_lastSync = now;
   g_initialSyncDone = true;
   if(InpVerbose)
      PrintFormat("Sync OK: %d deals, %d posiciones abiertas", dealCount, positionCount);
   return true;
}

//+------------------------------------------------------------------+
//| Series                                                           |
//+------------------------------------------------------------------+
ENUM_TIMEFRAMES TimeframeFromString(const string timeframe)
{
   if(timeframe == "M1")  return PERIOD_M1;
   if(timeframe == "M5")  return PERIOD_M5;
   if(timeframe == "M15") return PERIOD_M15;
   if(timeframe == "M30") return PERIOD_M30;
   if(timeframe == "H1")  return PERIOD_H1;
   if(timeframe == "H4")  return PERIOD_H4;
   if(timeframe == "D1")  return PERIOD_D1;
   if(timeframe == "W1")  return PERIOD_W1;
   return PERIOD_CURRENT;
}

bool SendSeries(const string payload, string &error)
{
   long tradeTicket = JsonLong("trade_ticket", payload);
   string symbol = JsonString("symbol", payload);
   string timeframe = JsonString("timeframe", payload);
   long fromValue = JsonLong("from_time", payload);
   long toValue = JsonLong("to_time", payload);

   ENUM_TIMEFRAMES period = TimeframeFromString(timeframe);
   if(period == PERIOD_CURRENT)
   {
      error = "Timeframe no válido";
      return false;
   }
   if(symbol == "" || tradeTicket <= 0)
   {
      error = "Comando de serie incompleto";
      return false;
   }
   if(!SymbolSelect(symbol, true))
   {
      error = "Símbolo no disponible en Market Watch";
      return false;
   }

   int seconds = PeriodSeconds(period);
   datetime now = TimeCurrent();
   datetime from = fromValue > 0 ? (datetime)fromValue : now - 86400;
   datetime to = toValue > 0 ? (datetime)toValue : now;
   if(to <= from)
      to = from + seconds * 30;
   from -= seconds * 20;
   to += seconds * 10;
   if(from < 0)
      from = 0;

   MqlRates rates[];
   int copied = CopyRates(symbol, period, from, to, rates);
   if(copied > MAX_SERIES_BARS)
   {
      from = to - (datetime)(seconds * MAX_SERIES_BARS);
      copied = CopyRates(symbol, period, from, to, rates);
   }
   if(copied <= 0)
   {
      error = StringFormat("Sin velas para %s %s (%d)", symbol, timeframe, GetLastError());
      return false;
   }

   string bars = "";
   for(int i = 0; i < copied; i++)
   {
      if(bars != "")
         bars += ",";
      bars += "{";
      bars += "\"time\":" + IntegerToString((long)rates[i].time) + ",";
      bars += "\"open\":" + Num(rates[i].open) + ",";
      bars += "\"high\":" + Num(rates[i].high) + ",";
      bars += "\"low\":" + Num(rates[i].low) + ",";
      bars += "\"close\":" + Num(rates[i].close) + ",";
      bars += "\"volume\":" + Num((double)rates[i].tick_volume) + ",";
      bars += "\"spread\":" + IntegerToString(rates[i].spread);
      bars += "}";
   }

   string body = "{";
   body += "\"login\":" + IntegerToString(AccountInfoInteger(ACCOUNT_LOGIN)) + ",";
   body += "\"server\":\"" + Esc(AccountInfoString(ACCOUNT_SERVER)) + "\",";
   body += "\"trade_ticket\":" + IntegerToString(tradeTicket) + ",";
   body += "\"symbol\":\"" + Esc(symbol) + "\",";
   body += "\"timeframe\":\"" + Esc(timeframe) + "\",";
   body += "\"from_time\":" + IntegerToString((long)from) + ",";
   body += "\"to_time\":" + IntegerToString((long)to) + ",";
   body += "\"bars\":[" + bars + "]";
   body += "}";

   string response = "";
   int status = 0;
   if(!HttpPost("/api/v1/mt5/series", body, response, status))
   {
      error = "Error al enviar las velas";
      return false;
   }
   if(InpVerbose)
      PrintFormat("Serie enviada: %s %s, %d velas", symbol, timeframe, copied);
   return true;
}

//+------------------------------------------------------------------+
//| Comandos                                                         |
//+------------------------------------------------------------------+
void SendCommandResult(const string commandId, const bool ok, const string error)
{
   if(commandId == "")
      return;
   string body = "{";
   body += "\"command_id\":\"" + Esc(commandId) + "\",";
   body += "\"ok\":" + (ok ? "true" : "false");
   if(!ok)
      body += ",\"error\":\"" + Esc(error) + "\"";
   body += "}";
   string response = "";
   int status = 0;
   HttpPost("/api/v1/mt5/commands/result", body, response, status);
}

void ProcessCommand(const string command)
{
   string id = JsonString("id", command);
   string kind = JsonString("kind", command);
   string payload = JsonBlock("payload", command);

   if(kind == "sync")
   {
      int days = (int)JsonLong("from_days", payload);
      bool ok = SyncData(true, days);
      SendCommandResult(id, ok, ok ? "" : "No se pudo sincronizar");
   }
   else if(kind == "series")
   {
      string error = "";
      bool ok = SendSeries(payload, error);
      SendCommandResult(id, ok, error);
   }
   else
   {
      SendCommandResult(id, false, "Comando desconocido: " + kind);
   }
}

void ProcessCommands(const string response)
{
   int key = StringFind(response, "\"commands\"");
   if(key < 0)
      return;
   int arrayStart = StringFind(response, "[", key);
   if(arrayStart < 0)
      return;

   int position = arrayStart + 1;
   int length = StringLen(response);
   while(position < length)
   {
      int objectStart = StringFind(response, "{", position);
      if(objectStart < 0)
         break;
      int objectEnd = FindMatching(response, objectStart);
      if(objectEnd < 0)
         break;
      ProcessCommand(StringSubstr(response, objectStart, objectEnd - objectStart + 1));
      position = objectEnd + 1;

      int nextObject = StringFind(response, "{", position);
      int arrayEnd = StringFind(response, "]", position);
      if(arrayEnd >= 0 && (nextObject < 0 || arrayEnd < nextObject))
         break;
   }
}

//+------------------------------------------------------------------+
//| Heartbeat                                                        |
//+------------------------------------------------------------------+
void Heartbeat()
{
   if(InpApiKey == "")
   {
      Comment("Plan de Trading: configura InpApiKey para conectar.");
      return;
   }

   string response = "";
   int status = 0;
   if(!HttpPost("/api/v1/mt5/heartbeat", BuildHeartbeat(), response, status))
   {
      Comment("Plan de Trading: sin conexión con la app local.");
      return;
   }

   g_lastHeartbeat = TimeCurrent();
   long offset = JsonLong("brokerUtcOffsetSeconds", response);
   if(offset != 0)
      g_brokerOffset = (int)offset;

   if(JsonBool("pendingConfirmation", response))
      Comment("Plan de Trading: cuenta pendiente de confirmar en la app.");
   else
      Comment("Plan de Trading: conectado · ", AccountInfoString(ACCOUNT_SERVER));

   ProcessCommands(response);
}

//+------------------------------------------------------------------+
//| Eventos                                                          |
//+------------------------------------------------------------------+
int OnInit()
{
   g_apiBase = InpApiUrl;
   while(StringLen(g_apiBase) > 0 &&
         StringGetCharacter(g_apiBase, StringLen(g_apiBase) - 1) == '/')
      g_apiBase = StringSubstr(g_apiBase, 0, StringLen(g_apiBase) - 1);

   if(InpApiKey == "")
      Print("Plan de Trading: InpApiKey está vacía. Configura la API key en los ajustes de la app.");

   EventSetTimer(1);
   g_forceHeartbeat = true;
   g_forceSync = true;
   PrintFormat("Plan de Trading Bridge %s iniciado. Endpoint: %s", EA_VERSION, g_apiBase);
   return INIT_SUCCEEDED;
}

void OnDeinit(const int reason)
{
   EventKillTimer();
   Comment("");
}

void OnTimer()
{
   datetime now = TimeLocal();
   int heartbeatSeconds = InpHeartbeatSeconds > 0 ? InpHeartbeatSeconds : 5;
   int syncSeconds = InpSyncSeconds > 0 ? InpSyncSeconds : 15;

   if(g_forceHeartbeat || now - g_lastHeartbeatAttempt >= heartbeatSeconds)
   {
      g_forceHeartbeat = false;
      g_lastHeartbeatAttempt = now;
      Heartbeat();
   }

   if(g_forceSync || now - g_lastSyncAttempt >= syncSeconds)
   {
      g_forceSync = false;
      g_lastSyncAttempt = now;
      SyncData(false, 0);
   }
}

void OnTradeTransaction(const MqlTradeTransaction &trans,
                        const MqlTradeRequest &request,
                        const MqlTradeResult &result)
{
   g_forceHeartbeat = true;
   g_forceSync = true;
}
//+------------------------------------------------------------------+
