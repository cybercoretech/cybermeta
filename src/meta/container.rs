//! Lossless container surgery for JPEG, PNG and WebP.
//!
//! A file is split into the blocks it is made of, each one labelled with
//! what it carries (picture data, EXIF, XMP, IPTC, a colour profile, a
//! comment, bytes hidden after the end of the image, ...). Scrubbing then
//! rebuilds the file from the blocks a [`Policy`] allows. Picture data is
//! copied byte for byte: nothing is decoded or re-encoded.
//!
//! This is deliberately independent of little_exif, which only ever sees
//! the EXIF block and so left XMP, IPTC, comments and trailers behind.

use anyhow::{bail, Result};
use serde::Serialize;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Heif,
    Avif,
    Tiff,
    Jxl,
    Unknown,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Webp => "WebP",
            Self::Heif => "HEIF/HEIC",
            Self::Avif => "AVIF",
            Self::Tiff => "TIFF",
            Self::Jxl => "JPEG XL",
            Self::Unknown => "unknown",
        }
    }

    /// Formats the scrubber can rebuild.
    pub fn scrubbable(self) -> bool {
        matches!(self, Self::Jpeg | Self::Png | Self::Webp)
    }
}

pub fn detect(bytes: &[u8]) -> Format {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Format::Jpeg;
    }
    if bytes.starts_with(PNG_SIG) {
        return Format::Png;
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Format::Webp;
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return Format::Tiff;
    }
    if bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(b"\0\0\0\x0CJXL \r\n\x87\n") {
        return Format::Jxl;
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return match &bytes[8..12] {
            b"avif" | b"avis" => Format::Avif,
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => {
                Format::Heif
            }
            _ => Format::Unknown,
        };
    }
    Format::Unknown
}

/// What a block carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    /// Needed to draw the picture (headers, tables, pixel data).
    Picture,
    Exif,
    Xmp,
    /// XMP too large for one JPEG segment, continued here.
    XmpExtended,
    /// IPTC / Photoshop resources (JPEG APP13, PNG "Raw profile type iptc").
    Iptc,
    /// ICC colour profile.
    Icc,
    /// JPEG COM segment or a PNG text chunk.
    Comment,
    /// A preview image embedded in the header (JFIF / JFXX thumbnails).
    Thumbnail,
    /// Multi-Picture Format index (points at extra images in the trailer).
    Mpf,
    /// PNG tIME: the last-modified time.
    Time,
    /// A vendor block nobody needs to draw the picture (APP12 Ducky, FPXR, ...).
    Vendor,
    /// Bytes after the end of the image: Samsung / Pixel motion photos,
    /// MPF depth or HDR maps, or anything else appended.
    Trailer,
}

impl BlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::Exif => "EXIF",
            Self::Xmp => "XMP",
            Self::XmpExtended => "XMP (extended)",
            Self::Iptc => "IPTC / Photoshop",
            Self::Icc => "ICC colour profile",
            Self::Comment => "comment / text",
            Self::Thumbnail => "embedded thumbnail",
            Self::Mpf => "Multi-Picture index",
            Self::Time => "modification time",
            Self::Vendor => "vendor block",
            Self::Trailer => "data after end of image",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub kind: BlockKind,
    /// e.g. "APP1 Exif", "tEXt Comment", "XMP chunk".
    pub label: String,
    /// The whole block as it sits in the file (marker/header included).
    pub range: Range<usize>,
    /// Where the payload starts inside `range` (after markers, signatures).
    pub payload: Range<usize>,
}

impl Block {
    fn new(
        kind: BlockKind,
        label: impl Into<String>,
        range: Range<usize>,
        payload: Range<usize>,
    ) -> Self {
        Self {
            kind,
            label: label.into(),
            range,
            payload,
        }
    }

    pub fn len(&self) -> usize {
        self.range.len()
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

/// A file split into blocks that, concatenated in order, are the file.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub format: Format,
    pub blocks: Vec<Block>,
}

impl Parsed {
    pub fn metadata(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter().filter(|b| b.kind != BlockKind::Picture)
    }
}

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let format = detect(bytes);
    let blocks = match format {
        Format::Jpeg => jpeg_blocks(bytes)?,
        Format::Png => png_blocks(bytes)?,
        Format::Webp => webp_blocks(bytes)?,
        other => bail!(
            "{} can't be scrubbed losslessly yet (JPEG, PNG and WebP can)",
            other.as_str()
        ),
    };
    Ok(Parsed { format, blocks })
}

// ---------------------------------------------------------------- JPEG ----

const XMP_SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_EXT_SIG: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

