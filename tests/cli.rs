use std::process::Command;

#[test]
fn help_describes_the_unified_executable() {
    let help = Command::new(env!("CARGO_BIN_EXE_chip8"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let usage = String::from_utf8_lossy(&help.stdout);
    assert!(usage.contains("chip8 agent"), "{usage}");
    for removed in ["--rpc", "--headless", "--port"] {
        assert!(!usage.contains(removed), "{usage}");
    }
}

#[test]
fn removed_controller_options_are_unknown() {
    let binary = env!("CARGO_BIN_EXE_chip8");
    for args in [&["--rpc"][..], &["--headless"], &["--port", "8787"]] {
        let output = Command::new(binary).args(args).output().unwrap();
        assert!(!output.status.success(), "{args:?} should fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("unknown option"), "{args:?}: {stderr}");
    }
}
