//! Anagrafica gestita dall'amministratore: classi, docenti, alunni e Consigli di Classe.

use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    response::{Html, Redirect},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::{SqliteConnection, SqlitePool};

use crate::{
    AppState,
    auth::{Admin, Caricamento, Modulo, Ruolo, Utente, crea_hash},
    csv_import::{RigaAlunno, RigaCdc, RigaClasse, RigaDocente, leggi, normalizza_data},
    error::AppError,
    routes::pagina,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/classi", get(classi).post(crea_classe))
        .route("/admin/docenti", get(docenti).post(crea_docente))
        .route("/admin/alunni", get(alunni).post(crea_alunno))
        .route("/admin/cdc", get(cdc).post(aggiungi_al_cdc))
        .route("/admin/cdc/rimuovi", post(rimuovi_dal_cdc))
        .route("/admin/import/{tipo}", post(importa))
}

fn richiesta(msg: &str) -> AppError {
    AppError::Richiesta(msg.into())
}

fn obbligatori(campi: &[(&str, &str)]) -> Result<(), AppError> {
    match campi.iter().find(|(_, valore)| valore.trim().is_empty()) {
        Some((nome, _)) => Err(AppError::Richiesta(format!(
            "campo obbligatorio mancante: {nome}"
        ))),
        None => Ok(()),
    }
}

// ---- Classi

#[derive(sqlx::FromRow)]
pub(super) struct Classe {
    pub(super) id: i64,
    pub(super) nome: String,
    pub(super) anno_scolastico: String,
    pub(super) indirizzo: String,
    pub(super) alunni: i64,
}

pub(super) async fn elenco_classi(db: &SqlitePool) -> Result<Vec<Classe>, AppError> {
    Ok(sqlx::query_as(
        "SELECT c.id, c.nome, c.anno_scolastico, c.indirizzo,
                (SELECT COUNT(*) FROM alunni a WHERE a.classe_id = c.id) AS alunni
         FROM classi c ORDER BY c.anno_scolastico DESC, c.nome",
    )
    .fetch_all(db)
    .await?)
}

#[derive(Template)]
#[template(path = "admin_classi.html")]
struct ClassiPagina {
    utente: Utente,
    classi: Vec<Classe>,
}

async fn classi(
    Admin(utente): Admin,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let classi = elenco_classi(&state.db).await?;
    pagina(ClassiPagina { utente, classi })
}

async fn inserisci_classe(conn: &mut SqliteConnection, r: &RigaClasse) -> Result<(), AppError> {
    obbligatori(&[
        ("nome", &r.nome),
        ("anno_scolastico", &r.anno_scolastico),
        ("indirizzo", &r.indirizzo),
    ])?;
    sqlx::query(
        "INSERT INTO classi (nome, anno_scolastico, indirizzo) VALUES (?, ?, ?)
         ON CONFLICT (nome, anno_scolastico) DO UPDATE SET indirizzo = excluded.indirizzo",
    )
    .bind(r.nome.trim())
    .bind(r.anno_scolastico.trim())
    .bind(r.indirizzo.trim())
    .execute(conn)
    .await?;
    Ok(())
}

async fn crea_classe(
    Admin(_): Admin,
    State(state): State<AppState>,
    Modulo(riga): Modulo<RigaClasse>,
) -> Result<Redirect, AppError> {
    let mut conn = state.db.acquire().await?;
    inserisci_classe(&mut conn, &riga).await?;
    Ok(Redirect::to("/admin/classi"))
}

// ---- Docenti e dirigente

#[derive(sqlx::FromRow)]
struct Docente {
    id: i64,
    cognome: String,
    nome: String,
    email: String,
    ruolo: String,
}

#[derive(Template)]
#[template(path = "admin_docenti.html")]
struct DocentiPagina {
    utente: Utente,
    docenti: Vec<Docente>,
}

async fn docenti(
    Admin(utente): Admin,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let docenti = sqlx::query_as(
        "SELECT id, cognome, nome, email, ruolo FROM users
         WHERE ruolo IN ('docente', 'dirigente') ORDER BY cognome, nome",
    )
    .fetch_all(&state.db)
    .await?;
    pagina(DocentiPagina { utente, docenti })
}

