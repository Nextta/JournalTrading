use serde::Serializer;
use serde::ser::Serialize as SerializeTrait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Error de base de datos: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Error de migración: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("No encontrado: {0}")]
    NotFound(String),
    #[error("Datos inválidos: {0}")]
    Validation(String),
    #[error("Error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error interno: {0}")]
    Internal(String),
}

impl SerializeTrait for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
