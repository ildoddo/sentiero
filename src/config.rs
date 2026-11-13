use std::{env, path::PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    pub database_url: String,
    /// Cartella di database, documenti generati e allegati: mai servita come statica.
    pub data_dir: PathBuf,
    pub cookie_secure: bool,
    pub seed_password: Option<String>,
}

impl Config {
    pub fn da_ambiente() -> Self {
        let data_dir =
            PathBuf::from(env::var("SENTIERO_DATA_DIR").unwrap_or_else(|_| "data".into()));
        Self {
            bind: env::var("SENTIERO_BIND").unwrap_or_else(|_| "127.0.0.1:3000".into()),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| format!("sqlite://{}/sentiero.db", data_dir.display())),
            data_dir,
            cookie_secure: env::var("SENTIERO_COOKIE_SECURE")
                .is_ok_and(|v| v == "true" || v == "1"),
            seed_password: env::var("SENTIERO_SEED_PASSWORD")
                .ok()
                .filter(|p| !p.is_empty()),
        }
    }
}
