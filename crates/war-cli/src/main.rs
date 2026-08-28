use clap::{Args, Parser, Subcommand};
use std::{
    fs::File,
    io::{self, BufReader},
    sync::Arc,
};
use war_protocol::{Action, ActionBatch, Modifiers, MouseButton, SnapshotScope, Target};
use war_runtime::WarRuntime;
use war_semantic::{render_delta, render_snapshot};
use war_uia::UiaProvider;

#[derive(Parser)]
#[command(name = "war", version, about = "Token-efficient Windows agent runtime")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Snapshot {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    Find {
        query: String,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    Act {
        #[command(subcommand)]
        action: ActCommand,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    Exec {
        file: std::path::PathBuf,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    Watch {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Serve Model Context Protocol over newline-delimited stdio.
    Mcp,
    Serve,
}

#[derive(Args, Default)]
struct ScopeArgs {
    #[arg(long, conflicts_with_all = ["process", "window", "focused_subtree"])]
    desktop: bool,
    #[arg(long, value_name = "PID", conflicts_with_all = ["desktop", "window", "focused_subtree"])]
    process: Option<u32>,
    #[arg(long, value_name = "HWND", conflicts_with_all = ["desktop", "process", "focused_subtree"])]
    window: Option<u64>,
    #[arg(long, conflicts_with_all = ["desktop", "process", "window"])]
    focused_subtree: bool,
}

impl ScopeArgs {
    fn resolve(self) -> SnapshotScope {
        if self.desktop {
            SnapshotScope::Desktop
        } else if let Some(process) = self.process {
            SnapshotScope::Process(process)
        } else if let Some(window) = self.window {
            SnapshotScope::Window(window)
        } else if self.focused_subtree {
            SnapshotScope::FocusedSubtree
        } else {
            SnapshotScope::FocusedWindow
        }
    }
}

#[derive(Subcommand)]
enum ActCommand {
    Invoke {
        target: String,
    },
    SetValue {
        target: String,
        value: String,
    },
    Toggle {
        target: String,
        value: Option<bool>,
    },
    Select {
        target: String,
    },
    Focus {
        target: String,
    },
    Click {
        target: String,
        #[arg(long, default_value = "left")]
        button: String,
    },
    TypeText {
        text: String,
    },
    Key {
        key: String,
        #[arg(long)]
        ctrl: bool,
        #[arg(long)]
        alt: bool,
        #[arg(long)]
        shift: bool,
        #[arg(long)]
        meta: bool,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let runtime = WarRuntime::new(Arc::new(UiaProvider::new()?));
    match cli.command {
        Command::Snapshot { json, scope } => {
            let snapshot = runtime.observe(scope.resolve())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                print!("{}", render_snapshot(&snapshot));
            }
        }
        Command::Find { query, json, scope } => {
            let snapshot = runtime.observe(scope.resolve())?;
            let query_lower = query.to_lowercase();
            let matches: Vec<_> = snapshot
                .nodes
                .iter()
                .filter(|node| {
                    node.name
                        .as_ref()
                        .is_some_and(|name| name.to_lowercase().contains(&query_lower))
                })
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&matches)?);
            } else {
                for node in matches {
                    println!("{}", war_semantic::render_node(node));
                }
            }
        }
        Command::Act { action, scope } => {
            let snapshot = runtime.observe(scope.resolve())?;
            let batch = ActionBatch {
                expected_session_id: Some(snapshot.session_id.clone()),
                expected_epoch: Some(snapshot.epoch),
                timeout_ms: None,
                actions: vec![to_action(action)?],
                precondition: None,
                postcondition: None,
                stop_on_error: true,
            };
            let report = runtime.act(&batch)?;
            for outcome in report.outcome.actions {
                if outcome.dispatched {
                    println!(
                        "dispatched ({}){}",
                        outcome.method.unwrap_or_default(),
                        if outcome.fallback_used == Some(true) {
                            " [fallback]"
                        } else {
                            ""
                        }
                    );
                } else {
                    println!("failed: {}", outcome.error.unwrap_or_default());
                }
            }
            print!("{}", render_delta(&report.delta));
        }
        Command::Exec { file, json, scope } => {
            let snapshot = runtime.observe(scope.resolve())?;
            let value: serde_json::Value =
                serde_json::from_reader(BufReader::new(File::open(file)?))?;
            let mut batch: ActionBatch = serde_json::from_value(value.clone()).or_else(|_| {
                serde_json::from_value::<Vec<Action>>(value).map(|actions| ActionBatch {
                    expected_session_id: None,
                    expected_epoch: None,
                    timeout_ms: None,
                    actions,
                    precondition: None,
                    postcondition: None,
                    stop_on_error: true,
                })
            })?;
            if batch.expected_epoch.is_none() {
                batch.expected_epoch = Some(snapshot.epoch);
            }
            if batch.expected_session_id.is_none() {
                batch.expected_session_id = Some(snapshot.session_id.clone());
            }
            let report = runtime.act(&batch)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report.delta)?);
            } else {
                print!("{}", render_delta(&report.delta));
            }
        }
        Command::Watch { json, scope } => runtime.watch(scope.resolve(), |delta| {
            if json {
                match serde_json::to_string(&delta) {
                    Ok(line) => println!("{line}"),
                    Err(error) => {
                        eprintln!("error: failed to serialize delta: {error}");
                        return false;
                    }
                }
            } else {
                print!("{}", render_delta(&delta));
            }
            true
        })?,
        Command::Mcp => war_mcp::serve_stdio(&runtime, io::stdin().lock(), io::stdout().lock())?,
        Command::Serve => runtime.serve_jsonl(io::stdin().lock(), io::stdout().lock())?,
    }
    Ok(())
}

