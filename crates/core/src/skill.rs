use std::{
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use vibe_ssh_protocol::ErrorCode;

use crate::{Error, Result};

/// Guides printed by `vibe-ssh skill [topic]`: they always match the installed version.
const GUIDES: [(&str, &str); 3] = [
    ("", include_str!("../../../skills/vibe-ssh/guide.md")),
    (
        "static-site",
        include_str!("../../../skills/vibe-ssh/static-site.md"),
    ),
    (
        "compose",
        include_str!("../../../skills/vibe-ssh/compose.md"),
    ),
];

/// The main guide, or the guide for one deployment flow.
pub fn guide(topic: Option<&str>) -> Result<&'static str> {
    let topic = topic.unwrap_or_default();
    match GUIDES.iter().find(|(name, _)| *name == topic) {
        Some((_, text)) => Ok(text),
        None => Err(Error::new(
            ErrorCode::InvalidRequest,
            "unknown topic; available: static-site, compose (or no topic for the main guide)",
        )),
    }
}
/// The installed Skill only tells agents to run `vibe-ssh skill`.
const BOOTSTRAP: &str = include_str!("../../../skills/vibe-ssh/SKILL.md");
/// Shared skill directories read by Codex and other agents, and by Claude Code.
const AGENT_DIRS: [&str; 2] = [".agents/skills", ".claude/skills"];

/// Recognizes the target of a link this installer made earlier.
type Ours = fn(&Path) -> bool;

/// Points `link` at `target`. A link this installer made earlier is re-pointed, so moving the
/// app or its data directory does not leave agents on a stale copy. Anything else that is
/// already there is left alone.
fn link(target: &Path, link: &Path, ours: Ours) -> std::result::Result<(), String> {
    let create = || {
        std::fs::create_dir_all(link.parent().expect("has a parent"))
            .and_then(|()| symlink(target, link))
            .map_err(|e| e.to_string())
    };
    match std::fs::read_link(link) {
        Ok(existing) if existing == target => Ok(()),
        // Dangling, or one of ours pointing at an older location.
        Ok(existing) if !link.exists() || ours(&existing) => std::fs::remove_file(link)
            .map_err(|e| e.to_string())
            .and_then(|()| create()),
        Err(_) if !link.exists() => create(),
        _ => Err("already exists and is not a Vibe-SSH link; left untouched".into()),
    }
}

fn our_skill(target: &Path) -> bool {
    target.ends_with("skill/vibe-ssh")
}

fn our_cli(target: &Path) -> bool {
    let name = target.file_name().and_then(|name| name.to_str());
    name.is_some_and(|name| name.starts_with("vibe-ssh"))
}

/// Explicit user action: writes the bootstrap Skill under `home`, links it into the agent
/// skill directories of `user_home`, and optionally links the CLI into `~/.local/bin`.
pub fn install(home: &Path, user_home: &Path, cli: Option<&Path>) -> Result<Value> {
    let io = |e: std::io::Error| Error::new(ErrorCode::Internal, format!("skill: {e}"));
    let source = home.join("skill/vibe-ssh");
    std::fs::create_dir_all(&source).map_err(io)?;
    std::fs::write(source.join("SKILL.md"), BOOTSTRAP).map_err(io)?;
    let mut wanted: Vec<(PathBuf, PathBuf, Ours)> = AGENT_DIRS
        .iter()
        .map(|dir| {
            let path = user_home.join(dir).join("vibe-ssh");
            (source.clone(), path, our_skill as Ours)
        })
        .collect();
    if let Some(cli) = cli {
        let path = user_home.join(".local/bin/vibe-ssh");
        wanted.push((cli.to_owned(), path, our_cli));
    }
    let (mut linked, mut skipped) = (Vec::new(), Vec::new());
    for (target, path, ours) in wanted {
        match link(&target, &path, ours) {
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

        // After the app and its data directory move, our own links follow; a foreign
        // symlink of the same name does not.
        let (moved, app) = (
            temp.path().join("elsewhere"),
            temp.path().join("vibe-ssh-desktop"),
        );
        std::fs::write(&app, "").unwrap();
        std::fs::remove_dir_all(&foreign).unwrap();
        symlink(temp.path(), &foreign).unwrap();
        let report = install(&moved, user, Some(&app)).unwrap();
        assert_eq!(report["skipped"][0]["path"], json!(foreign));
        assert_eq!(std::fs::read_link(&foreign).unwrap(), temp.path());
        assert_eq!(
            std::fs::read_link(user.join(".agents/skills/vibe-ssh")).unwrap(),
            moved.join("skill/vibe-ssh")
        );
        assert_eq!(
            std::fs::read_link(user.join(".local/bin/vibe-ssh")).unwrap(),
            app
        );
        let (home, cli) = (moved, app);

        // Running it again changes nothing and still succeeds for the links that are ours.
        let again = install(&home, user, Some(&cli)).unwrap();
        assert_eq!(again["linked"].as_array().unwrap().len(), 2);
        for (topic, text) in GUIDES {
            assert!(
                text.trim_end().ends_with("[vibe-ssh:skill] end of guide"),
                "{topic}"
            );
            assert!(guide((!topic.is_empty()).then_some(topic)).is_ok());
        }
        assert!(guide(Some("kubernetes")).is_err());
    }
}
