//! Privacy audit: what does this file give away?
//!
//! Findings come from two readers that share no code with little_exif:
//! nom-exif for EXIF (any format it knows, HEIC included), and the
//! container parser for everything around it (XMP, IPTC, comments, PNG
//! text, thumbnails, data hidden after the image). The scrubber runs the
//! same audit on its own output and refuses to report success if anything
//! but what you asked to keep is still there.

use super::container::{self, BlockKind, Format};
use super::xmp;
use nom_exif::{EntryValue, Exif, IfdIndex, IfdKind, MediaParser, MediaSource};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Where the photo was taken.
    Location,
    /// Who took or owns it: names, contacts, copyright.
    Person,
    /// Which device: serial numbers, unique IDs, maker notes, make/model.
    Device,
    /// When it was taken or edited.
    Time,
    /// Editing software and history.
    Software,
    /// Captions, comments, keywords.
    Text,
    /// A preview image that may show the photo before cropping or edits.
    Thumbnail,
    /// Data hidden after the image (motion-photo video, depth maps, ...).
    Hidden,
    /// Camera settings and anything else.
    Other,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Location => "location",
            Self::Person => "person",
            Self::Device => "device",
            Self::Time => "time",
            Self::Software => "software",
            Self::Text => "text",
            Self::Thumbnail => "thumbnail",
            Self::Hidden => "hidden data",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub category: Category,
    /// Where it was found: "EXIF", "XMP", "IPTC", "JPEG comment", ...
    pub source: String,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockInfo {
    pub kind: BlockKind,
    pub label: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gps {
    pub latitude: f64,
    pub longitude: f64,
    pub map_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Audit {
    pub format: Format,
    /// Highest severity found; `None` means nothing was found.
    pub risk: Option<Severity>,
    pub findings: Vec<Finding>,
    /// Metadata blocks in the container (empty for formats the container
    /// parser doesn't handle; their EXIF is still audited).
    pub blocks: Vec<BlockInfo>,
    pub gps: Option<Gps>,
    /// Can `scrub` rebuild this format losslessly?
    pub scrubbable: bool,
    /// The container could not be parsed (truncated or damaged file).
    pub damaged: Option<String>,
}

impl Audit {
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

pub fn audit(bytes: &[u8]) -> Audit {
    let format = container::detect(bytes);
    let mut findings = Vec::new();
    let mut blocks = Vec::new();
    let mut damaged = None;

    let exif = read_exif(bytes);
    let mut gps = None;
    if let Some(exif) = &exif {
        gps = exif_findings(exif, &mut findings);
    }

    if format.scrubbable() {
        match container::parse(bytes) {
            Ok(parsed) => {
                for blk in parsed.metadata() {
                    blocks.push(BlockInfo {
                        kind: blk.kind,
                        label: blk.label.clone(),
                        bytes: blk.len(),
                    });
                    block_findings(format, bytes, blk, exif.is_some(), &mut findings);
                }
            }
            Err(e) => damaged = Some(format!("{e:#}")),
        }
    }

    // Two readers can report the same thing (e.g. EXIF + XMP GPS); keep both
    // since they are separate copies, but drop exact duplicates.
    findings.dedup_by(|a, b| a.source == b.source && a.label == b.label && a.value == b.value);
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    let risk = findings.iter().map(|f| f.severity).max();
    Audit {
        format,
        risk,
        findings,
        blocks,
        gps,
        scrubbable: format.scrubbable(),
        damaged,
    }
}

fn read_exif(bytes: &[u8]) -> Option<Exif> {
    let ms = MediaSource::from_memory(bytes.to_vec()).ok()?;
    let mut parser = MediaParser::new();
    let iter = parser.parse_exif(ms).ok()?;
    Some(iter.into())
}

fn find(
    sev: Severity,
    cat: Category,
    source: &str,
    label: impl Into<String>,
    value: impl Into<String>,
) -> Finding {
    Finding {
        severity: sev,
        category: cat,
        source: source.into(),
        label: label.into(),
        value: value.into(),
    }
}

fn shown(v: &EntryValue) -> String {
    match v {
        EntryValue::Undefined(b) | EntryValue::U8Array(b) => {
            let printable: String = b
                .iter()
                .filter(|c| c.is_ascii_graphic() || **c == b' ')
                .take(48)
                .map(|&c| c as char)
                .collect();
            if printable.trim().len() >= 4 && b.len() <= 64 {
                printable.trim().to_string()
            } else {
                format!("{} bytes", b.len())
            }
        }
        other => clip(&other.to_string(), 80),
    }
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim().trim_matches('\0');
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

/// EXIF/TIFF tags that only describe how to draw the picture.
fn structural(code: u16) -> bool {
    matches!(
        code,
        0x0100
            | 0x0101
            | 0x0102
            | 0x0103
            | 0x0106
            | 0x0111
            | 0x0112
            | 0x0115
            | 0x0116
            | 0x0117
            | 0x011A
            | 0x011B
            | 0x011C
            | 0x0128
            | 0x0201
            | 0x0202
            | 0x0211
            | 0x0212
            | 0x0213
            | 0x0214
            | 0x8769
            | 0x8825
            | 0xA005
            | 0x9000
            | 0xA000
            | 0xA001
            | 0xA002
            | 0xA003
            | 0x9101
            | 0x0001
            | 0x0002
    )
}

fn exif_findings(exif: &Exif, out: &mut Vec<Finding>) -> Option<Gps> {
    const S: &str = "EXIF";
    let mut gps_tags = 0;
    let mut settings = 0;
    let mut thumbnail = false;
    for e in exif.entries() {
        let code = e.tag().code();
        if e.ifd() == IfdIndex::THUMBNAIL {
            thumbnail = true;
            continue;
        }
        if e.ifd_kind() == IfdKind::Gps {
            gps_tags += 1;
            continue;
        }
        if e.ifd_kind() == IfdKind::Interop || structural(code) {
            continue;
        }
        let v = shown(e.value());
        let f = match code {
            0x013B => find(Severity::High, Category::Person, S, "Artist", v),
            0xA430 => find(Severity::High, Category::Person, S, "Camera owner", v),
            0x8298 => find(Severity::Medium, Category::Person, S, "Copyright", v),
            0x9C9D => find(Severity::High, Category::Person, S, "Author (Windows)", v),
            0xA431 => find(
                Severity::High,
                Category::Device,
                S,
                "Camera serial number",
                v,
            ),
            0xA435 => find(Severity::High, Category::Device, S, "Lens serial number", v),
            0xA420 => find(Severity::Medium, Category::Device, S, "Unique image ID", v),
            0x927C => find(
                Severity::Medium,
                Category::Device,
                S,
                "Maker note (often holds serial numbers)",
                v,
            ),
            0x010F => find(Severity::Low, Category::Device, S, "Camera make", v),
            0x0110 => find(Severity::Low, Category::Device, S, "Camera model", v),
            0xA433 => find(Severity::Low, Category::Device, S, "Lens make", v),
            0xA434 => find(Severity::Low, Category::Device, S, "Lens model", v),
            0x9003 => find(Severity::Medium, Category::Time, S, "Taken", v),
            0x9004 => find(Severity::Medium, Category::Time, S, "Digitised", v),
            0x0132 => find(Severity::Low, Category::Time, S, "Modified", v),
            0x9010..=0x9012 => find(Severity::Low, Category::Time, S, "Time-zone offset", v),
            0x0131 | 0x000B => find(Severity::Low, Category::Software, S, "Software", v),
            0x013C => find(Severity::Medium, Category::Device, S, "Host computer", v),
            0x010E => find(Severity::Medium, Category::Text, S, "Description", v),
            0x9286 => find(Severity::Medium, Category::Text, S, "User comment", v),
            0x9C9B | 0x9C9C | 0x9C9E | 0x9C9F => find(
                Severity::Medium,
                Category::Text,
                S,
                "Windows title / comment",
                v,
            ),
            _ => {
                settings += 1;
                continue;
            }
        };
        out.push(f);
    }
    let gps = exif.gps_info().and_then(|g| {
        let (lat, lon) = (g.latitude_decimal()?, g.longitude_decimal()?);
        Some(Gps {
            latitude: lat,
            longitude: lon,
            map_url: format!(
                "https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=16/{lat:.6}/{lon:.6}"
            ),
        })
    });
    if let Some(g) = &gps {
        out.push(find(
            Severity::High,
            Category::Location,
            S,
            "GPS position",
            format!("{:.6}, {:.6}", g.latitude, g.longitude),
        ));
    } else if gps_tags > 0 {
        out.push(find(
            Severity::High,
            Category::Location,
            S,
            "GPS data",
            format!("{gps_tags} tags"),
        ));
    }
    if thumbnail {
        out.push(find(
            Severity::Medium,
            Category::Thumbnail,
            S,
            "Embedded thumbnail",
            "a small copy that may show the photo before cropping or edits",
        ));
    }
    if settings > 0 {
        out.push(find(
            Severity::Low,
            Category::Other,
            S,
            "Camera settings",
            format!("{settings} tags (exposure, lens, flash, ...)"),
        ));
    }
    gps
}

fn block_findings(
    format: Format,
    bytes: &[u8],
    blk: &container::Block,
    exif_read: bool,
    out: &mut Vec<Finding>,
) {
    let text = || container::block_text(format, bytes, blk).unwrap_or_default();
    match blk.kind {
        BlockKind::Picture | BlockKind::Icc => {}
        BlockKind::Exif => {
            if !exif_read {
                out.push(find(
                    Severity::Medium,
                    Category::Other,
                    "EXIF",
                    "EXIF block (couldn't be read)",
                    format!("{} bytes", blk.len()),
                ));
            }
        }
        BlockKind::Xmp | BlockKind::XmpExtended => xmp_findings(&text(), blk.len(), out),
        BlockKind::Iptc => {
            let data = &bytes[blk.payload.clone()];
            let records = if format == Format::Jpeg {
                iptc_records(data)
            } else {
                Vec::new()
            };
            if records.is_empty() {
                out.push(find(
                    Severity::Medium,
                    Category::Other,
                    "IPTC",
                    "IPTC / Photoshop block",
                    format!("{} bytes", blk.len()),
                ));
            }
            for (ds, value) in records {
                out.push(iptc_finding(ds, value));
            }
        }
        BlockKind::Comment => {
            let (source, label) = if format == Format::Jpeg {
                ("JPEG comment", "Comment".to_string())
            } else {
                ("PNG text", blk.label.get(5..).unwrap_or("text").to_string())
            };
            let k = label.to_ascii_lowercase();
            let (sev, cat) =
                if k.contains("author") || k.contains("copyright") || k.contains("artist") {
                    (Severity::High, Category::Person)
                } else if k.contains("time") || k.contains("date") {
                    (Severity::Medium, Category::Time)
                } else if k.contains("software") {
                    (Severity::Low, Category::Software)
                } else {
                    (Severity::Medium, Category::Text)
                };
            out.push(find(sev, cat, source, label, clip(&text(), 80)));
        }
        BlockKind::Thumbnail => out.push(find(
            Severity::Medium,
            Category::Thumbnail,
            "JFIF",
            "Embedded thumbnail",
            format!("{} bytes", blk.len()),
        )),
        BlockKind::Mpf => out.push(find(
            Severity::Low,
            Category::Hidden,
            "MPF",
            "Multi-Picture index (extra images after the photo)",
            format!("{} bytes", blk.len()),
        )),
        BlockKind::Time => out.push(find(
            Severity::Low,
            Category::Time,
            "PNG",
            "Last modified (tIME)",
            format!("{} bytes", blk.len()),
        )),
        BlockKind::Vendor => out.push(find(
            Severity::Low,
            Category::Other,
            "container",
            format!("Vendor block {}", blk.label),
            format!("{} bytes", blk.len()),
        )),
        BlockKind::Trailer => {
            let data = &bytes[blk.payload.clone()];
            if data.iter().all(|&b| b == 0) {
                return;
            }
            let motion = contains(data, b"ftyp")
                || contains(bytes, b"MotionPhoto")
                || contains(bytes, b"MicroVideo");
            let (sev, label) = if motion {
                (
                    Severity::High,
                    "Motion-photo video (can hold its own location and audio)",
                )
            } else {
                (Severity::Medium, "Data after the end of the image")
            };
            out.push(find(
                sev,
                Category::Hidden,
                "container",
                label,
                human_bytes(data.len()),
            ));
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

pub fn human_bytes(n: usize) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} bytes"),
    }
}

fn xmp_findings(xml: &str, len: usize, out: &mut Vec<Finding>) {
    const S: &str = "XMP";
    let mut other = 0;
    let mut located = false;
    let start = out.len();
    for (k, v) in xmp::properties(xml) {
        let l = k.to_ascii_lowercase();
        let v = clip(&v, 80);
        let f = if xmp::is_location_property(&k) {
            located = true;
            find(Severity::High, Category::Location, S, k, v)
        } else if l.contains("email")
            || l.contains("phone")
            || l.contains("address")
            || l.starts_with("ci")
            || (l.contains("creator") && l != "creatortool")
            || l.contains("author")
            || l.contains("artist")
            || l.contains("owner")
            || l == "credit"
            || l == "captionwriter"
        {
            find(Severity::High, Category::Person, S, k, v)
        } else if l == "rights" || l.contains("copyright") || l == "usageterms" {
            find(Severity::Medium, Category::Person, S, k, v)
        } else if l.contains("serial") || l == "imageuniqueid" {
            find(Severity::High, Category::Device, S, k, v)
        } else if l.contains("documentid") || l == "instanceid" {
            find(Severity::Medium, Category::Device, S, k, v)
        } else if l.contains("motionphoto") || l.contains("microvideo") {
            find(Severity::Medium, Category::Hidden, S, k, v)
        } else if l.contains("date") || l.ends_with("when") {
            find(Severity::Medium, Category::Time, S, k, v)
        } else if l.contains("software")
            || l == "creatortool"
            || l == "softwareagent"
            || l == "action"
        {
            find(Severity::Low, Category::Software, S, k, v)
        } else if matches!(
            l.as_str(),
            "title" | "description" | "subject" | "headline" | "keywords" | "label"
        ) {
            find(Severity::Medium, Category::Text, S, k, v)
        } else {
            other += 1;
            continue;
        };
        out.push(f);
    }
    if !located && xmp::has_location(xml) {
        out.push(find(
            Severity::High,
            Category::Location,
            S,
            "Location",
            "found in unreadable XMP",
        ));
    }
    if other > 0 || out.len() == start {
        out.push(find(
            Severity::Low,
            Category::Other,
            S,
            "XMP packet",
            format!("{other} other properties, {}", human_bytes(len)),
        ));
    }
}

/// IPTC-IIM datasets (record 2) from a JPEG APP13 Photoshop block:
/// 8BIM resource 0x0404 holds `0x1C 02 dataset len(2) value` records.
pub fn iptc_records(app13: &[u8]) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    let mut pos = app13
        .windows(4)
        .position(|w| w == b"8BIM")
        .unwrap_or(app13.len());
    while pos + 12 <= app13.len() && &app13[pos..pos + 4] == b"8BIM" {
        let id = u16::from_be_bytes([app13[pos + 4], app13[pos + 5]]);
        let name_len = app13[pos + 6] as usize;
        let name_total = (1 + name_len + 1) & !1; // pascal string padded to even
        let size_at = pos + 6 + name_total;
        let Some(size) = app13.get(size_at..size_at + 4) else {
            break;
        };
        let size = u32::from_be_bytes(size.try_into().unwrap()) as usize;
        let data_at = size_at + 4;
        let Some(data) = app13.get(data_at..data_at + size) else {
            break;
        };
        if id == 0x0404 {
            let mut i = 0;
            while i + 5 <= data.len() && data[i] == 0x1C {
                let (rec, ds) = (data[i + 1], data[i + 2]);
                let len = u16::from_be_bytes([data[i + 3], data[i + 4]]) as usize;
                let Some(v) = data.get(i + 5..i + 5 + len) else {
                    break;
                };
                if rec == 2 && ds != 0 {
                    out.push((ds, String::from_utf8_lossy(v).into_owned()));
                }
                i += 5 + len;
            }
        }
        pos = data_at + size + (size & 1);
    }
    out
}

fn iptc_finding(ds: u8, value: String) -> Finding {
    const S: &str = "IPTC";
    let v = clip(&value, 80);
    let (sev, cat, label) = match ds {
        80 => (Severity::High, Category::Person, "By-line (creator)"),
        85 => (Severity::Medium, Category::Person, "By-line title"),
        110 => (Severity::Medium, Category::Person, "Credit"),
        116 => (Severity::Medium, Category::Person, "Copyright"),
        118 => (Severity::High, Category::Person, "Contact"),
        122 => (Severity::Medium, Category::Person, "Caption writer"),
        90 => (Severity::High, Category::Location, "City"),
        92 => (Severity::High, Category::Location, "Sublocation"),
        95 => (Severity::High, Category::Location, "Province / state"),
        100 => (Severity::Medium, Category::Location, "Country code"),
        101 => (Severity::High, Category::Location, "Country"),
        55 | 60 | 62 | 63 => (Severity::Medium, Category::Time, "Date / time created"),
        120 => (Severity::Medium, Category::Text, "Caption"),
        105 => (Severity::Medium, Category::Text, "Headline"),
        5 => (Severity::Medium, Category::Text, "Object name"),
        25 => (Severity::Low, Category::Text, "Keyword"),
        65 => (Severity::Low, Category::Software, "Originating program"),
        _ => (Severity::Low, Category::Other, "IPTC field"),
    };
    let label = if label == "IPTC field" {
        format!("IPTC 2:{ds}")
    } else {
        label.to_string()
    };
    find(sev, cat, S, label, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::container::fixtures::*;

    fn has(a: &Audit, cat: Category, source: &str) -> bool {
        a.findings
            .iter()
            .any(|f| f.category == cat && f.source == source)
    }

    #[test]
    fn finds_every_leak_in_the_leaky_jpeg() {
        let a = audit(&leaky_jpeg());
        assert_eq!(a.risk, Some(Severity::High));
        let gps = a.gps.as_ref().expect("gps");
        assert!((gps.latitude - 45.5).abs() < 1e-6 && (gps.longitude + 122.6).abs() < 1e-6);
        for (cat, src) in [
            (Category::Location, "EXIF"),
            (Category::Device, "EXIF"),
            (Category::Person, "EXIF"),
            (Category::Time, "EXIF"),
            (Category::Location, "XMP"),
            (Category::Person, "XMP"),
            (Category::Person, "IPTC"),
            (Category::Location, "IPTC"),
            (Category::Text, "JPEG comment"),
            (Category::Hidden, "container"),
        ] {
            assert!(
                has(&a, cat, src),
                "missing {cat:?} from {src}: {:#?}",
                a.findings
            );
        }
        let serial = a
            .findings
            .iter()
            .find(|f| f.label == "Camera serial number")
            .unwrap();
        assert_eq!(serial.value, "ABC123");
        let city = a.findings.iter().find(|f| f.label == "City").unwrap();
        assert_eq!(city.value, "Portland");
    }

    #[test]
    fn a_clean_file_has_no_findings() {
        let a = audit(TINY_JPEG);
        assert!(a.findings.is_empty(), "{:#?}", a.findings);
        assert_eq!(a.risk, None);
        assert!(a.blocks.is_empty());
    }

    #[test]
    fn iptc_parser_reads_bylines_and_cities() {
        let seg = iptc_app13();
        let r = iptc_records(&seg[4..]);
        assert_eq!(r, vec![(80, "Raven Test".into()), (90, "Portland".into())]);
        assert!(iptc_records(b"Photoshop 3.0\08BIM\x04\x04\0\0\xff\xff\xff\xff").is_empty());
    }

    #[test]
    fn unknown_formats_still_get_an_answer() {
        let a = audit(b"\0\0\0\x18ftypheic\0\0\0\0");
        assert_eq!(a.format, Format::Heif);
        assert!(!a.scrubbable);
        assert!(a.findings.is_empty());
    }
}
