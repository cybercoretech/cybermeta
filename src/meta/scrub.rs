//! The operations that make a photo safe to share, on in-memory bytes so
//! the CLI, the TUI and the Android app run exactly the same code.
//!
//! Every operation audits its own output and fails, writing nothing, if
//! the output still carries what it was meant to remove.

use super::audit::{self, Audit, Category, Finding};
use super::container::{self, minimal_tiff, Format, Policy, Removed};
use super::gps_strip;
use anyhow::{bail, Context, Result};
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata;
use nom_exif::{EntryValue, MediaParser, MediaSource};
use serde::{Deserialize, Serialize};

/// What a full scrub may leave in the file. Nothing here identifies you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keep {
    /// The ICC colour profile, so colours don't shift (phones shoot
    /// Display P3; without the profile they render washed out).
    pub icc: bool,
    /// Rotation, written as a fresh EXIF block holding nothing else.
    pub orientation: bool,
    /// The capture time (DateTimeOriginal), in that same fresh block.
    pub date: bool,
}

impl Default for Keep {
    fn default() -> Self {
        Self {
            icc: true,
            orientation: true,
            date: false,
        }
    }
}

impl Keep {
    pub const NOTHING: Keep = Keep {
        icc: false,
        orientation: false,
        date: false,
    };

    /// "icc,orientation,date", "none" or "".
    pub fn parse(list: &str) -> Result<Keep> {
        let mut keep = Keep::NOTHING;
        for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match item.to_ascii_lowercase().as_str() {
                "icc" | "color" | "colour" | "profile" => keep.icc = true,
                "orientation" | "rotation" => keep.orientation = true,
                "date" | "time" => keep.date = true,
                "none" => {}
                other => {
                    bail!("unknown --keep item `{other}` (use icc, orientation, date or none)")
                }
            }
        }
        Ok(keep)
    }

    pub fn describe(&self) -> String {
        let mut v = Vec::new();
        if self.icc {
            v.push("colour profile");
        }
        if self.orientation {
            v.push("rotation");
        }
        if self.date {
            v.push("capture date");
        }
        if v.is_empty() {
            "nothing".into()
        } else {
            v.join(", ")
        }
    }
}

/// The result of an operation: new bytes plus a record of what changed.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub bytes: Vec<u8>,
    pub format: Format,
    pub removed: Vec<Removed>,
    pub before: Audit,
    pub after: Audit,
    /// Plain-language notes for the user ("motion photo video removed").
    pub notes: Vec<String>,
}

impl Outcome {
    /// Findings that are gone after the operation.
    pub fn cleared(&self) -> usize {
        self.before
            .findings
            .len()
            .saturating_sub(self.after.findings.len())
    }
}

/// Orientation and capture time as the source file has them.
fn source_values(bytes: &[u8]) -> (Option<u16>, Option<String>) {
    let Some(exif) = MediaSource::from_memory(bytes.to_vec())
        .ok()
        .and_then(|ms| MediaParser::new().parse_exif(ms).ok())
        .map(nom_exif::Exif::from)
    else {
        return (None, None);
    };
    let orientation = exif
        .get(nom_exif::ExifTag::Orientation)
        .and_then(EntryValue::as_u16);
    let date = exif
        .get(nom_exif::ExifTag::DateTimeOriginal)
        .and_then(|v| match v {
            EntryValue::DateTime(d) => Some(d.format("%Y:%m:%d %H:%M:%S").to_string()),
            EntryValue::NaiveDateTime(d) => Some(d.format("%Y:%m:%d %H:%M:%S").to_string()),
            EntryValue::Text(t) => Some(t.trim_end_matches('\0').to_string()),
            _ => None,
        });
    (orientation, date)
}

