use std::env;
use std::fs;
use std::fs::File;
use std::io::{self, Write, BufRead};
use std::path::PathBuf;
use std::collections::HashMap;
use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCombo {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

pub fn parse_key_combo(s: &str) -> Option<KeyCombo> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    if parts.is_empty() || parts[0].is_empty() { return None; }
    let mut mods = KeyModifiers::NONE;
    let key_part = parts.last()?;
    for p in parts.iter().take(parts.len() - 1) {
        match *p {
            "Ctrl" | "C" => mods.insert(KeyModifiers::CONTROL),
            "Shift" | "S" => mods.insert(KeyModifiers::SHIFT),
            "Alt" | "A" => mods.insert(KeyModifiers::ALT),
            _ => return None,
        }
    }
    let code = match *key_part {
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Space" => KeyCode::Char(' '),
        k if k.len() == 1 => KeyCode::Char(k.chars().next()?),
        k if k.starts_with('F') && k.len() > 1 => KeyCode::F(k[1..].parse().ok()?),
        _ => return None,
    };
    Some(KeyCombo { code, mods })
}

pub fn key_combo_to_string(c: &KeyCombo) -> String {
    let mut parts: Vec<String> = vec![];
    if c.mods.contains(KeyModifiers::CONTROL) { parts.push("Ctrl".into()); }
    if c.mods.contains(KeyModifiers::SHIFT) { parts.push("Shift".into()); }
    if c.mods.contains(KeyModifiers::ALT) { parts.push("Alt".into()); }
    parts.push(match c.code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(c) => c.to_uppercase().collect(),
        KeyCode::F(n) => format!("F{}", n),
        _ => format!("{:?}", c.code),
    });
    if parts.len() == 1 { parts[0].clone() } else { parts.join("+") }
}

pub fn default_bindings() -> Vec<(&'static str, KeyCombo)> {
    vec![
        ("quit", KeyCombo { code: KeyCode::Char('q'), mods: KeyModifiers::CONTROL }),
        ("save", KeyCombo { code: KeyCode::Char('s'), mods: KeyModifiers::CONTROL }),
        ("undo", KeyCombo { code: KeyCode::Char('z'), mods: KeyModifiers::CONTROL }),
        ("redo", KeyCombo { code: KeyCode::Char('y'), mods: KeyModifiers::CONTROL }),
        ("search", KeyCombo { code: KeyCode::Char('f'), mods: KeyModifiers::CONTROL }),
        ("copy", KeyCombo { code: KeyCode::Char('c'), mods: KeyModifiers::CONTROL }),
        ("cut", KeyCombo { code: KeyCode::Char('x'), mods: KeyModifiers::CONTROL }),
        ("paste", KeyCombo { code: KeyCode::Char('v'), mods: KeyModifiers::CONTROL }),
        ("select_all", KeyCombo { code: KeyCode::Char('a'), mods: KeyModifiers::CONTROL }),
        ("delete_line", KeyCombo { code: KeyCode::Char('d'), mods: KeyModifiers::CONTROL }),
        ("clear_buffer", KeyCombo { code: KeyCode::Char('k'), mods: KeyModifiers::CONTROL }),
        ("browser", KeyCombo { code: KeyCode::Char('n'), mods: KeyModifiers::CONTROL }),
        ("close_tab", KeyCombo { code: KeyCode::Char('w'), mods: KeyModifiers::CONTROL }),
        ("browser_back", KeyCombo { code: KeyCode::Left, mods: KeyModifiers::ALT }),
        ("home_file", KeyCombo { code: KeyCode::Home, mods: KeyModifiers::CONTROL }),
        ("end_file", KeyCombo { code: KeyCode::End, mods: KeyModifiers::CONTROL }),
        ("reorder", KeyCombo { code: KeyCode::Up, mods: KeyModifiers::CONTROL }),
        ("tab_prev", KeyCombo { code: KeyCode::Left, mods: KeyModifiers::CONTROL }),
        ("tab_next", KeyCombo { code: KeyCode::Right, mods: KeyModifiers::CONTROL }),
        ("save_quit", KeyCombo { code: KeyCode::Char('w'), mods: KeyModifiers::CONTROL | KeyModifiers::SHIFT }),
        ("sysinfo", KeyCombo { code: KeyCode::Char('l'), mods: KeyModifiers::CONTROL }),
        ("completion", KeyCombo { code: KeyCode::Char('m'), mods: KeyModifiers::CONTROL }),
    ]
}

