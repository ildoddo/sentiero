//! Modelli PFI gestiti dall'amministratore.

use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    response::{Html, Redirect},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::sqlite::SqliteQueryResult;

use crate::{
    AppState,
    auth::{Admin, Modulo, Ruolo, Utente},
    error::AppError,
    modello::{self, MAX_PERSONALIZZATE, Sezione},
    routes::pagina,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/modelli", get(elenco).post(crea))
        .route("/admin/modelli/{id}", get(dettaglio).post(aggiorna))
        .route("/admin/sezioni/{id}/attiva", post(attiva_sezione))
        .route(
            "/admin/sezioni/{id}/sottosezioni",
            post(aggiungi_sottosezione),
        )
        .route("/admin/sottosezioni/{id}", post(aggiorna_sottosezione))
        .route(
            "/admin/sottosezioni/{id}/elimina",
            post(elimina_sottosezione),
        )
        .route(
            "/admin/sottosezioni/{id}/indicatori",
            post(aggiungi_indicatore),
        )
        .route("/admin/indicatori/{id}", post(aggiorna_indicatore))
        .route("/admin/indicatori/{id}/elimina", post(elimina_indicatore))
}

fn richiesta(msg: &str) -> AppError {
    AppError::Richiesta(msg.into())
}

fn non_vuoto<'a>(valore: &'a str, campo: &str) -> Result<&'a str, AppError> {
    match valore.trim() {
        "" => Err(AppError::Richiesta(format!(
            "campo obbligatorio mancante: {campo}"
        ))),
        v => Ok(v),
    }
}

/// Le pagine tornano sempre alla scheda del modello: l'id arriva dal form e serve solo a questo.
fn torna(modello_id: i64) -> Redirect {
    Redirect::to(&format!("/admin/modelli/{modello_id}"))
}

fn trovato(esito: SqliteQueryResult) -> Result<(), AppError> {
    if esito.rows_affected() == 0 {
        Err(AppError::NonTrovato)
    } else {
        Ok(())
    }
}

fn nome_duplicato(e: sqlx::Error) -> AppError {
    if e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
    {
        richiesta("esiste già un modello con questo nome")
    } else {
        e.into()
    }
}

#[derive(sqlx::FromRow)]
struct ModelloRiga {
    id: i64,
    nome: String,
    indirizzo: String,
    alunni: i64,
}

#[derive(Template)]
#[template(path = "admin_modelli.html")]
struct ElencoPagina {
    utente: Utente,
    modelli: Vec<ModelloRiga>,
}

async fn elenco(
    Admin(utente): Admin,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let modelli = sqlx::query_as(
        "SELECT m.id, m.nome, m.indirizzo,
                (SELECT COUNT(*) FROM alunni a WHERE a.modello_id = m.id) AS alunni
         FROM modelli m ORDER BY m.indirizzo, m.nome",
    )
    .fetch_all(&state.db)
    .await?;
    pagina(ElencoPagina { utente, modelli })
}

#[derive(Deserialize)]
struct FormModello {
    nome: String,
    indirizzo: String,
}

async fn crea(
    Admin(_): Admin,
    State(state): State<AppState>,
    Modulo(f): Modulo<FormModello>,
) -> Result<Redirect, AppError> {
    let (nome, indirizzo) = (
        non_vuoto(&f.nome, "nome")?,
        non_vuoto(&f.indirizzo, "indirizzo")?,
    );
    let mut tx = state.db.begin().await?;
    let id = modello::crea(&mut tx, nome, indirizzo)
        .await
        .map_err(nome_duplicato)?;
    tx.commit().await?;
    Ok(torna(id))
}

#[derive(Template)]
#[template(path = "admin_modello.html")]
struct DettaglioPagina {
    utente: Utente,
    id: i64,
    nome: String,
    indirizzo: String,
    sezioni: Vec<Sezione>,
    max_personalizzate: usize,
}

