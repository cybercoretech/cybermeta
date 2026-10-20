//! Image metadata: read, audit, edit, scrub and spoof, without re-encoding
//! pixels.
//!
//! A [`MetaDoc`] holds the file's bytes in memory. Container-level work
//! (scrubbing, location removal, presets) rewrites those bytes; tag edits
//! go through little_exif. Nothing touches the disk until [`MetaDoc::save`].

pub mod audit;
pub mod container;
mod presets;
pub mod scrub;
pub mod summary;
pub mod tags;
pub mod xmp;

pub use audit::{Audit, Category, Finding, Severity};
pub use presets::{apply_preset, list_presets, Preset};
pub use scrub::{Keep, Outcome};
pub use summary::{summarize, Gps, Summary};
pub use tags::{parse_set_assignment, TagGroup, TagView};
pub use xmp::XmpDoc;

use anyhow::{bail, Context, Result};
use little_exif::exif_tag::ExifTag;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ExportTag {
    pub name: String,
    pub hex: String,
    pub group: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Tsv,
}

#[derive(Debug, Clone)]
pub struct MetaDoc {
    pub path: PathBuf,
    /// The file as it will be saved (edits applied).
    pub bytes: Vec<u8>,
    /// EXIF parsed from `bytes`, for the tag browser and edits.
    pub metadata: Metadata,
    pub dirty: bool,
}

/// little_exif errors when a container has no EXIF yet; that's an empty
/// document, so set / spoof still work.
fn read_metadata(bytes: &[u8], path: &Path) -> Result<Metadata> {
    let format = container::detect(bytes);
    let read = match scrub::little_ext(format) {
        Some(ext) => Metadata::new_from_vec(&bytes.to_vec(), ext),
        None => Metadata::new_from_path(path),
    };
    match read {
        Ok(m) => Ok(m),
        Err(err) if err.to_string().contains("No EXIF") => Ok(Metadata::new()),
        // A file whose EXIF little_exif can't parse can still be audited and
        // scrubbed; show it as having no editable tags.
        Err(_) if format.scrubbable() => Ok(Metadata::new()),
        Err(err) => {
            Err(err).with_context(|| format!("failed to read EXIF from {}", path.display()))
        }
    }
}

/// Remove GPS tags (and the GPS IFD pointer) from parsed EXIF.
pub(crate) fn gps_strip(m: &mut Metadata) -> usize {
    let before = tags::collect_tags(m).len();
    let gps: Vec<(u16, ExifTagGroup)> = tags::collect_tags(m)
        .into_iter()
        .filter(|t| t.group == TagGroup::Gps || t.name.starts_with("GPS"))
        .map(|t| (t.hex, t.group.into()))
        .collect();
    for (hex, group) in gps {
        m.remove_tag_by_hex_group(hex, group);
    }
    // GPSInfo is an IFD offset pointer (skipped by the value browser).
    m.remove_tag(ExifTag::GPSInfo(vec![0]));
    before.saturating_sub(tags::collect_tags(m).len())
}

