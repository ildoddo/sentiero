//! PDF in formato A4 con i font standard (Helvetica): nessun file di font da distribuire.

use printpdf::{
    BuiltinFont, Mm, Op, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt, TextItem,
};

use super::{Documento, Stile};

const LARGHEZZA: f32 = 210.0;
const ALTEZZA: f32 = 297.0;
const MARGINE: f32 = 20.0;
const MM_PER_PT: f32 = 0.352_778;

pub fn genera(doc: &Documento) -> Vec<u8> {
    let larghezza_testo = (LARGHEZZA - 2.0 * MARGINE) / MM_PER_PT;
    let mut pagine: Vec<Vec<Op>> = vec![Vec::new()];
    let mut y = ALTEZZA - MARGINE;

    for (stile, testo) in doc.paragrafi() {
        let (font, corpo, spazio_prima) = match stile {
            Stile::Titolo => (BuiltinFont::HelveticaBold, 16.0, 0.0),
            Stile::Sezione => (BuiltinFont::HelveticaBold, 13.0, 8.0),
            Stile::Sottosezione => (BuiltinFont::HelveticaBold, 11.0, 5.0),
            Stile::Etichetta => (BuiltinFont::HelveticaBold, 10.0, 3.0),
            Stile::Testo => (BuiltinFont::Helvetica, 10.0, 0.5),
        };
        let grassetto = font == BuiltinFont::HelveticaBold;
        let interlinea = corpo * 1.35 * MM_PER_PT;
        y -= spazio_prima;
        for capoverso in testo.split('\n') {
            for riga in a_capo(capoverso, corpo, grassetto, larghezza_testo) {
                // Lo spazio sotto il margine è riservato al piè di pagina.
                if y - interlinea < MARGINE + 8.0 {
                    pagine.push(Vec::new());
                    y = ALTEZZA - MARGINE;
                }
                y -= interlinea;
                scrivi(
                    pagine.last_mut().expect("almeno una pagina"),
                    font,
                    corpo,
                    MARGINE,
                    y,
                    riga,
                );
            }
        }
    }

    let totale = pagine.len();
    let pagine = pagine
        .into_iter()
        .enumerate()
        .map(|(i, mut ops)| {
            let piede = format!(
                "Sentiero · PFI di {} · versione {} · pagina {} di {}",
                doc.contenuto.alunno,
                doc.numero,
                i + 1,
                totale
            );
            scrivi(
                &mut ops,
                BuiltinFont::Helvetica,
                8.0,
                MARGINE,
                MARGINE - 6.0,
                piede,
            );
            PdfPage::new(Mm(LARGHEZZA), Mm(ALTEZZA), ops)
        })
        .collect();

    PdfDocument::new(&format!("Sentiero - PFI di {}", doc.contenuto.alunno))
        .with_pages(pagine)
        .save(&PdfSaveOptions::default(), &mut Vec::new())
}

fn scrivi(ops: &mut Vec<Op>, font: BuiltinFont, corpo: f32, x: f32, y: f32, testo: String) {
    ops.extend([
        Op::StartTextSection,
        Op::SetTextCursor {
            pos: Point::new(Mm(x), Mm(y)),
        },
        Op::SetFont {
            font: PdfFontHandle::Builtin(font),
            size: Pt(corpo),
        },
        Op::ShowText {
            items: vec![TextItem::Text(testo)],
        },
        Op::EndTextSection,
    ]);
}

// shortcut: larghezze stimate per eccesso invece delle metriche reali di Helvetica, quindi le righe
// vanno a capo un po' prima del necessario; passare a un font incorporato con metriche se serve
// un'impaginazione precisa.
fn larghezza(c: char, corpo: f32, grassetto: bool) -> f32 {
    let em = match c {
        'i' | 'j' | 'l' | 'I' | '.' | ',' | ';' | ':' | '\'' | '!' | '|' | ' ' => 0.29,
        'f' | 't' | 'r' | '(' | ')' | '-' | '/' | '"' => 0.36,
        'm' | 'M' | 'W' | 'w' | '@' | '%' | '—' => 0.96,
        c if c.is_uppercase() => 0.79,
        _ => 0.57,
    };
    em * corpo * if grassetto { 1.08 } else { 1.0 }
}

/// Spezza un capoverso in righe che stanno nella larghezza data; le parole troppo lunghe vengono divise.
fn a_capo(capoverso: &str, corpo: f32, grassetto: bool, max: f32) -> Vec<String> {
    let misura = |s: &str| {
        s.chars()
            .map(|c| larghezza(c, corpo, grassetto))
            .sum::<f32>()
    };
    let mut righe = Vec::new();
    let mut riga = String::new();
    // I caratteri di controllo (tabulazioni comprese) non hanno un glifo: diventano separatori.
    for parola in capoverso
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|p| !p.is_empty())
    {
        let mut parola = parola.to_owned();
        loop {
            let candidata = if riga.is_empty() {
                parola.clone()
            } else {
                format!("{riga} {parola}")
            };
            if misura(&candidata) <= max {
                riga = candidata;
                break;
            }
            if !riga.is_empty() {
                righe.push(std::mem::take(&mut riga));
                continue;
            }
            // La parola da sola non entra in una riga: se ne stampa il pezzo che ci sta.
            let mut taglio = 0;
            let mut usata = 0.0;
            for (i, c) in parola.char_indices() {
                usata += larghezza(c, corpo, grassetto);
                if usata > max && i > 0 {
                    break;
                }
                taglio = i + c.len_utf8();
            }
            righe.push(parola[..taglio].to_owned());
            parola = parola[taglio..].to_owned();
            if parola.is_empty() {
                break;
            }
        }
    }
    righe.push(riga);
    righe
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documenti::esempio;

    #[test]
    fn a_capo_rispetta_la_larghezza_e_conserva_il_testo() {
        let max = 200.0;
        let lunga = "x".repeat(150);
        let testo = format!(
            "Una frase abbastanza lunga da richiedere più righe, con\tuna tabulazione e {lunga} in mezzo."
        );
        let righe = a_capo(&testo, 10.0, false, max);
        assert!(righe.len() > 3);
        for riga in &righe {
            let larga: f32 = riga.chars().map(|c| larghezza(c, 10.0, false)).sum();
            assert!(larga <= max, "{riga}");
        }
        let senza_spazi = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(senza_spazi(&righe.concat()), senza_spazi(&testo));
        assert_eq!(a_capo("", 10.0, false, max), [""]);
    }

    #[test]
    fn pdf_valido_e_su_più_pagine() {
        let mut contenuto = esempio();
        let doc = Documento {
            contenuto: &contenuto,
            numero: 2,
            motivo: "revisione",
            data: "2026-11-13",
        };
        let breve = genera(&doc);
        assert!(breve.starts_with(b"%PDF-"));

        contenuto.sezioni[0].sottosezioni[0].voci[0].testo = "Una riga di testo.\n".repeat(200);
        let doc = Documento {
            contenuto: &contenuto,
            numero: 2,
            motivo: "revisione",
            data: "2026-11-13",
        };
        let lungo = genera(&doc);
        assert!(lungo.starts_with(b"%PDF-") && lungo.len() > breve.len());
        let pagine = printpdf::PdfDocument::parse(&lungo, &Default::default(), &mut Vec::new())
            .unwrap()
            .pages
            .len();
        assert!(pagine >= 3, "{pagine} pagine");
    }
}
