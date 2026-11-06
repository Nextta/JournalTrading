# AddOn de JournalTrading para NinjaTrader 8

AddOn de **solo lectura** que sincroniza cuentas, ejecuciones, posiciones abiertas y velas de
NinjaTrader 8 con JournalTrading.

## Requisitos

- NinjaTrader 8 instalado y con una cuenta conectada al menos una vez.
- .NET Framework 4.8 (lo instala NinjaTrader).
- MSBuild / Visual Studio Build Tools si quieres compilar desde PowerShell.
- JournalTrading abierto y su API key copiada desde **Ajustes > Seguridad**.

## Instalación rápida

1. Cierra NinjaTrader 8.
2. Desde la raíz del repositorio:

   ```powershell
   .\tools\install-nt8-bridge.ps1 -ApiKey "tu-api-key"
   ```

   El script:
   - Copia `JournalTradingBridge.cs` a `Documents\NinjaTrader 8\bin\Custom\AddOns`.
   - Añade el archivo a `NinjaTrader.Custom.csproj` (con copia de seguridad).
   - Genera `JournalTradingBridge.json` con la URL y la API key.
   - Compila con MSBuild y copia el DLL a la raíz de `Custom`.

3. Abre NinjaTrader 8. El AddOn se carga automáticamente.

Si prefieres compilar desde NinjaTrader:

```powershell
.\tools\install-nt8-bridge.ps1 -NoBuild
```

Después abre **Herramientas > Editor de NinjaScript** y pulsa **F5**.

## Configuración

Archivo: `Documents\NinjaTrader 8\bin\Custom\JournalTradingBridge.json`

```json
{
  "apiUrl": "http://127.0.0.1:4477",
  "apiKey": "tu-api-key",
  "heartbeatSeconds": 5,
  "syncSeconds": 15,
  "historyDays": 0,
  "verbose": true
}
```

La app puede generar o actualizar este archivo desde **Ajustes > Puente NinjaTrader 8 >
Escribir configuración NT8**. El AddOn detecta cambios en el archivo sin reiniciar NinjaTrader.

## Qué envía

- **Heartbeat** cada `heartbeatSeconds`: balance, equity, margen, divisa y datos de la cuenta.
- **Ejecuciones** cada `syncSeconds`: entradas y salidas agrupadas en operaciones (de plano a
  plano) por cuenta e instrumento. El beneficio de cada salida se calcula con el precio medio de
  la posición abierta y el `PointValue` del instrumento.
- **Posiciones abiertas**: volumen, precio medio, P&L no realizado y SL/TP aproximados a partir
  de las órdenes activas.
- **Velas** bajo demanda: la app solicita un timeframe y el AddOn usa `BarsRequest` para
  obtenerlas y enviarlas.

## Historial

El AddOn consulta la base de datos local de NinjaTrader con `Execution.DbGet(account, from, to)`.
En la primera sincronización (y al pulsar **Sincronizar** en la app) lee todo el historial de NT8
y reconstruye las operaciones recorriendo las ejecuciones de cada instrumento en orden y
acumulando la posición neta: una operación empieza cuando la posición sale de cero y termina
cuando vuelve a cero; las ejecuciones que invierten la posición se dividen en salida y entrada.
`historyDays` (0 = todo) solo limita qué operaciones se envían. Después solo envía las
operaciones abiertas o con actividad en los últimos 7 días.

Tras una sincronización completa, la app elimina los deals de NT8 de ese periodo que ya no
existen en NinjaTrader y las operaciones que se quedan sin deals (por ejemplo, agrupaciones
generadas por versiones anteriores del AddOn).

Las cuentas de backtest y playback se ignoran.

## Limitaciones

- No envía órdenes ni modifica posiciones.
- La cantidad de historial que NT8 conserva en su base de datos depende de la configuración de la
  cuenta (`LookbackDaysExecutions`). Si faltan operaciones antiguas, auméntala en NT8.
- Los símbolos de futuros se guardan con el contrato completo (`ES 12-25`). Para contratos
  expirados, el AddOn intenta resolver el instrumento con `GetInstrumentFuzzy`.
- El beneficio de una salida se calcula con el precio medio de entrada del grupo. En trades con
  scale-in/scale-out muy entrelazados puede ser una aproximación.
- El beneficio usa la fórmula de NinjaTrader: `(salida - entrada) * PointValue * LotSize * cantidad`,
  convertido a la divisa de la cuenta con el tipo de cambio de NT8 cuando está disponible. Para
  forex, `LotSize` es el **FX Lot Size** de la cuenta; para el resto de instrumentos es 1.
  Si usas un CFD con un Point Value personalizado, ajústalo en el Instrument Manager de NT8.
- El SL/TP de posiciones abiertas se infiere de las órdenes activas; si no hay órdenes de stop
  o target, se envía `null`.
- El AddOn usa la zona horaria configurada en NT8 (**Herramientas > Opciones > General**) como
  hora del bróker. La app agrupa los días con esa hora.

## Solución de problemas

- **No conecta**: revisa `JournalTradingBridge.json`, la API key y el puerto. En la app pulsa
  **Escribir configuración NT8**.
- **No aparecen operaciones**: mira la pestaña **Output** de NinjaTrader. La primera
  sincronización puede tardar si hay mucho historial.
- **No aparecen velas**: el instrumento debe tener datos históricos en NT8. Revisa los errores
  de `BarsRequest` en Output.
- **El DLL no se actualiza**: cierra NinjaTrader antes de compilar. El DLL está bloqueado
  mientras la aplicación está abierta.
- **Quiero desinstalarlo**: elimina `AddOns\JournalTradingBridge.cs`, quita su línea del
  `NinjaTrader.Custom.csproj`, recompila y borra `JournalTradingBridge.json`.