/// Remove everything but the picture and what `keep` allows.
pub fn scrub(bytes: &[u8], keep: &Keep) -> Result<Outcome> {
    let format = container::detect(bytes);
    if !format.scrubbable() {
        bail!(
            "{} can't be scrubbed losslessly yet. Convert it to JPEG first \
             (on Samsung: Settings → Camera → Advanced picture options → \
             turn off High efficiency pictures)",
            format.as_str()
        );
    }
    let before = audit::audit(bytes);
    let (orientation, date) = source_values(bytes);
    let tiff = minimal_tiff(
        orientation.filter(|_| keep.orientation),
        date.as_deref().filter(|_| keep.date),
    );
    let rebuilt = container::rebuild(bytes, &Policy::scrub(keep.icc, tiff.clone()))?;
    let after = audit::audit(&rebuilt.bytes);

    let left: Vec<&Finding> = after
        .findings
        .iter()
        .filter(|f| !(keep.date && f.category == Category::Time && f.source == "EXIF"))
        .collect();
    let blocks_ok = after.blocks.iter().all(|b| match b.kind {
        container::BlockKind::Icc => keep.icc,
        container::BlockKind::Exif => tiff.is_some(),
        _ => false,
    });
    let raw_leak = [
        &b"<x:xmpmeta"[..],
        b"Photoshop 3.0\0",
        b"http://ns.adobe.com/xap/1.0/",
    ]
    .iter()
    .any(|n| rebuilt.bytes.windows(n.len()).any(|w| w == *n));
    if !left.is_empty() || !blocks_ok || raw_leak || after.damaged.is_some() {
        let what: Vec<String> = left
            .iter()
            .map(|f| format!("{} {}", f.source, f.label))
            .collect();
        bail!(
            "refusing to write: the scrubbed file still carries {} — please report this",
            if what.is_empty() {
                "a metadata block".into()
            } else {
                what.join(", ")
            }
        );
    }

    let mut notes = Vec::new();
    if before
        .findings
        .iter()
        .any(|f| f.category == Category::Hidden && f.source == "container")
    {
        notes.push(
            "Data hidden after the image (e.g. a motion-photo video) was removed; the photo stays."
                .into(),
        );
    }
    if tiff.is_some() {
        notes.push(format!("Kept: {}.", keep.describe()));
    }
    Ok(Outcome {
        bytes: rebuilt.bytes,
        format,
        removed: rebuilt.removed,
        before,
        after,
        notes,
    })
}

pub(crate) fn little_ext(format: Format) -> Option<FileExtension> {
    Some(match format {
        Format::Jpeg => FileExtension::JPEG,
        Format::Png => FileExtension::PNG {
            as_zTXt_chunk: false,
        },
        Format::Webp => FileExtension::WEBP,
        Format::Tiff => FileExtension::TIFF,
        Format::Heif | Format::Avif => FileExtension::HEIF,
        Format::Jxl => FileExtension::JXL,
        Format::Unknown => return None,
    })
}

/// Apply little_exif edits to the EXIF block of `bytes`, leaving the rest
/// of the file alone. Returns the bytes unchanged when there is no EXIF.
pub(crate) fn edit_exif(bytes: &[u8], edit: impl FnOnce(&mut Metadata)) -> Result<Vec<u8>> {
    let format = container::detect(bytes);
    let ext = little_ext(format).context("unknown image format")?;
    let mut m = match Metadata::new_from_vec(&bytes.to_vec(), ext) {
        Ok(m) => m,
        Err(e) if e.to_string().contains("No EXIF") => Metadata::new(),
        Err(e) => return Err(e).context("read EXIF"),
    };
    edit(&mut m);
    let mut out = bytes.to_vec();
    m.write_to_vec(&mut out, ext).context("write EXIF")?;
    Ok(out)
}

/// Remove where the photo was taken: EXIF GPS, and any XMP or IPTC block
/// that names a place. Everything else stays.
pub fn strip_location(bytes: &[u8]) -> Result<Outcome> {
    let format = container::detect(bytes);
    let before = audit::audit(bytes);
    let has_gps = before
        .findings
        .iter()
        .any(|f| f.category == Category::Location && f.source == "EXIF");
    let mut out = if has_gps {
        edit_exif(bytes, |m| {
            gps_strip(m);
        })?
    } else {
        bytes.to_vec()
    };
    let mut removed = Vec::new();
    if format.scrubbable() {
        let policy = Policy {
            unless_location: true,
            ..Policy::keep_all()
        };
        let r = container::rebuild(&out, &policy)?;
        out = r.bytes;
        removed = r.removed;
    }
    let after = audit::audit(&out);
    let left: Vec<String> = after
        .findings
        .iter()
        .filter(|f| f.category == Category::Location)
        .map(|f| format!("{} {}", f.source, f.label))
        .collect();
    if !left.is_empty() {
        bail!(
            "refusing to write: location is still in {}",
            left.join(", ")
        );
    }
    let mut notes = Vec::new();
    if after
        .findings
        .iter()
        .any(|f| f.category == Category::Hidden)
    {
        notes.push(
            "This photo also hides extra data (e.g. a motion-photo video) that can carry its own \
             location. Use a full scrub to remove it."
                .into(),
        );
    }
    Ok(Outcome {
        bytes: out,
        format,
        removed,
        before,
        after,
        notes,
    })
}

