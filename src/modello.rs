//! Struttura di un modello PFI: sezioni A–E, sottosezioni e indicatori.

use sqlx::SqliteConnection;

pub const MAX_PERSONALIZZATE: i64 = 6;

type Indicatori = &'static [(&'static str, &'static str)];
type Sottosezioni = &'static [(&'static str, Indicatori)];

/// Contenuti proposti per ogni nuovo modello: (codice, titolo, sottosezioni con indicatore e testo proposto).
/// Testi originali di Sentiero, modificabili dall'amministratore modello per modello.
const STRUTTURA_BASE: &[(&str, &str, Sottosezioni)] = &[
    (
        "A",
        "Dati anagrafici e informazioni di presentazione",
        &[
            (
                "Dati dell'alunno",
                &[
                    (
                        "Dati anagrafici",
                        "Nome, cognome, data e luogo di nascita, cittadinanza.",
                    ),
                    (
                        "Recapiti della famiglia",
                        "Riferimenti dei genitori o di chi esercita la responsabilità genitoriale.",
                    ),
                ],
            ),
            (
                "Percorso scolastico precedente",
                &[
                    ("Scuola di provenienza", ""),
                    (
                        "Esito del primo ciclo",
                        "Voto dell'esame conclusivo e consiglio orientativo ricevuto.",
                    ),
                    ("Ripetenze o cambi di indirizzo", "Nessuno."),
                ],
            ),
            (
                "Presentazione",
                &[
                    ("Profilo sintetico dell'alunno", ""),
                    (
                        "Bisogni educativi e documenti collegati",
                        "Indicare l'eventuale presenza di PDP o PEI, pubblicati tra gli allegati.",
                    ),
                ],
            ),
        ],
    ),
    (
        "B",
        "Bilancio Personale Iniziale",
        &[
            (
                "Competenze acquisite in contesti formali",
                &[
                    ("Competenze certificate al termine del primo ciclo", ""),
                    ("Esiti delle prove d'ingresso", ""),
                ],
            ),
            (
                "Competenze acquisite in contesti non formali e informali",
                &[
                    (
                        "Attività extrascolastiche",
                        "Sport, musica, associazioni, corsi.",
                    ),
                    ("Esperienze di lavoro o volontariato", ""),
                ],
            ),
            (
                "Interessi, attitudini e aspirazioni",
                &[
                    ("Punti di forza e potenzialità", ""),
                    ("Carenze e aree da rafforzare", ""),
                    (
                        "Aspettative sul percorso di studi",
                        "Raccolte nel colloquio iniziale con l'alunno e la famiglia.",
                    ),
                ],
            ),
        ],
    ),
    (
        "C",
        "Personalizzazione degli apprendimenti",
        &[
            (
                "Obiettivi formativi",
                &[("Obiettivi educativi e di apprendimento", "")],
            ),
            (
                "Interventi di personalizzazione",
                &[
                    ("Attività di recupero e di potenziamento", ""),
                    ("Unità di apprendimento e progetti dedicati", ""),
                    ("Metodologie e strumenti", ""),
                    (
                        "Ore dedicate alla personalizzazione",
                        "Quota oraria prevista nel biennio e sua distribuzione.",
                    ),
                ],
            ),
            (
                "Orientamento ed esperienze nel mondo del lavoro",
                &[
                    ("Attività di orientamento", ""),
                    ("Stage, PCTO o apprendistato", ""),
                ],
            ),
        ],
    ),
    (
        "D",
        "Elementi conoscitivi della vita scolastica",
        &[
            (
                "Frequenza e partecipazione",
                &[
                    ("Frequenza", "Regolare."),
                    ("Partecipazione e comportamento", ""),
                ],
            ),
            (
                "Rapporti tra scuola e famiglia",
                &[("Colloqui e comunicazioni", "")],
            ),
        ],
    ),
    (
        "E",
        "Valutazioni",
        &[
            (
                "Valutazioni periodiche",
                &[
                    ("Esiti del primo periodo", ""),
                    ("Esiti dello scrutinio finale", ""),
                ],
            ),
            (
                "Competenze e carenze",
                &[
                    ("Livelli di competenza raggiunti", ""),
                    ("Carenze formative e modalità di recupero", ""),
                ],
            ),
            (
                "Revisione del progetto",
                &[(
                    "Conferma del percorso, riorientamento o passaggio",
                    "Il percorso è confermato.",
                )],
            ),
        ],
    ),
];

