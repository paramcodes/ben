use std::{env, io, path::PathBuf};

use clap::{Parser, Subcommand};

#[derive(Debug, Clone, Parser)]
#[command(name = "ben", version, about = "A local-first terminal coding agent")]
pub struct Cli {
    /// Workspace directory to work in (defaults to the current directory).
    ///
    /// This is an option rather than a positional argument so it can be
    /// combined with a session command: `ben sessions list -C <WORKSPACE>`.
    #[arg(short = 'C', long, value_name = "WORKSPACE", global = true)]
    directory: Option<PathBuf>,

    /// Model identifier to use.
    #[arg(long, value_name = "MODEL", value_parser = parse_model, global = true)]
    model: Option<String>,

    /// Manage conversations saved on this machine.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// List, resume, or clear a stored conversation.
    Sessions {
        #[command(subcommand)]
        action: SessionCommand,
    },
}

/// The parsed session subcommands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum SessionCommand {
    /// Print the identifiers of saved sessions.
    List,
    /// Continue the conversation stored under an identifier.
    Resume {
        /// Identifier reported by `ben sessions list`.
        #[arg(value_name = "ID", value_parser = parse_session_id)]
        id: String,
    },
    /// Delete the session stored under an identifier, after confirming in the
    /// terminal interface.
    Clear {
        /// Identifier reported by `ben sessions list`.
        #[arg(value_name = "ID", value_parser = parse_session_id)]
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupOptions {
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub session: SessionAction,
}

/// What the process should do before it hands control to the terminal
/// interface.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SessionAction {
    /// Start an empty conversation.
    #[default]
    New,
    /// Print stored session identifiers and exit without rendering.
    List,
    /// Continue a stored conversation.
    Resume { id: String },
    /// Delete a stored conversation once the user confirms it.
    Clear { id: String },
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
        let selected = cli.directory;
        let workspace = match selected {
            None => current_dir,
            Some(workspace) if workspace.is_absolute() => workspace,
            Some(workspace) => current_dir.join(workspace),
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
        let session = match cli.command {
            None => SessionAction::New,
            Some(Command::Sessions { action }) => match action {
                SessionCommand::List => SessionAction::List,
                SessionCommand::Resume { id } => SessionAction::Resume { id },
                SessionCommand::Clear { id } => SessionAction::Clear { id },
            },
        };

        Ok(Self {
            workspace,
            model,
            session,
        })
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

/// Session ids become file names, so an empty one is refused at the boundary
/// rather than reaching the store. Unsafe names are refused by the store.
fn parse_session_id(value: &str) -> Result<String, String> {
    let id = value.trim();
    if id.is_empty() {
        Err("session identifier cannot be empty".to_owned())
    } else {
        Ok(id.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, SessionAction, StartupOptions};
    use clap::Parser;
    use std::path::PathBuf;

    fn options(args: &[&str]) -> StartupOptions {
        let cli = Cli::try_parse_from(args).expect("arguments should parse");
        StartupOptions::from_cli(cli, PathBuf::from("/tmp/workspace")).unwrap()
    }

    #[test]
    fn resolves_default_workspace_to_current_directory() {
        let options = options(&["ben"]);

        assert_eq!(options.workspace, PathBuf::from("/tmp/workspace"));
        assert_eq!(options.model, None);
        assert_eq!(options.session, SessionAction::New);
    }

    #[test]
    fn resolves_relative_workspace_to_absolute_path() {
        let options = options(&["ben", "-C", "projects/demo"]);

        assert_eq!(
            options.workspace,
            PathBuf::from("/tmp/workspace/projects/demo")
        );
    }

    #[test]
    fn keeps_absolute_workspace_and_trims_model_identifier() {
        let options = options(&["ben", "-C", "/workspace", "--model", "  model-x  "]);

        assert_eq!(options.workspace, PathBuf::from("/workspace"));
        assert_eq!(options.model.as_deref(), Some("model-x"));
    }

    #[test]
    fn reads_the_session_list_command() {
        assert_eq!(
            options(&["ben", "sessions", "list"]).session,
            SessionAction::List
        );
    }

    #[test]
    fn reads_the_session_identifier_for_resume_and_clear() {
        assert_eq!(
            options(&["ben", "sessions", "resume", " 2026-10-04-notes "]).session,
            SessionAction::Resume {
                id: "2026-10-04-notes".into()
            }
        );
        assert_eq!(
            options(&["ben", "sessions", "clear", "old-draft"]).session,
            SessionAction::Clear {
                id: "old-draft".into()
            }
        );
    }

    #[test]
    fn keeps_the_workspace_when_a_session_command_is_used() {
        let options = options(&["ben", "sessions", "list", "-C", "/workspace"]);

        assert_eq!(options.workspace, PathBuf::from("/workspace"));
        assert_eq!(options.session, SessionAction::List);
    }

    #[test]
    fn a_bare_path_is_not_treated_as_a_workspace() {
        // A positional workspace would shadow the subcommand name, so the
        // workspace is only ever an option.
        assert!(
            Cli::try_parse_from(["ben", "projects/demo"]).is_err(),
            "a bare path must not be accepted as a workspace"
        );
    }

    #[test]
    fn refuses_an_empty_session_identifier() {
        for command in ["resume", "clear"] {
            assert!(
                Cli::try_parse_from(["ben", "sessions", command, "  "]).is_err(),
                "{command} must refuse an empty identifier"
            );
        }
    }
}
