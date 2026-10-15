use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use cybermeta::meta::xmp::valid_field_name;
use cybermeta::meta::{list_presets, ExportFormat, MetaDoc, Preset, XmpDoc};
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
    /// Strip GPS and/or all non-structural metadata
    Strip {
        file: PathBuf,
        #[arg(long, conflicts_with = "all")]
        gps: bool,
        #[arg(long)]
        all: bool,
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
    /// Apply a named scrub/spoof preset
    Spoof {
        file: Option<PathBuf>,
        #[arg(long, required_unless_present = "list")]
        preset: Option<String>,
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
                bail!("no TTY and no subcommand — use show/export/strip/set/spoof/xmp");
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
            output,
            in_place,
        }) => {
            if !gps && !all {
                bail!("specify --gps and/or --all");
            }
            let mut doc = MetaDoc::open(&file)?;
            if gps {
                let n = doc.strip_gps();
                eprintln!("stripped {n} GPS-related tags");
            }
            if all {
                let n = doc.strip_all();
                eprintln!("stripped {n} non-structural tags");
            }
            let dest = doc.save(output.as_deref(), in_place)?;
            eprintln!("wrote {}", dest.display());
            Ok(())
        }
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
            output,
            in_place,
            list,
        }) => {
            if list {
                for p in list_presets() {
                    println!("{}\t{}", p.as_str(), p.description());
                }
                return Ok(());
            }
            let file = file.context("FILE is required unless --list")?;
            let preset_name = preset.context("--preset is required unless --list")?;
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
            let mut doc =
                MetaDoc::open(&file).with_context(|| format!("open {}", file.display()))?;
            doc.apply_preset(preset)?;
            let dest = doc.save(output.as_deref(), in_place)?;
            eprintln!("applied {} → {}", preset.as_str(), dest.display());
            Ok(())
        }
    }
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
