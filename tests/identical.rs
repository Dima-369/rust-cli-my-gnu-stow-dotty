use assert_cmd::Command;
use predicates::str::contains;
use predicates::prelude::PredicateBooleanExt;
use std::fs;
use std::os::unix::fs as unix_fs;
use tempfile::TempDir;

#[test]
fn dry_run_conflict_identical_regular_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    // source file
    fs::write(root.join("a.txt"), b"SAME").unwrap();
    // existing target with identical content
    fs::write(home.join("a.txt"), b"SAME").unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("exists ").or(contains("Would link (already in place)")))
        .stdout(contains("identical").or(contains("Would link (already in place)")));
}

#[test]
fn dry_run_conflict_identical_symlink_points_to_source() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    // source file
    let src = root.join("b.txt");
    fs::write(&src, b"DATA").unwrap();
    // existing target is a symlink pointing to source
    let target = home.join("b.txt");
    unix_fs::symlink(&src, &target).unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("exists ").or(contains("Would link (already in place)")))
        .stdout(contains("identical").or(contains("Would link (already in place)")));
}
