//! Instalación de los bridges de NinjaTrader 8, ATAS X y MetaTrader 5 desde la app.
//!
//! Los scripts de `tools\` y el código de los bridges se empaquetan como recursos
//! (`bridges\`). Antes de ejecutar un instalador se copian a la carpeta de datos de
//! la app, porque el script compila y escribe junto al código (y la carpeta de
//! instalación puede no tener permisos de escritura).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

static RUNNING: Mutex<()> = Mutex::const_new(());

const TIMEOUT: Duration = Duration::from_secs(600);

/// Archivos que necesita cada instalador, relativos a la raíz de `bridges\`.
const NT8_FILES: &[&str] = &["tools/install-nt8-bridge.ps1", "nt8/JournalTradingBridge.cs"];
const ATAS_FILES: &[&str] = &[
    "tools/install-atas-bridge.ps1",
    "atas/JournalTradingBridge/JournalTradingBridge.csproj",
    "atas/JournalTradingBridge/JournalTradingBridge.cs",
    "atas/JournalTradingBridge/JournalTradingBridge.Copy.cs",
];
const MT5_SOURCE: &str = "mt5/TradingJournalBridge.mq5";
const MT5_EA_NAME: &str = "TradingJournalBridge";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub ok: bool,
    pub message: String,
    pub log: String,
    pub config_path: Option<String>,
}

/// Carpeta con los recursos empaquetados (o la raíz del repositorio en desarrollo).
fn source_root(state: &AppState) -> Option<PathBuf> {
    let bundled = state
        .app
        .path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("bridges"));
    if let Some(dir) = bundled.filter(|dir| dir.join("tools").is_dir()) {
        return Some(dir);
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.to_path_buf();
    repo.join("tools").is_dir().then_some(repo)
}

async fn stage(state: &AppState, files: &[&str]) -> AppResult<PathBuf> {
    let source = source_root(state).ok_or_else(|| {
        AppError::NotFound("No se encontraron los instaladores dentro de la aplicación.".into())
    })?;
    let target = state.data_dir.join("bridges");
    for file in files {
        let from = source.join(file);
        if !from.is_file() {
            return Err(AppError::NotFound(format!(
                "Falta {} en el paquete de la aplicación.",
                from.display()
            )));
        }
        let to = target.join(file);
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::copy(&from, &to).await?;
    }
    Ok(target)
}

fn process_running(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}.exe"), "/NH"])
            .creation_flags(0x0800_0000)
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .to_ascii_lowercase()
                    .contains(&format!("{}.exe", name.to_ascii_lowercase()))
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = name;
        false
    }
}

/// Ejecuta un script de PowerShell oculto y devuelve (éxito, salida).
async fn run_script(script: &Path, args: &[String]) -> AppResult<(bool, String)> {
    let mut command_text = String::from(
        "$ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; & '",
    );
    command_text.push_str(&script.display().to_string().replace('\'', "''"));
    command_text.push('\'');
    for arg in args {
        command_text.push(' ');
        command_text.push_str(arg);
    }

    let mut command = tokio::process::Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command_text,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000);

    let child = command
        .spawn()
        .map_err(|error| AppError::Internal(format!("No se pudo iniciar PowerShell: {error}")))?;
    let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| AppError::Internal("El instalador tardó más de 10 minutos y se canceló.".into()))??;

    let mut log = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        if !log.is_empty() {
            log.push_str("\n\n");
        }
        log.push_str(&stderr);
    }
    Ok((output.status.success(), log))
}

/// Última línea útil de la salida, para mostrar el motivo de un error.
fn error_summary(log: &str) -> String {
    log.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('+') && !line.starts_with("En "))
        .find(|line| !line.starts_with("At ") && !line.contains("CategoryInfo") && !line.contains("FullyQualifiedErrorId"))
        .map(ToString::to_string)
        .unwrap_or_else(|| "El instalador terminó con errores.".into())
}

