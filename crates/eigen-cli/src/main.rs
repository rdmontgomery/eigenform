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
    /// context surgery on a session JSONL: fork, rewind, inject (prints the new uuid)
    Surgery {
        #[command(subcommand)]
        action: SurgeryAction,
    },
    /// inspect sessions
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
}

#[derive(Subcommand, Debug)]
enum SessionsAction {
    /// render a session JSONL as a turn-tree
    Show {
        /// path to the session JSONL
        session: PathBuf,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        render: RenderFormat,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum RenderFormat {
    Text,
    Json,
    Html,
}

#[derive(Subcommand, Debug)]
enum SurgeryAction {
    /// fork a session so it ends at a chosen turn (optionally editing that turn)
    Fork {
        /// path to the source session JSONL
        session: PathBuf,
        /// uuid of the turn to fork at
        #[arg(long)]
        at: String,
        /// replace the turn's content with this file's text (edit-then-fork)
        #[arg(long)]
        edit: Option<PathBuf>,
    },
    /// rewind a session to a chosen turn (prefix only)
    Rewind {
        session: PathBuf,
        /// uuid of the turn to rewind to
        #[arg(long)]
        to: String,
    },
    /// inject a synthetic turn after a chosen turn, as the new resume head
    Inject {
        session: PathBuf,
        /// uuid of the turn to inject after
        #[arg(long)]
        at: String,
        /// role of the injected turn
        #[arg(long = "as", value_enum)]
        as_role: RoleArg,
        /// file whose text becomes the injected turn's content
        #[arg(long)]
        content: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum RoleArg {
    User,
    Assistant,
    System,
}

impl From<RoleArg> for eigen_surgery::Role {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::User => eigen_surgery::Role::User,
            RoleArg::Assistant => eigen_surgery::Role::Assistant,
            RoleArg::System => eigen_surgery::Role::System,
        }
    }
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
        Cmd::Surgery { action } => surgery(action),
        Cmd::Sessions { action } => match action {
            SessionsAction::Show { session, render } => sessions_show(session, render),
        },
    }
}

fn sessions_show(session: PathBuf, render: RenderFormat) -> Result<()> {
    match render {
        RenderFormat::Text => {}
        RenderFormat::Json | RenderFormat::Html => {
            anyhow::bail!(
                "--render {:?} is not supported yet; the json/html schema is deferred until \
                 we render in the browser. Use --render text.",
                render
            );
        }
    }
    let src = load_session(&session)?;
    let view = eigen_render::session_view(&src);
    print!("{}", eigen_render::render_text(&view));
    Ok(())
}

fn surgery(action: SurgeryAction) -> Result<()> {
    let (session_path, result) = match action {
        SurgeryAction::Fork { session, at, edit } => {
            let src = load_session(&session)?;
            let new = match edit {
                Some(edit_path) => {
                    let text = read_text(&edit_path)?;
                    eigen_surgery::edit_then_fork(&src, &at, &text)
                }
                None => eigen_surgery::fork_at(&src, &at),
            };
            (session, new)
        }
        SurgeryAction::Rewind { session, to } => {
            let src = load_session(&session)?;
            let new = eigen_surgery::rewind_to(&src, &to);
            (session, new)
        }
        SurgeryAction::Inject {
            session,
            at,
            as_role,
            content,
        } => {
            let src = load_session(&session)?;
            let text = read_text(&content)?;
            let new = eigen_surgery::inject(&src, &at, as_role.into(), &text);
            (session, new)
        }
    };

    let new = result.map_err(|e| anyhow::anyhow!("{e}"))?;
    let projects_dir = session_path
        .parent()
        .context("source session has no parent directory")?;
    let uuid = eigen_surgery::write(&new, projects_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{uuid}");
    Ok(())
}

fn load_session(path: &PathBuf) -> Result<eigen_surgery::Session> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading session {path:?}"))?;
    eigen_surgery::Session::parse_str(&contents).map_err(|e| anyhow::anyhow!("{e}"))
}

fn read_text(path: &PathBuf) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))
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
