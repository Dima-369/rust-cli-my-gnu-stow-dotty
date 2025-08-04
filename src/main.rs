use anyhow::{Context, Result};
use clap::Parser;
use mlua::{Lua, Value};
use std::fs;
use std::path::PathBuf;

/// Simple CLI to run a Lua file and print a returned string
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to the Lua file to execute
    path: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Read the Lua source from file
    let src = fs::read_to_string(&args.path)
        .with_context(|| format!("Failed to read Lua file: {}", args.path.display()))?;

    // Create a Lua state
    let lua = Lua::new();

    // Load the chunk; we expect the chunk to evaluate to a string, or return a string
    let chunk = lua.load(&src).set_name(args.path.to_string_lossy());

    let value = chunk
        .eval::<Value>()
        .map_err(|e| anyhow::anyhow!("Failed to execute Lua chunk: {}", e))?;

    // Support two common patterns:
    // 1) The chunk evaluates to a string literal / expression => Value::String
    // 2) The chunk returns a string (e.g., 'return "hello"') => same as above
    // For other types, produce an error
    match value {
        Value::String(s) => {
            println!("{}", s.to_string_lossy());
            Ok(())
        }
        other => Err(anyhow::anyhow!(
            "Lua chunk did not produce a string; got {:?}",
            other.type_name()
        )),
    }
}
