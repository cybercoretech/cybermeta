<p align="center">
  <img src="assets/cybermeta-brand/cybermeta-hero.svg" alt="cybermeta — TUI-first EXIF workstation" width="820">
</p>

<p align="center">
  <a href="https://cybercore-tech.github.io/cybermeta/">Site →</a>
</p>

[![CI](https://github.com/cybercore-tech/cybermeta/actions/workflows/ci.yml/badge.svg)](https://github.com/cybercore-tech/cybermeta/actions/workflows/ci.yml)
[![Release](https://github.com/cybercore-tech/cybermeta/actions/workflows/release.yml/badge.svg)](https://github.com/cybercore-tech/cybermeta/actions/workflows/release.yml)

# cybermeta

TUI-first EXIF workstation for the **Cybercore Systems Framework**.

Read, edit, strip, and spoof image metadata without re-encoding pixels. Interactive UI uses the embedded CYBERGRID palette from [`cybercore`](https://github.com/cybercore-tech/cybercore) (`~/.sysops/cybercore`). CLI subcommands stay machine-parsable for scripts and pipes.

## 📦 Install

```bash
curl -fsSL https://raw.githubusercontent.com/cybercore-tech/cybermeta/main/install.sh | sh
```

Downloads the latest release for your platform (Linux or macOS, x86_64
or aarch64), verifies its SHA-256 checksum, and installs `cybermeta`
to `~/.local/bin`. Or build from source with `cargo build --release`.

## 🧰 Usage

```text
cybermeta                      # TUI (path prompt)
cybermeta <file>               # TUI on file
cybermeta show <file> [--format json|tsv]
cybermeta show <file> --summary            # typed JSON: camera, exposure, time, GPS
cybermeta xmp write [-o out.xmp|-] [--title T] [--description D] [--keyword K]...
                    [--creator-tool APP] [--field NAME=VALUE]... [--recipe JSON|@file]
cybermeta xmp read <file.xmp|image>        # sidecar or embedded XMP → JSON
cybermeta export <file> [-f json|tsv]
cybermeta strip <file> --gps|--all [-o out] [--in-place]
cybermeta set <file> TAG=VALUE [-o out] [--in-place]
cybermeta spoof <file> --preset <name> [-o out] [--in-place]
cybermeta spoof --list
```

Writes default to `<stem>.cybermeta.<ext>` next to the source. Pass `--in-place` only when you intend to overwrite.

### ✍️ Editable string tags (`set` / TUI Enter)

`Make`, `Model`, `Software`, `Artist`, `Copyright`, `ImageDescription`, `ModifyDate` / `DateTime`, `DateTimeOriginal`, `CreateDate`, `LensMake`, `LensModel`, GPS latitude/longitude refs.

### 🕶️ Presets

| Preset | Effect |
|---|---|
| `strip-gps` | Remove GPS IFD / geolocation tags |
| `anonymous-camera` | Strip device identity; set generic Make/Model/Software |
| `clear-all` | Keep structural TIFF tags only |

## 📊 Summary JSON

`show --summary` decodes the tags apps usually want. It turns rationals into numbers and DMS GPS into decimal degrees with a map link. Every field may be `null`.

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

Also included: `make`, `model`, `exposure_bias`, `software`, `artist`, `copyright` and `orientation`.

## 🧬 XMP sidecars and packets

`xmp write` produces a standard XMP packet. Use it as a `.xmp` sidecar, or embed it in an image (the `<?xpacket?>` wrapper is included). It contains:

- **Dublin Core:** `dc:title`, `dc:description` and `dc:subject` keywords, which Lightroom, digiKam and darktable read.
- **XMP basic:** `xmp:CreatorTool` and `xmp:MetadataDate`.
- **`cybermeta:`** (`https://cybercore.tech/ns/cybermeta/1.0/`): `version`, any `--field NAME=VALUE` (letters, digits, `_`), and `recipe`, an app's edit recipe as JSON. Ghostframe stores its source identity and non-destructive edit here.

```sh
cybermeta xmp write -o photo.jpg.xmp --title "Harrow Point" --description "Late returns" \
  --keyword neon --keyword "night shoot" --creator-tool Ghostframe \
  --field sourceHash=1a2b3c --recipe '{"version":1,"ops":[{"op":"exposure","ev":0.5}]}'
cybermeta xmp read photo.jpg.xmp      # → {title, description, keywords, creator_tool, metadata_date, fields, recipe}
cybermeta xmp read exported.webp      # also finds the packet embedded in JPEG / PNG / WebP
cybermeta xmp read exported.png       # PNG iTXt / zTXt / tEXt, compressed or not (libvips writes zTXt)
```

Reading is namespace-aware, so packets from other tools work whatever prefixes they use. File output is atomic (written beside the target, then renamed).

## ⌨️ TUI keys

| Key | Action |
|---|---|
| `j` / `k` | Move |
| `/` | Filter |
| `Enter` | Edit selected string tag |
| `g` | Strip GPS |
| `X` | Strip all (confirm) |
| `p` | Spoof / scrub presets |
| `s` | Save to `*.cybermeta.*` |
| `S` | Save in-place |
| `e` | Export JSON sidecar |
| `o` | Open another path |
| `q` | Quit |

Theme follows `CYBERGRID_THEME` / cybercore’s active theme.

## 🖼️ Formats

JPEG / TIFF first-class. PNG, WebP, JXL, HEIF/AVIF where `little_exif` can write metadata without touching pixels.

### ⚠️ Known limitations

- **WebP:** `little_exif` rejects WebP EXIF chunks that begin with an `Exif\0\0` header ("Illegal endian information"), which exiftool and some cameras write. Rewriting a WebP's EXIF also drops an existing XMP chunk. For WebP output, clean the metadata before encoding; Ghostframe does this by running cybermeta on a JPEG carrier of the EXIF. `xmp read` works on WebP regardless.

## 📜 License

MIT — Copyright (c) 2026 Cybercore Tech (subgridsec.org)

Independent open-source utility under the Cyber prefix; no affiliation with any external cybersecurity vendor or agency.

Contact: cybercore.sh+cybermeta@gmail.com · [subgridsec.org](https://subgridsec.org)
