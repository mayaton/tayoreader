use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

#[test]
fn prints_version() {
    cargo_bin_cmd!("tayoreader")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(concat!(
            "tayoreader ",
            env!("CARGO_PKG_VERSION")
        )));
}
