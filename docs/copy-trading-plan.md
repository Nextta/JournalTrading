# Plan: Copy trading

Estado: **implementado** (versión 0.1.4) · 2026-11-13

## 1. Objetivo

Copiar en tiempo real las operaciones de una **cuenta principal** a una o varias **cuentas secundarias**, en MT5, NinjaTrader 8 y ATAS X. JournalTrading hace de intermediario: el bridge de la principal envía cada acción a la app, la app calcula el tamaño para cada secundaria y el bridge de cada secundaria ejecuta la orden.

Todo se configura desde la app: grupos, roles, gestión del lotaje, protecciones y activar o pausar la copia.

## 2. Decisiones tomadas

| Tema | Decisión |
| --- | --- |
| Latencia | Menos de 0,5 s de principal a secundaria. |
| Qué se copia | Entradas y salidas a mercado (incluidos cierres parciales), SL/TP y sus modificaciones, órdenes pendientes (alta, modificación y cancelación). |
| Bridges | Se amplía el bridge actual de cada plataforma. El rol (solo diario, principal o secundaria) se asigna desde la app; no hay que instalar nada nuevo. |
| Estructura | **Grupos de copia**: cada grupo tiene una principal y N secundarias. Puede haber varios grupos activos a la vez. |
| Símbolos | Emparejamiento automático para futuros (raíz + vencimiento) y tabla manual editable para el resto. |
| Compatibilidad | Solo se copia entre instrumentos del mismo tipo: futuros con futuros, CFD/Forex con CFD/Forex. Una cuenta MT5 que opere futuros sí puede copiarse con NT8/ATAS. |
| % de balance/equity | Es un **% de riesgo** calculado con la distancia al SL. |
| Kelly | Se calcula con el historial de la principal en el diario, con una fracción configurable (p. ej. ½ Kelly) y un riesgo máximo. |
| Secundaria desconectada | Al reconectar no entra tarde si se superan X segundos o X ticks/pips de desvío. Los cierres se aplican siempre. Se avisa en la app. |
| Protecciones | Tamaño máximo por operación y por posición, límite de pérdida diaria (pausa la cuenta), botón de pánico, filtro de símbolos y horario. |
| Operaciones manuales | Son independientes: la copia solo gestiona las posiciones que ha abierto ella (marcadas con un ID). |
| Extras por secundaria | Invertir dirección, y elegir si se copia o no el SL/TP de la principal. |

## 3. Arquitectura

```text
 Cuenta principal                 JournalTrading (backend Rust)                Cuentas secundarias
┌──────────────┐  evento    ┌──────────────────────────────────────┐  orden   ┌──────────────┐
│ Bridge (rol  │──────────▶│ 1. Recibe señal (copy_signals)        │────────▶│ Bridge (rol  │
│  principal)  │  POST     │ 2. Por cada secundaria del grupo:     │ canal    │  secundaria) │
└──────────────┘           │    filtros → símbolo → tamaño →       │ rápido   └──────┬───────┘
                           │    protecciones → orden (copy_orders) │                 │
                           │ 3. Registra resultado y latencia      │◀────────────────┘
                           └──────────────────────────────────────┘   resultado
```

### 3.1 Canal rápido app ↔ bridges

Hoy los comandos viajan en la respuesta del heartbeat (cada 1–5 s), que es demasiado lento. Se añade un canal dedicado:

