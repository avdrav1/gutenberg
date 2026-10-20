fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("gutenberg {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    match gutenberg_ui::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gutenberg: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
