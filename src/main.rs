mod config;
mod editor;
use crate::config::*;
use crate::editor::*;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseEventKind, MouseButton},
    terminal::{disable_raw_mode, enable_raw_mode, size},
    execute,
};
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
use std::process::Command;
use std::time::{Duration, Instant};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let arg_file = if args.len() > 1 { Some(args[1].clone()) } else { None };

    enable_raw_mode()?;
    execute!(io::stdout(), EnableMouseCapture)?;
    write!(io::stdout(), "\x1b[?2004h")?;

    let current_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut tabs: Vec<Tab> = if let Some(path) = &arg_file {
        let p = PathBuf::from(path);
        if p.exists() && p.is_file() {
            if let Ok(content) = fs::read_to_string(&p) {
                let lines: Vec<String> = content.lines().map(String::from).collect();
                let lines = if lines.is_empty() { vec![String::new()] } else { lines };
                vec![Tab::File(FileState::new(Some(path.clone()), lines, content.clone()))]
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
    if let Some(v) = cfg.get("tab_width") { if let Ok(n) = v.parse::<usize>() { if n >= 1 { tab_width = n; } } }
    let theme_name = cfg.get("theme").cloned().unwrap_or_else(|| "default".to_string());
    let mut current_theme: Option<Theme> = load_theme(&theme_name);
    let mut bindings = load_bindings(&cfg);
    let mut sysinfo_mode: u8 = 1;
    let mut completion_enabled = true;
    let mut sysinfo_cache = SysInfoCache::new();

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

        // scrolling — load from tab state
        if let Tab::File(ref file) = tabs[active_tab] {
            row_off = file.row_off;
            col_off = file.col_off;
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
                            file.clamp_cursor();
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
        // load block comment state from file
        let (mut in_block_comment, mut in_block_string) = match &tabs[active_tab] {
            Tab::File(f) => (f.block_comment, f.block_string),
            _ => (false, false),
        };

        // pre-scan from start of file to row_off for correct block state
        if let Tab::File(ref file) = tabs[active_tab] {
            if row_off > 0 {
                let (bc, bs) = prescan_block_state(&file.lines, row_off, false, false);
                in_block_comment = bc;
                in_block_string = bs;
            } else {
                in_block_comment = false;
                in_block_string = false;
            }
        }

        match &tabs[active_tab] {
            Tab::File(file) => {
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

                for r in 0..vh {
                    let li = r + row_off;
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
                        print!("{}\x1B[K\r\n", prefix + &rendered);
                    } else {
                        print!("\x1B[K\r\n");
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
                        render_browser_line("..", true, active, vw)
                    } else if i == total_entries - 1 {
                        render_browser_line("[New file]", false, active, vw)
                    } else {
                        let idx = i - 1;
                        let (name, is_dir) = &browser.entries[idx];
                        render_browser_line(name, *is_dir, active, vw)
                    };
                    print!("{}\x1B[K\r\n", entry_str);
                }
                for _ in (end - start)..content_lines {
                    print!("\x1B[K\r\n");
                }
            }
        }

        // save block state back to file
        if let Tab::File(ref mut f) = tabs[active_tab] {
            f.block_comment = in_block_comment;
            f.block_string = in_block_string;
        }

        // update search cache
        let (search_total, search_idx) = if let Tab::File(ref mut f) = tabs[active_tab] {
            if f.search_mode && !f.search_q.is_empty() && f.search_q != f.cached_search_q {
                let all = find_all_matches(&f.lines, &f.search_q);
                f.cached_total_matches = all.len();
                f.cached_current_match = all.iter().position(|m| *m == f.cursor).map(|i| i + 1).unwrap_or(0);
                f.cached_search_q = f.search_q.clone();
            }
            (f.cached_total_matches, f.cached_current_match)
        } else { (0, 0) };

        // status bar
        let mut status = if cmd_mode {
            if rename_prompt {
                let old = rename_target.as_ref().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
                format!("Rename: {} → {}", old, rename_buf)
            } else if newfile_prompt { format!("New file: {}", newfile_name) } else { format!(":{}", cmd_buf) }
        } else {
            match &tabs[active_tab] {
                Tab::File(file) => {
                    if file.search_mode {
                        format!("/ {}  ({}/{})", file.search_q, search_idx, search_total)
                    } else {
                        let dm = if file.dirty { " [+]" } else { "" };
                        let sm = if file.selecting { " [Sel]" } else { "" };
                        let cm = if completion_enabled { " [CMP]" } else { " [CMPOFF]" };
                        format!("{}{}{}  Ln {} Col {}", dm, sm, cm, file.cursor.y + 1, file.cursor.x + 1)
                    }
                }
                Tab::Browser(_) => {
                    format!("Tab {} of {} | Ctrl+N:browser  Ctrl+←/→:switch  Ctrl+↑+←/→:move  Esc:cmd", active_tab + 1, tabs.len())
                }
            }
        };
        if sysinfo_mode > 0 && !cmd_mode {
            if !matches!(tabs[active_tab], Tab::Browser(_)){
                refresh_sysinfo(&mut sysinfo_cache);
                let sys_str = format_sysinfo(&sysinfo_cache.cached, sysinfo_mode);
                if !sys_str.is_empty() {
                    status.push_str(" | ");
                    status.push_str(&sys_str);
                }
            }
        }
        let sp = vw.saturating_sub(status.chars().count());
        print!("\x1B[{};1H\x1B[7m{}{}\x1B[0m\x1B[K", th, " ".repeat(sp), status);

        // completion popup
        if completion_enabled {
            if let Tab::File(file) = &tabs[active_tab] {
                if file.completion_active && !file.completion_candidates.is_empty() {
                let max_items = 10.min(file.completion_candidates.len());
                let popup_width = file.completion_candidates.iter().take(max_items).map(|c| c.len()).max().unwrap_or(0) + 4;
                let popup_x = (prefix_len + file.cursor.x - col_off).min(vw.saturating_sub(popup_width));
                let cursor_row = file.cursor.y - row_off + 2;
                let th_u = th as usize;
                let start_row = if cursor_row + 1 + max_items <= th_u {
                    cursor_row + 1
                } else {
                    cursor_row.saturating_sub(max_items + 1).max(2)
                };
                for i in 0..max_items {
                    let row = start_row + i;
                    let item = &file.completion_candidates[if file.completion_idx >= max_items { file.completion_idx - max_items + 1 + i } else { i }];
                    let highlight = file.completion_candidates[file.completion_idx] == *item;
                    let padded = format!(" {:<width$} ", item, width = popup_width - 2);
                    print!("\x1B[{};{}H", row, popup_x + 1);
                    if highlight { print!("\x1B[7m"); }
                    print!("{}{}\x1B[0m", padded, "\x1B[K");
                }
            }
        }
        }

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

        // save scroll state back to tab
        if let Tab::File(ref mut file) = tabs[active_tab] {
            file.row_off = row_off;
            file.col_off = col_off;
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
                                "sysinfo" => sysinfo_mode = (sysinfo_mode + 1) % 2,
                                "q" => break,
                                "w" => {
                                    if let Tab::File(ref mut file) = tabs[active_tab] {
                                        file.save_to_disk();
                                    }
                                }
                                "wq" => {
                                    if let Tab::File(ref mut file) = tabs[active_tab] {
                                        file.save_to_disk();
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
                                    let text = help_content(&bindings);
                                    let lines: Vec<String> = text.lines().map(String::from).collect();
                                    tabs.push(Tab::File(FileState::new(None, lines, text)));
                                    active_tab = tabs.len() - 1;
                                    col_off = 0;
                                }
                                        cmd if cmd.starts_with("theme ") => {
                                    let name = cmd[6..].trim();
                                    if !name.is_empty() {
                                        current_theme = load_theme(name);
                                        if current_theme.is_some() { save_theme_setting(name); }
                                    }
                                }
                                cmd if cmd.starts_with("mktheme ") => {
                                    let name = cmd[8..].trim();
                                    if !name.is_empty() {
                                        let th_dir = themes_dir();
                                        let _ = fs::create_dir_all(&th_dir);
                                        let path = th_dir.join(format!("{}.theme", name));
                                        if !path.exists() {
                                            let tmpl = format!("{}: \"fn\" \"let\" \"mut\" \"pub\" \"struct\" \"enum\" \"impl\" \"use\" \"return\" \"if\" \"else\" \"for\" \"while\" \"match\"\ngreen: \"i32\" \"i64\" \"u32\" \"u64\" \"usize\" \"String\" \"Vec\" \"Result\" \"Option\" \"bool\"\nyellow: \"println\" \"format\"\ncyan: \"std\" \"main\" \"self\" \"super\"\n", name);
                                            let _ = fs::write(&path, &tmpl);
                                        }
                                        if let Ok(content) = fs::read_to_string(&path) {
                                            let lines: Vec<String> = content.lines().map(String::from).collect();
                                            let lines = if lines.is_empty() { vec![String::new()] } else { lines };
                                            tabs.push(Tab::File(FileState::new(
                                                Some(path.to_string_lossy().to_string()),
                                                lines, content,
                                            )));
                                            active_tab = tabs.len() - 1;
                                        }
                                    }
                                }
                                cmd if cmd.starts_with("edtheme ") => {
                                    let name = cmd[8..].trim();
                                    if !name.is_empty() {
                                        let path = themes_dir().join(format!("{}.theme", name));
                                        if path.exists() {
                                            if let Ok(content) = fs::read_to_string(&path) {
                                                let lines: Vec<String> = content.lines().map(String::from).collect();
                                                let lines = if lines.is_empty() { vec![String::new()] } else { lines };
                                                tabs.push(Tab::File(FileState::new(
                                                    Some(path.to_string_lossy().to_string()),
                                                    lines, content,
                                                )));
                                                active_tab = tabs.len() - 1;
                                            }
                                        }
                                    }
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

            let (norm_code, norm_mods) = normalize_key(&k.code, mods);
            if let Some(action) = find_action(&bindings, &norm_code, norm_mods) {
                match action {
                    "reorder" => { reorder_mode = !reorder_mode; continue; }
                    "tab_prev" | "tab_move_left" => {
                        if let Tab::File(ref mut f) = tabs[active_tab] { f.row_off = row_off; f.col_off = col_off; }
                        if reorder_mode {
                            if active_tab > 0 { tabs.swap(active_tab, active_tab - 1); active_tab -= 1; }
                        } else {
                            active_tab = if active_tab == 0 { tabs.len() - 1 } else { active_tab - 1 };
                        }
                        reorder_mode = false;
                        continue;
                    }
                    "tab_next" | "tab_move_right" => {
                        if let Tab::File(ref mut f) = tabs[active_tab] { f.row_off = row_off; f.col_off = col_off; }
                        if reorder_mode {
                            if active_tab + 1 < tabs.len() { tabs.swap(active_tab, active_tab + 1); active_tab += 1; }
                        } else {
                            active_tab = (active_tab + 1) % tabs.len();
                        }
                        reorder_mode = false;
                        continue;
                    }
                    "sysinfo" => { sysinfo_mode = (sysinfo_mode + 1) % 2; continue; }
                    "completion" => { completion_enabled = !completion_enabled; if !completion_enabled { if let Tab::File(ref mut f) = tabs[active_tab] { f.completion_active = false; f.completion_candidates.clear(); } } continue; }
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
                                file.cached_search_q.clear();
                            }
                            KeyCode::Backspace => {
                                file.search_q.pop();
                                if !file.search_q.is_empty() {
                                    if let Some(m) = find_next_match(&file.lines, &file.search_q, &file.cursor) {
                                        file.cursor = m;
                                    }
                                }
                                file.cached_search_q.clear();
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

                    let (norm_code, norm_mods) = normalize_key(&k.code, mods);
                    if let Some(action) = find_action(&bindings, &norm_code, norm_mods) {
                        match action {
                            "quit" => break,
                            "save" => file.save_to_disk(),
                            "undo" => {
                                if !file.undo.is_empty() {
                                    file.selecting = false;
                                    file.redo.push(file.lines.clone());
                                    file.lines = file.undo.pop().unwrap();
                                    file.clamp_cursor();
                                    file.dirty = true;
                                    last_action = Action::None;
                                }
                            }
                            "redo" => {
                                if !file.redo.is_empty() {
                                    file.selecting = false;
                                    if file.undo.len() >= file.max_undo { file.undo.remove(0); }
                                    file.undo.push(file.lines.clone());
                                    file.lines = file.redo.pop().unwrap();
                                    file.clamp_cursor();
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
                                file.save_undo(&mut last_action, Action::Other);
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
                                let paste_lines: Vec<String> = sys_text
                                    .map(|t| t.lines().map(|l| l.replace('\r', "").replace('\t', &" ".repeat(tab_width))).collect())
                                    .or_else(|| if !clipboard.is_empty() { Some(clipboard.clone()) } else { None })
                                    .unwrap_or_default();
                                paste_text(file, &paste_lines, &mut last_action);
                            }
                            "select_all" => {
                                file.selecting = true;
                                file.sel_start = Pos { x: 0, y: 0 };
                                file.cursor.y = file.lines.len() - 1;
                                file.cursor.x = file.lines[file.cursor.y].chars().count();
                            }
                            "delete_line" => {
                                file.selecting = false;
                                file.save_undo(&mut last_action, Action::Other);
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
                                file.save_undo(&mut last_action, Action::Other);
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
                                file.save_to_disk();
                                break;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if file.completion_active && k.code == KeyCode::Esc && mods == KeyModifiers::NONE {
                        file.completion_active = false;
                        file.completion_candidates.clear();
                        continue;
                    }

                    if k.code == KeyCode::Esc && mods == KeyModifiers::NONE {
                        cmd_mode = true;
                        cmd_buf.clear();
                        continue;
                    }

                    match k.code {
                        KeyCode::Up => {
                            if file.completion_active {
                                if file.completion_idx > 0 { file.completion_idx -= 1; }
                                continue;
                            }
                            nav_cursor(mods, &mut last_action, file);
                            if file.cursor.y > 0 { file.cursor.y -= 1; }
                            file.clamp_cursor();
                        }
                        KeyCode::Down => {
                            if file.completion_active {
                                if file.completion_idx + 1 < file.completion_candidates.len() { file.completion_idx += 1; }
                                continue;
                            }
                            nav_cursor(mods, &mut last_action, file);
                            if file.cursor.y < file.lines.len() - 1 { file.cursor.y += 1; }
                            file.clamp_cursor();
                        }
                        KeyCode::Left => {
                            file.completion_active = false;
                            file.completion_candidates.clear();
                            nav_cursor(mods, &mut last_action, file);
                            if file.cursor.x > 0 { file.cursor.x -= 1; }
                            else if file.cursor.y > 0 { file.cursor.y -= 1; file.cursor.x = file.lines[file.cursor.y].chars().count(); }
                        }
                        KeyCode::Right => {
                            file.completion_active = false;
                            file.completion_candidates.clear();
                            nav_cursor(mods, &mut last_action, file);
                            let cc = file.lines[file.cursor.y].chars().count();
                            if file.cursor.x < cc { file.cursor.x += 1; }
                            else if file.cursor.y < file.lines.len() - 1 { file.cursor.y += 1; file.cursor.x = 0; }
                        }
                        KeyCode::Home => {
                            file.completion_active = false;
                            file.completion_candidates.clear();
                            nav_cursor(mods, &mut last_action, file);
                            file.cursor.x = 0;
                        }
                        KeyCode::End => {
                            file.completion_active = false;
                            file.completion_candidates.clear();
                            nav_cursor(mods, &mut last_action, file);
                            file.cursor.x = file.lines[file.cursor.y].chars().count();
                        }
                        KeyCode::PageUp => {
                            file.completion_active = false;
                            file.completion_candidates.clear();
                            nav_cursor(mods, &mut last_action, file);
                            let page = vh.saturating_sub(1);
                            if file.cursor.y > page { file.cursor.y -= page; } else { file.cursor.y = 0; }
                            file.clamp_cursor();
                        }
                        KeyCode::PageDown => {
                            file.completion_active = false;
                            file.completion_candidates.clear();
                            nav_cursor(mods, &mut last_action, file);
                            let page = vh.saturating_sub(1);
                            let last = file.lines.len() - 1;
                            if file.cursor.y + page < last { file.cursor.y += page; } else { file.cursor.y = last; }
                            file.clamp_cursor();
                        }

                        KeyCode::Backspace if mods.contains(ctrl) => {
                            file.selecting = false;
                            file.save_undo(&mut last_action, Action::Delete);
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
                            if completion_enabled { file.update_completion(); }
                        }

                        _ => {
                            if mods.intersects(ctrl) { continue; }

                            if file.selecting {
                                file.save_undo(&mut last_action, Action::Other);
                                file.cursor = delete_selection(&mut file.lines, &file.sel_start, &file.cursor);
                                file.selecting = false;
                            }

                            match k.code {
                                KeyCode::Char(ch) => {
                                    if (ch as u32) < 0x20 { continue; }
                                    file.save_undo(&mut last_action, Action::Insert);
                                    let pair = if file.pasting { None } else {
                                        match ch {
                                            '"' if file.cursor.x == 0 || !file.lines[file.cursor.y].chars().nth(file.cursor.x - 1).map_or(false, |c| c.is_alphanumeric()) => Some('"'),
                                            '\'' if file.cursor.x == 0 || !file.lines[file.cursor.y].chars().nth(file.cursor.x - 1).map_or(false, |c| c.is_alphanumeric()) => Some('\''),
                                            _ => None,
                                        }
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
                                    if completion_enabled && !file.pasting && (ch.is_alphanumeric() || ch == '_') {
                                        file.update_completion();
                                    } else {
                                        file.completion_active = false;
                                    }
                                }
                                KeyCode::BackTab | KeyCode::Tab if mods.contains(shift) => {
                                    if file.selecting {
                                        file.save_undo(&mut last_action, Action::Other);
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
                                    if file.completion_active {
                                        file.completion_active = false;
                                        file.completion_candidates.clear();
                                    }
                                    if file.selecting {
                                        file.save_undo(&mut last_action, Action::Other);
                                        let indent = " ".repeat(tab_width);
                                        let top_y = file.sel_start.y.min(file.cursor.y);
                                        let bot_y = file.sel_start.y.max(file.cursor.y);
                                        for i in top_y..=bot_y {
                                            file.lines[i].insert_str(0, &indent);
                                        }
                                        file.cursor.x = file.cursor.x.saturating_add(tab_width);
                                        file.dirty = true;
                                    } else {
                                        file.save_undo(&mut last_action, Action::Other);
                                        let b = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                        file.lines[file.cursor.y].insert_str(b, &" ".repeat(tab_width));
                                        file.cursor.x = file.cursor.x.saturating_add(tab_width);
                                        file.dirty = true;
                                    }
                                }
                                KeyCode::Enter => {
                                    if file.completion_active {
                                        file.save_undo(&mut last_action, Action::Other);
                                        if let Some(candidate) = file.completion_candidates.get(file.completion_idx) {
                                            let chars: Vec<char> = file.lines[file.cursor.y].chars().collect();
                                            let mut start = file.cursor.x;
                                            while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
                                                start -= 1;
                                            }
                                            let b_start = byte_idx(&file.lines[file.cursor.y], start);
                                            let b_end = byte_idx(&file.lines[file.cursor.y], file.cursor.x);
                                            file.lines[file.cursor.y].drain(b_start..b_end);
                                            let b = byte_idx(&file.lines[file.cursor.y], start);
                                            file.lines[file.cursor.y].insert_str(b, candidate);
                                            file.cursor.x = start + candidate.len();
                                            file.dirty = true;
                                        }
                                        file.completion_active = false;
                                        file.completion_candidates.clear();
                                        continue;
                                    }
                                    file.save_undo(&mut last_action, Action::Other);
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
                                        file.save_undo(&mut last_action, Action::Delete);
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
                                    if completion_enabled { file.update_completion(); }
                                }
                                KeyCode::Delete => {
                                    let cc = file.lines[file.cursor.y].chars().count();
                                    if file.cursor.x < cc || file.cursor.y < file.lines.len() - 1 {
                                        file.save_undo(&mut last_action, Action::Delete);
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
                    let (norm_code, norm_mods) = normalize_key(&k.code, mods);
                    if let Some(action) = find_action(&bindings, &norm_code, norm_mods) {
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
                                            tabs.push(Tab::File(FileState::new(
                                                Some(full_path.to_string_lossy().to_string()),
                                                lines, content.clone(),
                                            )));
                                            active_tab = tabs.len() - 1;
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
                            if let Tab::File(ref mut f) = tabs[active_tab] { f.row_off = row_off; f.col_off = col_off; }
                            active_tab = (active_tab + 1) % tabs.len();
                        }

                        KeyCode::BackTab | KeyCode::Tab if mods.contains(shift) => {
                            if let Tab::File(ref mut f) = tabs[active_tab] { f.row_off = row_off; f.col_off = col_off; }
                            active_tab = if active_tab == 0 { tabs.len() - 1 } else { active_tab - 1 };
                        }
                        _ => {}
                    }
                }
            }
            }
            Event::Paste(text) => {
                if !cmd_mode {
                    if let Tab::File(ref mut file) = tabs[active_tab] {
                        if !file.search_mode {
                            let paste_lines: Vec<String> = text.lines()
                                .map(|l| l.replace('\r', "").replace('\t', &" ".repeat(tab_width)))
                                .collect();
                            paste_text(file, &paste_lines, &mut last_action);
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
    print!("\x1B[H");
    write!(io::stdout(), "\x1b[?2004l")?;
    execute!(io::stdout(), DisableMouseCapture)?;
    disable_raw_mode()?;
    Ok(())
}

fn paste_text(file: &mut FileState, paste_lines: &[String], last_action: &mut Action) {
    if paste_lines.is_empty() { return; }
    file.save_undo(last_action, Action::Other);
    if file.selecting {
        file.cursor = delete_selection(&mut file.lines, &file.sel_start, &file.cursor);
        file.selecting = false;
    }
    file.pasting = true;
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
    file.pasting = false;
    file.dirty = true;
}

fn normalize_key(code: &KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    match code {
        KeyCode::Char(c) if (1..=26).contains(&(*c as u32)) => {
            let letter = char::from_u32((*c as u32) + 0x60).unwrap_or(*c);
            (KeyCode::Char(letter), mods | KeyModifiers::CONTROL)
        }
        _ => (*code, mods),
    }
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

struct SysInfo {
    cpu_pct: f64,
    rss_kb: u64,
}

struct SysInfoCache {
    cached: SysInfo,
    prev_jiffies: u64,
    prev_time: Instant,
    has_baseline: bool,
    last_update: Instant,
    interval: Duration,
}

impl SysInfoCache {
    fn new() -> Self {
        Self {
            cached: SysInfo { cpu_pct: 0.0, rss_kb: 0},
            prev_jiffies: 0,
            prev_time: Instant::now(),
            has_baseline: false,
            last_update: Instant::now(),
            interval: Duration::from_millis(800),
        }
    }
}
fn read_proc_rss_kb() -> Option<u64> {
    let content = fs::read_to_string("/proc/self/status").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.trim().split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn read_proc_jiffies() -> Option<u64> {
    let content = fs::read_to_string("/proc/self/stat").ok()?;
    let rparen = content.rfind(')')?;
    let rest = &content[rparen + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 13 { return None; }
    let utime: u64 = fields[11].parse().ok()?;
    let stime: u64 = fields[12].parse().ok()?;
    Some(utime + stime)
}

fn refresh_sysinfo(cache: &mut SysInfoCache) {
    let now = Instant::now();
    if now.duration_since(cache.last_update) < cache.interval {
        return;
    }
    let rss_kb = read_proc_rss_kb().unwrap_or(0);
    let jiffies = read_proc_jiffies().unwrap_or(0);
    let cpu_pct = if cache.has_baseline {
        let dj = jiffies.saturating_sub(cache.prev_jiffies);
        let ds = now.duration_since(cache.prev_time).as_secs_f64();
        if ds > 0.0 { (dj as f64 / ds).min(100.0) } else { cache.cached.cpu_pct }
    } else {
        cache.has_baseline = true;
        0.0
    };
    cache.prev_jiffies = jiffies;
    cache.prev_time = now;
    cache.last_update = now;
    cache.cached = SysInfo { cpu_pct, rss_kb };
}

fn format_sysinfo(info: &SysInfo, mode: u8) -> String {
    match mode {
        1 => {
            let mem = if info.rss_kb < 1024 {
                format!("{}KB", info.rss_kb)
            } else {
                format!("{:.1}MB", info.rss_kb as f64 / 1024.0)
            };
            format!("CPU: {:.1}% | RAM: {}", info.cpu_pct, mem)
        }
        _ => String::new(),
    }
}

fn help_content(bindings: &HashMap<String, KeyCombo>) -> String {
    let mut text = "\
=== Commands ===\n:q        - quit\n:w        - save file\n:wq       - save and quit\n\
:numlist  - toggle line numbers\n:sysinfo  - toggle sysinfo display\n:set <key> <value>  - set config option\n\
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
    text
}