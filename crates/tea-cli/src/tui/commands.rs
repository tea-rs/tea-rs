use std::collections::BTreeSet;
use std::str::FromStr as _;

use tea_mcp::McpServerId;
use tea_protocol::{MessageId, ModelId, ReasoningEffort, SessionId};

use super::attachment::MAX_COMPOSER_ATTACHMENTS;

const BUILTINS: [&str; 14] = [
    "compact",
    "copy",
    "fork",
    "help",
    "image",
    "mcp",
    "model",
    "name",
    "new",
    "quit",
    "resume",
    "reasoning",
    "session",
    "tree",
];
const MAX_COMMANDS: usize = 512;
const MAX_IMAGE_PATH_BYTES: usize = 4096;

/// Parsed interactive slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// Create and switch to a new session.
    New,
    /// Resume an explicit session or open the session selector.
    Resume(Option<SessionId>),
    /// Open the session selector.
    Session,
    /// Set or clear a session display name.
    Name(Option<String>),
    /// Set a model or open the model selector.
    Model(Option<ModelId>),
    /// Set reasoning effort or open the reasoning selector.
    Reasoning(Option<ReasoningEffort>),
    /// Compact the active session.
    Compact,
    /// Open the branch tree selector.
    Tree,
    /// Fork from one durable message.
    Fork(MessageId),
    /// Copy the last assistant response.
    Copy,
    /// Load one explicit local image path.
    Image(String),
    /// Remove one image by its one-based composer index.
    ImageRemove(usize),
    /// Remove every image from the active session composer.
    ImageClear,
    /// Display safe MCP server health and frozen aliases.
    Mcp,
    /// Reconnect one MCP server only against its frozen discovery snapshot.
    McpReconnect(McpServerId),
    /// Show command help.
    Help,
    /// Exit the application.
    Quit,
    /// Expand one trusted declarative prompt template.
    Template {
        /// Canonical template name.
        name: String,
        /// Positional template arguments.
        arguments: Vec<String>,
    },
    /// Load one explicit trusted skill invocation.
    Skill(String),
}

/// Invalid command catalog or invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// The command name, arguments, or identifier is invalid.
    #[error("slash command is invalid")]
    Invalid,
    /// A declarative resource conflicts with another command.
    #[error("slash command catalog has a duplicate")]
    Duplicate,
    /// The command catalog exceeds its fixed bound.
    #[error("slash command catalog is too large")]
    TooMany,
}

/// Deterministic built-in and declarative slash command catalog.
#[derive(Debug, Clone)]
pub struct CommandCatalog {
    templates: BTreeSet<String>,
    skills: BTreeSet<String>,
    completions: Vec<String>,
}

impl CommandCatalog {
    /// Builds a bounded catalog from trusted resource names.
    ///
    /// # Errors
    ///
    /// Rejects invalid, duplicate, conflicting, or oversized names.
    pub fn new<T, S, TI, SI>(templates: TI, skills: SI) -> Result<Self, CommandError>
    where
        T: AsRef<str>,
        S: AsRef<str>,
        TI: IntoIterator<Item = T>,
        SI: IntoIterator<Item = S>,
    {
        let templates = collect_names(templates)?;
        let skills = collect_names(skills)?;
        if templates
            .iter()
            .any(|name| BUILTINS.contains(&name.as_str()))
        {
            return Err(CommandError::Duplicate);
        }
        if templates.len().saturating_add(skills.len()) + BUILTINS.len() > MAX_COMMANDS {
            return Err(CommandError::TooMany);
        }
        let mut completions = BUILTINS
            .into_iter()
            .map(|name| format!("/{name}"))
            .chain(templates.iter().map(|name| format!("/{name}")))
            .chain(skills.iter().map(|name| format!("/skill:{name}")))
            .collect::<Vec<_>>();
        completions.sort();
        Ok(Self {
            templates,
            skills,
            completions,
        })
    }

