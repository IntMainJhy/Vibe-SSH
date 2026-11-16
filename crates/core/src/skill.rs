use std::{
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use vibe_ssh_protocol::ErrorCode;

use crate::{Error, Result};

/// Printed by `vibe-ssh skill`: always matches the installed version.
pub const GUIDE: &str = include_str!("../../../skills/vibe-ssh/guide.md");
/// The installed Skill only tells agents to run `vibe-ssh skill`.
const BOOTSTRAP: &str = include_str!("../../../skills/vibe-ssh/SKILL.md");
/// Shared skill directories read by Codex and other agents, and by Claude Code.
const AGENT_DIRS: [&str; 2] = [".agents/skills", ".claude/skills"];

/// Points `link` at `target`. Anything already there that is not this exact link is left alone.
fn link(target: &Path, link: &Path) -> std::result::Result<(), String> {
    match std::fs::read_link(link) {
        Ok(existing) if existing == target => Ok(()),
        // A dangling link (the app was moved or rebuilt elsewhere) is ours to repair.
        Ok(_) if !link.exists() => std::fs::remove_file(link)
            .and_then(|()| symlink(target, link))
            .map_err(|e| e.to_string()),
        Err(_) if !link.exists() => std::fs::create_dir_all(link.parent().expect("has a parent"))
            .and_then(|()| symlink(target, link))
            .map_err(|e| e.to_string()),
        _ => Err("already exists and is not this Vibe-SSH link; left untouched".into()),
    }
}

/// Explicit user action: writes the bootstrap Skill under `home`, links it into the agent
/// skill directories of `user_home`, and optionally links the CLI into `~/.local/bin`.
pub fn install(home: &Path, user_home: &Path, cli: Option<&Path>) -> Result<Value> {
    let io = |e: std::io::Error| Error::new(ErrorCode::Internal, format!("skill: {e}"));
    let source = home.join("skill/vibe-ssh");
    std::fs::create_dir_all(&source).map_err(io)?;
    std::fs::write(source.join("SKILL.md"), BOOTSTRAP).map_err(io)?;
    let mut wanted: Vec<(PathBuf, PathBuf)> = AGENT_DIRS
        .iter()
        .map(|dir| (source.clone(), user_home.join(dir).join("vibe-ssh")))
        .collect();
    if let Some(cli) = cli {
        wanted.push((cli.to_owned(), user_home.join(".local/bin/vibe-ssh")));
    }
    let (mut linked, mut skipped) = (Vec::new(), Vec::new());
    for (target, path) in wanted {
        match link(&target, &path) {
            Ok(()) => linked.push(json!(path)),
            Err(reason) => skipped.push(json!({ "path": path, "reason": reason })),
        }
    }
    Ok(json!({ "linked": linked, "skipped": skipped }))
}

/// Which of the links `install` would create are already in place.
pub fn installed(home: &Path, user_home: &Path) -> Value {
    let source = home.join("skill/vibe-ssh");
    let skills: Vec<bool> = AGENT_DIRS
        .iter()
        .map(|dir| {
            std::fs::read_link(user_home.join(dir).join("vibe-ssh")).is_ok_and(|t| t == source)
        })
        .collect();
    json!({
        "skill": skills.iter().all(|linked| *linked),
        "cli": user_home.join(".local/bin/vibe-ssh").exists(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_links_agent_dirs_and_never_overwrites_unrelated_entries() {
        let temp = tempfile::tempdir().unwrap();
        let (home, user) = (temp.path().join(".vibe-ssh"), temp.path());
        let foreign = user.join(".claude/skills/vibe-ssh");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("SKILL.md"), "someone else's skill").unwrap();
        let cli = temp.path().join("vibe-ssh-binary");
        std::fs::write(&cli, "").unwrap();

        let report = install(&home, user, Some(&cli)).unwrap();
        assert_eq!(report["linked"].as_array().unwrap().len(), 2);
        assert_eq!(report["skipped"][0]["path"], json!(foreign));
        assert_eq!(
            std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
            "someone else's skill"
        );
        let linked = user.join(".agents/skills/vibe-ssh/SKILL.md");
        assert!(
            std::fs::read_to_string(linked)
                .unwrap()
                .contains("vibe-ssh skill")
        );
        assert_eq!(
            std::fs::read_link(user.join(".local/bin/vibe-ssh")).unwrap(),
            cli
        );
        assert_eq!(
            installed(&home, user),
            json!({ "skill": false, "cli": true })
        );

        // Running it again changes nothing and still succeeds for the links that are ours.
        let again = install(&home, user, Some(&cli)).unwrap();
        assert_eq!(again["linked"].as_array().unwrap().len(), 2);
        assert!(GUIDE.trim_end().ends_with("[vibe-ssh:skill] end of guide"));
    }
}
