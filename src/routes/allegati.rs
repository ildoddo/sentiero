//! Allegati dell'alunno (PDP, PEI, UdA) pubblicati dal tutor.

use axum::{
    Router,
    extract::{DefaultBodyLimit, Path, State},
    http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;

use crate::{
    AppState,
    auth::{
        Caricamento, Docente, Modulo, Ruolo, Utente, Vuoto, personale_puo_vedere, richiedi_tutor,
        token_casuale,
    },
    error::AppError,
};

pub const DIMENSIONE_MASSIMA: usize = 10 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/alunni/{id}/allegati",
            // Il margine oltre il file copre gli altri campi e i separatori del multipart.
            post(carica).layer(DefaultBodyLimit::max(DIMENSIONE_MASSIMA + 64 * 1024)),
        )
        .route("/allegati/{id}", get(scarica))
        .route("/allegati/{id}/elimina", post(elimina))
        .route("/allegati/{id}/visibilita", post(visibilita))
}

fn richiesta(msg: &str) -> AppError {
    AppError::Richiesta(msg.into())
}

/// Estensione e tipo MIME, accettati solo se il contenuto inizia con la firma del formato:
/// l'estensione e il tipo dichiarati dal browser non bastano.
fn tipo_file(nome: &str, dati: &[u8]) -> Option<(&'static str, &'static str)> {
    const ZIP: &[u8] = b"PK\x03\x04";
    let estensione = nome.rsplit_once('.')?.1.to_ascii_lowercase();
    match estensione.as_str() {
        "pdf" if dati.starts_with(b"%PDF-") => Some(("pdf", "application/pdf")),
        "odt" if dati.starts_with(ZIP) => Some(("odt", "application/vnd.oasis.opendocument.text")),
        "docx" if dati.starts_with(ZIP) => Some((
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )),
        _ => None,
    }
}

/// Nome mostrato in elenco: solo il nome del file, senza percorsi né caratteri speciali.
/// Sul disco il file ha comunque un nome casuale che non dipende da questo.
fn nome_sicuro(nome: &str) -> String {
    let base = nome.rsplit(['/', '\\']).next().unwrap_or_default();
    let pulito: String = base
        .chars()
        .filter(|c| c.is_alphanumeric() || " ._-()".contains(*c))
        .take(100)
        .collect();
    if pulito.trim_matches(['.', ' ']).is_empty() {
        "allegato".into()
    } else {
        pulito
    }
}

async fn carica(
    Docente(utente): Docente,
    State(state): State<AppState>,
    Path(alunno_id): Path<i64>,
    caricamento: Caricamento,
) -> Result<Redirect, AppError> {
    richiedi_tutor(&state.db, &utente, alunno_id).await?;
    let tipo = caricamento.campi.get("tipo").map_or("", String::as_str);
    if !matches!(tipo, "PDP" | "PEI" | "UdA") {
        return Err(richiesta("tipo di allegato non valido: PDP, PEI o UdA"));
    }
    let file = caricamento
        .file
        .ok_or_else(|| richiesta("nessun file selezionato"))?;
    if file.dati.len() > DIMENSIONE_MASSIMA {
        return Err(richiesta("il file supera la dimensione massima di 10 MB"));
    }
    let (estensione, mime) = tipo_file(&file.nome, &file.dati)
        .ok_or_else(|| richiesta("sono ammessi solo file PDF, ODT e DOCX"))?;

    let path = format!("allegati/{}.{estensione}", token_casuale()?);
    let completo = state.config.data_dir.join(&path);
    tokio::fs::create_dir_all(state.config.data_dir.join("allegati")).await?;
    tokio::fs::write(&completo, &file.dati).await?;
    let esito = sqlx::query(
        "INSERT INTO allegati (alunno_id, tipo, nome_file, path, mime, dimensione, caricato_da)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(alunno_id)
    .bind(tipo)
    .bind(nome_sicuro(&file.nome))
    .bind(&path)
    .bind(mime)
    .bind(file.dati.len() as i64)
    .bind(utente.id)
    .execute(&state.db)
    .await;
    if let Err(e) = esito {
        // Senza la riga nel database il file resterebbe orfano sul disco.
        let _ = tokio::fs::remove_file(&completo).await;
        return Err(e.into());
    }
    Ok(Redirect::to(&format!("/alunni/{alunno_id}/pfi")))
}

#[derive(sqlx::FromRow)]
struct AllegatoFile {
    alunno_id: i64,
    famiglia_user_id: Option<i64>,
    visibile_famiglia: bool,
    path: String,
    mime: String,
}

async fn trova(state: &AppState, id: i64) -> Result<AllegatoFile, AppError> {
    sqlx::query_as(
        "SELECT al.alunno_id, a.famiglia_user_id, al.visibile_famiglia, al.path, al.mime
         FROM allegati al JOIN alunni a ON a.id = al.alunno_id
         WHERE al.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NonTrovato)
}

/// Lo scaricano il personale che segue l'alunno e, solo se l'allegato è stato reso visibile, la famiglia.
async fn scarica(
    utente: Utente,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let a = trova(&state, id).await?;
    let famiglia = utente.ruolo == Ruolo::Famiglia
        && a.famiglia_user_id == Some(utente.id)
        && a.visibile_famiglia;
    if !famiglia && !personale_puo_vedere(&state.db, &utente, a.alunno_id).await? {
        return Err(AppError::Vietato);
    }
    let dati = match tokio::fs::read(state.config.data_dir.join(&a.path)).await {
        Ok(dati) => dati,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(AppError::NonTrovato),
        Err(e) => return Err(e.into()),
    };
    let estensione = a.path.rsplit_once('.').map_or("", |(_, e)| e);
    Ok((
        [
            (CONTENT_TYPE, a.mime),
            // Sempre come download e con un nome generato: il nome originale non entra negli header.
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"allegato-{id}.{estensione}\""),
            ),
            (X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (CACHE_CONTROL, "no-store".to_owned()),
        ],
        dati,
    )
        .into_response())
}

