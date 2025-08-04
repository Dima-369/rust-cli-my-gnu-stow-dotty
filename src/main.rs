use anyhow::{bail, Context, Result};
use clap::Parser;
use mlua::{Lua, Value};
use std::fs;
use std::fs::read_dir;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

// Simple color helpers using ANSI escapes (runtime switchable)
#[derive(Clone, Copy, Debug)]
struct Colorize(bool);
impl Colorize {
    fn green(&self, s: &str) -> String { if self.0 { format!("\x1b[32m{}\x1b[0m", s) } else { s.to_string() } }
    fn red(&self, s: &str) -> String { if self.0 { format!("\x1b[31m{}\x1b[0m", s) } else { s.to_string() } }
    fn blue(&self, s: &str) -> String { if self.0 { format!("\x1b[34m{}\x1b[0m", s) } else { s.to_string() } }
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
}

fn lua_decision(lua: &Lua, lua_file: &Path) -> Result<LuaDecision> {
    let src = fs::read_to_string(lua_file)
        .with_context(|| format!("Failed to read Lua file: {}", lua_file.display()))?;
    let chunk = lua.load(&src).set_name(lua_file.to_string_lossy());
    let value = chunk
        .eval::<Value>()
        .map_err(|e| anyhow::anyhow!("Failed to execute Lua chunk: {}", e))?;
    match value {
        Value::Boolean(b) => Ok(LuaDecision { include: b, rename_to: None }),
        Value::Table(t) => {
            // read optional rename_to
            let rt: Option<String> = t.get("rename_to").map_err(|e| anyhow::anyhow!("Invalid rename_to: {}", e))?;
            if let Some(name) = &rt {
                // Validate: must not contain path separators
                if name.contains('/') || name.contains('\\') {
                    bail!("rename_to must be a file name without path separators: {}", name);
                }
                if name.is_empty() {
                    bail!("rename_to must not be empty");
                }
            }
            Ok(LuaDecision { include: true, rename_to: rt })
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
    color: Colorize,
}

fn process(root: &Path, opts: Options) -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME must be set")?);
    let lua = Lua::new();

    fn walk_dir(root: &Path, rel: &Path, home: &Path, lua: &Lua, opts: Options) -> Result<()> {
        let mut planned: usize = 0;
        let mut conflicts: usize = 0;
        let mut skips: usize = 0;
        for entry in read_dir(root.join(rel)).with_context(|| format!("Failed to read dir {}", root.join(rel).display()))? {
            let entry = entry?;
            let path = entry.path();
            let rel_path = rel.join(entry.file_name());
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if file_name_str.ends_with(".lua") {
                // skip companion files
                continue;
            }

            if path.is_dir() {
                // Recurse into directories
                walk_dir(root, &rel_path, home, lua, opts)?;
                continue;
            }

            // Only symlink actual files
            if path.is_file() {
                // Build companion path by appending .lua to the filename
                let companion = path.with_extension(format!(
                    "{}lua",
                    path.extension()
                        .map(|e| format!("{}.", e.to_string_lossy()))
                        .unwrap_or_else(|| "".to_string())
                ));

                let mut include = true;
                if companion.exists() {
                    let decision = lua_decision(lua, &companion)?;
                    if let Some(new_name) = decision.rename_to {
                        // Warn if rename_to equals original filename
                        if rel_path.file_name().and_then(|s| s.to_str()).map(|s| s == new_name).unwrap_or(false) && opts.dry_run {
                            println!("{} {}", opts.color.blue("ℹ".into()), format!("rename_to is same as original for {}", rel_path.display()));
                        }
                        // adjust target relative path filename
                        let new_rel = rel_path.with_file_name(new_name);
                        let target = home.join(&new_rel);
                        // Always create parent directories
                        if let Some(parent) = target.parent() {
                            if !opts.dry_run {
                                fs::create_dir_all(parent).with_context(|| format!(
                                    "Failed to create parent directories for {}",
                                    target.display()
                                ))?;
                            }
                        }
                        if target.exists() || target.is_symlink() {
                            if opts.dry_run {
                                println!("{} {}", opts.color.red("✗"), format!("Conflict: target exists {} (source: {})", target.display(), path.display()));
                                conflicts += 1;
                            } else {
                                bail!("Target already exists: {}", target.display());
                            }
                            continue;
                        }
                        if opts.dry_run {
                            println!("{} {}", opts.color.green("✔"), format!("Would symlink {} -> {}", target.display(), path.display()));
                            planned += 1;
                        } else {
                            unix_fs::symlink(&path, &target).with_context(|| {
                                format!("Failed to symlink {} -> {}", target.display(), path.display())
                            })?;
                        }
                        continue;
                    }
                    include = decision.include; // table implies include=true by default
                }
                if !include {
                    if opts.dry_run { println!("{} {}", opts.color.blue("ℹ".into()), format!("Skipped by lua: {}", rel_path.display())); }
                    skips += 1;
                    continue;
                }

                let target = home.join(&rel_path);

                // Always create parent directories
                if let Some(parent) = target.parent() {
                    if !opts.dry_run {
                        fs::create_dir_all(parent).with_context(|| format!(
                            "Failed to create parent directories for {}",
                            target.display()
                        ))?;
                    }
                }

                // Report if target already exists
                if target.exists() || target.is_symlink() {
                    if opts.dry_run {
                        println!("{} {}", opts.color.red("✗"), format!("Conflict: target exists {} (source: {})", target.display(), path.display()));
                        conflicts += 1;
                        continue;
                    } else {
                        bail!("Target already exists: {}", target.display());
                    }
                }

                if opts.dry_run {
                    println!("{} {}", opts.color.green("✔"), format!("Would symlink {} -> {}", target.display(), path.display()));
                    planned += 1;
                } else {
                    unix_fs::symlink(&path, &target).with_context(|| {
                        format!("Failed to symlink {} -> {}", target.display(), path.display())
                    })?;
                }
            }
        }
        if opts.dry_run && rel.as_os_str().is_empty() {
            let conflicts_label = if conflicts == 1 { "conflict" } else { "conflicts" };
            let planned_label = if planned == 1 { "planned" } else { "planned" }; // same word reads fine
            let skipped_label = if skips == 1 { "skipped by lua" } else { "skipped by lua" }; // keep phrase
            println!(
                "\nSummary: {} {}, {} {}, {} {}",
                opts.color.green(&planned.to_string()), planned_label,
                opts.color.red(&conflicts.to_string()), conflicts_label,
                opts.color.blue(&skips.to_string()), skipped_label
            );
        }
        Ok(())
    }

    walk_dir(root, Path::new(""), &home, &lua, opts)
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
    let opts = Options { dry_run: cli.dry_run, color };
    process(&root_path, opts)
}
