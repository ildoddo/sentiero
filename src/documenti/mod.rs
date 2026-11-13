//! Contenuto di un PFI così come viene congelato in una versione e stampato nei documenti.

use serde::{Deserialize, Serialize};

/// Copia autonoma di struttura e testi: non dipende più dal modello, che può cambiare in seguito.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contenuto {
    pub alunno: String,
    pub data_nascita: String,
    pub classe: String,
    pub anno_scolastico: String,
    pub indirizzo: String,
    pub modello: String,
    pub tutor: String,
    pub sezioni: Vec<SezioneDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SezioneDoc {
    pub codice: String,
    pub titolo: String,
    pub sottosezioni: Vec<SottosezioneDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SottosezioneDoc {
    pub titolo: String,
    pub voci: Vec<Voce>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voce {
    pub indicatore_id: i64,
    pub etichetta: String,
    pub testo: String,
}

pub mod odt;
pub mod pdf;

/// Una versione pronta per essere impaginata.
pub struct Documento<'a> {
    pub contenuto: &'a Contenuto,
    pub numero: i64,
    pub motivo: &'a str,
    pub data: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stile {
    Titolo,
    Sezione,
    Sottosezione,
    Etichetta,
    Testo,
}

impl Documento<'_> {
    /// Sequenza di paragrafi comune a PDF e ODT, così i due formati non possono divergere.
    pub fn paragrafi(&self) -> Vec<(Stile, String)> {
        let c = self.contenuto;
        let mut p = vec![
            (
                Stile::Titolo,
                "Sentiero · Progetto Formativo Individuale".to_owned(),
            ),
            (
                Stile::Testo,
                format!("Alunno: {} (nato il {})", c.alunno, c.data_nascita),
            ),
            (
                Stile::Testo,
                format!(
                    "Classe: {} · Anno scolastico {} · {}",
                    c.classe, c.anno_scolastico, c.indirizzo
                ),
            ),
            (Stile::Testo, format!("Tutor: {}", c.tutor)),
            (
                Stile::Testo,
                format!(
                    "Versione {} ({}) del {}",
                    self.numero, self.motivo, self.data
                ),
            ),
        ];
        for s in &c.sezioni {
            p.push((Stile::Sezione, format!("{} · {}", s.codice, s.titolo)));
            for ss in &s.sottosezioni {
                p.push((Stile::Sottosezione, ss.titolo.clone()));
                for v in &ss.voci {
                    p.push((Stile::Etichetta, v.etichetta.clone()));
                    let testo = if v.testo.is_empty() { "—" } else { &v.testo };
                    p.push((Stile::Testo, testo.to_owned()));
                }
            }
        }
        p
    }
}

#[cfg(test)]
pub(crate) fn esempio() -> Contenuto {
    Contenuto {
        alunno: "Bianchi Giulia".into(),
        data_nascita: "2012-03-14".into(),
        classe: "1A".into(),
        anno_scolastico: "2026/2027".into(),
        indirizzo: "Servizi commerciali".into(),
        modello: "Modello base".into(),
        tutor: "Tommaso Tutor".into(),
        sezioni: vec![SezioneDoc {
            codice: "A".into(),
            titolo: "Dati anagrafici".into(),
            sottosezioni: vec![SottosezioneDoc {
                titolo: "Presentazione".into(),
                voci: vec![Voce {
                    indicatore_id: 1,
                    etichetta: "Profilo <sintetico> & note".into(),
                    testo: "Più righe:\nperché è così.".into(),
                }],
            }],
        }],
    }
}