fn jpeg_blocks(b: &[u8]) -> Result<Vec<Block>> {
    let mut blocks = vec![Block::new(BlockKind::Picture, "SOI", 0..2, 2..2)];
    let mut pos = 2;
    loop {
        // Markers may be preceded by any number of 0xFF fill bytes.
        let start = pos;
        if pos >= b.len() || b[pos] != 0xFF {
            bail!("broken JPEG: expected a marker at byte {pos}");
        }
        while pos < b.len() && b[pos] == 0xFF {
            pos += 1;
        }
        let Some(&marker) = b.get(pos) else {
            bail!("truncated JPEG: no end-of-image marker");
        };
        pos += 1;
        if marker == 0xD9 {
            blocks.push(Block::new(BlockKind::Picture, "EOI", start..pos, pos..pos));
            if pos < b.len() {
                blocks.push(Block::new(
                    BlockKind::Trailer,
                    "after EOI",
                    pos..b.len(),
                    pos..b.len(),
                ));
            }
            return Ok(blocks);
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            blocks.push(Block::new(BlockKind::Picture, "RST", start..pos, pos..pos));
            continue;
        }
        if pos + 2 > b.len() {
            bail!("truncated JPEG segment at byte {start}");
        }
        let len = u16::from_be_bytes([b[pos], b[pos + 1]]) as usize;
        if len < 2 || pos + len > b.len() {
            bail!("broken JPEG segment length at byte {start}");
        }
        let body = pos + 2..pos + len;
        let end = pos + len;
        let data = &b[body.clone()];
        let (kind, label, skip) = jpeg_kind(marker, data);
        let payload = body.start + skip..body.end;
        if marker == 0xDA {
            // Start of scan: entropy-coded data runs until the next real
            // marker (0xFF not followed by 0x00, RSTn or fill).
            let mut i = end;
            loop {
                if i + 1 >= b.len() {
                    bail!("truncated JPEG: scan data runs off the end");
                }
                if b[i] == 0xFF {
                    let n = b[i + 1];
                    if n != 0x00 && !(0xD0..=0xD7).contains(&n) && n != 0xFF {
                        break;
                    }
                }
                i += 1;
            }
            blocks.push(Block::new(
                BlockKind::Picture,
                "SOS + scan",
                start..i,
                payload,
            ));
            pos = i;
        } else {
            blocks.push(Block::new(kind, label, start..end, payload));
            pos = end;
        }
    }
}

fn jpeg_kind(marker: u8, data: &[u8]) -> (BlockKind, String, usize) {
    let app = |n: u8, what: &str| format!("APP{} {what}", n - 0xE0);
    match marker {
        0xE0 if data.starts_with(b"JFIF\0") => {
            // JFIF may carry a thumbnail; it's rewritten without one.
            let kind = if data.len() > 14 {
                BlockKind::Thumbnail
            } else {
                BlockKind::Picture
            };
            (kind, "APP0 JFIF".into(), 0)
        }
        0xE0 if data.starts_with(b"JFXX\0") => {
            (BlockKind::Thumbnail, "APP0 JFXX thumbnail".into(), 5)
        }
        0xE1 if data.starts_with(b"Exif\0\0") => (BlockKind::Exif, "APP1 Exif".into(), 6),
        0xE1 if data.starts_with(b"Exif\0") => (BlockKind::Exif, "APP1 Exif".into(), 5),
        0xE1 if data.starts_with(XMP_SIG) => (BlockKind::Xmp, "APP1 XMP".into(), XMP_SIG.len()),
        0xE1 if data.starts_with(XMP_EXT_SIG) => (
            BlockKind::XmpExtended,
            "APP1 extended XMP".into(),
            XMP_EXT_SIG.len(),
        ),
        0xE2 if data.starts_with(b"ICC_PROFILE\0") => {
            (BlockKind::Icc, "APP2 ICC profile".into(), 14)
        }
        0xE2 if data.starts_with(b"MPF\0") => (BlockKind::Mpf, "APP2 MPF".into(), 4),
        0xED => (BlockKind::Iptc, "APP13 Photoshop / IPTC".into(), 0),
        // Adobe APP14 tells decoders how colours are transformed; dropping
        // it can turn CMYK/RGB JPEGs the wrong colour. It holds no identity.
        0xEE if data.starts_with(b"Adobe") => (BlockKind::Picture, "APP14 Adobe".into(), 0),
        0xE0..=0xEF => (BlockKind::Vendor, app(marker, &signature(data)), 0),
        0xFE => (BlockKind::Comment, "COM".into(), 0),
        _ => (BlockKind::Picture, format!("marker {marker:02X}"), 0),
    }
}

/// The printable signature at the start of an APPn segment, for labels.
fn signature(data: &[u8]) -> String {
    let s: String = data
        .iter()
        .take_while(|&&c| c != 0 && c.is_ascii_graphic())
        .take(24)
        .map(|&c| c as char)
        .collect();
    if s.is_empty() {
        "(unnamed)".into()
    } else {
        s
    }
}

/// JFIF APP0 rebuilt with a 0×0 thumbnail.
fn jfif_without_thumbnail(seg: &[u8]) -> Vec<u8> {
    // FF E0, len(2), "JFIF\0", version(2), units, Xdens(2), Ydens(2), Xthumb, Ythumb
    let mut out = seg[..4 + 14].to_vec();
    out[2..4].copy_from_slice(&16u16.to_be_bytes());
    out[16] = 0;
    out[17] = 0;
    out
}

// ----------------------------------------------------------------- PNG ----