#[tauri::command]
pub async fn install_nt8_bridge(state: tauri::State<'_, AppState>) -> AppResult<InstallResult> {
    let _guard = RUNNING
        .try_lock()
        .map_err(|_| AppError::Validation("Ya hay una instalación en curso.".into()))?;
    if process_running("NinjaTrader") {
        return Ok(InstallResult {
            ok: false,
            message: "Cierra NinjaTrader 8 y vuelve a pulsar Instalar: el AddOn no se puede compilar con NinjaTrader abierto.".into(),
            log: String::new(),
            config_path: None,
        });
    }
    let root = stage(&state, NT8_FILES).await?;
    let script = root.join("tools").join("install-nt8-bridge.ps1");
    let (ok, log) = run_script(&script, &["-NoConfig".to_string()]).await?;
    if !ok {
        return Ok(InstallResult {
            ok: false,
            message: error_summary(&log.lines().rev().collect::<Vec<_>>().join("\n")),
            log,
            config_path: None,
        });
    }
    let config_path = crate::commands::write_nt8_config_file(&state, false).await?;
    Ok(InstallResult {
        ok: true,
        message: "AddOn de NinjaTrader 8 instalado. Abre NinjaTrader para que empiece a sincronizar.".into(),
        log,
        config_path,
    })
}

#[tauri::command]
pub async fn install_atas_bridge(state: tauri::State<'_, AppState>) -> AppResult<InstallResult> {
    let _guard = RUNNING
        .try_lock()
        .map_err(|_| AppError::Validation("Ya hay una instalación en curso.".into()))?;
    if process_running("OFT.PlatformX") {
        return Ok(InstallResult {
            ok: false,
            message: "Cierra ATAS X y vuelve a pulsar Instalar: ATAS bloquea el archivo del indicador mientras está abierto.".into(),
            log: String::new(),
            config_path: None,
        });
    }
    let root = stage(&state, ATAS_FILES).await?;
    let script = root.join("tools").join("install-atas-bridge.ps1");
    let (ok, log) = run_script(&script, &[]).await?;
    if !ok {
        return Ok(InstallResult {
            ok: false,
            message: error_summary(&log.lines().rev().collect::<Vec<_>>().join("\n")),
            log,
            config_path: None,
        });
    }
    let config_path = crate::commands::write_atas_config_file(&state, false).await?;
    Ok(InstallResult {
        ok: true,
        message: "Indicador de ATAS instalado. En ATAS, añade «JournalTrading Bridge» a un gráfico de cada cuenta.".into(),
        log,
        config_path,
    })
}

/// Terminal de MetaTrader 5 elegido por el usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mt5Terminal {
    /// Carpeta del programa (donde están `terminal64.exe` y `metaeditor64.exe`).
    install_dir: PathBuf,
    /// Carpeta de datos (la que contiene `MQL5`).
    data_dir: PathBuf,
}

/// Lee un archivo de texto de MetaTrader (UTF-16 LE con BOM o UTF-8).
fn read_mt_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return Some(String::from_utf16_lossy(&units));
    }
    let text = String::from_utf8_lossy(&bytes);
    Some(text.trim_start_matches('\u{feff}').to_string())
}

