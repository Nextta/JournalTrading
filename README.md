# Plan de Trading

Diario de trading de escritorio para Windows con sincronización en tiempo real desde MetaTrader 5. La aplicación combina un frontend Angular, un backend Rust/Tauri con SQLite y un Expert Advisor (EA) para MT5.

## Características

- **Dashboard**: resumen de cuenta, balance, equity, rendimiento y actividad reciente.
- **Diario de trading**: una entrada por cuenta y día, con notas generales, análisis por activo, imágenes y checklist de estrategia.
- **Operaciones**: listado filtrable por fecha, símbolo, estrategia y estado, con detalle de deals, notas e imágenes.
- **Gráficos**: velas M1, M5, M15, M30, H1, H4, D1 y W1 bajo demanda, con caché local, marcadores de entrada/salida con volumen, motivo de cierre (SL/TP/cierre) y línea de recorrido.
- **Estadísticas**: beneficio neto, win rate, profit factor, expectancy, drawdown, distribución de R, resultados por símbolo, estrategia, día de la semana, hora y mes.
- **Estrategias**: checklists agrupadas por secciones, puntos obligatorios y asignación por activo.
- **Cuentas**: soporte multi-cuenta, activación, archivo y aprobación de cuentas nuevas detectadas por el EA.
- **Ajustes**: API key, puerto local, zona horaria, carpeta de datos y copias de seguridad.
- **Integración MT5**: captura todas las operaciones de la cuenta, sin filtrar por número mágico.

## Arquitectura

```text
MetaTrader 5
  └─ Expert Advisor (MQL5)
        │ HTTP JSON + X-Api-Key
        ▼
Backend Tauri / Rust (axum, 127.0.0.1:4477)
  ├─ SQLite (%APPDATA%\com.jairo.plandetrading)
  ├─ Eventos Tauri
  └─ Comandos Tauri
        │ IPC
        ▼
Frontend Angular 22 (WebView2)
```

- El frontend no accede directamente a la API HTTP: usa comandos y eventos de Tauri.
- El EA envía heartbeats, operaciones, deals y velas; las peticiones de velas se encolan y el EA las resuelve en la siguiente sincronización.
- Toda la información se guarda localmente. No se envía nada a servicios externos.

## Stack

| Componente | Tecnología |
| --- | --- |
| Escritorio | Tauri 2.12 |
| Backend | Rust 2024, axum, sqlx, SQLite |
| Frontend | Angular 22 standalone, signals, Angular Material |
| Gráficos | ECharts, lightweight-charts |
| Editor | Quill 2 |
| EA | MQL5 |
| Instalador | NSIS |

## Requisitos

- Windows 10/11 x64.
- Node.js 24+ y npm 11+.
- Rust 1.90+ con toolchain MSVC.
- Visual Studio Build Tools con C++ y Windows SDK.
- WebView2 Runtime.
- PowerShell 5.1+ (solo para el simulador).
- MetaTrader 5 (opcional, para datos reales).

## Desarrollo

Instala las dependencias del proyecto y del frontend:

```powershell
npm install
npm --prefix frontend install
```

Arranca la aplicación en modo desarrollo:

```powershell
npm run dev
```

Este comando inicia Angular en `http://localhost:4200` y abre la ventana de Tauri. En el primer arranque se crea la base de datos y una API key nueva.

Para servir solo el frontend en el navegador:

```powershell
npm start
```

Ten en cuenta que en ese modo no están disponibles las APIs de Tauri (`invoke` y eventos), por lo que las páginas que cargan datos desde el backend no funcionarán.

## Compilación

```powershell
npm run build
```

Artefactos generados:

- Ejecutable: `backend\target\release\plan-de-trading.exe`
- Instalador: `backend\target\release\bundle\nsis\Plan de Trading_0.1.0_x64-setup.exe`

También puedes compilar únicamente el frontend:

```powershell
npm run build:web
```

## Conexión con MetaTrader 5

1. Abre la app y ve a **Ajustes > Seguridad** para copiar la API key.
2. En MT5, ve a **Herramientas > Opciones > Expert Advisors**:
   - Activa el trading automático.
   - Añade `http://127.0.0.1:4477` a la lista de URLs permitidas para WebRequest.