pub const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn png_blocks(b: &[u8]) -> Result<Vec<Block>> {
    let mut blocks = vec![Block::new(BlockKind::Picture, "signature", 0..8, 8..8)];
    let mut pos = 8;
    while pos < b.len() {
        if pos + 12 > b.len() {
            bail!("truncated PNG chunk at byte {pos}");
        }
        let len = u32::from_be_bytes(b[pos..pos + 4].try_into().unwrap()) as usize;
        let ty: [u8; 4] = b[pos + 4..pos + 8].try_into().unwrap();
        let end = pos
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .filter(|&e| e <= b.len());
        let Some(end) = end else {
            bail!("broken PNG chunk length at byte {pos}");
        };
        let data = pos + 8..pos + 8 + len;
        let name = String::from_utf8_lossy(&ty).to_string();
        let (kind, label) = png_kind(&ty, &b[data.clone()]);
        blocks.push(Block::new(kind, label.unwrap_or(name), pos..end, data));
        pos = end;
        if &ty == b"IEND" {
            if pos < b.len() {
                blocks.push(Block::new(
                    BlockKind::Trailer,
                    "after IEND",
                    pos..b.len(),
                    pos..b.len(),
                ));
            }
            return Ok(blocks);
        }
    }
    bail!("truncated PNG: no IEND chunk")
}

fn png_kind(ty: &[u8; 4], data: &[u8]) -> (BlockKind, Option<String>) {
    let keyword = || {
        let k: String = data
            .iter()
            .take_while(|&&c| c != 0)
            .take(79)
            .map(|&c| c as char)
            .collect();
        k
    };
    match ty {
        b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" | b"tRNS" | b"gAMA" | b"cHRM" | b"sRGB" | b"sBIT"
        | b"bKGD" | b"pHYs" | b"acTL" | b"fcTL" | b"fdAT" | b"cICP" | b"mDCv" | b"cLLi"
        | b"mDCV" | b"cLLI" => (BlockKind::Picture, None),
        b"iCCP" => (BlockKind::Icc, Some("iCCP colour profile".into())),
        b"eXIf" | b"exIf" => (BlockKind::Exif, Some("eXIf".into())),
        b"tIME" => (BlockKind::Time, Some("tIME".into())),
        b"tEXt" | b"zTXt" | b"iTXt" => {
            let k = keyword();
            let label = format!("{} {k}", String::from_utf8_lossy(ty));
            let lower = k.to_ascii_lowercase();
            let kind = if k == "XML:com.adobe.xmp" || lower == "raw profile type xmp" {
                BlockKind::Xmp
            } else if lower == "raw profile type exif" || lower == "raw profile type app1" {
                BlockKind::Exif
            } else if lower == "raw profile type iptc" || lower == "raw profile type 8bim" {
                BlockKind::Iptc
            } else if lower == "raw profile type icc" || lower == "raw profile type icm" {
                BlockKind::Icc
            } else {
                BlockKind::Comment
            };
            (kind, Some(label))
        }
        // Any other critical chunk (uppercase first letter) is needed to
        // decode; unknown ancillary chunks are someone's private data.
        _ if ty[0].is_ascii_uppercase() => (BlockKind::Picture, None),
        _ => (BlockKind::Vendor, None),
    }
}

fn png_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut h = crc32fast::Hasher::new();
    h.update(ty);
    h.update(data);
    out.extend_from_slice(&h.finalize().to_be_bytes());
    out
}

/// Text of a tEXt / zTXt / iTXt chunk (keyword stripped, inflated).
pub fn png_text(ty: &str, data: &[u8]) -> Option<String> {
    let nul = data.iter().position(|&c| c == 0)?;
    let rest = &data[nul + 1..];
    let raw = match ty {
        "tEXt" => rest.to_vec(),
        "zTXt" => miniz_oxide::inflate::decompress_to_vec_zlib(rest.get(1..)?).ok()?,
        "iTXt" => {
            let (flag, rest) = (*rest.first()?, rest.get(2..)?);
            // language tag \0 translated keyword \0 text
            let a = rest.iter().position(|&c| c == 0)?;
            let rest = &rest[a + 1..];
            let b = rest.iter().position(|&c| c == 0)?;
            let text = &rest[b + 1..];
            if flag == 1 {
                miniz_oxide::inflate::decompress_to_vec_zlib(text).ok()?
            } else {
                text.to_vec()
            }
        }
        _ => return None,
    };
    Some(String::from_utf8_lossy(&raw).into_owned())
}

// ---------------------------------------------------------------- WebP ----

const VP8X_ICC: u8 = 0x20;
const VP8X_EXIF: u8 = 0x08;
const VP8X_XMP: u8 = 0x04;

