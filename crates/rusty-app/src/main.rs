use tracing_subscriber::EnvFilter;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("--version") | Some("-V") => {
            println!("rusty {VERSION}");
            return Ok(());
        }
        Some(flag) if flag.starts_with('-') => {
            eprintln!("Unknown flag: {flag}");
            eprintln!("Usage: rusty [--version]");
            std::process::exit(1);
        }
        _ => {}
    }

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("rusty starting");

    let config = rusty_config::Config::load();
    let shell  = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    rusty_ui::window::TerminalWindow::run(&shell, config);

    Ok(())
}
