//! XMP packets: Dublin Core title / description / keywords, the XMP basic
//! CreatorTool and MetadataDate, and a `cybermeta:` namespace for app data
//! (e.g. Ghostframe's source identity and edit recipe).
//!
//! The same packet works as a `.xmp` sidecar and embedded in an image.
//! Reading accepts either. Inside an image the packet is found by its
//! `<x:xmpmeta>` element (JPEG, WebP, uncompressed PNG), or in a PNG text
//! chunk keyed `XML:com.adobe.xmp` (iTXt, zTXt or tEXt; may be zlib-compressed).

use anyhow::{bail, Context, Result};
use quick_xml::escape::escape;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const NS_CYBERMETA: &str = "https://cybercore.tech/ns/cybermeta/1.0/";
const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct XmpDoc {
    pub title: Option<String>,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    pub creator_tool: Option<String>,
    pub metadata_date: Option<String>,
    /// `cybermeta:` fields (name → text), except `recipe`.
    pub fields: BTreeMap<String, String>,
    /// `cybermeta:recipe`, parsed as JSON.
    pub recipe: Option<serde_json::Value>,
}

/// XML element names allowed in the cybermeta namespace.
pub fn valid_field_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Current UTC time as "YYYY-MM-DDTHH:MM:SSZ" (no date crate needed).
pub fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

impl XmpDoc {
    /// The XMP packet (with `<?xpacket?>` wrapper, so it can be embedded).
    pub fn to_xml(&self) -> Result<String> {
        let mut out = String::from(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n \
             <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  \
             <rdf:Description rdf:about=\"\"\n    \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n    \
             xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n    \
             xmlns:cybermeta=\"https://cybercore.tech/ns/cybermeta/1.0/\">\n",
        );
        let alt = |name: &str, value: &str| {
            format!(
                "   <{name}><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></{name}>\n",
                escape(value)
            )
        };
        if let Some(title) = &self.title {
            out.push_str(&alt("dc:title", title));
        }
        if let Some(description) = &self.description {
            out.push_str(&alt("dc:description", description));
        }
        if !self.keywords.is_empty() {
            out.push_str("   <dc:subject><rdf:Bag>");
            for keyword in &self.keywords {
                out.push_str(&format!("<rdf:li>{}</rdf:li>", escape(keyword)));
            }
            out.push_str("</rdf:Bag></dc:subject>\n");
        }
        if let Some(tool) = &self.creator_tool {
            out.push_str(&format!(
                "   <xmp:CreatorTool>{}</xmp:CreatorTool>\n",
                escape(tool)
            ));
        }
        let date = self.metadata_date.clone().unwrap_or_else(utc_now);
        out.push_str(&format!(
            "   <xmp:MetadataDate>{}</xmp:MetadataDate>\n",
            escape(&date)
        ));
        out.push_str("   <cybermeta:version>1.0</cybermeta:version>\n");
        for (name, value) in &self.fields {
            if name == "version" || name == "recipe" {
                continue;
            }
            if !valid_field_name(name) {
                bail!("invalid cybermeta field name `{name}` (letters, digits, _)");
            }
            out.push_str(&format!(
                "   <cybermeta:{name}>{}</cybermeta:{name}>\n",
                escape(value)
            ));
        }
        if let Some(recipe) = &self.recipe {
            let json = serde_json::to_string(recipe)?;
            out.push_str(&format!(
                "   <cybermeta:recipe>{}</cybermeta:recipe>\n",
                escape(&json)
            ));
        }
        out.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>\n");
        Ok(out)
    }

    /// Parses an XMP packet (namespace-aware, so other prefixes work too).
    pub fn parse(xml: &str) -> Result<Self> {
        let mut reader = NsReader::from_str(xml);
        let mut doc = XmpDoc::default();
        // (namespace, local name) of open elements.
        let mut stack: Vec<(String, String)> = Vec::new();
        let mut text = String::new();
        loop {
            match reader.read_resolved_event().context("malformed XMP")? {
                (ns, Event::Start(e)) => {
                    let uri = match ns {
                        ResolveResult::Bound(n) => String::from_utf8_lossy(n.as_ref()).into_owned(),
                        _ => String::new(),
                    };
                    let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                    // Short form: cybermeta:/dc:/xmp: values as attributes.
                    if uri == NS_RDF && local == "Description" {
                        for attr in e.attributes().flatten() {
                            let (attr_ns, attr_local) = reader.resolve_attribute(attr.key);
                            let attr_uri = match attr_ns {
                                ResolveResult::Bound(n) => {
                                    String::from_utf8_lossy(n.as_ref()).into_owned()
                                }
                                _ => continue,
                            };
                            let name = String::from_utf8_lossy(attr_local.as_ref()).into_owned();
                            let value = attr.unescape_value()?.into_owned();
                            doc.store(&attr_uri, &name, value)?;
                        }
                    }
                    stack.push((uri, local));
                    text.clear();
                }
                (_, Event::Text(t)) => text.push_str(&t.unescape()?),
                (_, Event::CData(t)) => text.push_str(&String::from_utf8_lossy(&t)),
                (_, Event::End(_)) => {
                    let Some((uri, local)) = stack.pop() else {
                        break;
                    };
                    // A value is the text of the property element itself, or of
                    // an rdf:li under rdf:Alt / rdf:Bag / rdf:Seq inside it.
                    let property = if uri == NS_RDF && local == "li" {
                        stack.iter().rev().nth(1).cloned()
                    } else {
                        Some((uri, local))
                    };
                    if let Some((p_uri, p_local)) = property {
                        let value = text.trim().to_string();
                        if p_uri != NS_RDF && !value.is_empty() {
                            doc.store(&p_uri, &p_local, value)?;
                        }
                    }
                    text.clear();
                }
                (_, Event::Eof) => break,
                _ => {}
            }
        }
        Ok(doc)
    }

