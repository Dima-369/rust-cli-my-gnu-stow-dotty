use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use mlua::{Function, Lua, Value};
use std::fs;
use std::fs::read_dir;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

fn shorten_home(p: &Path) -> String {
    let p_str = p.to_string_lossy();
    if let Ok(home) = std::env::var("HOME")
        && p_str.starts_with(&home)
    {
        return format!("~{}", &p_str[home.len()..]);
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

/// Build the companion .lua path for a given source path.
/// Always appends ".lua" to the full file name to handle names with dots correctly.
/// e.g. "my.dir" -> "my.dir.lua", "foo" -> "foo.lua", "bar.txt" -> "bar.txt.lua"
fn companion_lua_path(source: &Path) -> PathBuf {
    let mut name = source.file_name().unwrap_or_default().to_os_string();
    name.push(".lua");
    source.with_file_name(name)
}

#[derive(Debug)]
struct LuaDecision {
    include: bool,
    rename_to: Option<String>,
    transform: Option<String>,
    /// When true (directories only), symlink the entire directory instead of recursing.
    link: bool,
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
            link: false,
        }),
        Value::Table(t) => {
            let rt: Option<String> = t.get("rename_to").unwrap_or_default();
            if let Some(name) = &rt {
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

            let link: bool = t
                .get::<Option<bool>>("link")
                .unwrap_or_default()
                .unwrap_or(false);

            let transform_fn: Option<Function> = t.get("transform").unwrap_or_default();
            let transformed_content = if let Some(func) = transform_fn {
                if source_file.is_dir() {
                    bail!(
                        "transform is not supported for directories: {}",
                        source_file.display()
                    );
                }
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
                link,
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

/// Compare two paths for equality using canonicalize when possible,
/// falling back to direct comparison.
fn paths_match(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Result of attempting to create or verify a symlink at `target` pointing to `source`.
enum SymlinkResult {
    /// Symlink was created or already in place.
    Planned,
    /// Target already exists and conflicts.
    Conflict,
    /// Target exists but is identical (content or link matches).
    Override,
}

/// Handle symlink creation/conflict for both files and directories.
/// `label` is "dir" or "" for log messages.
fn handle_symlink(
    source: &Path,
    target: &Path,
    label: &str,
    opts: Options,
    content_matches: bool,
) -> Result<SymlinkResult> {
    let label_prefix = if label.is_empty() {
        "".to_string()
    } else {
        format!("{label} ")
    };

    // Create parent dirs if not dry-run
    if !opts.dry_run
        && let Some(parent) = target.parent()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create parent directories for {}",
                target.display()
            )
        })?;
    }

    if target.exists() || target.is_symlink() {
        let is_symlink = fs::symlink_metadata(target)
            .ok()
            .is_some_and(|m| m.file_type().is_symlink());
        let link_target_matches = is_symlink
            && fs::read_link(target)
                .ok()
                .is_some_and(|link_dest| paths_match(&link_dest, source));
        let identical = link_target_matches || content_matches;

        if link_target_matches {
            if opts.dry_run || opts.verbose {
                println!(
                    "{} Would link {label_prefix}(already in place) {} -> {}",
                    opts.color.green("✔"),
                    shorten_home(target),
                    shorten_home(source)
                );
            }
            return Ok(SymlinkResult::Planned);
        }

        if opts.override_identical && identical && !opts.dry_run && !target.is_dir() {
            println!(
                "{} override identical: {} <- {}",
                opts.color.green("↻"),
                shorten_home(target),
                shorten_home(source)
            );
            let _ = fs::remove_file(target);
            unix_fs::symlink(source, target).with_context(|| {
                format!(
                    "Failed to symlink {} -> {}",
                    target.display(),
                    source.display()
                )
            })?;
            println!(
                "{} Linked {label_prefix}{} -> {}",
                opts.color.green("✔"),
                shorten_home(target),
                shorten_home(source)
            );
            return Ok(SymlinkResult::Override);
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
        let state_suffix = if state.is_empty() {
            String::new()
        } else {
            format!(" ({state})")
        };
        println!(
            "{} {} {} <- {}{state_suffix}",
            opts.color.red("✗"),
            opts.color.red("exists"),
            shorten_home(target),
            shorten_home(source),
        );
        return Ok(SymlinkResult::Conflict);
    }

    // No conflict — create symlink
    if opts.dry_run {
        println!(
            "{} Would symlink {label_prefix}{} -> {}",
            opts.color.green("✔"),
            shorten_home(target),
            shorten_home(source)
        );
    } else {
        unix_fs::symlink(source, target).with_context(|| {
            format!(
                "Failed to symlink {} -> {}",
                target.display(),
                source.display()
            )
        })?;
        println!(
            "{} Linked {label_prefix}{} -> {}",
            opts.color.green("✔"),
            shorten_home(target),
            shorten_home(source)
        );
    }
    Ok(SymlinkResult::Planned)
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
                // Check if this is a companion file by seeing if there's a corresponding non-.lua entry
                let corresponding = root.join(rel).join(base_name);
                if corresponding.exists() {
                    // This is a companion file, skip it
                    continue;
                }
                // This is a standalone .lua file, process it normally
            }

            if path.is_dir() {
                // Skip symlinks-to-directories in source root to prevent circular recursion
                let meta = fs::symlink_metadata(&path)
                    .with_context(|| format!("Failed to read metadata for {}", path.display()))?;
                if meta.file_type().is_symlink() {
                    continue;
                }

                // Check for companion .lua file
                let dir_companion = companion_lua_path(&path);
                if dir_companion.is_file() {
                    let decision = lua_decision(lua, &dir_companion, &path)?;
                    if !decision.include {
                        if opts.dry_run {
                            println!(
                                "{} Skipped by lua: {}",
                                opts.color.blue("ℹ"),
                                shorten_home(&home.join(&rel_path))
                            );
                        }
                        skips += 1;
                        continue;
                    }
                    if decision.link {
                        let target_rel_path = if let Some(new_name) = &decision.rename_to {
                            rel_path.with_file_name(new_name)
                        } else {
                            rel_path.to_path_buf()
                        };
                        let target = home.join(&target_rel_path);

                        match handle_symlink(&path, &target, "dir", opts, false)? {
                            SymlinkResult::Planned => planned += 1,
                            SymlinkResult::Conflict => conflicts += 1,
                            SymlinkResult::Override => {
                                planned += 1;
                                overrides += 1;
                            }
                        }
                        continue;
                    }
                    // If link is not set, fall through to normal recursion
                }

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
                let companion = companion_lua_path(&path);

                let decision = if companion.exists() {
                    lua_decision(lua, &companion, &path)?
                } else {
                    LuaDecision {
                        include: true,
                        rename_to: None,
                        transform: None,
                        link: false,
                    }
                };

                if !decision.include {
                    if opts.dry_run {
                        println!(
                            "{} Skipped by lua: {}",
                            opts.color.blue("ℹ"),
                            shorten_home(&home.join(&rel_path))
                        );
                    }
                    skips += 1;
                    continue;
                }

                let target_rel_path = if let Some(new_name) = &decision.rename_to {
                    rel_path.with_file_name(new_name)
                } else {
                    rel_path.to_path_buf()
                };
                let target = home.join(&target_rel_path);

                // Handle transformed files (write/override)
                if let Some(transformed_content) = &decision.transform {
                    if !opts.dry_run
                        && let Some(parent) = target.parent()
                    {
                        fs::create_dir_all(parent).with_context(|| {
                            format!(
                                "Failed to create parent directories for {}",
                                target.display()
                            )
                        })?;
                    }

                    if target.is_dir() {
                        println!(
                            "{} Conflict: cannot write file, target is a directory: {}",
                            opts.color.red("✗"),
                            shorten_home(&target)
                        );
                        conflicts += 1;
                        continue;
                    }

                    let content_is_identical = target.is_file()
                        && fs::read(&target).ok().as_deref()
                            == Some(transformed_content.as_bytes());

                    if content_is_identical {
                        planned += 1;
                        if opts.dry_run || opts.verbose {
                            println!(
                                "{} Would write (already in place) {} from {}",
                                opts.color.green("✔"),
                                shorten_home(&target),
                                shorten_home(&path)
                            );
                        }
                        continue;
                    }

                    let target_existed = target.exists();
                    if opts.dry_run {
                        let action = if target_existed { "overwrite" } else { "write" };
                        println!(
                            "{} Would {action} transformed file {} from {}",
                            opts.color.green("✔"),
                            shorten_home(&target),
                            shorten_home(&path)
                        );
                    } else {
                        fs::write(&target, transformed_content).with_context(|| {
                            format!("Failed to write transformed file {}", target.display())
                        })?;
                        let action = if target_existed { "Overwrote" } else { "Wrote" };
                        println!(
                            "{} {action} transformed file {} from {}",
                            opts.color.green("✔"),
                            shorten_home(&target),
                            shorten_home(&path)
                        );
                    }
                    planned += 1;
                    continue;
                }

                // Handle symlinks via shared helper
                let content_matches = {
                    let is_symlink = target
                        .symlink_metadata()
                        .ok()
                        .is_some_and(|m| m.file_type().is_symlink());
                    target.is_file()
                        && !is_symlink
                        && path.is_file()
                        && fs::read(&target).ok() == fs::read(&path).ok()
                };

                match handle_symlink(&path, &target, "", opts, content_matches)? {
                    SymlinkResult::Planned => planned += 1,
                    SymlinkResult::Conflict => conflicts += 1,
                    SymlinkResult::Override => {
                        planned += 1;
                        overrides += 1;
                    }
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
    let skipped_label = "skipped by lua";
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
