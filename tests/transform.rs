use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn dry_run_transform_plans_write() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("config.txt"), b"email = old @example.com").unwrap();
    let lua_script = r#"
        return {
            transform = function(content)
                return content:gsub("old @example.com", "new @example.com")
            end
        }
    "#;
    fs::write(root.join("config.txt.lua"), lua_script).unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root")
        .arg(&root)
        .arg("--dry-run")
        .arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Would write transformed file"));
}

#[test]
fn actual_run_transform_writes_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("config.txt"), b"email = old @example.com").unwrap();
    let lua_script = r#"
        return {
            transform = function(content)
                return content:gsub("old @example.com", "new @example.com")
            end
        }
    "#;
    fs::write(root.join("config.txt.lua"), lua_script).unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root").arg(&root).arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Wrote transformed file"));

    let target_path = home.join("config.txt");
    assert!(target_path.is_file());
    assert!(
        !target_path.is_symlink(),
        "Target should be a regular file, not a symlink"
    );
    let content = fs::read_to_string(target_path).unwrap();
    assert_eq!(content, "email = new @example.com");
}

#[test]
fn transform_with_rename_to() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("config.txt"), b"old @example.com").unwrap();
    let lua_script = r#"
        return {
            rename_to = ".config_renamed",
            transform = function(content)
                return content:gsub("old", "new")
            end
        }
    "#;
    fs::write(root.join("config.txt.lua"), lua_script).unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root").arg(&root).arg("--no-color");
    cmd.env("HOME", &home);
    cmd.assert().success();

    let target_path = home.join(".config_renamed");
    assert!(target_path.is_file());
    let content = fs::read_to_string(target_path).unwrap();
    assert_eq!(content, "new @example.com");
}

#[test]
fn transform_overwrites_existing_different_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("config.txt"), b"old").unwrap();
    let lua_script = r#"
        return { transform = function(content) return "new" end }
    "#;
    fs::write(root.join("config.txt.lua"), lua_script).unwrap();

    let target_path = home.join("config.txt");
    fs::write(&target_path, b"pre-existing").unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root").arg(&root).arg("--no-color");
    cmd.env("HOME", &home);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Overwrote transformed file"));

    let content = fs::read_to_string(target_path).unwrap();
    assert_eq!(content, "new");
}

#[test]
fn transform_skips_writing_identical_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("config.txt"), b"old").unwrap();
    let lua_script = r#"
        return { transform = function(content) return "new" end }
    "#;
    fs::write(root.join("config.txt.lua"), lua_script).unwrap();

    let target_path = home.join("config.txt");
    fs::write(&target_path, b"new").unwrap();

    let mut cmd_dry = Command::cargo_bin("dotty").unwrap();
    cmd_dry
        .arg("--root")
        .arg(&root)
        .arg("--dry-run")
        .arg("--no-color");
    cmd_dry.env("HOME", &home);

    cmd_dry
        .assert()
        .success()
        .stdout(predicate::str::contains("Would write (already in place)"));
}

#[test]
fn transform_without_rename_to_uses_original_name() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let home = tmp.path().join("home");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(root.join("original_name.txt"), b"input").unwrap();
    let lua_script = r#"
        return {
            transform = function(content)
                return content:gsub("in", "out")
            end
        }
    "#;
    fs::write(root.join("original_name.txt.lua"), lua_script).unwrap();

    let mut cmd = Command::cargo_bin("dotty").unwrap();
    cmd.arg("--root").arg(&root).arg("--no-color");
    cmd.env("HOME", &home);
    cmd.assert().success();

    let target_path = home.join("original_name.txt");
    assert!(target_path.is_file());
    let content = fs::read_to_string(target_path).unwrap();
    assert_eq!(content, "output");
}