/// Il tutor può ritirare un allegato, per esempio un documento caricato sull'alunno sbagliato.
async fn elimina(
    Docente(utente): Docente,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(_): Modulo<Vuoto>,
) -> Result<Redirect, AppError> {
    let a = trova(&state, id).await?;
    richiedi_tutor(&state.db, &utente, a.alunno_id).await?;
    sqlx::query("DELETE FROM allegati WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    match tokio::fs::remove_file(state.config.data_dir.join(&a.path)).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    Ok(Redirect::to(&format!("/alunni/{}/pfi", a.alunno_id)))
}

#[derive(Deserialize)]
struct FormVisibile {
    visibile: bool,
}

/// Il tutor sceglie, allegato per allegato, se la famiglia può vederlo.
async fn visibilita(
    Docente(utente): Docente,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Modulo(f): Modulo<FormVisibile>,
) -> Result<Redirect, AppError> {
    let a = trova(&state, id).await?;
    richiedi_tutor(&state.db, &utente, a.alunno_id).await?;
    sqlx::query("UPDATE allegati SET visibile_famiglia = ? WHERE id = ?")
        .bind(f.visibile)
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(Redirect::to(&format!("/alunni/{}/pfi", a.alunno_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tipo_dal_contenuto() {
        assert_eq!(
            tipo_file("Piano.PDF", b"%PDF-1.7 ...").map(|t| t.0),
            Some("pdf")
        );
        assert_eq!(
            tipo_file("piano.odt", b"PK\x03\x04...").map(|t| t.0),
            Some("odt")
        );
        assert_eq!(
            tipo_file("piano.docx", b"PK\x03\x04...").map(|t| t.0),
            Some("docx")
        );
        assert!(tipo_file("virus.pdf", b"MZ\x90\x00").is_none());
        assert!(tipo_file("piano.pdf", b"PK\x03\x04").is_none());
        assert!(tipo_file("script.html", b"%PDF-").is_none());
        assert!(tipo_file("senza_estensione", b"%PDF-").is_none());
    }

    #[test]
    fn nome_ripulito() {
        assert_eq!(nome_sicuro("../../etc/passwd.pdf"), "passwd.pdf");
        assert_eq!(
            nome_sicuro("C:\\Documenti\\PDP \"Rossi\".pdf"),
            "PDP Rossi.pdf"
        );
        assert_eq!(nome_sicuro("piano<script>.pdf"), "pianoscript.pdf");
        assert_eq!(nome_sicuro("/"), "allegato");
        assert_eq!(nome_sicuro(".."), "allegato");
        assert_eq!(nome_sicuro(&"x".repeat(300)).len(), 100);
    }
}
