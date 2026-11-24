//+------------------------------------------------------------------+
//|                                         TradingJournalBridge.mq5 |
//|                    Puente entre MetaTrader 5 y JournalTrading    |
//+------------------------------------------------------------------+
#property copyright "JournalTrading"
#property version   "1.20"
#property description "Envía heartbeat, operaciones, deals y velas a la app local."
#property description "Copy trading: copia operaciones como cuenta principal o secundaria."

#include <Trade\Trade.mqh>

input string InpApiUrl           = "http://127.0.0.1:4477"; // URL base de la app
input string InpApiKey           = "";                       // API key (Ajustes > Seguridad)
input int    InpHeartbeatSeconds = 5;                        // Heartbeat (segundos)
input int    InpSyncSeconds      = 15;                       // Sincronización (segundos)
input int    InpHistoryDays      = 0;                        // Días de historial inicial (0 = todo)
input bool   InpVerbose          = true;                     // Logs detallados
input bool   InpCopyTrading      = true;                     // Copy trading (el rol se asigna en la app)
input long   InpCopyMagic        = 7744001;                  // Número mágico de las operaciones copiadas
input int    InpCopyPollMs       = 150;                      // Secundaria: consulta de órdenes (ms)

#define EA_VERSION "1.2.0"
#define MAX_SERIES_BARS 20000
#define SYNC_TIMEOUT_MS 60000

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
bool HttpPost(const string path, const string body, string &response, int &status, const int timeoutMs = 5000)
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
   status = WebRequest("POST", g_apiBase + path, headers, timeoutMs, data, result, resultHeaders);
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
bool SyncData(const bool forceFull, const int requestedDays, const bool allHistory = false)
{
   if(InpApiKey == "")
      return false;

   datetime now = TimeCurrent();
   datetime from = 0;
   datetime to = now + 86400;

   if(forceFull || !g_initialSyncDone)
   {
      int days = requestedDays > 0 ? requestedDays : InpHistoryDays;
      if(allHistory || days <= 0)
         from = 0;
      else
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
   if(!HttpPost("/api/v1/mt5/sync", body, response, status, SYNC_TIMEOUT_MS))
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
      bool allHistory = JsonBool("all_history", payload);
      bool ok = SyncData(true, days, allHistory);
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
      Comment("JournalTrading: configura InpApiKey para conectar.");
      return;
   }

   string response = "";
   int status = 0;
   if(!HttpPost("/api/v1/mt5/heartbeat", BuildHeartbeat(), response, status))
   {
      Comment("JournalTrading: sin conexión con la app local.");
      return;
   }

   g_lastHeartbeat = TimeCurrent();
   long offset = JsonLong("brokerUtcOffsetSeconds", response);
   if(offset != 0)
      g_brokerOffset = (int)offset;

   if(JsonBool("pendingConfirmation", response))
      Comment("JournalTrading: cuenta pendiente de confirmar en la app.");
   else
      Comment("JournalTrading: conectado · ", AccountInfoString(ACCOUNT_SERVER));

   ProcessCommands(response);
}

//+------------------------------------------------------------------+
//| Copy trading                                                     |
//|                                                                  |
//| Rol principal: compara posiciones y órdenes pendientes cada      |
//| ciclo y envía cada cambio a /api/v1/copy/signal.                  |
//| Rol secundaria: recoge sus órdenes con /api/v1/copy/poll, las     |
//| ejecuta y devuelve el resultado a /api/v1/copy/result.            |
//| La copia solo gestiona posiciones con su etiqueta (comentario     |
//| "JT…") y su número mágico: lo manual no se toca.                  |
//+------------------------------------------------------------------+
struct CopyPosSnap
{
   string ref;
   string symbol;
   int    type;
   double volume;
   double sl;
   double tp;
   double priceOpen;
};

struct CopyOrderSnap
{
   ulong  ticket;
   string symbol;
   int    type;
   double volume;
   double price;
   double stopLimit;
   double sl;
   double tp;
};

string        g_copyRole = "none";
bool          g_copyEnabled = false;
bool          g_copyBaseline = false;
ulong         g_copyLastPoll = 0;
ulong         g_copyLastSpecs = 0;
CopyPosSnap   g_copyPositions[];
CopyOrderSnap g_copyOrders[];
CTrade        g_trade;

string CopyMarket(const string symbol)
{
   ENUM_SYMBOL_CALC_MODE mode = (ENUM_SYMBOL_CALC_MODE)SymbolInfoInteger(symbol, SYMBOL_TRADE_CALC_MODE);
   switch(mode)
   {
      case SYMBOL_CALC_MODE_FUTURES:
      case SYMBOL_CALC_MODE_EXCH_FUTURES:
      case SYMBOL_CALC_MODE_EXCH_FUTURES_FORTS:
         return "futures";
      case SYMBOL_CALC_MODE_FOREX:
      case SYMBOL_CALC_MODE_FOREX_NO_LEVERAGE:
      case SYMBOL_CALC_MODE_CFD:
      case SYMBOL_CALC_MODE_CFDINDEX:
      case SYMBOL_CALC_MODE_CFDLEVERAGE:
         return "cfd_forex";
   }
   return "other";
}