    fn store(&mut self, uri: &str, name: &str, value: String) -> Result<()> {
        match (uri, name) {
            (NS_DC, "title") => self.title = Some(value),
            (NS_DC, "description") => self.description = Some(value),
            (NS_DC, "subject") => self.keywords.push(value),
            (NS_XMP, "CreatorTool") => self.creator_tool = Some(value),
            (NS_XMP, "MetadataDate") => self.metadata_date = Some(value),
            (NS_CYBERMETA, "recipe") => {
                self.recipe = Some(
                    serde_json::from_str(&value).context("cybermeta:recipe is not valid JSON")?,
                )
            }
            (NS_CYBERMETA, field) => {
                self.fields.insert(field.to_string(), value);
            }
            _ => {}
        }
        Ok(())
    }

    /// Reads a `.xmp` sidecar, or the XMP packet embedded in an image.
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let Some(xml) = extract_packet(&bytes) else {
            bail!("no XMP packet in {}", path.display());
        };
        Self::parse(&xml)
    }
}

/// The XMP packet in a file's bytes: a PNG XMP text chunk (inflated if
/// compressed), else a plain `<x:xmpmeta>` element anywhere in the file.
pub fn extract_packet(bytes: &[u8]) -> Option<String> {
    png_text_xmp(bytes).or_else(|| find_packet(bytes).map(str::to_string))
}

/// PNG: walk the chunks for iTXt / zTXt / tEXt keyed `XML:com.adobe.xmp`
/// (iTXt is the spec; libvips writes a zlib-compressed zTXt).
fn png_text_xmp(bytes: &[u8]) -> Option<String> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return None;
    }
    let mut pos = SIGNATURE.len();
    while pos + 8 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data = bytes.get(pos + 8..pos + 8 + length)?;
        let keyword_end = data.iter().position(|&b| b == 0);
        let is_xmp = keyword_end.is_some_and(|end| &data[..end] == b"XML:com.adobe.xmp");
        if is_xmp && (kind == b"zTXt" || kind == b"tEXt") {
            let end = keyword_end?;
            // zTXt: keyword \0 method zlib-data · tEXt: keyword \0 text
            let text = if kind == b"zTXt" {
                miniz_oxide::inflate::decompress_to_vec_zlib(data.get(end + 2..)?).ok()?
            } else {
                data.get(end + 1..)?.to_vec()
            };
            return String::from_utf8(text).ok();
        }
        if is_xmp && kind == b"iTXt" {
            // keyword \0 compression-flag compression-method language \0 translated \0 text
            let keyword_end = keyword_end?;
            {
                let compressed = *data.get(keyword_end + 1)? == 1;
                let mut rest = data.get(keyword_end + 3..)?;
                for _ in 0..2 {
                    let end = rest.iter().position(|&b| b == 0)?;
                    rest = &rest[end + 1..];
                }
                let text = if compressed {
                    miniz_oxide::inflate::decompress_to_vec_zlib(rest).ok()?
                } else {
                    rest.to_vec()
                };
                return String::from_utf8(text).ok();
            }
        }
        if kind == b"IEND" {
            break;
        }
        pos += 12 + length; // length + type + data + CRC
    }
    None
}

