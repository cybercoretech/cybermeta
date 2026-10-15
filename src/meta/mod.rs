//! EXIF read / write / strip / spoof / export — no pixel re-encode.

mod presets;
pub mod summary;
pub mod tags;
pub mod xmp;

pub use presets::{apply_preset, list_presets, Preset};
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
    pub metadata: Metadata,
    pub dirty: bool,
}

impl MetaDoc {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            bail!("not a file: {}", path.display());
        }
        let metadata = match Metadata::new_from_path(&path) {
            Ok(m) => m,
            Err(err) => {
                let msg = err.to_string();
                // little_exif errors when the container has no APP1/EXIF yet —
                // treat that as an empty document so set/spoof still work.
                if msg.contains("No EXIF data found") {
                    Metadata::new()
                } else {
                    return Err(err)
                        .with_context(|| format!("failed to read EXIF from {}", path.display()));
                }
            }
        };
        Ok(Self {
            path,
            metadata,
            dirty: false,
        })
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

    pub fn strip_gps(&mut self) -> usize {
        let before = self.tags().len();
        let gps_hexes: Vec<(u16, ExifTagGroup)> = self
            .tags()
            .into_iter()
            .filter(|t| t.group == TagGroup::Gps || t.name.starts_with("GPS"))
            .map(|t| (t.hex, t.group.into()))
            .collect();
        for (hex, group) in gps_hexes {
            self.metadata.remove_tag_by_hex_group(hex, group);
        }
        // GPSInfo is an IFD offset pointer (skipped by the value browser).
        self.metadata.remove_tag(ExifTag::GPSInfo(vec![0]));
        self.dirty = true;
        before.saturating_sub(self.tags().len())
    }

    pub fn strip_all(&mut self) -> usize {
        let before = self.tags().len();
        self.metadata.reduce_to_a_minimum();
        self.dirty = true;
        before.saturating_sub(self.tags().len())
    }

    pub fn set_tag_from_assignment(&mut self, assignment: &str) -> Result<()> {
        let tag = parse_set_assignment(assignment)?;
        self.metadata.set_tag(tag);
        self.dirty = true;
        Ok(())
    }

    pub fn set_string_tag(&mut self, name: &str, value: &str) -> Result<()> {
        let tag = tags::string_tag_by_name(name, value)?;
        self.metadata.set_tag(tag);
        self.dirty = true;
        Ok(())
    }

    pub fn remove_named_tag(&mut self, name: &str) -> Result<usize> {
        let (hex, group) = tags::lookup_tag_hex_group(name)?;
        let n = self.metadata.remove_tag_by_hex_group(hex, group);
        if n > 0 {
            self.dirty = true;
        }
        Ok(n)
    }

    pub fn apply_preset(&mut self, preset: Preset) -> Result<()> {
        apply_preset(self, preset)?;
        self.dirty = true;
        Ok(())
    }

    /// Write metadata. Prefer `-o` / derived path; `--in-place` overwrites source.
    pub fn save(&mut self, output: Option<&Path>, in_place: bool) -> Result<PathBuf> {
        let dest = resolve_output(&self.path, output, in_place)?;
        if dest != self.path {
            fs::copy(&self.path, &dest).with_context(|| {
                format!(
                    "copy {} → {} before writing metadata",
                    self.path.display(),
                    dest.display()
                )
            })?;
        }
        self.metadata
            .write_to_file(&dest)
            .with_context(|| format!("failed to write EXIF to {}", dest.display()))?;
        self.path = dest.clone();
        self.dirty = false;
        Ok(dest)
    }
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
