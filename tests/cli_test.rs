use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn cli_prints_help() {
    Command::cargo_bin("tempcheck")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Monitor system temperatures"));
}

#[test]
fn daemon_subcommand_help() {
    Command::cargo_bin("tempcheck")
        .unwrap()
        .arg("daemon")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("interval-secs"));
}

#[test]
fn mcp_subcommand_help() {
    Command::cargo_bin("tempcheck")
        .unwrap()
        .arg("mcp")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("audit-log"));
}

#[test]
fn once_subcommand_help() {
    Command::cargo_bin("tempcheck")
        .unwrap()
        .arg("once")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("save"));
}
