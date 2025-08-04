use anyhow::{bail, Context, Result};
use clap::Parser;
use mlua::{Lua, Value};
use std::fs;
use std::fs::read_dir;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

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

fn lua_should_include(lua: &Lua, lua_file: &Path) -> Result<bool> {
    let src = fs::read_to_string(lua_file)
        .with_context(|| format!("Failed to read Lua file: {}", lua_file.display()))?;
    let chunk = lua.load(&src).set_name(lua_file.to_string_lossy());
    let value = chunk
        .eval::<Value>()
        .map_err(|e| anyhow::anyhow!("Failed to execute Lua chunk: {}", e))?;
    match value {
        Value::Boolean(b) => Ok(b),
        other => bail!(
            "Lua filter did not return boolean for {}. Got {}",
            lua_file.display(),
            other.type_name()
        ),
    }
}

#[derive(Clone, Copy, Debug)]
struct Options {
    dry_run: bool,
}

fn process(root: &Path, opts: Options) -> Result<()> {
    let home = PathBuf::from(std::env::var("HOME").context("HOME must be set")?);
    let lua = Lua::new();

    fn walk_dir(root: &Path, rel: &Path, home: &Path, lua: &Lua, opts: Options) -> Result<()> {
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
                    include = lua_should_include(lua, &companion)?;
                }
                if !include {
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

                // Abort if target already exists
                if target.exists() || target.is_symlink() {
                    bail!("Target already exists: {}", target.display());
                }

                if opts.dry_run {
                    println!("Would symlink {} -> {}", target.display(), path.display());
                } else {
                    unix_fs::symlink(&path, &target).with_context(|| {
                        format!("Failed to symlink {} -> {}", target.display(), path.display())
                    })?;
                }
            }
        }
        Ok(())
    }

    walk_dir(root, Path::new(""), &home, &lua, opts)
}

fn main() -> Result<()> {
    #[derive(Parser, Debug)]
    #[command(author, version, about)]
    struct Cli {
        /// Root directory that contains dotfiles to stow
        #[arg(short, long, default_value = "~/Developer/dotfiles/dotty/")]
        root: String,
        /// Dry run: only print operations, do not modify filesystem
        #[arg(long)]
        dry_run: bool,
    }

    let cli = Cli::parse();
    let root_path = expand_tilde(&cli.root)?;
    if !root_path.is_dir() {
        bail!("Root directory is not a directory: {}", root_path.display());
    }
    let opts = Options { dry_run: cli.dry_run };
    process(&root_path, opts)
}
