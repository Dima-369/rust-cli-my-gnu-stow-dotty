use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn dry_run_rename_to_plans_symlink_with_new_name() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    // original file and companion returning rename_to
    fs::write(root.join("orig.txt"), b"content").unwrap();
    fs::write(
        root.join("orig.txt.lua"),
        b"return { rename_to = 'renamed.txt' }",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(
            predicate::str::contains("Would symlink").and(
                predicate::str::contains("/home/renamed.txt")
                    .or(predicate::str::contains("renamed.txt")),
            ),
        );
}