/// A time offset like `+3d`, `-2h30m`, `+1d4h`, `90m`, `-45s`.
pub fn parse_shift(text: &str) -> Result<i64> {
    let t = text.trim();
    let (sign, rest) = match t.as_bytes().first() {
        Some(b'-') => (-1, &t[1..]),
        Some(b'+') => (1, &t[1..]),
        _ => (1, t),
    };
    if rest.is_empty() {
        bail!("empty shift; try +3d, -2h30m or 90m");
    }
    let mut total = 0i64;
    let mut num = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: i64 = num.parse().with_context(|| format!("bad shift `{text}`"))?;
        num.clear();
        total += n * match c {
            'd' => 86_400,
            'h' => 3_600,
            'm' => 60,
            's' => 1,
            _ => bail!("bad unit `{c}` in `{text}` (use d, h, m, s)"),
        };
    }
    if !num.is_empty() {
        bail!("`{text}` needs a unit on the last number (d, h, m or s)");
    }
    Ok(sign * total)
}

/// Shift a `YYYY:MM:DD HH:MM:SS` timestamp by `seconds`.
pub fn shift_date(date: &str, seconds: i64) -> Option<String> {
    let d = chrono::NaiveDateTime::parse_from_str(date.trim_end_matches('\0'), "%Y:%m:%d %H:%M:%S")
        .ok()?;
    let shifted = d.checked_add_signed(chrono::Duration::seconds(seconds))?;
    Some(shifted.format("%Y:%m:%d %H:%M:%S").to_string())
}

/// Move every EXIF timestamp by `seconds`, drop the GPS clock, and drop
/// XMP and IPTC (they carry their own, unshifted dates).
pub fn date_shift(bytes: &[u8], seconds: i64) -> Result<Outcome> {
    use little_exif::exif_tag::ExifTag;
    let format = container::detect(bytes);
    let before = audit::audit(bytes);
    let mut failed = false;
    let mut out = edit_exif(bytes, |m| {
        let mut set = Vec::new();
        for tag in m.into_iter() {
            match tag {
                ExifTag::DateTimeOriginal(v) => {
                    set.push(shift_date(v, seconds).map(ExifTag::DateTimeOriginal))
                }
                ExifTag::CreateDate(v) => set.push(shift_date(v, seconds).map(ExifTag::CreateDate)),
                ExifTag::ModifyDate(v) => set.push(shift_date(v, seconds).map(ExifTag::ModifyDate)),
                _ => {}
            }
        }
        for t in set {
            match t {
                Some(t) => m.set_tag(t),
                None => failed = true,
            }
        }
        for hex in [0x0007u16, 0x001D] {
            m.remove_tag_by_hex_group(hex, little_exif::ifd::ExifTagGroup::GPS);
        }
    })?;
    if failed {
        bail!("a timestamp in this file isn't in EXIF's YYYY:MM:DD HH:MM:SS form; nothing was changed");
    }
    let mut removed = Vec::new();
    if format.scrubbable() {
        let policy = Policy {
            xmp: false,
            iptc: false,
            time: false,
            ..Policy::keep_all()
        };
        let r = container::rebuild(&out, &policy)?;
        out = r.bytes;
        removed = r.removed;
    }
    let after = audit::audit(&out);
    Ok(Outcome {
        bytes: out,
        format,
        removed,
        before,
        after,
        notes: vec![format!("Timestamps moved by {}.", human_shift(seconds))],
    })
}

pub fn human_shift(seconds: i64) -> String {
    let sign = if seconds < 0 { "-" } else { "+" };
    let mut s = seconds.unsigned_abs();
    let mut out = String::from(sign);
    for (unit, size) in [("d", 86_400), ("h", 3_600), ("m", 60), ("s", 1)] {
        if s >= size {
            out.push_str(&format!("{}{unit}", s / size));
            s %= size;
        }
    }
    if out.len() == 1 {
        out.push_str("0s");
    }
    out
}

