//! ODT minimale: un archivio zip con `mimetype`, il manifest e `content.xml`.

use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::{Documento, Stile};

const MIME: &str = "application/vnd.oasis.opendocument.text";

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
<manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.text"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#;

const INIZIO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">
<office:automatic-styles>
<style:style style:name="Titolo" style:family="paragraph"><style:paragraph-properties fo:margin-bottom="0.3cm"/><style:text-properties fo:font-size="16pt" fo:font-weight="bold"/></style:style>
<style:style style:name="Sezione" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.7cm" fo:margin-bottom="0.2cm" fo:keep-with-next="always"/><style:text-properties fo:font-size="13pt" fo:font-weight="bold"/></style:style>
<style:style style:name="Sottosezione" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.4cm" fo:keep-with-next="always"/><style:text-properties fo:font-size="11pt" fo:font-weight="bold"/></style:style>
<style:style style:name="Etichetta" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.25cm" fo:keep-with-next="always"/><style:text-properties fo:font-size="10pt" fo:font-weight="bold"/></style:style>
<style:style style:name="Testo" style:family="paragraph"><style:text-properties fo:font-size="10pt"/></style:style>
</office:automatic-styles>
<office:body><office:text>
"#;

const FINE: &str = "</office:text></office:body></office:document-content>";

pub fn genera(doc: &Documento) -> zip::result::ZipResult<Vec<u8>> {
    let mut contenuto = String::from(INIZIO);
    for (stile, testo) in doc.paragrafi() {
        let nome = match stile {
            Stile::Titolo => "Titolo",
            Stile::Sezione => "Sezione",
            Stile::Sottosezione => "Sottosezione",
            Stile::Etichetta => "Etichetta",
            Stile::Testo => "Testo",
        };
        contenuto.push_str(&format!(
            "<text:p text:style-name=\"{nome}\">{}</text:p>\n",
            xml(&testo)
        ));
    }
    contenuto.push_str(FINE);

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    // Il formato richiede che `mimetype` sia il primo file e non sia compresso.
    zip.start_file(
        "mimetype",
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
    )?;
    zip.write_all(MIME.as_bytes())?;
    let compresso = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("META-INF/manifest.xml", compresso)?;
    zip.write_all(MANIFEST.as_bytes())?;
    zip.start_file("content.xml", compresso)?;
    zip.write_all(contenuto.as_bytes())?;
    Ok(zip.finish()?.into_inner())
}

/// Escape per il testo XML; i ritorni a capo diventano interruzioni di riga, e i caratteri di
/// controllo, non ammessi in XML 1.0, vengono scartati.
fn xml(testo: &str) -> String {
    let mut out = String::with_capacity(testo.len());
    for c in testo.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("<text:line-break/>"),
            '\t' => out.push_str("<text:tab/>"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::documenti::esempio;

    #[test]
    fn odt_apribile_con_testo_protetto() {
        let contenuto = esempio();
        let doc = Documento {
            contenuto: &contenuto,
            numero: 2,
            motivo: "revisione",
            data: "2026-11-13",
        };
        let dati = genera(&doc).unwrap();

        // `mimetype` in chiaro subito dopo l'intestazione del primo file: così lo riconoscono le applicazioni.
        assert_eq!(&dati[30..38], b"mimetype");
        assert_eq!(&dati[38..38 + MIME.len()], MIME.as_bytes());

        let mut archivio = zip::ZipArchive::new(Cursor::new(dati)).unwrap();
        assert_eq!(archivio.len(), 3);
        let mut xml = String::new();
        archivio
            .by_name("content.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("Sentiero · Progetto Formativo Individuale"));
        assert!(xml.contains("Profilo &lt;sintetico&gt; &amp; note"));
        assert!(xml.contains("Più righe:<text:line-break/>perché è così."));
        assert!(xml.contains("Versione 2 (revisione) del 2026-11-13"));
    }
}
