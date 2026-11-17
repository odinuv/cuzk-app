use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
pub enum CoreError {
    #[error("HTTP chyba {status}: {body}")]
    Http { status: u16, body: String },
    #[error("Chyba sítě: {0}")]
    Network(String),
    #[error("Nenalezeno")]
    NotFound,
    #[error("Nelze určit objekt: {0}")]
    Resolve(String),
    #[error("Chyba databáze: {0}")]
    Db(String),
    #[error("Kontrola již běží")]
    AlreadyRunning,
    #[error("API klíč není nastaven")]
    NoApiKey,
    #[error("API klíč je neplatný")]
    InvalidApiKey,
    #[error("Denní limit volání API byl vyčerpán")]
    Budget,
}
