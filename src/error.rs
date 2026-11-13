use axum::{
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("non autenticato")]
    NonAutenticato,
    #[error("accesso negato")]
    Vietato,
    #[error("risorsa non trovata")]
    NonTrovato,
    #[error("richiesta non valida: {0}")]
    Richiesta(String),
    #[error("errore del database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("errore di template: {0}")]
    Template(#[from] askama::Error),
    #[error("errore di I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("errore interno: {0}")]
    Interno(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::NonAutenticato => Redirect::to("/login").into_response(),
            Self::Vietato => (StatusCode::FORBIDDEN, "Accesso negato").into_response(),
            Self::NonTrovato => (StatusCode::NOT_FOUND, "Risorsa non trovata").into_response(),
            Self::Richiesta(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            Self::Db(_) | Self::Template(_) | Self::Io(_) | Self::Interno(_) => {
                // Solo il tipo di errore: i messaggi non contengono valori dei campi degli alunni.
                tracing::error!(errore = %self, "errore interno");
                (StatusCode::INTERNAL_SERVER_ERROR, "Errore interno").into_response()
            }
        }
    }
}