async fn crea_utente(
    conn: &mut SqliteConnection,
    email: &str,
    password: &str,
    ruolo: &str,
    nome: &str,
    cognome: &str,
) -> Result<i64, AppError> {
    if !email.contains('@') {
        return Err(richiesta("email non valida"));
    }
    if password.chars().count() < 8 {
        return Err(richiesta(
            "per un nuovo utente serve una password di almeno 8 caratteri",
        ));
    }
    let hash = crea_hash(password).await?;
    Ok(sqlx::query_scalar(
        "INSERT INTO users (email, password_hash, ruolo, nome, cognome) VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(email)
    .bind(hash)
    .bind(ruolo)
    .bind(nome)
    .bind(cognome)
    .fetch_one(conn)
    .await?)
}

async fn utente_per_email(
    conn: &mut SqliteConnection,
    email: &str,
) -> Result<Option<(i64, String)>, AppError> {
    Ok(
        sqlx::query_as("SELECT id, ruolo FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(conn)
            .await?,
    )
}

/// Crea il docente o, se l'email esiste già con lo stesso profilo, ne aggiorna il nome.
/// La password di un utente esistente non viene mai toccata dall'importazione.
async fn inserisci_docente(conn: &mut SqliteConnection, r: &RigaDocente) -> Result<(), AppError> {
    obbligatori(&[
        ("cognome", &r.cognome),
        ("nome", &r.nome),
        ("email", &r.email),
    ])?;
    let ruolo = match r.ruolo.trim() {
        "" | "docente" => "docente",
        "dirigente" => "dirigente",
        _ => return Err(richiesta("ruolo non valido: usare docente o dirigente")),
    };
    let email = r.email.trim();
    match utente_per_email(conn, email).await? {
        Some((id, esistente)) if esistente == ruolo => {
            sqlx::query("UPDATE users SET nome = ?, cognome = ? WHERE id = ?")
                .bind(r.nome.trim())
                .bind(r.cognome.trim())
                .bind(id)
                .execute(conn)
                .await?;
        }
        Some(_) => {
            return Err(richiesta(
                "email già usata da un utente con un altro profilo",
            ));
        }
        None => {
            crea_utente(
                conn,
                email,
                &r.password,
                ruolo,
                r.nome.trim(),
                r.cognome.trim(),
            )
            .await?;
        }
    }
    Ok(())
}

async fn crea_docente(
    Admin(_): Admin,
    State(state): State<AppState>,
    Modulo(riga): Modulo<RigaDocente>,
) -> Result<Redirect, AppError> {
    let mut tx = state.db.begin().await?;
    inserisci_docente(&mut tx, &riga).await?;
    tx.commit().await?;
    Ok(Redirect::to("/admin/docenti"))
}

// ---- Alunni

#[derive(sqlx::FromRow)]
struct AlunnoRiga {
    cognome: String,
    nome: String,
    data_nascita: String,
    classe: String,
    anno_scolastico: String,
    email_famiglia: Option<String>,
}

#[derive(Template)]
#[template(path = "admin_alunni.html")]
struct AlunniPagina {
    utente: Utente,
    alunni: Vec<AlunnoRiga>,
    classi: Vec<Classe>,
}

async fn alunni(
    Admin(utente): Admin,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let alunni = sqlx::query_as(
        "SELECT a.cognome, a.nome, a.data_nascita, c.nome AS classe, c.anno_scolastico,
                f.email AS email_famiglia
         FROM alunni a
         JOIN classi c ON c.id = a.classe_id
         LEFT JOIN users f ON f.id = a.famiglia_user_id
         ORDER BY c.anno_scolastico DESC, c.nome, a.cognome, a.nome",
    )
    .fetch_all(&state.db)
    .await?;
    let classi = elenco_classi(&state.db).await?;
    pagina(AlunniPagina {
        utente,
        alunni,
        classi,
    })
}

struct NuovoAlunno<'a> {
    classe_id: i64,
    cognome: &'a str,
    nome: &'a str,
    data_nascita: &'a str,
    email_famiglia: &'a str,
    password_famiglia: &'a str,
}

/// Inserisce l'alunno (o lo ritrova, se già presente) e collega l'account della famiglia,
/// creandolo se l'email è nuova. Più alunni possono condividere lo stesso account.
async fn inserisci_alunno(
    conn: &mut SqliteConnection,
    a: &NuovoAlunno<'_>,
) -> Result<(), AppError> {
    obbligatori(&[("cognome", a.cognome), ("nome", a.nome)])?;
    let data = normalizza_data(a.data_nascita.trim())
        .ok_or_else(|| richiesta("data di nascita non valida: usare AAAA-MM-GG o GG/MM/AAAA"))?;
    let (cognome, email) = (a.cognome.trim(), a.email_famiglia.trim());
    let famiglia_id = if email.is_empty() {
        None
    } else {
        Some(match utente_per_email(conn, email).await? {
            Some((id, ruolo)) if ruolo == "famiglia" => id,
            Some(_) => {
                return Err(richiesta(
                    "email della famiglia già usata da un utente con un altro profilo",
                ));
            }
            None => {
                crea_utente(
                    conn,
                    email,
                    a.password_famiglia,
                    "famiglia",
                    "Famiglia",
                    cognome,
                )
                .await?
            }
        })
    };
    sqlx::query(
        "INSERT INTO alunni (classe_id, cognome, nome, data_nascita, famiglia_user_id) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (classe_id, cognome, nome, data_nascita)
         DO UPDATE SET famiglia_user_id = COALESCE(excluded.famiglia_user_id, famiglia_user_id)",
    )
    .bind(a.classe_id)
    .bind(cognome)
    .bind(a.nome.trim())
    .bind(data)
    .bind(famiglia_id)
    .execute(conn)
    .await?;
    Ok(())
}

async fn classe_per_nome(
    conn: &mut SqliteConnection,
    nome: &str,
    anno: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar("SELECT id FROM classi WHERE nome = ? AND anno_scolastico = ?")
        .bind(nome)
        .bind(anno)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| richiesta("classe non trovata: importare prima le classi"))
}

