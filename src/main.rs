use std::sync::Arc;

use sentiero::{AppState, apri_db, auth::token_casuale, config::Config, seed};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let config = Config::da_ambiente();
    std::fs::create_dir_all(&config.data_dir)?;
    let db = apri_db(&config.database_url).await?;

    let utenti: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&db)
        .await?;
    if utenti == 0 {
        let generata = match &config.seed_password {
            Some(_) => None,
            None => Some(token_casuale()?[..16].to_owned()),
        };
        let password = config
            .seed_password
            .as_deref()
            .or(generata.as_deref())
            .unwrap_or_default();
        seed::esegui(&db, password).await?;
        seed::pfi_di_esempio(&db, &config.data_dir).await?;
        if let Some(password) = generata {
            // Su stdout e non nei log: compare una sola volta, alla creazione del database.
            println!("Dati di esempio creati. Password degli utenti di esempio: {password}");
        }
    }

    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(indirizzo = %config.bind, "Sentiero in ascolto");
    let state = AppState {
        db,
        config: Arc::new(config),
    };
    axum::serve(listener, sentiero::app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
