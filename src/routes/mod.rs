use askama::Template;
use axum::{
    Router,
    extract::State,
    http::header::CONTENT_TYPE,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};

use crate::{
    AppState,
    auth::{self, Ruolo, Utente},
    error::AppError,
};

mod admin;
mod allegati;
mod assegnazioni;
mod export;
mod famiglia;
mod modelli;
pub mod pfi;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(admin::router())
        .merge(allegati::router())
        .merge(assegnazioni::router())
        .merge(export::router())
        .merge(famiglia::router())
        .merge(modelli::router())
        .merge(pfi::router())
        .route("/", get(home))
        .route("/login", get(auth::login_form).post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/static/style.css", get(css))
        .route("/static/htmx.min.js", get(htmx))
}

pub fn pagina(template: impl Template) -> Result<Html<String>, AppError> {
    Ok(Html(template.render()?))
}

#[derive(sqlx::FromRow)]
struct AlunnoSeguito {
    id: i64,
    cognome: String,
    nome: String,
    classe: String,
    tutor: bool,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomePagina {
    utente: Utente,
    alunni: Vec<AlunnoSeguito>,
    /// Classi esportabili: solo per il dirigente.
    classi: Vec<(i64, String)>,
}

async fn home(utente: Utente, State(state): State<AppState>) -> Result<Response, AppError> {
    if utente.ruolo == Ruolo::Famiglia {
        return Ok(Redirect::to("/famiglia").into_response());
    }
    // Stessa regola di `personale_puo_vedere`: tutti per il dirigente, tutorati e classi del proprio
    // CdC per un docente, nessuno per gli altri ruoli.
    let alunni = sqlx::query_as(
        "SELECT a.id, a.cognome, a.nome, c.nome AS classe, COALESCE(a.tutor_id = ?1, 0) AS tutor
         FROM alunni a JOIN classi c ON c.id = a.classe_id
         WHERE ?2 OR a.tutor_id = ?1 OR EXISTS (
             SELECT 1 FROM cdc WHERE cdc.classe_id = a.classe_id AND cdc.docente_id = ?1)
         ORDER BY c.nome, a.cognome, a.nome",
    )
    .bind(utente.id)
    .bind(utente.ruolo == Ruolo::Dirigente)
    .fetch_all(&state.db)
    .await?;
    let classi = if utente.ruolo == Ruolo::Dirigente {
        sqlx::query_as(
            "SELECT id, nome || ' (' || anno_scolastico || ')' FROM classi ORDER BY anno_scolastico DESC, nome",
        )
        .fetch_all(&state.db)
        .await?
    } else {
        Vec::new()
    };
    Ok(pagina(HomePagina {
        utente,
        alunni,
        classi,
    })?
    .into_response())
}

// File statici incorporati nel binario: nessuna cartella pubblica sul disco.
async fn css() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/style.css"),
    )
}

async fn htmx() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/htmx.min.js"),
    )
}
