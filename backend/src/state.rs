use std::path::{Path, PathBuf};
use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

use crate::db;
use crate::error::{AppError, AppResult};
use crate::models::BridgeStatus;
use crate::util::now_iso;

const LEGACY_DATA_DIR_NAME: &str = "com.jairo.plandetrading";

#[derive(Clone)]
pub struct AppState {
    pub app: AppHandle,
    pub db: SqlitePool,
    pub data_dir: PathBuf,
    pub api_key: Arc<RwLock<String>>,
    pub bridge: Arc<BridgeServerManager>,
}

pub struct BridgeServerManager {
    pub mt5: RwLock<BridgeStatus>,
    pub nt8: RwLock<BridgeStatus>,
    pub handle: Mutex<Option<JoinHandle<()>>>,
}

impl AppState {
    pub async fn new(app: &AppHandle) -> AppResult<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        migrate_legacy_data_dir(&data_dir).await?;
        tokio::fs::create_dir_all(&data_dir).await?;

        tokio::fs::create_dir_all(data_dir.join("media").join("thumbs")).await?;

        let db_path = data_dir.join("trading-journal.db");
        let pool = db::init_db(&db_path).await?;
        db::ensure_default_settings(&pool).await?;

        let api_key = db::get_setting(&pool, "api_key").await?.unwrap_or_default();
        let port = db::get_api_port(&pool).await?;

        let mt5_status = BridgeStatus {
            running: false,
            port,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            ..BridgeStatus::default()
        };
        let nt8_status = mt5_status.clone();

        Ok(Self {
            app: app.clone(),
            db: pool,
            data_dir,
            api_key: Arc::new(RwLock::new(api_key)),
            bridge: Arc::new(BridgeServerManager {
                mt5: RwLock::new(mt5_status),
                nt8: RwLock::new(nt8_status),
                handle: Mutex::new(None),
            }),
        })
    }

    pub async fn start_server(&self, app: AppHandle) -> AppResult<BridgeStatus> {
        {
            let mut guard = self.bridge.handle.lock().await;
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        let port = db::get_api_port(&self.db).await?;
        {
            let mut mt5 = self.bridge.mt5.write().await;
            reset_status(&mut mt5, port);
            let mut nt8 = self.bridge.nt8.write().await;
            reset_status(&mut nt8, port);
        }

        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => listener,
            Err(error) => {
                let message = format!(
                    "No se pudo abrir el puerto {port}: {error}. Cambia el puerto en Ajustes."
                );
                let mt5_snapshot = {
                    let mut mt5 = self.bridge.mt5.write().await;
                    mt5.error = Some(message.clone());
                    mt5.clone()
                };
                let nt8_snapshot = {
                    let mut nt8 = self.bridge.nt8.write().await;
                    nt8.error = Some(message);
                    nt8.clone()
                };
                emit_status(&app, "mt5", &mt5_snapshot);
                emit_status(&app, "nt8", &nt8_snapshot);
                return Ok(mt5_snapshot);
            }
        };

        let router = crate::mt5::router(self.clone());
        let state_for_task = self.clone();
        let app_for_task = app.clone();

        let (mt5_snapshot, nt8_snapshot) = {
            let mut mt5 = self.bridge.mt5.write().await;
            mt5.running = true;
            mt5.started_at = Some(now_iso());
            let mut nt8 = self.bridge.nt8.write().await;
            nt8.running = true;
            nt8.started_at = mt5.started_at.clone();
            (mt5.clone(), nt8.clone())
        };
        emit_status(&app, "mt5", &mt5_snapshot);
        emit_status(&app, "nt8", &nt8_snapshot);

        let handle = tauri::async_runtime::spawn(async move {
            log::info!("Servidor de puentes escuchando en 127.0.0.1:{port}");
            if let Err(error) = axum::serve(listener, router).await {
                log::error!("Servidor de puentes detenido: {error}");
                let message = error.to_string();
                let mt5_snapshot = {
                    let mut mt5 = state_for_task.bridge.mt5.write().await;
                    mt5.running = false;
                    mt5.error = Some(message.clone());
                    mt5.clone()
                };
                let nt8_snapshot = {
                    let mut nt8 = state_for_task.bridge.nt8.write().await;
                    nt8.running = false;
                    nt8.error = Some(message);
                    nt8.clone()
                };
                emit_status(&app_for_task, "mt5", &mt5_snapshot);
                emit_status(&app_for_task, "nt8", &nt8_snapshot);
            }
        });

        *self.bridge.handle.lock().await = Some(handle);
        Ok(mt5_snapshot)
    }

    pub async fn restart_server(&self, app: AppHandle) -> AppResult<BridgeStatus> {
        self.start_server(app).await
    }

    pub async fn status(&self) -> BridgeStatus {
        self.bridge.mt5.read().await.clone()
    }

    pub async fn nt8_status(&self) -> BridgeStatus {
        self.bridge.nt8.read().await.clone()
    }

    pub async fn set_api_key(&self, value: String) {
        *self.api_key.write().await = value;
    }

    pub async fn current_api_key(&self) -> String {
        self.api_key.read().await.clone()
    }
}

