# 🕶️ cybermeta Agent Handoff

The state of the project for the next agent (or person) picking it up.
Update it in the same PR as any significant change.

## 📍 Repository state (2026-10-23)

- **Repo:** `cybercoretech/cybermeta` (org), with the local checkout at
  `~/Devspace/Cybercore/tooling/cybermeta` on Blackbox. The remote
  `cybercore-tech` is the old, suspended account; `origin` points at the org.
- **`main`:** v0.3.0 merged as PR #1 and tagged `v0.3.0`.
- **Binary release:** the `v*` GitHub Actions workflow builds Linux and macOS
  binaries and attaches them to a GitHub Release.
- **crates.io:** `cybermeta` v0.3.0 was published on 2026-10-23 under
  `ghost_443014`. The established registry release process is manual:
  dry-run, then `cargo publish --locked`. The earlier decision to defer
  publication has been superseded by the user's approval.
- **v0.3.1 update:** adds the crates.io README card and cross-project links,
  and fills in package metadata.
- **Companion:** `cybercoretech/cybermeta-android-app`
  (`~/Devspace/Android/Cybermeta`) uses this crate with
  `default-features = false` through UniFFI.

## ✅ Strict resume checklist

1. Read this file and `README.md`.
2. `git status`, `git log --oneline -5`, and open PRs:
   ```sh
   GH_TOKEN=$(gh auth token -u darkstardevx) gh pr list -R cybercoretech/cybermeta
   ```
3. Build and test before changing anything:
   ```sh
   cargo test                                   # 30 unit + 3 end-to-end (needs exiftool + ImageMagick)
   cargo clippy --all-targets -- -D warnings
   cargo build --lib --no-default-features      # the Android build path
   ```
4. Work on a branch; one PR per change; never stack PRs. The user merges.
   Pace GitHub activity: no bursts, no force-push storms.
5. Commit identity: `git config user.email` must be
   `19244308+darkstardevx@users.noreply.github.com` (it comes from the
   `includeIf` for `cybercoretech/**`). Check before every commit.
6. Update the docs with every change:
   - this file and `README.md`
   - the Wiki page `~/Wiki/src/tools/cybermeta.md` (run `mdbook build`)
   - the vault note `~/Vaults/blackbox-notes/Projects/Cybermeta.md`
7. After a merge, install the build: `install -m755 ~/.cargo-target/release/cybermeta ~/.local/bin/`.
   Ghostframe calls it from `PATH`.

## 🧭 Decisions already made

- **Engine (user, 2026-10-19):** our own container scrubber, plus a second,
  independent reader to check it.
  - **nom-exif** reads EXIF, including from HEIC.
  - **little_exif** stays only for tag edits (`set`, `anonymous-camera`
    make/model, date shift).
  - Rejected: `img-parts` (doesn't handle trailers), `rexiv2`/exiv2 (C++,
    painful for the Android NDK), and an exiftool shell-out (unusable on
    Android).
- **Defaults:** keep the ICC profile and orientation; drop the date. The
  thumbnail and data after the image always go.
- **Orientation and date are re-created, never copied:** `minimal_tiff()`
  writes a fresh big-endian TIFF holding only those values.
- **Location-only (`strip --gps`)** drops any XMP or IPTC block that names a
  place. Blocks it can't read are dropped too, so it fails safe.
- **HEIC/AVIF/TIFF/JXL** are refused with an explanation. Samsung:
  *Settings → Camera → Advanced picture options → High efficiency pictures*
  off.

## 🧱 Architecture

| Area | File | Notes |
|---|---|---|
| Container surgery | `src/meta/container.rs` | `detect`, `parse` → `Block`s, `rebuild(src, &Policy)`, `minimal_tiff`, PNG text inflate. Test fixtures live in `container::fixtures`. |
| Privacy audit | `src/meta/audit.rs` | nom-exif plus container blocks → `Finding { severity, category, source, label, value }`; the IPTC-IIM parser. |
| Operations | `src/meta/scrub.rs` | `scrub`, `strip_location`, `date_shift`, `anonymous_camera`, `Keep`, `Outcome`. Each one audits its own output. |
| Document | `src/meta/mod.rs` | `MetaDoc` holds the **bytes**; ops replace them; `save` writes atomically. `collect_images` for batches. |
| Presets | `src/meta/presets.rs` | share-safe, strip-gps, anonymous-camera, clear-all. |
| XMP | `src/meta/xmp.rs` | sidecars (`XmpDoc`), `properties()` flat walk, `has_location`. |
| CLI / TUI | `src/main.rs`, `src/tui/mod.rs` | behind the `cli` feature (default). |

## ⚠️ Invariants and gotchas

- **Picture bytes are copied, never re-encoded.** `scrub_keeps_the_picture_byte_for_byte`
  pins it: scrubbing the leaky fixture with `Keep::NOTHING` must equal
  `tests/fixtures/tiny.jpg` exactly.
- **Never return success with leftovers.**
  - `scrub` re-audits its output and checks the block kinds.
  - It also scans the raw bytes for `<x:xmpmeta`, `Photoshop 3.0` and the
    XMP namespace.
  - Keep all three checks.
- **The JPEG scan** runs from SOS to the next marker that isn't `FF00`,
  `RSTn` or `FF` fill. Bytes after EOI are the "trailer" block: motion
  photos and MPF images.
- **WebP:** after a rebuild, the VP8X flags (ICC 0x20, EXIF 0x08, XMP 0x04)
  and the RIFF size are recomputed. EXIF goes after the image data, as the
  bare TIFF (no `Exif\0\0`).
- **PNG:** `eXIf` is inserted right after `IHDR` (it must come before
  IDAT). Unknown ancillary chunks (lowercase first letter) are dropped;
  unknown critical chunks are kept.
- **little_exif** writes PNG EXIF as zTXt unless it's given
  `FileExtension::PNG { as_zTXt_chunk: false }`; `little_ext()` handles
  this.
- **The end-to-end tests** call `convert`/`identify` when `magick` is
  missing (Ubuntu ships IM6). On CI (`CI` set) they **fail** instead of
  skipping.
- **Ghostframe** uses `strip --gps|--all --in-place`,
  `spoof --preset anonymous-camera` and `set` on a tiny carrier JPEG. Its
  60 tests pass against 0.3.0. `anonymous-camera` now also drops exposure
  settings, which is stronger and intended.

## 🖥️ Machine context

- **Cargo target dir:** `~/.cargo-target` (shared). The release binary is
  at `~/.cargo-target/release/cybermeta`.
- **Installed:** `~/.local/bin/cybermeta` is the 0.2.0 branch build until
  0.3.0 merges.
- **TUI smoke test without a display:**
  ```sh
  tmux new -d -s t -x 160 -y 40 'cybermeta photo.jpg'
  tmux capture-pane -p -t t
  ```

## 📜 PR history

- #2, #4, #5: link moves to cybercore-tech.
- #9: v0.2.0, `show --summary` and `xmp write|read` (for Ghostframe Phase 3).
- #1: v0.3.0, lossless scrubber, privacy audit, batch mode, and Android library
  feature gate.
