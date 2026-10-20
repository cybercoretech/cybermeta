<p align="center">
  <img src="assets/cybermeta-brand/cybermeta-hero.svg" alt="cybermeta — find and remove what your photos give away" width="820">
</p>

<p align="center">
  <a href="https://cybercoretech.github.io/cybermeta/">Site →</a> ·
  <a href="https://github.com/cybercoretech/cybermeta-android-app">Cybermeta for Android →</a>
</p>

[![CI](https://github.com/cybercoretech/cybermeta/actions/workflows/ci.yml/badge.svg)](https://github.com/cybercoretech/cybermeta/actions/workflows/ci.yml)
[![Release](https://github.com/cybercoretech/cybermeta/actions/workflows/release.yml/badge.svg)](https://github.com/cybercoretech/cybermeta/actions/workflows/release.yml)

# cybermeta

Find and remove what your photos give away, without re-encoding a single
pixel. A TUI and CLI for the **Cybercore Systems Framework**.

A photo carries far more than the picture. It can hold where you stood,
which camera body took it, your name, when it was taken, the editing apps
you used, a thumbnail of the photo before you cropped it, and, on modern
phones, a whole video clip. `cybermeta audit` shows you all of it, and
`cybermeta scrub` removes it and then checks its own work.

## 🔦 What's new in 0.3.0

- **🧽 A scrubber you can trust.** In 0.2, `strip --all` only cleared the
  EXIF block, so **XMP GPS, IPTC names and places, JPEG comments, and data
  hidden after the image all survived**. 0.3 rebuilds the file from its
  picture data instead, and fails rather than writing a file that still
  leaks.
- **🕵️ `audit`:** a privacy report with a risk level for every photo or
  folder.
- **📂 Batch scrubbing** of files and folders, plus a TUI folder mode.
- **🧪 Self-checking:** every scrub is re-read by a second, independent
  parser (nom-exif) before it's written.
- **🎚️ Keep options**, two new presets (`share-safe` and `--shift` date
  shifting), and a before/after report in the TUI.
- **WebP** EXIF and XMP now come out cleanly. This was a known limitation
  in 0.2.

## 📦 Install

```bash
curl -fsSL https://raw.githubusercontent.com/cybercoretech/cybermeta/main/install.sh | sh
```

This downloads the latest release for your platform (Linux or macOS,
x86_64 or aarch64), verifies its SHA-256 checksum, and installs
`cybermeta` to `~/.local/bin`. You can also build from source with
`cargo build --release`.

On a phone, use **[Cybermeta for Android](https://github.com/cybercoretech/cybermeta-android-app)**.
It runs this same engine, and you can share photos to it from your gallery.

## 🧰 Usage

```text
cybermeta                          # TUI (path prompt)
cybermeta <file|folder>            # TUI on a photo, or folder mode
cybermeta audit <paths>... [-r] [--format text|json]
cybermeta scrub <paths>... [-r] [--keep icc,orientation,date|none]
                [-o out] [--in-place] [--dry-run] [--format text|json]
cybermeta strip <file> --gps|--all [--keep ...] [-o out] [--in-place]
cybermeta spoof <file> --preset <name> | --shift <offset> [-o out] [--in-place]
cybermeta spoof --list
cybermeta show <file> [--format json|tsv] [--summary]
cybermeta export <file> [-f json|tsv]
cybermeta set <file> TAG=VALUE [-o out] [--in-place]
cybermeta xmp write [-o out.xmp|-] [--title T] [--description D] [--keyword K]...
                    [--creator-tool APP] [--field NAME=VALUE]... [--recipe JSON|@file]
cybermeta xmp read <file.xmp|image>
```

Writes default to `<stem>.cybermeta.<ext>` next to the source. Pass
`--in-place` only when you intend to overwrite. Every write goes to a
temporary file first and is then renamed into place, so a crash never
leaves half a photo.

## 🕵️ Audit

```text
$ cybermeta audit IMG_2041.jpg
IMG_2041.jpg  [HIGH] JPEG
  high   device     EXIF         Camera serial number: ABC123
  high   location   EXIF         GPS position: 45.500000, -122.600000
  high   person     IPTC         By-line (creator): Raven Test
  high   location   IPTC         City: Portland
  high   person     XMP          creator: Raven Test
  high   location   XMP          GPSLatitude: 45,30.0N
  medium text       JPEG comment Comment: home address
  low    device     EXIF         Camera make: Fuji
  map: https://www.openstreetmap.org/?mlat=45.500000&mlon=-122.600000#map=16/45.500000/-122.600000
```

| Category | What it catches |
|---|---|
| 📍 **location** | EXIF GPS; XMP `GPS*`, `City`, `Location*`; IPTC city, sublocation, state and country |
| 🧑 **person** | Artist, camera owner, XMP creator / author / contact info, IPTC by-line, copyright, PNG `Author` |
| 📷 **device** | Body and lens serial numbers, unique image ID, maker notes, make/model, host computer, XMP document IDs |
| 🕰️ **time** | Taken / digitised / modified, time-zone offsets, IPTC dates, PNG `tIME` |
| 🖼️ **thumbnail** | EXIF IFD1 and JFIF/JFXX previews, which can show the photo *before* you cropped it |
| 🎞️ **hidden data** | Bytes after the end of the image: Samsung/Pixel motion-photo video, MPF depth and HDR maps |
| ✍️ **text / software** | Captions, comments, keywords, editing software and history |

Use `--format json` to script it. Folders work too, and `-r` walks them
all the way down.

## 🧽 Scrub

```bash
cybermeta scrub IMG_2041.jpg                    # → IMG_2041.cybermeta.jpg
cybermeta scrub ~/Pictures/to-share -r          # every photo, all the way down
cybermeta scrub photo.jpg --keep none           # pixels only
cybermeta scrub photo.jpg --keep icc,orientation,date
cybermeta scrub photo.jpg --dry-run             # show what would go
```

How it works:

1. The file is split into its blocks: picture data, EXIF, XMP, IPTC, the
   ICC profile, comments, thumbnails, vendor blocks and trailers.
2. It's rebuilt from the **picture data** alone. The compressed image is
   copied **byte for byte**; nothing is decoded or re-encoded.
3. What you chose to keep goes back in. **Rotation and date** are written
   as a **brand-new EXIF block containing only those values**, never as a
   copy of the old one.
4. The result is audited again by a different parser. If anything but what
   you kept is still there, cybermeta **refuses to write** and tells you
   what it found.

| `--keep` | Default | Why you'd keep it |
|---|---|---|
| `icc` | ✔ | Phones shoot in Display P3. Without the profile, colours look washed out. It says nothing about you. |
| `orientation` | ✔ | Without it, portrait photos can display sideways. |
| `date` | ✘ | Only when the capture time doesn't matter to you. |

The **thumbnail** and **data after the image** always go. For a motion
photo, the still photo stays and the video part is removed.

| Format | Scrub |
|---|---|
| JPEG | ✔ all APPn segments, comments, JFIF/JFXX thumbnails, trailers. Adobe APP14 is kept because it holds colour-transform data, not identity. |
| PNG | ✔ `tEXt`/`zTXt`/`iTXt`, `eXIf`, `tIME`, private chunks. CRCs are recomputed for anything written. |
| WebP | ✔ `EXIF`, `XMP `, unknown chunks. `VP8X` flags and the RIFF size are fixed up. |
| HEIC / AVIF / TIFF / JXL | ✘ refused with an explanation. `audit` still reads their EXIF. On Samsung, turn off *Settings → Camera → Advanced picture options → High efficiency pictures* to shoot JPEG. |

## 🕶️ Presets

| Preset | Effect |
|---|---|
| `share-safe` | Full scrub; keep colour profile and rotation (the defaults) |
| `strip-gps` | Location only: EXIF GPS, plus any XMP or IPTC block that names a place. Everything else stays. |
| `anonymous-camera` | Full scrub (keeps colour, rotation and date), then *Generic Camera* as the make and model |
| `clear-all` | Pixels only: no metadata, no colour profile, no rotation |
| `--shift +3d` | Moves every EXIF timestamp, drops the GPS clock, and drops XMP and IPTC, which carry their own dates |

`strip --gps` is `strip-gps`, and `strip --all` is a full scrub.

## ⌨️ TUI keys

| Key | Action |
|---|---|
| `X` | Scrub everything: a preview of what goes, then `y` to apply |
| `g` | Remove location |
| `p` | Presets |
| `t` | Shift every timestamp |
| `1` `2` `3` | Toggle keeping colour profile / rotation / date |
| `r` | Show the last before/after report again |
| `Enter` | Edit the selected string tag |
| `/` | Filter tags |
| `s` / `S` | Save to `*.cybermeta.*` / save in place |
| `e` | Export the tags as a JSON sidecar |
| `o` | Open another photo or folder |
| `b` | Back to the folder (folder mode) |
| `q` | Quit |

**Folder mode** (`cybermeta ~/Pictures/to-share`) lists every photo with its
risk badge. `Enter` opens one, `A` scrubs them all to `*.cybermeta.*`
copies, and `R` rescans. The theme follows `CYBERGRID_THEME` / cybercore's
active theme.

## ✍️ Editable string tags (`set` / TUI Enter)

`Make`, `Model`, `Software`, `Artist`, `Copyright`, `ImageDescription`,
`ModifyDate` / `DateTime`, `DateTimeOriginal`, `CreateDate`, `LensMake`,
`LensModel`, and the GPS latitude/longitude refs. Tag edits go through
`little_exif`, which only ever touches the EXIF block.

## 📊 Summary JSON

`show --summary` decodes the tags apps usually want. It turns rationals
into numbers and DMS GPS into decimal degrees with a map link. Every field
may be `null`.

```json
{
  "camera": "Fujifilm X-T5",
  "lens": "XF23mmF1.4 R LM WR",
  "aperture": 2.8,
  "shutter": "1/250 s",
  "exposure_seconds": 0.004,
  "iso": 400,
  "focal_length": 23.0,
  "captured": "2026-10-01T21:14:05",
  "gps": {
    "latitude": 45.5152,
    "longitude": -122.6784,
    "altitude": null,
    "map_url": "https://www.openstreetmap.org/?mlat=45.515200&mlon=-122.678400#map=16/45.515200/-122.678400"
  }
}
```

Also included: `make`, `model`, `exposure_bias`, `software`, `artist`,
`copyright` and `orientation`.

## 🧬 XMP sidecars and packets

`xmp write` produces a standard XMP packet. Use it as a `.xmp` sidecar, or
embed it in an image (the `<?xpacket?>` wrapper is included). It contains:

- **Dublin Core:** `dc:title`, `dc:description` and `dc:subject` keywords,
  which Lightroom, digiKam and darktable read.
- **XMP basic:** `xmp:CreatorTool` and `xmp:MetadataDate`.
- **`cybermeta:`** (`https://cybercore.tech/ns/cybermeta/1.0/`):
  `version`, any `--field NAME=VALUE` (letters, digits, `_`), and `recipe`,
  an app's edit recipe as JSON. Ghostframe stores its source identity and
  non-destructive edit here.

```sh
cybermeta xmp write -o photo.jpg.xmp --title "Harrow Point" --description "Late returns" \
  --keyword neon --keyword "night shoot" --creator-tool Ghostframe \
  --field sourceHash=1a2b3c --recipe '{"version":1,"ops":[{"op":"exposure","ev":0.5}]}'
cybermeta xmp read photo.jpg.xmp      # → {title, description, keywords, creator_tool, metadata_date, fields, recipe}
cybermeta xmp read exported.webp      # also finds the packet embedded in JPEG / PNG / WebP
cybermeta xmp read exported.png       # PNG iTXt / zTXt / tEXt, compressed or not (libvips writes zTXt)
```

Reading is namespace-aware, so packets from other tools work whatever
prefixes they use.

## 🧩 As a library

The engine builds without the TUI and CLI. The Android app uses it this
way:

```toml
cybermeta = { git = "https://github.com/cybercoretech/cybermeta", tag = "v0.3.0", default-features = false }
```

```rust
use cybermeta::meta::{audit::audit, scrub::{scrub, Keep}};
let report = audit(&bytes);                    // findings, risk, GPS, blocks
let clean = scrub(&bytes, &Keep::default())?;  // bytes + what was removed + before/after audits
```

Every operation takes and returns bytes, so the same code works on files,
share-sheet streams and content URIs.

## 🧪 Development

```bash
cargo test                       # 30 unit tests + 3 end-to-end tests
cargo clippy --all-targets -- -D warnings
```

The end-to-end tests in `tests/exiftool.rs` have exiftool write every leak
it can into real JPEG, PNG and WebP files. They check that cybermeta's
output contains nothing exiftool can find, and that ImageMagick still
decodes it. They skip themselves if exiftool or ImageMagick is missing;
CI installs both.

## ⚠️ Known limitations

- **HEIC/AVIF/TIFF/JXL** can be audited but not scrubbed yet; see the
  formats table above.
- **Tag edits on WebP** (`set`, `anonymous-camera`) go through
  `little_exif`, which rejects WebP EXIF that starts with an `Exif\0\0`
  header. Scrubbing WebP is not affected.
- `strip --gps` keeps a motion photo's video, which can carry its own
  location. cybermeta says so when it happens; use a full scrub to remove
  the video.

## 📜 License

MIT — Copyright (c) 2026 Cybercore Tech (subgridsec.org)

Independent open-source utility under the Cyber prefix; no affiliation with any external cybersecurity vendor or agency.

Contact: cybercore.sh+cybermeta@gmail.com · [subgridsec.org](https://subgridsec.org)
