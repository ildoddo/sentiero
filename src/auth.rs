use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use askama::Template;
use axum::{
    Form,
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Multipart, Request, State},
    http::{
        HeaderMap, StatusCode,
        header::{COOKIE, SET_COOKIE},
        request::Parts,
    },
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, de::DeserializeOwned};
use sqlx::SqlitePool;

use crate::{AppState, error::AppError, routes::pagina};

const COOKIE_SESSIONE: &str = "sid";
const COOKIE_CSRF_LOGIN: &str = "csrf_login";
const DURATA_SESSIONE: i64 = 8 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum Ruolo {
    Admin,
    Docente,
    Dirigente,
    Famiglia,
}

/// Utente autenticato della richiesta corrente, con il token CSRF della sua sessione.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Utente {
    pub id: i64,
    pub ruolo: Ruolo,
    pub nome: String,
    pub cognome: String,
    pub csrf: String,
}

impl FromRequestParts<AppState> for Utente {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let token = cookie(&parts.headers, COOKIE_SESSIONE)
            .ok_or(AppError::NonAutenticato)?
            .to_owned();
        sqlx::query_as(
            "SELECT u.id, u.ruolo, u.nome, u.cognome, s.csrf_token AS csrf
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token = ? AND s.scade_il > ?",
        )
        .bind(token)
        .bind(adesso())
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NonAutenticato)
    }
}

macro_rules! estrattore_ruolo {
    ($nome:ident) => {
        /// Estrattore che ammette solo gli utenti con questo ruolo (403 per gli altri).
        pub struct $nome(pub Utente);

        impl FromRequestParts<AppState> for $nome {
            type Rejection = AppError;

            async fn from_request_parts(
                parts: &mut Parts,
                state: &AppState,
            ) -> Result<Self, AppError> {
                let utente = Utente::from_request_parts(parts, state).await?;
                if utente.ruolo == Ruolo::$nome {
                    Ok(Self(utente))
                } else {
                    Err(AppError::Vietato)
                }
            }
        }
    };
}
estrattore_ruolo!(Admin);
estrattore_ruolo!(Docente);
estrattore_ruolo!(Dirigente);
estrattore_ruolo!(Famiglia);

/// Form urlencoded di un utente autenticato, accettato solo se il campo `_csrf`
/// coincide con il token della sessione.
pub struct Modulo<T>(pub T);

impl<T: DeserializeOwned> FromRequest<AppState> for Modulo<T> {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, AppError> {
        let (mut parts, body) = req.into_parts();
        let utente = Utente::from_request_parts(&mut parts, state).await?;
        let corpo = Bytes::from_request(Request::from_parts(parts, body), state)
            .await
            .map_err(|_| AppError::Richiesta("corpo della richiesta non leggibile".into()))?;
        let non_valido = |_| AppError::Richiesta("modulo non valido".into());
        let campi: Vec<(String, String)> =
            serde_urlencoded::from_bytes(&corpo).map_err(non_valido)?;
        let token = campi
            .iter()
            .find(|(k, _)| k == "_csrf")
            .map_or("", |(_, v)| v);
        verifica_csrf(&utente, token)?;
        serde_urlencoded::from_bytes(&corpo)
            .map(Self)
            .map_err(non_valido)
    }
}

pub struct FileCaricato {
    pub nome: String,
    pub mime: String,
    pub dati: Bytes,
}

/// Form multipart di un utente autenticato, con lo stesso controllo CSRF di [`Modulo`].
/// Il corpo è letto in memoria, entro il limite di dimensione della route.
pub struct Caricamento {
    pub campi: HashMap<String, String>,
    pub file: Option<FileCaricato>,
}

impl FromRequest<AppState> for Caricamento {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, AppError> {
        let (mut parts, body) = req.into_parts();
        let utente = Utente::from_request_parts(&mut parts, state).await?;
        let non_valido =
            || AppError::Richiesta("caricamento non valido o file troppo grande".into());
        let mut multipart = Multipart::from_request(Request::from_parts(parts, body), state)
            .await
            .map_err(|_| non_valido())?;
        let (mut campi, mut file) = (HashMap::new(), None);
        while let Some(campo) = multipart.next_field().await.map_err(|_| non_valido())? {
            let nome = campo.name().unwrap_or_default().to_owned();
            if let Some(nome_file) = campo.file_name().map(str::to_owned) {
                let mime = campo.content_type().unwrap_or_default().to_owned();
                let dati = campo.bytes().await.map_err(|_| non_valido())?;
                // Un input file lasciato vuoto arriva comunque, senza contenuto.
                if !dati.is_empty() {
                    file = Some(FileCaricato {
                        nome: nome_file,
                        mime,
                        dati,
                    });
                }
            } else {
                campi.insert(nome, campo.text().await.map_err(|_| non_valido())?);
            }
        }
        verifica_csrf(&utente, campi.get("_csrf").map_or("", String::as_str))?;
        Ok(Self { campi, file })
    }
}

/// Ammette solo il tutor assegnato all'alunno; per chiunque altro, e per alunni inesistenti, 403.
pub async fn richiedi_tutor(
    db: &SqlitePool,
    utente: &Utente,
    alunno_id: i64,
) -> Result<(), AppError> {
    let trovato: Option<i64> =
        sqlx::query_scalar("SELECT id FROM alunni WHERE id = ? AND tutor_id = ?")
            .bind(alunno_id)
            .bind(utente.id)
            .fetch_optional(db)
            .await?;
    trovato.map(|_| ()).ok_or(AppError::Vietato)
}

