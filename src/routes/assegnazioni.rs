//! Assegnazione del modello PFI e del tutor agli alunni, per classe intera o per alunni scelti.

use askama::Template;
use axum::{
    Router,
    extract::{Query, State},
    response::{Html, Redirect},
    routing::get,
};
use serde::Deserialize;

use super::admin::{Classe, elenco_classi};
use crate::{
    AppState,
    auth::{Admin, Modulo, Ruolo, Utente},
    error::AppError,
    routes::pagina,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/assegnazioni", get(mostra).post(assegna))
}

fn richiesta(msg: &str) -> AppError {
    AppError::Richiesta(msg.into())
}

#[derive(Deserialize)]
struct Filtro {
    classe_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct AlunnoAssegnato {
    id: i64,
    cognome: String,
    nome: String,
    modello: Option<String>,
    tutor: Option<String>,
}

#[derive(sqlx::FromRow)]
struct Voce {
    id: i64,
    nome: String,
}

#[derive(Template)]
#[template(path = "admin_assegnazioni.html")]
struct AssegnazioniPagina {
    utente: Utente,
    classi: Vec<Classe>,
    classe_id: i64,
    alunni: Vec<AlunnoAssegnato>,
    modelli: Vec<Voce>,
    docenti: Vec<Voce>,
}

async fn mostra(
    Admin(utente): Admin,
    State(state): State<AppState>,
    Query(filtro): Query<Filtro>,
) -> Result<Html<String>, AppError> {
    let classi = elenco_classi(&state.db).await?;
    // Senza scelta esplicita si parte dalla prima classe; 0 non corrisponde a nessuna classe.
    let classe_id = filtro
        .classe_id
        .or(classi.first().map(|c| c.id))
        .unwrap_or(0);
    let alunni = sqlx::query_as(
        "SELECT a.id, a.cognome, a.nome, m.nome AS modello, t.cognome || ' ' || t.nome AS tutor
         FROM alunni a
         LEFT JOIN modelli m ON m.id = a.modello_id
         LEFT JOIN users t ON t.id = a.tutor_id
         WHERE a.classe_id = ? ORDER BY a.cognome, a.nome",
    )
    .bind(classe_id)
    .fetch_all(&state.db)
    .await?;
    let modelli = sqlx::query_as(
        "SELECT id, nome || ' (' || indirizzo || ')' AS nome FROM modelli ORDER BY indirizzo, nome",
    )
    .fetch_all(&state.db)
    .await?;
    let docenti = sqlx::query_as(
        "SELECT u.id, u.cognome || ' ' || u.nome AS nome
         FROM cdc JOIN users u ON u.id = cdc.docente_id
         WHERE cdc.classe_id = ? ORDER BY u.cognome, u.nome",
    )
    .bind(classe_id)
    .fetch_all(&state.db)
    .await?;
    pagina(AssegnazioniPagina {
        utente,
        classi,
        classe_id,
        alunni,
        modelli,
        docenti,
    })
}

/// Applica modello e/o tutor a tutta la classe (`ambito=classe`) o ai soli alunni spuntati.
/// I campi lasciati vuoti non vengono modificati.
async fn assegna(
    Admin(_): Admin,
    State(state): State<AppState>,
    Modulo(campi): Modulo<Vec<(String, String)>>,
) -> Result<Redirect, AppError> {
    let valore = |nome: &str| {
        campi
            .iter()
            .find(|(k, _)| k == nome)
            .map_or("", |(_, v)| v.trim())
    };
    let intero = |nome: &str| match valore(nome) {
        "" => Ok(None),
        v => v
            .parse::<i64>()
            .map(Some)
            .map_err(|_| richiesta("valore non valido")),
    };
    let classe_id = intero("classe_id")?.ok_or_else(|| richiesta("classe mancante"))?;
    let (modello_id, tutor_id) = (intero("modello_id")?, intero("tutor_id")?);
    if modello_id.is_none() && tutor_id.is_none() {
        return Err(richiesta("scegliere un modello o un tutor da assegnare"));
    }

    let mut tx = state.db.begin().await?;
    let alunni: Vec<i64> = if valore("ambito") == "classe" {
        sqlx::query_scalar("SELECT id FROM alunni WHERE classe_id = ?")
            .bind(classe_id)
            .fetch_all(&mut *tx)
            .await?
    } else {
        campi
            .iter()
            .filter(|(k, _)| k == "alunno_id")
            .map(|(_, v)| v.parse().map_err(|_| richiesta("alunno non valido")))
            .collect::<Result<_, _>>()?
    };
    if alunni.is_empty() {
        return Err(richiesta("nessun alunno selezionato"));
    }

    if let Some(modello_id) = modello_id {
        let esiste: Option<i64> = sqlx::query_scalar("SELECT id FROM modelli WHERE id = ?")
            .bind(modello_id)
            .fetch_optional(&mut *tx)
            .await?;
        esiste.ok_or_else(|| richiesta("modello non trovato"))?;
    }
    if let Some(tutor_id) = tutor_id {
        let nel_cdc: Option<i64> =
            sqlx::query_scalar("SELECT docente_id FROM cdc WHERE classe_id = ? AND docente_id = ?")
                .bind(classe_id)
                .bind(tutor_id)
                .fetch_optional(&mut *tx)
                .await?;
        nel_cdc
            .ok_or_else(|| richiesta("il tutor deve essere un docente del Consiglio di Classe"))?;
    }

    // Il filtro sulla classe in ogni UPDATE ignora gli id di alunni di altre classi.
    for alunno_id in alunni {
        if let Some(modello_id) = modello_id {
            // Le risposte in bozza sono legate agli indicatori del modello con cui il PFI è nato.
            let avviato: Option<i64> =
                sqlx::query_scalar("SELECT id FROM pfi WHERE alunno_id = ? AND modello_id <> ?")
                    .bind(alunno_id)
                    .bind(modello_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if avviato.is_some() {
                return Err(richiesta(
                    "il modello non si può cambiare per gli alunni con un PFI già avviato",
                ));
            }
            sqlx::query("UPDATE alunni SET modello_id = ? WHERE id = ? AND classe_id = ?")
                .bind(modello_id)
                .bind(alunno_id)
                .bind(classe_id)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(tutor_id) = tutor_id {
            sqlx::query("UPDATE alunni SET tutor_id = ? WHERE id = ? AND classe_id = ?")
                .bind(tutor_id)
                .bind(alunno_id)
                .bind(classe_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(Redirect::to(&format!(
        "/admin/assegnazioni?classe_id={classe_id}"
    )))
}