string CopySpecJson(const string symbol)
{
   string json = "{";
   json += "\"symbol\":\"" + Esc(symbol) + "\",";
   json += "\"market\":\"" + CopyMarket(symbol) + "\",";
   json += "\"digits\":" + IntegerToString(SymbolInfoInteger(symbol, SYMBOL_DIGITS)) + ",";
   json += "\"tick_size\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_SIZE)) + ",";
   json += "\"tick_value\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_VALUE)) + ",";
   json += "\"contract_size\":" + Num(SymbolInfoDouble(symbol, SYMBOL_TRADE_CONTRACT_SIZE)) + ",";
   json += "\"volume_min\":" + Num(SymbolInfoDouble(symbol, SYMBOL_VOLUME_MIN)) + ",";
   json += "\"volume_max\":" + Num(SymbolInfoDouble(symbol, SYMBOL_VOLUME_MAX)) + ",";
   json += "\"volume_step\":" + Num(SymbolInfoDouble(symbol, SYMBOL_VOLUME_STEP)) + ",";
   json += "\"bid\":" + Num(SymbolInfoDouble(symbol, SYMBOL_BID)) + ",";
   json += "\"ask\":" + Num(SymbolInfoDouble(symbol, SYMBOL_ASK));
   json += "}";
   return json;
}

string CopySpecsJson()
{
   string json = "";
   int total = SymbolsTotal(true);
   for(int i = 0; i < total; i++)
   {
      string symbol = SymbolName(i, true);
      if(symbol == "")
         continue;
      if(json != "")
         json += ",";
      json += CopySpecJson(symbol);
   }
   return json;
}

string CopyAccountJson(const bool withSpecs, const bool withCopies)
{
   string json = "\"login\":" + IntegerToString(AccountInfoInteger(ACCOUNT_LOGIN)) + ",";
   json += "\"server\":\"" + Esc(AccountInfoString(ACCOUNT_SERVER)) + "\",";
   json += "\"balance\":" + Num(AccountInfoDouble(ACCOUNT_BALANCE)) + ",";
   json += "\"equity\":" + Num(AccountInfoDouble(ACCOUNT_EQUITY));
   if(withSpecs)
      json += ",\"specs\":[" + CopySpecsJson() + "]";
   if(withCopies)
      json += ",\"copies\":[" + CopyStateJson() + "]";
   return json;
}

bool IsCopyTag(const string comment)
{
   return StringLen(comment) >= 10 && StringSubstr(comment, 0, 2) == "JT";
}

bool IsHedging()
{
   return (ENUM_ACCOUNT_MARGIN_MODE)AccountInfoInteger(ACCOUNT_MARGIN_MODE) == ACCOUNT_MARGIN_MODE_RETAIL_HEDGING;
}

// Volumen copiado agrupado por etiqueta.
string CopyStateJson()
{
   string tags[];
   double volumes[];
   int total = PositionsTotal();
   for(int i = 0; i < total; i++)
   {
      ulong ticket = PositionGetTicket(i);
      if(ticket == 0 || !PositionSelectByTicket(ticket))
         continue;
      string comment = PositionGetString(POSITION_COMMENT);
      if(!IsCopyTag(comment) || PositionGetInteger(POSITION_MAGIC) != InpCopyMagic)
         continue;
      double volume = PositionGetDouble(POSITION_VOLUME);
      int found = -1;
      for(int j = 0; j < ArraySize(tags); j++)
         if(tags[j] == comment) { found = j; break; }
      if(found < 0)
      {
         found = ArraySize(tags);
         ArrayResize(tags, found + 1);
         ArrayResize(volumes, found + 1);
         tags[found] = comment;
         volumes[found] = 0.0;
      }
      volumes[found] += volume;
   }
   string json = "";
   for(int k = 0; k < ArraySize(tags); k++)
   {
      if(json != "")
         json += ",";
      json += "{\"tag\":\"" + Esc(tags[k]) + "\",\"volume\":" + Num(volumes[k]) + "}";
   }
   return json;
}

//--- Utilidades de precio y volumen -------------------------------------
double CopyNormalizePrice(const string symbol, const double price)
{
   if(price <= 0.0)
      return 0.0;
   double tick = SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_SIZE);
   int digits = (int)SymbolInfoInteger(symbol, SYMBOL_DIGITS);
   if(tick > 0.0)
      return NormalizeDouble(MathRound(price / tick) * tick, digits);
   return NormalizeDouble(price, digits);
}

double CopyNormalizeVolume(const string symbol, const double volume)
{
   double step = SymbolInfoDouble(symbol, SYMBOL_VOLUME_STEP);
   double minVolume = SymbolInfoDouble(symbol, SYMBOL_VOLUME_MIN);
   double maxVolume = SymbolInfoDouble(symbol, SYMBOL_VOLUME_MAX);
   if(step <= 0.0)
      step = 0.01;
   double value = MathFloor(volume / step + 1e-9) * step;
   if(maxVolume > 0.0 && value > maxVolume)
      value = maxVolume;
   if(value < minVolume - 1e-12)
      return 0.0;
   int decimals = 0;
   double probe = step;
   while(decimals < 8 && MathAbs(probe - MathRound(probe)) > 1e-9)
   {
      probe *= 10.0;
      decimals++;
   }
   return NormalizeDouble(value, decimals);
}

// Tamaño de un pip (CFD/Forex) o de un tick (futuros) para la desviación máxima.
double CopyDeviationUnit(const string symbol, const string market)
{
   if(market == "futures")
      return SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_SIZE);
   double point = SymbolInfoDouble(symbol, SYMBOL_POINT);
   int digits = (int)SymbolInfoInteger(symbol, SYMBOL_DIGITS);
   return (digits == 3 || digits == 5) ? point * 10.0 : point;
}

int CopySlippagePoints(const string symbol, const string market, const double maxDeviation)
{
   double point = SymbolInfoDouble(symbol, SYMBOL_POINT);
   if(point <= 0.0 || maxDeviation <= 0.0)
      return 50;
   return (int)MathMax(5.0, MathRound(maxDeviation * CopyDeviationUnit(symbol, market) / point));
}

