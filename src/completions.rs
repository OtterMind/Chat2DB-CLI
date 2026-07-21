use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{CommandFactory, ValueEnum};
use clap_complete::generate;
use clap_complete::shells::{Bash, Fish, Zsh};
use serde::Serialize;

use crate::cli::Cli;
use crate::error::{Chat2DbError, Result};

const RC_MARKER_BEGIN: &str = "# >>> chat2db completions >>>";
const RC_MARKER_END: &str = "# <<< chat2db completions <<<";
const BIN_NAME: &str = "chat2db";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

impl Shell {
    pub fn detect_from_env() -> Result<Self> {
        let shell_env =
            std::env::var("SHELL").map_err(|_| Chat2DbError::CompletionsShellUnknown)?;
        let basename = Path::new(&shell_env)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(Chat2DbError::CompletionsShellUnknown)?;
        match basename {
            "zsh" => Ok(Shell::Zsh),
            "bash" => Ok(Shell::Bash),
            "fish" => Ok(Shell::Fish),
            other => Err(Chat2DbError::CompletionsShellUnsupported(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Fish => "fish",
        }
    }
}

impl fmt::Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct CompletionsRequest {
    pub shell: Option<Shell>,
    pub install: bool,
    pub home_dir: PathBuf,
}

impl CompletionsRequest {
    pub fn production(shell: Option<Shell>, install: bool) -> Self {
        let home_dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            shell,
            install,
            home_dir,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RcAction {
    Added,
    AlreadyConfigured,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionsOutcome {
    pub command: &'static str,
    pub status: &'static str,
    pub shell: Shell,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rc_path: Option<PathBuf>,
    pub rc_action: RcAction,
    #[serde(skip_serializing)]
    pub script: Option<Vec<u8>>,
}

pub fn run(request: CompletionsRequest) -> Result<CompletionsOutcome> {
    let shell = match request.shell {
        Some(s) => s,
        None => Shell::detect_from_env()?,
    };
    let script = render_script(shell);

    if !request.install {
        return Ok(CompletionsOutcome {
            command: "completions",
            status: "printed",
            shell,
            script_path: None,
            rc_path: None,
            rc_action: RcAction::NotApplicable,
            script: Some(script),
        });
    }

    let layout = install_layout(shell, &request.home_dir);
    if let Some(parent) = layout.script_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&layout.script_path, &script)?;

    let rc_action = match &layout.rc {
        Some(rc) => ensure_rc_source(&rc.path, &rc.block)?,
        None => RcAction::NotApplicable,
    };

    Ok(CompletionsOutcome {
        command: "completions",
        status: "installed",
        shell,
        script_path: Some(layout.script_path),
        rc_path: layout.rc.map(|rc| rc.path),
        rc_action,
        script: None,
    })
}

pub fn render_script(shell: Shell) -> Vec<u8> {
    let mut cmd = Cli::command();
    cmd.set_bin_name(BIN_NAME);
    let mut buf = Vec::new();
    match shell {
        Shell::Zsh => {
            generate(Zsh, &mut cmd, BIN_NAME, &mut buf);
            // clap_complete emits `'--opt=[desc]:VAL:action'` for value-taking
            // long options, which makes zsh complete `--opt=` (glued form).
            // Drop the trailing `=` so completion produces `--opt <space>` and
            // lets the user pick a value with the next tab.
            buf = strip_zsh_equals_form(&buf);
        }
        Shell::Bash => generate(Bash, &mut cmd, BIN_NAME, &mut buf),
        Shell::Fish => generate(Fish, &mut cmd, BIN_NAME, &mut buf),
    }
    buf
}

fn strip_zsh_equals_form(script: &[u8]) -> Vec<u8> {
    // Only `'--name=[` appears in clap's zsh output for value-taking long
    // options. Replacing `=[` with `[` converts each spec from the glued form
    // to the space-separated form without touching descriptions or values.
    let needle = b"=[";
    let replacement = b"[";
    let mut out = Vec::with_capacity(script.len());
    let mut i = 0;
    while i < script.len() {
        if script[i..].starts_with(needle) {
            out.extend_from_slice(replacement);
            i += needle.len();
        } else {
            out.push(script[i]);
            i += 1;
        }
    }
    out
}

struct InstallLayout {
    script_path: PathBuf,
    rc: Option<RcTarget>,
}

struct RcTarget {
    path: PathBuf,
    block: String,
}

fn install_layout(shell: Shell, home_dir: &Path) -> InstallLayout {
    match shell {
        Shell::Zsh => {
            let script_dir = "$HOME/.local/share/chat2db";
            InstallLayout {
                script_path: home_dir.join(".local/share/chat2db/_chat2db"),
                rc: Some(RcTarget {
                    path: home_dir.join(".zshrc"),
                    block: format!(
                        "fpath=(\"{script_dir}\" $fpath)\nautoload -Uz compinit\ncompinit"
                    ),
                }),
            }
        }
        Shell::Bash => {
            let script_path = home_dir.join(".local/share/chat2db/chat2db.bash");
            InstallLayout {
                script_path,
                rc: Some(RcTarget {
                    path: home_dir.join(".bashrc"),
                    block: "[ -s \"$HOME/.local/share/chat2db/chat2db.bash\" ] && source \"$HOME/.local/share/chat2db/chat2db.bash\"".to_string(),
                }),
            }
        }
        Shell::Fish => InstallLayout {
            script_path: home_dir.join(".config/fish/completions/chat2db.fish"),
            rc: None,
        },
    }
}

fn ensure_rc_source(rc_path: &Path, block_body: &str) -> Result<RcAction> {
    let existing = match fs::read_to_string(rc_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };

    if existing.contains(RC_MARKER_BEGIN) {
        let updated = replace_marker_block(&existing, block_body);
        if updated == existing {
            return Ok(RcAction::AlreadyConfigured);
        }
        fs::write(rc_path, updated)?;
        return Ok(RcAction::Added);
    }

    let needs_leading_newline = !existing.is_empty() && !existing.ends_with('\n');
    let block = format!(
        "{leading}\n{begin}\n{body}\n{end}\n",
        leading = if needs_leading_newline { "\n" } else { "" },
        begin = RC_MARKER_BEGIN,
        end = RC_MARKER_END,
        body = block_body,
    );

    let mut file = OpenOptions::new().create(true).append(true).open(rc_path)?;
    file.write_all(block.as_bytes())?;

    Ok(RcAction::Added)
}

fn replace_marker_block(existing: &str, block_body: &str) -> String {
    let Some(begin_start) = existing.find(RC_MARKER_BEGIN) else {
        return existing.to_string();
    };
    let search_from = begin_start + RC_MARKER_BEGIN.len();
    let Some(end_rel) = existing[search_from..].find(RC_MARKER_END) else {
        return existing.to_string();
    };
    let end_start = search_from + end_rel;
    let end_finish = end_start + RC_MARKER_END.len();
    let trailing_newline = existing[end_finish..].starts_with('\n');
    let replace_end = if trailing_newline {
        end_finish + 1
    } else {
        end_finish
    };
    let block = format!("{RC_MARKER_BEGIN}\n{block_body}\n{RC_MARKER_END}\n");
    format!(
        "{}{}{}",
        &existing[..begin_start],
        block,
        &existing[replace_end..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_zsh_script_with_compdef_header() {
        let script = render_script(Shell::Zsh);
        let text = String::from_utf8(script).unwrap();
        assert!(text.starts_with("#compdef chat2db"));
        // Spot-check that flag descriptions land in the script.
        assert!(text.contains("Choose which Chat2DB edition to install"));
        assert!(text.contains("(community pro local all)"));
        // Glued `--opt=` form is rewritten to space form so tab completes
        // `--opt <space>` instead of `--opt=`.
        assert!(!text.contains("--edition=["));
        assert!(text.contains("'--edition[Choose which Chat2DB edition to install"));
    }

    #[test]
    fn renders_bash_and_fish_scripts() {
        let bash = String::from_utf8(render_script(Shell::Bash)).unwrap();
        assert!(bash.contains("_chat2db"));

        let fish = String::from_utf8(render_script(Shell::Fish)).unwrap();
        assert!(fish.contains("chat2db"));
        assert!(fish.contains("install"));
    }

    #[test]
    fn install_writes_zsh_script_and_rc_block_idempotently() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();

        let first = run(CompletionsRequest {
            shell: Some(Shell::Zsh),
            install: true,
            home_dir: home.clone(),
        })
        .unwrap();
        assert_eq!(first.status, "installed");
        assert_eq!(first.rc_action, RcAction::Added);
        let script_path = first.script_path.unwrap();
        assert!(script_path.is_file());
        let rc_path = first.rc_path.unwrap();
        let rc_text = fs::read_to_string(&rc_path).unwrap();
        assert!(rc_text.contains(RC_MARKER_BEGIN));
        assert!(rc_text.contains("fpath=(\"$HOME/.local/share/chat2db\" $fpath)"));
        assert!(rc_text.contains("autoload -Uz compinit"));
        assert!(rc_text.contains("compinit"));
        assert!(!rc_text.contains("source \"$HOME/.local/share/chat2db/_chat2db\""));

        let second = run(CompletionsRequest {
            shell: Some(Shell::Zsh),
            install: true,
            home_dir: home,
        })
        .unwrap();
        assert_eq!(second.rc_action, RcAction::AlreadyConfigured);
        let rc_after = fs::read_to_string(&rc_path).unwrap();
        assert_eq!(
            rc_after.matches(RC_MARKER_BEGIN).count(),
            1,
            "rc must not gain duplicate marker blocks"
        );
    }

    #[test]
    fn install_replaces_old_zsh_source_block() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();
        let zshrc = home.join(".zshrc");
        fs::write(
            &zshrc,
            "# before\n# >>> chat2db completions >>>\n[ -s \"$HOME/.local/share/chat2db/_chat2db\" ] && source \"$HOME/.local/share/chat2db/_chat2db\"\n# <<< chat2db completions <<<\n# after\n",
        )
        .unwrap();

        let outcome = run(CompletionsRequest {
            shell: Some(Shell::Zsh),
            install: true,
            home_dir: home,
        })
        .unwrap();

        assert_eq!(outcome.rc_action, RcAction::Added);
        let rc_text = fs::read_to_string(&zshrc).unwrap();
        assert!(rc_text.contains("# before"));
        assert!(rc_text.contains("# after"));
        assert!(rc_text.contains("fpath=(\"$HOME/.local/share/chat2db\" $fpath)"));
        assert!(rc_text.contains("autoload -Uz compinit"));
        assert!(rc_text.contains("compinit"));
        assert!(!rc_text.contains("source \"$HOME/.local/share/chat2db/_chat2db\""));
        assert_eq!(rc_text.matches(RC_MARKER_BEGIN).count(), 1);
    }

    #[test]
    fn install_for_fish_skips_rc() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();

        let outcome = run(CompletionsRequest {
            shell: Some(Shell::Fish),
            install: true,
            home_dir: home.clone(),
        })
        .unwrap();

        assert_eq!(outcome.rc_action, RcAction::NotApplicable);
        assert!(outcome.rc_path.is_none());
        let script_path = outcome.script_path.unwrap();
        assert!(script_path.starts_with(home.join(".config/fish/completions")));
        assert!(script_path.is_file());
    }

    #[test]
    fn print_does_not_touch_filesystem() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();

        let outcome = run(CompletionsRequest {
            shell: Some(Shell::Zsh),
            install: false,
            home_dir: home.clone(),
        })
        .unwrap();

        assert_eq!(outcome.status, "printed");
        assert!(outcome.script.is_some());
        assert!(outcome.script_path.is_none());
        assert!(!home.join(".zshrc").exists());
    }
}
