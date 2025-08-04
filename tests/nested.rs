use assert_cmd::Command;
use predicates::str::contains;
use std::fs;
use tempfile::TempDir;

#[test]
fn nested_counts_aggregate_across_subdirs() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join(".config/kitty")).unwrap();
    fs::create_dir_all(&home).unwrap();

    // Files in top-level and nested dirs
    fs::write(root.join("a.txt"), b"A").unwrap(); // planned
    fs::write(root.join("b.txt"), b"B").unwrap(); // skipped by lua
    fs::write(root.join("b.txt.lua"), b"return false").unwrap();
    fs::write(root.join(".config/kitty/kitty.conf"), b"conf").unwrap(); // conflict

    // conflict in home for nested file
    fs::create_dir_all(home.join(".config/kitty")).unwrap();
    fs::write(home.join(".config/kitty/kitty.conf"), b"existing").unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        // Ensure all categories appear
        .stdout(contains("Would symlink"))
        .stdout(contains("exists "))
        .stdout(contains("Skipped by lua"))
        // Summary must count across subdirs: 1 planned, 1 conflict, 1 skipped by lua
        .stdout(contains("Summary: 1 planned, 1 conflict, 1 skipped by lua"));
}
