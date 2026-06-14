use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show config file path
    Path,
    /// Print merged config
    Print,
    /// Validate config and show warnings
    Validate,
}

pub fn handle_config_action(action: &ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Path => {
            match crate::config::global_config_path() {
                Some(p) => println!("Global config:  {}", p.display()),
                None => println!("Global config:  (unable to determine)"),
            }
            println!(
                "Project config: {}",
                crate::config::project_config_path().display()
            );
            Ok(())
        }
        ConfigAction::Print => {
            let config = crate::config::Config::load()?;
            let toml_str = toml::to_string_pretty(&config)?;
            println!("{}", toml_str);
            Ok(())
        }
        ConfigAction::Validate => {
            let config = crate::config::Config::load()?;
            let warnings = config.validate()?;
            if warnings.is_empty() {
                println!("Config: OK");
            } else {
                println!("Config: OK with warnings:");
                for w in &warnings {
                    println!("  WARN: {}", w);
                }
            }
            Ok(())
        }
    }
}
