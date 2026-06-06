use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::Deserialize;

// ── TOML schema ───────────────────────────────────────────────────────────────

/// A single flag definition, e.g. `--verbose` / `-v`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FlagSpec {
    pub long: Option<String>,
    pub short: Option<String>,
    pub description: Option<String>,
    /// If true the flag takes a value: `--output=<file>` or `--output <file>`.
    #[serde(default)]
    pub takes_value: bool,
    /// Hint shown after `=` when `takes_value` is true (e.g. "file", "dir", "number").
    pub value_hint: Option<String>,
}

/// A subcommand (e.g. `git add`, `brew install`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SubcommandSpec {
    pub description: Option<String>,
    #[serde(default)]
    pub flags: Vec<FlagSpec>,
    /// Sub-subcommands, if any.
    #[serde(default)]
    pub subcommands: Vec<SubcommandSpec>,
    pub name: Option<String>,
}

/// What kind of filesystem argument a command accepts.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ArgsType {
    /// Files and directories (default).
    #[default]
    Any,
    /// Directories only (e.g. cd, pushd, rmdir).
    Directory,
    /// Files only.
    File,
    /// No filesystem argument — don't offer path completions at all.
    None,
}

/// Top-level completion spec for one command.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CommandSpec {
    /// The command name this spec applies to (e.g. "grep").
    pub command: String,
    pub description: Option<String>,
    #[serde(default)]
    pub flags: Vec<FlagSpec>,
    #[serde(default)]
    pub subcommands: Vec<SubcommandSpec>,
    /// Controls what filesystem completions are offered for non-flag arguments.
    #[serde(default)]
    pub args: ArgsType,
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Loaded and merged completion specs, keyed by command name.
pub struct CompletionRegistry {
    specs: HashMap<String, CommandSpec>,
}

impl CompletionRegistry {
    pub fn load() -> Self {
        let mut registry = Self { specs: HashMap::new() };

        // 1. Bundled specs shipped with rusty (next to the binary / in the app bundle).
        if let Some(bundled) = bundled_dir() {
            registry.load_dir(&bundled);
        }

        // 2. User overrides — these win, so load second.
        if let Some(user) = user_completions_dir() {
            registry.load_dir(&user);
        }

        registry
    }

    fn load_dir(&mut self, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            match toml::from_str::<CommandSpec>(&text) {
                Ok(spec) => {
                    self.specs.insert(spec.command.clone(), spec);
                }
                Err(e) => {
                    tracing::warn!("completion spec {:?}: {e}", path);
                }
            }
        }
    }

    /// Look up the spec for `command`, if any.
    pub fn get(&self, command: &str) -> Option<&CommandSpec> {
        self.specs.get(command)
    }
}

// ── Directory resolution ──────────────────────────────────────────────────────

fn user_completions_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/rusty/completions"))
}

fn bundled_dir() -> Option<PathBuf> {
    // Allow the build / dev environment to override the path explicitly.
    if let Ok(p) = std::env::var("RUSTY_COMPLETIONS_DIR") {
        let pb = PathBuf::from(p);
        if pb.is_dir() { return Some(pb); }
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    // Release / app bundle locations.
    // Dev: binary is at target/debug/rusty, so ../.. reaches the workspace root.
    let candidates = [
        exe_dir.join("completions"),
        exe_dir.join("../share/rusty/completions"),
        exe_dir.join("../..").join("completions-toml"),   // target/debug → workspace root
        exe_dir.join("../../..").join("completions-toml"), // target/debug/deps → workspace root
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

// ── Flat suggestion list from a spec ─────────────────────────────────────────

/// A resolved suggestion ready for display, derived from a spec.
#[derive(Debug, Clone)]
pub struct SpecSuggestion {
    pub label: String,
    pub description: Option<String>,
}

/// Build flag suggestions for a command (and optionally a subcommand already typed).
pub fn flag_suggestions(spec: &CommandSpec, subcommand: Option<&str>) -> Vec<SpecSuggestion> {
    let flags = if let Some(sub) = subcommand {
        spec.subcommands.iter()
            .find(|s| s.name.as_deref() == Some(sub))
            .map(|s| s.flags.as_slice())
            .unwrap_or(&spec.flags)
    } else {
        &spec.flags
    };

    let mut out = Vec::new();
    for f in flags {
        if let Some(long) = &f.long {
            let label = if f.takes_value {
                format!("--{}=", long)
            } else {
                format!("--{}", long)
            };
            out.push(SpecSuggestion { label, description: f.description.clone() });
        }
        if let Some(short) = &f.short {
            out.push(SpecSuggestion {
                label: format!("-{}", short),
                description: f.description.clone(),
            });
        }
    }
    out
}

/// Subcommand suggestions for a command.
pub fn subcommand_suggestions(spec: &CommandSpec) -> Vec<SpecSuggestion> {
    spec.subcommands.iter()
        .filter_map(|s| s.name.as_ref().map(|n| SpecSuggestion {
            label: n.clone(),
            description: s.description.clone(),
        }))
        .collect()
}
