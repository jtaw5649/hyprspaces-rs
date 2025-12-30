fn main() {
    if let Err(err) = hyprspaces::cli::run() {
        eprintln!("error: {}", err);
        std::process::exit(1);
    }
}