#[derive(Deserialize)]
struct FormAlunno {
    classe_id: i64,
    cognome: String,
    nome: String,
    data_nascita: String,
    #[serde(default)]
    email_famiglia: String,
    #[serde(default)]
    password_famiglia: String,
}

async fn crea_alunno(
    Admin(_): Admin,
    State(state): State<AppState>,
    Modulo(f): Modulo<FormAlunno>,
) -> Result<Redirect, AppError> {
    let mut tx = state.db.begin().await?;
    let esiste: Option<i64> = sqlx::query_scalar("SELECT id FROM classi WHERE id = ?")
        .bind(f.classe_id)
        .fetch_optional(&mut *tx)
        .await?;
    esiste.ok_or_else(|| richiesta("classe non trovata"))?;
    let alunno = NuovoAlunno {
        classe_id: f.classe_id,
        cognome: &f.cognome,
        nome: &f.nome,
        data_nascita: &f.data_nascita,
        email_famiglia: &f.email_famiglia,
        password_famiglia: &f.password_famiglia,
    };
    inserisci_alunno(&mut tx, &alunno).await?;
    tx.commit().await?;
    Ok(Redirect::to("/admin/alunni"))
}

// ---- Consigli di Classe

#[derive(sqlx::FromRow)]
struct CdcRiga {
    classe_id: i64,
    classe: String,
    anno_scolastico: String,
    docente_id: i64,
    cognome: String,
    nome: String,
}

#[derive(Template)]
#[template(path = "admin_cdc.html")]
struct CdcPagina {
    utente: Utente,
    membri: Vec<CdcRiga>,
    classi: Vec<Classe>,
    docenti: Vec<Docente>,
}

async fn cdc(
    Admin(utente): Admin,
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    let membri = sqlx::query_as(
        "SELECT c.id AS classe_id, c.nome AS classe, c.anno_scolastico,
                u.id AS docente_id, u.cognome, u.nome
         FROM cdc
         JOIN classi c ON c.id = cdc.classe_id
         JOIN users u ON u.id = cdc.docente_id
         ORDER BY c.anno_scolastico DESC, c.nome, u.cognome, u.nome",
    )
    .fetch_all(&state.db)
    .await?;
    let docenti = sqlx::query_as(
        "SELECT id, cognome, nome, email, ruolo FROM users WHERE ruolo = 'docente' ORDER BY cognome, nome",
    )
    .fetch_all(&state.db)
    .await?;
    let classi = elenco_classi(&state.db).await?;
    pagina(CdcPagina {
        utente,
        membri,
        classi,
        docenti,
    })
}

#[derive(Deserialize)]
struct FormCdc {
    classe_id: i64,
    docente_id: i64,
}

