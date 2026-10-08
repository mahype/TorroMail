//! Hermes' interactive `mcp add` is unsuitable for a Connect button. Update
//! its supported YAML surface without launching the agent or probing mail.
use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use crate::clients::{SERVER_NAME, SetupError, TOKEN_VARIABLE};

pub(crate) fn config_path(home: &Path, override_home: Option<&Path>) -> PathBuf {
    if let Some(override_home) = override_home {
        return override_home.join("config.yaml");
    }
    let root = home.join(".hermes");
    let profile = std::fs::read_to_string(root.join("active_profile")).unwrap_or_default();
    let profile = profile.trim().trim_start_matches('\u{feff}');
    if !profile.is_empty()
        && profile != "default"
        && profile
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return root.join("profiles").join(profile).join("config.yaml");
    }
    root.join("config.yaml")
}

fn source(config: &Path) -> Result<String, SetupError> {
    match std::fs::read_to_string(config) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(config).is_ok() {
                return Err(SetupError::UnreadableConfig);
            }
            Ok(String::new())
        }
        Err(_) => Err(SetupError::UnreadableConfig),
    }
}

fn parse(text: &str) -> Result<Mapping, SetupError> {
    if text
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
    {
        return Ok(Mapping::new());
    }
    let value: Value = serde_yaml_ng::from_str(text).map_err(|_| SetupError::UnreadableConfig)?;
    let Value::Mapping(root) = value else {
        return Err(SetupError::UnreadableConfig);
    };
    if let Some(servers) = root.get(Value::from("mcp_servers"))
        && !servers.is_null()
        && !servers.is_mapping()
    {
        return Err(SetupError::UnreadableConfig);
    }
    Ok(root)
}

pub(crate) fn entry(config: &Path) -> Option<Value> {
    let root = parse(&source(config).ok()?).ok()?;
    root.get(Value::from("mcp_servers"))?
        .as_mapping()?
        .get(Value::from(SERVER_NAME))
        .cloned()
        .filter(Value::is_mapping)
}

pub(crate) fn token(config: &Path) -> Option<String> {
    entry(config)?
        .get("env")?
        .get(TOKEN_VARIABLE)?
        .as_str()
        .map(str::to_owned)
}

pub(crate) fn set(config: &Path, command: &str, token: &str) -> Result<(), SetupError> {
    let root = parse(&source(config)?)?;
    let mut definition = root.get(Value::from("mcp_servers"))
        .and_then(Value::as_mapping).and_then(|servers| servers.get(Value::from(SERVER_NAME)))
        .and_then(Value::as_mapping).cloned().unwrap_or_default();
    for key in ["url", "type", "transport", "headers", "auth", "oauth", "args"] {
        definition.remove(Value::from(key));
    }
    let mut environment = definition.get(Value::from("env")).and_then(Value::as_mapping).cloned().unwrap_or_default();
    environment.remove(Value::from("TORROMAIL_POLICY_PATH"));
    environment.insert(Value::from(TOKEN_VARIABLE), Value::from(token));
    definition.insert(Value::from("command"), Value::from(command));
    definition.insert(Value::from("env"), Value::Mapping(environment));
    definition.insert(Value::from("enabled"), Value::Bool(true));
    update(config, Some(Value::Mapping(definition)))
}

pub(crate) fn remove(config: &Path) -> Result<(), SetupError> {
    update(config, None)
}

fn update(config: &Path, definition: Option<Value>) -> Result<(), SetupError> {
    let original = source(config)?;
    let mut root = parse(&original)?;
    let key = Value::from("mcp_servers");
    let mut servers = root
        .get(&key)
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    if definition.is_none() && !servers.contains_key(Value::from(SERVER_NAME)) {
        return Ok(());
    }
    let entry_text = if let Some(definition) = definition {
        servers.insert(Value::from(SERVER_NAME), definition.clone());
        let mut own = Mapping::new();
        own.insert(Value::from(SERVER_NAME), definition);
        serde_yaml_ng::to_string(&own)
            .map_err(|error| SetupError::WriteFailed(error.to_string()))?
    } else {
        servers.remove(Value::from(SERVER_NAME));
        String::new()
    };
    root.insert(key, Value::Mapping(servers));
    // A conservative formatting optimization: keep ordinary block YAML
    // byte-for-byte around our entry, and validate the edit with the parser.
    // Flow mappings and aliases fall back to a semantic YAML rewrite.
    let text = patch_block(&original, &entry_text)
        .filter(|candidate| parse(candidate).ok().as_ref() == Some(&root));
    let text = match text {
        Some(text) => text,
        None => serde_yaml_ng::to_string(&root)
            .map_err(|error| SetupError::WriteFailed(error.to_string()))?,
    };
    let target = std::fs::canonicalize(config).unwrap_or_else(|_| config.to_path_buf());
    crate::state::write_atomically(&target, text.as_bytes())
        .map_err(|error| SetupError::WriteFailed(error.to_string()))
}

fn patch_block(original: &str, entry: &str) -> Option<String> {
    let mut lines: Vec<String> = original.split('\n').map(str::to_owned).collect();
    let content = |line: &str| line.split('#').next().unwrap_or("").trim().to_owned();
    let significant = |line: &str| !content(line).is_empty();
    let indent = |line: &str| line.bytes().take_while(|byte| *byte == b' ').count();
    let Some(start) = lines
        .iter()
        .position(|line| indent(line) == 0 && content(line) == "mcp_servers:")
    else {
        if original.contains("mcp_servers") || entry.is_empty() {
            return None;
        }
        let newline = if original.is_empty() || original.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        let entry = entry
            .lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Some(format!("{original}{newline}mcp_servers:\n{entry}\n"));
    };
    let end = (start + 1..lines.len())
        .find(|index| significant(&lines[*index]) && indent(&lines[*index]) == 0)
        .unwrap_or(lines.len());
    let children: Vec<usize> = (start + 1..end)
        .filter(|index| significant(&lines[*index]))
        .collect();
    let level = children.first().map_or(2, |index| indent(&lines[*index]));
    if level == 0 {
        return None;
    }
    let own = children.iter().copied().find(|index| {
        indent(&lines[*index]) == level && content(&lines[*index]).starts_with("torromail:")
    });
    let mut lower = own.unwrap_or(end);
    if own.is_none() {
        while lower > start + 1 && !significant(&lines[lower - 1]) {
            lower -= 1;
        }
    }
    let mut upper = own.map_or(lower, |index| {
        (index + 1..end)
            .find(|index| significant(&lines[*index]) && indent(&lines[*index]) <= level)
            .unwrap_or(end)
    });
    while upper > lower + 1 && !significant(&lines[upper - 1]) {
        upper -= 1;
    }
    let replacement = entry
        .lines()
        .map(|line| format!("{}{line}", " ".repeat(level)));
    lines.splice(lower..upper, replacement);
    let newline = if lines.last().is_some_and(String::is_empty) {
        ""
    } else {
        "\n"
    };
    Some(lines.join("\n") + newline)
}
