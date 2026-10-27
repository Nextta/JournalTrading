use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

use crate::db;
use crate::error::{AppError, AppResult};
use crate::models::Mt5Status;
use crate::util::now_iso;

#[derive(Clone)]
pub struct AppState {
    pub app: AppHandle,
    pub db: SqlitePool,
    pub data_dir: PathBuf,
    pub api_key: Arc<RwLock<String>>,
    pub mt5: Arc<Mt5ServerManager>,
}

pub struct Mt5ServerManager {
    pub status: RwLock<Mt5Status>,
    pub handle: Mutex<Option<JoinHandle<()>>>,
}

impl AppState {
    pub async fn new(app: &AppHandle) -> AppResult<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        tokio::fs::create_dir_all(&data_dir).await?;

        tokio::fs::create_dir_all(data_dir.join("media").join("thumbs")).await?;

        let db_path = data_dir.join("trading-journal.db");
        let pool = db::init_db(&db_path).await?;
        db::ensure_default_settings(&pool).await?;

        let api_key = db::get_setting(&pool, "api_key").await?.unwrap_or_default();
        let port = db::get_api_port(&pool).await?;

        Ok(Self {
            app: app.clone(),
            db: pool,
            data_dir,
            api_key: Arc::new(RwLock::new(api_key)),
            mt5: Arc::new(Mt5ServerManager {
                status: RwLock::new(Mt5Status {
                    running: false,
                    port,
                    app_version: env!("CARGO_PKG_VERSION").to_string(),
                    ..Mt5Status::default()
                }),
                handle: Mutex::new(None),
            }),
        })
    }

    pub async fn start_server(&self, app: AppHandle) -> AppResult<Mt5Status> {
        {
            let mut guard = self.mt5.handle.lock().await;
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        let port = db::get_api_port(&self.db).await?;
        {
            let mut status = self.mt5.status.write().await;
            status.running = false;
            status.connected = false;
            status.port = port;
            status.error = None;
            status.started_at = None;
        }

        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => listener,
            Err(error) => {
                let mut status = self.mt5.status.write().await;
                status.error = Some(format!(
                    "No se pudo abrir el puerto {port}: {error}. Cambia el puerto en Ajustes."
                ));
                let snapshot = status.clone();
                drop(status);
                emit_status(&app, &snapshot);
                return Ok(snapshot);
            }
        };

        let router = crate::mt5::router(self.clone());
        let state_for_task = self.clone();
        let app_for_task = app.clone();

        {
            let mut status = self.mt5.status.write().await;
            status.running = true;
            status.started_at = Some(now_iso());
        }

        let snapshot = self.mt5.status.read().await.clone();
        emit_status(&app, &snapshot);

        let handle = tauri::async_runtime::spawn(async move {
            log::info!("Servidor MT5 escuchando en 127.0.0.1:{port}");
            if let Err(error) = axum::serve(listener, router).await {
                log::error!("Servidor MT5 detenido: {error}");
                let mut status = state_for_task.mt5.status.write().await;
                status.running = false;
                status.error = Some(error.to_string());
                let snapshot = status.clone();
                drop(status);
                emit_status(&app_for_task, &snapshot);
            }
        });

        *self.mt5.handle.lock().await = Some(handle);
        Ok(snapshot)
    }

    pub async fn restart_server(&self, app: AppHandle) -> AppResult<Mt5Status> {
        self.start_server(app).await
    }

    pub async fn status(&self) -> Mt5Status {
        self.mt5.status.read().await.clone()
    }

    pub async fn set_api_key(&self, value: String) {
        *self.api_key.write().await = value;
    }

    pub async fn current_api_key(&self) -> String {
        self.api_key.read().await.clone()
    }
}

fn emit_status(app: &AppHandle, status: &Mt5Status) {
    let _ = app.emit("mt5://status", status.clone());
}
