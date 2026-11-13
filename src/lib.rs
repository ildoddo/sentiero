use std::{str::FromStr, sync::Arc};

use axum::Router;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

pub mod auth;
pub mod config;
pub mod csv_import;
pub mod documenti;
pub mod error;
pub mod modello;
pub mod routes;
pub mod seed;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<config::Config>,
}

/// Apre il database (creandolo se manca) e applica le migrazioni.
pub async fn apri_db(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let opzioni = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true);
    // Un database in memoria esiste per singola connessione: il pool non deve aprirne altre.
    let connessioni = if url.contains(":memory:") { 1 } else { 5 };
    let pool = SqlitePoolOptions::new()
        .max_connections(connessioni)
        .connect_with(opzioni)
        .await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

pub fn app(state: AppState) -> Router {
    routes::router().with_state(state)
}
