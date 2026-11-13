//! Area della famiglia: il PFI più recente tra quelli resi visibili, gli allegati visibili e il visto.

use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    response::{Html, Redirect},
    routing::{get, post},
};

use crate::{
    AppState,
    auth::{Famiglia, Modulo, Ruolo, Utente, Vuoto},
    documenti::Contenuto,
    error::AppError,
    routes::pagina,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/famiglia", get(mostra))
        .route("/famiglia/versioni/{id}/visto", post(visto))
}

struct VersioneFamiglia {
    id: i64,
    numero: i64,
    data: String,
    contenuto: Contenuto,
    /// Data del visto già apposto da questo utente.
    visto: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AllegatoFamiglia {
    id: i64,
    tipo: String,
    nome_file: String,
}

struct Figlio {
    nome: String,
    classe: String,
    versione: Option<VersioneFamiglia>,
    allegati: Vec<AllegatoFamiglia>,
}

#[derive(Template)]
#[template(path = "famiglia.html")]
struct FamigliaPagina {
    utente: Utente,
    figli: Vec<Figlio>,
}

/// Ogni query filtra su `famiglia_user_id`: una famiglia raggiunge solo i dati dei propri alunni.
async fn mostra(
    Famiglia(utente): Famiglia,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let alunni: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT a.id, a.nome || ' ' || a.cognome, c.nome || ' (' || c.anno_scolastico || ')'
         FROM alunni a JOIN classi c ON c.id = a.classe_id
         WHERE a.famiglia_user_id = ? ORDER BY a.cognome, a.nome",
    )
    .bind(utente.id)
    .fetch_all(&state.db)
    .await?;

    let mut figli = Vec::new();
    for (alunno_id, nome, classe) in alunni {
        let ultima: Option<(i64, i64, String, String, Option<String>)> = sqlx::query_as(
            "SELECT v.id, v.numero, date(v.creata_il, 'localtime'), v.contenuto_json,
                    (SELECT datetime(vi.data, 'localtime') FROM visti vi
                     WHERE vi.versione_id = v.id AND vi.user_id = ?1)
             FROM versioni v JOIN pfi p ON p.id = v.pfi_id
             WHERE p.alunno_id = ?2 AND v.visibile_famiglia = 1
             ORDER BY v.numero DESC LIMIT 1",
        )
        .bind(utente.id)
        .bind(alunno_id)
        .fetch_optional(&state.db)
        .await?;
        let versione = match ultima {
            Some((id, numero, data, json, visto)) => Some(VersioneFamiglia {
                id,
                numero,
                data,
                contenuto: serde_json::from_str(&json)
                    .map_err(|e| AppError::Interno(e.to_string()))?,
                visto,
            }),
            None => None,
        };
        let allegati = sqlx::query_as(
            "SELECT id, tipo, nome_file FROM allegati
             WHERE alunno_id = ? AND visibile_famiglia = 1 ORDER BY caricato_il DESC, id DESC",
        )
        .bind(alunno_id)
        .fetch_all(&state.db)
        .await?;
        figli.push(Figlio {
            nome,
            classe,
            versione,
            allegati,
        });
    }
    pagina(FamigliaPagina { utente, figli })
}

/// Registra il visto con data e utente; vale solo per una versione visibile del proprio alunno.
async fn visto(
    Famiglia(utente): Famiglia,
    State(state): State<AppState>,
    Path(versione_id): Path<i64>,
    Modulo(_): Modulo<Vuoto>,
) -> Result<Redirect, AppError> {
    let propria: Option<i64> = sqlx::query_scalar(
        "SELECT v.id FROM versioni v
         JOIN pfi p ON p.id = v.pfi_id
         JOIN alunni a ON a.id = p.alunno_id
         WHERE v.id = ? AND a.famiglia_user_id = ? AND v.visibile_famiglia = 1",
    )
    .bind(versione_id)
    .bind(utente.id)
    .fetch_optional(&state.db)
    .await?;
    propria.ok_or(AppError::Vietato)?;
    // Il primo visto resta quello valido: ripetere la richiesta non ne cambia la data.
    sqlx::query("INSERT OR IGNORE INTO visti (versione_id, user_id) VALUES (?, ?)")
        .bind(versione_id)
        .bind(utente.id)
        .execute(&state.db)
        .await?;
    Ok(Redirect::to("/famiglia"))
}