/// Full scrub, then a generic camera identity, so the photo looks like it
/// came from no camera in particular.
pub fn anonymous_camera(bytes: &[u8]) -> Result<Outcome> {
    use little_exif::exif_tag::ExifTag;
    let keep = Keep {
        icc: true,
        orientation: true,
        date: true,
    };
    let mut outcome = scrub(bytes, &keep)?;
    outcome.bytes = edit_exif(&outcome.bytes, |m| {
        m.set_tag(ExifTag::Make("Generic".into()));
        m.set_tag(ExifTag::Model("Camera".into()));
        m.set_tag(ExifTag::Software("cybermeta".into()));
    })?;
    outcome.after = audit::audit(&outcome.bytes);
    outcome
        .notes
        .push("Camera set to \"Generic Camera\".".into());
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::audit::Severity;
    use crate::meta::container::fixtures::*;

    #[test]
    fn scrub_removes_every_leak_the_old_strip_all_missed() {
        let src = leaky_jpeg();
        let out = scrub(&src, &Keep::default()).unwrap();
        assert!(out.after.findings.is_empty(), "{:#?}", out.after.findings);
        let text = String::from_utf8_lossy(&out.bytes);
        for leak in [
            "Raven Test",
            "Portland",
            "ABC123",
            "home address",
            "GPSLatitude",
            "MotionPhoto",
            "Ducky",
        ] {
            assert!(!text.contains(leak), "{leak} survived");
        }
        // Rotation survived as the only EXIF.
        let (orientation, date) = source_values(&out.bytes);
        assert_eq!(orientation, Some(6));
        assert_eq!(date, None);
        assert!(out.notes.iter().any(|n| n.contains("motion")));
    }

    #[test]
    fn keep_date_and_keep_nothing() {
        let src = leaky_jpeg();
        let out = scrub(
            &src,
            &Keep {
                date: true,
                ..Keep::default()
            },
        )
        .unwrap();
        assert_eq!(
            source_values(&out.bytes).1.as_deref(),
            Some("2026:10:01 21:14:05")
        );
        assert!(out
            .after
            .findings
            .iter()
            .all(|f| f.category == Category::Time));
        let out = scrub(&src, &Keep::NOTHING).unwrap();
        assert_eq!(out.bytes, TINY_JPEG);
    }

    #[test]
    fn heic_is_refused_with_a_way_out() {
        let err = scrub(b"\0\0\0\x18ftypheic\0\0\0\0", &Keep::default()).unwrap_err();
        assert!(err.to_string().contains("High efficiency"));
    }

    #[test]
    fn strip_location_keeps_the_rest() {
        let src = leaky_jpeg();
        let out = strip_location(&src).unwrap();
        assert!(
            out.after
                .findings
                .iter()
                .all(|f| f.category != Category::Location),
            "{:#?}",
            out.after.findings
        );
        // Serial number and comment stay: this is the "location only" option.
        assert!(out
            .after
            .findings
            .iter()
            .any(|f| f.label == "Camera serial number"));
        assert!(out
            .after
            .findings
            .iter()
            .any(|f| f.source == "JPEG comment"));
        assert!(out.notes.iter().any(|n| n.contains("motion")));
        assert_eq!(out.before.risk, Some(Severity::High));
    }

    #[test]
    fn shifts_parse_and_apply() {
        assert_eq!(parse_shift("+3d").unwrap(), 3 * 86_400);
        assert_eq!(parse_shift("-2h30m").unwrap(), -(2 * 3600 + 1800));
        assert_eq!(parse_shift("90m").unwrap(), 5400);
        assert!(parse_shift("3").is_err());
        assert!(parse_shift("3w").is_err());
        assert_eq!(human_shift(-(86_400 + 3600 + 5)), "-1d1h5s");
        assert_eq!(
            shift_date("2026:12:31 23:00:00", 3600).as_deref(),
            Some("2027:01:01 00:00:00")
        );
        let out = date_shift(&leaky_jpeg(), -86_400).unwrap();
        let taken = out
            .after
            .findings
            .iter()
            .find(|f| f.label == "Taken")
            .unwrap();
        assert!(
            taken.value.contains("2026-09-30") || taken.value.contains("2026:09:30"),
            "{}",
            taken.value
        );
        assert!(out
            .after
            .findings
            .iter()
            .all(|f| f.source != "XMP" && f.source != "IPTC"));
    }

    #[test]
    fn anonymous_camera_reads_generic() {
        let out = anonymous_camera(&leaky_jpeg()).unwrap();
        let make = out
            .after
            .findings
            .iter()
            .find(|f| f.label == "Camera make")
            .unwrap();
        assert_eq!(make.value, "Generic");
        assert!(
            out.after
                .findings
                .iter()
                .all(|f| f.severity < Severity::High),
            "{:#?}",
            out.after.findings
        );
    }
}
