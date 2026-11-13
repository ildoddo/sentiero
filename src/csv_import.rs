//! Lettura dei file CSV dell'anagrafica: una riga di intestazione, separatore `;` oppure `,`, UTF-8.

use serde::{Deserialize, de::DeserializeOwned};

use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct RigaClasse {
    pub nome: String,
    pub anno_scolastico: String,
    pub indirizzo: String,
}

#[derive(Debug, Deserialize)]
pub struct RigaDocente {
    pub cognome: String,
    pub nome: String,
    pub email: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub ruolo: String,
}

#[derive(Debug, Deserialize)]
pub struct RigaAlunno {
    pub classe: String,
    pub anno_scolastico: String,
    pub cognome: String,
    pub nome: String,
    pub data_nascita: String,
    #[serde(default)]
    pub email_famiglia: String,
    #[serde(default)]
    pub password_famiglia: String,
}

#[derive(Debug, Deserialize)]
pub struct RigaCdc {
    pub classe: String,
    pub anno_scolastico: String,
    pub email_docente: String,
}

/// Restituisce le righe con il loro numero nel file (l'intestazione è la riga 1).
pub fn leggi<T: DeserializeOwned>(dati: &[u8]) -> Result<Vec<(usize, T)>, AppError> {
    let testo = std::str::from_utf8(dati)
        .map_err(|_| AppError::Richiesta("il file deve essere in UTF-8".into()))?
        .trim_start_matches('\u{feff}');
    // Excel in italiano esporta con il punto e virgola.
    let separatore = if testo.lines().next().is_some_and(|l| l.contains(';')) {
        b';'
    } else {
        b','
    };
    csv::ReaderBuilder::new()
        .delimiter(separatore)
        .trim(csv::Trim::All)
        .from_reader(testo.as_bytes())
        .deserialize()
        .enumerate()
        .map(|(i, riga)| {
            let n = i + 2;
            riga.map(|r| (n, r)).map_err(|_| {
                AppError::Richiesta(format!("riga {n}: colonne mancanti o non valide"))
            })
        })
        .collect()
}

/// Accetta `AAAA-MM-GG` oppure `GG/MM/AAAA` e restituisce la data in formato ISO.
pub fn normalizza_data(s: &str) -> Option<String> {
    let parti: Vec<&str> = s.split(['-', '/']).collect();
    let [a, b, c] = parti[..] else { return None };
    let (anno, mese, giorno) = if s.contains('/') {
        (c, b, a)
    } else {
        (a, b, c)
    };
    let (y, m, d): (u16, u8, u8) = (anno.parse().ok()?, mese.parse().ok()?, giorno.parse().ok()?);
    let giorni = match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return None,
    };
    ((1900..=2100).contains(&y) && (1..=giorni).contains(&d))
        .then(|| format!("{y:04}-{m:02}-{d:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date() {
        assert_eq!(normalizza_data("2012-03-14").as_deref(), Some("2012-03-14"));
        assert_eq!(normalizza_data("14/03/2012").as_deref(), Some("2012-03-14"));
        assert_eq!(normalizza_data("29/02/2012").as_deref(), Some("2012-02-29"));
        for errata in [
            "29/02/2013",
            "2012-13-01",
            "31/04/2012",
            "ieri",
            "",
            "2012-03",
            "0012-03-14",
        ] {
            assert_eq!(normalizza_data(errata), None, "{errata}");
        }
    }

    #[test]
    fn separatore_bom_e_spazi() {
        let righe: Vec<(usize, RigaClasse)> =
            leggi("\u{feff}nome;anno_scolastico;indirizzo\n 1B ;2026/2027;Meccanica\n".as_bytes())
                .unwrap();
        assert_eq!((righe[0].0, righe[0].1.nome.as_str()), (2, "1B"));

        let righe: Vec<(usize, RigaClasse)> =
            leggi(b"nome,anno_scolastico,indirizzo\n1B,2026/2027,\"Moda, tessile\"\n").unwrap();
        assert_eq!(righe[0].1.indirizzo, "Moda, tessile");
    }

    #[test]
    fn colonne_mancanti_ed_opzionali() {
        assert!(leggi::<RigaClasse>(b"nome;indirizzo\n1B;Meccanica\n").is_err());
        let righe: Vec<(usize, RigaDocente)> =
            leggi(b"cognome;nome;email\nRossi;Ada;ada@example.test\n").unwrap();
        assert_eq!(righe[0].1.ruolo, "");
    }
}
