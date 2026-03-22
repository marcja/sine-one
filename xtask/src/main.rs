use std::process::Command;

const PLUGIN_NAME: &str = "sine_one";
const BUNDLE: &str = "target/bundled/SineOne.clap";

fn main() -> nih_plug_xtask::Result<()> {
    // Intercept "deploy" subcommand; delegate everything else to nih-plug's bundler.
    if std::env::args().nth(1).as_deref() == Some("deploy") {
        deploy();
        return Ok(());
    }
    nih_plug_xtask::main()
}

/// Build, validate, and install the CLAP bundle.
///
/// Equivalent to the old deploy.sh:
///   1. cargo xtask bundle sine_one --release
///   2. clap-validator validate target/bundled/SineOne.clap --only-failed
///   3. cp -r bundle → ~/Library/Audio/Plug-Ins/CLAP/
fn deploy() {
    // NOTE: macOS-specific CLAP plugin path. Linux would use ~/.clap.
    let clap_dir = format!(
        "{}/Library/Audio/Plug-Ins/CLAP",
        std::env::var("HOME").expect("HOME not set")
    );

    run("cargo", &["xtask", "bundle", PLUGIN_NAME, "--release"]);
    run("clap-validator", &["validate", BUNDLE, "--only-failed"]);
    run("cp", &["-r", BUNDLE, &clap_dir]);

    println!("Installed to {clap_dir}/SineOne.clap");
    println!("→ Rescan plugins in Bitwig: Preferences > Plug-ins > Rescan");
}

/// Run a command, printing it before execution. Exits on failure.
fn run(program: &str, args: &[&str]) {
    println!("→ {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {program}: {e}"));
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}
