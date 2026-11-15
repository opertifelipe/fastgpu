use std::process::Command;
fn cli(dir: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fastgpu"));
    command
        .env("GPU_CONFIG_DIR", dir.join("profile"))
        .env("HOME", dir)
        .env_remove("RUNPOD_API_KEY");
    command
}
#[test]
fn version_help_and_config_without_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let output = cli(dir.path()).arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "fastgpu version 0.3.0\n"
    );
    for args in [
        vec!["--help"],
        vec!["help", "deploy"],
        vec!["volume", "--help"],
        vec!["worker", "--help"],
        vec!["config"],
        vec!["model"],
        vec!["status"],
    ] {
        let output = cli(dir.path()).args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!dir.path().join("profile").exists());
}
#[test]
fn errors_use_fastgpu_prefix_and_worker_rejects_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["unknown"],
        vec!["pods"],
        vec!["up", "--medium", "--large"],
        vec!["worker", "prepare", "unknown"],
        vec!["worker", "deploy"],
    ] {
        let output = cli(dir.path()).args(&args).output().unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("fastgpu:"));
    }
}
#[test]
fn malformed_profile_is_reported_without_dumping_contents() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("profile")).unwrap();
    std::fs::write(dir.path().join("profile/config.json"), "{invalid}").unwrap();
    let output = cli(dir.path()).arg("config").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("config.json"));
}