- **Principal → app**: el bridge hace `POST /api/v1/copy/signal` en el mismo momento del evento (`OnTradeTransaction` en MT5, eventos de `Account` en NT8, eventos de `TradingManager` en ATAS). No espera al heartbeat.
- **App → secundaria**:
  - **NT8 y ATAS (C#)**: *long-polling* asíncrono `GET /api/v1/copy/orders?wait=25s`. La app responde en cuanto hay una orden, normalmente en milisegundos.
  - **MT5**: `WebRequest` bloquea el EA, así que se usa sondeo corto (temporizador de 100 ms, consulta cada `InpCopyPollMs` = 150 ms) contra `127.0.0.1`.
- **Secundaria → app**: `POST /api/v1/copy/result` con el estado, el precio de ejecución y el ticket o ID de la orden.
- **Detección en la principal**: los tres bridges comparan cada 100 ms una foto de posiciones y órdenes con la anterior (aperturas, ampliaciones, cierres parciales, inversiones, SL/TP, pendientes). En NT8 y ATAS, las órdenes stop/limit contrarias a la posición se interpretan como SL/TP.
- **Estado de lo copiado**: la app guarda el volumen copiado por posición (`copy_positions`). Los bridges informan en cada consulta del volumen real por etiqueta (`copies`) y la app lo concilia (cierres por SL/TP o manuales). NT8 y ATAS guardan su parte en `JournalTradingCopyState.json`.
- Todas las rutas usan la misma `X-Api-Key`.

### 3.2 Identificación de posiciones copiadas

Cada orden copiada lleva un ID de copia `JT:<grupo>:<señal>`:

- **MT5**: en el `comment`, y además un `magic` propio.
- **NT8**: en el nombre de la orden (`Order.Name`) o el OCO/tag.
- **ATAS**: en el comentario de la orden o en un mapa interno del bridge.

Esto permite cerrar, modificar o cancelar exactamente lo que abrió la copia, sin tocar lo manual.

### 3.3 Idempotencia y orden

- Cada señal tiene un número de secuencia por grupo. Las secundarias confirman cada orden y la app no la reenvía si ya está confirmada.
- Si una orden falla (mercado cerrado, margen insuficiente, símbolo no disponible…), se registra el error y se avisa. No se reintenta una entrada salvo que siga dentro de la ventana de desvío.

## 4. Cambios por plataforma

| | MT5 (EA) | NinjaTrader 8 (AddOn) | ATAS X (indicador) |
| --- | --- | --- | --- |
| Detectar acciones (principal) | `OnTradeTransaction`: deals, órdenes, cambios de SL/TP | `Account.ExecutionUpdate`, `OrderUpdate`, `PositionUpdate` | `TradingManager`: `OrderChanged`, `NewMyTrade`, `PositionChanged` |
| Ejecutar (secundaria) | `CTrade`: `Buy/Sell`, `PositionModify`, `PositionClosePartial`, `OrderSend` pendientes | `Account.CreateOrder` + `Submit`, `Change`, `Cancel`; SL/TP como órdenes OCO | Ver riesgo de ATAS abajo |
| Datos para el lotaje | `SYMBOL_VOLUME_MIN/STEP/MAX`, `TRADE_CONTRACT_SIZE`, `TICK_VALUE`, balance, equity | `Instrument.MasterInstrument.PointValue/TickSize`, balance de la cuenta | `Security.TickSize/TickCost`, balance del portfolio |
| Particularidades | Cuentas *netting* frente a *hedging*. En *hedging* cada copia es una posición propia. | SL/TP no son atributos de la posición: se gestionan como órdenes stop/limit vinculadas. | Igual que NT8: SL/TP como órdenes separadas. |

**ATAS:** comprobado en la API instalada: el indicador puede enviar, modificar y cancelar órdenes con `TradingManager.OpenOrderAsync/ModifyOrderAsync/CancelOrderAsync`, así que no hace falta una `ChartStrategy`. La limitación es que `TradingManager` opera la cuenta y el instrumento del gráfico: en cada secundaria hace falta un gráfico con el indicador por instrumento. Cada indicador informa de su instrumento (`tradable`) y solo recibe las órdenes de ese instrumento.

## 5. Compatibilidad y símbolos

1. **Tipo de mercado**: cada símbolo se clasifica como `futures` o `cfd_forex`:
   - **NT8 y ATAS**: siempre `futures` (a no ser que el instrumento indique otra cosa).
   - **MT5**: según `SYMBOL_TRADE_CALC_MODE`. `FUTURES`/`EXCH_FUTURES` es `futures`; `FOREX`/`CFD*` es `cfd_forex`.
2. **Al configurar un grupo**, la app avisa si una secundaria no puede operar ningún símbolo compatible con la principal (por ejemplo, una principal MT5 solo Forex con una secundaria NT8).
3. **Al copiar**, si el símbolo no es compatible o no se puede emparejar, esa secundaria se salta y se registra el motivo.
4. **Emparejamiento automático de futuros**: se normaliza a `RAÍZ + MES + AÑO`:
   - ATAS `NQZ6@CME` → `NQ-2026-12`
   - NT8 `NQ 12-26` → `NQ-2026-12`
   - MT5 `NQZ26`, `NQ_Z6`, etc., con patrones habituales.
5. **Tabla manual** por secundaria (`símbolo principal → símbolo secundaria`) para CFD/Forex con sufijos (`EURUSD` → `EURUSD.m`) y para casos que el automático no resuelva.

## 6. Gestión del tamaño

Cada secundaria tiene su método. **Por defecto: mismo tamaño que la principal.**

| Método | Futuros | CFD/Forex | Cálculo |
| --- | :---: | :---: | --- |
| Mismo tamaño | ✔ | ✔ | `lotes = lotes_principal` |
| Tamaño fijo | ✔ | ✔ | `lotes = N` indicado por el usuario |
| Multiplicador | ✔ | ✔ | `lotes = lotes_principal × M` |
| % de riesgo sobre balance | — | ✔ | `lotes = (balance × %) / (distancia_SL × valor_por_lote_y_punto)` |
| % de riesgo sobre equity | — | ✔ | igual que el anterior, con la equity |
| Kelly | — | ✔ | `f = W − (1 − W) / R`, luego `riesgo = min(f × fracción, riesgo_máx)` y se aplica como % de riesgo sobre el balance |
| Fórmula nocional | — | ✔ | `lotes = (balance / (precio × contract_size)) × M` |

Detalles:

- **Cierres parciales**: se cierra la misma **proporción** que en la principal. Si la principal cierra el 50 %, la secundaria cierra el 50 % de su posición copiada, sea cual sea el método.
- **Sin SL** en los métodos de riesgo (% y Kelly): se usa un **método de respaldo** configurable (por defecto, mismo tamaño). Si la principal pone el SL después de entrar, el tamaño no se recalcula.
- **Kelly**:
  - W = win rate y R = beneficio medio / pérdida media de las últimas N operaciones cerradas de la principal (N configurable, por defecto 50).
  - Con menos de 20 operaciones se usa el respaldo.
  - Si `f ≤ 0`, no se copia y se avisa.
- **Fórmula nocional**: si la divisa de cotización no es la de la cuenta, se convierte el precio con el tipo de cambio que ofrezca el bróker de la secundaria.
- **Normalización**: el resultado se redondea **hacia abajo** al paso de volumen (`volume_step`) y se limita a `[mínimo, máximo]` del símbolo y al **tamaño máximo** de la cuenta. Si queda por debajo del mínimo, no se copia y se avisa. En futuros se redondea a contratos enteros.
- **Invertir dirección**: compra ↔ venta. SL y TP se intercambian de forma coherente.

## 7. Protecciones y control

- **Interruptor por grupo y por secundaria** (activar o pausar) y un **interruptor global** en la cabecera de la app.
- **Botón de pánico**: cierra todas las posiciones y cancela todas las órdenes copiadas en todas las secundarias (o en un grupo) y pausa la copia.
- **Tamaño máximo**: por operación y por posición abierta total del símbolo.
- **Límite de pérdida diaria**: realizado + flotante desde el inicio del día del bróker. Al alcanzarlo se pausa la copia de esa cuenta. Opcionalmente, también se cierran las copias abiertas.
- **Filtro de símbolos** (lista blanca o negra) y **franja horaria** por secundaria.
- **Entrada tardía**: no se abre si han pasado más de *X* s (por defecto 5) o si el precio se ha desviado más de *X* ticks/pips (por defecto 10). Las salidas, cierres y cancelaciones se aplican siempre.
- **Pérdida de conexión de la principal**: las secundarias mantienen sus posiciones y SL/TP, y la app avisa. No se cierra nada automáticamente.

## 8. Modelo de datos (SQLite)

| Tabla | Contenido |
| --- | --- |
| `copy_groups` | id, nombre, `master_account_id`, activo |
| `copy_followers` | id, `group_id`, `account_id`, activo, método de tamaño + parámetros (JSON), método de respaldo, invertir, copiar SL/TP, máx. por operación y por posición, pérdida diaria máx., filtros de símbolos y horario, desvío máx. (s / ticks) |
| `copy_symbol_map` | `follower_id`, símbolo principal, símbolo secundaria |
| `copy_signals` | id, `group_id`, secuencia, tipo (abrir, cerrar, parcial, modificar SL/TP, pendiente alta/mod/cancel), payload, hora |
| `copy_orders` | id, `signal_id`, `follower_id`, ID de copia, estado (pendiente, enviada, ejecutada, rechazada, saltada), motivo, lotes calculados, precio, latencia en ms |
| `accounts` | nueva columna `copy_role`: `none` / `master` / `follower` (derivada de los grupos) |

Una cuenta puede ser principal de un grupo **o** secundaria de uno o varios, pero nunca principal y secundaria a la vez, para evitar bucles.

En el diario, las operaciones copiadas se guardan como cualquier otra de la secundaria, con un enlace a la operación de la principal (`copy_signal_id`) para poder comparar resultados y slippage.

## 9. Interfaz (nueva página "Copy trading")

- **Lista de grupos**: principal, secundarias, estado de conexión de cada cuenta (en línea, desconectada o sin bridge con copia), interruptor y botón de pánico.
- **Editor de grupo**: elegir la principal y añadir secundarias (solo aparecen las compatibles; las incompatibles salen deshabilitadas con el motivo).
- **Ficha de secundaria**: método de tamaño con sus parámetros y una **vista previa** ("con esta operación de la principal se abrirían 0,37 lotes"), protecciones, filtros, tabla de símbolos e invertir / copiar SL/TP.
- **Registro en vivo**: cada señal y su resultado por cuenta (latencia, lotes, slippage, errores).
- **Ajustes > Cómo conectar tus plataformas**: añadir los pasos de copy trading por plataforma (activar el trading algorítmico en MT5, permisos en NT8, estrategia en ATAS si hace falta).

## 10. Fases de implementación

1. **Backend base**: tablas y migración, API `copy/*`, motor de copia (filtros → símbolo → tamaño → protecciones → orden), canal rápido (long-polling) y tests unitarios del cálculo de tamaños y de símbolos.
2. **MT5 ↔ MT5**: rol principal y secundaria en el EA. Es el caso más fácil de probar con dos cuentas demo. Incluye todos los métodos de tamaño de CFD/Forex.
3. **Interfaz**: página de copy trading, editor de grupos y secundarias, registro en vivo, interruptores y botón de pánico.
4. **NinjaTrader 8**: principal y secundaria, SL/TP como OCO, futuros NT8 ↔ NT8 y NT8 ↔ MT5 (futuros).
5. **ATAS X**: verificar la capacidad de enviar órdenes (indicador frente a `ChartStrategy`), y después principal y secundaria.
6. **Protecciones avanzadas**: pérdida diaria, horario, entrada tardía con reconexión y métricas de slippage en el diario.
7. **Documentación**: README, tutoriales en Ajustes y scripts de simulación (`tools/simulate-*.ps1`) para probar sin plataformas.

Cada fase termina con la app compilando, pruebas con cuentas **demo** y la subida de versión.

## 11. Preguntas resueltas

1. **Mini ↔ micro ↔ nano**: se pueden copiar entre sí (NQ ↔ MNQ, ES ↔ MES, BTC ↔ MBT…). El emparejamiento automático reconoce la familia y el tamaño se ajusta por el valor del punto para que, con «mismo tamaño», la exposición sea equivalente (1 NQ → 10 MNQ). El usuario puede desactivar el ajuste por familia y copiar contrato a contrato.
2. **Pendientes en CFD/Forex**: se copia la **distancia al precio actual** (offset), no el precio exacto. SL y TP también se trasladan como distancia al precio de entrada. En futuros del mismo contrato se copia el precio exacto.
3. **Pausar la copia**: las órdenes pendientes y posiciones copiadas **se dejan**. Al pausar, la app ofrece una opción explícita para cancelarlas o cerrarlas.
4. **Cuentas de fondeo**: no hace falta nada más que el filtro horario; el resto lo gestiona el usuario.
