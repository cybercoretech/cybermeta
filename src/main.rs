use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use cybermeta::meta::xmp::valid_field_name;
use cybermeta::meta::{
    collect_images, list_presets, write_atomic, Audit, ExportFormat, Keep, MetaDoc, Outcome,
    Preset, Severity, XmpDoc,
};
use cybermeta::tui;
use std::collections::BTreeMap;

#[derive(Debug, Parser)]
#[command(
    name = "cybermeta",
    about = "TUI-first EXIF workstation for the Cybercore Systems Framework",
    version
)]
struct Cli {
    /// Image path — opens the TUI when no subcommand is given
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// List tags (JSON by default)
    Show {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = OutFormat::Json)]
        format: OutFormat,
        /// Typed JSON summary instead: camera, lens, exposure, capture
        /// time and GPS (decimal degrees + map link)
        #[arg(long)]
        summary: bool,
    },
    /// Export tags to stdout
    Export {
        file: PathBuf,
        #[arg(short = 'f', long, value_enum, default_value_t = OutFormat::Json)]
        format: OutFormat,
    },
    /// Privacy report: what each photo gives away (GPS, serials, names,
    /// times, thumbnails, hidden data)
    Audit {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Walk directories all the way down
        #[arg(short = 'r', long)]
        recursive: bool,
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        format: ReportFormat,
    },
    /// Remove all metadata from photos and folders, losslessly, and check
    /// the result
    Scrub {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[arg(short = 'r', long)]
        recursive: bool,
        /// What may stay: icc, orientation, date (comma-separated) or none
        #[arg(long, default_value = "icc,orientation")]
        keep: String,
        /// Output file (single input only)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        #[arg(long)]
        in_place: bool,
        /// Report what would be removed; write nothing
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        format: ReportFormat,
    },
    /// Strip location (--gps) or everything (--all, same as scrub)
    Strip {
        file: PathBuf,
        #[arg(long, conflicts_with = "all")]
        gps: bool,
        #[arg(long)]
        all: bool,
        /// With --all: what may stay (icc, orientation, date, none)
        #[arg(long, default_value = "icc,orientation")]
        keep: String,
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        #[arg(long)]
        in_place: bool,
    },
    /// Set a string tag: TAG=VALUE
    Set {
        file: PathBuf,
        assignment: String,
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        #[arg(long)]
        in_place: bool,
    },
    /// Write or read XMP (sidecars or packets embedded in images)
    Xmp {
        #[command(subcommand)]
        action: XmpAction,
    },
    /// Apply a named scrub/spoof preset, or shift every timestamp
    Spoof {
        file: Option<PathBuf>,
        #[arg(long, required_unless_present_any = ["list", "shift"], conflicts_with = "shift")]
        preset: Option<String>,
        /// Move every EXIF timestamp, e.g. +3d, -2h30m (drops XMP/IPTC, which
        /// carry their own dates)
        #[arg(long, allow_hyphen_values = true)]
        shift: Option<String>,
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        #[arg(long)]
        in_place: bool,
        /// List available presets and exit
        #[arg(long)]
        list: bool,
    },
}

