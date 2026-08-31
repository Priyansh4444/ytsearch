use std::process::Command;

#[test]
fn help_is_a_successful_image_smoke_check() {
    for argument in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_youtube-raw-search"))
            .arg(argument)
            .output()
            .expect("Raw search binary should run");

        assert!(
            output.status.success(),
            "{argument} should exit successfully"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("usage: youtube-raw-search "),
            "{argument} should print usage to stdout"
        );
        assert!(
            output.stderr.is_empty(),
            "{argument} should not log an error"
        );
    }
}
