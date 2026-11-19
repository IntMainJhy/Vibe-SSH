use super::{Reply, invalid};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

const MAX_BYTES: u64 = 256 * 1024;
const MAX_ALIASES: usize = 256;

#[tauri::command]
pub async fn choose_ssh_config() -> Reply<Option<Value>> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("选择 OpenSSH 配置")
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let metadata = tokio::fs::metadata(file.path())
        .await
        .map_err(|_| invalid("无法读取所选配置。"))?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(invalid("请选择不超过 256 KiB 的普通配置文件。"));
    }
    let file = tokio::fs::File::open(file.path())
        .await
        .map_err(|_| invalid("无法打开所选配置。"))?;
    let mut content = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut content)
        .await
        .map_err(|_| invalid("无法读取所选配置。"))?;
    if content.len() as u64 > MAX_BYTES {
        return Err(invalid("配置文件超过 256 KiB。"));
    }
    let source =
        std::str::from_utf8(&content).map_err(|_| invalid("配置文件需使用 UTF-8 编码。"))?;
    parse(source).map(Some)
}

fn arguments(line: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut present = false;
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => {
                word.push(chars.next()?);
                present = true;
            }
            '"' => {
                quoted = !quoted;
                present = true;
            }
            '#' if !quoted => break,
            value if value.is_ascii_whitespace() && !quoted => {
                if present {
                    result.push(std::mem::take(&mut word));
                    present = false;
                }
            }
            value => {
                word.push(value);
                present = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if present {
        result.push(word);
    }
    Some(result)
}

fn matches(pattern: &str, alias: &str) -> bool {
    let pattern = pattern.as_bytes();
    let alias = alias.as_bytes();
    let (mut at, mut index) = (0, 0);
    let mut retry = None;
    while index < alias.len() {
        match pattern.get(at) {
            Some(b'*') => {
                at += 1;
                retry = Some((at, index));
            }
            Some(byte) if *byte == b'?' || byte.eq_ignore_ascii_case(&alias[index]) => {
                at += 1;
                index += 1;
            }
            _ => match retry {
                Some((next, start)) => {
                    at = next;
                    index = start + 1;
                    retry = Some((next, index));
                }
                None => return false,
            },
        }
    }
    pattern[at..].iter().all(|byte| *byte == b'*')
}

#[derive(Default)]
struct Section {
    patterns: Vec<String>,
    options: Vec<(usize, String, Vec<String>)>,
}

fn parse(source: &str) -> Reply<Value> {
    if source.lines().count() > 4096 || source.lines().any(|line| line.len() > 4096) {
        return Err(invalid("配置最多支持 4096 行，每行最多 4096 字节。"));
    }
    let mut sections = vec![Section {
        patterns: vec!["*".into()],
        ..Section::default()
    }];
    let mut aliases = Vec::<String>::new();
    let mut warnings = Vec::new();
    let mut in_match = false;
    for (index, line) in source.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let split = line
            .find(|character: char| character.is_ascii_whitespace() || character == '=')
            .unwrap_or(line.len());
        let name = line[..split].to_ascii_lowercase();
        let rest = line[split..].trim_start();
        let values = arguments(rest.strip_prefix('=').unwrap_or(rest));
        let number = index + 1;
        if name == "match" {
            in_match = true;
            warnings.push(json!({"line":number,"directive":"Match"}));
            continue;
        }
        if name == "host" {
            in_match = false;
            let patterns = values.unwrap_or_default();
            if patterns.is_empty() {
                warnings.push(json!({"line":number,"directive":"Host"}));
            }
            for alias in patterns.iter().filter(|alias| {
                !alias.is_empty()
                    && alias.len() <= 253
                    && alias
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-:".contains(&byte))
            }) {
                if !aliases
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(alias))
                {
                    if aliases.len() >= MAX_ALIASES {
                        return Err(invalid("配置超过 256 个可导入的主机。"));
                    }
                    aliases.push(alias.clone());
                }
            }
            sections.push(Section {
                patterns,
                ..Section::default()
            });
            continue;
        }
        if in_match {
            continue;
        }
        if matches!(
            name.as_str(),
            "hostname" | "user" | "port" | "identityfile" | "proxyjump" | "proxycommand"
        ) {
            sections
                .last_mut()
                .unwrap()
                .options
                .push((number, name, values.unwrap_or_default()));
        } else if warnings.len() < 256
            && name.len() <= 64
            && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            warnings.push(json!({"line":number,"directive":name}));
        }
    }
    let candidates: Vec<_> = aliases.iter().map(|alias| {
        let selected = sections.iter().filter(|section| {
            section.patterns.iter().any(|pattern| !pattern.starts_with('!') && matches(pattern, alias))
                && !section.patterns.iter().any(|pattern| pattern.strip_prefix('!').is_some_and(|pattern| matches(pattern, alias)))
        });
        let mut host = None;
        let mut user = None;
        let mut port = None;
        let mut identity = None;
        let mut issues = Vec::new();
        let mut routed = false;
        for (line, name, values) in selected.flat_map(|section| &section.options) {
            if matches!(name.as_str(), "proxyjump" | "proxycommand") {
                if !matches!(values.as_slice(), [value] if value.eq_ignore_ascii_case("none")) { routed = true; }
                continue;
            }
            let value = match values.as_slice() {
                [value] if !value.is_empty() && !value.contains(['%', '$', '\0']) => value,
                _ => { if issues.len() < 8 { issues.push(format!("第 {line} 行：{name} 需要手动确认")); } continue; }
            };
            match name.as_str() {
                "hostname" if host.is_none() => host = Some(value),
                "user" if user.is_none() => user = Some(value),
                "port" if port.is_none() => {
                    port = value.parse::<u16>().ok().filter(|port| *port > 0);
                    if port.is_none() && issues.len() < 8 { issues.push(format!("第 {line} 行：端口无效")); }
                }
                "identityfile" if identity.is_none() && value != "none" => identity = Some(value),
                "identityfile" if identity.is_some() && issues.len() < 8 => issues.push(format!("第 {line} 行：其他私钥需手动选择")),
                _ => {}
            }
        }
        json!({"name":alias,"host":host.unwrap_or(alias),"username":user,"port":port.unwrap_or(22),"identity_file":identity,"issues":issues,"routed":routed})
    }).collect();
    Ok(json!({"candidates":candidates,"warnings":warnings}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_aliases_with_defaults_and_never_evaluates_directives() {
        let value = parse(
            r#"
Host demo stage
  hOsTnAmE = 192.0.2.4
  IdentityFile "~/.ssh/demo # key"
  ProxyCommand command-that-must-not-run %h
Host !stage *
  Port 2200
Host *
  User deploy
Include unselected-file
Match exec "command-that-must-not-run"
  HostName ignored
"#,
        )
        .unwrap();
        let candidates = value["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0]["host"], "192.0.2.4");
        assert_eq!(candidates[0]["identity_file"], "~/.ssh/demo # key");
        assert_eq!(candidates[0]["username"], "deploy");
        assert_eq!(candidates[0]["port"], 2200);
        assert_eq!(candidates[0]["routed"], true);
        assert_eq!(candidates[1]["port"], 22);
        assert_eq!(value["warnings"].as_array().unwrap().len(), 2);
    }
}
