mod config;
mod editor;
use crate::config::*;
use crate::editor::*;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseEventKind, MouseButton},
    terminal::{disable_raw_mode, enable_raw_mode, size},
    execute,
};
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
use std::process::Command;

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let arg_file = if args.len() > 1 { Some(args[1].clone()) } else { None };

    enable_raw_mode()?;
    execute!(io::stdout(), EnableMouseCapture)?;

    let current_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut tabs: Vec<Tab> = if let Some(path) = &arg_file {
        let p = PathBuf::from(path);
        if p.exists() && p.is_file() {
            if let Ok(content) = fs::read_to_string(&p) {
                let lines: Vec<String> = content.lines().map(String::from).collect();
                let lines = if lines.is_empty() { vec![String::new()] } else { lines };
                vec![Tab::File(FileState {
                    path: Some(path.clone()),
                    lines,
                    cursor: Pos { x: 0, y: 0 },
                    dirty: false,
                    snapshot: content.clone(),
                    undo: Vec::new(),
                    redo: Vec::new(),
                    selecting: false,
                    sel_start: Pos { x: 0, y: 0 },
                    search_q: String::new(),
                    search_mode: false,
                })]
            } else {
                let entries = scan_dir(&current_dir).unwrap_or_default();
                vec![Tab::Browser(BrowserState {
                    dir: current_dir,
                    entries,
                    cursor: 0,
                    scroll: 0,
                    history: Vec::new(),
                })]
            }
        } else {
            let entries = scan_dir(&current_dir).unwrap_or_default();
            vec![Tab::Browser(BrowserState {
                dir: current_dir,
                entries,
                cursor: 0,
                scroll: 0,
                history: Vec::new(),
            })]
        }
    } else {
        let entries = scan_dir(&current_dir).unwrap_or_default();
        vec![Tab::Browser(BrowserState {
            dir: current_dir,
            entries,
            cursor: 0,
            scroll: 0,
            history: Vec::new(),
        })]
    };
    let mut active_tab: usize = 0;

    let mut row_off = 0;
    let mut col_off = 0;
    let mut clipboard: Vec<String> = Vec::new();
    let mut cmd_mode = false;
    let mut cmd_buf = String::new();
    let mut cmd_history: Vec<String> = Vec::new();
    let mut cmd_history_idx: Option<usize> = None;
    let mut show_numbers = true;
    let mut newfile_prompt = false;
    let mut newfile_name = String::new();
    let mut rename_prompt = false;
    let mut rename_buf = String::new();
    let mut rename_target: Option<std::path::PathBuf> = None;

    let mut last_action = Action::None;
    let mut reorder_mode = false;
    let mut tab_width = 4;
    let cfg = load_config_raw();
    if let Some(v) = cfg.get("show_numbers") { show_numbers = v == "true"; }
    if let Some(v) = cfg.get("tab_width") { if let Ok(n) = v.parse::<usize>() { tab_width = n; } }
    let theme_name = cfg.get("theme").cloned().unwrap_or_else(|| "default".to_string());
    let mut current_theme: Option<Theme> = load_theme(&theme_name);
    let mut bindings = load_bindings(&cfg);

    loop {
        print!("\x1B[?25l");
        let (tw, th) = size()?;
        let vw = tw as usize;
        let vh = (th as usize).saturating_sub(2);

        let num_width = match &tabs[active_tab] {
            Tab::File(f) => if show_numbers && !f.lines.is_empty() { f.lines.len().to_string().len() } else { 0 },
            Tab::Browser(_) => 0,
        };
        let prefix_len = if show_numbers && num_width > 0 { num_width + 1 } else { 0 };
        let text_vw = vw.saturating_sub(prefix_len + 1);

        // scrolling
        if let Tab::File(ref file) = tabs[active_tab] {
            if file.lines.len() > 0 {
                if file.cursor.y < row_off { row_off = file.cursor.y; }
                else if file.cursor.y >= row_off + vh { row_off = file.cursor.y - vh + 1; }
            }
            if file.cursor.x < col_off { col_off = file.cursor.x; }
            else if file.cursor.x >= col_off + text_vw { col_off = file.cursor.x - text_vw + 1; }
        }

        // auto-reload files 
        for tab in &mut tabs {
            if let Tab::File(file) = tab {
                if let Some(ref path) = file.path {
                    if let Ok(content) = fs::read_to_string(path) {
                        if content != file.snapshot {
                            let new_lines: Vec<String> = content.lines().map(String::from).collect();
                            let new_lines = if new_lines.is_empty() { vec![String::new()] } else { new_lines };
                            file.lines = new_lines;
                            file.snapshot = content;
                            file.dirty = false;
                            if file.cursor.y >= file.lines.len() {
                                file.cursor.y = file.lines.len().saturating_sub(1);
                            }
                            let cc = file.lines[file.cursor.y].chars().count();
                            if file.cursor.x > cc { file.cursor.x = cc; }
                        }
                    }
                }
            }
        }

        // tab bar
        print!("\x1B[H\x1B[K");
        let mut tab_line = String::new();
        for (i, tab) in tabs.iter().enumerate() {
            let name = match tab {
                Tab::File(f) => {
                    let base = f.path.as_ref().and_then(|p| PathBuf::from(p).file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| "[No Name]".to_string());
                    let marker = if f.dirty { "●" } else { "" };
                    format!(" {}{} ", base, marker)
                }
                Tab::Browser(_) => " [Browser] ".to_string(),
            };
            if i == active_tab {
                tab_line.push_str(&format!("\x1B[7m{}\x1B[0m", name));
            } else {
                tab_line.push_str(&name);
            }
            if i + 1 < tabs.len() { tab_line.push('│'); }
        }
        if tab_line.chars().count() > vw {
            tab_line = tab_line.chars().skip(tab_line.chars().count() - vw + 3).collect();
            tab_line = format!("...{}", tab_line);
        }
        print!("{}\x1B[K\r\n", tab_line);

        // body
        if let Tab::Browser(browser) = &mut tabs[active_tab] {
            if browser.cursor < browser.scroll { let _ = browser.scroll; }
            if browser.cursor >= browser.scroll + vh - 1 { browser.scroll = browser.cursor - vh + 2; }
        }
        match &tabs[active_tab] {
            Tab::File(file) => {
                let mut in_block_comment = false;
                let mut in_block_string = false;
                let sel_for_line = |li: usize, ll: usize| -> Option<(usize, usize)> {
                    if file.selecting && file.sel_start.y == file.cursor.y && file.sel_start.x == file.cursor.x { return None; }
                    line_sel_range(li, &file.sel_start, &file.cursor, file.selecting, ll)
                };

                let mut search_line_matches: Vec<Vec<(usize, usize)>> = Vec::new();
                if !file.search_q.is_empty() {
                    for r in 0..vh {
                        let li = r + row_off;
                        if li < file.lines.len() {
                            search_line_matches.push(find_matches(&file.lines[li], &file.search_q));
                        } else {
                            search_line_matches.push(vec![]);
                        }
                    }
                }

                let total = file.lines.len();
                for r in 0..vh {
                    let li = r + row_off;
                    let sb = if total <= vh { " " }
                    else {
                        let thumb = (vh * vh / total).max(1);
                        let pos = row_off * vh / total;
                        if r >= pos && r < pos + thumb { "\x1B[48;5;239m \x1B[0m" }
                        else { " " }
                    };
                    if li < file.lines.len() {
                        let prefix = if show_numbers {
                            format!("{:>1$} ", li + 1, num_width)
                        } else { String::new() };

                        let full_line = &file.lines[li];
                        let full_len = full_line.chars().count();
                        let vis_start = col_off.min(full_len);
                        let vis_end = (col_off + text_vw).min(full_len);

                        let plain_line: String = full_line.chars().skip(vis_start).take(vis_end - vis_start).collect();

                        let adj_sel = sel_for_line(li, full_len).and_then(|(s, e)| {
                            let ns = s.saturating_sub(vis_start);
                            let ne = e.saturating_sub(vis_start);
                            if ns >= ne || ns > plain_line.chars().count() { None }
                            else { Some((ns, ne.min(plain_line.chars().count()))) }
                        });

                        let adj_sm: Vec<(usize, usize)> = search_line_matches.get(r)
                            .map(|matches| {
                                matches.iter().filter_map(|&(s, e)| {
                                    let ns = s.saturating_sub(vis_start);
                                    let ne = e.saturating_sub(vis_start);
                                    if ns >= ne || ns > plain_line.chars().count() { None }
                                    else { Some((ns, ne.min(plain_line.chars().count()))) }
                                }).collect()
                            })
                            .unwrap_or_default();

                        let rendered = render_line(&plain_line, adj_sel, &adj_sm, current_theme.as_ref(), &mut in_block_comment, &mut in_block_string);
                        print!("{}{}\x1B[K\r\n", prefix + &rendered, sb);
                    } else {
                        print!("{}\x1B[K\r\n", sb);
                    }
                }
            }
            Tab::Browser(browser) => {
                let header = format!("{}:", browser.dir.display());
                let h: String = header.chars().take(vw).collect();
                print!("{}\x1B[K\r\n", h);

                let total_entries = browser.entries.len() + 2;
                let start = browser.scroll;
                let content_lines = vh.saturating_sub(1);
                let end = (start + content_lines).min(total_entries);

                for i in start..end {
                    let active = i == browser.cursor;
                    let entry_str = if i == 0 {
                        render_browser_line("..", true, active, vw.saturating_sub(1))
                    } else if i == total_entries - 1 {
                        render_browser_line("[New file]", false, active, vw.saturating_sub(1))
                    } else {
                        let idx = i - 1;
                        let (name, is_dir) = &browser.entries[idx];
                        render_browser_line(name, *is_dir, active, vw.saturating_sub(1))
                    };
                    let idx = i - start;
                    let sb = if total_entries <= vh { " " }
                    else {
                        let thumb = (content_lines * vh / total_entries).max(1);
                        let pos = browser.scroll * vh / total_entries;
                        if idx >= pos && idx < pos + thumb { "\x1B[48;5;239m \x1B[0m" }
                        else { " " }
                    };
                    print!("{}{}\x1B[K\r\n", entry_str, sb);
                }
                let used = end - start;
                for idx in used..content_lines {
                    let sb = if total_entries <= vh { " " }
                    else {
                        let thumb = (content_lines * vh / total_entries).max(1);
                        let pos = browser.scroll * vh / total_entries;
                        if idx >= pos && idx < pos + thumb { "\x1B[48;5;239m \x1B[0m" }
                        else { " " }
                    };
                    print!("{}\x1B[K\r\n", sb);
                }
            }
        }

        // status bar
        let status = if cmd_mode {
            if rename_prompt {
                let old = rename_target.as_ref().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
                format!("Rename: {} → {}", old, rename_buf)
            } else if newfile_prompt { format!("New file: {}", newfile_name) } else { format!(":{}", cmd_buf) }
        } else {
            match &tabs[active_tab] {
                Tab::File(file) => {
                    if file.search_mode {
                        let all_matches = find_all_matches(&file.lines, &file.search_q);
                        let total = all_matches.len();
                        let idx = all_matches.iter().position(|m| *m == file.cursor).map(|i| i + 1).unwrap_or(0);
                        format!("/ {}  ({}/{})", file.search_q, idx, total)
                    } else {
                        let finfo = file.path.as_deref().unwrap_or("[No Name]");
                        let dm = if file.dirty { " [+]" } else { "" };
                        let sm = if file.selecting { " [Sel]" } else { "" };
                        format!("{}{}{} | Ln {} Col {}", finfo, dm, sm, file.cursor.y + 1, file.cursor.x + 1)
                    }
                }
                Tab::Browser(_) => {
                    format!("Tab {} of {} | Ctrl+N:browser  Ctrl+←/→:switch  Ctrl+↑+←/→:move  Esc:cmd", active_tab + 1, tabs.len())
                }
            }
        };
        let sp = vw.saturating_sub(status.chars().count());
        print!("\x1B[{};1H\x1B[7m{}{}\x1B[0m\x1B[K", th, " ".repeat(sp), status);

        // cursor
        match &tabs[active_tab] {
            Tab::File(file) => {
                let vr = file.cursor.y - row_off;
                let vc = prefix_len + file.cursor.x - col_off;
                print!("\x1B[{};{}H\x1B[?25h", vr + 2, vc + 1);
            }
            Tab::Browser(browser) => {
                let cursor_row = browser.cursor - browser.scroll + 1;
                print!("\x1B[{};1H\x1B[?25h", cursor_row + 1);
            }
        }

        io::stdout().flush()?;

        match event::read()? {
            Event::Key(k) => {
            let ctrl = KeyModifiers::CONTROL;
            let shift = KeyModifiers::SHIFT;
            let mods = k.modifiers;

            if cmd_mode {
                match k.code {
                    KeyCode::Char(c) => {
                        if rename_prompt { rename_buf.push(c); }
                        else if newfile_prompt { newfile_name.push(c); } else { cmd_buf.push(c); }
                    }
                    KeyCode::Backspace => {
                        if rename_prompt { rename_buf.pop(); }
                        else if newfile_prompt { newfile_name.pop(); } else { cmd_buf.pop(); }
                    }
                    KeyCode::Up if !rename_prompt && !newfile_prompt => {
                        let idx = cmd_history_idx.map(|i| i.saturating_sub(1)).or(Some(cmd_history.len().saturating_sub(1)));
                        if let Some(i) = idx {
                            if i < cmd_history.len() {
                                cmd_buf = cmd_history[i].clone();
                                cmd_history_idx = Some(i);
                            }
                        }
                    }
                    KeyCode::Down if !rename_prompt && !newfile_prompt => {
                        cmd_history_idx = cmd_history_idx.and_then(|i| {
                            let next = i + 1;
                            if next < cmd_history.len() {
                                cmd_buf = cmd_history[next].clone();
                                Some(next)
                            } else {
                                cmd_buf.clear();
                                None
                            }
                        });
                    }
                    KeyCode::Enter => {
                        if rename_prompt && !rename_buf.is_empty() {
                            if let (Some(target), Tab::Browser(ref mut browser)) = (rename_target.take(), &mut tabs[active_tab]) {
                                let new_path = browser.dir.join(&rename_buf);
                                if fs::rename(&target, &new_path).is_ok() {
                                    browser.entries = scan_dir(&browser.dir).unwrap_or_default();
                                }
                            }
                            rename_buf.clear();
                        } else if newfile_prompt && !newfile_name.is_empty() {
                            if let Tab::Browser(ref mut browser) = tabs[active_tab] {
                                let new_path = browser.dir.join(&newfile_name);
                                if fs::write(&new_path, "").is_ok() {
                                    browser.entries = scan_dir(&browser.dir).unwrap_or_default();
                                }
                            }
                            newfile_prompt = false;
                            newfile_name.clear();
                        } else if !cmd_buf.is_empty() && !rename_prompt && !newfile_prompt {
                            if cmd_history.last().map_or(true, |last| *last != cmd_buf) {
                                cmd_history.push(cmd_buf.clone());
                            }
                            cmd_history_idx = None;
                            if let Ok(line_num) = cmd_buf.parse::<usize>() {
                                if let Tab::File(ref mut file) = tabs[active_tab] {
                                    if line_num > 0 && line_num <= file.lines.len() {
                                        file.cursor.y = line_num - 1;
                                        file.cursor.x = 0;
                                        file.selecting = false;
                                    }
                                }
                            } else {
                                match cmd_buf.as_str() {
                                    "numlist" => show_numbers = !show_numbers,
                                "q" => break,
                                "w" => {
                                    if let Tab::File(ref mut file) = tabs[active_tab] {
                                        if let Some(ref path) = file.path {
                                            if let Some(parent) = std::path::Path::new(path).parent() {
                                                let _ = fs::create_dir_all(parent);
                                            }
                                            if let Ok(mut f) = File::create(path) {
                                                let _ = write!(f, "{}", file.lines.join("\n"));
                                                file.dirty = false;
                                                file.snapshot = file.lines.join("\n");
                                            }
                                        }
                                    }
                                }
                                "wq" => {
                                    if let Tab::File(ref mut file) = tabs[active_tab] {
                                        if let Some(ref path) = file.path {
                                            if let Some(parent) = std::path::Path::new(path).parent() {
                                                let _ = fs::create_dir_all(parent);
                                            }
                                            if let Ok(mut f) = File::create(path) {
                                                let _ = write!(f, "{}", file.lines.join("\n"));
                                            }
                                        }
                                    }
                                    break;
                                }
                                cmd if cmd.starts_with("set ") => {
                                    let rest = cmd[4..].trim();
                                    if let Some(pos) = rest.find(' ') {
                                        let key = rest[..pos].trim();
                                        let val = rest[pos+1..].trim();
                                        match key {
                                            "show_numbers" => show_numbers = val == "true" || val == "on" || val == "1",
                                            "tab_width" => if let Ok(n) = val.parse::<usize>() { if n >= 1 { tab_width = n; } }
                                            "theme" => {
                                                current_theme = load_theme(val);
                                                if current_theme.is_some() { save_theme_setting(val); }
                                            }
                                            _ if key.starts_with("bind_") => {
                                                let action = &key[5..];
                                                if let Some(combo) = parse_key_combo(val) {
                                                    bindings.insert(action.to_string(), combo);
                                                    let mut cfg = load_config_raw();
                                                    cfg.insert(key.to_string(), val.to_string());
                                                    save_config_raw(&cfg);
                                                }
                                            }
                                            _ => {}
                                        }
                                        let mut cfg = load_config_raw();
                                        cfg.insert("show_numbers".to_string(), if show_numbers { "true".to_string() } else { "false".to_string() });
                                        cfg.insert("tab_width".to_string(), tab_width.to_string());
                                        save_config_raw(&cfg);
                                    } else {
                                        match rest {
                                            "show_numbers" => cmd_buf = format!("show_numbers = {}", show_numbers),
                                            "tab_width" => cmd_buf = format!("tab_width = {}", tab_width),
                                            "theme" => cmd_buf = format!("theme = {}", current_theme.as_ref().map(|t| &t.name).unwrap_or(&"none".to_string())),
                                            _ => {}
                                        }
                                    }
                                }
                                "help" => {
                                    let mut text = "\
=== Commands ===\n:q        - quit\n:w        - save file\n:wq       - save and quit\n\
:numlist  - toggle line numbers\n:set <key> <value>  - set config option\n\
:theme <name>       - apply theme\n:mktheme <name>     - create/edit theme\n\
:edtheme <name>     - open theme for editing\n:<number>           - go to line\n\
:help               - this help\n\
\n=== Key Bindings (configurable via :set bind_<action> <key>) ===\n\
\nAction          Default         Current\n──────────────────────────────────────────\n\
".to_string();
                                    for (name, def) in default_bindings() {
                                        let cur = &bindings[name];
                                        let def_s = key_combo_to_string(&def);
                                        let cur_s = key_combo_to_string(cur);
                                        text.push_str(&format!("{:<16} {:<16} {}\n", name, def_s,
                                            if cur_s == def_s { String::new() } else { cur_s }));
                                    }
                                    text.push_str("\nConfig options: show_numbers (bool), tab_width (1+), theme (name)\nBind actions: ");
                                    for (name, _) in default_bindings() { text.push_str(name); text.push_str(", "); }
                                    text.pop(); text.pop(); text.push('\n');
                                    let lines: Vec<String> = text.lines().map(String::from).collect();
                                    tabs.push(Tab::File(FileState {
                                        path: None, lines, cursor: Pos { x: 0, y: 0 }, dirty: false, snapshot: text,
                                        undo: Vec::new(), redo: Vec::new(), selecting: false, sel_start: Pos { x: 0, y: 0 },
                                        search_q: String::new(), search_mode: false,
                                    }));
                                    active_tab = tabs.len() - 1;
                                    row_off = 0;
                                    col_off = 0;
                                }
                                _ => {}
                            }
                        }
                        cmd_mode = false;
                        cmd_buf.clear();
                        rename_prompt = false;
                        rename_buf.clear();
                        rename_target = None;
                    }
                    }
                    KeyCode::Esc => { cmd_mode = false; cmd_buf.clear(); newfile_prompt = false; newfile_name.clear(); rename_prompt = false; rename_buf.clear(); rename_target = None; }
                    _ => {}
                }
                continue;
            }

            if let Some(action) = find_action(&bindings, &k.code, mods) {
                match action {
                    "reorder" => { reorder_mode = !reorder_mode; continue; }
                    "tab_prev" | "tab_move_left" => {
                        if reorder_mode {
                            if active_tab > 0 { tabs.swap(active_tab, active_tab - 1); active_tab -= 1; }
                        } else {
                            active_tab = if active_tab == 0 { tabs.len() - 1 } else { active_tab - 1 };
                        }
                        reorder_mode = false;
                        continue;
                    }
                    "tab_next" | "tab_move_right" => {
                        if reorder_mode {
                            if active_tab + 1 < tabs.len() { tabs.swap(active_tab, active_tab + 1); active_tab += 1; }
                        } else {
                            active_tab = (active_tab + 1) % tabs.len();
                        }
                        reorder_mode = false;
                        continue;
                    }
                    _ => {}
                }
                reorder_mode = false;
            } else if !mods.contains(ctrl) && !mods.contains(KeyModifiers::ALT) {
                reorder_mode = false;
            }

            match &mut tabs[active_tab] {
                Tab::File(file) => {
                    if file.search_mode {
                        match k.code {
                            KeyCode::Char(c) => {
                                file.search_q.push(c);
                                if let Some(m) = find_next_match(&file.lines, &file.search_q, &file.cursor) {
                                    file.cursor = m;
                                }
                            }
                            KeyCode::Backspace => {
                                file.search_q.pop();
                                if !file.search_q.is_empty() {
                                    if let Some(m) = find_next_match(&file.lines, &file.search_q, &file.cursor) {
                                        file.cursor = m;
                                    }
                                }
                            }
                            KeyCode::Up => {
                                let from = Pos { x: file.cursor.x.saturating_sub(1), y: file.cursor.y };
                                if let Some(m) = find_prev_match(&file.lines, &file.search_q, &from) {
                                    file.cursor = m;
                                }
                            }
                            KeyCode::Down => {
                                let from = Pos { x: file.cursor.x + 1, y: file.cursor.y };
                                if let Some(m) = find_next_match(&file.lines, &file.search_q, &from) {
                                    file.cursor = m;
                                }
                            }
                            KeyCode::Enter | KeyCode::Esc if mods == KeyModifiers::NONE => {
                                if k.code == KeyCode::Esc { file.search_q.clear(); }
                                file.search_mode = false;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if let Some(action) = find_action(&bindings, &k.code, mods) {
                        match action {
                            "quit" => break,
                            "save" => {
                                if let Some(ref path) = file.path {
                                    if let Ok(mut f) = File::create(path) {
                                        if write!(f, "{}", file.lines.join("\n")).is_ok() {
                                            file.dirty = false;
                                            file.snapshot = file.lines.join("\n");
                                        }
                                    }
                                }
                            }
                            "undo" => {
                                if !file.undo.is_empty() {
                                    file.selecting = false;
                                    file.redo.push(file.lines.clone());
                                    file.lines = file.undo.pop().unwrap();
                                    if file.cursor.y >= file.lines.len() { file.cursor.y = file.lines.len().saturating_sub(1); }
                                    let cc = file.lines[file.cursor.y].chars().count();
                                    if file.cursor.x > cc { file.cursor.x = cc; }
                                    file.dirty = true;
                                    last_action = Action::None;
                                }
                            }
                            "redo" => {
                                if !file.redo.is_empty() {
                                    file.selecting = false;
                                    file.undo.push(file.lines.clone());
                                    file.lines = file.redo.pop().unwrap();
                                    if file.cursor.y >= file.lines.len() { file.cursor.y = file.lines.len().saturating_sub(1); }
                                    let cc = file.lines[file.cursor.y].chars().count();
                                    if file.cursor.x > cc { file.cursor.x = cc; }
                                    file.dirty = true;
                                    last_action = Action::None;
                                }
                            }
                            "search" => {
                                file.search_mode = true;
                                file.search_q.clear();
                            }
                            "copy" => {
                                if file.selecting {
                                    clipboard = collect_selection_text(&file.lines, &file.sel_start, &file.cursor);
                                    file.selecting = false;
                                } else {
                                    clipboard = vec![file.lines[file.cursor.y].clone()];
                                }
                                sys_clipboard_set(&clipboard.join("\n"));
                                last_action = Action::None;
                            }
                            "cut" => {
                                let same = last_action == Action::Other && last_action != Action::None;
                                if !same { file.undo.push(file.lines.clone()); }
                                last_action = Action::Other;
                                file.redo.clear();
                                if file.selecting {
                                    clipboard = collect_selection_text(&file.lines, &file.sel_start, &file.cursor);
                                    file.cursor = delete_selection(&mut file.lines, &file.sel_start, &file.cursor);
                                    file.selecting = false;
                                } else {
                                    clipboard = vec![file.lines[file.cursor.y].clone()];
                                    if file.lines.len() > 1 {
                                        file.lines.remove(file.cursor.y);
                                        if file.cursor.y >= file.lines.len() { file.cursor.y = file.lines.len() - 1; }
                                    } else {
                                        file.lines[0].clear();
                                    }
                                    file.cursor.x = 0;
                                }
                                sys_clipboard_set(&clipboard.join("\n"));
                                file.dirty = true;
                            }
                            "paste" => {
                                let sys_text = sys_clipboard_get().filter(|t| !t.is_empty());
                                let paste_lines = sys_text
                                    .as_ref()
                                    .map(|t| t.lines().map(|l| l.to_string()).collect())
                                    .or_else(|| if !clipboard.is_empty() { Some(clipboard.clone()) } else { None });

                                if let Some(ref paste_lines) = paste_lines {
                                    let same = last_action == Action::Other && last_action != Action::None;
                                    if !same { file.undo.push(file.lines.clone()); }
                                    last_action = Action::Other;
                                    file.redo.clear();
                                    if file.selecting {
                                        file.cursor = delete_selection(&mut file.lines, &file.sel_start, &file.cursor);
                                        file.selecting = false;
                                    }
                                    if paste_lines.len() == 1 {
                                        let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                        file.lines[file.cursor.y].insert_str(b, &paste_lines[0]);
                                        file.cursor.x += paste_lines[0].chars().count();
                                    } else {
                                        let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                        let rest = file.lines[file.cursor.y].split_off(b);
                                        file.lines[file.cursor.y].push_str(&paste_lines[0]);
                                        for i in 1..paste_lines.len() {
                                            file.lines.insert(file.cursor.y + i, paste_lines[i].clone());
                                        }
                                        let last_line_idx = file.cursor.y + paste_lines.len() - 1;
                                        file.lines[last_line_idx].push_str(&rest);
                                        file.cursor.y = last_line_idx;
                                        file.cursor.x = paste_lines.last().unwrap().chars().count();
                                    }
                                    file.dirty = true;
                                }
                            }
                            "select_all" => {
                                file.selecting = true;
                                file.sel_start = Pos { x: 0, y: 0 };
                                file.cursor.y = file.lines.len() - 1;
                                file.cursor.x = file.lines[file.cursor.y].chars().count();
                            }
                            "delete_line" => {
                                file.selecting = false;
                                let same = last_action == Action::Other && last_action != Action::None;
                                if !same { file.undo.push(file.lines.clone()); }
                                last_action = Action::Other;
                                file.redo.clear();
                                if file.lines.len() > 1 {
                                    file.lines.remove(file.cursor.y);
                                    if file.cursor.y >= file.lines.len() { file.cursor.y = file.lines.len() - 1; }
                                } else {
                                    file.lines[0].clear();
                                }
                                file.cursor.x = 0;
                                file.dirty = true;
                            }
                            "clear_buffer" => {
                                file.selecting = false;
                                let same = last_action == Action::Other && last_action != Action::None;
                                if !same { file.undo.push(file.lines.clone()); }
                                last_action = Action::Other;
                                file.redo.clear();
                                file.lines.clear();
                                file.lines.push(String::new());
                                file.cursor = Pos { x: 0, y: 0 };
                                file.dirty = true;
                            }
                            "browser" => {
                                let has_browser = tabs.iter().any(|t| matches!(t, Tab::Browser(_)));
                                if has_browser {
                                    active_tab = tabs.iter().position(|t| matches!(t, Tab::Browser(_))).unwrap();
                                } else {
                                    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                                    let entries = scan_dir(&cwd).unwrap_or_default();
                                    tabs.push(Tab::Browser(BrowserState {
                                        dir: cwd,
                                        entries,
                                        cursor: 0,
                                        scroll: 0,
                                        history: Vec::new(),
                                    }));
                                    active_tab = tabs.len() - 1;
                                }
                            }
                            "close_tab" => {
                                if tabs.len() > 1 {
                                    tabs.remove(active_tab);
                                    if active_tab >= tabs.len() { active_tab = tabs.len() - 1; }
                                }
                            }
                            "home_file" => {
                                last_action = Action::None;
                                file.selecting = false;
                                file.cursor.y = 0;
                                file.cursor.x = 0;
                            }
                            "end_file" => {
                                last_action = Action::None;
                                file.selecting = false;
                                file.cursor.y = file.lines.len() - 1;
                                file.cursor.x = file.lines[file.cursor.y].chars().count();
                            }
                            "save_quit" => {
                                if let Some(ref path) = file.path {
                                    if let Ok(mut f) = File::create(path) {
                                        let _ = write!(f, "{}", file.lines.join("\n"));
                                    }
                                }
                                break;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if k.code == KeyCode::Esc && mods == KeyModifiers::NONE {
                        cmd_mode = true;
                        cmd_buf.clear();
                        continue;
                    }

                    match k.code {
                        KeyCode::Up => {
                            nav_cursor(mods, &mut last_action, file);
                            if file.cursor.y > 0 { file.cursor.y -= 1; }
                            let cc = file.lines[file.cursor.y].chars().count();
                            if file.cursor.x > cc { file.cursor.x = cc; }
                        }
                        KeyCode::Down => {
                            nav_cursor(mods, &mut last_action, file);
                            if file.cursor.y < file.lines.len() - 1 { file.cursor.y += 1; }
                            let cc = file.lines[file.cursor.y].chars().count();
                            if file.cursor.x > cc { file.cursor.x = cc; }
                        }
                        KeyCode::Left => {
                            nav_cursor(mods, &mut last_action, file);
                            if file.cursor.x > 0 { file.cursor.x -= 1; }
                            else if file.cursor.y > 0 { file.cursor.y -= 1; file.cursor.x = file.lines[file.cursor.y].chars().count(); }
                        }
                        KeyCode::Right => {
                            nav_cursor(mods, &mut last_action, file);
                            let cc = file.lines[file.cursor.y].chars().count();
                            if file.cursor.x < cc { file.cursor.x += 1; }
                            else if file.cursor.y < file.lines.len() - 1 { file.cursor.y += 1; file.cursor.x = 0; }
                        }
                        KeyCode::Home => {
                            nav_cursor(mods, &mut last_action, file);
                            file.cursor.x = 0;
                        }
                        KeyCode::End => {
                            nav_cursor(mods, &mut last_action, file);
                            file.cursor.x = file.lines[file.cursor.y].chars().count();
                        }
                        KeyCode::PageUp => {
                            nav_cursor(mods, &mut last_action, file);
                            let page = vh.saturating_sub(1);
                            if file.cursor.y > page { file.cursor.y -= page; } else { file.cursor.y = 0; }
                            let cc = file.lines[file.cursor.y].chars().count();
                            if file.cursor.x > cc { file.cursor.x = cc; }
                        }
                        KeyCode::PageDown => {
                            nav_cursor(mods, &mut last_action, file);
                            let page = vh.saturating_sub(1);
                            let last = file.lines.len() - 1;
                            if file.cursor.y + page < last { file.cursor.y += page; } else { file.cursor.y = last; }
                            let cc = file.lines[file.cursor.y].chars().count();
                            if file.cursor.x > cc { file.cursor.x = cc; }
                        }

                        KeyCode::Backspace if mods.contains(ctrl) => {
                            file.selecting = false;
                            let same = last_action == Action::Delete && last_action != Action::None;
                            if !same { file.undo.push(file.lines.clone()); }
                            last_action = Action::Delete;
                            file.redo.clear();
                            if file.cursor.x > 0 {
                                let chars: Vec<char> = file.lines[file.cursor.y].chars().collect();
                                let mut start = file.cursor.x;
                                while start > 0 && chars[start - 1] == ' ' { start -= 1; }
                                while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') { start -= 1; }
                                for _ in 0..(file.cursor.x - start) {
                                    let b = byte_idx(&file.lines[file.cursor.y], start);
                                    file.lines[file.cursor.y].remove(b);
                                }
                                file.cursor.x = start;
                                file.dirty = true;
                            } else if file.cursor.y > 0 {
                                let cur = file.lines.remove(file.cursor.y);
                                file.cursor.y -= 1;
                                file.cursor.x = file.lines[file.cursor.y].chars().count();
                                file.lines[file.cursor.y].push_str(&cur);
                                file.dirty = true;
                            }
                        }

                        _ => {
                            if mods.intersects(ctrl) { continue; }

                            if file.selecting {
                                let same = last_action == Action::Other && last_action != Action::None;
                                if !same { file.undo.push(file.lines.clone()); }
                                last_action = Action::Other;
                                file.redo.clear();
                                file.cursor = delete_selection(&mut file.lines, &file.sel_start, &file.cursor);
                                file.selecting = false;
                            }

                            match k.code {
                                KeyCode::Char(ch) => {
                                    let same = last_action == Action::Insert && last_action != Action::None;
                                    if !same { file.undo.push(file.lines.clone()); }
                                    last_action = Action::Insert;
                                    file.redo.clear();
                                    let pair = match ch {
                                        '(' => Some(')'),
                                        '{' => Some('}'),
                                        '[' => Some(']'),
                                        '"' if file.cursor.x == 0 || !file.lines[file.cursor.y].chars().nth(file.cursor.x - 1).map_or(false, |c| c.is_alphanumeric()) => Some('"'),
                                        '\'' if file.cursor.x == 0 || !file.lines[file.cursor.y].chars().nth(file.cursor.x - 1).map_or(false, |c| c.is_alphanumeric()) => Some('\''),
                                        _ => None,
                                    };
                                    if let Some(close) = pair {
                                        let next = file.lines[file.cursor.y].chars().nth(file.cursor.x);
                                        if next == Some(close) && ch != '"' && ch != '\'' {
                                            file.cursor.x += 1;
                                        } else {
                                            let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                            file.lines[file.cursor.y].insert(b, ch);
                                            file.lines[file.cursor.y].insert(b + 1, close);
                                            file.cursor.x += 1;
                                            file.dirty = true;
                                        }
                                    } else {
                                        if file.selecting {
                                            file.cursor = delete_selection(&mut file.lines, &file.sel_start, &file.cursor);
                                            file.selecting = false;
                                        }
                                        let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                        file.lines[file.cursor.y].insert(b, ch);
                                        file.cursor.x += 1;
                                        file.dirty = true;
                                    }
                                }
                                KeyCode::BackTab | KeyCode::Tab if mods.contains(shift) => {
                                    if file.selecting {
                                        let same = last_action == Action::Other && last_action != Action::None;
                                        if !same { file.undo.push(file.lines.clone()); }
                                        last_action = Action::Other;
                                        file.redo.clear();
                                        let top_y = file.sel_start.y.min(file.cursor.y);
                                        let bot_y = file.sel_start.y.max(file.cursor.y);
                                        for i in top_y..=bot_y {
                                            let remove = file.lines[i].chars().take(tab_width).take_while(|&c| c == ' ').count();
                                            for _ in 0..remove {
                                                file.lines[i].remove(0);
                                            }
                                        }
                                        file.dirty = true;
                                    }
                                }
                                KeyCode::Tab => {
                                    if file.selecting {
                                        let same = last_action == Action::Other && last_action != Action::None;
                                        if !same { file.undo.push(file.lines.clone()); }
                                        last_action = Action::Other;
                                        file.redo.clear();
                                        let indent = " ".repeat(tab_width);
                                        let top_y = file.sel_start.y.min(file.cursor.y);
                                        let bot_y = file.sel_start.y.max(file.cursor.y);
                                        for i in top_y..=bot_y {
                                            file.lines[i].insert_str(0, &indent);
                                        }
                                        file.cursor.x = file.cursor.x.saturating_add(tab_width);
                                        file.dirty = true;
                                    } else {
                                        let same = last_action == Action::Other && last_action != Action::None;
                                        if !same { file.undo.push(file.lines.clone()); }
                                        last_action = Action::Other;
                                        file.redo.clear();
                                        let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                        file.lines[file.cursor.y].insert_str(b, &" ".repeat(tab_width));
                                        file.cursor.x = file.cursor.x.saturating_add(tab_width);
                                        file.dirty = true;
                                    }
                                }
                                KeyCode::Enter => {
                                    let same = last_action == Action::Other && last_action != Action::None;
                                    if !same { file.undo.push(file.lines.clone()); }
                                    last_action = Action::Other;
                                    file.redo.clear();
                                    let indent: String = file.lines[file.cursor.y].chars().take_while(|&c| c == ' ').collect();
                                    let indent_len = indent.len();
                                    let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                    let rest = file.lines[file.cursor.y].split_off(b);
                                    file.lines.insert(file.cursor.y + 1, indent + &rest);
                                    file.cursor.y += 1;
                                    file.cursor.x = indent_len;
                                    file.dirty = true;
                                }
                                KeyCode::Backspace => {
                                    if file.cursor.x > 0 || file.cursor.y > 0 {
                                        let same = last_action == Action::Delete && last_action != Action::None;
                                        if !same { file.undo.push(file.lines.clone()); }
                                        last_action = Action::Delete;
                                        file.redo.clear();
                                    }
                                    if file.cursor.x > 0 {
                                        let l = &file.lines[file.cursor.y].clone();
                                        let chars: Vec<char> = l.chars().collect();
                                        let mut n = 1;
                                        if file.cursor.x >= 4 && chars[file.cursor.x - 4..file.cursor.x].iter().all(|&c| c == ' ') {
                                            n = 4;
                                        }
                                        for _ in 0..n {
                                            let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x - 1);
                                            file.lines[file.cursor.y].remove(b);
                                            file.cursor.x -= 1;
                                        }
                                        file.dirty = true;
                                    } else if file.cursor.y > 0 {
                                        let cur = file.lines.remove(file.cursor.y);
                                        file.cursor.y -= 1;
                                        file.cursor.x = file.lines[file.cursor.y].chars().count();
                                        file.lines[file.cursor.y].push_str(&cur);
                                        file.dirty = true;
                                    }
                                }
                                KeyCode::Delete => {
                                    let cc = file.lines[file.cursor.y].chars().count();
                                    if file.cursor.x < cc || file.cursor.y < file.lines.len() - 1 {
                                        let same = last_action == Action::Delete && last_action != Action::None;
                                        if !same { file.undo.push(file.lines.clone()); }
                                        last_action = Action::Delete;
                                        file.redo.clear();
                                    }
                                    if file.cursor.x < cc {
                                        let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                        file.lines[file.cursor.y].remove(b);
                                        file.dirty = true;
                                    } else if file.cursor.y < file.lines.len() - 1 {
                                        let next = file.lines.remove(file.cursor.y + 1);
                                        file.lines[file.cursor.y].push_str(&next);
                                        file.dirty = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }

                Tab::Browser(browser) => {
                    if let Some(action) = find_action(&bindings, &k.code, mods) {
                        match action {
                            "quit" => break,
                            "close_tab" => {
                                if tabs.len() > 1 {
                                    tabs.remove(active_tab);
                                    if active_tab >= tabs.len() { active_tab = tabs.len() - 1; }
                                }
                            }
                            "browser_back" => {
                                if let Some(prev) = browser.history.pop() {
                                    browser.dir = prev;
                                    browser.entries = scan_dir(&browser.dir).unwrap_or_default();
                                    browser.cursor = 0;
                                    browser.scroll = 0;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    match k.code {

                        KeyCode::Up => {
                            if browser.cursor > 0 { browser.cursor -= 1; }
                        }

                        KeyCode::Down => {
                            let max = browser.entries.len() + 1;
                            if browser.cursor < max { browser.cursor += 1; }
                        }

                        KeyCode::Enter => {
                            let total = browser.entries.len() + 2;
                            if browser.cursor == 0 {
                                if let Some(parent) = browser.dir.parent().map(|p| p.to_path_buf()) {
                                    browser.history.push(browser.dir.clone());
                                    browser.dir = parent;
                                    browser.entries = scan_dir(&browser.dir).unwrap_or_default();
                                    browser.cursor = 0;
                                    browser.scroll = 0;
                                }
                            } else if browser.cursor == total - 1 {
                                cmd_mode = true;
                                newfile_prompt = true;
                                newfile_name.clear();
                            } else {
                                let idx = browser.cursor - 1;
                                if idx < browser.entries.len() {
                                    let (ref name, is_dir) = browser.entries[idx].clone();
                                    let full_path = browser.dir.join(&name);
                                    if is_dir {
                                        browser.history.push(browser.dir.clone());
                                        browser.dir = full_path;
                                        browser.entries = scan_dir(&browser.dir).unwrap_or_default();
                                        browser.cursor = 0;
                                        browser.scroll = 0;
                                    } else {
                                        if let Ok(content) = fs::read_to_string(&full_path) {
                                            let lines: Vec<String> = content.lines().map(String::from).collect();
                                            let lines = if lines.is_empty() { vec![String::new()] } else { lines };
                                            tabs.push(Tab::File(FileState {
                                                path: Some(full_path.to_string_lossy().to_string()),
                                                lines,
                                                cursor: Pos { x: 0, y: 0 },
                                                dirty: false,
                                                snapshot: content.clone(),
                                                undo: Vec::new(),
                                                redo: Vec::new(),
                                                selecting: false,
                                                sel_start: Pos { x: 0, y: 0 },
                                                search_q: String::new(),
                                                search_mode: false,
                                            }));
                                            active_tab = tabs.len() - 1;
                                            row_off = 0;
                                            col_off = 0;
                                        }
                                    }
                                }
                            }
                        }

                        KeyCode::Delete => {
                            let total = browser.entries.len() + 2;
                            if browser.cursor > 0 && browser.cursor < total - 1 {
                                let idx = browser.cursor - 1;
                                if idx < browser.entries.len() {
                                    let (name, is_dir) = &browser.entries[idx];
                                    let full_path = browser.dir.join(name);
                                    let ok = if *is_dir { fs::remove_dir_all(&full_path).is_ok() } else { fs::remove_file(&full_path).is_ok() };
                                    if ok {
                                        browser.entries = scan_dir(&browser.dir).unwrap_or_default();
                                        let max = browser.entries.len() + 1;
                                        if browser.cursor > max { browser.cursor = max; }
                                    }
                                }
                            }
                        }

                        KeyCode::Char('r') if mods == KeyModifiers::NONE => {
                            let total = browser.entries.len() + 2;
                            if browser.cursor > 0 && browser.cursor < total - 1 {
                                let idx = browser.cursor - 1;
                                if idx < browser.entries.len() {
                                    let (name, _) = &browser.entries[idx];
                                    rename_target = Some(browser.dir.join(name));
                                    rename_buf = name.clone();
                                    rename_prompt = true;
                                    cmd_mode = true;
                                    cmd_buf.clear();
                                }
                            }
                        }

                        KeyCode::Esc => {
                            cmd_mode = true;
                            cmd_buf.clear();
                        }

                        KeyCode::Tab if mods == KeyModifiers::NONE => {
                            active_tab = (active_tab + 1) % tabs.len();
                        }

                        KeyCode::BackTab | KeyCode::Tab if mods.contains(shift) => {
                            active_tab = if active_tab == 0 { tabs.len() - 1 } else { active_tab - 1 };
                        }

                        _ => {}
                    }
                }
            }
            }
            Event::Mouse(m) => {
                if !cmd_mode {
                    match &mut tabs[active_tab] {
                        Tab::File(file) => {
                            if !file.search_mode {
                                if m.kind == MouseEventKind::Down(MouseButton::Left) && m.row >= 2 {
                                    let r = (m.row - 2) as usize + row_off+1;
                                    if r < file.lines.len() {
                                        file.cursor.y = r;
                                        let cc = file.lines[r].chars().count();
                                        if m.column >= 1 {
                                            let col = (m.column as usize).saturating_sub(1).saturating_sub(prefix_len) + col_off;
                                            file.cursor.x = col.min(cc)+1;
                                        }
                                    }
                                } else if m.kind == MouseEventKind::ScrollUp {
                                    if row_off > 0 { row_off = row_off.saturating_sub(1); }
                                } else if m.kind == MouseEventKind::ScrollDown {
                                    let max_off = file.lines.len().saturating_sub(vh);
                                    if row_off < max_off { row_off += 1; }
                                }
                            }
                        }
                        Tab::Browser(browser) => {
                            if m.kind == MouseEventKind::Down(MouseButton::Left) && m.row >= 2 {
                                let cursor = (m.row - 2) as usize + browser.scroll - 1;
                                let max = browser.entries.len() + 1;
                                if cursor <= max {
                                    browser.cursor = cursor;
                                }
                            } else if m.kind == MouseEventKind::ScrollUp {
                                if browser.scroll > 0 { browser.scroll = browser.scroll.saturating_sub(1); }
                            } else if m.kind == MouseEventKind::ScrollDown {
                                let total = browser.entries.len() + 2;
                                let max_scroll = total.saturating_sub(vh.saturating_sub(1));
                                if browser.scroll < max_scroll { browser.scroll += 1; }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    execute!(io::stdout(), DisableMouseCapture)?;
    disable_raw_mode()?;
    Ok(())
}

fn sys_clipboard_set(text: &str) {
    let cmds: &[&[&str]] = &[
        &["xclip", "-selection", "clipboard"],
        &["wl-copy"],
        &["xsel", "-ib"],
    ];
    for cmd in cmds {
        if let Ok(mut child) = Command::new(cmd[0]).args(&cmd[1..]).stdin(std::process::Stdio::piped()).spawn() {
            let _ = child.stdin.take().map(|mut s| { let _ = s.write_all(text.as_bytes()); });
            let _ = child.wait();
            return;
        }
    }
}

fn sys_clipboard_get() -> Option<String> {
    let cmds: &[&[&str]] = &[
        &["xclip", "-selection", "clipboard", "-o"],
        &["wl-paste"],
        &["xsel", "-ob"],
    ];
    for cmd in cmds {
        if let Ok(out) = Command::new(cmd[0]).args(&cmd[1..]).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).to_string();
                if !s.is_empty() { return Some(s); }
            }
        }
    }
    None
}