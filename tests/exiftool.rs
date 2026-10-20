//! End-to-end: exiftool writes every kind of metadata it knows into real
//! JPEG / PNG / WebP files, the cybermeta binary scrubs them, and exiftool
//! (a third reader, sharing no code with cybermeta) must find nothing left
//! but what was asked to stay. ImageMagick checks the pixels still decode.
//!
//! Skipped when exiftool or ImageMagick isn't installed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn have(tool: &str) -> bool {
    Command::new(tool).arg("-ver").output().is_ok()
        || Command::new(tool).arg("-version").output().is_ok()
}

/// ImageMagick 7 is `magick`; 6 (Ubuntu's) is `convert` / `identify`.
fn im(sub: &str) -> Command {
    if have("magick") {
        let mut c = Command::new("magick");
        if sub == "identify" {
            c.arg("identify");
        }
        c
    } else {
        Command::new(if sub == "identify" {
            "identify"
        } else {
            "convert"
        })
    }
}

/// Skip locally when the tools are missing, but never silently on CI.
fn tools_present() -> bool {
    let ok = have("exiftool") && (have("magick") || have("convert"));
    if !ok {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must install exiftool and ImageMagick"
        );
        eprintln!("skipped: needs exiftool and ImageMagick");
    }
    ok
}

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cybermeta-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "{cmd:?} failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every tag exiftool reports, minus file-system and composite noise.
fn tags(path: &Path) -> Vec<String> {
    run(Command::new("exiftool")
        .args(["-a", "-G1", "-s", "-n"])
        .arg(path))
    .lines()
    .filter(|l| {
        ![
            "[System]",
            "[File]",
            "[ExifTool]",
            "[Composite]",
            "[PNG]",
            "[RIFF]",
            "[JFIF]",
            "[ICC",
            "[VP8",
        ]
        .iter()
        .any(|p| l.starts_with(p))
    })
    .map(str::to_string)
    .collect()
}

fn make_leaky(dir: &Path, ext: &str) -> PathBuf {
    let path = dir.join(format!("leaky.{ext}"));
    run(im("convert")
        .args(["-size", "32x24", "gradient:red-blue"])
        .arg(&path));
    let mut cmd = Command::new("exiftool");
    cmd.args([
        "-q",
        "-overwrite_original",
        "-GPSLatitude=45.5",
        "-GPSLatitudeRef=N",
        "-GPSLongitude=122.6",
        "-GPSLongitudeRef=W",
        "-Make=Fujifilm",
        "-Model=X-T5",
        "-SerialNumber=ABC123",
        "-OwnerName=Raven Test",
        "-Artist=Raven Test",
        "-Orientation#=6",
        "-DateTimeOriginal=2026:10:01 21:14:05",
        "-XMP:GPSLatitude=45.5",
        "-XMP:GPSLongitude=-122.6",
        "-XMP:Creator=Raven Test",
        "-XMP:City=Portland",
    ]);
    if ext == "jpg" {
        cmd.args([
            "-IPTC:By-line=Raven Test",
            "-IPTC:City=Portland",
            "-Comment=home address",
        ]);
    }
    if ext == "png" {
        cmd.args(["-PNG:Comment=home address", "-PNG:Author=Raven Test"]);
    }
    run(cmd.arg(&path));
    if ext == "jpg" {
        // A motion-photo style trailer, as Samsung and Pixel append.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\0\0\0\x18ftypmp42\0\0\0\0MotionPhoto_Data");
        std::fs::write(&path, bytes).unwrap();
    }
    let before = tags(&path).join("\n");
    for leak in ["45.5", "ABC123", "Raven Test", "Portland"] {
        assert!(
            before.contains(leak),
            "fixture is missing {leak}:\n{before}"
        );
    }
    path
}

#[test]
fn scrub_leaves_nothing_exiftool_can_find() {
    if !tools_present() {
        return;
    }
    let bin = env!("CARGO_BIN_EXE_cybermeta");
    for ext in ["jpg", "png", "webp"] {
        let dir = tmpdir(ext);
        let src = make_leaky(&dir, ext);
        let out = dir.join(format!("clean.{ext}"));
        run(Command::new(bin).arg("scrub").arg(&src).arg("-o").arg(&out));
        let left = tags(&out);
        // Rotation is the only thing allowed to remain by default.
        let unexpected: Vec<_> = left
            .iter()
            .filter(|l| !l.contains("Orientation") && !l.contains("ExifByteOrder"))
            .collect();
        assert!(unexpected.is_empty(), "{ext}: left behind {unexpected:#?}");
        assert!(
            left.iter()
                .any(|l| l.contains("Orientation") && l.ends_with(" 6")),
            "{ext}: {left:?}"
        );
        // Still a valid picture with the same size.
        let size = run(im("identify").args(["-format", "%wx%h"]).arg(&out));
        assert_eq!(size, "32x24", "{ext}");

        // --keep none: nothing at all.
        let bare = dir.join(format!("bare.{ext}"));
        run(Command::new(bin)
            .arg("scrub")
            .arg(&src)
            .args(["--keep", "none", "-o"])
            .arg(&bare));
        assert!(tags(&bare).is_empty(), "{ext}: {:#?}", tags(&bare));

        // The old entry point now does the same thing.
        let via_strip = dir.join(format!("strip.{ext}"));
        run(Command::new(bin)
            .arg("strip")
            .arg(&src)
            .args(["--all", "-o"])
            .arg(&via_strip));
        assert_eq!(
            std::fs::read(&via_strip).unwrap(),
            std::fs::read(&out).unwrap(),
            "{ext}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn strip_gps_removes_location_from_exif_and_xmp() {
    if !tools_present() {
        return;
    }
    let bin = env!("CARGO_BIN_EXE_cybermeta");
    let dir = tmpdir("gps");
    let src = make_leaky(&dir, "jpg");
    let out = dir.join("nogps.jpg");
    run(Command::new(bin)
        .arg("strip")
        .arg(&src)
        .args(["--gps", "-o"])
        .arg(&out));
    let left = tags(&out).join("\n");
    for gone in ["GPS", "45.5", "Portland"] {
        assert!(!left.contains(gone), "{gone} survived:\n{left}");
    }
    // Location only: the serial number is meant to stay.
    assert!(left.contains("ABC123"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_json_reports_the_leaks() {
    if !tools_present() {
        return;
    }
    let bin = env!("CARGO_BIN_EXE_cybermeta");
    let dir = tmpdir("audit");
    let src = make_leaky(&dir, "jpg");
    let json = run(Command::new(bin)
        .arg("audit")
        .arg(&src)
        .args(["--format", "json"]));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let a = &v[0]["audit"];
    assert_eq!(a["risk"], "high");
    let labels: Vec<&str> = a["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    for want in [
        "GPS position",
        "Camera serial number",
        "Camera owner",
        "City",
        "Comment",
    ] {
        assert!(labels.contains(&want), "missing {want} in {labels:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
