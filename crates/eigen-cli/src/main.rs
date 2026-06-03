use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "eigen", version, about = "control surface over Claude Code sessions")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// inspect skills available at hierarchical resolution levels
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
}

#[derive(Subcommand, Debug)]
enum SkillsAction {
    /// show the full override stack: every skill at every layer, with collisions marked
    Tree {
        /// override the working directory used to compute the repo layer
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Skills { action } => match action {
            SkillsAction::Tree { cwd } => skills_tree(cwd),
        },
    }
}

fn skills_tree(cwd_override: Option<PathBuf>) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;
    let cwd = match cwd_override {
        Some(p) => p,
        None => env::current_dir().context("could not read current dir")?,
    };

    let roots = eigen_skills::canonical_roots(&home, &cwd);
    let found = eigen_skills::scan_many(&roots)
        .with_context(|| format!("scanning skills under home={:?} cwd={:?}", home, cwd))?;

    print!("{}", eigen_skills::render_tree(&found));
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}
