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
    /// run woland: the browser workbench (serves a pty terminal at localhost)
    Daemon {
        /// port to bind on localhost
        #[arg(long, default_value_t = 4317)]
        port: u16,
        /// command to run in the pty (default: $SHELL, else bash). NOT claude unless you ask.
        #[arg(long)]
        cmd: Option<String>,
        /// directory of built frontend assets (default: ./web)
        #[arg(long)]
        web: Option<PathBuf>,
        /// dev mode: inject the live-reload hook and serve /api/dev/reload
        #[arg(long)]
        dev: bool,
    },
}

#[derive(Subcommand, Debug)]
enum SessionsAction {
    /// render a session as a turn-tree (resolve by uuid, prefix, or path)
    Show {
        /// session uuid, unique prefix, or a path to the JSONL
        session: String,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        render: RenderFormat,
    },
    /// diff two sessions side-by-side (source vs fork, aligned by turn uuid)
    Diff {
        /// source session: uuid, prefix, or path
        a: String,
        /// fork session: uuid, prefix, or path
        b: String,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        render: RenderFormat,
    },
    /// list recent sessions (current project, last 7 days, by default)
    List {
        /// recency window: e.g. 7d, 24h, 30m, 2w, or `all`
        #[arg(long)]
        since: Option<String>,
        /// list across every project, not just the current one
        #[arg(long)]
        all_projects: bool,
        /// list the project at this cwd instead of the current directory
        #[arg(long)]
        cwd: Option<PathBuf>,
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
        /// source session: uuid, unique prefix, or a path
        session: String,
        /// uuid of the turn to fork at
        #[arg(long)]
        at: String,
        /// replace the turn's content with this file's text (edit-then-fork)
        #[arg(long)]
        edit: Option<PathBuf>,
        /// also print a source→fork diff to stderr
        #[arg(long)]
        diff: bool,
    },
    /// rewind a session to a chosen turn (prefix only)
    Rewind {
        /// source session: uuid, unique prefix, or a path
        session: String,
        /// uuid of the turn to rewind to
        #[arg(long)]
        to: String,
    },
    /// inject a synthetic turn after a chosen turn, as the new resume head
    Inject {
        /// source session: uuid, unique prefix, or a path
        session: String,
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
        Cmd::Daemon { port, cmd, web, dev } => daemon(port, cmd, web, dev),
        Cmd::Sessions { action } => match action {
            SessionsAction::Show { session, render } => sessions_show(session, render),
            SessionsAction::Diff { a, b, render } => sessions_diff(a, b, render),
            SessionsAction::List {
                since,
                all_projects,
                cwd,
                render,
            } => sessions_list(since, all_projects, cwd, render),
        },
    }
}

fn require_text(render: RenderFormat) -> Result<()> {
    match render {
        RenderFormat::Text => Ok(()),
        RenderFormat::Json | RenderFormat::Html => anyhow::bail!(
            "--render {:?} is not supported yet; the json/html schema is deferred until we \
             render in the browser. Use --render text.",
            render
        ),
    }
}

fn projects_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .context("could not determine home directory")?
        .join(".claude/projects"))
}

fn sessions_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .context("could not determine home directory")?
        .join(".claude/sessions"))
}

fn state_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .context("could not determine home directory")?
        .join(".eigen/state"))
}

