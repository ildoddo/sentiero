//! Test di integrazione su autenticazione, CSRF e controlli di accesso per ruolo.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode, header},
    routing::get,
};
use sentiero::{
    AppState, apri_db,
    auth::{Admin, Dirigente, Docente, Famiglia},
    config::Config,
    seed,
};
use tower::ServiceExt;

const PASSWORD: &str = "password-di-prova";

/// Una cartella dati per ogni test: i documenti generati non si sovrappongono tra test paralleli.
fn cartella_di_prova() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let cartella = std::env::temp_dir().join(format!("sentiero-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cartella);
    cartella
}

async fn stato() -> AppState {
    let db = apri_db("sqlite::memory:").await.unwrap();
    seed::esegui(&db, PASSWORD).await.unwrap();
    let config = Config {
        bind: String::new(),
        database_url: String::new(),
        data_dir: cartella_di_prova(),
        cookie_secure: false,
        seed_password: None,
    };
    AppState {
        db,
        config: Arc::new(config),
    }
}

/// Crea direttamente una sessione per l'utente e restituisce l'header Cookie; il token CSRF è "csrf".
async fn sessione(stato: &AppState, email: &str) -> String {
    let token = format!("sessione-{email}");
    sqlx::query(
        "INSERT OR REPLACE INTO sessions (token, user_id, csrf_token, scade_il)
         SELECT ?, id, 'csrf', unixepoch() + 3600 FROM users WHERE email = ?",
    )
    .bind(&token)
    .bind(email)
    .execute(&stato.db)
    .await
    .unwrap();
    format!("sid={token}")
}

async fn get_come(app: &Router, uri: &str, cookie: Option<&str>) -> Response<Body> {
    let mut req = Request::get(uri);
    if let Some(cookie) = cookie {
        req = req.header(header::COOKIE, cookie);
    }
    app.clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn post_form(app: &Router, uri: &str, cookie: &str, corpo: &str) -> Response<Body> {
    let req = Request::post(uri)
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(corpo.to_owned()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

#[tokio::test]
async fn anonimo_rimandato_al_login() {
    let app = sentiero::app(stato().await);
    let res = get_come(&app, "/", None).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers()[header::LOCATION], "/login");

    let res = get_come(&app, "/", Some("sid=inesistente")).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn sessione_scaduta_non_vale() {
    let stato = stato().await;
    let cookie = sessione(&stato, "admin@sentiero.test").await;
    sqlx::query("UPDATE sessions SET scade_il = unixepoch() - 1")
        .execute(&stato.db)
        .await
        .unwrap();
    let res = get_come(&sentiero::app(stato), "/", Some(&cookie)).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn login_con_password_e_csrf() {
    let app = sentiero::app(stato().await);

    let res = get_come(&app, "/login", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    let set_cookie = res.headers()[header::SET_COOKIE].to_str().unwrap();
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    let csrf = cookie.strip_prefix("csrf_login=").unwrap();

    let corpo = |password: &str, csrf: &str| {
        format!("email=tutor%40sentiero.test&password={password}&_csrf={csrf}")
    };

    let res = post_form(&app, "/login", &cookie, &corpo(PASSWORD, "sbagliato")).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let res = post_form(&app, "/login", &cookie, &corpo("errata", csrf)).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(res.headers().get(header::SET_COOKIE).is_none());

    let res = post_form(&app, "/login", &cookie, &corpo(PASSWORD, csrf)).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let sid = res.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(sid.starts_with("sid=") && sid.contains("HttpOnly"));
    let sid = sid.split(';').next().unwrap();
    assert_eq!(
        get_come(&app, "/", Some(sid)).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn logout_richiede_csrf() {
    let stato = stato().await;
    let cookie = sessione(&stato, "tutor@sentiero.test").await;
    let app = sentiero::app(stato);

    let res = post_form(&app, "/logout", &cookie, "_csrf=sbagliato").await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = post_form(&app, "/logout", &cookie, "").await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        get_come(&app, "/", Some(&cookie)).await.status(),
        StatusCode::OK
    );

    let res = post_form(&app, "/logout", &cookie, "_csrf=csrf").await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        get_come(&app, "/", Some(&cookie)).await.status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn estrattori_di_ruolo() {
    let stato = stato().await;
    let app = Router::new()
        .route("/admin", get(|_: Admin| async {}))
        .route("/docente", get(|_: Docente| async {}))
        .route("/dirigente", get(|_: Dirigente| async {}))
        .route("/famiglia", get(|_: Famiglia| async {}))
        .with_state(stato.clone());

    let utenti = [
        ("admin@sentiero.test", "/admin"),
        ("tutor@sentiero.test", "/docente"),
        ("dirigente@sentiero.test", "/dirigente"),
        ("famiglia@sentiero.test", "/famiglia"),
    ];
    for (email, propria) in utenti {
        let cookie = sessione(&stato, email).await;
        for (_, uri) in utenti {
            let atteso = if uri == propria {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            };
            assert_eq!(
                get_come(&app, uri, Some(&cookie)).await.status(),
                atteso,
                "{email} su {uri}"
            );
        }
    }
    assert_eq!(
        get_come(&app, "/admin", None).await.status(),
        StatusCode::SEE_OTHER
    );
}

async fn post_csv(app: &Router, uri: &str, cookie: &str, csrf: &str, csv: &str) -> Response<Body> {
    let corpo = format!(
        "--confine\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n\
         --confine\r\nContent-Disposition: form-data; name=\"file\"; filename=\"dati.csv\"\r\n\
         Content-Type: text/csv\r\n\r\n{csv}\r\n--confine--\r\n"
    );
    let req = Request::post(uri)
        .header(header::COOKIE, cookie)
        .header(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=confine",
        )
        .body(Body::from(corpo))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

async fn conta(stato: &AppState, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&stato.db).await.unwrap()
}

const PAGINE_ADMIN: [&str; 7] = [
    "/admin/classi",
    "/admin/docenti",
    "/admin/alunni",
    "/admin/cdc",
    "/admin/modelli",
    "/admin/modelli/1",
    "/admin/assegnazioni",
];

#[tokio::test]
async fn anagrafica_riservata_all_amministratore() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let nuova_classe = "_csrf=csrf&nome=2Z&anno_scolastico=2026%2F2027&indirizzo=Prova";

    for uri in PAGINE_ADMIN {
        assert_eq!(
            get_come(&app, uri, None).await.status(),
            StatusCode::SEE_OTHER
        );
    }
    for email in [
        "tutor@sentiero.test",
        "dirigente@sentiero.test",
        "famiglia@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        for uri in PAGINE_ADMIN {
            let res = get_come(&app, uri, Some(&cookie)).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email} su {uri}");
        }
        let res = post_form(&app, "/admin/classi", &cookie, nuova_classe).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let res = post_csv(
            &app,
            "/admin/import/classi",
            &cookie,
            "csrf",
            "nome;anno_scolastico;indirizzo\n2Z;x;y",
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM classi").await, 1);

    let cookie = sessione(&stato, "admin@sentiero.test").await;
    for uri in PAGINE_ADMIN {
        assert_eq!(
            get_come(&app, uri, Some(&cookie)).await.status(),
            StatusCode::OK
        );
    }
    let res = post_form(&app, "/admin/classi", &cookie, nuova_classe).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM classi").await, 2);
}

#[tokio::test]
async fn import_alunni_csv() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let cookie = sessione(&stato, "admin@sentiero.test").await;
    let alunni = "SELECT COUNT(*) FROM alunni";
    let intestazione =
        "classe;anno_scolastico;cognome;nome;data_nascita;email_famiglia;password_famiglia";

    let valido = format!(
        "{intestazione}\n1A;2026/2027;Russo;Luca;05/09/2012;russo@example.test;password-famiglia\n\
         1A;2026/2027;Russo;Elena;2012-09-05;russo@example.test;\n"
    );
    let res = post_csv(&app, "/admin/import/alunni", &cookie, "sbagliato", &valido).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(conta(&stato, alunni).await, 3);

    let res = post_csv(&app, "/admin/import/alunni", &cookie, "csrf", &valido).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(conta(&stato, alunni).await, 5);
    // I due fratelli condividono un solo account famiglia, creato dalla prima riga.
    let collegati = "SELECT COUNT(*) FROM alunni a JOIN users u ON u.id = a.famiglia_user_id
                     WHERE u.email = 'russo@example.test' AND u.ruolo = 'famiglia'";
    assert_eq!(conta(&stato, collegati).await, 2);

    // Reimportare lo stesso file non duplica nulla.
    let res = post_csv(&app, "/admin/import/alunni", &cookie, "csrf", &valido).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(conta(&stato, alunni).await, 5);

    // Una riga sbagliata annulla l'intero file.
    let errato = format!(
        "{intestazione}\n1A;2026/2027;Verdi;Anna;2012-01-01;;\n1A;2026/2027;Neri;Ugo;31/02/2012;;\n"
    );
    let res = post_csv(&app, "/admin/import/alunni", &cookie, "csrf", &errato).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(conta(&stato, alunni).await, 5);

    // L'email di un docente non può diventare un account famiglia.
    let errato = format!(
        "{intestazione}\n1A;2026/2027;Verdi;Anna;2012-01-01;tutor@sentiero.test;password-famiglia\n"
    );
    let res = post_csv(&app, "/admin/import/alunni", &cookie, "csrf", &errato).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let res = post_csv(&app, "/admin/import/altro", &cookie, "csrf", &valido).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn import_docenti_e_cdc() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let cookie = sessione(&stato, "admin@sentiero.test").await;

    let docenti = "cognome,nome,email,password\nGalli,Piero,galli@example.test,password-docente\n\
                   Tutor,Tommaso Maria,tutor@sentiero.test,\n";
    let res = post_csv(&app, "/admin/import/docenti", &cookie, "csrf", docenti).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM users WHERE nome = 'Tommaso Maria'"
        )
        .await,
        1
    );

    // Un nuovo utente senza password viene rifiutato.
    let res = post_csv(
        &app,
        "/admin/import/docenti",
        &cookie,
        "csrf",
        "cognome,nome,email\nNeri,Ugo,neri@example.test\n",
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let cdc = "classe;anno_scolastico;email_docente\n1A;2026/2027;galli@example.test\n";
    let res = post_csv(&app, "/admin/import/cdc", &cookie, "csrf", cdc).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM cdc").await, 3);

    // Il dirigente non è un docente: non entra nel CdC.
    let cdc = "classe;anno_scolastico;email_docente\n1A;2026/2027;dirigente@sentiero.test\n";
    let res = post_csv(&app, "/admin/import/cdc", &cookie, "csrf", cdc).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn uscita_dal_cdc_revoca_il_tutor() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let cookie = sessione(&stato, "admin@sentiero.test").await;
    let tutor = conta(
        &stato,
        "SELECT id FROM users WHERE email = 'tutor@sentiero.test'",
    )
    .await;
    sqlx::query("UPDATE alunni SET tutor_id = ?")
        .bind(tutor)
        .execute(&stato.db)
        .await
        .unwrap();

    let corpo = format!("_csrf=csrf&classe_id=1&docente_id={tutor}");
    let res = post_form(&app, "/admin/cdc/rimuovi", &cookie, &corpo).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM cdc").await, 1);
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM alunni WHERE tutor_id IS NOT NULL"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn modelli_riservati_all_amministratore() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let azioni = [
        (
            "/admin/modelli",
            "_csrf=csrf&nome=Altro&indirizzo=Meccanica",
        ),
        (
            "/admin/modelli/1",
            "_csrf=csrf&nome=Cambiato&indirizzo=Meccanica",
        ),
        (
            "/admin/sezioni/1/attiva",
            "_csrf=csrf&modello_id=1&attiva=false",
        ),
        (
            "/admin/sezioni/1/sottosezioni",
            "_csrf=csrf&modello_id=1&titolo=T&etichetta=E",
        ),
        (
            "/admin/sottosezioni/1",
            "_csrf=csrf&modello_id=1&titolo=Cambiato",
        ),
        ("/admin/sottosezioni/1/elimina", "_csrf=csrf&modello_id=1"),
        (
            "/admin/sottosezioni/1/indicatori",
            "_csrf=csrf&modello_id=1&etichetta=E",
        ),
        (
            "/admin/indicatori/1",
            "_csrf=csrf&modello_id=1&etichetta=Cambiato",
        ),
        ("/admin/indicatori/1/elimina", "_csrf=csrf&modello_id=1"),
    ];
    let impronta = "SELECT (SELECT COUNT(*) FROM modelli) * 1000000 + (SELECT COUNT(*) FROM sottosezioni) * 1000
                    + (SELECT COUNT(*) FROM indicatori) + (SELECT SUM(attiva) FROM sezioni) * 100000000";
    let prima = conta(&stato, impronta).await;
    for email in [
        "tutor@sentiero.test",
        "dirigente@sentiero.test",
        "famiglia@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        for (uri, corpo) in azioni {
            let res = post_form(&app, uri, &cookie, corpo).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email} su {uri}");
        }
    }
    assert_eq!(conta(&stato, impronta).await, prima);
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM modelli WHERE nome = 'Modello base'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn regole_dei_modelli() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let cookie = sessione(&stato, "admin@sentiero.test").await;

    // Un nuovo modello nasce con le cinque sezioni, ognuna con sottosezioni e indicatori.
    let res = post_form(
        &app,
        "/admin/modelli",
        &cookie,
        "_csrf=csrf&nome=Meccanica&indirizzo=Manutenzione",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers()[header::LOCATION], "/admin/modelli/2");
    assert_eq!(
        conta(&stato, "SELECT COUNT(*) FROM sezioni WHERE modello_id = 2").await,
        5
    );
    let vuote = "SELECT COUNT(*) FROM sottosezioni ss
                 WHERE NOT EXISTS (SELECT 1 FROM indicatori i WHERE i.sottosezione_id = ss.id)";
    assert_eq!(conta(&stato, vuote).await, 0);
    let res = post_form(
        &app,
        "/admin/modelli",
        &cookie,
        "_csrf=csrf&nome=Meccanica&indirizzo=Altro",
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Al massimo sei sottosezioni personalizzate per sezione.
    let sezione = conta(
        &stato,
        "SELECT id FROM sezioni WHERE modello_id = 2 AND codice = 'C'",
    )
    .await;
    let uri = format!("/admin/sezioni/{sezione}/sottosezioni");
    for n in 1..=6 {
        let corpo = format!("_csrf=csrf&modello_id=2&titolo=Extra+{n}&etichetta=Indicatore");
        assert_eq!(
            post_form(&app, &uri, &cookie, &corpo).await.status(),
            StatusCode::SEE_OTHER
        );
    }
    let res = post_form(
        &app,
        &uri,
        &cookie,
        "_csrf=csrf&modello_id=2&titolo=Settima&etichetta=I",
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM sottosezioni WHERE personalizzata = 1"
        )
        .await,
        6
    );
    // Il limite è per sezione: un'altra sezione ne accetta ancora.
    let altra = conta(
        &stato,
        "SELECT id FROM sezioni WHERE modello_id = 2 AND codice = 'A'",
    )
    .await;
    let res = post_form(
        &app,
        &format!("/admin/sezioni/{altra}/sottosezioni"),
        &cookie,
        "_csrf=csrf&modello_id=2&titolo=Extra&etichetta=I",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Si eliminano solo le personalizzate; quelle base si disattivano.
    let base = conta(
        &stato,
        "SELECT MIN(id) FROM sottosezioni WHERE personalizzata = 0",
    )
    .await;
    let res = post_form(
        &app,
        &format!("/admin/sottosezioni/{base}/elimina"),
        &cookie,
        "_csrf=csrf&modello_id=1",
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = post_form(
        &app,
        &format!("/admin/sottosezioni/{base}"),
        &cookie,
        "_csrf=csrf&modello_id=1&titolo=Dati",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let spenta = sqlx::query_scalar::<_, bool>("SELECT attiva FROM sottosezioni WHERE id = ?")
        .bind(base)
        .fetch_one(&stato.db)
        .await
        .unwrap();
    assert!(!spenta);
    let extra = conta(
        &stato,
        "SELECT MAX(id) FROM sottosezioni WHERE personalizzata = 1",
    )
    .await;
    let res = post_form(
        &app,
        &format!("/admin/sottosezioni/{extra}/elimina"),
        &cookie,
        "_csrf=csrf&modello_id=2",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // L'ultimo indicatore di una sottosezione non si elimina.
    let unico = conta(
        &stato,
        "SELECT MIN(id) FROM indicatori GROUP BY sottosezione_id HAVING COUNT(*) = 1 ORDER BY 1 LIMIT 1",
    )
    .await;
    let res = post_form(
        &app,
        &format!("/admin/indicatori/{unico}/elimina"),
        &cookie,
        "_csrf=csrf&modello_id=1",
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        conta(&stato, "SELECT COUNT(*) FROM indicatori WHERE id = 1").await,
        1
    );
    let res = post_form(
        &app,
        "/admin/indicatori/1/elimina",
        &cookie,
        "_csrf=csrf&modello_id=1",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        conta(&stato, "SELECT COUNT(*) FROM indicatori WHERE id = 1").await,
        0
    );

    // Sezione D disattivabile e riattivabile.
    let d = conta(
        &stato,
        "SELECT id FROM sezioni WHERE modello_id = 1 AND codice = 'D'",
    )
    .await;
    let res = post_form(
        &app,
        &format!("/admin/sezioni/{d}/attiva"),
        &cookie,
        "_csrf=csrf&modello_id=1&attiva=false",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        conta(
            &stato,
            "SELECT SUM(attiva) FROM sezioni WHERE modello_id = 1"
        )
        .await,
        4
    );
    assert_eq!(
        get_come(&app, "/admin/modelli/1", Some(&cookie))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        get_come(&app, "/admin/modelli/99", Some(&cookie))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

async fn id_utente(stato: &AppState, email: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(&stato.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn assegnazioni() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let tutor = id_utente(&stato, "tutor@sentiero.test").await;
    let docente = id_utente(&stato, "docente@sentiero.test").await;
    let dirigente = id_utente(&stato, "dirigente@sentiero.test").await;
    let con_docente = "SELECT COUNT(*) FROM alunni a JOIN users u ON u.id = a.tutor_id
                       WHERE u.email = 'docente@sentiero.test'";

    // Solo l'amministratore assegna.
    let corpo = format!("_csrf=csrf&classe_id=1&ambito=classe&tutor_id={docente}");
    for email in [
        "tutor@sentiero.test",
        "dirigente@sentiero.test",
        "famiglia@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        let res = post_form(&app, "/admin/assegnazioni", &cookie, &corpo).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
    }
    assert_eq!(conta(&stato, con_docente).await, 0);

    let cookie = sessione(&stato, "admin@sentiero.test").await;
    let invia = |corpo: String| {
        let (app, cookie) = (app.clone(), cookie.clone());
        async move {
            post_form(&app, "/admin/assegnazioni", &cookie, &corpo)
                .await
                .status()
        }
    };

    // Tutor a un singolo alunno; un alunno di un'altra classe nella selezione viene ignorato.
    sqlx::query("INSERT INTO classi (id, nome, anno_scolastico, indirizzo) VALUES (2, '1B', '2026/2027', 'Altro')")
        .execute(&stato.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO alunni (id, classe_id, nome, cognome, data_nascita) VALUES (50, 2, 'Ugo', 'Neri', '2012-01-01')")
        .execute(&stato.db)
        .await
        .unwrap();
    let corpo = format!(
        "_csrf=csrf&classe_id=1&ambito=selezionati&alunno_id=2&alunno_id=50&tutor_id={docente}"
    );
    assert_eq!(invia(corpo).await, StatusCode::SEE_OTHER);
    assert_eq!(conta(&stato, con_docente).await, 1);
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM alunni WHERE id = 50 AND tutor_id IS NULL"
        )
        .await,
        1
    );
    assert_eq!(
        conta(&stato, "SELECT tutor_id FROM alunni WHERE id = 1").await,
        tutor
    );

    // Il tutor deve far parte del CdC della classe: né il dirigente né un docente di un'altra classe.
    for esterno in [dirigente, tutor] {
        let classe = if esterno == tutor { 2 } else { 1 };
        let corpo = format!("_csrf=csrf&classe_id={classe}&ambito=classe&tutor_id={esterno}");
        assert_eq!(invia(corpo).await, StatusCode::BAD_REQUEST);
    }

    // Modello a tutta la classe.
    post_form(
        &app,
        "/admin/modelli",
        &cookie,
        "_csrf=csrf&nome=Secondo&indirizzo=Altro",
    )
    .await;
    assert_eq!(
        invia("_csrf=csrf&classe_id=1&ambito=classe&modello_id=2".into()).await,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        conta(&stato, "SELECT COUNT(*) FROM alunni WHERE modello_id = 2").await,
        3
    );

    // Con un PFI già avviato il modello non cambia, per nessuno degli alunni della richiesta.
    sqlx::query("INSERT INTO pfi (alunno_id, modello_id) VALUES (3, 2)")
        .execute(&stato.db)
        .await
        .unwrap();
    assert_eq!(
        invia("_csrf=csrf&classe_id=1&ambito=classe&modello_id=1".into()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        conta(&stato, "SELECT COUNT(*) FROM alunni WHERE modello_id = 2").await,
        3
    );

    // Richieste incomplete.
    assert_eq!(
        invia("_csrf=csrf&classe_id=1&ambito=classe".into()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        invia("_csrf=csrf&classe_id=1&ambito=selezionati&modello_id=2".into()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        invia("_csrf=csrf&classe_id=1&ambito=classe&modello_id=99".into()).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn compilazione_riservata_al_tutor() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let docente = id_utente(&stato, "docente@sentiero.test").await;
    // L'alunno 3 passa a un altro tutor: il primo non può più toccarlo.
    sqlx::query("UPDATE alunni SET tutor_id = ? WHERE id = 3")
        .bind(docente)
        .execute(&stato.db)
        .await
        .unwrap();

    let casi = [
        ("admin@sentiero.test", 1),
        ("dirigente@sentiero.test", 1),
        ("famiglia@sentiero.test", 1),
        ("docente@sentiero.test", 1), // nel CdC, ma non tutor di questo alunno
        ("tutor@sentiero.test", 3),
        ("tutor@sentiero.test", 999),
    ];
    for (email, alunno) in casi {
        let cookie = sessione(&stato, email).await;
        let modifica = format!("/alunni/{alunno}/pfi/modifica");
        let versioni = format!("/alunni/{alunno}/pfi/versioni");
        assert_eq!(
            get_come(&app, &modifica, Some(&cookie)).await.status(),
            StatusCode::FORBIDDEN,
            "{email}"
        );
        let res = post_form(&app, &modifica, &cookie, "_csrf=csrf&r_1=intrusione").await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
        let res = post_form(&app, &versioni, &cookie, "_csrf=csrf&motivo=revisione").await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
    }
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM pfi").await, 0);
    assert_eq!(
        get_come(&app, "/alunni/1/pfi/modifica", None)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );

    let cookie = sessione(&stato, "tutor@sentiero.test").await;
    assert_eq!(
        get_come(&app, "/alunni/1/pfi/modifica", Some(&cookie))
            .await
            .status(),
        StatusCode::OK
    );
    let res = post_form(
        &app,
        "/alunni/1/pfi/modifica",
        &cookie,
        "_csrf=sbagliato&r_1=x",
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

async fn testo_versione(stato: &AppState, numero: i64) -> String {
    sqlx::query_scalar("SELECT contenuto_json FROM versioni WHERE numero = ?")
        .bind(numero)
        .fetch_one(&stato.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn bozza_e_versioni() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let admin = sessione(&stato, "admin@sentiero.test").await;
    let cookie = sessione(&stato, "tutor@sentiero.test").await;
    post_form(
        &app,
        "/admin/modelli",
        &admin,
        "_csrf=csrf&nome=Secondo&indirizzo=Altro",
    )
    .await;
    let estraneo = conta(
        &stato,
        "SELECT MIN(i.id) FROM indicatori i JOIN sottosezioni ss ON ss.id = i.sottosezione_id
                                  JOIN sezioni s ON s.id = ss.sezione_id WHERE s.modello_id = 2",
    )
    .await;

    // Salvataggio della bozza: l'indicatore di un altro modello viene ignorato.
    let corpo =
        format!("_csrf=csrf&r_1=Primo+testo%0D%0Aseconda+riga&r_{estraneo}=intruso&r_abc=x");
    let res = post_form(&app, "/alunni/1/pfi/modifica", &cookie, &corpo).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM risposte").await, 1);
    let testo: String = sqlx::query_scalar("SELECT testo FROM risposte")
        .fetch_one(&stato.db)
        .await
        .unwrap();
    assert_eq!(testo, "Primo testo\nseconda riga");

    // Con HTMX la risposta è un frammento, non un redirect.
    let req = Request::post("/alunni/1/pfi/modifica")
        .header(header::COOKIE, &cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("HX-Request", "true")
        .body(Body::from("_csrf=csrf&r_2=Altro"))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );

    // Versione 1: contiene il testo salvato e, dove manca, il contenuto proposto dal modello.
    let res = post_form(
        &app,
        "/alunni/1/pfi/versioni",
        &cookie,
        "_csrf=csrf&motivo=boh",
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = post_form(
        &app,
        "/alunni/1/pfi/versioni",
        &cookie,
        "_csrf=csrf&motivo=revisione",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let v1 = testo_versione(&stato, 1).await;
    assert!(
        v1.contains("Primo testo")
            && v1.contains("Il percorso è confermato.")
            && v1.contains("Bianchi Giulia")
    );
    assert!(v1.contains("Elementi conoscitivi della vita scolastica"));

    // Versione 2: salva il testo inviato insieme alla richiesta ed esclude la sezione D disattivata.
    sqlx::query("UPDATE sezioni SET attiva = 0 WHERE modello_id = 1 AND codice = 'D'")
        .execute(&stato.db)
        .await
        .unwrap();
    let res = post_form(
        &app,
        "/alunni/1/pfi/versioni",
        &cookie,
        "_csrf=csrf&motivo=scrutinio&r_1=Testo+rivisto",
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let v2 = testo_versione(&stato, 2).await;
    assert!(v2.contains("Testo rivisto") && !v2.contains("Primo testo"));
    assert!(!v2.contains("Elementi conoscitivi della vita scolastica"));
    // Lo storico resta intatto.
    assert_eq!(testo_versione(&stato, 1).await, v1);
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM versioni WHERE motivo = 'scrutinio' AND numero = 2"
        )
        .await,
        1
    );
    assert_eq!(
        get_come(&app, "/alunni/1/pfi/modifica", Some(&cookie))
            .await
            .status(),
        StatusCode::OK
    );

    // Senza modello assegnato non si compila.
    sqlx::query("UPDATE alunni SET modello_id = NULL WHERE id = 2")
        .execute(&stato.db)
        .await
        .unwrap();
    let res = post_form(&app, "/alunni/2/pfi/modifica", &cookie, "_csrf=csrf&r_1=x").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        get_come(&app, "/alunni/2/pfi/modifica", Some(&cookie))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

async fn corpo(res: Response<Body>) -> Vec<u8> {
    axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

#[tokio::test]
async fn documenti_generati_e_accesso_ai_download() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let tutor = sessione(&stato, "tutor@sentiero.test").await;

    // Due versioni: ognuna ha i suoi file PDF e ODT.
    for testo in ["Prima+stesura", "Seconda+stesura+con+%C3%A0%C3%A8%C3%AC"] {
        let corpo = format!("_csrf=csrf&motivo=revisione&r_1={testo}");
        let res = post_form(&app, "/alunni/1/pfi/versioni", &tutor, &corpo).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
    }
    for file in ["pfi1-v1.pdf", "pfi1-v1.odt", "pfi1-v2.pdf", "pfi1-v2.odt"] {
        assert!(
            stato.config.data_dir.join("documenti").join(file).is_file(),
            "{file}"
        );
    }

    let res = get_come(&app, "/versioni/2/pdf", Some(&tutor)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/pdf");
    assert!(corpo(res).await.starts_with(b"%PDF-"));
    let res = get_come(&app, "/versioni/2/odt", Some(&tutor)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let odt = corpo(res).await;
    assert_eq!(&odt[..2], b"PK");
    assert_eq!(&odt[30..38], b"mimetype");
    assert_eq!(
        get_come(&app, "/versioni/2/exe", Some(&tutor))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_come(&app, "/versioni/99/pdf", Some(&tutor))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Personale che segue l'alunno: docente del CdC e dirigente sì, amministratore no.
    assert_eq!(
        get_come(&app, "/versioni/1/pdf", None).await.status(),
        StatusCode::SEE_OTHER
    );
    for (email, atteso) in [
        ("docente@sentiero.test", StatusCode::OK),
        ("dirigente@sentiero.test", StatusCode::OK),
        ("admin@sentiero.test", StatusCode::FORBIDDEN),
        ("famiglia@sentiero.test", StatusCode::FORBIDDEN),
    ] {
        let cookie = sessione(&stato, email).await;
        for formato in ["pdf", "odt"] {
            let res = get_come(&app, &format!("/versioni/1/{formato}"), Some(&cookie)).await;
            assert_eq!(res.status(), atteso, "{email} {formato}");
        }
    }
    // Un docente fuori dal CdC non vede nulla.
    let docente = id_utente(&stato, "docente@sentiero.test").await;
    sqlx::query("DELETE FROM cdc WHERE docente_id = ?")
        .bind(docente)
        .execute(&stato.db)
        .await
        .unwrap();
    let cookie = sessione(&stato, "docente@sentiero.test").await;
    assert_eq!(
        get_come(&app, "/versioni/1/pdf", Some(&cookie))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    // La famiglia scarica solo la versione resa visibile, e solo quella del proprio alunno.
    sqlx::query("UPDATE versioni SET visibile_famiglia = 1 WHERE numero = 2")
        .execute(&stato.db)
        .await
        .unwrap();
    let famiglia = sessione(&stato, "famiglia@sentiero.test").await;
    assert_eq!(
        get_come(&app, "/versioni/2/pdf", Some(&famiglia))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        get_come(&app, "/versioni/1/pdf", Some(&famiglia))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE alunni SET famiglia_user_id = NULL WHERE id = 1")
        .execute(&stato.db)
        .await
        .unwrap();
    assert_eq!(
        get_come(&app, "/versioni/2/pdf", Some(&famiglia))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

async fn post_file(
    app: &Router,
    uri: &str,
    cookie: &str,
    campi: &[(&str, &str)],
    nome_file: &str,
    dati: &[u8],
) -> Response<Body> {
    let mut corpo = Vec::new();
    for (nome, valore) in campi {
        corpo.extend(
            format!(
                "--confine\r\nContent-Disposition: form-data; name=\"{nome}\"\r\n\r\n{valore}\r\n"
            )
            .bytes(),
        );
    }
    corpo.extend(
        format!(
            "--confine\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{nome_file}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .bytes(),
    );
    corpo.extend(dati);
    corpo.extend(b"\r\n--confine--\r\n");
    let req = Request::post(uri)
        .header(header::COOKIE, cookie)
        .header(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=confine",
        )
        .body(Body::from(corpo))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

const PDF: &[u8] = b"%PDF-1.4 contenuto di prova";

#[tokio::test]
async fn allegati_upload_validato_e_riservato_al_tutor() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let ok = [("_csrf", "csrf"), ("tipo", "PDP")];
    let allegati = "SELECT COUNT(*) FROM allegati";

    for email in [
        "docente@sentiero.test",
        "dirigente@sentiero.test",
        "admin@sentiero.test",
        "famiglia@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        let res = post_file(&app, "/alunni/1/allegati", &cookie, &ok, "pdp.pdf", PDF).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
    }
    let tutor = sessione(&stato, "tutor@sentiero.test").await;
    let invia = |campi: Vec<(&'static str, &'static str)>, nome: &'static str, dati: Vec<u8>| {
        let (app, tutor) = (app.clone(), tutor.clone());
        async move {
            post_file(&app, "/alunni/1/allegati", &tutor, &campi, nome, &dati)
                .await
                .status()
        }
    };
    assert_eq!(
        invia(
            vec![("_csrf", "no"), ("tipo", "PDP")],
            "pdp.pdf",
            PDF.to_vec()
        )
        .await,
        StatusCode::FORBIDDEN
    );
    // Tipo di allegato, estensione e contenuto vengono controllati.
    assert_eq!(
        invia(
            vec![("_csrf", "csrf"), ("tipo", "Altro")],
            "pdp.pdf",
            PDF.to_vec()
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        invia(ok.to_vec(), "programma.exe", PDF.to_vec()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        invia(ok.to_vec(), "finto.pdf", b"MZ eseguibile".to_vec()).await,
        StatusCode::BAD_REQUEST
    );
    // Oltre la dimensione massima.
    let mut enorme = PDF.to_vec();
    enorme.resize(10 * 1024 * 1024 + 1, b' ');
    assert_eq!(
        invia(ok.to_vec(), "enorme.pdf", enorme.clone()).await,
        StatusCode::BAD_REQUEST
    );
    enorme.resize(11 * 1024 * 1024, b' ');
    assert_eq!(
        invia(ok.to_vec(), "enorme.pdf", enorme).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(conta(&stato, allegati).await, 0);
    assert!(
        !stato.config.data_dir.join("allegati").exists()
            || std::fs::read_dir(stato.config.data_dir.join("allegati"))
                .unwrap()
                .next()
                .is_none()
    );

    // Caricamento valido: il nome sul disco è casuale, quello mostrato è ripulito dal percorso.
    assert_eq!(
        invia(ok.to_vec(), "../../segreto/PDP Bianchi.pdf", PDF.to_vec()).await,
        StatusCode::SEE_OTHER
    );
    let (nome, path, mime): (String, String, String) =
        sqlx::query_as("SELECT nome_file, path, mime FROM allegati")
            .fetch_one(&stato.db)
            .await
            .unwrap();
    assert_eq!(
        (nome.as_str(), mime.as_str()),
        ("PDP Bianchi.pdf", "application/pdf")
    );
    assert!(path.starts_with("allegati/") && !path.contains("Bianchi") && !path.contains(".."));
    assert_eq!(
        std::fs::read(stato.config.data_dir.join(&path)).unwrap(),
        PDF
    );
    // Il tutor di un altro alunno non può caricare qui.
    sqlx::query("UPDATE alunni SET tutor_id = NULL WHERE id = 2")
        .execute(&stato.db)
        .await
        .unwrap();
    let res = post_file(&app, "/alunni/2/allegati", &tutor, &ok, "pdp.pdf", PDF).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Eliminazione: solo il tutor, e il file sparisce dal disco.
    for email in [
        "docente@sentiero.test",
        "dirigente@sentiero.test",
        "famiglia@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        let res = post_form(&app, "/allegati/1/elimina", &cookie, "_csrf=csrf").await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
    }
    assert_eq!(
        post_form(&app, "/allegati/1/elimina", &tutor, "_csrf=csrf")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(conta(&stato, allegati).await, 0);
    assert!(!stato.config.data_dir.join(&path).exists());
}

#[tokio::test]
async fn consultazione_di_pfi_e_allegati() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let tutor = sessione(&stato, "tutor@sentiero.test").await;
    post_form(
        &app,
        "/alunni/1/pfi/versioni",
        &tutor,
        "_csrf=csrf&motivo=revisione&r_1=Testo+consultabile",
    )
    .await;
    let campi = [("_csrf", "csrf"), ("tipo", "PEI")];
    assert_eq!(
        post_file(&app, "/alunni/1/allegati", &tutor, &campi, "pei.pdf", PDF)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );

    // Vedono PFI e allegati il tutor, i docenti del CdC e il dirigente.
    for email in [
        "tutor@sentiero.test",
        "docente@sentiero.test",
        "dirigente@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        let res = get_come(&app, "/alunni/1/pfi", Some(&cookie)).await;
        assert_eq!(res.status(), StatusCode::OK, "{email}");
        let html = String::from_utf8(corpo(res).await).unwrap();
        assert!(
            html.contains("Testo consultabile") && html.contains("pei.pdf"),
            "{email}"
        );
        // Il form di pubblicazione compare solo al tutor.
        assert_eq!(
            html.contains("Pubblica un allegato"),
            email == "tutor@sentiero.test",
            "{email}"
        );
        let res = get_come(&app, "/allegati/1", Some(&cookie)).await;
        assert_eq!(res.status(), StatusCode::OK, "{email}");
        assert_eq!(res.headers()["x-content-type-options"], "nosniff");
        assert_eq!(corpo(res).await, PDF);
        assert_eq!(
            get_come(&app, "/", Some(&cookie)).await.status(),
            StatusCode::OK
        );
    }
    // Anche senza versioni la pagina si apre.
    assert_eq!(
        get_come(&app, "/alunni/2/pfi", Some(&tutor)).await.status(),
        StatusCode::OK
    );

    for email in ["admin@sentiero.test", "famiglia@sentiero.test"] {
        let cookie = sessione(&stato, email).await;
        assert_eq!(
            get_come(&app, "/alunni/1/pfi", Some(&cookie))
                .await
                .status(),
            StatusCode::FORBIDDEN,
            "{email}"
        );
        assert_eq!(
            get_come(&app, "/allegati/1", Some(&cookie)).await.status(),
            StatusCode::FORBIDDEN,
            "{email}"
        );
    }
    assert_eq!(
        get_come(&app, "/alunni/1/pfi", None).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        get_come(&app, "/allegati/1", None).await.status(),
        StatusCode::SEE_OTHER
    );

    // La famiglia scarica l'allegato solo se reso visibile, e solo per il proprio alunno.
    let famiglia = sessione(&stato, "famiglia@sentiero.test").await;
    sqlx::query("UPDATE allegati SET visibile_famiglia = 1")
        .execute(&stato.db)
        .await
        .unwrap();
    assert_eq!(
        get_come(&app, "/allegati/1", Some(&famiglia))
            .await
            .status(),
        StatusCode::OK
    );
    sqlx::query("UPDATE alunni SET famiglia_user_id = NULL WHERE id = 1")
        .execute(&stato.db)
        .await
        .unwrap();
    assert_eq!(
        get_come(&app, "/allegati/1", Some(&famiglia))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    // Un docente che esce dal CdC perde la consultazione.
    let docente = id_utente(&stato, "docente@sentiero.test").await;
    sqlx::query("DELETE FROM cdc WHERE docente_id = ?")
        .bind(docente)
        .execute(&stato.db)
        .await
        .unwrap();
    let cookie = sessione(&stato, "docente@sentiero.test").await;
    assert_eq!(
        get_come(&app, "/alunni/1/pfi", Some(&cookie))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get_come(&app, "/allegati/1", Some(&cookie)).await.status(),
        StatusCode::FORBIDDEN
    );
}

async fn pagina_di(app: &Router, uri: &str, cookie: &str) -> String {
    let res = get_come(app, uri, Some(cookie)).await;
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    String::from_utf8(corpo(res).await).unwrap()
}

#[tokio::test]
async fn approvazione_del_cdc() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let tutor = sessione(&stato, "tutor@sentiero.test").await;
    for n in 1..=2 {
        let corpo = format!("_csrf=csrf&motivo=revisione&r_1=Stesura+{n}");
        post_form(&app, "/alunni/1/pfi/versioni", &tutor, &corpo).await;
    }
    let approvate = "SELECT COUNT(*) FROM versioni WHERE approvata_il IS NOT NULL";

    // Non approvano: amministratore, famiglia, un docente del CdC che non è il tutor.
    for email in [
        "admin@sentiero.test",
        "famiglia@sentiero.test",
        "docente@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        let res = post_form(&app, "/versioni/2/approva", &cookie, "_csrf=csrf").await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
    }
    assert_eq!(
        post_form(&app, "/versioni/2/approva", &tutor, "_csrf=no")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(conta(&stato, approvate).await, 0);

    // Solo l'ultima versione; la visibilità richiede l'approvazione.
    assert_eq!(
        post_form(&app, "/versioni/1/approva", &tutor, "_csrf=csrf")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post_form(&app, "/versioni/99/approva", &tutor, "_csrf=csrf")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = post_form(
        &app,
        "/versioni/2/visibilita",
        &tutor,
        "_csrf=csrf&visibile=true",
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    assert_eq!(
        post_form(&app, "/versioni/2/approva", &tutor, "_csrf=csrf")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let id_tutor = id_utente(&stato, "tutor@sentiero.test").await;
    assert_eq!(
        conta(&stato, "SELECT approvata_da FROM versioni WHERE numero = 2").await,
        id_tutor
    );
    // Il dirigente può registrarla a sua volta, ma la prima registrazione resta.
    let dirigente = sessione(&stato, "dirigente@sentiero.test").await;
    assert_eq!(
        post_form(&app, "/versioni/2/approva", &dirigente, "_csrf=csrf")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        conta(&stato, "SELECT approvata_da FROM versioni WHERE numero = 2").await,
        id_tutor
    );
    assert!(
        pagina_di(&app, "/alunni/1/pfi", &dirigente)
            .await
            .contains("Approvata dal Consiglio di Classe")
    );

    // Il dirigente approva per un altro alunno, ma la visibilità la sceglie solo il tutor.
    post_form(
        &app,
        "/alunni/2/pfi/versioni",
        &tutor,
        "_csrf=csrf&motivo=scrutinio",
    )
    .await;
    assert_eq!(
        post_form(&app, "/versioni/3/approva", &dirigente, "_csrf=csrf")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(conta(&stato, approvate).await, 2);
    for email in [
        "dirigente@sentiero.test",
        "docente@sentiero.test",
        "admin@sentiero.test",
        "famiglia@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        let res = post_form(
            &app,
            "/versioni/3/visibilita",
            &cookie,
            "_csrf=csrf&visibile=true",
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
    }
    let res = post_form(
        &app,
        "/versioni/3/visibilita",
        &tutor,
        "_csrf=csrf&visibile=true&ritorno=modifica",
    )
    .await;
    assert_eq!(res.headers()[header::LOCATION], "/alunni/2/pfi/modifica");
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM versioni WHERE visibile_famiglia = 1"
        )
        .await,
        1
    );
    assert!(
        pagina_di(&app, "/alunni/2/pfi/modifica", &tutor)
            .await
            .contains("Nascondi")
    );
}

#[tokio::test]
async fn famiglia_vede_solo_il_visibile_e_appone_il_visto() {
    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let tutor = sessione(&stato, "tutor@sentiero.test").await;
    let famiglia = sessione(&stato, "famiglia@sentiero.test").await;
    // Una seconda famiglia, collegata all'alunno 2.
    sqlx::query(
        "INSERT INTO users (email, password_hash, ruolo, nome, cognome)
         VALUES ('altra@example.test', 'x', 'famiglia', 'Famiglia', 'Esposito')",
    )
    .execute(&stato.db)
    .await
    .unwrap();
    let altra_id = id_utente(&stato, "altra@example.test").await;
    sqlx::query("UPDATE alunni SET famiglia_user_id = ? WHERE id = 2")
        .bind(altra_id)
        .execute(&stato.db)
        .await
        .unwrap();
    let altra = sessione(&stato, "altra@example.test").await;

    // La home della famiglia è la sua area; il personale non vi entra.
    let res = get_come(&app, "/", Some(&famiglia)).await;
    assert_eq!(res.headers()[header::LOCATION], "/famiglia");
    for email in [
        "tutor@sentiero.test",
        "dirigente@sentiero.test",
        "admin@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        assert_eq!(
            get_come(&app, "/famiglia", Some(&cookie)).await.status(),
            StatusCode::FORBIDDEN,
            "{email}"
        );
        let res = post_form(&app, "/famiglia/versioni/1/visto", &cookie, "_csrf=csrf").await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{email}");
    }
    assert_eq!(
        get_come(&app, "/famiglia", None).await.status(),
        StatusCode::SEE_OTHER
    );

    // Versione 1 approvata ma non ancora visibile: la famiglia non vede nulla e non può vistare.
    post_form(
        &app,
        "/alunni/1/pfi/versioni",
        &tutor,
        "_csrf=csrf&motivo=revisione&r_1=Testo+approvato",
    )
    .await;
    post_form(&app, "/versioni/1/approva", &tutor, "_csrf=csrf").await;
    let html = pagina_di(&app, "/famiglia", &famiglia).await;
    assert!(html.contains("Giulia Bianchi") && !html.contains("Testo approvato"));
    assert!(!html.contains("Esposito"));
    assert_eq!(
        post_form(&app, "/famiglia/versioni/1/visto", &famiglia, "_csrf=csrf")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get_come(&app, "/versioni/1/pdf", Some(&famiglia))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    // Resa visibile: la vede solo la famiglia di quell'alunno.
    post_form(
        &app,
        "/versioni/1/visibilita",
        &tutor,
        "_csrf=csrf&visibile=true",
    )
    .await;
    let html = pagina_di(&app, "/famiglia", &famiglia).await;
    assert!(html.contains("Testo approvato") && html.contains("Apponi il visto"));
    assert_eq!(
        get_come(&app, "/versioni/1/pdf", Some(&famiglia))
            .await
            .status(),
        StatusCode::OK
    );
    assert!(
        !pagina_di(&app, "/famiglia", &altra)
            .await
            .contains("Testo approvato")
    );
    assert_eq!(
        get_come(&app, "/versioni/1/pdf", Some(&altra))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_form(&app, "/famiglia/versioni/1/visto", &altra, "_csrf=csrf")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_form(&app, "/famiglia/versioni/1/visto", &famiglia, "_csrf=no")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM visti").await, 0);

    // Visto: registrati utente e data; ripeterlo non crea doppioni.
    for _ in 0..2 {
        let res = post_form(&app, "/famiglia/versioni/1/visto", &famiglia, "_csrf=csrf").await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
    }
    let id_famiglia = id_utente(&stato, "famiglia@sentiero.test").await;
    let (utente, data): (i64, String) = sqlx::query_as("SELECT user_id, data FROM visti")
        .fetch_one(&stato.db)
        .await
        .unwrap();
    assert_eq!(utente, id_famiglia);
    assert_eq!(data.len(), "2026-01-01 00:00:00".len());
    assert!(
        pagina_di(&app, "/famiglia", &famiglia)
            .await
            .contains("Visto apposto il")
    );
    assert!(
        pagina_di(&app, "/alunni/1/pfi", &tutor)
            .await
            .contains("Visto della famiglia: ")
    );

    // Una versione successiva non ancora pubblicata non sostituisce quella visibile.
    post_form(
        &app,
        "/alunni/1/pfi/versioni",
        &tutor,
        "_csrf=csrf&motivo=scrutinio&r_1=Bozza+riservata",
    )
    .await;
    let html = pagina_di(&app, "/famiglia", &famiglia).await;
    assert!(html.contains("Testo approvato") && !html.contains("Bozza riservata"));
    assert_eq!(
        get_come(&app, "/versioni/2/pdf", Some(&famiglia))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    // Approvata e pubblicata, diventa quella mostrata.
    post_form(&app, "/versioni/2/approva", &tutor, "_csrf=csrf").await;
    post_form(
        &app,
        "/versioni/2/visibilita",
        &tutor,
        "_csrf=csrf&visibile=true",
    )
    .await;
    let html = pagina_di(&app, "/famiglia", &famiglia).await;
    assert!(html.contains("Bozza riservata") && html.contains("Apponi il visto"));

    // Allegati: visibilità scelta per ogni singolo file, solo dal tutor.
    for nome in ["pdp.pdf", "pei.pdf"] {
        post_file(
            &app,
            "/alunni/1/allegati",
            &tutor,
            &[("_csrf", "csrf"), ("tipo", "PDP")],
            nome,
            PDF,
        )
        .await;
    }
    for cookie in [
        &famiglia,
        &sessione(&stato, "dirigente@sentiero.test").await,
    ] {
        let res = post_form(
            &app,
            "/allegati/1/visibilita",
            cookie,
            "_csrf=csrf&visibile=true",
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(
        post_form(
            &app,
            "/allegati/1/visibilita",
            &tutor,
            "_csrf=csrf&visibile=true"
        )
        .await
        .status(),
        StatusCode::SEE_OTHER
    );
    let html = pagina_di(&app, "/famiglia", &famiglia).await;
    assert!(html.contains("pdp.pdf") && !html.contains("pei.pdf"));
    assert_eq!(
        get_come(&app, "/allegati/1", Some(&famiglia))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        get_come(&app, "/allegati/2", Some(&famiglia))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get_come(&app, "/allegati/1", Some(&altra)).await.status(),
        StatusCode::FORBIDDEN
    );

    // Nascosta di nuovo, la versione sparisce dall'area della famiglia.
    post_form(
        &app,
        "/versioni/2/visibilita",
        &tutor,
        "_csrf=csrf&visibile=false",
    )
    .await;
    assert!(
        !pagina_di(&app, "/famiglia", &famiglia)
            .await
            .contains("Bozza riservata")
    );
}

#[tokio::test]
async fn export_solo_pfi_approvati_e_vistati() {
    use std::io::Read;

    let stato = stato().await;
    let app = sentiero::app(stato.clone());
    let tutor = sessione(&stato, "tutor@sentiero.test").await;
    let famiglia = sessione(&stato, "famiglia@sentiero.test").await;
    let dirigente = sessione(&stato, "dirigente@sentiero.test").await;

    // Alunno 1: v1 approvata, visibile e vistata; v2 approvata e visibile ma senza visto; v3 in bozza.
    for n in 1..=3 {
        let corpo = format!("_csrf=csrf&motivo=revisione&r_1=Stesura+{n}");
        post_form(&app, "/alunni/1/pfi/versioni", &tutor, &corpo).await;
        if n < 3 {
            post_form(
                &app,
                &format!("/versioni/{n}/approva"),
                &tutor,
                "_csrf=csrf",
            )
            .await;
            post_form(
                &app,
                &format!("/versioni/{n}/visibilita"),
                &tutor,
                "_csrf=csrf&visibile=true",
            )
            .await;
        }
        if n == 1 {
            let res = post_form(&app, "/famiglia/versioni/1/visto", &famiglia, "_csrf=csrf").await;
            assert_eq!(res.status(), StatusCode::SEE_OTHER);
        }
    }
    // Alunno 2: approvata, mai vistata. Alunno 3: mai approvata.
    post_form(
        &app,
        "/alunni/2/pfi/versioni",
        &tutor,
        "_csrf=csrf&motivo=scrutinio",
    )
    .await;
    post_form(&app, "/versioni/4/approva", &tutor, "_csrf=csrf").await;
    post_form(
        &app,
        "/alunni/3/pfi/versioni",
        &tutor,
        "_csrf=csrf&motivo=scrutinio",
    )
    .await;
    assert_eq!(conta(&stato, "SELECT COUNT(*) FROM versioni").await, 5);
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM versioni WHERE approvata_il IS NOT NULL"
        )
        .await,
        3
    );

    // Solo il dirigente esporta.
    assert_eq!(
        get_come(&app, "/export?classe_id=1", None).await.status(),
        StatusCode::SEE_OTHER
    );
    for email in [
        "tutor@sentiero.test",
        "docente@sentiero.test",
        "admin@sentiero.test",
        "famiglia@sentiero.test",
    ] {
        let cookie = sessione(&stato, email).await;
        for uri in ["/export?classe_id=1", "/export?alunno_id=1"] {
            assert_eq!(
                get_come(&app, uri, Some(&cookie)).await.status(),
                StatusCode::FORBIDDEN,
                "{email}"
            );
        }
    }

    // Per classe e per alunno: dentro c'è solo la versione 1 dell'alunno 1, più l'indice.
    for uri in ["/export?classe_id=1", "/export?alunno_id=1"] {
        let res = get_come(&app, uri, Some(&dirigente)).await;
        assert_eq!(res.status(), StatusCode::OK, "{uri}");
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/zip");
        let mut archivio = zip::ZipArchive::new(std::io::Cursor::new(corpo(res).await)).unwrap();
        let mut nomi: Vec<String> = archivio
            .file_names()
            .map(|n| n.unwrap().into_owned())
            .collect();
        nomi.sort();
        assert_eq!(
            nomi,
            [
                "1A/Bianchi_Giulia_1/PFI-v1.odt",
                "1A/Bianchi_Giulia_1/PFI-v1.pdf",
                "indice.csv"
            ],
            "{uri}"
        );

        let mut pdf = Vec::new();
        archivio
            .by_name("1A/Bianchi_Giulia_1/PFI-v1.pdf")
            .unwrap()
            .read_to_end(&mut pdf)
            .unwrap();
        assert_eq!(
            pdf,
            std::fs::read(stato.config.data_dir.join("documenti/pfi1-v1.pdf")).unwrap()
        );
        let mut indice = String::new();
        archivio
            .by_name("indice.csv")
            .unwrap()
            .read_to_string(&mut indice)
            .unwrap();
        assert_eq!(indice.lines().count(), 2);
        assert!(
            indice.contains("Bianchi;Giulia;1;revisione;") && indice.contains("Famiglia Bianchi")
        );
        assert!(
            indice.contains("Tommaso Tutor")
                && !indice.contains("Esposito")
                && !indice.contains("Colombo")
        );
    }

    // Niente da esportare, o richiesta ambigua.
    for uri in [
        "/export?alunno_id=2",
        "/export?alunno_id=3",
        "/export?classe_id=99",
        "/export",
        "/export?alunno_id=1&classe_id=1",
    ] {
        assert_eq!(
            get_come(&app, uri, Some(&dirigente)).await.status(),
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }
    assert_eq!(
        get_come(&app, "/", Some(&dirigente)).await.status(),
        StatusCode::OK
    );

    // Dopo il visto anche la versione 2 entra nell'archivio.
    post_form(&app, "/famiglia/versioni/2/visto", &famiglia, "_csrf=csrf").await;
    let res = get_come(&app, "/export?classe_id=1", Some(&dirigente)).await;
    let archivio = zip::ZipArchive::new(std::io::Cursor::new(corpo(res).await)).unwrap();
    assert_eq!(archivio.len(), 5);
}

#[tokio::test]
async fn pfi_di_esempio_completo() {
    let stato = stato().await;
    seed::pfi_di_esempio(&stato.db, &stato.config.data_dir)
        .await
        .unwrap();

    // Ogni indicatore del modello ha un testo scritto apposta, non il contenuto proposto.
    let indicatori = conta(&stato, "SELECT COUNT(*) FROM indicatori").await;
    assert_eq!(
        conta(&stato, "SELECT COUNT(*) FROM risposte WHERE testo <> ''").await,
        indicatori
    );
    // Versione 1 con i suoi documenti, ancora da approvare.
    assert_eq!(
        conta(
            &stato,
            "SELECT COUNT(*) FROM versioni WHERE numero = 1 AND approvata_il IS NULL"
        )
        .await,
        1
    );
    for file in ["pfi1-v1.pdf", "pfi1-v1.odt"] {
        assert!(
            stato.config.data_dir.join("documenti").join(file).is_file(),
            "{file}"
        );
    }
    let app = sentiero::app(stato.clone());
    let tutor = sessione(&stato, "tutor@sentiero.test").await;
    let html = pagina_di(&app, "/alunni/1/pfi", &tutor).await;
    assert!(
        html.contains("Il negozio in classe")
            && html.contains("approvazione del Consiglio di Classe</button>")
    );
}
