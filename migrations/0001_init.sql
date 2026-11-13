CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    ruolo         TEXT NOT NULL CHECK (ruolo IN ('admin', 'docente', 'dirigente', 'famiglia')),
    nome          TEXT NOT NULL,
    cognome       TEXT NOT NULL
);

CREATE TABLE sessions (
    token      TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    scade_il   INTEGER NOT NULL
);

CREATE TABLE classi (
    id              INTEGER PRIMARY KEY,
    nome            TEXT NOT NULL,
    anno_scolastico TEXT NOT NULL,
    indirizzo       TEXT NOT NULL,
    UNIQUE (nome, anno_scolastico)
);

CREATE TABLE cdc (
    classe_id  INTEGER NOT NULL REFERENCES classi (id) ON DELETE CASCADE,
    docente_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (classe_id, docente_id)
);

CREATE TABLE modelli (
    id        INTEGER PRIMARY KEY,
    nome      TEXT NOT NULL UNIQUE,
    indirizzo TEXT NOT NULL
);

CREATE TABLE sezioni (
    id         INTEGER PRIMARY KEY,
    modello_id INTEGER NOT NULL REFERENCES modelli (id) ON DELETE CASCADE,
    codice     TEXT NOT NULL CHECK (codice IN ('A', 'B', 'C', 'D', 'E')),
    titolo     TEXT NOT NULL,
    attiva     INTEGER NOT NULL DEFAULT 1,
    ordine     INTEGER NOT NULL,
    UNIQUE (modello_id, codice)
);

CREATE TABLE sottosezioni (
    id             INTEGER PRIMARY KEY,
    sezione_id     INTEGER NOT NULL REFERENCES sezioni (id) ON DELETE CASCADE,
    titolo         TEXT NOT NULL,
    personalizzata INTEGER NOT NULL DEFAULT 0,
    attiva         INTEGER NOT NULL DEFAULT 1,
    ordine         INTEGER NOT NULL
);
CREATE INDEX sottosezioni_sezione ON sottosezioni (sezione_id);

CREATE TABLE indicatori (
    id              INTEGER PRIMARY KEY,
    sottosezione_id INTEGER NOT NULL REFERENCES sottosezioni (id) ON DELETE CASCADE,
    etichetta       TEXT NOT NULL,
    testo_proposto  TEXT NOT NULL DEFAULT '',
    ordine          INTEGER NOT NULL
);
CREATE INDEX indicatori_sottosezione ON indicatori (sottosezione_id);

CREATE TABLE alunni (
    id               INTEGER PRIMARY KEY,
    classe_id        INTEGER NOT NULL REFERENCES classi (id),
    nome             TEXT NOT NULL,
    cognome          TEXT NOT NULL,
    data_nascita     TEXT NOT NULL,
    modello_id       INTEGER REFERENCES modelli (id),
    tutor_id         INTEGER REFERENCES users (id) ON DELETE SET NULL,
    famiglia_user_id INTEGER REFERENCES users (id) ON DELETE SET NULL,
    UNIQUE (classe_id, cognome, nome, data_nascita)
);

CREATE TABLE pfi (
    id         INTEGER PRIMARY KEY,
    alunno_id  INTEGER NOT NULL UNIQUE REFERENCES alunni (id) ON DELETE CASCADE,
    modello_id INTEGER NOT NULL REFERENCES modelli (id)
);

-- Bozza di lavoro del tutor: le versioni ne conservano una copia immutabile.
CREATE TABLE risposte (
    pfi_id        INTEGER NOT NULL REFERENCES pfi (id) ON DELETE CASCADE,
    indicatore_id INTEGER NOT NULL REFERENCES indicatori (id) ON DELETE CASCADE,
    testo         TEXT NOT NULL,
    PRIMARY KEY (pfi_id, indicatore_id)
);

CREATE TABLE versioni (
    id                INTEGER PRIMARY KEY,
    pfi_id            INTEGER NOT NULL REFERENCES pfi (id) ON DELETE CASCADE,
    numero            INTEGER NOT NULL,
    motivo            TEXT NOT NULL CHECK (motivo IN ('revisione', 'scrutinio')),
    creata_il         TEXT NOT NULL DEFAULT (datetime('now')),
    creata_da         INTEGER NOT NULL REFERENCES users (id),
    contenuto_json    TEXT NOT NULL,
    pdf_path          TEXT NOT NULL,
    odt_path          TEXT NOT NULL,
    approvata_il      TEXT,
    approvata_da      INTEGER REFERENCES users (id),
    visibile_famiglia INTEGER NOT NULL DEFAULT 0,
    UNIQUE (pfi_id, numero)
);

CREATE TABLE allegati (
    id                INTEGER PRIMARY KEY,
    alunno_id         INTEGER NOT NULL REFERENCES alunni (id) ON DELETE CASCADE,
    tipo              TEXT NOT NULL CHECK (tipo IN ('PDP', 'PEI', 'UdA')),
    nome_file         TEXT NOT NULL,
    path              TEXT NOT NULL,
    mime              TEXT NOT NULL,
    dimensione        INTEGER NOT NULL,
    caricato_da       INTEGER NOT NULL REFERENCES users (id),
    caricato_il       TEXT NOT NULL DEFAULT (datetime('now')),
    visibile_famiglia INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX allegati_alunno ON allegati (alunno_id);

CREATE TABLE visti (
    versione_id INTEGER NOT NULL REFERENCES versioni (id) ON DELETE CASCADE,
    user_id     INTEGER NOT NULL REFERENCES users (id),
    data        TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (versione_id, user_id)
);
