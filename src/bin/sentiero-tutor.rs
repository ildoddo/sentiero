//! Sentiero per uso personale del tutor: i dati restano sul computer e non ci sono account né
//! password. Con la funzionalità `finestra` l'interfaccia si apre in una finestra propria,
//! altrimenti nel browser predefinito.

// Su Windows l'app con la finestra non deve aprire anche una console. Solo in release: in debug
// la console serve a leggere i messaggi.
#![cfg_attr(
    all(windows, feature = "finestra", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::{ffi::OsStr, path::PathBuf, process::Command, sync::Arc};

use sentiero::{AppState, apri_db, auth::token_casuale, config::Config, seed};

type Errore = Box<dyn std::error::Error>;

fn main() -> Result<(), Errore> {
    tracing_subscriber::fmt::init();
    // Runtime esplicito e `main` sincrono: su macOS la finestra deve vivere nel thread principale.
    let runtime = tokio::runtime::Runtime::new()?;
    let (listener, state, url) = runtime.block_on(prepara())?;
    println!(
        "I dati di Sentiero sono in: {}",
        state.config.data_dir.display()
    );
    let server = runtime.spawn(async move {
        axum::serve(listener, sentiero::app(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    });

    #[cfg(feature = "finestra")]
    {
        // Il server prosegue sul runtime anche senza il suo handle.
        drop(server);
        // Non ritorna: alla chiusura della finestra il programma termina, e con lui il server.
        finestra(&url)
    }
    #[cfg(not(feature = "finestra"))]
    {
        println!("Sentiero è aperto nel browser. Se non si apre, visita: {url}");
        println!("Per chiudere Sentiero premi Ctrl+C in questa finestra.");
        apri_con_il_sistema(OsStr::new(&url));
        runtime.block_on(server)??;
        Ok(())
    }
}

/// Apre il database dell'utente, lo inizializza al primo avvio e mette il server in ascolto.
/// Restituisce anche il link di avvio, che contiene il token d'ingresso di questa esecuzione.
async fn prepara() -> Result<(tokio::net::TcpListener, AppState, String), Errore> {
    let data_dir = cartella_dati();
    std::fs::create_dir_all(&data_dir)?;
    // Solo su questo computer, su una porta libera scelta dal sistema.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let indirizzo = listener.local_addr()?;
    let token = token_casuale()?;
    let config = Config {
        bind: indirizzo.to_string(),
        database_url: format!("sqlite://{}/sentiero.db", data_dir.display()),
        data_dir,
        cookie_secure: false,
        seed_password: None,
        avvio_personale: Some(token.clone()),
    };
    let db = apri_db(&config.database_url).await?;
    let utenti: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&db)
        .await?;
    if utenti == 0 {
        seed::personale(&db).await?;
    }
    let state = AppState {
        db,
        config: Arc::new(config),
    };
    Ok((listener, state, format!("http://{indirizzo}/avvio/{token}")))
}

/// Cartella dei dati dell'utente secondo le convenzioni del sistema, o `SENTIERO_DATA_DIR` se impostata.
fn cartella_dati() -> PathBuf {
    if let Some(scelta) = std::env::var_os("SENTIERO_DATA_DIR") {
        return scelta.into();
    }
    let casa = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        casa.join("Library/Application Support/Sentiero")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("Sentiero")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map_or_else(|| casa.join(".local/share"), PathBuf::from)
            .join("sentiero")
    }
}

/// Apre un indirizzo o un file con il programma predefinito del sistema.
fn apri_con_il_sistema(cosa: &OsStr) {
    let esito = if cfg!(target_os = "macos") {
        Command::new("open").arg(cosa).spawn()
    } else if cfg!(windows) {
        // Il primo argomento vuoto è il titolo della finestra, che `start` altrimenti prenderebbe dal percorso.
        let mut comando = Command::new("cmd");
        comando.args([OsStr::new("/C"), OsStr::new("start"), OsStr::new(""), cosa]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: senza, `cmd` fa comparire per un attimo una console.
            comando.creation_flags(0x0800_0000);
        }
        comando.spawn()
    } else {
        Command::new("xdg-open").arg(cosa).spawn()
    };
    if let Err(e) = esito {
        tracing::warn!(errore = %e, "impossibile aprire con il programma predefinito");
    }
}

#[cfg(feature = "finestra")]
fn finestra(url: &str) -> Result<(), Errore> {
    use std::{cell::RefCell, rc::Rc};

    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoop},
        window::WindowBuilder,
    };

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Sentiero")
        .with_inner_size(LogicalSize::new(1100.0, 800.0))
        .build(&event_loop)?;

    #[cfg(target_os = "macos")]
    let _menu = menu()?;

    // PDF, ODT e allegati finiscono nella cartella Download e si aprono appena scaricati: in una
    // finestra senza barra dei download l'utente non avrebbe altro segnale.
    let scaricato = Rc::new(RefCell::new(None::<PathBuf>));
    let in_corso = Rc::clone(&scaricato);
    let builder = wry::WebViewBuilder::new()
        .with_url(url)
        .with_download_started_handler(move |_, destinazione| {
            *in_corso.borrow_mut() = Some(destinazione.clone());
            true
        })
        .with_download_completed_handler(move |_, _, riuscito| {
            // shortcut: si ricorda solo l'ultimo download avviato, quindi con due download contemporanei se ne apre uno; tenere una mappa per URL se servirà.
            if let (true, Some(file)) = (riuscito, scaricato.borrow_mut().take()) {
                apri_con_il_sistema(file.as_os_str());
            }
        });

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let _webview = builder.build(&window)?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or("finestra GTK senza contenitore")?;
        builder.build_gtk(vbox)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    })
}

/// Su macOS le scorciatoie da tastiera (copia, incolla, esci) arrivano alla pagina solo attraverso
/// le voci di menu. Il primo sottomenu è quello dell'applicazione.
#[cfg(all(feature = "finestra", target_os = "macos"))]
fn menu() -> muda::Result<muda::Menu> {
    use muda::{Menu, PredefinedMenuItem as Voce, Submenu};

    let applicazione = Submenu::with_items(
        "Sentiero",
        true,
        &[
            &Voce::hide(Some("Nascondi Sentiero")),
            &Voce::separator(),
            &Voce::quit(Some("Esci da Sentiero")),
        ],
    )?;
    let modifica = Submenu::with_items(
        "Modifica",
        true,
        &[
            &Voce::undo(Some("Annulla")),
            &Voce::redo(Some("Ripristina")),
            &Voce::separator(),
            &Voce::cut(Some("Taglia")),
            &Voce::copy(Some("Copia")),
            &Voce::paste(Some("Incolla")),
            &Voce::select_all(Some("Seleziona tutto")),
        ],
    )?;
    let menu = Menu::with_items(&[&applicazione, &modifica])?;
    menu.init_for_nsapp();
    Ok(menu)
}
