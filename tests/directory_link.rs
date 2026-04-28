use assert_cmd::Command;
use predicates::str::contains;
use std::fs;
use std::os::unix::fs as unix_fs;
use tempfile::TempDir;

#[test]
fn dry_run_directory_link_plans_symlink() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join("mydir/subfile")).unwrap();
    fs::create_dir_all(&home).unwrap();

    // Create a directory with contents
    fs::write(root.join("mydir/a.txt"), b"A").unwrap();
    fs::write(root.join("mydir/subfile/b.txt"), b"B").unwrap();

    // Companion lua that says link the whole directory
    fs::write(root.join("mydir.lua"), b"return { link = true }").unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("Would symlink dir"))
        .stdout(contains("Summary: 1 planned, 0 conflicts"));
}

#[test]
fn actual_run_directory_link_creates_symlink() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join("mydir")).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("mydir/a.txt"), b"A").unwrap();
    fs::write(root.join("mydir.lua"), b"return { link = true }").unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("Linked dir"));

    // Verify the symlink was created
    let target = home.join("mydir");
    assert!(target.is_symlink());
    assert_eq!(fs::read_link(&target).unwrap(), root.join("mydir"));
}

#[test]
fn directory_link_already_in_place() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join("mydir")).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("mydir/a.txt"), b"A").unwrap();
    fs::write(root.join("mydir.lua"), b"return { link = true }").unwrap();

    // Pre-create the correct symlink
    unix_fs::symlink(root.join("mydir"), home.join("mydir")).unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--verbose")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("Would link dir (already in place)"))
        .stdout(contains("Summary: 1 planned, 0 conflicts"));
}

#[test]
fn directory_link_conflict_when_target_exists() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join("mydir")).unwrap();
    fs::create_dir_all(home.join("mydir")).unwrap(); // real directory exists

    fs::write(root.join("mydir/a.txt"), b"A").unwrap();
    fs::write(root.join("mydir.lua"), b"return { link = true }").unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("exists"))
        .stdout(contains("Summary: 0 planned, 1 conflict"));
}

#[test]
fn directory_link_with_rename() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join("mydir")).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("mydir/a.txt"), b"A").unwrap();
    fs::write(
        root.join("mydir.lua"),
        b"return { link = true, rename_to = '.my-hidden-dir' }",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("Linked dir"));

    let target = home.join(".my-hidden-dir");
    assert!(target.is_symlink());
    assert_eq!(fs::read_link(&target).unwrap(), root.join("mydir"));
}

#[test]
fn directory_skipped_by_lua_returning_false() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join("mydir")).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("mydir/a.txt"), b"A").unwrap();
    fs::write(root.join("mydir.lua"), b"return false").unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("Skipped by lua"))
        .stdout(contains("Summary: 0 planned, 0 conflicts, 1 skipped by lua"));
}

#[test]
fn directory_without_lua_recurses_normally() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(root.join("mydir")).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("mydir/a.txt"), b"A").unwrap();
    // No companion .lua file — should recurse and link individual files

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(root.to_string_lossy().to_string())
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(contains("Would symlink"))
        .stdout(contains("Summary: 1 planned, 0 conflicts"));
}
