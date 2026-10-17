use std::process::Command;

#[test]
fn binary_prints_its_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_gutenberg"))
        .arg("--version")
        .output()
        .expect("run gutenberg");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("gutenberg {}", env!("CARGO_PKG_VERSION"))
    );
}