int JsonStringArray(const string key, const string source, string &out[])
{
   ArrayResize(out, 0);
   string block = JsonBlock(key, source);
   if(block == "")
      return 0;
   int length = StringLen(block);
   int i = 0;
   while(i < length)
   {
      if(StringGetCharacter(block, i) != '"')
      {
         i++;
         continue;
      }
      string value = "";
      i++;
      bool escaped = false;
      while(i < length)
      {
         ushort character = StringGetCharacter(block, i);
         i++;
         if(escaped)
         {
            value += ShortToString(character);
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
         value += ShortToString(character);
      }
      int count = ArraySize(out);
      ArrayResize(out, count + 1);
      out[count] = value;
   }
   return ArraySize(out);
}

bool JsonIsNull(const string key, const string source)
{
   int pos = ValueStart(source, key);
   return pos < 0 || StringFind(source, "null", pos) == pos;
}

// Primer símbolo de la lista que existe en este terminal.
string CopyResolveSymbol(const string order)
{
   string candidates[];
   int count = JsonStringArray("symbols", order, candidates);
   for(int i = 0; i < count; i++)
   {
      string symbol = candidates[i];
      if(symbol == "")
         continue;
      bool custom = false;
      if(SymbolExist(symbol, custom) && SymbolSelect(symbol, true))
         return symbol;
   }
   return "";
}

//--- Resultados ---------------------------------------------------------
string g_copyResults = "";

void CopyAddResult(const string orderId, const bool ok, const string status, const string error,
                   const string symbol, const double volume, const double price,
                   const string positionRef, const string orderRef)
{
   string json = "{";
   json += "\"order_id\":\"" + Esc(orderId) + "\",";
   json += "\"ok\":" + (ok ? "true" : "false") + ",";
   json += "\"status\":\"" + status + "\"";
   if(error != "")
      json += ",\"error\":\"" + Esc(error) + "\"";
   if(symbol != "")
      json += ",\"symbol\":\"" + Esc(symbol) + "\"";
   if(volume > 0.0)
      json += ",\"volume\":" + Num(volume);
   if(price > 0.0)
      json += ",\"price\":" + Num(price);
   if(positionRef != "")
      json += ",\"position_ref\":\"" + Esc(positionRef) + "\"";
   if(orderRef != "")
      json += ",\"order_ref\":\"" + Esc(orderRef) + "\"";
   json += "}";
   if(g_copyResults != "")
      g_copyResults += ",";
   g_copyResults += json;
}

void CopyFlushResults()
{
   if(g_copyResults == "")
      return;
   string body = "{\"provider\":\"mt5\",\"results\":[" + g_copyResults + "]}";
   g_copyResults = "";
   string response = "";
   int status = 0;
   HttpPost("/api/v1/copy/result", body, response, status);
}

string CopyTradeError()
{
   return StringFormat("%s (%d)", g_trade.ResultComment(), g_trade.ResultRetcode());
}

bool CopyTradeOk()
{
   uint code = g_trade.ResultRetcode();
   return code == TRADE_RETCODE_DONE || code == TRADE_RETCODE_PLACED || code == TRADE_RETCODE_DONE_PARTIAL;
}

string CopyPositionRefFromDeal(const ulong deal)
{
   if(deal == 0)
      return "";
   if(HistoryDealSelect(deal))
      return IntegerToString(HistoryDealGetInteger(deal, DEAL_POSITION_ID));
   HistorySelect(TimeCurrent() - 3600, TimeCurrent() + 3600);
   if(HistoryDealSelect(deal))
      return IntegerToString(HistoryDealGetInteger(deal, DEAL_POSITION_ID));
   return "";
}

void CopySlTp(const string order, const string symbol, const string side, const double basePrice,
              double &sl, double &tp)
{
   sl = 0.0;
   tp = 0.0;
   if(JsonString("price_mode", order) == "absolute")
   {
      sl = JsonDouble("sl", order);
      tp = JsonDouble("tp", order);
   }
   else
   {
      if(!JsonIsNull("sl_offset", order))
         sl = basePrice + JsonDouble("sl_offset", order);
      if(!JsonIsNull("tp_offset", order))
         tp = basePrice + JsonDouble("tp_offset", order);
   }
   sl = CopyNormalizePrice(symbol, sl);
   tp = CopyNormalizePrice(symbol, tp);
}

//--- Ejecución en la secundaria -----------------------------------------
void CopyMarketEntry(const string order, const string id, const string symbol)
{
   string side = JsonString("side", order);
   string market = JsonString("market", order);
   string tag = JsonString("tag", order);
   double volume = CopyNormalizeVolume(symbol, JsonDouble("volume", order));
   if(volume <= 0.0)
   {
      CopyAddResult(id, false, "skipped", "Volumen por debajo del mínimo del símbolo", symbol, 0, 0, "", "");
      return;
   }
   double price = side == "buy" ? SymbolInfoDouble(symbol, SYMBOL_ASK) : SymbolInfoDouble(symbol, SYMBOL_BID);
   double maxDeviation = JsonDouble("max_deviation", order);
   double reference = JsonDouble("reference_price", order);
   if(maxDeviation > 0.0 && reference > 0.0 && market == "futures")
   {
      double unit = CopyDeviationUnit(symbol, market);
      if(unit > 0.0 && MathAbs(price - reference) / unit > maxDeviation + 1e-9)
      {
         CopyAddResult(id, false, "skipped",
                       StringFormat("Precio desviado %.1f ticks de la entrada de la principal", MathAbs(price - reference) / unit),
                       symbol, 0, 0, "", "");
         return;
      }
   }
   double sl, tp;
   CopySlTp(order, symbol, side, price, sl, tp);
   g_trade.SetDeviationInPoints(CopySlippagePoints(symbol, market, maxDeviation));
   bool sent = side == "buy"
               ? g_trade.Buy(volume, symbol, 0.0, sl, tp, tag)
               : g_trade.Sell(volume, symbol, 0.0, sl, tp, tag);
   if(!sent || !CopyTradeOk())
   {
      CopyAddResult(id, false, "rejected", CopyTradeError(), symbol, 0, 0, "", "");
      return;
   }
   double filled = g_trade.ResultVolume() > 0.0 ? g_trade.ResultVolume() : volume;
   CopyAddResult(id, true, "filled", "", symbol, filled, g_trade.ResultPrice(),
                 CopyPositionRefFromDeal(g_trade.ResultDeal()), "");
}

// Cierra `volume` de lo copiado con la etiqueta (o de la posición neta en cuentas netting).
void CopyReduce(const string order, const string id, const string symbol, const bool closeAll)
{
   string tag = JsonString("tag", order);
   string positionSide = JsonString("position_side", order);
   double remaining = closeAll ? 1e12 : JsonDouble("volume", order);
   double closed = 0.0;
   double lastPrice = 0.0;
   g_trade.SetDeviationInPoints(CopySlippagePoints(symbol, JsonString("market", order), 0.0));

   if(IsHedging())
   {
      for(int i = PositionsTotal() - 1; i >= 0 && remaining > 1e-9; i--)
      {
         ulong ticket = PositionGetTicket(i);
         if(ticket == 0 || !PositionSelectByTicket(ticket))
            continue;
         if(PositionGetString(POSITION_COMMENT) != tag || PositionGetInteger(POSITION_MAGIC) != InpCopyMagic)
            continue;
         string posSymbol = PositionGetString(POSITION_SYMBOL);
         double posVolume = PositionGetDouble(POSITION_VOLUME);
         double part = MathMin(posVolume, remaining);
         bool ok = part >= posVolume - 1e-9
                   ? g_trade.PositionClose(ticket)
                   : g_trade.PositionClosePartial(ticket, CopyNormalizeVolume(posSymbol, part));
         if(ok && CopyTradeOk())
         {
            closed += part;
            remaining -= part;
            lastPrice = g_trade.ResultPrice();
         }
         else
         {
            CopyAddResult(id, closed > 0.0, closed > 0.0 ? "done" : "rejected", CopyTradeError(), symbol, closed, lastPrice, "", "");
            return;
         }
      }
   }
   else
   {
      if(!PositionSelect(symbol))
      {
         CopyAddResult(id, true, "done", "No hay posición abierta", symbol, 0, 0, "", "");
         return;
      }
      int type = (int)PositionGetInteger(POSITION_TYPE);
      string currentSide = type == POSITION_TYPE_BUY ? "buy" : "sell";
      if(positionSide != "" && positionSide != currentSide)
      {
         CopyAddResult(id, true, "done", "La posición ya no está en la misma dirección", symbol, 0, 0, "", "");
         return;
      }
      double posVolume = PositionGetDouble(POSITION_VOLUME);
      double part = CopyNormalizeVolume(symbol, MathMin(posVolume, remaining));
      if(part > 0.0)
      {
         bool ok = currentSide == "buy"
                   ? g_trade.Sell(part, symbol, 0.0, 0.0, 0.0, tag)
                   : g_trade.Buy(part, symbol, 0.0, 0.0, 0.0, tag);
         if(!ok || !CopyTradeOk())
         {
            CopyAddResult(id, false, "rejected", CopyTradeError(), symbol, 0, 0, "", "");
            return;
         }
         closed = part;
         lastPrice = g_trade.ResultPrice();
      }
   }
   CopyAddResult(id, true, closed > 0.0 ? "filled" : "done", closed > 0.0 ? "" : "Nada que cerrar",
                 symbol, closed, lastPrice, "", "");
}

void CopyModifySlTp(const string order, const string id, const string symbol)
{
   string tag = JsonString("tag", order);
   string side = JsonString("position_side", order);
   int modified = 0;
   string error = "";
   for(int i = PositionsTotal() - 1; i >= 0; i--)
   {
      ulong ticket = PositionGetTicket(i);
      if(ticket == 0 || !PositionSelectByTicket(ticket))
         continue;
      bool own = PositionGetString(POSITION_COMMENT) == tag && PositionGetInteger(POSITION_MAGIC) == InpCopyMagic;
      if(!own && (IsHedging() || PositionGetString(POSITION_SYMBOL) != symbol))
         continue;
      double sl, tp;
      CopySlTp(order, symbol, side, PositionGetDouble(POSITION_PRICE_OPEN), sl, tp);
      if(MathAbs(sl - PositionGetDouble(POSITION_SL)) < 1e-12 && MathAbs(tp - PositionGetDouble(POSITION_TP)) < 1e-12)
      {
         modified++;
         continue;
      }
      if(g_trade.PositionModify(ticket, sl, tp) && CopyTradeOk())
         modified++;
      else
         error = CopyTradeError();
   }
   if(error != "")
      CopyAddResult(id, false, "rejected", error, symbol, 0, 0, "", "");
   else
      CopyAddResult(id, true, "done", modified > 0 ? "" : "No se encontró la posición", symbol, 0, 0, "", "");
}

ENUM_ORDER_TYPE CopyPendingType(const string side, const string orderType)
{
   if(side == "buy")
   {
      if(orderType == "stop")
         return ORDER_TYPE_BUY_STOP;
      if(orderType == "stop_limit")
         return ORDER_TYPE_BUY_STOP_LIMIT;
      return ORDER_TYPE_BUY_LIMIT;
   }
   if(orderType == "stop")
      return ORDER_TYPE_SELL_STOP;
   if(orderType == "stop_limit")
      return ORDER_TYPE_SELL_STOP_LIMIT;
   return ORDER_TYPE_SELL_LIMIT;
}

void CopyPendingPrices(const string order, const string symbol, const string side,
                       double &price, double &stopLimit)
{
   if(JsonString("price_mode", order) == "absolute")
   {
      price = JsonDouble("price", order);
      stopLimit = JsonDouble("stop_limit_price", order);
   }
   else
   {
      double reference = side == "buy" ? SymbolInfoDouble(symbol, SYMBOL_ASK) : SymbolInfoDouble(symbol, SYMBOL_BID);
      price = reference + JsonDouble("price_offset", order);
      stopLimit = JsonIsNull("stop_limit_offset", order) ? 0.0 : reference + JsonDouble("stop_limit_offset", order);
   }
   price = CopyNormalizePrice(symbol, price);
   stopLimit = CopyNormalizePrice(symbol, stopLimit);
}

void CopyPendingPlace(const string order, const string id, const string symbol)
{
   string side = JsonString("side", order);
   string orderType = JsonString("order_type", order);
   string tag = JsonString("tag", order);
   double volume = CopyNormalizeVolume(symbol, JsonDouble("volume", order));
   if(volume <= 0.0)
   {
      CopyAddResult(id, false, "skipped", "Volumen por debajo del mínimo del símbolo", symbol, 0, 0, "", "");
      return;
   }
   double price, stopLimit;
   CopyPendingPrices(order, symbol, side, price, stopLimit);
   double sl, tp;
   CopySlTp(order, symbol, side, orderType == "stop_limit" && stopLimit > 0.0 ? stopLimit : price, sl, tp);
   ENUM_ORDER_TYPE type = CopyPendingType(side, orderType);
   if(!g_trade.OrderOpen(symbol, type, volume, stopLimit, price, sl, tp, ORDER_TIME_GTC, 0, tag) || !CopyTradeOk())
   {
      CopyAddResult(id, false, "rejected", CopyTradeError(), symbol, 0, 0, "", "");
      return;
   }
   CopyAddResult(id, true, "placed", "", symbol, volume, price, "", IntegerToString(g_trade.ResultOrder()));
}

ulong CopyFindOrder(const string order)
{
   ulong ticket = (ulong)StringToInteger(JsonString("follower_order_ref", order));
   if(ticket > 0 && OrderSelect(ticket))
      return ticket;
   string tag = JsonString("tag", order);
   for(int i = OrdersTotal() - 1; i >= 0; i--)
   {
      ulong candidate = OrderGetTicket(i);
      if(candidate > 0 && OrderSelect(candidate) && OrderGetString(ORDER_COMMENT) == tag)
         return candidate;
   }
   return 0;
}

void CopyPendingModify(const string order, const string id, const string symbol)
{
   ulong ticket = CopyFindOrder(order);
   if(ticket == 0)
   {
      CopyAddResult(id, true, "done", "La orden pendiente ya no existe", symbol, 0, 0, "", "");
      return;
   }
   string side = (OrderGetInteger(ORDER_TYPE) % 2 == 0) ? "buy" : "sell";
   double price, stopLimit;
   CopyPendingPrices(order, symbol, side, price, stopLimit);
   double sl, tp;
   CopySlTp(order, symbol, side, stopLimit > 0.0 ? stopLimit : price, sl, tp);
   if(!g_trade.OrderModify(ticket, price, sl, tp, ORDER_TIME_GTC, 0, stopLimit) || !CopyTradeOk())
   {
      CopyAddResult(id, false, "rejected", CopyTradeError(), symbol, 0, 0, "", "");
      return;
   }
   CopyAddResult(id, true, "done", "", symbol, 0, price, "", IntegerToString(ticket));
}

void CopyPendingCancel(const string order, const string id, const string symbol)
{
   ulong ticket = CopyFindOrder(order);
   if(ticket == 0)
   {
      CopyAddResult(id, true, "done", "La orden pendiente ya no existe", symbol, 0, 0, "", "");
      return;
   }
   if(!g_trade.OrderDelete(ticket) || !CopyTradeOk())
   {
      CopyAddResult(id, false, "rejected", CopyTradeError(), symbol, 0, 0, "", "");
      return;
   }
   CopyAddResult(id, true, "done", "", symbol, 0, 0, "", IntegerToString(ticket));
}

// La pendiente de la principal se ejecutó: si la copia sigue pendiente se cancela y se entra a mercado;
// si ya se ejecutó, se informa la posición resultante.
void CopyPendingFill(const string order, const string id, const string symbol)
{
   ulong ticket = CopyFindOrder(order);
   if(ticket > 0)
   {
      g_trade.OrderDelete(ticket);
      CopyMarketEntry(order, id, symbol);
      return;
   }
   string pendingTag = JsonString("pending_tag", order);
   double volume = 0.0;
   string positionRef = "";
   double price = 0.0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
   {
      ulong position = PositionGetTicket(i);
      if(position == 0 || !PositionSelectByTicket(position))
         continue;
      if(PositionGetString(POSITION_COMMENT) != pendingTag)
         continue;
      volume += PositionGetDouble(POSITION_VOLUME);
      positionRef = IntegerToString(PositionGetInteger(POSITION_IDENTIFIER));
      price = PositionGetDouble(POSITION_PRICE_OPEN);
   }
   if(volume > 0.0)
      CopyAddResult(id, true, "filled", "", symbol, volume, price, positionRef, "");
   else
      CopyAddResult(id, false, "skipped", "La orden pendiente copiada ya no existe", symbol, 0, 0, "", "");
}

void CopyExecute(const string order)
{
   string id = JsonString("id", order);
   string action = JsonString("action", order);
   string symbol = CopyResolveSymbol(order);
   if(symbol == "")
   {
      string candidates[];
      JsonStringArray("symbols", order, candidates);
      string list = "";
      for(int i = 0; i < ArraySize(candidates); i++)
         list += (i > 0 ? ", " : "") + candidates[i];
      CopyAddResult(id, false, "rejected", "Símbolo no disponible en este terminal: " + list, "", 0, 0, "", "");
      return;
   }
   g_trade.SetTypeFillingBySymbol(symbol);

   if(action == "market" && JsonBool("reduce_only", order))
      CopyReduce(order, id, symbol, false);
   else if(action == "market")
      CopyMarketEntry(order, id, symbol);
   else if(action == "close")
      CopyReduce(order, id, symbol, true);
   else if(action == "sltp")
      CopyModifySlTp(order, id, symbol);
   else if(action == "pending_place")
      CopyPendingPlace(order, id, symbol);
   else if(action == "pending_modify")
      CopyPendingModify(order, id, symbol);
   else if(action == "pending_cancel")
      CopyPendingCancel(order, id, symbol);
   else if(action == "pending_fill")
      CopyPendingFill(order, id, symbol);
   else
      CopyAddResult(id, false, "rejected", "Acción desconocida: " + action, symbol, 0, 0, "", "");
}

void CopyProcessOrders(const string response)
{
   int key = StringFind(response, "\"orders\"");
   if(key < 0)
      return;
   int arrayStart = StringFind(response, "[", key);
   if(arrayStart < 0)
      return;
   int arrayEnd = FindMatching(response, arrayStart);
   if(arrayEnd < 0)
      return;
   int position = arrayStart + 1;
   while(position < arrayEnd)
   {
      int objectStart = StringFind(response, "{", position);
      if(objectStart < 0 || objectStart > arrayEnd)
         break;
      int objectEnd = FindMatching(response, objectStart);
      if(objectEnd < 0)
         break;
      CopyExecute(StringSubstr(response, objectStart, objectEnd - objectStart + 1));
      position = objectEnd + 1;
   }
   CopyFlushResults();
}

//--- Principal: detección de cambios ------------------------------------
void CopySnapshot(CopyPosSnap &positions[], CopyOrderSnap &orders[])
{
   ArrayResize(positions, 0);
   ArrayResize(orders, 0);
   int total = PositionsTotal();
   for(int i = 0; i < total; i++)
   {
      ulong ticket = PositionGetTicket(i);
      if(ticket == 0 || !PositionSelectByTicket(ticket))
         continue;
      int n = ArraySize(positions);
      ArrayResize(positions, n + 1);
      positions[n].ref = IntegerToString(PositionGetInteger(POSITION_IDENTIFIER));
      positions[n].symbol = PositionGetString(POSITION_SYMBOL);
      positions[n].type = (int)PositionGetInteger(POSITION_TYPE);
      positions[n].volume = PositionGetDouble(POSITION_VOLUME);
      positions[n].sl = PositionGetDouble(POSITION_SL);
      positions[n].tp = PositionGetDouble(POSITION_TP);
      positions[n].priceOpen = PositionGetDouble(POSITION_PRICE_OPEN);
   }
   total = OrdersTotal();
   for(int j = 0; j < total; j++)
   {
      ulong ticket = OrderGetTicket(j);
      if(ticket == 0 || !OrderSelect(ticket))
         continue;
      int type = (int)OrderGetInteger(ORDER_TYPE);
      if(type < ORDER_TYPE_BUY_LIMIT || type > ORDER_TYPE_SELL_STOP_LIMIT)
         continue;
      int n = ArraySize(orders);
      ArrayResize(orders, n + 1);
      orders[n].ticket = ticket;
      orders[n].symbol = OrderGetString(ORDER_SYMBOL);
      orders[n].type = type;
      orders[n].volume = OrderGetDouble(ORDER_VOLUME_CURRENT);
      orders[n].price = OrderGetDouble(ORDER_PRICE_OPEN);
      orders[n].stopLimit = OrderGetDouble(ORDER_PRICE_STOPLIMIT);
      orders[n].sl = OrderGetDouble(ORDER_SL);
      orders[n].tp = OrderGetDouble(ORDER_TP);
   }
}

string CopySide(const int positionType)
{
   return positionType == POSITION_TYPE_BUY ? "buy" : "sell";
}

string CopyOrderSide(const int orderType)
{
   return (orderType % 2 == 0) ? "buy" : "sell";
}

string CopyOrderKind(const int orderType)
{
   if(orderType == ORDER_TYPE_BUY_LIMIT || orderType == ORDER_TYPE_SELL_LIMIT)
      return "limit";
   if(orderType == ORDER_TYPE_BUY_STOP || orderType == ORDER_TYPE_SELL_STOP)
      return "stop";
   return "stop_limit";
}

string CopyPrice(const string symbol, const string side)
{
   return Num(side == "buy" ? SymbolInfoDouble(symbol, SYMBOL_ASK) : SymbolInfoDouble(symbol, SYMBOL_BID));
}

string CopyPositionSignal(const string kind, const CopyPosSnap &pos, const double volume,
                          const double remaining, const double price, const string orderRef)
{
   string side = CopySide(pos.type);
   string json = "{";
   json += "\"kind\":\"" + kind + "\",";
   json += "\"ref\":\"" + pos.ref + "\",";
   if(orderRef != "")
      json += "\"order_ref\":\"" + orderRef + "\",";
   json += "\"symbol\":\"" + Esc(pos.symbol) + "\",";
   json += "\"market\":\"" + CopyMarket(pos.symbol) + "\",";
   json += "\"side\":\"" + side + "\",";
   json += "\"volume\":" + Num(volume) + ",";
   json += "\"position_volume\":" + Num(remaining) + ",";
   json += "\"price\":" + Num(price) + ",";
   json += "\"entry_price\":" + Num(pos.priceOpen) + ",";
   json += "\"sl\":" + OptNum(pos.sl) + ",";
   json += "\"tp\":" + OptNum(pos.tp) + ",";
   json += "\"bid\":" + Num(SymbolInfoDouble(pos.symbol, SYMBOL_BID)) + ",";
   json += "\"ask\":" + Num(SymbolInfoDouble(pos.symbol, SYMBOL_ASK));
   json += "}";
   return json;
}

string CopyOrderSignal(const string kind, const CopyOrderSnap &ord)
{
   string side = CopyOrderSide(ord.type);
   string json = "{";
   json += "\"kind\":\"" + kind + "\",";
   json += "\"ref\":\"" + IntegerToString(ord.ticket) + "\",";
   json += "\"order_ref\":\"" + IntegerToString(ord.ticket) + "\",";
   json += "\"symbol\":\"" + Esc(ord.symbol) + "\",";
   json += "\"market\":\"" + CopyMarket(ord.symbol) + "\",";
   json += "\"side\":\"" + side + "\",";
   json += "\"order_type\":\"" + CopyOrderKind(ord.type) + "\",";
   json += "\"volume\":" + Num(ord.volume) + ",";
   json += "\"price\":" + CopyPrice(ord.symbol, side) + ",";
   json += "\"order_price\":" + Num(ord.price) + ",";
   json += "\"stop_limit_price\":" + OptNum(ord.stopLimit) + ",";
   json += "\"sl\":" + OptNum(ord.sl) + ",";
   json += "\"tp\":" + OptNum(ord.tp) + ",";
   json += "\"bid\":" + Num(SymbolInfoDouble(ord.symbol, SYMBOL_BID)) + ",";
   json += "\"ask\":" + Num(SymbolInfoDouble(ord.symbol, SYMBOL_ASK));
   json += "}";
   return json;
}

void CopyAppend(string &list, const string item)
{
   if(list != "")
      list += ",";
   list += item;
}

bool CopyOrderFilled(const ulong ticket)
{
   if(!HistoryOrderSelect(ticket))
   {
      HistorySelect(TimeCurrent() - 86400, TimeCurrent() + 3600);
      if(!HistoryOrderSelect(ticket))
         return false;
   }
   ENUM_ORDER_STATE state = (ENUM_ORDER_STATE)HistoryOrderGetInteger(ticket, ORDER_STATE);
   return state == ORDER_STATE_FILLED || state == ORDER_STATE_PARTIAL;
}

void CopyStorePositions(const CopyPosSnap &source[])
{
   int n = ArraySize(source);
   ArrayResize(g_copyPositions, n);
   for(int i = 0; i < n; i++)
      g_copyPositions[i] = source[i];
}

void CopyStoreOrders(const CopyOrderSnap &source[])
{
   int n = ArraySize(source);
   ArrayResize(g_copyOrders, n);
   for(int i = 0; i < n; i++)
      g_copyOrders[i] = source[i];
}

void CopyMasterCycle()
{
   CopyPosSnap positions[];
   CopyOrderSnap orders[];
   CopySnapshot(positions, orders);

   if(!g_copyBaseline)
   {
      // Lo que ya estaba abierto al activar la copia no se copia.
      CopyStorePositions(positions);
      CopyStoreOrders(orders);
      g_copyBaseline = true;
      return;
   }

   string signals = "";
   string symbolsUsed[];

   // Órdenes pendientes que han desaparecido: ejecutadas o canceladas.
   ulong filledOrders[];
   string filledSymbols[];
   for(int i = 0; i < ArraySize(g_copyOrders); i++)
   {
      bool alive = false;
      for(int j = 0; j < ArraySize(orders); j++)
         if(orders[j].ticket == g_copyOrders[i].ticket) { alive = true; break; }
      if(alive)
         continue;
      if(CopyOrderFilled(g_copyOrders[i].ticket))
      {
         int n = ArraySize(filledOrders);
         ArrayResize(filledOrders, n + 1);
         ArrayResize(filledSymbols, n + 1);
         filledOrders[n] = g_copyOrders[i].ticket;
         filledSymbols[n] = g_copyOrders[i].symbol;
      }
      else
      {
         CopyAppend(signals, CopyOrderSignal("pending_cancel", g_copyOrders[i]));
         AddSymbol(g_copyOrders[i].symbol, symbolsUsed);
      }
   }

   // Posiciones nuevas, ampliadas, reducidas, invertidas o con SL/TP cambiados.
   for(int p = 0; p < ArraySize(positions); p++)
   {
      int prev = -1;
      for(int q = 0; q < ArraySize(g_copyPositions); q++)
         if(g_copyPositions[q].ref == positions[p].ref) { prev = q; break; }

      string orderRef = "";
      for(int f = 0; f < ArraySize(filledOrders); f++)
      {
         if(filledSymbols[f] == positions[p].symbol &&
            (IntegerToString(filledOrders[f]) == positions[p].ref || prev >= 0))
         {
            orderRef = IntegerToString(filledOrders[f]);
            break;
         }
      }

      string side = CopySide(positions[p].type);
      double now = side == "buy" ? SymbolInfoDouble(positions[p].symbol, SYMBOL_ASK) : SymbolInfoDouble(positions[p].symbol, SYMBOL_BID);
      if(prev < 0)
      {
         CopyAppend(signals, CopyPositionSignal("open", positions[p], positions[p].volume, positions[p].volume, positions[p].priceOpen, orderRef));
         AddSymbol(positions[p].symbol, symbolsUsed);
         continue;
      }

      CopyPosSnap before = g_copyPositions[prev];
      if(before.type != positions[p].type)
      {
         CopyAppend(signals, CopyPositionSignal("close", before, before.volume, 0.0, now, ""));
         CopyAppend(signals, CopyPositionSignal("open", positions[p], positions[p].volume, positions[p].volume, positions[p].priceOpen, orderRef));
         AddSymbol(positions[p].symbol, symbolsUsed);
         continue;
      }
      if(positions[p].volume > before.volume + 1e-9)
      {
         CopyAppend(signals, CopyPositionSignal("increase", positions[p], positions[p].volume - before.volume, positions[p].volume, now, orderRef));
         AddSymbol(positions[p].symbol, symbolsUsed);
      }
      else if(positions[p].volume < before.volume - 1e-9)
      {
         double exitPrice = side == "buy" ? SymbolInfoDouble(positions[p].symbol, SYMBOL_BID) : SymbolInfoDouble(positions[p].symbol, SYMBOL_ASK);
         CopyAppend(signals, CopyPositionSignal("reduce", positions[p], before.volume - positions[p].volume, positions[p].volume, exitPrice, ""));
         AddSymbol(positions[p].symbol, symbolsUsed);
      }
      if(MathAbs(positions[p].sl - before.sl) > 1e-12 || MathAbs(positions[p].tp - before.tp) > 1e-12)
      {
         CopyAppend(signals, CopyPositionSignal("sltp", positions[p], 0.0, positions[p].volume, now, ""));
         AddSymbol(positions[p].symbol, symbolsUsed);
      }
   }

   for(int c = 0; c < ArraySize(g_copyPositions); c++)
   {
      bool alive = false;
      for(int p2 = 0; p2 < ArraySize(positions); p2++)
         if(positions[p2].ref == g_copyPositions[c].ref) { alive = true; break; }
      if(alive)
         continue;
      string side = CopySide(g_copyPositions[c].type);
      double exitPrice = side == "buy" ? SymbolInfoDouble(g_copyPositions[c].symbol, SYMBOL_BID) : SymbolInfoDouble(g_copyPositions[c].symbol, SYMBOL_ASK);
      CopyAppend(signals, CopyPositionSignal("close", g_copyPositions[c], g_copyPositions[c].volume, 0.0, exitPrice, ""));
      AddSymbol(g_copyPositions[c].symbol, symbolsUsed);
   }

   // Órdenes pendientes nuevas o modificadas.
   for(int o = 0; o < ArraySize(orders); o++)
   {
      int prev = -1;
      for(int r = 0; r < ArraySize(g_copyOrders); r++)
         if(g_copyOrders[r].ticket == orders[o].ticket) { prev = r; break; }
      if(prev < 0)
      {
         CopyAppend(signals, CopyOrderSignal("pending_place", orders[o]));
         AddSymbol(orders[o].symbol, symbolsUsed);
         continue;
      }
      CopyOrderSnap before = g_copyOrders[prev];
      if(MathAbs(before.price - orders[o].price) > 1e-12 || MathAbs(before.stopLimit - orders[o].stopLimit) > 1e-12 ||
         MathAbs(before.sl - orders[o].sl) > 1e-12 || MathAbs(before.tp - orders[o].tp) > 1e-12)
      {
         CopyAppend(signals, CopyOrderSignal("pending_modify", orders[o]));
         AddSymbol(orders[o].symbol, symbolsUsed);
      }
   }

   CopyStorePositions(positions);
   CopyStoreOrders(orders);

   if(signals == "")
      return;

   string specs = "";
   for(int s = 0; s < ArraySize(symbolsUsed); s++)
      CopyAppend(specs, CopySpecJson(symbolsUsed[s]));
   string body = "{\"provider\":\"mt5\"," + CopyAccountJson(false, false) +
                 ",\"specs\":[" + specs + "],\"signals\":[" + signals + "]}";
   string response = "";
   int status = 0;
   if(!HttpPost("/api/v1/copy/signal", body, response, status))
      Print("JournalTrading: no se pudo enviar la señal de copia");
}

//--- Ciclo ---------------------------------------------------------------
void CopyPoll()
{
   ulong now = GetTickCount64();
   bool specsDue = g_copyLastSpecs == 0 || now - g_copyLastSpecs >= 10000;
   if(specsDue)
      g_copyLastSpecs = now;
   bool follower = g_copyRole == "follower";
   string body = "{\"provider\":\"mt5\",\"bridge_version\":\"" + EA_VERSION + "\",\"wait_ms\":0,\"accounts\":[{" +
                 CopyAccountJson(specsDue, follower) + "}]}";
   string response = "";
   int status = 0;
   if(!HttpPost("/api/v1/copy/poll", body, response, status, 3000))
      return;

   string role = JsonString("role", response);
   bool enabled = JsonBool("enabled", response);
   if(role == "")
      role = "none";
   if(role != g_copyRole || enabled != g_copyEnabled)
   {
      if(role == "master" && enabled && !(g_copyRole == "master" && g_copyEnabled))
         g_copyBaseline = false;
      g_copyRole = role;
      g_copyEnabled = enabled;
      PrintFormat("JournalTrading: copy trading · rol %s · %s", role, enabled ? "activo" : "pausado");
   }
   if(g_copyRole == "follower")
      CopyProcessOrders(response);
}

void CopyCycle()
{
   if(InpApiKey == "" || !InpCopyTrading)
      return;
   ulong now = GetTickCount64();
   ulong interval = g_copyRole == "follower" ? (ulong)MathMax(50, InpCopyPollMs) : 1000;
   if(g_copyLastPoll == 0 || now - g_copyLastPoll >= interval)
   {
      g_copyLastPoll = now;
      CopyPoll();
   }
   if(g_copyRole == "master" && g_copyEnabled)
      CopyMasterCycle();
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
      Print("JournalTrading: InpApiKey está vacía. Configura la API key en los ajustes de la app.");

   g_trade.SetExpertMagicNumber(InpCopyMagic);
   g_trade.SetTypeFillingBySymbol(Symbol());
   g_trade.SetAsyncMode(false);
   EventSetMillisecondTimer(100);
   g_forceHeartbeat = true;
   g_forceSync = true;
   PrintFormat("JournalTrading Bridge %s iniciado. Endpoint: %s", EA_VERSION, g_apiBase);
   return INIT_SUCCEEDED;
}

void OnDeinit(const int reason)
{
   EventKillTimer();
   Comment("");
}

void OnTimer()
{
   CopyCycle();

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
