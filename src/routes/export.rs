//! Conservazione di fine anno: archivio zip dei PFI approvati dal CdC e vistati dalla famiglia.

use std::io::{Cursor, Write};

use axum::{
    Router,
    extract::{Query, State},
    http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use zip::{ZipWriter, write::SimpleFileOptions};

use crate::{AppState, auth::Dirigente, error::AppError};

pub fn router() -> Router<AppState> {
    Router::new().route("/export", get(esporta))
}

fn richiesta(msg: &str) -> AppError {
    AppError::Richiesta(msg.into())
}

fn interno(e: impl std::fmt::Display) -> AppError {
    AppError::Interno(e.to_string())
}

#[derive(Deserialize)]
struct Filtro {
    alunno_id: Option<i64>,
    classe_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct Riga {
    alunno_id: i64,
    classe: String,
    anno_scolastico: String,
    cognome: String,
    nome: String,
    numero: i64,
    motivo: String,
    pdf_path: String,
    odt_path: String,
    approvata_il: String,
    approvata_da: String,
    visti: String,
}

/// Solo lettere, cifre, `-` e `_`: i nomi finiscono nei percorsi dentro l'archivio.
fn pulisci(testo: &str) -> String {
    testo
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Esporta le versioni approvate e vistate di un alunno (`alunno_id`) o di una classe (`classe_id`),
/// con un indice che riporta approvazione e visti, altrimenti presenti solo nel database.
async fn esporta(
    Dirigente(_): Dirigente,
    State(state): State<AppState>,
    Query(f): Query<Filtro>,
) -> Result<Response, AppError> {
    let nome_archivio = match (f.alunno_id, f.classe_id) {
        (Some(id), None) => format!("sentiero-pfi-alunno-{id}.zip"),
        (None, Some(id)) => format!("sentiero-pfi-classe-{id}.zip"),
        _ => return Err(richiesta("indicare un alunno oppure una classe")),
    };
    let righe: Vec<Riga> = sqlx::query_as(
        "SELECT a.id AS alunno_id, c.nome AS classe, c.anno_scolastico, a.cognome, a.nome,
                v.numero, v.motivo, v.pdf_path, v.odt_path,
                datetime(v.approvata_il, 'localtime') AS approvata_il,
                ap.nome || ' ' || ap.cognome AS approvata_da,
                (SELECT group_concat(datetime(vi.data, 'localtime') || ' ' || u.nome || ' ' || u.cognome, '; ')
                 FROM visti vi JOIN users u ON u.id = vi.user_id WHERE vi.versione_id = v.id) AS visti
         FROM versioni v
         JOIN pfi p ON p.id = v.pfi_id
         JOIN alunni a ON a.id = p.alunno_id
         JOIN classi c ON c.id = a.classe_id
         JOIN users ap ON ap.id = v.approvata_da
         WHERE v.approvata_il IS NOT NULL
           AND EXISTS (SELECT 1 FROM visti vi WHERE vi.versione_id = v.id)
           AND (a.id = ?1 OR a.classe_id = ?2)
         ORDER BY a.cognome, a.nome, a.id, v.numero",
    )
    .bind(f.alunno_id)
    .bind(f.classe_id)
    .fetch_all(&state.db)
    .await?;
    if righe.is_empty() {
        return Err(richiesta("nessun PFI approvato e vistato da esportare"));
    }

    let opzioni = SimpleFileOptions::default();
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let mut indice = csv::WriterBuilder::new()
        .delimiter(b';')
        .from_writer(Vec::new());
    indice
        .write_record([
            "classe",
            "anno_scolastico",
            "cognome",
            "nome",
            "versione",
            "motivo",
            "approvata_il",
            "approvazione_registrata_da",
            "visti",
            "cartella",
        ])
        .map_err(interno)?;
    for r in &righe {
        // L'id distingue due alunni omonimi della stessa classe.
        let cartella = format!(
            "{}/{}_{}_{}",
            pulisci(&r.classe),
            pulisci(&r.cognome),
            pulisci(&r.nome),
            r.alunno_id
        );
        for (path, estensione) in [(&r.pdf_path, "pdf"), (&r.odt_path, "odt")] {
            // Un archivio di conservazione incompleto sarebbe peggio di un errore esplicito.
            let dati = tokio::fs::read(state.config.data_dir.join(path)).await.map_err(|_| {
                AppError::Richiesta(format!(
                    "documento {estensione} mancante per la versione {} di {} {}: generare una nuova versione",
                    r.numero, r.cognome, r.nome
                ))
            })?;
            zip.start_file(
                format!("{cartella}/PFI-v{}.{estensione}", r.numero),
                opzioni,
            )
            .map_err(interno)?;
            zip.write_all(&dati)?;
        }
        indice
            .write_record([
                &r.classe,
                &r.anno_scolastico,
                &r.cognome,
                &r.nome,
                &r.numero.to_string(),
                &r.motivo,
                &r.approvata_il,
                &r.approvata_da,
                &r.visti,
                &cartella,
            ])
            .map_err(interno)?;
    }
    zip.start_file("indice.csv", opzioni).map_err(interno)?;
    zip.write_all(&indice.into_inner().map_err(interno)?)?;
    let archivio = zip.finish().map_err(interno)?.into_inner();

    Ok((
        [
            (CONTENT_TYPE, "application/zip".to_owned()),
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{nome_archivio}\""),
            ),
            (CACHE_CONTROL, "no-store".to_owned()),
        ],
        archivio,
    )
        .into_response())
}