3. Copia `mt5\TradingJournalBridge.ex5` en la carpeta `MQL5\Experts` de tu terminal (puedes abrirla desde MT5 con **Archivo > Abrir carpeta de datos**). También puedes abrir `TradingJournalBridge.mq5` en MetaEditor y compilarlo.
4. Adjunta el EA a un gráfico de la cuenta que quieras sincronizar y activa el botón **Algo Trading**.
5. Configura los parámetros del EA:

| Parámetro | Valor por defecto | Descripción |
| --- | --- | --- |
| `InpApiUrl` | `http://127.0.0.1:4477` | URL local del backend |
| `InpApiKey` | vacío | API key copiada desde Ajustes |
| `InpHeartbeatSeconds` | `5` | Frecuencia del heartbeat |
| `InpSyncSeconds` | `15` | Frecuencia de sincronización |
| `InpHistoryDays` | `90` | Días de historial en la primera sincronización |
| `InpVerbose` | `true` | Logs detallados en MT5 |

6. La primera vez, la cuenta aparecerá en estado **pendiente** en la sección **Cuentas**. Actívala para empezar a usarla.

Notas:

- El EA envía todas las operaciones de la cuenta, independientemente del magic number.
- La sincronización inicial descarga los últimos 90 días y después envía solo cambios.
- Los gráficos se solicitan al EA la primera vez que se abren y quedan cacheados en SQLite.
- Si cambias el puerto en Ajustes, actualiza también `InpApiUrl` en el EA.
- La app escucha solo en `127.0.0.1`; no expongas el puerto a la red.

## Simulador MT5

El repositorio incluye un simulador en PowerShell para probar la integración sin MT5. La app debe estar en ejecución.

```powershell
$env:PLAN_TRADING_API_KEY = "tu-api-key"
.\tools\simulate-mt5.ps1 -HeartbeatSeconds 2 -SyncSeconds 5
```

Parámetros disponibles:

| Parámetro | Por defecto | Descripción |
| --- | --- | --- |
| `-ApiKey` | `$env:PLAN_TRADING_API_KEY` | API key de la app |
| `-Port` | `4477` | Puerto del backend |
| `-Login` | `990001` | Login de la cuenta simulada |
| `-Server` | `Simulator-Demo` | Servidor de la cuenta simulada |
| `-OffsetSeconds` | `7200` | Desfase horario del bróker |
| `-HeartbeatSeconds` | `5` | Frecuencia del heartbeat |
| `-SyncSeconds` | `15` | Frecuencia de sincronización |

El simulador crea la cuenta `990001 / Simulator-Demo`, con balance inicial de 10.000 USD, tres operaciones de ejemplo y velas para los gráficos. También responde a los comandos de series que solicite la app.

## Datos y copias de seguridad

Ruta de datos en Windows:

```text
%APPDATA%\com.jairo.plandetrading\
├─ trading-journal.db
├─ media\
└─ backups\
```