/// Crea un modello con la struttura base e ne restituisce l'id.
pub async fn crea(
    conn: &mut SqliteConnection,
    nome: &str,
    indirizzo: &str,
) -> Result<i64, sqlx::Error> {
    let modello_id: i64 =
        sqlx::query_scalar("INSERT INTO modelli (nome, indirizzo) VALUES (?, ?) RETURNING id")
            .bind(nome)
            .bind(indirizzo)
            .fetch_one(&mut *conn)
            .await?;
    for (i, (codice, titolo, sottosezioni)) in STRUTTURA_BASE.iter().enumerate() {
        let sezione_id: i64 = sqlx::query_scalar(
            "INSERT INTO sezioni (modello_id, codice, titolo, ordine) VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(modello_id)
        .bind(codice)
        .bind(titolo)
        .bind(i as i64)
        .fetch_one(&mut *conn)
        .await?;
        for (j, (titolo, indicatori)) in sottosezioni.iter().enumerate() {
            let sottosezione_id: i64 = sqlx::query_scalar(
                "INSERT INTO sottosezioni (sezione_id, titolo, ordine) VALUES (?, ?, ?) RETURNING id",
            )
            .bind(sezione_id)
            .bind(titolo)
            .bind(j as i64)
            .fetch_one(&mut *conn)
            .await?;
            for (k, (etichetta, testo)) in indicatori.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO indicatori (sottosezione_id, etichetta, testo_proposto, ordine) VALUES (?, ?, ?, ?)",
                )
                .bind(sottosezione_id)
                .bind(etichetta)
                .bind(testo)
                .bind(k as i64)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    Ok(modello_id)
}

#[derive(Debug, sqlx::FromRow)]
pub struct Sezione {
    pub id: i64,
    pub codice: String,
    pub titolo: String,
    pub attiva: bool,
    #[sqlx(skip)]
    pub sottosezioni: Vec<Sottosezione>,
}

impl Sezione {
    pub fn personalizzate(&self) -> usize {
        self.sottosezioni
            .iter()
            .filter(|s| s.personalizzata)
            .count()
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct Sottosezione {
    pub id: i64,
    pub sezione_id: i64,
    pub titolo: String,
    pub personalizzata: bool,
    pub attiva: bool,
    #[sqlx(skip)]
    pub indicatori: Vec<Indicatore>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Indicatore {
    pub id: i64,
    pub sottosezione_id: i64,
    pub etichetta: String,
    pub testo_proposto: String,
}

/// Carica l'intera struttura del modello, comprese le parti disattivate.
pub async fn carica(
    db: &mut SqliteConnection,
    modello_id: i64,
) -> Result<Vec<Sezione>, sqlx::Error> {
    let mut sezioni: Vec<Sezione> = sqlx::query_as(
        "SELECT id, codice, titolo, attiva FROM sezioni WHERE modello_id = ? ORDER BY ordine",
    )
    .bind(modello_id)
    .fetch_all(&mut *db)
    .await?;
    let mut sottosezioni: Vec<Sottosezione> = sqlx::query_as(
        "SELECT ss.id, ss.sezione_id, ss.titolo, ss.personalizzata, ss.attiva
         FROM sottosezioni ss JOIN sezioni s ON s.id = ss.sezione_id
         WHERE s.modello_id = ? ORDER BY ss.ordine, ss.id",
    )
    .bind(modello_id)
    .fetch_all(&mut *db)
    .await?;
    let indicatori: Vec<Indicatore> = sqlx::query_as(
        "SELECT i.id, i.sottosezione_id, i.etichetta, i.testo_proposto
         FROM indicatori i
         JOIN sottosezioni ss ON ss.id = i.sottosezione_id
         JOIN sezioni s ON s.id = ss.sezione_id
         WHERE s.modello_id = ? ORDER BY i.ordine, i.id",
    )
    .bind(modello_id)
    .fetch_all(&mut *db)
    .await?;

    for indicatore in indicatori {
        if let Some(ss) = sottosezioni
            .iter_mut()
            .find(|ss| ss.id == indicatore.sottosezione_id)
        {
            ss.indicatori.push(indicatore);
        }
    }
    for sottosezione in sottosezioni {
        if let Some(s) = sezioni.iter_mut().find(|s| s.id == sottosezione.sezione_id) {
            s.sottosezioni.push(sottosezione);
        }
    }
    Ok(sezioni)
}
