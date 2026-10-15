//! A friendly, typed summary of the tags people care about (camera, lens,
//! exposure, capture time, location), for apps such as Ghostframe:
//! `cybermeta show <file> --summary`.

use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata;
use little_exif::rational::{iR64, uR64};
use serde::Serialize;

/// Every field is optional: images often carry only some of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub make: Option<String>,
    pub model: Option<String>,
    /// "Make Model", without the make repeated when the model already starts with it.
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// e.g. 2.8
    pub aperture: Option<f64>,
    /// Seconds, e.g. 0.004
    pub exposure_seconds: Option<f64>,
    /// Display form: "1/250 s" or "2.5 s"
    pub shutter: Option<String>,
    pub iso: Option<u32>,
    /// Millimetres
    pub focal_length: Option<f64>,
    /// EV
    pub exposure_bias: Option<f64>,
    /// "YYYY-MM-DDTHH:MM:SS", plus the offset when the file records one
    pub captured: Option<String>,
    pub software: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub orientation: Option<u16>,
    pub gps: Option<Gps>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gps {
    /// Decimal degrees, south and west negative.
    pub latitude: f64,
    pub longitude: f64,
    /// Metres; negative below sea level.
    pub altitude: Option<f64>,
    /// An OpenStreetMap link to the spot.
    pub map_url: String,
}

fn ratio(r: &uR64) -> Option<f64> {
    (r.denominator != 0).then(|| f64::from(r.nominator) / f64::from(r.denominator))
}

fn signed_ratio(r: &iR64) -> Option<f64> {
    (r.denominator != 0).then(|| f64::from(r.nominator) / f64::from(r.denominator))
}

fn first_ratio(values: &[uR64]) -> Option<f64> {
    values.first().and_then(ratio)
}

/// Degrees, minutes, seconds → decimal degrees.
fn dms(values: &[uR64]) -> Option<f64> {
    if values.len() < 3 {
        return None;
    }
    Some(ratio(&values[0])? + ratio(&values[1])? / 60.0 + ratio(&values[2])? / 3600.0)
}

