use sqlx::SqlitePool;

use std::path::Path;

use crate::{auth::crea_hash, error::AppError, modello, routes::pfi};

pub const UTENTI: [(&str, &str, &str, &str); 5] = [
    ("admin@sentiero.test", "admin", "Anna", "Amministratrice"),
    ("tutor@sentiero.test", "docente", "Tommaso", "Tutor"),
    ("docente@sentiero.test", "docente", "Donata", "Docente"),
    ("dirigente@sentiero.test", "dirigente", "Dario", "Dirigente"),
    ("famiglia@sentiero.test", "famiglia", "Famiglia", "Bianchi"),
];

/// Dati di esempio per provare l'app; tutti gli utenti condividono la stessa password.
pub async fn esegui(db: &SqlitePool, password: &str) -> Result<(), AppError> {
    let hash = crea_hash(password).await?;

    let mut tx = db.begin().await?;
    for (email, ruolo, nome, cognome) in UTENTI {
        sqlx::query(
            "INSERT INTO users (email, password_hash, ruolo, nome, cognome) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(email)
        .bind(&hash)
        .bind(ruolo)
        .bind(nome)
        .bind(cognome)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO classi (id, nome, anno_scolastico, indirizzo)
         VALUES (1, '1A', '2026/2027', 'Servizi commerciali')",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO cdc (classe_id, docente_id)
         SELECT 1, id FROM users WHERE ruolo = 'docente'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO alunni (classe_id, nome, cognome, data_nascita, famiglia_user_id) VALUES
         (1, 'Giulia', 'Bianchi', '2012-03-14', (SELECT id FROM users WHERE ruolo = 'famiglia')),
         (1, 'Marco', 'Esposito', '2012-07-02', NULL),
         (1, 'Sara', 'Colombo', '2012-11-23', NULL)",
    )
    .execute(&mut *tx)
    .await?;
    let modello_id = modello::crea(&mut tx, "Modello base", "Servizi commerciali").await?;
    sqlx::query(
        "UPDATE alunni SET modello_id = ?,
             tutor_id = (SELECT id FROM users WHERE email = 'tutor@sentiero.test')",
    )
    .bind(modello_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Testi del PFI di esempio, per etichetta dell'indicatore del modello base. Persona e fatti sono inventati.
const PFI_DI_ESEMPIO: &[(&str, &str)] = &[
    (
        "Dati anagrafici",
        "Giulia Bianchi, nata il 14 marzo 2012 a Verona, cittadinanza italiana.",
    ),
    (
        "Recapiti della famiglia",
        "Madre e padre, entrambi reperibili tramite l'account famiglia collegato e il numero indicato in segreteria.",
    ),
    (
        "Scuola di provenienza",
        "Scuola secondaria di primo grado dell'Istituto Comprensivo del quartiere.",
    ),
    (
        "Esito del primo ciclo",
        "Esame conclusivo superato con voto 7/10. Il consiglio orientativo indicava un percorso professionale nell'area dei servizi.",
    ),
    ("Ripetenze o cambi di indirizzo", "Nessuno."),
    (
        "Profilo sintetico dell'alunno",
        "Alunna riservata ma collaborativa, puntuale nelle consegne. Lavora volentieri in piccolo gruppo e preferisce le attività pratiche alle lezioni frontali.\nSi è inserita bene nella classe fin dalle prime settimane.",
    ),
    (
        "Bisogni educativi e documenti collegati",
        "Non sono presenti certificazioni. Nessun PDP o PEI da allegare.",
    ),
    (
        "Competenze certificate al termine del primo ciclo",
        "Livello intermedio nelle competenze alfabetiche funzionali e digitali; livello base in quelle matematiche e nella lingua inglese.",
    ),
    (
        "Esiti delle prove d'ingresso",
        "Italiano: comprensione del testo adeguata, ortografia da consolidare.\nMatematica: difficoltà con le frazioni e le percentuali.\nInglese: lessico di base presente, produzione orale incerta.",
    ),
    (
        "Attività extrascolastiche",
        "Pallavolo in una società sportiva del quartiere, due allenamenti a settimana. Frequenta un corso di disegno digitale.",
    ),
    (
        "Esperienze di lavoro o volontariato",
        "Aiuta saltuariamente nel negozio di famiglia, dove ha imparato a usare il registratore di cassa e ad accogliere i clienti.",
    ),
    (
        "Punti di forza e potenzialità",
        "Ordine e precisione nel lavoro, buona manualità con gli strumenti digitali, disponibilità ad aiutare i compagni. Mostra interesse per la grafica e per la relazione con il cliente.",
    ),
    (
        "Carenze e aree da rafforzare",
        "Calcolo con percentuali e proporzioni, esposizione orale davanti alla classe, lessico tecnico in inglese.",
    ),
    (
        "Aspettative sul percorso di studi",
        "Nel colloquio iniziale Giulia e la famiglia hanno espresso l'obiettivo di conseguire la qualifica e proseguire fino al diploma, con interesse per la comunicazione commerciale.",
    ),
    (
        "Obiettivi educativi e di apprendimento",
        "Consolidare il calcolo percentuale applicato a sconti, IVA e ricarichi.\nEsporre un argomento per tre minuti con il supporto di una scaletta.\nAcquisire il lessico inglese di base dell'accoglienza e della vendita.",
    ),
    (
        "Attività di recupero e di potenziamento",
        "Sportello di matematica un'ora a settimana nel primo periodo. Potenziamento di grafica digitale nel laboratorio pomeridiano.",
    ),
    (
        "Unità di apprendimento e progetti dedicati",
        "UdA \"Il negozio in classe\": allestimento di un punto vendita simulato, con listino, volantino e scheda cliente.",
    ),
    (
        "Metodologie e strumenti",
        "Lavoro cooperativo, compiti di realtà, mappe e schemi forniti in anticipo, verifiche orali programmate.",
    ),
    (
        "Ore dedicate alla personalizzazione",
        "40 ore nel primo anno: 20 di recupero in matematica, 12 di potenziamento in grafica, 8 di accompagnamento allo studio con il tutor.",
    ),
    (
        "Attività di orientamento",
        "Incontro con ex studenti dell'indirizzo e visita a un'azienda della grande distribuzione nel secondo periodo.",
    ),
    (
        "Stage, PCTO o apprendistato",
        "Non previsti nel primo anno. Dal secondo anno stage osservativo di una settimana in un'attività commerciale.",
    ),
    (
        "Frequenza",
        "Regolare: 6 giorni di assenza nel primo periodo, tutti giustificati.",
    ),
    (
        "Partecipazione e comportamento",
        "Rispetta le regole e i tempi di lavoro. Interviene poco spontaneamente, ma risponde in modo pertinente quando è sollecitata.",
    ),
    (
        "Colloqui e comunicazioni",
        "Colloquio iniziale con entrambi i genitori a ottobre; un colloquio intermedio a dicembre. La famiglia segue con regolarità le comunicazioni.",
    ),
    (
        "Esiti del primo periodo",
        "Profitto sufficiente o più che sufficiente in tutte le discipline, con una insufficienza non grave in matematica (5).",
    ),
    (
        "Esiti dello scrutinio finale",
        "Da compilare allo scrutinio finale.",
    ),
    (
        "Livelli di competenza raggiunti",
        "Asse dei linguaggi: intermedio. Asse matematico: base, in miglioramento. Asse professionale: intermedio.",
    ),
    (
        "Carenze formative e modalità di recupero",
        "Matematica: percentuali e proporzioni. Recupero con lo sportello settimanale e verifica a fine febbraio.",
    ),
    (
        "Conferma del percorso, riorientamento o passaggio",
        "Il percorso è confermato.",
    ),
];

/// Completa il PFI di Giulia Bianchi e ne genera la versione 1, ancora da approvare: così si
/// possono provare subito approvazione, pubblicazione, visto ed export.
pub async fn pfi_di_esempio(db: &SqlitePool, data_dir: &Path) -> Result<(), AppError> {
    let mut tx = db.begin().await?;
    let (alunno_id, tutor_id): (i64, i64) = sqlx::query_as(
        "SELECT id, tutor_id FROM alunni WHERE cognome = 'Bianchi' AND nome = 'Giulia' AND tutor_id IS NOT NULL",
    )
    .fetch_one(&mut *tx)
    .await?;
    let mut campi = Vec::new();
    for (etichetta, testo) in PFI_DI_ESEMPIO {
        let indicatore_id: i64 = sqlx::query_scalar(
            "SELECT i.id FROM indicatori i
             JOIN sottosezioni ss ON ss.id = i.sottosezione_id
             JOIN sezioni s ON s.id = ss.sezione_id
             JOIN alunni a ON a.modello_id = s.modello_id
             WHERE a.id = ? AND i.etichetta = ?",
        )
        .bind(alunno_id)
        .bind(etichetta)
        .fetch_one(&mut *tx)
        .await?;
        campi.push((format!("r_{indicatore_id}"), (*testo).to_owned()));
    }
    let pfi_id = pfi::salva_risposte(&mut tx, alunno_id, &campi).await?;
    pfi::congela_versione(&mut tx, data_dir, alunno_id, pfi_id, "revisione", tutor_id).await?;
    tx.commit().await?;
    Ok(())
}
