//! Compilazione del PFI da parte del tutor e creazione delle versioni numerate.

use std::collections::HashMap;

use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    http::{
        HeaderMap,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::SqliteConnection;

use crate::{
    AppState,
    auth::{Docente, Modulo, Ruolo, Utente, Vuoto, personale_puo_vedere, richiedi_tutor},
    documenti::{Contenuto, Documento, SezioneDoc, SottosezioneDoc, Voce, odt, pdf},
    error::AppError,
    modello,
    routes::pagina,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/alunni/{id}/pfi", get(consulta))
        .route("/alunni/{id}/pfi/modifica", get(modifica).post(salva))
        .route("/alunni/{id}/pfi/versioni", post(crea_versione))
        .route("/versioni/{id}/approva", post(approva))
        .route("/versioni/{id}/visibilita", post(visibilita))
        .route("/versioni/{id}/{formato}", get(scarica))
}

fn richiesta(msg: &str) -> AppError {
    AppError::Richiesta(msg.into())
}

#[derive(sqlx::FromRow)]
struct Testata {
    cognome: String,
    nome: String,
    data_nascita: String,
    classe: String,
    anno_scolastico: String,
    indirizzo: String,
    modello_id: Option<i64>,
    modello: Option<String>,
    tutor: Option<String>,
}

/// Stato attuale della bozza: sezioni e sottosezioni attive del modello, con il testo salvato
/// dal tutor oppure, dove manca, il contenuto proposto dal modello.
async fn componi(conn: &mut SqliteConnection, alunno_id: i64) -> Result<Contenuto, AppError> {
    let t: Testata = sqlx::query_as(
        "SELECT a.cognome, a.nome, a.data_nascita, c.nome AS classe, c.anno_scolastico, c.indirizzo,
                a.modello_id, m.nome AS modello, t.nome || ' ' || t.cognome AS tutor
         FROM alunni a
         JOIN classi c ON c.id = a.classe_id
         LEFT JOIN modelli m ON m.id = a.modello_id
         LEFT JOIN users t ON t.id = a.tutor_id
         WHERE a.id = ?",
    )
    .bind(alunno_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(AppError::NonTrovato)?;
    let modello_id = t
        .modello_id
        .ok_or_else(|| richiesta("all'alunno non è stato assegnato un modello PFI"))?;

    let risposte: HashMap<i64, String> = sqlx::query_as(
        "SELECT r.indicatore_id, r.testo FROM risposte r JOIN pfi p ON p.id = r.pfi_id WHERE p.alunno_id = ?",
    )
    .bind(alunno_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();

    let sezioni = modello::carica(conn, modello_id)
        .await?
        .into_iter()
        .filter(|s| s.attiva)
        .map(|s| SezioneDoc {
            codice: s.codice,
            titolo: s.titolo,
            sottosezioni: s
                .sottosezioni
                .into_iter()
                .filter(|ss| ss.attiva)
                .map(|ss| SottosezioneDoc {
                    titolo: ss.titolo,
                    voci: ss
                        .indicatori
                        .into_iter()
                        .map(|i| Voce {
                            indicatore_id: i.id,
                            testo: risposte.get(&i.id).cloned().unwrap_or(i.testo_proposto),
                            etichetta: i.etichetta,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();

    Ok(Contenuto {
        alunno: format!("{} {}", t.cognome, t.nome),
        data_nascita: t.data_nascita,
        classe: t.classe,
        anno_scolastico: t.anno_scolastico,
        indirizzo: t.indirizzo,
        modello: t.modello.unwrap_or_default(),
        tutor: t.tutor.unwrap_or_default(),
        sezioni,
    })
}

#[derive(sqlx::FromRow)]
struct VersioneRiga {
    id: i64,
    numero: i64,
    motivo: String,
    creata_il: String,
    autore: String,
    approvata: bool,
    visibile_famiglia: bool,
}

#[derive(Template)]
#[template(path = "pfi_modifica.html")]
struct ModificaPagina {
    utente: Utente,
    alunno_id: i64,
    pfi: Contenuto,
    versioni: Vec<VersioneRiga>,
    prossima: i64,
}

async fn modifica(
    Docente(utente): Docente,
    State(state): State<AppState>,
    Path(alunno_id): Path<i64>,
) -> Result<Html<String>, AppError> {
    richiedi_tutor(&state.db, &utente, alunno_id).await?;
    let mut conn = state.db.acquire().await?;
    let pfi = componi(&mut conn, alunno_id).await?;
    let versioni: Vec<VersioneRiga> = sqlx::query_as(
        "SELECT v.id, v.numero, v.motivo, datetime(v.creata_il, 'localtime') AS creata_il,
                u.nome || ' ' || u.cognome AS autore,
                v.approvata_il IS NOT NULL AS approvata, v.visibile_famiglia
         FROM versioni v
         JOIN pfi p ON p.id = v.pfi_id
         JOIN users u ON u.id = v.creata_da
         WHERE p.alunno_id = ? ORDER BY v.numero DESC",
    )
    .bind(alunno_id)
    .fetch_all(&mut *conn)
    .await?;
    let prossima = versioni.first().map_or(1, |v| v.numero + 1);
    pagina(ModificaPagina {
        utente,
        alunno_id,
        pfi,
        versioni,
        prossima,
    })
}

/// Salva i campi `r_<id indicatore>` nella bozza, creando il PFI al primo salvataggio.
/// Gli indicatori che non appartengono al modello dell'alunno vengono ignorati.
pub async fn salva_risposte(
    conn: &mut SqliteConnection,
    alunno_id: i64,
    campi: &[(String, String)],
) -> Result<i64, AppError> {
    let modello_id: Option<i64> = sqlx::query_scalar("SELECT modello_id FROM alunni WHERE id = ?")
        .bind(alunno_id)
        .fetch_one(&mut *conn)
        .await?;
    let modello_id =
        modello_id.ok_or_else(|| richiesta("all'alunno non è stato assegnato un modello PFI"))?;
    // L'UPDATE che non cambia nulla serve a ottenere l'id anche quando il PFI esiste già.
    let pfi_id: i64 = sqlx::query_scalar(
        "INSERT INTO pfi (alunno_id, modello_id) VALUES (?, ?)
         ON CONFLICT (alunno_id) DO UPDATE SET modello_id = modello_id RETURNING id",
    )
    .bind(alunno_id)
    .bind(modello_id)
    .fetch_one(&mut *conn)
    .await?;
    for (campo, testo) in campi {
        let Some(indicatore_id) = campo
            .strip_prefix("r_")
            .and_then(|id| id.parse::<i64>().ok())
        else {
            continue;
        };
        sqlx::query(
            "INSERT INTO risposte (pfi_id, indicatore_id, testo)
             SELECT p.id, i.id, ?
             FROM pfi p, indicatori i
             JOIN sottosezioni ss ON ss.id = i.sottosezione_id
             JOIN sezioni s ON s.id = ss.sezione_id
             WHERE p.id = ? AND i.id = ? AND s.modello_id = p.modello_id
             ON CONFLICT (pfi_id, indicatore_id) DO UPDATE SET testo = excluded.testo",
        )
        // I browser inviano i ritorni a capo delle textarea come CRLF.
        .bind(testo.replace("\r\n", "\n").trim())
        .bind(pfi_id)
        .bind(indicatore_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(pfi_id)
}

async fn salva(
    Docente(utente): Docente,
    State(state): State<AppState>,
    Path(alunno_id): Path<i64>,
    headers: HeaderMap,
    Modulo(campi): Modulo<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    richiedi_tutor(&state.db, &utente, alunno_id).await?;
    let mut tx = state.db.begin().await?;
    salva_risposte(&mut tx, alunno_id, &campi).await?;
    tx.commit().await?;
    // Con HTMX la pagina resta dov'è e riceve solo la conferma; senza JavaScript si ricarica.
    if headers.contains_key("hx-request") {
        let ora: String = sqlx::query_scalar("SELECT strftime('%H:%M:%S', 'now', 'localtime')")
            .fetch_one(&state.db)
            .await?;
        return Ok(Html(format!("Bozza salvata alle {ora}.")).into_response());
    }
    Ok(Redirect::to(&format!("/alunni/{alunno_id}/pfi/modifica")).into_response())
}

/// Congela lo stato attuale della bozza in una nuova versione numerata e ne scrive PDF e ODT.
/// Va chiamata dentro una transazione; restituisce il numero della versione.
pub async fn congela_versione(
    conn: &mut SqliteConnection,
    cartella: &std::path::Path,
    alunno_id: i64,
    pfi_id: i64,
    motivo: &str,
    creata_da: i64,
) -> Result<i64, AppError> {
    let contenuto = componi(conn, alunno_id).await?;
    let json = serde_json::to_string(&contenuto).map_err(|e| AppError::Interno(e.to_string()))?;
    let numero: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(numero), 0) + 1 FROM versioni WHERE pfi_id = ?")
            .bind(pfi_id)
            .fetch_one(&mut *conn)
            .await?;
    // Percorsi relativi alla cartella dati, che così può essere spostata.
    let pdf_path = format!("documenti/pfi{pfi_id}-v{numero}.pdf");
    let odt_path = format!("documenti/pfi{pfi_id}-v{numero}.odt");
    let data: String = sqlx::query_scalar(
        "INSERT INTO versioni (pfi_id, numero, motivo, creata_da, contenuto_json, pdf_path, odt_path)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING date(creata_il, 'localtime')",
    )
    .bind(pfi_id)
    .bind(numero)
    .bind(motivo)
    .bind(creata_da)
    .bind(json)
    .bind(&pdf_path)
    .bind(&odt_path)
    .fetch_one(&mut *conn)
    .await?;

    // I file si scrivono prima del commit del chiamante: se la scrittura fallisce la versione non resta registrata.
    let documento = Documento {
        contenuto: &contenuto,
        numero,
        motivo,
        data: &data,
    };
    let pdf = pdf::genera(&documento);
    let odt = odt::genera(&documento).map_err(|e| AppError::Interno(e.to_string()))?;
    tokio::fs::create_dir_all(cartella.join("documenti")).await?;
    tokio::fs::write(cartella.join(&pdf_path), pdf).await?;
    tokio::fs::write(cartella.join(&odt_path), odt).await?;
    Ok(numero)
}

/// Salva la bozza inviata e la congela in una nuova versione numerata.
async fn crea_versione(
    Docente(utente): Docente,
    State(state): State<AppState>,
    Path(alunno_id): Path<i64>,
    Modulo(campi): Modulo<Vec<(String, String)>>,
) -> Result<Redirect, AppError> {
    richiedi_tutor(&state.db, &utente, alunno_id).await?;
    let motivo = campi
        .iter()
        .find(|(k, _)| k == "motivo")
        .map_or("", |(_, v)| v);
    if !matches!(motivo, "revisione" | "scrutinio") {
        return Err(richiesta("motivo non valido: revisione o scrutinio"));
    }

    let mut tx = state.db.begin().await?;
    let pfi_id = salva_risposte(&mut tx, alunno_id, &campi).await?;
    congela_versione(
        &mut tx,
        &state.config.data_dir,
        alunno_id,
        pfi_id,
        motivo,
        utente.id,
    )
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/alunni/{alunno_id}/pfi/modifica")))
}

#[derive(sqlx::FromRow)]
struct VersioneFile {
    alunno_id: i64,
    famiglia_user_id: Option<i64>,
    numero: i64,
    visibile_famiglia: bool,
    pdf_path: String,
    odt_path: String,
}

/// Scarica il PDF o l'ODT di una versione. Lo vedono il personale che segue l'alunno e,
/// solo se la versione è stata resa visibile, la sua famiglia.
async fn scarica(
    utente: Utente,
    State(state): State<AppState>,
    Path((versione_id, formato)): Path<(i64, String)>,
) -> Result<Response, AppError> {
    let v: VersioneFile = sqlx::query_as(
        "SELECT a.id AS alunno_id, a.famiglia_user_id, v.numero, v.visibile_famiglia, v.pdf_path, v.odt_path
         FROM versioni v
         JOIN pfi p ON p.id = v.pfi_id
         JOIN alunni a ON a.id = p.alunno_id
         WHERE v.id = ?",
    )
    .bind(versione_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NonTrovato)?;
    let famiglia = utente.ruolo == Ruolo::Famiglia
        && v.famiglia_user_id == Some(utente.id)
        && v.visibile_famiglia;
    if !famiglia && !personale_puo_vedere(&state.db, &utente, v.alunno_id).await? {
        return Err(AppError::Vietato);
    }

    let (path, mime) = match formato.as_str() {
        "pdf" => (v.pdf_path, "application/pdf"),
        "odt" => (v.odt_path, "application/vnd.oasis.opendocument.text"),
        _ => return Err(AppError::NonTrovato),
    };
    let dati = match tokio::fs::read(state.config.data_dir.join(path)).await {
        Ok(dati) => dati,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(AppError::NonTrovato),
        Err(e) => return Err(e.into()),
    };
    // Nessun nome nell'header: il file si chiama con l'id dell'alunno e il numero di versione.
    let nome = format!(
        "attachment; filename=\"PFI-{}-v{}.{formato}\"",
        v.alunno_id, v.numero
    );
    Ok((
        [
            (CONTENT_TYPE, mime.to_owned()),
            (CONTENT_DISPOSITION, nome),
            (CACHE_CONTROL, "no-store".to_owned()),
        ],
        dati,
    )
        .into_response())
}

struct VersioneVista {
    id: i64,
    numero: i64,
    motivo: String,
    data: String,
    contenuto: Contenuto,
    /// Data dell'approvazione e nome di chi l'ha registrata.
    approvazione: Option<String>,
    visibile_famiglia: bool,
    /// Data e autore di ogni visto della famiglia.
    visti: Option<String>,
}

#[derive(sqlx::FromRow)]
struct VersioneDb {
    id: i64,
    numero: i64,
    motivo: String,
    data: String,
    contenuto_json: String,
    approvazione: Option<String>,
    visibile_famiglia: bool,
    visti: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AllegatoRiga {
    id: i64,
    tipo: String,
    nome_file: String,
    dimensione: i64,
    caricato_il: String,
    visibile_famiglia: bool,
}

#[derive(Template)]
#[template(path = "pfi_consulta.html")]
struct ConsultaPagina {
    utente: Utente,
    alunno_id: i64,
    alunno: String,
    classe: String,
    e_tutor: bool,
    puo_approvare: bool,
    versione: Option<VersioneVista>,
    allegati: Vec<AllegatoRiga>,
}

/// Consultazione per il personale che segue l'alunno: ultima versione del PFI e allegati.
async fn consulta(
    utente: Utente,
    State(state): State<AppState>,
    Path(alunno_id): Path<i64>,
) -> Result<Html<String>, AppError> {
    if !personale_puo_vedere(&state.db, &utente, alunno_id).await? {
        return Err(AppError::Vietato);
    }
    let (alunno, classe, tutor_id): (String, String, Option<i64>) = sqlx::query_as(
        "SELECT a.cognome || ' ' || a.nome, c.nome || ' (' || c.anno_scolastico || ')', a.tutor_id
         FROM alunni a JOIN classi c ON c.id = a.classe_id WHERE a.id = ?",
    )
    .bind(alunno_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NonTrovato)?;

    let ultima: Option<VersioneDb> = sqlx::query_as(
        "SELECT v.id, v.numero, v.motivo, date(v.creata_il, 'localtime') AS data, v.contenuto_json,
                date(v.approvata_il, 'localtime') || ' (registrata da ' || ap.nome || ' ' || ap.cognome || ')'
                    AS approvazione,
                v.visibile_famiglia,
                (SELECT group_concat(datetime(vi.data, 'localtime') || ' · ' || u.nome || ' ' || u.cognome, '; ')
                 FROM visti vi JOIN users u ON u.id = vi.user_id WHERE vi.versione_id = v.id) AS visti
         FROM versioni v
         JOIN pfi p ON p.id = v.pfi_id
         LEFT JOIN users ap ON ap.id = v.approvata_da
         WHERE p.alunno_id = ? ORDER BY v.numero DESC LIMIT 1",
    )
    .bind(alunno_id)
    .fetch_optional(&state.db)
    .await?;
    let versione = match ultima {
        Some(v) => Some(VersioneVista {
            id: v.id,
            numero: v.numero,
            motivo: v.motivo,
            data: v.data,
            contenuto: serde_json::from_str(&v.contenuto_json)
                .map_err(|e| AppError::Interno(e.to_string()))?,
            approvazione: v.approvazione,
            visibile_famiglia: v.visibile_famiglia,
            visti: v.visti,
        }),
        None => None,
    };
    let allegati = sqlx::query_as(
        "SELECT id, tipo, nome_file, dimensione, datetime(caricato_il, 'localtime') AS caricato_il,
                visibile_famiglia
         FROM allegati WHERE alunno_id = ? ORDER BY caricato_il DESC, id DESC",
    )
    .bind(alunno_id)
    .fetch_all(&state.db)
    .await?;
    let e_tutor = tutor_id == Some(utente.id);
    pagina(ConsultaPagina {
        e_tutor,
        puo_approvare: e_tutor || utente.ruolo == Ruolo::Dirigente,
        utente,
        alunno_id,
        alunno,
        classe,
        versione,
        allegati,
    })
}

/// Registra l'approvazione del Consiglio di Classe sull'ultima versione del PFI.
/// La registrano il tutor dell'alunno o il dirigente, con data e utente.
async fn approva(
    utente: Utente,
    State(state): State<AppState>,
    Path(versione_id): Path<i64>,
    Modulo(_): Modulo<Vuoto>,
) -> Result<Redirect, AppError> {
    let (alunno_id, tutor_id, ultima): (i64, Option<i64>, bool) = sqlx::query_as(
        "SELECT a.id, a.tutor_id,
                v.numero = (SELECT MAX(numero) FROM versioni WHERE pfi_id = v.pfi_id)
         FROM versioni v
         JOIN pfi p ON p.id = v.pfi_id
         JOIN alunni a ON a.id = p.alunno_id
         WHERE v.id = ?",
    )
    .bind(versione_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NonTrovato)?;
    if utente.ruolo != Ruolo::Dirigente && tutor_id != Some(utente.id) {
        return Err(AppError::Vietato);
    }
    if !ultima {
        return Err(richiesta("si può approvare solo l'ultima versione del PFI"));
    }
    // Un'approvazione già registrata non viene sovrascritta.
    sqlx::query(
        "UPDATE versioni SET approvata_il = datetime('now'), approvata_da = ?
         WHERE id = ? AND approvata_il IS NULL",
    )
    .bind(utente.id)
    .bind(versione_id)
    .execute(&state.db)
    .await?;
    Ok(Redirect::to(&format!("/alunni/{alunno_id}/pfi")))
}

#[derive(Deserialize)]
struct FormVisibile {
    visibile: bool,
    /// `modifica` quando la richiesta parte dallo storico del tutor.
    #[serde(default)]
    ritorno: String,
}

/// Il tutor sceglie se la famiglia vede questa versione; solo le versioni approvate si possono mostrare.
async fn visibilita(
    Docente(utente): Docente,
    State(state): State<AppState>,
    Path(versione_id): Path<i64>,
    Modulo(f): Modulo<FormVisibile>,
) -> Result<Redirect, AppError> {
    let (alunno_id, approvata): (i64, bool) = sqlx::query_as(
        "SELECT p.alunno_id, v.approvata_il IS NOT NULL
         FROM versioni v JOIN pfi p ON p.id = v.pfi_id WHERE v.id = ?",
    )
    .bind(versione_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NonTrovato)?;
    richiedi_tutor(&state.db, &utente, alunno_id).await?;
    if f.visibile && !approvata {
        return Err(richiesta(
            "solo una versione approvata dal Consiglio di Classe può essere resa visibile alla famiglia",
        ));
    }
    sqlx::query("UPDATE versioni SET visibile_famiglia = ? WHERE id = ?")
        .bind(f.visibile)
        .bind(versione_id)
        .execute(&state.db)
        .await?;
    let pagina = if f.ritorno == "modifica" {
        "/modifica"
    } else {
        ""
    };
    Ok(Redirect::to(&format!("/alunni/{alunno_id}/pfi{pagina}")))
}