impl MetaDoc {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            bail!("not a file: {}", path.display());
        }
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        Self::from_bytes(path, bytes)
    }

    /// A document for bytes that came from somewhere other than a file
    /// (a share sheet, a content URI); `path` names it and sets the
    /// default output name.
    pub fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self> {
        let metadata = read_metadata(&bytes, &path)?;
        Ok(Self {
            path,
            bytes,
            metadata,
            dirty: false,
        })
    }

    fn replace_bytes(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.metadata = read_metadata(&bytes, &self.path)?;
        self.bytes = bytes;
        self.dirty = true;
        Ok(())
    }

    /// Write `self.metadata` back into the bytes.
    fn commit_metadata(&mut self) -> Result<()> {
        let m = self.metadata.clone();
        let bytes = scrub::edit_exif(&self.bytes, |target| *target = m)?;
        self.replace_bytes(bytes)
    }

    /// What this file gives away.
    pub fn audit(&self) -> Audit {
        audit::audit(&self.bytes)
    }

    /// Typed summary of camera, exposure, capture time and location.
    pub fn summary(&self) -> Summary {
        summarize(&self.metadata)
    }

    pub fn tags(&self) -> Vec<TagView> {
        tags::collect_tags(&self.metadata)
    }

    pub fn export_tags(&self) -> Vec<ExportTag> {
        self.tags()
            .into_iter()
            .map(|t| ExportTag {
                name: t.name,
                hex: format!("0x{:04X}", t.hex),
                group: t.group.as_str().to_string(),
                value: t.value,
            })
            .collect()
    }

    pub fn export_string(&self, format: ExportFormat) -> Result<String> {
        let rows = self.export_tags();
        match format {
            ExportFormat::Json => Ok(serde_json::to_string_pretty(&rows)?),
            ExportFormat::Tsv => {
                let mut out = String::from("name\thex\tgroup\tvalue\n");
                for row in rows {
                    let value = row.value.replace(['\t', '\n'], " ");
                    out.push_str(&format!(
                        "{}\t{}\t{}\t{}\n",
                        row.name, row.hex, row.group, value
                    ));
                }
                Ok(out)
            }
        }
    }

    fn apply(&mut self, outcome: Result<Outcome>) -> Result<Outcome> {
        let outcome = outcome?;
        self.replace_bytes(outcome.bytes.clone())?;
        Ok(outcome)
    }

    /// Remove location: EXIF GPS plus XMP / IPTC blocks that name a place.
    pub fn strip_gps(&mut self) -> Result<Outcome> {
        let o = scrub::strip_location(&self.bytes);
        self.apply(o)
    }

    /// Remove everything but the picture and what `keep` allows, then
    /// check the result. See [`scrub::scrub`].
    pub fn scrub(&mut self, keep: &Keep) -> Result<Outcome> {
        let o = scrub::scrub(&self.bytes, keep);
        self.apply(o)
    }

    pub fn date_shift(&mut self, seconds: i64) -> Result<Outcome> {
        let o = scrub::date_shift(&self.bytes, seconds);
        self.apply(o)
    }

    pub fn set_tag_from_assignment(&mut self, assignment: &str) -> Result<()> {
        let tag = parse_set_assignment(assignment)?;
        self.metadata.set_tag(tag);
        self.commit_metadata()
    }

    pub fn set_string_tag(&mut self, name: &str, value: &str) -> Result<()> {
        let tag = tags::string_tag_by_name(name, value)?;
        self.metadata.set_tag(tag);
        self.commit_metadata()
    }

    pub fn apply_preset(&mut self, preset: Preset) -> Result<Outcome> {
        let o = apply_preset(&self.bytes, preset);
        self.apply(o)
    }

    /// Write the bytes. Prefer `-o` / derived path; `--in-place` overwrites
    /// the source. Writes go to a temporary file beside the target and are
    /// renamed over it, so a failure never leaves half a photo behind.
    pub fn save(&mut self, output: Option<&Path>, in_place: bool) -> Result<PathBuf> {
        let dest = resolve_output(&self.path, output, in_place)?;
        write_atomic(&dest, &self.bytes)?;
        self.path = dest.clone();
        self.dirty = false;
        Ok(dest)
    }
}

pub fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<()> {
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("image");
    let tmp = dest.with_file_name(format!(".{name}.cybermeta-tmp"));
    fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, dest).with_context(|| {
        let _ = fs::remove_file(&tmp);
        format!("rename to {}", dest.display())
    })
}

pub fn resolve_output(src: &Path, output: Option<&Path>, in_place: bool) -> Result<PathBuf> {
    if let Some(out) = output {
        return Ok(out.to_path_buf());
    }
    if in_place {
        return Ok(src.to_path_buf());
    }
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
    let parent = src.parent().unwrap_or_else(|| Path::new("."));
    Ok(parent.join(format!("{stem}.cybermeta.{ext}")))
}

pub fn supported_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "tif" | "tiff" | "png" | "webp" | "jxl" | "heic" | "heif" | "avif")
    )
}

/// Image files under `paths`: files as given, directories walked (one
/// level, or all the way down with `recursive`). Hidden files and our own
/// `*.cybermeta.*` outputs are skipped inside directories.
pub fn collect_images(paths: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .with_context(|| format!("read {}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                if recursive {
                    walk(&p, recursive, out)?;
                }
            } else if supported_extension(&p) && !name.contains(".cybermeta.") {
                out.push(p);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, recursive, &mut out)?;
        } else if p.is_file() {
            out.push(p.clone());
        } else {
            bail!("not found: {}", p.display());
        }
    }
    Ok(out)
}