fn reset_status(status: &mut BridgeStatus, port: u16) {
    status.running = false;
    status.connected = false;
    status.port = port;
    status.error = None;
    status.started_at = None;
}

async fn migrate_legacy_data_dir(data_dir: &Path) -> AppResult<()> {
    let db_path = data_dir.join("trading-journal.db");
    if db_path.exists() {
        return Ok(());
    }
    let Some(parent) = data_dir.parent() else {
        return Ok(());
    };
    let legacy_dir = parent.join(LEGACY_DATA_DIR_NAME);
    let legacy_db = legacy_dir.join("trading-journal.db");
    if !legacy_db.is_file() {
        return Ok(());
    }

    copy_dir_contents(&legacy_dir, data_dir, true).await?;

    let temp_db = data_dir.join("trading-journal.db.migrating");
    tokio::fs::copy(&legacy_db, &temp_db).await?;
    tokio::fs::rename(&temp_db, &db_path).await?;

    log::info!("Datos migrados desde {}", legacy_dir.display());
    Ok(())
}

async fn copy_dir_contents(src: &Path, dst: &Path, skip_root_db: bool) -> AppResult<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut pending = vec![(src.to_path_buf(), dst.to_path_buf(), skip_root_db)];

    while let Some((from, to, skip_db)) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&from).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let name = entry.file_name();
            let target = to.join(&name);

            if file_type.is_dir() {
                tokio::fs::create_dir_all(&target).await?;
                pending.push((entry.path(), target, false));
            } else if file_type.is_file() {
                if skip_db && name == "trading-journal.db" {
                    continue;
                }
                tokio::fs::copy(entry.path(), &target).await?;
            }
        }
    }

    Ok(())
}

fn emit_status(app: &AppHandle, provider: &str, status: &BridgeStatus) {
    let _ = app.emit(&format!("{provider}://status"), status.clone());
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[tokio::test]
    async fn migrates_legacy_data_and_does_not_overwrite_existing_data() {
        let root =
            std::env::temp_dir().join(format!("journal-trading-test-{}", uuid::Uuid::new_v4()));
        let legacy = root.join(LEGACY_DATA_DIR_NAME);
        let current = root.join("com.jairo.journaltrading");

        fs::create_dir_all(legacy.join("media").join("thumbs")).unwrap();
        fs::write(legacy.join("trading-journal.db"), b"legacy-db").unwrap();
        fs::write(legacy.join("trading-journal.db-wal"), b"legacy-wal").unwrap();
        fs::write(
            legacy.join("media").join("thumbs").join("image.png"),
            b"image",
        )
        .unwrap();

        migrate_legacy_data_dir(&current).await.unwrap();

        assert_eq!(
            fs::read(current.join("trading-journal.db")).unwrap(),
            b"legacy-db"
        );
        assert_eq!(
            fs::read(current.join("trading-journal.db-wal")).unwrap(),
            b"legacy-wal"
        );
        assert_eq!(
            fs::read(current.join("media").join("thumbs").join("image.png")).unwrap(),
            b"image"
        );
        assert!(!current.join("trading-journal.db.migrating").exists());

        fs::write(current.join("trading-journal.db"), b"current-db").unwrap();
        migrate_legacy_data_dir(&current).await.unwrap();
        assert_eq!(
            fs::read(current.join("trading-journal.db")).unwrap(),
            b"current-db"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