/// Consultazione da parte del personale: il dirigente vede tutti gli alunni, un docente quelli
/// di cui è tutor o che appartengono a una classe del cui CdC fa parte.
pub async fn personale_puo_vedere(
    db: &SqlitePool,
    utente: &Utente,
    alunno_id: i64,
) -> Result<bool, AppError> {
    match utente.ruolo {
        Ruolo::Dirigente => Ok(true),
        Ruolo::Docente => {
            let trovato: Option<i64> = sqlx::query_scalar(
                "SELECT a.id FROM alunni a
                 WHERE a.id = ?1 AND (a.tutor_id = ?2 OR EXISTS (
                     SELECT 1 FROM cdc WHERE cdc.classe_id = a.classe_id AND cdc.docente_id = ?2))",
            )
            .bind(alunno_id)
            .bind(utente.id)
            .fetch_optional(db)
            .await?;
            Ok(trovato.is_some())
        }
        Ruolo::Admin | Ruolo::Famiglia => Ok(false),
    }
}

fn verifica_csrf(utente: &Utente, token: &str) -> Result<(), AppError> {
    if uguali(&utente.csrf, token) {
        Ok(())
    } else {
        Err(AppError::Vietato)
    }
}

/// Confronto a tempo costante.
fn uguali(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn cookie<'a>(headers: &'a HeaderMap, nome: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| {
            let (k, v) = c.trim().split_once('=')?;
            (k == nome).then_some(v)
        })
}

fn adesso() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn token_casuale() -> Result<String, AppError> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(|e| AppError::Interno(e.to_string()))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn hash_password(password: &str) -> Result<String, AppError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AppError::Interno(e.to_string()))
}

/// Argon2 è volutamente lento: il calcolo non deve bloccare il runtime asincrono.
pub async fn crea_hash(password: &str) -> Result<String, AppError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| AppError::Interno(e.to_string()))?
}

fn verifica_password(password: &str, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

// Verificato quando l'email non esiste, così i tempi di risposta non rivelano quali account esistono.
static HASH_FINTO: LazyLock<String> =
    LazyLock::new(|| hash_password("sentiero").unwrap_or_default());

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPagina<'a> {
    csrf: &'a str,
    errore: bool,
}

pub async fn login_form() -> Result<Response, AppError> {
    let csrf = token_casuale()?;
    let html = pagina(LoginPagina {
        csrf: &csrf,
        errore: false,
    })?;
    // Double-submit: prima del login non esiste una sessione a cui legare il token.
    let cookie = format!("{COOKIE_CSRF_LOGIN}={csrf}; HttpOnly; SameSite=Strict; Path=/login");
    Ok(([(SET_COOKIE, cookie)], html).into_response())
}

#[derive(Deserialize)]
pub struct Credenziali {
    email: String,
    password: String,
    #[serde(rename = "_csrf")]
    csrf: String,
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(c): Form<Credenziali>,
) -> Result<Response, AppError> {
    let atteso = cookie(&headers, COOKIE_CSRF_LOGIN).unwrap_or_default();
    if atteso.is_empty() || !uguali(atteso, &c.csrf) {
        return Err(AppError::Vietato);
    }

    let riga: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE email = ?")
            .bind(c.email.trim())
            .fetch_optional(&state.db)
            .await?;
    let (id, hash) = match riga {
        Some((id, hash)) => (Some(id), hash),
        None => (None, HASH_FINTO.clone()),
    };
    let password = c.password;
    let valida = tokio::task::spawn_blocking(move || verifica_password(&password, &hash))
        .await
        .map_err(|e| AppError::Interno(e.to_string()))?;
    let Some(id) = id.filter(|_| valida) else {
        let html = pagina(LoginPagina {
            csrf: atteso,
            errore: true,
        })?;
        return Ok((StatusCode::UNAUTHORIZED, html).into_response());
    };

    let ora = adesso();
    sqlx::query("DELETE FROM sessions WHERE scade_il <= ?")
        .bind(ora)
        .execute(&state.db)
        .await?;
    let token = token_casuale()?;
    // shortcut: il token è salvato in chiaro, chi legge il DB può usare le sessioni attive (max 8 ore); salvarne l'hash se il DB esce dal server.
    sqlx::query("INSERT INTO sessions (token, user_id, csrf_token, scade_il) VALUES (?, ?, ?, ?)")
        .bind(&token)
        .bind(id)
        .bind(token_casuale()?)
        .bind(ora + DURATA_SESSIONE)
        .execute(&state.db)
        .await?;

    let secure = if state.config.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{COOKIE_SESSIONE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={DURATA_SESSIONE}{secure}"
    );
    Ok(([(SET_COOKIE, cookie)], Redirect::to("/")).into_response())
}

#[derive(Deserialize)]
pub struct Vuoto {}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Modulo(_): Modulo<Vuoto>,
) -> Result<Response, AppError> {
    if let Some(token) = cookie(&headers, COOKIE_SESSIONE) {
        sqlx::query("DELETE FROM sessions WHERE token = ?")
            .bind(token)
            .execute(&state.db)
            .await?;
    }
    let cookie = format!("{COOKIE_SESSIONE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    Ok(([(SET_COOKIE, cookie)], Redirect::to("/login")).into_response())
}