fn normalize_dir(path: &Path) -> String {
    path.to_string_lossy()
        .trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn has_terminal_exe(dir: &Path) -> bool {
    dir.join("terminal64.exe").is_file() || dir.join("terminal.exe").is_file()
}

/// Busca en `terminals_root` (`%APPDATA%\MetaQuotes\Terminal`) la carpeta de datos
/// cuyo `origin.txt` apunta a `install_dir`.
fn find_data_dir(terminals_root: &Path, install_dir: &Path) -> Option<PathBuf> {
    let wanted = normalize_dir(install_dir);
    std::fs::read_dir(terminals_root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|dir| dir.join("MQL5").is_dir())
        .find(|dir| {
            read_mt_text(&dir.join("origin.txt"))
                .is_some_and(|origin| normalize_dir(Path::new(origin.trim())) == wanted)
        })
}

/// Resuelve la carpeta elegida: admite la carpeta de instalación de MT5, su carpeta de
/// datos (Archivo > Abrir carpeta de datos) o la carpeta `MQL5` de esta.
fn resolve_mt5_terminal(selected: &Path, terminals_root: Option<&Path>) -> Result<Mt5Terminal, String> {
    let mut dir = selected.to_path_buf();
    if dir
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("MQL5"))
    {
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        }
    }

    // Carpeta de datos elegida directamente.
    if !has_terminal_exe(&dir) && dir.join("MQL5").is_dir() {
        let origin = read_mt_text(&dir.join("origin.txt"))
            .map(|text| PathBuf::from(text.trim()))
            .filter(|origin| has_terminal_exe(origin));
        return match origin {
            Some(install_dir) => Ok(Mt5Terminal { install_dir, data_dir: dir }),
            None => Err(
                "Esa carpeta de datos no indica dónde está instalado MetaTrader 5. Selecciona la carpeta de instalación (la que contiene terminal64.exe)."
                    .into(),
            ),
        };
    }

    if !has_terminal_exe(&dir) {
        return Err(format!(
            "«{}» no es una instalación de MetaTrader 5: no contiene terminal64.exe. Selecciona la carpeta donde está instalado (por ejemplo C:\\Program Files\\MetaTrader 5).",
            dir.display()
        ));
    }

    if let Some(data_dir) = terminals_root.and_then(|root| find_data_dir(root, &dir)) {
        return Ok(Mt5Terminal { install_dir: dir, data_dir });
    }
    // Instalación portable: los datos están junto al programa.
    if dir.join("MQL5").is_dir() {
        return Ok(Mt5Terminal { install_dir: dir.clone(), data_dir: dir });
    }
    Err(
        "No se encontró la carpeta de datos de este MetaTrader 5. Ábrelo al menos una vez, ciérralo y vuelve a pulsar Instalar."
            .into(),
    )
}

fn mql_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Sustituye el valor por defecto de un `input string` del EA.
fn set_input_default(source: &str, name: &str, value: &str) -> String {
    source
        .split_inclusive('\n')
        .map(|line| {
            let is_input = line
                .trim_start()
                .strip_prefix("input string ")
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix(name))
                .is_some_and(|rest| rest.starts_with([' ', '=']));
            if !is_input {
                return line.to_string();
            }
            let Some(start) = line.find('"') else { return line.to_string() };
            let Some(len) = line[start + 1..].find('"') else { return line.to_string() };
            let end = start + 1 + len;
            format!("{}\"{}\"{}", &line[..start], mql_string(value), &line[end + 1..])
        })
        .collect()
}

