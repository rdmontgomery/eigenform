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
    /// inspect Claude Code auto-memory at the project / cross-project levels
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
}

#[derive(Subcommand, Debug)]
enum MemoryAction {
    /// memory entries for one project (defaults to current cwd)
    Tree {
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// memory entries across every project
    List {
        #[arg(long)]
        all_projects: bool,
    },
}

#[derive(Subcommand, Debug)]
enum SkillsAction {
    /// show the full override stack for one resolution context
    Tree {
        /// override the working directory used to compute the repo layer
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// inventory of skills across every Claude Code project on this machine
    List {
        /// scan every project under ~/.claude/projects/ in addition to global+plugins
        #[arg(long)]
        all_projects: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Skills { action } => match action {
            SkillsAction::Tree { cwd } => skills_tree(cwd),
            SkillsAction::List { all_projects } => skills_list(all_projects),
        },
        Cmd::Memory { action } => match action {
            MemoryAction::Tree { cwd } => memory_tree(cwd),
            MemoryAction::List { all_projects } => memory_list(all_projects),
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

fn skills_list(all_projects: bool) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;

    if !all_projects {
        anyhow::bail!("eigen skills list currently requires --all-projects");
    }

    let projects_dir = home.join(".claude/projects");
    let projects = eigen_projects::enumerate_projects(&projects_dir)
        .with_context(|| format!("enumerating projects in {:?}", projects_dir))?;

    let cwds: Vec<PathBuf> = projects.iter().map(|p| p.cwd.clone()).collect();
    let roots = eigen_skills::all_projects_roots(&home, &cwds);
    let found = eigen_skills::scan_many(&roots)
        .with_context(|| "scanning skills across all projects")?;

    println!("# projects discovered: {}", projects.len());
    for p in &projects {
        println!("#   {} -> {}", p.dir_name, p.cwd.display());
    }
    println!();
    print!("{}", eigen_skills::render_tree(&found));
    Ok(())
}

fn memory_tree(cwd_override: Option<PathBuf>) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;
    let cwd = match cwd_override {
        Some(p) => p,
        None => env::current_dir().context("could not read current dir")?,
    };
    let projects_dir = home.join(".claude/projects");

    let project = eigen_projects::project_for_cwd(&projects_dir, &cwd)
        .with_context(|| format!("looking up project for cwd {:?}", cwd))?;
    let Some(project) = project else {
        println!("MEMORY");
        println!("======");
        println!();
        println!("(no memory: no Claude Code project recorded for {:?})", cwd);
        return Ok(());
    };

    let memory_dir = projects_dir.join(&project.dir_name).join("memory");
    let entries = eigen_memory::scan_memory_dir(&memory_dir)
        .with_context(|| format!("scanning memory in {:?}", memory_dir))?;

    println!("# project: {} -> {}", project.dir_name, project.cwd.display());
    println!("# memory dir: {}", memory_dir.display());
    println!();
    print!("{}", eigen_memory::render_memory_tree(&entries));
    Ok(())
}

fn memory_list(all_projects: bool) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;
    if !all_projects {
        anyhow::bail!("eigen memory list currently requires --all-projects");
    }
    let projects_dir = home.join(".claude/projects");
    let projects = eigen_projects::enumerate_projects(&projects_dir)
        .with_context(|| format!("enumerating projects in {:?}", projects_dir))?;

    println!("# projects discovered: {}", projects.len());
    println!();

    for p in &projects {
        let memory_dir = projects_dir.join(&p.dir_name).join("memory");
        let entries = eigen_memory::scan_memory_dir(&memory_dir)
            .with_context(|| format!("scanning {:?}", memory_dir))?;

        println!("================================================================");
        println!("project: {} -> {}", p.dir_name, p.cwd.display());
        println!("================================================================");
        print!("{}", eigen_memory::render_memory_tree(&entries));
        println!();
    }
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}
