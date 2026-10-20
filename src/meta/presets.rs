//! Named scrub / spoof profiles.

use super::scrub::{self, Keep, Outcome};
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Everything goes; colour profile and rotation stay.
    ShareSafe,
    StripGps,
    AnonymousCamera,
    /// Everything goes, colour profile and rotation included.
    ClearAll,
}

impl Preset {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "share-safe" | "share_safe" | "share" | "safe" => Some(Self::ShareSafe),
            "strip-gps" | "strip_gps" | "gps" | "location" => Some(Self::StripGps),
            "anonymous-camera" | "anonymous_camera" | "anon" => Some(Self::AnonymousCamera),
            "clear-all" | "clear_all" | "clear" | "all" => Some(Self::ClearAll),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShareSafe => "share-safe",
            Self::StripGps => "strip-gps",
            Self::AnonymousCamera => "anonymous-camera",
            Self::ClearAll => "clear-all",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ShareSafe => "Remove all metadata; keep colour profile + rotation",
            Self::StripGps => "Remove location only (EXIF GPS, XMP/IPTC places)",
            Self::AnonymousCamera => "Remove all metadata, then claim a generic camera",
            Self::ClearAll => "Pixels only: no metadata, no colour profile, no rotation",
        }
    }
}

pub fn list_presets() -> &'static [Preset] {
    &[
        Preset::ShareSafe,
        Preset::StripGps,
        Preset::AnonymousCamera,
        Preset::ClearAll,
    ]
}

/// Date shifting takes an offset, so it's `spoof --shift` rather than a
/// preset.
pub fn apply_preset(bytes: &[u8], preset: Preset) -> Result<Outcome> {
    match preset {
        Preset::ShareSafe => scrub::scrub(bytes, &Keep::default()),
        Preset::StripGps => scrub::strip_location(bytes),
        Preset::AnonymousCamera => scrub::anonymous_camera(bytes),
        Preset::ClearAll => scrub::scrub(bytes, &Keep::NOTHING),
    }
}