/// Número de errores según el log de MetaEditor (`Result: N errors, ...`).
fn compile_errors(log: &str) -> Option<u32> {
    log.lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("Result:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

fn failure(message: impl Into<String>, log: String) -> InstallResult {
    InstallResult {
        ok: false,
        message: message.into(),
        log,
        config_path: None,
    }
}

#[tauri::command]
pub async fn install_mt5_bridge(
    state: tauri::State<'_, AppState>,
    terminal_dir: String,
) -> AppResult<InstallResult> {
    let _guard = RUNNING
        .try_lock()
        .map_err(|_| AppError::Validation("Ya hay una instalación en curso.".into()))?;

    let terminals_root = std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("MetaQuotes").join("Terminal"));
    let terminal = match resolve_mt5_terminal(Path::new(terminal_dir.trim()), terminals_root.as_deref()) {
        Ok(terminal) => terminal,
        Err(message) => return Ok(failure(message, String::new())),
    };
    let Some(editor) = ["metaeditor64.exe", "metaeditor.exe"]
        .iter()
        .map(|name| terminal.install_dir.join(name))
        .find(|path| path.is_file())
    else {
        return Ok(failure(
            format!(
                "No se encontró MetaEditor (metaeditor64.exe) en {}.",
                terminal.install_dir.display()
            ),
            String::new(),
        ));
    };

    let source_dir = source_root(&state).ok_or_else(|| {
        AppError::NotFound("No se encontraron los instaladores dentro de la aplicación.".into())
    })?;
    let source = tokio::fs::read_to_string(source_dir.join(MT5_SOURCE))
        .await
        .map_err(|error| {
            AppError::NotFound(format!("Falta el EA de MT5 en el paquete de la aplicación: {error}"))
        })?;
    let api_port = crate::db::get_api_port(&state.db).await?;
    let api_key = state.current_api_key().await;
    let source = set_input_default(&source, "InpApiUrl", &format!("http://127.0.0.1:{api_port}"));
    let source = set_input_default(&source, "InpApiKey", &api_key);

    let experts = terminal.data_dir.join("MQL5").join("Experts");
    let target = experts.join(format!("{MT5_EA_NAME}.mq5"));
    let compiled = experts.join(format!("{MT5_EA_NAME}.ex5"));
    let write = async {
        tokio::fs::create_dir_all(&experts).await?;
        // Con BOM para que MetaEditor lea el código como UTF-8 y no como ANSI.
        tokio::fs::write(&target, format!("\u{feff}{}", source.trim_start_matches('\u{feff}'))).await?;
        if compiled.exists() {
            tokio::fs::remove_file(&compiled).await?;
        }
        Ok::<_, std::io::Error>(())
    };
    if let Err(error) = write.await {
        let message = if error.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "Sin permisos para escribir en {}. Si MetaTrader es portable y está dentro de Archivos de programa, ejecuta JournalTrading como administrador.",
                experts.display()
            )
        } else {
            format!("No se pudo copiar el EA a {}: {error}", experts.display())
        };
        return Ok(failure(message, String::new()));
    }

    let log_path = state.data_dir.join("bridges").join("mt5-compile.log");
    if let Some(parent) = log_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let _ = tokio::fs::remove_file(&log_path).await;

    let mut command = tokio::process::Command::new(&editor);
    #[cfg(target_os = "windows")]
    {
        // MetaEditor espera /compile:"ruta", con las comillas solo alrededor de la ruta.
        command
            .raw_arg(format!("/compile:\"{}\"", target.display()))
            .raw_arg(format!("/log:\"{}\"", log_path.display()))
            .creation_flags(0x0800_0000);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| AppError::Internal(format!("No se pudo iniciar MetaEditor: {error}")))?;
    if tokio::time::timeout(Duration::from_secs(180), child.wait()).await.is_err() {
        let _ = child.kill().await;
        return Ok(failure("MetaEditor tardó demasiado en compilar y se canceló.", String::new()));
    }

    let log = read_mt_text(&log_path).unwrap_or_default().trim().to_string();
    let errors = compile_errors(&log);
    if errors != Some(0) || !compiled.is_file() {
        let summary = match errors {
            Some(count) if count > 0 => format!("MetaEditor encontró {count} errores al compilar el EA."),
            _ => format!("MetaEditor no generó {MT5_EA_NAME}.ex5."),
        };
        return Ok(failure(
            format!(
                "{summary} El código quedó en {}: ábrelo en MetaEditor y pulsa Compilar (F7) para ver el detalle.",
                target.display()
            ),
            log,
        ));
    }

    let name = terminal
        .install_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "MetaTrader 5".into());
    Ok(InstallResult {
        ok: true,
        message: format!(
            "EA instalado en «{name}» con la API key y el puerto actuales ({}). En MT5, permite WebRequest para http://127.0.0.1:{api_port} y arrastra «{MT5_EA_NAME}» desde Navegador > Asesores expertos a un gráfico. Si ya estaba en un gráfico, se recarga solo.",
            compiled.display()
        ),
        log,
        config_path: None,
    })
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_output_and_error_reason() {
        let dir = std::env::temp_dir().join(format!("jt-installer-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok_script = dir.join("ok.ps1");
        std::fs::write(&ok_script, "Write-Host 'Indicador copiado a destino'\n").unwrap();
        let (ok, log) = run_script(&ok_script, &[]).await.unwrap();
        assert!(ok);
        assert!(log.contains("Indicador copiado"));

        let bad_script = dir.join("bad.ps1");
        std::fs::write(
            &bad_script,
            // Con BOM, como los instaladores reales: PowerShell 5.1 lee los .ps1 sin BOM como ANSI.
            "\u{feff}$ErrorActionPreference = 'Stop'\nWrite-Host 'Compilando'\nthrow 'No se encontró MSBuild.'\n",
        )
        .unwrap();
        let (ok, log) = run_script(&bad_script, &[]).await.unwrap();
        assert!(!ok);
        let reversed = log.lines().rev().collect::<Vec<_>>().join("\n");
        assert_eq!(error_summary(&reversed), "No se encontró MSBuild.");

        let _ = std::fs::remove_dir_all(dir);
    }

    fn utf16(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    #[test]
    fn resolves_install_data_and_portable_folders() {
        let root = std::env::temp_dir().join(format!("jt-mt5-test-{}", uuid::Uuid::new_v4()));
        let install = root.join("Program Files").join("Broker MetaTrader 5");
        let portable = root.join("Portable MT5");
        let terminals = root.join("Terminal");
        let data = terminals.join("ABC123");
        let other = terminals.join("OTHER");
        for dir in [&install, &portable.join("MQL5"), &data.join("MQL5"), &other.join("MQL5")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(install.join("terminal64.exe"), b"").unwrap();
        std::fs::write(portable.join("terminal64.exe"), b"").unwrap();
        std::fs::write(data.join("origin.txt"), utf16(&format!("{}\\", install.display()))).unwrap();
        std::fs::write(other.join("origin.txt"), utf16("C:\\Otro MT5")).unwrap();

        let expected = Mt5Terminal { install_dir: install.clone(), data_dir: data.clone() };
        assert_eq!(resolve_mt5_terminal(&install, Some(&terminals)), Ok(expected.clone()));
        let from_data = resolve_mt5_terminal(&data, Some(&terminals)).unwrap();
        assert_eq!(normalize_dir(&from_data.install_dir), normalize_dir(&install));
        assert_eq!(from_data.data_dir, data);
        assert_eq!(resolve_mt5_terminal(&data.join("MQL5"), Some(&terminals)).unwrap().data_dir, data);
        assert_eq!(
            resolve_mt5_terminal(&portable, Some(&terminals)),
            Ok(Mt5Terminal { install_dir: portable.clone(), data_dir: portable.clone() })
        );
        assert!(resolve_mt5_terminal(&root, Some(&terminals)).is_err());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bakes_inputs_and_reads_compile_result() {
        let source = concat!(
            "input string InpApiUrl           = \"http://127.0.0.1:4477\"; // URL base\r\n",
            "input string InpApiKey           = \"\";                       // API key\r\n",
            "input string InpApiKeyOther = \"x\";\r\n",
        );
        let baked = set_input_default(source, "InpApiUrl", "http://127.0.0.1:5000");
        let baked = set_input_default(&baked, "InpApiKey", "ab\"c");
        assert_eq!(
            baked,
            concat!(
                "input string InpApiUrl           = \"http://127.0.0.1:5000\"; // URL base\r\n",
                "input string InpApiKey           = \"ab\\\"c\";                       // API key\r\n",
                "input string InpApiKeyOther = \"x\";\r\n",
            )
        );

        assert_eq!(compile_errors(" : information: code generated\r\nResult: 0 errors, 0 warnings, 6092 ms elapsed"), Some(0));
        assert_eq!(compile_errors("Result: 3 errors, 1 warnings"), Some(3));
        assert_eq!(compile_errors(""), None);
    }
}
