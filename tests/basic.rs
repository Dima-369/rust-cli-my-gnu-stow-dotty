use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn dry_run_reports_plans_conflicts_and_skips() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    // Create files: a.txt (allowed), b.txt (skipped by lua), c.txt (conflict)
    fs::write(root.join("a.txt"), b"A").unwrap();
    fs::write(root.join("b.txt"), b"B").unwrap();
    fs::write(root.join("b.txt.lua"), b"return false").unwrap();
    fs::write(root.join("c.txt"), b"C").unwrap();

    // Create conflict at home
    fs::write(home.join("c.txt"), b"existing").unwrap();

    // Build command
    let mut cmd = assert_cmd::Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root").arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");

    // Set HOME override for the test
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Would symlink"))
        .stdout(predicate::str::contains("Conflict: target exists"))
        .stdout(predicate::str::contains("Skipped by lua"))
        .stdout(predicate::str::contains("Summary: 1 planned, 1 conflicts, 1 skipped by lua"));
}