async fn dettaglio(
    Admin(utente): Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, AppError> {
    let (nome, indirizzo): (String, String) =
        sqlx::query_as("SELECT nome, indirizzo FROM modelli WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NonTrovato)?;
    let mut conn = state.db.acquire().await?;
    let sezioni = modello::carica(&mut conn, id).await?;
    pagina(DettaglioPagina {
        utente,
        id,
        nome,
        indirizzo,
        sezioni,
        max_personalizzate: MAX_PERSONALIZZATE as usize,
    })
}

async fn aggiorna(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(f): Modulo<FormModello>,
) -> Result<Redirect, AppError> {
    let esito = sqlx::query("UPDATE modelli SET nome = ?, indirizzo = ? WHERE id = ?")
        .bind(non_vuoto(&f.nome, "nome")?)
        .bind(non_vuoto(&f.indirizzo, "indirizzo")?)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(nome_duplicato)?;
    trovato(esito)?;
    Ok(torna(id))
}

#[derive(Deserialize)]
struct FormAttiva {
    modello_id: i64,
    attiva: bool,
}

async fn attiva_sezione(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(f): Modulo<FormAttiva>,
) -> Result<Redirect, AppError> {
    let esito = sqlx::query("UPDATE sezioni SET attiva = ? WHERE id = ?")
        .bind(f.attiva)
        .bind(id)
        .execute(&state.db)
        .await?;
    trovato(esito)?;
    Ok(torna(f.modello_id))
}

#[derive(Deserialize)]
struct FormNuovaSottosezione {
    modello_id: i64,
    titolo: String,
    etichetta: String,
}

/// Aggiunge una sottosezione personalizzata con il suo primo indicatore, fino al massimo per sezione.
async fn aggiungi_sottosezione(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(sezione_id): Path<i64>,
    Modulo(f): Modulo<FormNuovaSottosezione>,
) -> Result<Redirect, AppError> {
    let (titolo, etichetta) = (
        non_vuoto(&f.titolo, "titolo")?,
        non_vuoto(&f.etichetta, "indicatore")?,
    );
    let mut tx = state.db.begin().await?;
    let esiste: Option<i64> = sqlx::query_scalar("SELECT id FROM sezioni WHERE id = ?")
        .bind(sezione_id)
        .fetch_optional(&mut *tx)
        .await?;
    esiste.ok_or(AppError::NonTrovato)?;
    let personalizzate: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sottosezioni WHERE sezione_id = ? AND personalizzata = 1",
    )
    .bind(sezione_id)
    .fetch_one(&mut *tx)
    .await?;
    if personalizzate >= MAX_PERSONALIZZATE {
        return Err(AppError::Richiesta(format!(
            "ogni sezione può avere al massimo {MAX_PERSONALIZZATE} sottosezioni personalizzate"
        )));
    }
    let sottosezione_id: i64 = sqlx::query_scalar(
        "INSERT INTO sottosezioni (sezione_id, titolo, personalizzata, ordine)
         SELECT ?, ?, 1, COALESCE(MAX(ordine), -1) + 1 FROM sottosezioni WHERE sezione_id = ?
         RETURNING id",
    )
    .bind(sezione_id)
    .bind(titolo)
    .bind(sezione_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO indicatori (sottosezione_id, etichetta, ordine) VALUES (?, ?, 0)")
        .bind(sottosezione_id)
        .bind(etichetta)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(torna(f.modello_id))
}

#[derive(Deserialize)]
struct FormSottosezione {
    modello_id: i64,
    titolo: String,
    /// Checkbox: presente solo se spuntata.
    attiva: Option<String>,
}

async fn aggiorna_sottosezione(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(f): Modulo<FormSottosezione>,
) -> Result<Redirect, AppError> {
    let esito = sqlx::query("UPDATE sottosezioni SET titolo = ?, attiva = ? WHERE id = ?")
        .bind(non_vuoto(&f.titolo, "titolo")?)
        .bind(f.attiva.is_some())
        .bind(id)
        .execute(&state.db)
        .await?;
    trovato(esito)?;
    Ok(torna(f.modello_id))
}

#[derive(Deserialize)]
struct FormElimina {
    modello_id: i64,
}

/// Solo le sottosezioni personalizzate si eliminano: quelle della struttura base si disattivano.
async fn elimina_sottosezione(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(f): Modulo<FormElimina>,
) -> Result<Redirect, AppError> {
    let esito = sqlx::query("DELETE FROM sottosezioni WHERE id = ? AND personalizzata = 1")
        .bind(id)
        .execute(&state.db)
        .await?;
    trovato(esito)?;
    Ok(torna(f.modello_id))
}

#[derive(Deserialize)]
struct FormIndicatore {
    modello_id: i64,
    etichetta: String,
    #[serde(default)]
    testo_proposto: String,
}

async fn aggiungi_indicatore(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(sottosezione_id): Path<i64>,
    Modulo(f): Modulo<FormIndicatore>,
) -> Result<Redirect, AppError> {
    let esito = sqlx::query(
        "INSERT INTO indicatori (sottosezione_id, etichetta, testo_proposto, ordine)
         SELECT ss.id, ?, ?, (SELECT COALESCE(MAX(ordine), -1) + 1 FROM indicatori WHERE sottosezione_id = ss.id)
         FROM sottosezioni ss WHERE ss.id = ?",
    )
    .bind(non_vuoto(&f.etichetta, "indicatore")?)
    .bind(f.testo_proposto.trim())
    .bind(sottosezione_id)
    .execute(&state.db)
    .await?;
    trovato(esito)?;
    Ok(torna(f.modello_id))
}

async fn aggiorna_indicatore(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(f): Modulo<FormIndicatore>,
) -> Result<Redirect, AppError> {
    let esito = sqlx::query("UPDATE indicatori SET etichetta = ?, testo_proposto = ? WHERE id = ?")
        .bind(non_vuoto(&f.etichetta, "indicatore")?)
        .bind(f.testo_proposto.trim())
        .bind(id)
        .execute(&state.db)
        .await?;
    trovato(esito)?;
    Ok(torna(f.modello_id))
}

async fn elimina_indicatore(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(f): Modulo<FormElimina>,
) -> Result<Redirect, AppError> {
    // Ogni sottosezione conserva almeno un indicatore.
    let esito = sqlx::query(
        "DELETE FROM indicatori WHERE id = ?1
         AND (SELECT COUNT(*) FROM indicatori i
              WHERE i.sottosezione_id = (SELECT sottosezione_id FROM indicatori WHERE id = ?1)) > 1",
    )
    .bind(id)
    .execute(&state.db)
    .await?;
    if esito.rows_affected() == 0 {
        return Err(richiesta(
            "indicatore non trovato, oppure è l'unico della sottosezione",
        ));
    }
    Ok(torna(f.modello_id))
}