async fn aggiungi_al_cdc(
    Admin(_): Admin,
    State(state): State<AppState>,
    Modulo(f): Modulo<FormCdc>,
) -> Result<Redirect, AppError> {
    // Il filtro sul ruolo e sulla classe evita di inserire nel CdC chi non è un docente.
    sqlx::query(
        "INSERT OR IGNORE INTO cdc (classe_id, docente_id)
         SELECT c.id, u.id FROM classi c, users u
         WHERE c.id = ? AND u.id = ? AND u.ruolo = 'docente'",
    )
    .bind(f.classe_id)
    .bind(f.docente_id)
    .execute(&state.db)
    .await?;
    Ok(Redirect::to("/admin/cdc"))
}

async fn rimuovi_dal_cdc(
    Admin(_): Admin,
    State(state): State<AppState>,
    Modulo(f): Modulo<FormCdc>,
) -> Result<Redirect, AppError> {
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM cdc WHERE classe_id = ? AND docente_id = ?")
        .bind(f.classe_id)
        .bind(f.docente_id)
        .execute(&mut *tx)
        .await?;
    // Il tutor deve far parte del CdC: chi esce dal consiglio perde i suoi alunni in quella classe.
    sqlx::query("UPDATE alunni SET tutor_id = NULL WHERE classe_id = ? AND tutor_id = ?")
        .bind(f.classe_id)
        .bind(f.docente_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Redirect::to("/admin/cdc"))
}

// ---- Importazione CSV

fn con_riga(n: usize, e: AppError) -> AppError {
    match e {
        AppError::Richiesta(msg) => AppError::Richiesta(format!("riga {n}: {msg}")),
        altro => altro,
    }
}

/// Importa un file CSV in un'unica transazione: se una riga è sbagliata non viene salvato nulla.
async fn importa(
    Admin(_): Admin,
    State(state): State<AppState>,
    Path(tipo): Path<String>,
    caricamento: Caricamento,
) -> Result<Redirect, AppError> {
    let file = caricamento
        .file
        .ok_or_else(|| richiesta("nessun file selezionato"))?;
    let mut tx = state.db.begin().await?;
    match tipo.as_str() {
        "classi" => {
            for (n, riga) in leggi::<RigaClasse>(&file.dati)? {
                inserisci_classe(&mut tx, &riga)
                    .await
                    .map_err(|e| con_riga(n, e))?;
            }
        }
        "docenti" => {
            for (n, riga) in leggi::<RigaDocente>(&file.dati)? {
                inserisci_docente(&mut tx, &riga)
                    .await
                    .map_err(|e| con_riga(n, e))?;
            }
        }
        "alunni" => {
            for (n, r) in leggi::<RigaAlunno>(&file.dati)? {
                let esito = async {
                    let classe_id = classe_per_nome(&mut tx, &r.classe, &r.anno_scolastico).await?;
                    let alunno = NuovoAlunno {
                        classe_id,
                        cognome: &r.cognome,
                        nome: &r.nome,
                        data_nascita: &r.data_nascita,
                        email_famiglia: &r.email_famiglia,
                        password_famiglia: &r.password_famiglia,
                    };
                    inserisci_alunno(&mut tx, &alunno).await
                };
                esito.await.map_err(|e| con_riga(n, e))?;
            }
        }
        "cdc" => {
            for (n, r) in leggi::<RigaCdc>(&file.dati)? {
                let esito = async {
                    let classe_id = classe_per_nome(&mut tx, &r.classe, &r.anno_scolastico).await?;
                    let docente_id = match utente_per_email(&mut tx, &r.email_docente).await? {
                        Some((id, ruolo)) if ruolo == "docente" => id,
                        _ => {
                            return Err(richiesta(
                                "docente non trovato: importare prima i docenti",
                            ));
                        }
                    };
                    sqlx::query("INSERT OR IGNORE INTO cdc (classe_id, docente_id) VALUES (?, ?)")
                        .bind(classe_id)
                        .bind(docente_id)
                        .execute(&mut *tx)
                        .await?;
                    Ok(())
                };
                esito.await.map_err(|e| con_riga(n, e))?;
            }
        }
        _ => return Err(AppError::NonTrovato),
    }
    tx.commit().await?;
    Ok(Redirect::to(&format!("/admin/{tipo}")))
}
