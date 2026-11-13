//! Per-user preferences: display choices (`prefs/<userId>.json`) and keyboard shortcuts
//! (`prefs/<userId>.shortcuts.json`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};

use crate::paths;

pub const MAX_KEY: usize = 200;
pub const MAX_TOTAL: usize = 256 * 1024;
pub const MAX_BINDINGS: usize = 500;

/// One writer at a time.
static WRITE: Mutex<()> = Mutex::new(());

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn display_path(data_dir: &Path, user_id: &str) -> PathBuf {
    data_dir.join("prefs").join(format!("{user_id}.json"))
}

fn shortcuts_path(data_dir: &Path, user_id: &str) -> PathBuf {
    data_dir.join("prefs").join(format!("{user_id}.shortcuts.json"))
}

pub fn display(data_dir: &Path, user_id: &str) -> Map<String, Value> {
    if !safe_id(user_id) {
        return Map::new();
    }
    match paths::read_json::<Value>(&display_path(data_dir, user_id)) {
        Ok(Some(Value::Object(m))) => m,
        _ => Map::new(),
    }
}

/// Merges `patch` into the stored object (`null` removes a key).
pub fn merge(mut current: Map<String, Value>, patch: Map<String, Value>) -> Result<Map<String, Value>, String> {
    for (k, v) in patch {
        if k.is_empty() || k.chars().count() > MAX_KEY {
            return Err("a preference key has 1 to 200 characters".into());
        }
        if v.is_null() {
            current.remove(&k);
        } else {
            current.insert(k, v);
        }
    }
    let size = serde_json::to_vec(&current).map(|v| v.len()).unwrap_or(usize::MAX);
    if size > MAX_TOTAL {
        return Err("the preferences are too large (256 KB at most)".into());
    }
    Ok(current)
}

pub fn patch_display(data_dir: &Path, user_id: &str, patch: Map<String, Value>) -> Result<(), String> {
    if !safe_id(user_id) {
        return Err("bad user id".into());
    }
    let _w = WRITE.lock().unwrap();
    let merged = merge(display(data_dir, user_id), patch)?;
    paths::write_json(&display_path(data_dir, user_id), &Value::Object(merged), false).map_err(|e| e.to_string())
}

/// The user's shortcut bindings, `None` = the defaults.
pub fn shortcuts(data_dir: &Path, user_id: &str) -> Option<Value> {
    if !safe_id(user_id) {
        return None;
    }
    paths::read_json::<Value>(&shortcuts_path(data_dir, user_id)).ok().flatten().filter(|v| v.is_array())
}

/// Checks `[{action, keys}]`: action a non-empty string, keys a string or a list of strings.
pub fn check_bindings(v: &Value) -> Result<(), String> {
    let list = v.as_array().ok_or("bindings is a list")?;
    if list.len() > MAX_BINDINGS {
        return Err("too many shortcuts".into());
    }
    for b in list {
        let action = b.get("action").and_then(Value::as_str).unwrap_or("");
        if action.is_empty() || action.len() > MAX_KEY {
            return Err("each shortcut has an action".into());
        }
        let keys_ok = match b.get("keys") {
            Some(Value::String(s)) => s.len() <= MAX_KEY,
            Some(Value::Array(a)) => a.len() <= 8 && a.iter().all(|k| k.as_str().is_some_and(|s| s.len() <= MAX_KEY)),
            Some(Value::Null) => true,
            _ => false,
        };
        if !keys_ok {
            return Err("the keys of a shortcut are text".into());
        }
    }
    Ok(())
}

pub fn set_shortcuts(data_dir: &Path, user_id: &str, bindings: Option<Value>) -> Result<(), String> {
    if !safe_id(user_id) {
        return Err("bad user id".into());
    }
    let _w = WRITE.lock().unwrap();
    let path = shortcuts_path(data_dir, user_id);
    match bindings {
        None => match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        },
        Some(b) => {
            check_bindings(&b)?;
            paths::write_json(&path, &b, false).map_err(|e| e.to_string())
        }
    }
}

/// Removes the preferences of a deleted user.
pub fn forget(data_dir: &Path, user_id: &str) {
    if safe_id(user_id) {
        let _ = std::fs::remove_file(display_path(data_dir, user_id));
        let _ = std::fs::remove_file(shortcuts_path(data_dir, user_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn merge_rules() {
        let m = merge(obj(json!({"a": 1, "b": 2})), obj(json!({"b": null, "c": [1]}))).unwrap();
        assert_eq!(Value::Object(m), json!({"a": 1, "c": [1]}));
        assert!(merge(Map::new(), obj(json!({ "k".repeat(201): 1 }))).is_err());
        assert!(merge(Map::new(), obj(json!({"big": "x".repeat(MAX_TOTAL)}))).is_err());
    }

    #[test]
    fn bindings_rules() {
        assert!(check_bindings(&json!([{"action": "scan", "keys": ["Ctrl", "K"]}, {"action": "help", "keys": "?"}])).is_ok());
        assert!(check_bindings(&json!([{"keys": "?"}])).is_err());
        assert!(check_bindings(&json!([{"action": "x", "keys": 3}])).is_err());
        assert!(check_bindings(&json!({})).is_err());
    }
}
