use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use mlua::{Function, Lua, Value};
use std::fs;
use std::fs::read_dir;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

fn shorten_home(p: &Path) -> String {
    let p_str = p.to_string_lossy();
    if let Ok(home) = std::env::var("HOME") {
        if p_str.starts_with(&home) {
            return format!("~{}", &p_str[home.len()..]);
        }
    }
    p_str.to_string()
}

// Simple color helpers using ANSI escapes (runtime switchable)
#[derive(Clone, Copy, Debug)]
struct Colorize(bool);
impl Colorize {
    fn green(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[32m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn red(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[31m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn blue(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[34m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn yellow(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[33m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Root directory that contains dotfiles to stow
    #[arg(short, long, default_value = "~/Developer/dotfiles/dotty/")]
    root: String,
}

fn expand_tilde(p: &str) -> Result<PathBuf> {
    if let Some(rest) = p.strip_prefix("~/") {
        let home = std::env::var("HOME").context("HOME environment variable must be set")?;
        Ok(PathBuf::from(home).join(rest))
    } else if p == "~" {
        let home = std::env::var("HOME").context("HOME environment variable must be set")?;
        Ok(PathBuf::from(home))
    } else {
        Ok(PathBuf::from(p))
    }
}

#[derive(Debug)]
struct LuaDecision {
    include: bool,
    rename_to: Option<String>,
    transform: Option<String>,
}

fn lua_decision(lua: &Lua, lua_file: &Path, source_file: &Path) -> Result<LuaDecision> {
    let src = fs::read_to_string(lua_file)
        .with_context(|| format!("Failed to read Lua file: {}", lua_file.display()))?;
    let chunk = lua.load(&src).set_name(lua_file.to_string_lossy());
    let value = chunk
        .eval::<Value>()
        .map_err(|e| anyhow::anyhow!("Failed to execute Lua chunk: {}", e))?;
    match value {
        Value::Boolean(b) => Ok(LuaDecision {
            include: b,
            rename_to: None,
            transform: None,
        }),
        Value::Table(t) => {
            // read optional rename_to
            let rt: Option<String> = match t.get("rename_to") {
                Ok(v) => v,
                Err(_) => None,
            };
            if let Some(name) = &rt {
                // Validate: must not contain path separators
                if name.contains('/') || name.contains('\\') {
                    bail!(
                        "rename_to must be a file name without path separators: {}",
                        name
                    );
                }
                if name.is_empty() {
                    bail!("rename_to must not be empty");
                }
            }

            // New: read optional transform function
            let transform_fn: Option<Function> = match t.get("transform") {
                Ok(v) => v,
                Err(_) => None,
            };
            let transformed_content = if let Some(func) = transform_fn {
                let original_content = fs::read_to_string(source_file).with_context(|| {
                    format!(
                        "Failed to read source file for transform: {}",
                        source_file.display()
                    )
                })?;
                let result: String = func
                    .call(original_content)
                    .map_err(|e| anyhow!("Lua transform function error: {}", e))?;
                Some(result)
            } else {
                None
            };

            Ok(LuaDecision {
                include: true,
                rename_to: rt,
                transform: transformed_content,
            })
        }
        other => bail!(
            "Lua filter must return boolean or table for {}. Got {}",
            lua_file.display(),
            other.type_name()
        ),
    }
}

#[derive(Clone, Copy, Debug)]
struct Options {
    dry_run: bool,
    override_identical: bool,
    verbose: bool,
    color: Colorize,
}

fn process(root: &Path, opts: Options) -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME must be set")?);
    let lua = Lua::new();

    #[derive(Default)]
    struct WalkCounts {
        planned: usize,
        conflicts: usize,
        skips: usize,
        overrides: usize,
    }
    fn walk_dir(
        root: &Path,
        rel: &Path,
        home: &Path,
        lua: &Lua,
        opts: Options,
    ) -> Result<WalkCounts> {
        let mut planned: usize = 0;
        let mut conflicts: usize = 0;
        let mut skips: usize = 0;
        let mut overrides: usize = 0;
        for entry in read_dir(root.join(rel))
            .with_context(|| format!("Failed to read dir {}", root.join(rel).display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let rel_path = rel.join(entry.file_name());
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if let Some(base_name) = file_name_str.strip_suffix(".lua") {
                // Check if this is a companion file by seeing if there's a corresponding non-.lua file
                // Remove ".lua"
                let corresponding_file = root.join(rel).join(base_name);
                if corresponding_file.exists() {
                    // This is a companion file, skip it
                    continue;
                }
                // This is a standalone .lua file, process it normally
            }

            if path.is_dir() {
                // Recurse into directories
                let sub = walk_dir(root, &rel_path, home, lua, opts)?;
                planned += sub.planned;
                conflicts += sub.conflicts;
                skips += sub.skips;
                overrides += sub.overrides;
                continue;
            }

            // Only symlink or transform actual files
            if path.is_file() {
                // 1. Determine Lua decision
                let companion = path.with_extension(format!(
                    "{}lua",
                    path.extension()
                        .map(|e| format!("{}.", e.to_string_lossy()))
                        .unwrap_or_else(|| "".to_string())
                ));

                let decision = if companion.exists() {
                    lua_decision(lua, &companion, &path)?
                } else {
                    LuaDecision {
                        include: true,
                        rename_to: None,
                        transform: None,
                    }
                };

                // 2. Handle skip
                if !decision.include {
                    if opts.dry_run {
                        println!(
                            "{} {}",
                            opts.color.blue("ℹ"),
                            format!("Skipped by lua: {}", shorten_home(&home.join(&rel_path)))
                        );
                    }
                    skips += 1;
                    continue;
                }

                // 3. Determine target path
                let target_rel_path = if let Some(new_name) = &decision.rename_to {
                    rel_path.with_file_name(new_name)
                } else {
                    rel_path.to_path_buf()
                };
                let target = home.join(&target_rel_path);

                // 4. Create parent dirs if not dry-run
                if !opts.dry_run {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!(
                                "Failed to create parent directories for {}",
                                target.display()
                            )
                        })?;
                    }
                }

                // 5. Check for conflicts and process
                if target.exists() || target.is_symlink() {
                    // --- CONFLICT PATH ---
                    let (identical, is_already_linked) = if let Some(content) = &decision.transform
                    {
                        // Transformed file identity check
                        let is_identical = fs::read(&target)
                            .ok()
                            .map_or(false, |e| e == content.as_bytes());
                        (is_identical, false)
                    } else {
                        // Symlink identity check
                        let is_symlink = fs::symlink_metadata(&target)
                            .ok()
                            .map_or(false, |m| m.file_type().is_symlink());
                        let link_target_matches =
                            is_symlink && fs::read_link(&target).ok() == Some(path.to_path_buf());
                        let content_matches = target.is_file()
                            && path.is_file()
                            && fs::read(&target).ok() == fs::read(&path).ok();
                        (link_target_matches || content_matches, link_target_matches)
                    };

                    if is_already_linked || (decision.transform.is_some() && identical) {
                        planned += 1;
                        if opts.dry_run || opts.verbose {
                            let action_desc = if decision.transform.is_some() {
                                "write"
                            } else {
                                "link"
                            };
                            println!(
                                "{} {}",
                                opts.color.green("✔"),
                                format!(
                                    "Would {} (already in place) {} -> {}",
                                    action_desc,
                                    shorten_home(&target),
                                    shorten_home(&path)
                                )
                            );
                        }
                        continue;
                    }

                    if opts.override_identical && identical && !opts.dry_run {
                        if !target.is_dir() {
                            println!(
                                "{} {}",
                                opts.color.green("↻"),
                                format!(
                                    "override identical: {} <- {}",
                                    shorten_home(&target),
                                    shorten_home(&path)
                                )
                            );
                            let _ = fs::remove_file(&target);
                            if let Some(content_to_write) = decision.transform {
                                fs::write(&target, content_to_write).with_context(|| {
                                    format!("Failed to write transformed file {}", target.display())
                                })?;
                                println!(
                                    "{} {}",
                                    opts.color.green("✔"),
                                    format!(
                                        "Wrote transformed file {} from {}",
                                        shorten_home(&target),
                                        shorten_home(&path)
                                    )
                                );
                            } else {
                                unix_fs::symlink(&path, &target).with_context(|| {
                                    format!(
                                        "Failed to symlink {} -> {}",
                                        target.display(),
                                        path.display()
                                    )
                                })?;
                                println!(
                                    "{} {}",
                                    opts.color.green("✔"),
                                    format!(
                                        "Linked {} -> {}",
                                        shorten_home(&target),
                                        shorten_home(&path)
                                    )
                                );
                            }
                            planned += 1;
                            overrides += 1;
                            continue;
                        }
                    }

                    // Real conflict
                    let mut state = String::new();
                    if opts.dry_run || opts.verbose {
                        state = if identical {
                            opts.color.green("identical")
                        } else {
                            opts.color.yellow("differs")
                        };
                    }
                    println!(
                        "{} {}",
                        &format!("{} {}", opts.color.red("✗"), opts.color.red("exists")),
                        {
                            format!(
                                "{} <- {}{}",
                                shorten_home(&target),
                                shorten_home(&path),
                                if !state.is_empty() {
                                    format!(" ({})", state)
                                } else {
                                    "".to_string()
                                }
                            )
                        }
                    );
                    conflicts += 1;
                } else {
                    // --- NO CONFLICT PATH (PLANNED) ---
                    if let Some(content_to_write) = decision.transform {
                        if opts.dry_run {
                            println!(
                                "{} {}",
                                opts.color.green("✔"),
                                format!(
                                    "Would write transformed file {} from {}",
                                    shorten_home(&target),
                                    shorten_home(&path)
                                )
                            );
                        } else {
                            fs::write(&target, content_to_write).with_context(|| {
                                format!("Failed to write transformed file {}", target.display())
                            })?;
                            println!(
                                "{} {}",
                                opts.color.green("✔"),
                                format!(
                                    "Wrote transformed file {} from {}",
                                    shorten_home(&target),
                                    shorten_home(&path)
                                )
                            );
                        }
                    } else {
                        if opts.dry_run {
                            println!(
                                "{} {}",
                                opts.color.green("✔"),
                                format!(
                                    "Would symlink {} -> {}",
                                    shorten_home(&target),
                                    shorten_home(&path)
                                )
                            );
                        } else {
                            unix_fs::symlink(&path, &target).with_context(|| {
                                format!(
                                    "Failed to symlink {} -> {}",
                                    target.display(),
                                    path.display()
                                )
                            })?;
                            println!(
                                "{} {}",
                                opts.color.green("✔"),
                                format!(
                                    "Linked {} -> {}",
                                    shorten_home(&target),
                                    shorten_home(&path)
                                )
                            );
                        }
                    }
                    planned += 1;
                }
            }
        }
        Ok(WalkCounts {
            planned,
            conflicts,
            skips,
            overrides,
        })
    }

    let totals = walk_dir(root, Path::new(""), &home, &lua, opts)?;
    let conflicts_label = if totals.conflicts == 1 {
        "conflict"
    } else {
        "conflicts"
    };
    let planned_label = if opts.dry_run { "planned" } else { "linked" };
    let skipped_label = if totals.skips == 1 {
        "skipped by lua"
    } else {
        "skipped by lua"
    };
    println!(
        "\nSummary: {} {}, {} {}, {} {}, {} overrides",
        opts.color.green(&totals.planned.to_string()),
        planned_label,
        opts.color.red(&totals.conflicts.to_string()),
        conflicts_label,
        opts.color.blue(&totals.skips.to_string()),
        skipped_label,
        opts.color.green(&totals.overrides.to_string()),
    );
    Ok(())
}

fn main() -> Result<()> {
    // This tool is intended for macOS only
    #[cfg(not(target_os = "macos"))]
    compile_error!("This tool only supports macOS (target_os=macos)");
    #[derive(Parser, Debug)]
    #[command(author, version, about)]
    struct Cli {
        /// Root directory that contains dotfiles to stow
        #[arg(short, long, default_value = "~/Developer/dotfiles/dotty/")]
        root: String,
        /// Dry run: only print operations, do not modify filesystem
        #[arg(long)]
        dry_run: bool,
        /// If set, when a conflict target has identical content, delete it and create the symlink
        #[arg(long)]
        override_identical: bool,
        /// Verbose output
        #[arg(long)]
        verbose: bool,
        /// Disable colored output
        #[arg(long)]
        no_color: bool,
    }

    let cli = Cli::parse();
    let root_path = expand_tilde(&cli.root)?;
    if !root_path.is_dir() {
        bail!("Root directory is not a directory: {}", root_path.display());
    }
    // Auto-detect TTY to decide default colors, allow --no-color to override
    let stdout_is_tty = atty::is(atty::Stream::Stdout);
    let color = Colorize(stdout_is_tty && !cli.no_color);
    let opts = Options {
        dry_run: cli.dry_run,
        override_identical: cli.override_identical,
        verbose: cli.verbose,
        color,
    };
    process(&root_path, opts)
}