pub fn load_bindings(cfg: &HashMap<String, String>) -> HashMap<String, KeyCombo> {
    let mut bindings = HashMap::new();
    for (name, combo) in default_bindings() {
        let key = format!("bind_{}", name);
        if let Some(val) = cfg.get(&key) {
            if let Some(c) = parse_key_combo(val) {
                bindings.insert(name.to_string(), c);
                continue;
            }
        }
        bindings.insert(name.to_string(), combo);
    }
    bindings
}

pub fn find_action<'a>(bindings: &'a HashMap<String, KeyCombo>, code: &KeyCode, mods: KeyModifiers) -> Option<&'a str> {
    for (name, combo) in bindings {
        if combo.code == *code && mods.contains(combo.mods) {
            return Some(name);
        }
    }
    None
}

pub fn config_dir() -> PathBuf {
    let base = env::var("XDG_CONFIG_HOME").map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("wispy")
}

pub fn config_path() -> PathBuf { config_dir().join("config") }
pub fn themes_dir() -> PathBuf { config_dir().join("themes") }

pub fn load_config_raw() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(file) = File::open(config_path()) {
        for line in io::BufReader::new(file).lines().flatten() {
            let line = line.trim().to_string();
            if let Some(pos) = line.find('=') {
                let key = line[..pos].trim().to_string();
                let val = line[pos+1..].trim().to_string();
                map.insert(key, val);
            }
        }
    }
    map
}

pub fn save_config_raw(map: &HashMap<String, String>) {
    let _ = fs::create_dir_all(config_dir());
    if let Ok(mut f) = File::create(config_path()) {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            let _ = writeln!(f, "{} = {}", k, map[k]);
        }
    }
}

pub fn save_theme_setting(name: &str) {
    let mut cfg = load_config_raw();
    cfg.insert("theme".to_string(), name.to_string());
    save_config_raw(&cfg);
}

pub struct Theme {
    pub name: String,
    pub rules: HashMap<String, Vec<String>>,
}

pub fn load_theme(name: &str) -> Option<Theme> {
    let path = themes_dir().join(format!("{}.theme", name));
    let file = File::open(&path).ok()?;
    let mut rules: HashMap<String, Vec<String>> = HashMap::new();
    for line in io::BufReader::new(file).lines().flatten() {
        let line = line.trim().to_string();
        if let Some(pos) = line.find(':') {
            let color = line[..pos].trim().to_lowercase();
            let rest = line[pos+1..].trim();
            let words: Vec<String> = rest.split_whitespace()
                .map(|s| s.trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !words.is_empty() {
                rules.entry(color).or_default().extend(words);
            }
        }
    }
    if rules.is_empty() { None }
    else { Some(Theme { name: name.to_string(), rules }) }
}

pub fn theme_color(word: &str, theme: &Theme) -> Option<&'static str> {
    let color_map: [(&str, &str); 8] = [
        ("red", "\x1B[31m"), ("green", "\x1B[32m"), ("yellow", "\x1B[33m"),
        ("blue", "\x1B[34m"), ("magenta", "\x1B[35m"), ("cyan", "\x1B[36m"),
        ("white", "\x1B[37m"), ("black", "\x1B[30m"),
    ];
    for (name, code) in &color_map {
        if let Some(words) = theme.rules.get(*name) {
            if words.iter().any(|w| w == word) {
                return Some(code);
            }
        }
    }
    None
}

pub fn semantic_color(category: &str, theme: &Theme) -> Option<&'static str> {
    let color_map: [(&str, &str); 8] = [
        ("red", "\x1B[31m"), ("green", "\x1B[32m"), ("yellow", "\x1B[33m"),
        ("blue", "\x1B[34m"), ("magenta", "\x1B[35m"), ("cyan", "\x1B[36m"),
        ("white", "\x1B[37m"), ("black", "\x1B[30m"),
    ];
    for (name, code) in &color_map {
        if let Some(words) = theme.rules.get(*name) {
            if words.iter().any(|w| w == category) {
                return Some(code);
            }
        }
    }
    None
}