    /// Parses one complete command line.
    ///
    /// # Errors
    ///
    /// Rejects unknown commands, malformed identifiers, or extra arguments.
    pub fn parse(&self, input: &str) -> Result<SlashCommand, CommandError> {
        if input.is_empty() || input.contains('\0') || input.len() > crate::tui::MAX_EDITOR_BYTES {
            return Err(CommandError::Invalid);
        }
        if let Some(skill) = input.strip_prefix("/skill:") {
            let name = skill
                .split_whitespace()
                .next()
                .ok_or(CommandError::Invalid)?;
            if self.skills.contains(name) {
                return Ok(SlashCommand::Skill(input.to_owned()));
            }
            return Err(CommandError::Invalid);
        }
        if input == "/image" || input.starts_with("/image ") {
            return parse_image(input);
        }
        let mut parts = input.split_whitespace();
        let command = parts
            .next()
            .and_then(|value| value.strip_prefix('/'))
            .ok_or(CommandError::Invalid)?;
        let arguments = parts.map(str::to_owned).collect::<Vec<_>>();
        match command {
            "new" if arguments.is_empty() => Ok(SlashCommand::New),
            "resume" if arguments.len() <= 1 => Ok(SlashCommand::Resume(
                arguments
                    .first()
                    .map(|value| SessionId::from_str(value))
                    .transpose()
                    .map_err(|_| CommandError::Invalid)?,
            )),
            "session" if arguments.is_empty() => Ok(SlashCommand::Session),
            "name" => Ok(SlashCommand::Name(
                (!arguments.is_empty()).then(|| arguments.join(" ")),
            )),
            "model" if arguments.len() <= 1 => Ok(SlashCommand::Model(
                arguments
                    .first()
                    .map(|value| ModelId::from_str(value))
                    .transpose()
                    .map_err(|_| CommandError::Invalid)?,
            )),
            "reasoning" if arguments.len() <= 1 => Ok(SlashCommand::Reasoning(
                arguments
                    .first()
                    .map(|value| ReasoningEffort::from_str(value))
                    .transpose()
                    .map_err(|_| CommandError::Invalid)?,
            )),
            "compact" if arguments.is_empty() => Ok(SlashCommand::Compact),
            "tree" if arguments.is_empty() => Ok(SlashCommand::Tree),
            "fork" if arguments.len() == 1 => MessageId::from_str(&arguments[0])
                .map(SlashCommand::Fork)
                .map_err(|_| CommandError::Invalid),
            "copy" if arguments.is_empty() => Ok(SlashCommand::Copy),
            "mcp" if arguments.is_empty() => Ok(SlashCommand::Mcp),
            "mcp" if arguments.len() == 2 && arguments[0] == "reconnect" => {
                McpServerId::from_str(&arguments[1])
                    .map(SlashCommand::McpReconnect)
                    .map_err(|_| CommandError::Invalid)
            }
            "help" if arguments.is_empty() => Ok(SlashCommand::Help),
            "quit" if arguments.is_empty() => Ok(SlashCommand::Quit),
            name if self.templates.contains(name) => Ok(SlashCommand::Template {
                name: name.to_owned(),
                arguments,
            }),
            _ => Err(CommandError::Invalid),
        }
    }

    /// Returns sorted prefix completions up to the caller's bound.
    #[must_use]
    pub fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.completions
            .iter()
            .filter(|command| command.starts_with(prefix))
            .take(limit)
            .cloned()
            .collect()
    }
}

fn parse_image(input: &str) -> Result<SlashCommand, CommandError> {
    let argument = input
        .strip_prefix("/image ")
        .map(str::trim)
        .filter(|argument| !argument.is_empty())
        .ok_or(CommandError::Invalid)?;
    if argument.len() > MAX_IMAGE_PATH_BYTES || argument.chars().any(char::is_control) {
        return Err(CommandError::Invalid);
    }

    let mut parts = argument.split_whitespace();
    match parts.next() {
        Some("clear") if parts.next().is_none() => Ok(SlashCommand::ImageClear),
        Some("clear") | None => Err(CommandError::Invalid),
        Some("remove") => {
            let index = parts
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|index| (1..=MAX_COMPOSER_ATTACHMENTS).contains(index))
                .ok_or(CommandError::Invalid)?;
            if parts.next().is_some() {
                return Err(CommandError::Invalid);
            }
            Ok(SlashCommand::ImageRemove(index))
        }
        Some(_) => Ok(SlashCommand::Image(argument.to_owned())),
    }
}

fn collect_names<T, I>(values: I) -> Result<BTreeSet<String>, CommandError>
where
    T: AsRef<str>,
    I: IntoIterator<Item = T>,
{
    let mut names = BTreeSet::new();
    for value in values {
        let value = value.as_ref();
        if !valid_name(value) {
            return Err(CommandError::Invalid);
        }
        if !names.insert(value.to_owned()) {
            return Err(CommandError::Duplicate);
        }
        if names.len() > MAX_COMMANDS {
            return Err(CommandError::TooMany);
        }
    }
    Ok(names)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
