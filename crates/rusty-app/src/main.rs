use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("rusty starting");

    let config = rusty_config::Config::load();
    let shell  = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    rusty_ui::window::TerminalWindow::run(&shell, config);

    Ok(())
}
