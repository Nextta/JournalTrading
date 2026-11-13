//! Instalación de los bridges de NinjaTrader 8 y ATAS X desde la app.
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
}