fn clean(text: &str) -> Option<String> {
    let t = text.trim_matches(char::from(0)).trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// "1/250 s" for fast shutters, "2.5 s" for slow ones.
fn shutter_text(seconds: f64) -> String {
    if seconds > 0.0 && seconds < 0.5 {
        format!("1/{} s", (1.0 / seconds).round() as u64)
    } else {
        let s = format!("{seconds:.1}");
        format!("{} s", s.trim_end_matches(".0"))
    }
}

/// EXIF "2026:10:01 21:14:05" → "2026-10-01T21:14:05" (+ offset).
fn iso_datetime(exif: &str, offset: Option<&str>) -> Option<String> {
    let exif = clean(exif)?;
    let (date, time) = exif.split_once(' ')?;
    let date = date.replace(':', "-");
    let mut out = format!("{date}T{time}");
    if let Some(off) = offset.and_then(clean) {
        out.push_str(&off);
    }
    Some(out)
}

pub fn summarize(metadata: &Metadata) -> Summary {
    let mut s = Summary::default();
    let (mut lat, mut lon, mut lat_ref, mut lon_ref) = (None, None, None, None);
    let (mut altitude, mut below_sea) = (None, false);
    let (mut original, mut offset) = (None, None);

    for tag in metadata {
        match tag {
            ExifTag::Make(v) => s.make = clean(v),
            ExifTag::Model(v) => s.model = clean(v),
            ExifTag::LensModel(v) => s.lens = clean(v),
            ExifTag::FNumber(v) => s.aperture = first_ratio(v),
            ExifTag::ExposureTime(v) => s.exposure_seconds = first_ratio(v),
            ExifTag::ISO(v) => s.iso = v.first().map(|&i| u32::from(i)),
            ExifTag::FocalLength(v) => s.focal_length = first_ratio(v),
            ExifTag::ExposureCompensation(v) => s.exposure_bias = v.first().and_then(signed_ratio),
            ExifTag::DateTimeOriginal(v) => original = Some(v.clone()),
            ExifTag::OffsetTimeOriginal(v) => offset = Some(v.clone()),
            ExifTag::Software(v) => s.software = clean(v),
            ExifTag::Artist(v) => s.artist = clean(v),
            ExifTag::Copyright(v) => s.copyright = clean(v),
            ExifTag::Orientation(v) => s.orientation = v.first().copied(),
            ExifTag::GPSLatitude(v) => lat = dms(v),
            ExifTag::GPSLongitude(v) => lon = dms(v),
            ExifTag::GPSLatitudeRef(v) => lat_ref = clean(v),
            ExifTag::GPSLongitudeRef(v) => lon_ref = clean(v),
            ExifTag::GPSAltitude(v) => altitude = first_ratio(v),
            ExifTag::GPSAltitudeRef(v) => below_sea = v.first() == Some(&1),
            _ => {}
        }
    }

    s.camera = match (&s.make, &s.model) {
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => {
            Some(model.clone())
        }
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (None, Some(model)) => Some(model.clone()),
        (Some(make), None) => Some(make.clone()),
        (None, None) => None,
    };
    s.shutter = s.exposure_seconds.map(shutter_text);
    s.captured = original
        .as_deref()
        .and_then(|o| iso_datetime(o, offset.as_deref()));

    if let (Some(mut latitude), Some(mut longitude)) = (lat, lon) {
        if lat_ref.as_deref() == Some("S") {
            latitude = -latitude;
        }
        if lon_ref.as_deref() == Some("W") {
            longitude = -longitude;
        }
        s.gps = Some(Gps {
            latitude,
            longitude,
            altitude: altitude.map(|a| if below_sea { -a } else { a }),
            map_url: format!(
                "https://www.openstreetmap.org/?mlat={latitude:.6}&mlon={longitude:.6}#map=16/{latitude:.6}/{longitude:.6}"
            ),
        });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u32, d: u32) -> uR64 {
        uR64 {
            nominator: n,
            denominator: d,
        }
    }

    #[test]
    fn summary_decodes_camera_exposure_and_gps() {
        let mut m = Metadata::new();
        m.set_tag(ExifTag::Make("FUJIFILM".into()));
        m.set_tag(ExifTag::Model("X-T5".into()));
        m.set_tag(ExifTag::LensModel("XF23mmF1.4 R LM WR".into()));
        m.set_tag(ExifTag::FNumber(vec![r(14, 5)]));
        m.set_tag(ExifTag::ExposureTime(vec![r(1, 250)]));
        m.set_tag(ExifTag::ISO(vec![400]));
        m.set_tag(ExifTag::FocalLength(vec![r(23, 1)]));
        m.set_tag(ExifTag::DateTimeOriginal("2026:10:01 21:14:05".into()));
        m.set_tag(ExifTag::OffsetTimeOriginal("-07:00".into()));
        m.set_tag(ExifTag::GPSLatitudeRef("N".into()));
        m.set_tag(ExifTag::GPSLatitude(vec![r(45, 1), r(30, 1), r(5472, 100)]));
        m.set_tag(ExifTag::GPSLongitudeRef("W".into()));
        m.set_tag(ExifTag::GPSLongitude(vec![
            r(122, 1),
            r(40, 1),
            r(4224, 100),
        ]));

        let s = summarize(&m);
        assert_eq!(s.camera.as_deref(), Some("FUJIFILM X-T5"));
        assert_eq!(s.lens.as_deref(), Some("XF23mmF1.4 R LM WR"));
        assert_eq!(s.aperture, Some(2.8));
        assert_eq!(s.shutter.as_deref(), Some("1/250 s"));
        assert_eq!(s.iso, Some(400));
        assert_eq!(s.focal_length, Some(23.0));
        assert_eq!(s.captured.as_deref(), Some("2026-10-01T21:14:05-07:00"));
        let gps = s.gps.expect("gps");
        assert!((gps.latitude - 45.5152).abs() < 1e-4);
        assert!((gps.longitude + 122.6784).abs() < 1e-4);
        assert!(gps.map_url.contains("mlat=45.5152"));
    }

    #[test]
    fn camera_name_does_not_repeat_the_make() {
        let mut m = Metadata::new();
        m.set_tag(ExifTag::Make("Canon".into()));
        m.set_tag(ExifTag::Model("Canon EOS R6".into()));
        assert_eq!(summarize(&m).camera.as_deref(), Some("Canon EOS R6"));
    }

    #[test]
    fn empty_metadata_is_an_empty_summary() {
        assert_eq!(summarize(&Metadata::new()), Summary::default());
    }

    #[test]
    fn slow_shutters_read_as_seconds() {
        assert_eq!(shutter_text(2.5), "2.5 s");
        assert_eq!(shutter_text(1.0), "1 s");
        assert_eq!(shutter_text(1.0 / 60.0), "1/60 s");
    }
}