fn target(value: &str) -> Result<Target, String> {
    Target::parse_ref(value)
        .ok_or_else(|| format!("target must be a session ref such as @12, got {value}"))
}
fn to_action(action: ActCommand) -> Result<Action, String> {
    Ok(match action {
        ActCommand::Invoke { target: value } => Action::Invoke {
            target: target(&value)?,
        },
        ActCommand::SetValue {
            target: value,
            value: text,
        } => Action::SetValue {
            target: target(&value)?,
            value: text,
        },
        ActCommand::Toggle {
            target: target_ref,
            value,
        } => Action::Toggle {
            target: target(&target_ref)?,
            value,
        },
        ActCommand::Select { target: value } => Action::Select {
            target: target(&value)?,
        },
        ActCommand::Focus { target: value } => Action::Focus {
            target: target(&value)?,
        },
        ActCommand::Click {
            target: value,
            button,
        } => Action::Click {
            target: target(&value)?,
            button: match button.as_str() {
                "left" => MouseButton::Left,
                "right" => MouseButton::Right,
                "middle" => MouseButton::Middle,
                _ => return Err(format!("unknown mouse button {button}")),
            },
        },
        ActCommand::TypeText { text } => Action::TypeText { text },
        ActCommand::Key {
            key,
            ctrl,
            alt,
            shift,
            meta,
        } => Action::Key {
            key: parse_key(&key)?,
            modifiers: Modifiers {
                ctrl,
                alt,
                shift,
                meta,
            },
        },
    })
}

fn parse_key(value: &str) -> Result<war_protocol::Key, String> {
    use war_protocol::Key;
    let named = match value.to_ascii_lowercase().as_str() {
        "enter" => Some(Key::Enter),
        "escape" | "esc" => Some(Key::Escape),
        "tab" => Some(Key::Tab),
        "space" => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        _ => None,
    };
    if let Some(key) = named {
        return Ok(key);
    }
    let mut characters = value.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => Ok(Key::Character(character)),
        _ => Err(format!(
            "unknown key {value:?}; use a named key or one Unicode character"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_and_character_keys() {
        assert_eq!(parse_key("esc").unwrap(), war_protocol::Key::Escape);
        assert_eq!(parse_key("界").unwrap(), war_protocol::Key::Character('界'));
        assert!(parse_key("unknown-key").is_err());
    }
}
