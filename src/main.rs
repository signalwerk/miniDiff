//! MiniDiff – a native diff / merge tool built with egui and tree-sitter.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod demo;
mod diff;
mod folder;
mod highlight;
mod merge;
mod platform;
mod source;
mod text;
mod theme;
mod ui;
mod update;
mod cli {
    use std::path::PathBuf;

    use clap::Parser;

    use crate::app::Launch;
    use crate::source::Entry;

    /// Compare files and folders, or resolve merge conflicts.
    ///
    /// Examples:
    ///   minidiff a.rs b.rs                 compare two files
    ///   minidiff dir1 dir2                 compare two folders
    ///   minidiff conflicted.rs             resolve conflict markers in place
    ///   minidiff --merge LOCAL REMOTE BASE MERGED      (Tower / git mergetool order)
    ///   minidiff --merge --local L --remote R --base B --output M
    #[derive(Parser, Debug)]
    #[command(name = "minidiff", version, verbatim_doc_comment)]
    pub struct Cli {
        /// Paths: two to compare, one conflicted file, or merge inputs with --merge.
        pub paths: Vec<PathBuf>,
        /// Run as merge tool. Exits with 0 when the result was saved, 1 otherwise.
        #[arg(long)]
        pub merge: bool,
        #[arg(long)]
        pub base: Option<PathBuf>,
        #[arg(long)]
        pub local: Option<PathBuf>,
        #[arg(long)]
        pub remote: Option<PathBuf>,
        /// Where to write the merge result.
        #[arg(long, short = 'o')]
        pub output: Option<PathBuf>,
        /// Pane titles (repeat: left/right, or local/base/remote).
        #[arg(long = "label", short = 'L')]
        pub labels: Vec<String>,
    }

    fn existing(p: Option<PathBuf>) -> Option<Entry> {
        p.filter(|p| p.exists() && std::fs::metadata(p).is_ok_and(|m| m.len() > 0 || m.is_dir()))
            .map(Entry::Fs)
    }

    pub fn parse() -> Result<Launch, String> {
        // macOS used to pass a `-psn_…` process serial number when launched from Finder.
        let args = std::env::args_os().filter(|a| !a.to_string_lossy().starts_with("-psn_"));
        let cli = Cli::parse_from(args);

        if cli.merge {
            let mut paths = cli.paths.into_iter();
            let (local, remote, base, output) = match paths.len() {
                4 => (paths.next(), paths.next(), paths.next(), paths.next()),
                3 => (paths.next(), paths.next(), None, paths.next()),
                _ => (cli.local, cli.remote, cli.base, cli.output),
            };
            let local = local.ok_or("--merge needs a local file")?;
            let remote = remote.ok_or("--merge needs a remote file")?;
            let labels = (cli.labels.len() == 3).then(|| [cli.labels[0].clone(), cli.labels[1].clone(), cli.labels[2].clone()]);
            return Ok(Launch::Merge {
                base: existing(base),
                local: Entry::Fs(local),
                remote: Entry::Fs(remote),
                output,
                labels,
            });
        }

        for p in &cli.paths {
            if !p.exists() {
                return Err(format!("{}: no such file or directory", p.display()));
            }
        }
        Ok(match cli.paths.len() {
            0 => Launch::Welcome,
            2 => {
                let labels = (cli.labels.len() == 2).then(|| (cli.labels[0].clone(), cli.labels[1].clone()));
                let mut it = cli.paths.into_iter();
                Launch::Compare {
                    left: Entry::Fs(it.next().unwrap()),
                    right: Entry::Fs(it.next().unwrap()),
                    labels,
                }
            }
            _ => Launch::Open(cli.paths.into_iter().map(Entry::Fs).collect()),
        })
    }
}
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let launch = match cli::parse() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("minidiff: {e}");
            std::process::exit(2);
        }
    };

    #[cfg(target_os = "macos")]
    platform::macos::install_open_handler();

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MiniDiff")
        .with_app_id("minidiff")
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([640.0, 400.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        // Screenshot automation uses an isolated profile, including its initial window size.
        persistence_path: std::env::var_os("MINIDIFF_STORAGE_PATH").map(std::path::PathBuf::from),
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(
        "MiniDiff",
        options,
        Box::new(|cc| Ok(Box::new(app::MiniDiffApp::new(cc, launch)))),
    ) {
        eprintln!("minidiff: {e}");
        std::process::exit(2);
    }
    std::process::exit(platform::exit_code());
}