fn webp_blocks(b: &[u8]) -> Result<Vec<Block>> {
    let riff_len = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let riff_end = riff_len.checked_add(8).filter(|&e| e <= b.len());
    let Some(riff_end) = riff_end else {
        bail!("truncated WebP: RIFF size runs past the end of the file");
    };
    let mut blocks = vec![Block::new(BlockKind::Picture, "RIFF header", 0..12, 12..12)];
    let mut pos = 12;
    while pos < riff_end {
        if pos + 8 > riff_end {
            bail!("truncated WebP chunk at byte {pos}");
        }
        let fourcc: [u8; 4] = b[pos..pos + 4].try_into().unwrap();
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let padded = len + (len & 1);
        let end = (pos + 8).checked_add(padded).filter(|&e| e <= riff_end);
        let Some(end) = end else {
            bail!("broken WebP chunk length at byte {pos}");
        };
        let data = pos + 8..pos + 8 + len;
        let name = String::from_utf8_lossy(&fourcc).trim_end().to_string();
        let kind = match &fourcc {
            b"VP8X" | b"VP8 " | b"VP8L" | b"ALPH" | b"ANIM" | b"ANMF" => BlockKind::Picture,
            b"ICCP" => BlockKind::Icc,
            b"EXIF" => BlockKind::Exif,
            b"XMP " => BlockKind::Xmp,
            _ => BlockKind::Vendor,
        };
        blocks.push(Block::new(kind, format!("{name} chunk"), pos..end, data));
        pos = end;
    }
    if riff_end < b.len() {
        blocks.push(Block::new(
            BlockKind::Trailer,
            "after RIFF",
            riff_end..b.len(),
            riff_end..b.len(),
        ));
    }
    Ok(blocks)
}

// ------------------------------------------------------------- rebuild ----

/// Which blocks survive a rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub exif: bool,
    pub xmp: bool,
    /// Drop XMP and IPTC blocks only when they name a place (GPS, city,
    /// ...). Ignored for whichever of the two is already dropped.
    pub unless_location: bool,
    pub iptc: bool,
    pub icc: bool,
    pub comments: bool,
    pub thumbnails: bool,
    pub mpf: bool,
    pub time: bool,
    pub vendor: bool,
    pub trailer: bool,
    /// A raw TIFF structure (see [`minimal_tiff`]) written as the file's
    /// only EXIF block.
    pub insert_exif: Option<Vec<u8>>,
}

impl Policy {
    /// Everything goes except the picture and, when asked, its colour profile.
    pub fn scrub(icc: bool, insert_exif: Option<Vec<u8>>) -> Self {
        Self {
            exif: false,
            xmp: false,
            unless_location: false,
            iptc: false,
            icc,
            comments: false,
            thumbnails: false,
            mpf: false,
            time: false,
            vendor: false,
            trailer: false,
            insert_exif,
        }
    }

    /// Keep everything.
    pub fn keep_all() -> Self {
        Self {
            exif: true,
            xmp: true,
            unless_location: false,
            iptc: true,
            icc: true,
            comments: true,
            thumbnails: true,
            mpf: true,
            time: true,
            vendor: true,
            trailer: true,
            insert_exif: None,
        }
    }

