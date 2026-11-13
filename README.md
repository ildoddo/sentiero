# Sentiero

Sentiero è un'applicazione web open source con cui una scuola redige, rivede, approva e conserva il
Progetto Formativo Individuale (PFI, D.Lgs. 61/2017) di ogni alunno.

Il progetto si ispira alle funzionalità di Argo PFI. Non è affiliato ad Argo Software né ne riusa
testi, grafica o codice.

Funzionalità: modelli PFI in cinque sezioni personalizzabili, assegnazione di modello e tutor,
anagrafica con importazione da CSV, compilazione con versioni numerate, documenti PDF e ODT per
ogni versione, allegati, consultazione da parte del Consiglio di Classe e del dirigente,
approvazione, area della famiglia con visto, esportazione di fine anno.

## Avvio

Serve Rust stable.

```bash
cargo run
```

Al primo avvio Sentiero crea `data/sentiero.db`, applica le migrazioni e inserisce i dati di esempio.
La password degli utenti di esempio viene stampata una sola volta sul terminale, oppure si imposta
con `SENTIERO_SEED_PASSWORD`. Poi si apre <http://127.0.0.1:3000>.

Utenti di esempio (stessa password): `admin@sentiero.test`, `tutor@sentiero.test`,
`docente@sentiero.test`, `dirigente@sentiero.test`, `famiglia@sentiero.test`.
I dati di esempio comprendono la classe 1A con tre alunni, già assegnati al "Modello base" e al tutor,
e un PFI completo per Giulia Bianchi con la versione 1 già generata e ancora da approvare.

## Configurazione

| Variabile | Predefinito | Significato |
|---|---|---|
| `SENTIERO_BIND` | `127.0.0.1:3000` | Indirizzo di ascolto |
| `SENTIERO_DATA_DIR` | `data` | Cartella di database, documenti e allegati |
| `DATABASE_URL` | `sqlite://<DATA_DIR>/sentiero.db` | Database SQLite |
| `SENTIERO_COOKIE_SECURE` | `false` | `true` dietro HTTPS: il cookie di sessione viaggia solo cifrato |
| `SENTIERO_SEED_PASSWORD` | casuale | Password degli utenti di esempio al primo avvio |

In produzione Sentiero va esposto solo tramite HTTPS, con `SENTIERO_COOKIE_SECURE=true`.

## Chi fa che cosa