- `trading-journal.db`: base de datos SQLite.
- `media\`: imágenes adjuntas al diario y a las operaciones.
- `backups\`: copias creadas desde **Ajustes > Datos y copias > Crear copia de seguridad**.

Las copias se generan con `VACUUM INTO`, por lo que son archivos SQLite completos y consistentes. Para restaurar una copia, cierra la app y sustituye `trading-journal.db` por el archivo de backup.

Para reiniciar la aplicación desde cero, cierra la app y elimina `trading-journal.db` junto con `trading-journal.db-shm` y `trading-journal.db-wal`.

## API local

El backend expone una API HTTP en `127.0.0.1:4477` (configurable). Todas las rutas excepto `health` requieren la cabecera `X-Api-Key`.

| Método | Ruta | Descripción |
| --- | --- | --- |
| `GET` | `/api/v1/health` | Estado del backend (pública) |
| `POST` | `/api/v1/mt5/heartbeat` | Estado de cuenta y conexión del EA |
| `POST` | `/api/v1/mt5/sync` | Posiciones, deals y operaciones |
| `POST` | `/api/v1/mt5/series` | Velas solicitadas por la app |
| `POST` | `/api/v1/mt5/events` | Eventos y logs del EA |
| `POST` | `/api/v1/mt5/commands/result` | Resultado de un comando |

Comprobación rápida:

```powershell
Invoke-RestMethod http://127.0.0.1:4477/api/v1/health
```

## Estructura del proyecto

```text
PlanDeTrading/
├─ backend/
│  ├─ migrations/            Esquema SQLite
│  ├─ src/
│  │  ├─ commands.rs         Comandos Tauri
│  │  ├─ mt5.rs              API HTTP, ingesta y cola de comandos
│  │  ├─ stats.rs            Cálculo de estadísticas
│  │  ├─ services.rs         Lógica de diario, activos e imágenes
│  │  ├─ db.rs               Conexión, migraciones y ajustes
│  │  ├─ models.rs           DTOs compartidos
│  │  └─ state.rs            Estado de la aplicación
│  └─ tauri.conf.json
├─ frontend/
│  └─ src/app/
│     ├─ core/               Modelos, API, store y formateadores
│     ├─ pages/              Dashboard, diario, operaciones, etc.
│     └─ shared/             Componentes reutilizables
├─ mt5/
│  ├─ TradingJournalBridge.mq5
│  └─ TradingJournalBridge.ex5
├─ tools/
│  └─ simulate-mt5.ps1
└─ package.json
```

## Scripts

| Comando | Descripción |
| --- | --- |
| `npm run dev` | Desarrollo completo con Tauri y Angular |
| `npm start` | Solo servidor de desarrollo Angular |
| `npm run build` | Build de producción y instalador NSIS |
| `npm run build:web` | Build del frontend |
| `npm test` | Tests del frontend con Vitest |
| `npm run test:rust` | Tests del backend |
| `npm run check:rust` | Formato y clippy con warnings como errores |
| `npm run fmt:rust` | Formatea el código Rust |

## Modelo de datos

- Una **cuenta** se identifica por `login + servidor` y puede estar `pending`, `active` o `archived`.
- Solo puede haber una cuenta activa a la vez.
- El **diario** se identifica por cuenta y fecha del bróker.
- Cada **activo del día** se identifica por diario y símbolo.
- Cada **operación** se identifica por cuenta y ticket de posición.
- Los **deals** se identifican por cuenta y ticket de deal.
- Las **velas** se cachean por operación y timeframe.
- Los puntos de la estrategia se copian como instantánea al activo del día, de modo que cambios posteriores en la estrategia no alteran el pasado.
- Se guarda la hora del bróker y la hora UTC; los agrupamientos diarios usan la fecha del bróker.

## Solución de problemas

**El EA no conecta**

- Comprueba que la app esté abierta y que el puerto coincida con `InpApiUrl`.
- Verifica la API key en Ajustes y en los parámetros del EA.
- Añade la URL exacta a WebRequest en MT5.
- Activa **Algo Trading** y revisa la pestaña **Expertos** de MT5.
- Comprueba que `127.0.0.1:4477` no esté bloqueado por firewall o antivirus.

**La cuenta aparece pendiente**

- Es el comportamiento esperado para cuentas nuevas. Actívala desde **Cuentas**.

**Los gráficos no aparecen**

- La app solicita las velas al EA y las recibe en la siguiente sincronización (15 s por defecto).
- Comprueba que el EA siga conectado y que el símbolo tenga historial en MT5.
- Si el timeframe ya está cacheado, se muestra al instante.

**El puerto está ocupado**

- Cambia el puerto en **Ajustes**, reinicia la app y actualiza `InpApiUrl` en el EA.

**La interfaz no carga o se ve sin estilos**

- Verifica que WebView2 esté instalado y actualizado.
- En producción, el build de Angular debe mantener `inlineCritical: false` por la política CSP de Tauri.

**Quiero empezar de cero**

- Cierra la app y elimina `trading-journal.db`, `trading-journal.db-shm` y `trading-journal.db-wal` de `%APPDATA%\com.jairo.plandetrading`.

## Estado

La aplicación está verificada de extremo a extremo con el simulador incluido: ingesta de cuentas, operaciones, deals, estadísticas, diario, estrategias, series de velas, backups y compilación del instalador. Para usar datos reales solo falta configurar MT5 y el EA como se indica arriba.