    fn keeps(&self, kind: BlockKind) -> bool {
        match kind {
            BlockKind::Picture => true,
            BlockKind::Exif => self.exif && self.insert_exif.is_none(),
            BlockKind::Xmp | BlockKind::XmpExtended => self.xmp,
            BlockKind::Iptc => self.iptc,
            BlockKind::Icc => self.icc,
            BlockKind::Comment => self.comments,
            BlockKind::Thumbnail => self.thumbnails,
            BlockKind::Mpf => self.mpf,
            BlockKind::Time => self.time,
            BlockKind::Vendor => self.vendor,
            BlockKind::Trailer => self.trailer,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Removed {
    pub kind: BlockKind,
    pub label: String,
    pub bytes: usize,
}

impl Removed {
    pub fn kind_label(&self) -> &'static str {
        self.kind.as_str()
    }
}

#[derive(Debug, Clone)]
pub struct Rebuilt {
    pub bytes: Vec<u8>,
    pub removed: Vec<Removed>,
}

pub fn rebuild(src: &[u8], policy: &Policy) -> Result<Rebuilt> {
    let parsed = parse(src)?;
    let mut removed: Vec<Removed> = Vec::new();
    let decide = |blk: &Block| -> bool {
        let mut ok = policy.keeps(blk.kind);
        if ok && policy.unless_location {
            match blk.kind {
                BlockKind::Xmp | BlockKind::XmpExtended => {
                    let text = block_text(parsed.format, src, blk);
                    // Unreadable (compressed, broken) XMP can't be checked: drop it.
                    ok = text.is_some_and(|t| !super::xmp::has_location(&t));
                }
                BlockKind::Iptc => {
                    let data = &src[blk.payload.clone()];
                    let records = super::audit::iptc_records(data);
                    ok = parsed.format == Format::Jpeg
                        && !records.is_empty()
                        && !records
                            .iter()
                            .any(|(ds, _)| matches!(ds, 90 | 92 | 95 | 100 | 101));
                }
                _ => {}
            }
        }
        ok
    };
    let keep = |blk: &Block, removed: &mut Vec<Removed>| -> bool {
        let ok = decide(blk);
        if !ok {
            removed.push(Removed {
                kind: blk.kind,
                label: blk.label.clone(),
                bytes: blk.len(),
            });
        }
        ok
    };
    let bytes = match parsed.format {
        Format::Jpeg => {
            let mut out = Vec::with_capacity(src.len());
            let mut inserted = policy.insert_exif.is_none();
            for blk in &parsed.blocks {
                let seg = &src[blk.range.clone()];
                if blk.label == "APP0 JFIF"
                    && blk.kind == BlockKind::Thumbnail
                    && !policy.thumbnails
                {
                    out.extend_from_slice(&jfif_without_thumbnail(seg));
                    removed.push(Removed {
                        kind: BlockKind::Thumbnail,
                        label: "JFIF thumbnail".into(),
                        bytes: seg.len() - 18,
                    });
                } else if keep(blk, &mut removed) {
                    out.extend_from_slice(seg);
                }
                // EXIF goes right after SOI / JFIF, where readers look first.
                if !inserted && (blk.label == "SOI" || blk.label == "APP0 JFIF") {
                    let next_is_jfif = blk.label == "SOI"
                        && parsed.blocks.get(1).is_some_and(|b| b.label == "APP0 JFIF");
                    if !next_is_jfif {
                        let tiff = policy.insert_exif.as_ref().unwrap();
                        let mut payload = b"Exif\0\0".to_vec();
                        payload.extend_from_slice(tiff);
                        out.extend_from_slice(&[0xFF, 0xE1]);
                        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
                        out.extend_from_slice(&payload);
                        inserted = true;
                    }
                }
            }
            out
        }
        Format::Png => {
            let mut out = Vec::with_capacity(src.len());
            for blk in &parsed.blocks {
                if keep(blk, &mut removed) {
                    out.extend_from_slice(&src[blk.range.clone()]);
                }
                // eXIf must come before the first IDAT; right after IHDR is safe.
                if blk.label == "IHDR" {
                    if let Some(tiff) = &policy.insert_exif {
                        out.extend_from_slice(&png_chunk(b"eXIf", tiff));
                    }
                }
            }
            out
        }
        Format::Webp => {
            let mut out = Vec::with_capacity(src.len());
            let mut flags_at = None;
            let mut has = (false, false, false); // icc, exif, xmp
            for blk in &parsed.blocks {
                if !keep(blk, &mut removed) {
                    continue;
                }
                if blk.label == "VP8X chunk" {
                    flags_at = Some(out.len() + 8);
                }
                match blk.kind {
                    BlockKind::Icc => has.0 = true,
                    BlockKind::Exif => has.1 = true,
                    BlockKind::Xmp => has.2 = true,
                    _ => {}
                }
                out.extend_from_slice(&src[blk.range.clone()]);
            }
            if let (Some(tiff), Some(_)) = (&policy.insert_exif, flags_at) {
                // WebP stores the bare TIFF structure, after the image data.
                out.extend_from_slice(b"EXIF");
                out.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
                out.extend_from_slice(tiff);
                if tiff.len() % 2 == 1 {
                    out.push(0);
                }
                has.1 = true;
            }
            if let Some(at) = flags_at {
                let mut f = out[at] & !(VP8X_ICC | VP8X_EXIF | VP8X_XMP);
                if has.0 {
                    f |= VP8X_ICC;
                }
                if has.1 {
                    f |= VP8X_EXIF;
                }
                if has.2 {
                    f |= VP8X_XMP;
                }
                out[at] = f;
            }
            let riff = (out.len() - 8) as u32;
            out[4..8].copy_from_slice(&riff.to_le_bytes());
            out
        }
        _ => unreachable!("parse() rejects other formats"),
    };
    Ok(Rebuilt { bytes, removed })
}

/// The readable text of an XMP / comment block, if it has any.
pub fn block_text(format: Format, src: &[u8], blk: &Block) -> Option<String> {
    let data = &src[blk.payload.clone()];
    match format {
        Format::Png if blk.label.len() > 4 => png_text(&blk.label[..4], data),
        _ => {
            let data = if blk.kind == BlockKind::XmpExtended {
                // GUID(32) + full length(4) + offset(4) precede the text.
                data.get(40..)?
            } else {
                data
            };
            Some(String::from_utf8_lossy(data).into_owned())
        }
    }
}

// ----------------------------------------------------------- tiny EXIF ----

/// A minimal big-endian TIFF structure holding only the orientation and,
/// optionally, the capture time. This is what "keep rotation" writes, so a
/// scrubbed photo still displays upright without carrying anything else.
pub fn minimal_tiff(orientation: Option<u16>, date: Option<&str>) -> Option<Vec<u8>> {
    let orientation = orientation.filter(|o| (2..=8).contains(o));
    let date = date.filter(|d| valid_exif_date(d));
    if orientation.is_none() && date.is_none() {
        return None;
    }
    let mut ifd0: Vec<(u16, u16, u32, [u8; 4])> = Vec::new();
    if let Some(o) = orientation {
        let v = o.to_be_bytes();
        ifd0.push((0x0112, 3, 1, [v[0], v[1], 0, 0]));
    }
    let ifd0_len = 2 + 12 * (ifd0.len() + usize::from(date.is_some())) + 4;
    let exif_ifd_at = 8 + ifd0_len;
    if date.is_some() {
        ifd0.push((0x8769, 4, 1, (exif_ifd_at as u32).to_be_bytes()));
    }
    let mut out = b"MM\0\x2a\0\0\0\x08".to_vec();
    out.extend_from_slice(&(ifd0.len() as u16).to_be_bytes());
    for (tag, ty, count, value) in &ifd0 {
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&ty.to_be_bytes());
        out.extend_from_slice(&count.to_be_bytes());
        out.extend_from_slice(value);
    }
    out.extend_from_slice(&0u32.to_be_bytes());
    if let Some(d) = date {
        let text_at = (exif_ifd_at + 2 + 12 + 4) as u32;
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&0x9003u16.to_be_bytes());
        out.extend_from_slice(&2u16.to_be_bytes());
        out.extend_from_slice(&20u32.to_be_bytes());
        out.extend_from_slice(&text_at.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(d.as_bytes());
        out.push(0);
    }
    Some(out)
}

/// `YYYY:MM:DD HH:MM:SS`
pub fn valid_exif_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 => c == b':',
            10 => c == b' ',
            13 | 16 => c == b':',
            _ => c.is_ascii_digit(),
        })
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Hand-built files: every metadata block a real camera or editor
    //! writes, around a tiny but valid picture.

    use super::*;

    /// 8×8 grey baseline JPEG produced by libjpeg, with nothing else in it.
    pub const TINY_JPEG: &[u8] = include_bytes!("../../tests/fixtures/tiny.jpg");

    fn seg(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    pub const XMP_WITH_GPS: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:exif="http://ns.adobe.com/exif/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" exif:GPSLatitude="45,30.0N" exif:GPSLongitude="122,36.0W"><dc:creator><rdf:Seq><rdf:li>Raven Test</rdf:li></rdf:Seq></dc:creator></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#;

    pub const XMP_PLAIN: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="Darktable"/></rdf:RDF></x:xmpmeta>"#;

    /// IPTC By-line (2:80) "Raven Test" and City (2:90) "Portland" inside a
    /// Photoshop 8BIM 0x0404 resource.
    pub fn iptc_app13() -> Vec<u8> {
        let mut iim = Vec::new();
        for (ds, text) in [(80u8, &b"Raven Test"[..]), (90, b"Portland")] {
            iim.extend_from_slice(&[0x1C, 2, ds]);
            iim.extend_from_slice(&(text.len() as u16).to_be_bytes());
            iim.extend_from_slice(text);
        }
        let mut body = b"Photoshop 3.0\0".to_vec();
        body.extend_from_slice(b"8BIM");
        body.extend_from_slice(&0x0404u16.to_be_bytes());
        body.extend_from_slice(&[0, 0]); // empty pascal name, padded
        body.extend_from_slice(&(iim.len() as u32).to_be_bytes());
        body.extend_from_slice(&iim);
        if iim.len() % 2 == 1 {
            body.push(0);
        }
        seg(0xED, &body)
    }

    /// A TIFF with GPS, serial number, owner name, a MakerNote and a date.
    pub fn leaky_tiff() -> Vec<u8> {
        // Built with little_exif so the fixture doesn't share code with
        // minimal_tiff().
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata;
        use little_exif::rational::uR64;
        let mut m = Metadata::new();
        let r = |n| uR64 {
            nominator: n,
            denominator: 1,
        };
        m.set_tag(ExifTag::Make("Fujifilm".into()));
        m.set_tag(ExifTag::Model("X-T5".into()));
        m.set_tag(ExifTag::Orientation(vec![6]));
        m.set_tag(ExifTag::Artist("Raven Test".into()));
        m.set_tag(ExifTag::DateTimeOriginal("2026:10:01 21:14:05".into()));
        m.set_tag(ExifTag::SerialNumber("ABC123".into()));
        m.set_tag(ExifTag::OwnerName("Raven Test".into()));
        m.set_tag(ExifTag::MakerNote(b"FUJIFILM serial 0042".to_vec()));
        m.set_tag(ExifTag::GPSLatitudeRef("N".into()));
        m.set_tag(ExifTag::GPSLatitude(vec![r(45), r(30), r(0)]));
        m.set_tag(ExifTag::GPSLongitudeRef("W".into()));
        m.set_tag(ExifTag::GPSLongitude(vec![r(122), r(36), r(0)]));
        let app1 = m.as_u8_vec(FileExtension::JPEG).expect("encode exif");
        // as_u8_vec(JPEG) returns the whole APP1 segment, marker included.
        app1
    }

    /// TINY_JPEG with every kind of leak: EXIF (GPS, serial, owner,
    /// MakerNote), XMP (GPS, creator), IPTC (name, city), a comment, a
    /// vendor APP12 block and a motion-photo style trailer.
    pub fn leaky_jpeg() -> Vec<u8> {
        let mut xmp = XMP_SIG.to_vec();
        xmp.extend_from_slice(XMP_WITH_GPS.as_bytes());
        let mut out = TINY_JPEG[..2].to_vec();
        out.extend_from_slice(&leaky_tiff());
        out.extend_from_slice(&seg(0xE1, &xmp));
        out.extend_from_slice(&iptc_app13());
        out.extend_from_slice(&seg(0xEC, b"Ducky\0\x01\x00\x04\x00\x00\x00\x50"));
        out.extend_from_slice(&seg(0xFE, b"home address: 12 Elm St"));
        out.extend_from_slice(&TINY_JPEG[2..]);
        out.extend_from_slice(b"\0\0\0\x18ftypmp42MotionPhoto_Data trailer");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn detects_formats() {
        assert_eq!(detect(TINY_JPEG), Format::Jpeg);
        assert_eq!(detect(PNG_SIG), Format::Png);
        assert_eq!(detect(b"RIFF\0\0\0\0WEBPVP8 "), Format::Webp);
        assert_eq!(detect(b"\0\0\0\x18ftypheic\0\0\0\0"), Format::Heif);
        assert_eq!(detect(b"\0\0\0\x18ftypavif\0\0\0\0"), Format::Avif);
        assert_eq!(detect(b"II*\0...."), Format::Tiff);
        assert!(parse(b"\0\0\0\x18ftypheic\0\0\0\0").is_err());
    }

    #[test]
    fn jpeg_blocks_concatenate_back_to_the_file() {
        let src = leaky_jpeg();
        let p = parse(&src).unwrap();
        let joined: Vec<u8> = p
            .blocks
            .iter()
            .flat_map(|b| src[b.range.clone()].to_vec())
            .collect();
        assert_eq!(joined, src);
        let kinds: Vec<BlockKind> = p.metadata().map(|b| b.kind).collect();
        for k in [
            BlockKind::Exif,
            BlockKind::Xmp,
            BlockKind::Iptc,
            BlockKind::Vendor,
            BlockKind::Comment,
            BlockKind::Trailer,
        ] {
            assert!(kinds.contains(&k), "missing {k:?} in {kinds:?}");
        }
    }

    #[test]
    fn scrub_keeps_the_picture_byte_for_byte() {
        let src = leaky_jpeg();
        let out = rebuild(&src, &Policy::scrub(true, None)).unwrap().bytes;
        assert_eq!(out, TINY_JPEG);
        // Idempotent.
        assert_eq!(
            rebuild(&out, &Policy::scrub(true, None)).unwrap().bytes,
            out
        );
    }

    #[test]
    fn keep_all_is_the_identity() {
        let src = leaky_jpeg();
        assert_eq!(rebuild(&src, &Policy::keep_all()).unwrap().bytes, src);
    }

    #[test]
    fn location_only_drops_xmp_that_has_location() {
        let mut xmp_plain = XMP_SIG.to_vec();
        xmp_plain.extend_from_slice(XMP_PLAIN.as_bytes());
        let mut src = TINY_JPEG[..2].to_vec();
        let mut gps = XMP_SIG.to_vec();
        gps.extend_from_slice(XMP_WITH_GPS.as_bytes());
        src.extend_from_slice(&[0xFF, 0xE1]);
        src.extend_from_slice(&((gps.len() + 2) as u16).to_be_bytes());
        src.extend_from_slice(&gps);
        src.extend_from_slice(&[0xFF, 0xE1]);
        src.extend_from_slice(&((xmp_plain.len() + 2) as u16).to_be_bytes());
        src.extend_from_slice(&xmp_plain);
        src.extend_from_slice(&TINY_JPEG[2..]);
        let policy = Policy {
            unless_location: true,
            ..Policy::keep_all()
        };
        let r = rebuild(&src, &policy).unwrap();
        assert_eq!(r.removed.len(), 1);
        let text = String::from_utf8_lossy(&r.bytes);
        assert!(!text.contains("GPSLatitude"));
        assert!(text.contains("Darktable"));
    }

    #[test]
    fn jfif_thumbnails_are_cut_out() {
        let mut jfif = b"JFIF\0\x01\x02\x00\x00\x48\x00\x48\x02\x01".to_vec();
        jfif.extend_from_slice(&[0xAB; 6]); // 2×1 RGB thumbnail
        let mut src = TINY_JPEG[..2].to_vec();
        src.extend_from_slice(&[0xFF, 0xE0]);
        src.extend_from_slice(&((jfif.len() + 2) as u16).to_be_bytes());
        src.extend_from_slice(&jfif);
        src.extend_from_slice(&TINY_JPEG[2..]);
        let out = rebuild(&src, &Policy::scrub(true, None)).unwrap();
        assert!(!out.bytes.windows(6).any(|w| w == [0xAB; 6]));
        assert_eq!(out.removed[0].kind, BlockKind::Thumbnail);
        assert!(parse(&out.bytes).unwrap().metadata().next().is_none());
    }

    #[test]
    fn minimal_tiff_holds_orientation_and_date_only() {
        assert!(minimal_tiff(Some(1), None).is_none(), "1 is the default");
        assert!(minimal_tiff(None, Some("yesterday")).is_none());
        let t = minimal_tiff(Some(6), Some("2026:10:01 21:14:05")).unwrap();
        let mut jpeg = TINY_JPEG[..2].to_vec();
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&((t.len() + 8) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&t);
        jpeg.extend_from_slice(&TINY_JPEG[2..]);
        let m = little_exif::metadata::Metadata::new_from_vec(
            &jpeg,
            little_exif::filetype::FileExtension::JPEG,
        )
        .unwrap();
        let tags: Vec<String> = m.into_iter().map(|t| format!("{t:?}")).collect();
        assert!(tags.iter().any(|t| t == "Orientation([6])"), "{tags:?}");
        assert!(
            tags.iter().any(|t| t.contains("2026:10:01 21:14:05")),
            "{tags:?}"
        );
    }

    fn png_with(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut v = PNG_SIG.to_vec();
        // 1×1 greyscale
        v.extend_from_slice(&png_chunk(
            b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0],
        ));
        for (ty, data) in chunks {
            v.extend_from_slice(&png_chunk(ty, data));
        }
        let idat = miniz_oxide::deflate::compress_to_vec_zlib(&[0, 0x80], 6);
        v.extend_from_slice(&png_chunk(b"IDAT", &idat));
        v.extend_from_slice(&png_chunk(b"IEND", &[]));
        v
    }

    #[test]
    fn png_text_exif_and_time_are_dropped_and_crcs_stay_valid() {
        let ztxt = {
            let mut d = b"Comment\0\0".to_vec();
            d.extend(miniz_oxide::deflate::compress_to_vec_zlib(
                b"home address",
                6,
            ));
            d
        };
        let src = png_with(&[
            (b"tEXt", b"Author\0Raven Test".to_vec()),
            (b"zTXt", ztxt),
            (b"tIME", vec![7, 234, 10, 1, 12, 0, 0]),
            (b"eXIf", leaky_tiff()[10..].to_vec()),
            (b"prVt", b"vendor".to_vec()),
        ]);
        let clean = png_with(&[]);
        let r = rebuild(&src, &Policy::scrub(true, None)).unwrap();
        assert_eq!(r.bytes, clean);
        assert_eq!(r.removed.len(), 5);
        let p = parse(&src).unwrap();
        let z = p
            .blocks
            .iter()
            .find(|b| b.label.starts_with("zTXt"))
            .unwrap();
        assert_eq!(
            block_text(Format::Png, &src, z).as_deref(),
            Some("home address")
        );
        // Orientation is written as a fresh eXIf with a valid CRC.
        let t = minimal_tiff(Some(8), None).unwrap();
        let r = rebuild(&src, &Policy::scrub(true, Some(t.clone()))).unwrap();
        let p = parse(&r.bytes).unwrap();
        let e = p.blocks.iter().find(|b| b.kind == BlockKind::Exif).unwrap();
        assert_eq!(&r.bytes[e.payload.clone()], &t[..]);
    }

    fn webp_with(extra: &[(&[u8; 4], &[u8])], flags: u8) -> Vec<u8> {
        let mut body = b"WEBP".to_vec();
        let mut push = |fourcc: &[u8; 4], data: &[u8]| {
            body.extend_from_slice(fourcc);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            if data.len() % 2 == 1 {
                body.push(0);
            }
        };
        push(b"VP8X", &[flags, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for (fourcc, data) in extra.iter().filter(|(f, _)| *f == b"ICCP") {
            push(fourcc, data);
        }
        push(
            b"VP8L",
            &[
                0x2f, 0, 0, 0, 0x10, 0x07, 0x10, 0x11, 0x11, 0x88, 0x88, 0xfe, 0x07, 0x00,
            ],
        );
        for (fourcc, data) in extra.iter().filter(|(f, _)| *f != b"ICCP") {
            push(fourcc, data);
        }
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&body);
        v
    }

    #[test]
    fn webp_exif_and_xmp_go_and_flags_follow() {
        let exif = leaky_tiff()[10..].to_vec();
        let src = webp_with(
            &[
                (b"ICCP", b"fake-icc"),
                (b"EXIF", &exif),
                (b"XMP ", XMP_WITH_GPS.as_bytes()),
            ],
            VP8X_ICC | VP8X_EXIF | VP8X_XMP,
        );
        let r = rebuild(&src, &Policy::scrub(true, None)).unwrap();
        assert_eq!(r.bytes, webp_with(&[(b"ICCP", b"fake-icc")], VP8X_ICC));
        let r = rebuild(&src, &Policy::scrub(false, None)).unwrap();
        assert_eq!(r.bytes, webp_with(&[], 0));
        // Orientation: a fresh EXIF chunk and the flag set again.
        let t = minimal_tiff(Some(3), None).unwrap();
        let r = rebuild(&src, &Policy::scrub(false, Some(t.clone()))).unwrap();
        assert_eq!(r.bytes, webp_with(&[(b"EXIF", &t)], VP8X_EXIF));
    }

    #[test]
    fn truncated_files_are_errors_not_panics() {
        let src = leaky_jpeg();
        for cut in [3, 10, 40, src.len() / 2] {
            let _ = parse(&src[..cut]);
        }
        let png = png_with(&[]);
        for cut in [9, 20, png.len() - 3] {
            assert!(parse(&png[..cut]).is_err());
        }
        assert!(parse(b"RIFF\xff\xff\xff\x00WEBPVP8X").is_err());
    }
}