| Profilo | Può |
|---|---|
| Amministratore | Gestire anagrafica, modelli PFI e assegnazioni. Non vede i PFI. |
| Tutor (docente assegnato all'alunno) | Compilare il PFI, generare le versioni, pubblicare gli allegati, registrare l'approvazione del CdC, scegliere che cosa vede la famiglia. |
| Docente del CdC | Consultare l'ultima versione del PFI e gli allegati degli alunni della classe. |
| Dirigente | Consultare tutti i PFI, registrare l'approvazione del CdC ed esportare l'archivio di fine anno. |
| Famiglia / Alunno | Vedere il PFI più recente tra quelli resi visibili e gli allegati visibili del proprio alunno, e apporre il visto. |

Il tutor non è un profilo a parte: è un docente del Consiglio di Classe indicato nelle assegnazioni.
Una versione può essere mostrata alla famiglia solo dopo l'approvazione del CdC; la visibilità si
sceglie separatamente per il PFI e per ogni allegato.

## Percorso di prova

Con i dati di esempio:

1. `tutor@sentiero.test` apre un alunno, compila il PFI e genera la versione 1, poi la 2: ogni
   versione ha il suo PDF e il suo ODT nello storico.
2. Dalla pagina di consultazione il tutor registra l'approvazione del CdC e rende il PFI visibile
   alla famiglia.
3. `famiglia@sentiero.test` (collegata a Giulia Bianchi) vede il PFI e appone il visto.
4. `dirigente@sentiero.test` esporta dalla home l'archivio zip della classe o del singolo alunno.

## Conservazione di fine anno

Il dirigente scarica un archivio zip per classe o per alunno. Contiene solo le versioni che sono
sia approvate dal CdC sia vistate dalla famiglia: per ognuna il PDF e l'ODT, in
`<classe>/<Cognome>_<Nome>_<id>/`, più `indice.csv` con data dell'approvazione, chi l'ha
registrata e data e autore di ogni visto.

## Importazione dell'anagrafica da CSV

L'amministratore carica i file dalle pagine Classi, Docenti, Alunni e Consigli di Classe, in
quest'ordine. I file sono in UTF-8, con una riga di intestazione e separatore `;` oppure `,`.

| Pagina | Colonne |
|---|---|
| Classi | `nome;anno_scolastico;indirizzo` |
| Docenti | `cognome;nome;email;password;ruolo` (`ruolo` facoltativo: `docente` o `dirigente`) |
| Alunni | `classe;anno_scolastico;cognome;nome;data_nascita;email_famiglia;password_famiglia` (ultime due facoltative) |
| Consigli di Classe | `classe;anno_scolastico;email_docente` |

- Ogni file è importato per intero oppure rifiutato: l'errore indica la riga da correggere.
- Reimportare lo stesso file non crea duplicati.
- La password serve solo per creare un nuovo account (almeno 8 caratteri); quella di un account
  esistente non viene modificata.
- La data di nascita è `AAAA-MM-GG` oppure `GG/MM/AAAA`.
- Più alunni con la stessa `email_famiglia` condividono lo stesso account famiglia.

## Sviluppo

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

I test di integrazione in `tests/accesso.rs` usano un database in memoria e verificano, per ogni
route, quali profili possono accedere.

## Dipendenze

| Crate | Perché |
|---|---|
| `axum` | Router, estrattori e server HTTP |
| `tokio` | Runtime asincrono richiesto da axum e sqlx |
| `sqlx` | Accesso a SQLite con query parametrizzate e migrazioni incorporate |
| `askama` | Template HTML compilati, con escaping automatico |
| `argon2` | Hash delle password |
| `csv` | Lettura dei CSV dell'anagrafica e scrittura dell'indice dell'archivio di conservazione |
| `getrandom` | Token casuali di sessione e CSRF dal generatore del sistema operativo |
| `printpdf` | Generazione dei PDF con i font standard, senza funzionalità opzionali (HTML, immagini) |
| `serde` | Deserializzazione dei form |
| `serde_json` | Copia immutabile del contenuto di ogni versione del PFI |
| `serde_urlencoded` | Lettura dei form insieme al controllo del token CSRF |
| `thiserror` | Tipo di errore dell'applicazione |
| `tracing`, `tracing-subscriber` | Log strutturati, senza dati personali |
| `zip` | Scrittura dei file ODT (che sono archivi zip) e dell'archivio di conservazione |
| `tower` (solo test) | Invio di richieste al router senza aprire una porta |

HTMX 2.0.4 è incluso come file in `static/` e incorporato nel binario: nessun build step Node.

## Dati e file

Tutto ciò che Sentiero scrive sta in `SENTIERO_DATA_DIR`, che non viene mai servita come cartella
pubblica: ogni download passa da una route con controllo di accesso.

- `sentiero.db`: database SQLite.
- `documenti/`: PDF e ODT di ogni versione dei PFI.
- `allegati/`: file caricati dai tutor (PDF, ODT o DOCX, fino a 10 MB), salvati con un nome casuale.
  Il tipo è verificato sul contenuto del file, non solo sull'estensione.

La cartella contiene dati personali degli alunni: va protetta e inclusa nei backup.

## Limiti noti

- Nessun cambio o recupero password dall'interfaccia: le password iniziali arrivano dal CSV o dal
  form dell'amministratore.
- Nessun limite ai tentativi di accesso: va messo sul reverse proxy.
- I record dell'anagrafica si aggiungono e si reimportano, ma non si eliminano dall'interfaccia.
- Non esiste il passaggio d'anno: ogni classe porta il proprio anno scolastico e gli alunni non
  vengono spostati da una classe all'altra.
- Nel PDF l'a capo usa larghezze stimate dei caratteri e i caratteri fuori dall'alfabeto
  occidentale diventano `?`; nell'ODT il testo resta invariato.
- Il token di sessione è salvato in chiaro nel database, con scadenza a 8 ore.