#[derive(Debug, Subcommand)]
enum XmpAction {
    /// Write an XMP packet: dc title/description/keywords plus cybermeta:
    /// fields and an edit recipe. Usable as a .xmp sidecar or embedded.
    Write {
        /// Output file, or - for stdout
        #[arg(short = 'o', long, default_value = "-")]
        output: String,
        #[arg(long)]
        title: Option<String>,
        /// dc:description (caption)
        #[arg(long)]
        description: Option<String>,
        /// Keyword (repeatable)
        #[arg(long = "keyword")]
        keywords: Vec<String>,
        #[arg(long)]
        creator_tool: Option<String>,
        /// cybermeta:NAME=VALUE (repeatable)
        #[arg(long = "field")]
        fields: Vec<String>,
        /// Edit recipe as JSON, or @file to read it from a file
        #[arg(long)]
        recipe: Option<String>,
    },
    /// Read a .xmp sidecar or an image's embedded XMP as JSON
    Read { file: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ReportFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutFormat {
    Json,
    Tsv,
}

impl From<OutFormat> for ExportFormat {
    fn from(value: OutFormat) -> Self {
        match value {
            OutFormat::Json => ExportFormat::Json,
            OutFormat::Tsv => ExportFormat::Tsv,
        }
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("cybermeta: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            let path = cli.file;
            if path.is_none() && !io::stdout().is_terminal() {
                bail!("no TTY and no subcommand — use audit/scrub/show/export/strip/set/spoof/xmp");
            }
            tui::run(path)
        }
        Some(Commands::Show {
            file,
            format,
            summary,
        }) => {
            let doc = MetaDoc::open(&file)?;
            if summary {
                println!("{}", serde_json::to_string_pretty(&doc.summary())?);
            } else {
                print!("{}", doc.export_string(format.into())?);
            }
            Ok(())
        }
        Some(Commands::Xmp { action }) => run_xmp(action),
        Some(Commands::Export { file, format }) => {
            let doc = MetaDoc::open(&file)?;
            print!("{}", doc.export_string(format.into())?);
            Ok(())
        }
        Some(Commands::Strip {
            file,
            gps,
            all,
            keep,
            output,
            in_place,
        }) => {
            if !gps && !all {
                bail!("specify --gps or --all");
            }
            let mut doc = MetaDoc::open(&file)?;
            let outcome = if all {
                doc.scrub(&Keep::parse(&keep)?)?
            } else {
                doc.strip_gps()?
            };
            let dest = doc.save(output.as_deref(), in_place)?;
            print_outcome(&dest, &outcome);
            Ok(())
        }
        Some(Commands::Audit {
            paths,
            recursive,
            format,
        }) => run_audit(&paths, recursive, format),
        Some(Commands::Scrub {
            paths,
            recursive,
            keep,
            output,
            in_place,
            dry_run,
            format,
        }) => run_scrub(
            &paths,
            recursive,
            &Keep::parse(&keep)?,
            output.as_deref(),
            in_place,
            dry_run,
            format,
        ),
        Some(Commands::Set {
            file,
            assignment,
            output,
            in_place,
        }) => {
            let mut doc = MetaDoc::open(&file)?;
            doc.set_tag_from_assignment(&assignment)?;
            let dest = doc.save(output.as_deref(), in_place)?;
            eprintln!("wrote {}", dest.display());
            Ok(())
        }
        Some(Commands::Spoof {
            file,
            preset,
            shift,
            output,
            in_place,
            list,
        }) => {
            if list {
                for p in list_presets() {
                    println!("{}\t{}", p.as_str(), p.description());
                }
                println!("--shift OFFSET\tMove every EXIF timestamp (e.g. +3d, -2h30m)");
                return Ok(());
            }
            let file = file.context("FILE is required unless --list")?;
            let mut doc =
                MetaDoc::open(&file).with_context(|| format!("open {}", file.display()))?;
            let (label, outcome) = if let Some(shift) = shift {
                let seconds = cybermeta::meta::scrub::parse_shift(&shift)?;
                ("date-shift".to_string(), doc.date_shift(seconds)?)
            } else {
                let preset_name = preset.context("--preset or --shift is required")?;
                let Some(preset) = Preset::parse(&preset_name) else {
                    bail!(
                        "unknown preset `{preset_name}` — try: {}",
                        list_presets()
                            .iter()
                            .map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                };
                (preset.as_str().to_string(), doc.apply_preset(preset)?)
            };
            let dest = doc.save(output.as_deref(), in_place)?;
            eprintln!("applied {label}");
            print_outcome(&dest, &outcome);
            Ok(())
        }
    }
}

fn print_outcome(dest: &std::path::Path, o: &Outcome) {
    let removed: usize = o.removed.iter().map(|r| r.bytes).sum();
    eprintln!(
        "wrote {} — cleared {} finding(s), removed {} block(s) ({})",
        dest.display(),
        o.cleared(),
        o.removed.len(),
        cybermeta::meta::audit::human_bytes(removed)
    );
    for r in &o.removed {
        eprintln!(
            "  - {} [{}] {}",
            r.kind_label(),
            r.label,
            cybermeta::meta::audit::human_bytes(r.bytes)
        );
    }
    for n in &o.notes {
        eprintln!("  note: {n}");
    }
    if !o.after.findings.is_empty() {
        eprintln!("  still in the file:");
        for f in &o.after.findings {
            eprintln!(
                "    {} {} {}: {}",
                f.severity.as_str(),
                f.source,
                f.label,
                f.value
            );
        }
    }
}

fn risk_text(risk: Option<Severity>) -> &'static str {
    match risk {
        None => "CLEAN",
        Some(Severity::Low) => "LOW",
        Some(Severity::Medium) => "MEDIUM",
        Some(Severity::High) => "HIGH",
    }
}

fn print_audit(path: &std::path::Path, a: &Audit) {
    println!(
        "{}  [{}] {}",
        path.display(),
        risk_text(a.risk),
        a.format.as_str()
    );
    if let Some(d) = &a.damaged {
        println!("  ! damaged container: {d}");
    }
    if !a.scrubbable {
        println!("  ! {} can't be scrubbed losslessly yet", a.format.as_str());
    }
    for f in &a.findings {
        println!(
            "  {:<6} {:<10} {:<12} {}: {}",
            f.severity.as_str(),
            f.category.as_str(),
            f.source,
            f.label,
            f.value
        );
    }
    if let Some(g) = &a.gps {
        println!("  map: {}", g.map_url);
    }
}

fn run_audit(paths: &[PathBuf], recursive: bool, format: ReportFormat) -> Result<()> {
    let files = collect_images(paths, recursive)?;
    let mut reports = Vec::new();
    for file in &files {
        let bytes = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
        let a = cybermeta::meta::audit::audit(&bytes);
        if format == ReportFormat::Text {
            print_audit(file, &a);
        }
        reports.push(serde_json::json!({ "path": file, "audit": a }));
    }
    if format == ReportFormat::Json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else if files.len() > 1 {
        let risky = reports
            .iter()
            .filter(|r| r["audit"]["risk"] == "high")
            .count();
        println!("\n{} file(s), {risky} high risk", files.len());
    }
    Ok(())
}

fn run_scrub(
    paths: &[PathBuf],
    recursive: bool,
    keep: &Keep,
    output: Option<&std::path::Path>,
    in_place: bool,
    dry_run: bool,
    format: ReportFormat,
) -> Result<()> {
    let files = collect_images(paths, recursive)?;
    if output.is_some() && files.len() != 1 {
        bail!("-o works with a single file; use --in-place or the default *.cybermeta.* names");
    }
    let (mut ok, mut failed) = (0, 0);
    let mut reports = Vec::new();
    for file in &files {
        let result = (|| -> Result<(PathBuf, Outcome)> {
            let bytes = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
            let outcome = cybermeta::meta::scrub::scrub(&bytes, keep)?;
            let dest = cybermeta::meta::resolve_output(file, output, in_place)?;
            if !dry_run {
                write_atomic(&dest, &outcome.bytes)?;
            }
            Ok((dest, outcome))
        })();
        match result {
            Ok((dest, outcome)) => {
                ok += 1;
                if format == ReportFormat::Text {
                    if dry_run {
                        eprintln!("{} (dry run)", file.display());
                        for r in &outcome.removed {
                            eprintln!("  - {} [{}]", r.kind_label(), r.label);
                        }
                    } else {
                        print_outcome(&dest, &outcome);
                    }
                }
                reports.push(serde_json::json!({
                    "path": file,
                    "output": if dry_run { None } else { Some(&dest) },
                    "ok": true,
                    "removed": outcome.removed,
                    "before": outcome.before,
                    "after": outcome.after,
                    "notes": outcome.notes,
                }));
            }
            Err(err) => {
                failed += 1;
                if format == ReportFormat::Text {
                    eprintln!("{}: {err:#}", file.display());
                }
                reports.push(
                    serde_json::json!({ "path": file, "ok": false, "error": format!("{err:#}") }),
                );
            }
        }
    }
    if format == ReportFormat::Json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else if files.len() > 1 {
        eprintln!(
            "\n{ok} scrubbed, {failed} failed (kept: {})",
            keep.describe()
        );
    }
    if failed > 0 {
        bail!("{failed} file(s) could not be scrubbed");
    }
    Ok(())
}

fn run_xmp(action: XmpAction) -> Result<()> {
    match action {
        XmpAction::Write {
            output,
            title,
            description,
            keywords,
            creator_tool,
            fields,
            recipe,
        } => {
            let mut map = BTreeMap::new();
            for field in fields {
                let Some((name, value)) = field.split_once('=') else {
                    bail!("expected --field NAME=VALUE, got `{field}`");
                };
                let name = name.trim();
                if !valid_field_name(name) {
                    bail!("invalid cybermeta field name `{name}` (letters, digits, _)");
                }
                map.insert(name.to_string(), value.to_string());
            }
            let recipe = match recipe {
                None => None,
                Some(text) => {
                    let json = match text.strip_prefix('@') {
                        Some(path) => std::fs::read_to_string(path)
                            .with_context(|| format!("read recipe {path}"))?,
                        None => text,
                    };
                    Some(serde_json::from_str(&json).context("--recipe is not valid JSON")?)
                }
            };
            let doc = XmpDoc {
                title,
                description,
                keywords: keywords
                    .into_iter()
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect(),
                creator_tool,
                metadata_date: None,
                fields: map,
                recipe,
            };
            let xml = doc.to_xml()?;
            if output == "-" {
                print!("{xml}");
            } else {
                // Atomic: write beside the target, then rename over it.
                let tmp = format!("{output}.cybermeta-tmp");
                std::fs::write(&tmp, xml).with_context(|| format!("write {tmp}"))?;
                std::fs::rename(&tmp, &output).with_context(|| format!("rename to {output}"))?;
                eprintln!("wrote {output}");
            }
            Ok(())
        }
        XmpAction::Read { file } => {
            let doc = XmpDoc::read(&file)?;
            println!("{}", serde_json::to_string_pretty(&doc)?);
            Ok(())
        }
    }
}