/// The `<x:xmpmeta>…</x:xmpmeta>` element inside arbitrary bytes.
pub fn find_packet(bytes: &[u8]) -> Option<&str> {
    let open = b"<x:xmpmeta";
    let close = b"</x:xmpmeta>";
    let start = bytes.windows(open.len()).position(|w| w == open)?;
    let end_rel = bytes[start..]
        .windows(close.len())
        .position(|w| w == close)?;
    std::str::from_utf8(&bytes[start..start + end_rel + close.len()]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> XmpDoc {
        let mut fields = BTreeMap::new();
        fields.insert("sourceHash".into(), "abc123".into());
        fields.insert("sourceFilename".into(), "IMG <1> & co.jpg".into());
        XmpDoc {
            title: Some("Neon Coffin".into()),
            description: Some("Late returns, \"night\" edition".into()),
            keywords: vec!["neon".into(), "coffin & video".into()],
            creator_tool: Some("Ghostframe".into()),
            metadata_date: Some("2026-10-14T12:00:00Z".into()),
            fields,
            recipe: Some(
                serde_json::json!({"version": 1, "ops": [{"op": "rotate", "degrees": 90}]}),
            ),
        }
    }

    #[test]
    fn round_trips_through_xml() {
        let doc = sample();
        let xml = doc.to_xml().unwrap();
        assert!(xml.starts_with("<?xpacket"));
        assert!(xml.contains("&amp;"));
        let mut back = XmpDoc::parse(&xml).unwrap();
        assert_eq!(back.fields.remove("version").as_deref(), Some("1.0"));
        assert_eq!(back, doc);
    }

    #[test]
    fn reads_other_prefixes_and_attribute_form() {
        let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:d="http://purl.org/dc/elements/1.1/" xmlns:cm="https://cybercore.tech/ns/cybermeta/1.0/"
                xmlns:xap="http://ns.adobe.com/xap/1.0/" xap:CreatorTool="Other app" cm:sourceHash="ff00">
              <d:title><rdf:Alt><rdf:li xml:lang="x-default">Hello</rdf:li></rdf:Alt></d:title>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let doc = XmpDoc::parse(xml).unwrap();
        assert_eq!(doc.title.as_deref(), Some("Hello"));
        assert_eq!(doc.creator_tool.as_deref(), Some("Other app"));
        assert_eq!(
            doc.fields.get("sourceHash").map(String::as_str),
            Some("ff00")
        );
    }

    #[test]
    fn finds_a_packet_inside_image_bytes() {
        let xml = sample().to_xml().unwrap();
        let mut bytes = b"\xFF\xD8\xFF\xE1junkhttp://ns.adobe.com/xap/1.0/\0".to_vec();
        bytes.extend_from_slice(xml.as_bytes());
        bytes.extend_from_slice(b"\xFF\xD9");
        let found = find_packet(&bytes).unwrap();
        assert_eq!(
            XmpDoc::parse(found).unwrap().title.as_deref(),
            Some("Neon Coffin")
        );
        assert!(find_packet(b"no xmp here").is_none());
    }

    #[test]
    fn reads_compressed_xmp_from_png_itxt() {
        let xml = sample().to_xml().unwrap();
        let mut chunk = b"XML:com.adobe.xmp\0\x01\x00\0\0".to_vec();
        chunk.extend(miniz_oxide::deflate::compress_to_vec_zlib(
            xml.as_bytes(),
            6,
        ));
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [
            (&b"IHDR"[..], &[0u8; 13][..]),
            (b"iTXt", &chunk),
            (b"IEND", &[]),
        ] {
            png.extend((data.len() as u32).to_be_bytes());
            png.extend(kind);
            png.extend(data);
            png.extend([0u8; 4]); // CRC (not checked)
        }
        assert!(find_packet(&png).is_none()); // compressed: invisible to a byte scan
        let found = extract_packet(&png).unwrap();
        assert_eq!(
            XmpDoc::parse(&found).unwrap().title.as_deref(),
            Some("Neon Coffin")
        );

        // libvips' form: zTXt (keyword \0 method zlib-data).
        let mut ztxt = b"XML:com.adobe.xmp\0\x00".to_vec();
        ztxt.extend(miniz_oxide::deflate::compress_to_vec_zlib(
            xml.as_bytes(),
            6,
        ));
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [
            (&b"IHDR"[..], &[0u8; 13][..]),
            (b"zTXt", &ztxt),
            (b"IEND", &[]),
        ] {
            png.extend((data.len() as u32).to_be_bytes());
            png.extend(kind);
            png.extend(data);
            png.extend([0u8; 4]);
        }
        let found = extract_packet(&png).unwrap();
        assert_eq!(
            XmpDoc::parse(&found).unwrap().title.as_deref(),
            Some("Neon Coffin")
        );
    }

    #[test]
    fn rejects_bad_field_names_and_recipes() {
        let mut doc = sample();
        doc.fields.insert("bad name".into(), "x".into());
        assert!(doc.to_xml().is_err());
        let broken = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:cybermeta=\"https://cybercore.tech/ns/cybermeta/1.0/\"><cybermeta:recipe>{not json</cybermeta:recipe></rdf:Description></rdf:RDF></x:xmpmeta>";
        assert!(XmpDoc::parse(broken).is_err());
    }

    #[test]
    fn utc_now_is_well_formed() {
        let now = utc_now();
        assert_eq!(now.len(), 20);
        assert!(now.ends_with('Z') && now.as_bytes()[10] == b'T');
        assert!(now.starts_with("20"));
    }
}
