use std::{env, io, path::PathBuf};

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(name = "ben", version, about = "A local-first terminal coding agent")]
pub struct Cli {
    /// Workspace directory to work in (defaults to the current directory).
    #[arg(value_name = "WORKSPACE", default_value = ".")]
    workspace: PathBuf,

    /// Model identifier to use.
    #[arg(long, value_name = "MODEL", value_parser = parse_model)]
    model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupOptions {
    pub workspace: PathBuf,
    pub model: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("model identifier cannot be empty")]
    EmptyModel,
    #[error("could not determine the current directory")]
    CurrentDirectory(#[from] io::Error),
}

impl StartupOptions {
    fn from_cli(cli: Cli, current_dir: PathBuf) -> Result<Self, CliError> {
        let workspace = if cli.workspace.is_absolute() {
            cli.workspace
        } else {
            current_dir.join(cli.workspace)
        };
        let model = cli
            .model
            .map(|model| model.trim().to_owned())
            .map(|model| {
                if model.is_empty() {
                    Err(CliError::EmptyModel)
                } else {
                    Ok(model)
                }
            })
            .transpose()?;

        Ok(Self { workspace, model })
    }
}

pub fn parse() -> Result<StartupOptions, CliError> {
    let cli = Cli::parse();
    StartupOptions::from_cli(cli, env::current_dir()?)
}

fn parse_model(value: &str) -> Result<String, String> {
    let model = value.trim();
    if model.is_empty() {
        Err("model identifier cannot be empty".to_owned())
    } else {
        Ok(model.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, StartupOptions};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn resolves_default_workspace_to_current_directory() {
        let cli = Cli::try_parse_from(["ben"]).unwrap();
        let current_dir = PathBuf::from("/tmp/workspace");

        let options = StartupOptions::from_cli(cli, current_dir.clone()).unwrap();

        assert_eq!(options.workspace, current_dir);
        assert_eq!(options.model, None);
    }

    #[test]
    fn resolves_relative_workspace_to_absolute_path() {
        let cli = Cli::try_parse_from(["ben", "projects/demo"]).unwrap();

        let options = StartupOptions::from_cli(cli, PathBuf::from("/home/user")).unwrap();

        assert_eq!(options.workspace, PathBuf::from("/home/user/projects/demo"));
    }

    #[test]
    fn keeps_absolute_workspace_and_trims_model_identifier() {
        let cli = Cli::try_parse_from(["ben", "/workspace", "--model", "  model-x  "]).unwrap();

        let options = StartupOptions::from_cli(cli, PathBuf::from("/ignored")).unwrap();

        assert_eq!(options.workspace, PathBuf::from("/workspace"));
        assert_eq!(options.model.as_deref(), Some("model-x"));
    }
}