/// Resolve a `<uuid|prefix|path>` argument to a session file path. A literal existing
/// file wins; otherwise resolve as a uuid/prefix machine-wide, printing candidates on
/// ambiguity.
fn resolve_session(session: &str) -> Result<PathBuf> {
    if PathBuf::from(session).is_file() {
        return Ok(PathBuf::from(session));
    }
    let dir = projects_dir()?;
    match eigen_forest::resolve(&dir, session) {
        Ok(p) => Ok(p),
        Err(eigen_forest::ResolveError::Ambiguous(candidates)) => {
            eprintln!("`{session}` is ambiguous — {} sessions match:", candidates.len());
            for c in &candidates {
                let title = eigen_forest::session_ref(c)
                    .title
                    .unwrap_or_else(|| "(untitled)".to_string());
                eprintln!("  {}  {}", &c.uuid[..c.uuid.len().min(8)], title);
            }
            anyhow::bail!("specify more of the uuid");
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

fn sessions_show(session: String, render: RenderFormat) -> Result<()> {
    require_text(render)?;
    let path = resolve_session(&session)?;
    let src = load_session(&path)?;
    let view = eigen_render::session_view(&src);
    print!("{}", eigen_render::render_text(&view));
    Ok(())
}

fn sessions_list(
    since: Option<String>,
    all_projects: bool,
    cwd: Option<PathBuf>,
    render: RenderFormat,
) -> Result<()> {
    require_text(render)?;
    let dir = projects_dir()?;

    let scope = if all_projects {
        eigen_forest::Scope::AllProjects
    } else {
        let here = match cwd {
            Some(c) => c,
            None => env::current_dir().context("could not read current dir")?,
        };
        eigen_forest::Scope::Project(here)
    };

    let window = parse_since(since.as_deref())?;
    let now = chrono::Utc::now();
    let sessions = eigen_forest::list(&dir, scope, window, now).map_err(|e| anyhow::anyhow!("{e}"))?;
    print!("{}", eigen_render::render_text(&eigen_render::sessions_view(&sessions, now, all_projects)));
    Ok(())
}

/// Parse a recency window: `None` defaults to 7 days; `all` means no window; otherwise
/// `<n><unit>` with unit m/h/d/w.
fn parse_since(since: Option<&str>) -> Result<Option<chrono::Duration>> {
    let Some(s) = since else {
        return Ok(Some(chrono::Duration::days(7)));
    };
    if s == "all" {
        return Ok(None);
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: i64 = num
        .parse()
        .with_context(|| format!("invalid --since `{s}` (try 7d, 24h, 30m, 2w, or all)"))?;
    let dur = match unit {
        "m" => chrono::Duration::minutes(n),
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        "w" => chrono::Duration::weeks(n),
        _ => anyhow::bail!("invalid --since unit in `{s}` (use m, h, d, w, or all)"),
    };
    Ok(Some(dur))
}

fn surgery(action: SurgeryAction) -> Result<()> {
    // `diff_src` is set when the op should print a source→fork diff to stderr.
    let (session_path, result, diff_src) = match action {
        SurgeryAction::Fork {
            session,
            at,
            edit,
            diff,
        } => {
            let path = resolve_session(&session)?;
            let src = load_session(&path)?;
            let new = match edit {
                Some(edit_path) => {
                    let text = read_text(&edit_path)?;
                    eigen_surgery::edit_then_fork(&src, &at, &text)
                }
                None => eigen_surgery::fork_at(&src, &at),
            };
            (path, new, diff.then_some(src))
        }
        SurgeryAction::Rewind { session, to } => {
            let path = resolve_session(&session)?;
            let src = load_session(&path)?;
            let new = eigen_surgery::rewind_to(&src, &to);
            (path, new, None)
        }
        SurgeryAction::Inject {
            session,
            at,
            as_role,
            content,
        } => {
            let path = resolve_session(&session)?;
            let src = load_session(&path)?;
            let text = read_text(&content)?;
            let new = eigen_surgery::inject(&src, &at, as_role.into(), &text);
            (path, new, None)
        }
    };

    let new = result.map_err(|e| anyhow::anyhow!("{e}"))?;
    let projects_dir = session_path
        .parent()
        .context("source session has no parent directory")?;
    let uuid = eigen_surgery::write(&new, projects_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{uuid}");
    if let Some(src) = diff_src {
        eprint!("{}", eigen_render::render_text(&eigen_render::fork_diff_view(&src, &new)));
    }
    Ok(())
}

fn daemon(port: u16, cmd: Option<String>, web: Option<PathBuf>, dev: bool) -> Result<()> {
    let cwd = env::current_dir().context("could not read current dir")?;

    // The pty command: explicit --cmd, else the user's shell, else bash. Not claude
    // unless the user asks for it explicitly.
    let program = cmd
        .or_else(|| env::var("SHELL").ok())
        .unwrap_or_else(|| "bash".to_string());

    // Frontend assets: explicit --web, else ./web if it has a built bundle.
    let web_dir = web.or_else(|| {
        let candidate = cwd.join("web");
        candidate.join("dist/main.js").is_file().then_some(candidate)
    });
    if web_dir.is_none() {
        eprintln!(
            "note: no built frontend found (run `just build-web`); serving the /pty API only"
        );
    }

    let config = eigen_daemon::Config {
        program,
        args: Vec::new(),
        cwd: Some(cwd),
        web_dir,
        projects_dir: Some(projects_dir()?),
        sessions_dir: Some(sessions_dir()?),
        state_dir: Some(state_dir()?),
        dev,
    };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    println!("woland → http://{addr}{}", if dev { "  (dev: live-reload on)" } else { "" });

    let rt = tokio::runtime::Runtime::new().context("starting tokio runtime")?;
    rt.block_on(eigen_daemon::serve(addr, config))
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn sessions_diff(a: String, b: String, render: RenderFormat) -> Result<()> {
    require_text(render)?;
    let source = load_session(&resolve_session(&a)?)?;
    let fork = load_session(&resolve_session(&b)?)?;
    print!("{}", eigen_render::render_text(&eigen_render::fork_diff_view(&source, &fork)));
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
