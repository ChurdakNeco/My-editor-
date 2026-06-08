use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::fs::File;
use crate::config::{Theme, theme_color, semantic_color};
use crossterm::event::KeyModifiers;

#[derive(Copy, Clone, PartialEq)]
pub struct Pos {
    pub x: usize,
    pub y: usize,
}

pub fn byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices().map(|(b, _)| b).nth(char_idx).unwrap_or(s.len())
}

pub fn is_keyword(w: &str) -> bool {
    matches!(w, "fn" | "let" | "mut" | "struct" | "enum" | "impl" | "use" | "pub"
        | "return" | "if" | "else" | "loop" | "while" | "for" | "in" | "match"
        | "break" | "continue" | "mod" | "crate" | "as" | "true" | "false")
}

pub fn is_type(w: &str) -> bool {
    matches!(w, "i32" | "i64" | "u32" | "u64" | "usize" | "isize" | "f32" | "f64"
        | "str" | "String" | "Vec" | "Result" | "Option" | "bool" | "char")
}

pub fn is_selected(sel: Option<(usize, usize)>, start: usize, end: usize) -> bool {
    sel.map_or(false, |(s, e)| end > s && start < e)
}

pub fn in_ranges(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
    ranges.iter().any(|&(s, e)| end > s && start < e)
}

pub fn find_matches(line: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() { return vec![]; }
    let mut res = vec![];
    let lc: Vec<char> = line.chars().collect();
    let qc: Vec<char> = query.chars().collect();
    if qc.len() > lc.len() { return res; }
    let mut i = 0;
    while i + qc.len() <= lc.len() {
        if lc[i..i + qc.len()] == qc[..] {
            res.push((i, i + qc.len()));
            i += qc.len();
        } else {
            i += 1;
        }
    }
    res
}

pub fn render_line(line: &str, sel_range: Option<(usize, usize)>, search_matches: &[(usize, usize)], theme: Option<&Theme>,
    in_block_comment: &mut bool, in_block_string: &mut bool) -> String {
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    let str_color = theme.and_then(|t| semantic_color("@string", t)).unwrap_or("\x1B[32m");
    let num_color = theme.and_then(|t| semantic_color("@number", t)).unwrap_or("\x1B[33m");
    let fn_color = theme.and_then(|t| semantic_color("@function", t)).unwrap_or("\x1B[33m");
    let path_color = theme.and_then(|t| semantic_color("@path", t)).unwrap_or("\x1B[36m");
    let comment_color = theme.and_then(|t| semantic_color("@comment", t)).unwrap_or("\x1B[90m");
    let directive_color = theme.and_then(|t| semantic_color("@directive", t)).unwrap_or("\x1B[36m");

    fn emit_range(out: &mut String, chars: &[char], start: usize, end: usize) {
        for j in start..end { out.push(chars[j]); }
    }

    while i < chars.len() {
        if *in_block_comment {
            let mut end = chars.len();
            let mut found_close = false;
            for j in i..chars.len().saturating_sub(1) {
                if chars[j] == '*' && chars[j + 1] == '/' {
                    end = j + 2;
                    found_close = true;
                    break;
                }
            }
            let selected = is_selected(sel_range, i, end);
            let searched = !selected && in_ranges(search_matches, i, end);
            if selected { out.push_str("\x1B[7m"); }
            else if searched { out.push_str("\x1B[43m"); }
            else { out.push_str(comment_color); }
            emit_range(&mut out, &chars, i, end);
            out.push_str("\x1B[0m");
            i = end;
            if found_close { *in_block_comment = false; }
            continue;
        }

        if *in_block_string {
            let mut end = chars.len();
            let mut found_close = false;
            for j in i..chars.len().saturating_sub(2) {
                if chars[j] == '"' && chars[j + 1] == '"' && chars[j + 2] == '"' {
                    end = j + 3;
                    found_close = true;
                    break;
                }
            }
            let selected = is_selected(sel_range, i, end);
            let searched = !selected && in_ranges(search_matches, i, end);
            if selected { out.push_str("\x1B[7m"); out.push_str(str_color); }
            else if searched { out.push_str("\x1B[43m"); out.push_str(str_color); }
            else { out.push_str(str_color); }
            emit_range(&mut out, &chars, i, end);
            out.push_str("\x1B[0m");
            i = end;
            if found_close { *in_block_string = false; }
            continue;
        }

        if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '/' {
            let start = i;
            i = chars.len();
            let selected = is_selected(sel_range, start, i);
            let searched = !selected && in_ranges(search_matches, start, i);
            if selected {
                out.push_str("\x1B[7m");
                out.push_str(comment_color);
            } else if searched {
                out.push_str("\x1B[43m");
                out.push_str(comment_color);
            } else {
                out.push_str(comment_color);
            }
            emit_range(&mut out, &chars, start, i);
            out.push_str("\x1B[0m");
            continue;
        }

        if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '*' {
            *in_block_comment = true;
            out.push_str(comment_color);
            out.push_str("/*");
            i += 2;
            continue;
        }

        if i + 2 < chars.len() && chars[i] == '"' && chars[i + 1] == '"' && chars[i + 2] == '"' {
            *in_block_string = true;
            out.push_str(str_color);
            out.push_str("\"\"\"");
            i += 3;
            continue;
        }

        if chars[i] == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < chars.len() { i += 1; }
                i += 1;
            }
            i = i.min(chars.len());
            if i < chars.len() { i += 1; }
            let end = i;
            let selected = is_selected(sel_range, start, end);
            let searched = !selected && in_ranges(search_matches, start, end);
            if selected { out.push_str("\x1B[7m"); out.push_str(str_color); }
            else if searched { out.push_str("\x1B[43m"); out.push_str(str_color); }
            else { out.push_str(str_color); }
            emit_range(&mut out, &chars, start, end);
            out.push_str("\x1B[0m");
            continue;
        }

        if chars[i] == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() { i += 1; }
                i += 1;
            }
            i = i.min(chars.len());
            if i < chars.len() { i += 1; }
            let end = i;
            let selected = is_selected(sel_range, start, end);
            let searched = !selected && in_ranges(search_matches, start, end);
            if selected {
                out.push_str("\x1B[7m");
                out.push_str(str_color);
            } else if searched {
                out.push_str("\x1B[43m");
                out.push_str(str_color);
            } else {
                out.push_str(str_color);
            }
            emit_range(&mut out, &chars, start, end);
            out.push_str("\x1B[0m");
            continue;
        }

        if chars[i] == '#' && i + 1 < chars.len() {
            let is_inner = i + 2 < chars.len() && chars[i + 1] == '!' && chars[i + 2] == '[';
            let is_outer = chars[i + 1] == '[';
            if is_inner || is_outer {
                let start = i;
                i += if is_inner { 3 } else { 2 };
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '[' { depth += 1; }
                    else if chars[i] == ']' { depth -= 1; }
                    if depth > 0 { i += 1; }
                }
                if depth == 0 { i += 1; }
                let end = i.min(chars.len());
                let selected = is_selected(sel_range, start, end);
                let searched = !selected && in_ranges(search_matches, start, end);
                if selected { out.push_str("\x1B[7m"); out.push_str(directive_color); }
                else if searched { out.push_str("\x1B[43m"); out.push_str(directive_color); }
                else { out.push_str(directive_color); }
                emit_range(&mut out, &chars, start, end);
                out.push_str("\x1B[0m");
                continue;
            }
        }

        if chars[i].is_ascii_digit() || (chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) {
            let start = i;
            if chars[i] == '0' && i + 1 < chars.len() && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
                i += 2;
                while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '_') { i += 1; }
            } else {
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') { i += 1; }
                if i < chars.len() && chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') { i += 1; }
                }
            }
            let end = i;
            let selected = is_selected(sel_range, start, end);
            let searched = !selected && in_ranges(search_matches, start, end);
            if selected {
                out.push_str("\x1B[7m");
                out.push_str(num_color);
            } else if searched {
                out.push_str("\x1B[43m");
                out.push_str(num_color);
            } else {
                out.push_str(num_color);
            }
            emit_range(&mut out, &chars, start, end);
            out.push_str("\x1B[0m");
            continue;
        }

        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            let word: String = chars[start..i].iter().collect();
            let is_fn = i < chars.len() && chars[i] == '(';
            let is_path_ahead = i + 1 < chars.len() && chars[i] == ':' && chars[i + 1] == ':';
            let is_path_behind = start >= 2 && chars[start - 2] == ':' && chars[start - 1] == ':';
            let is_path = is_path_ahead || is_path_behind;
            let color = theme.and_then(|t| theme_color(&word, t)).unwrap_or("");
            let color = if !color.is_empty() { color }
                       else if is_fn { fn_color }
                       else if is_path { path_color }
                       else if is_keyword(&word) { "\x1B[35m" }
                       else if is_type(&word) { "\x1B[32m" }
                       else { "" };
            let selected = is_selected(sel_range, start, i);
            let searched = !selected && in_ranges(search_matches, start, i);
            if selected {
                out.push_str("\x1B[7m");
                if !color.is_empty() { out.push_str(color); }
            } else if searched {
                out.push_str("\x1B[43m");
                if !color.is_empty() { out.push_str(color); }
            } else if !color.is_empty() {
                out.push_str(color);
            }
            out.push_str(&word);
            if selected || searched || !color.is_empty() {
                out.push_str("\x1B[0m");
            }
            continue;
        }

        let c = chars[i];
        let selected = is_selected(sel_range, i, i + 1);
        let searched = !selected && in_ranges(search_matches, i, i + 1);
        if selected {
            out.push_str("\x1B[7m");
            out.push(c);
            out.push_str("\x1B[0m");
        } else if searched {
            out.push_str("\x1B[43m");
            out.push(c);
            out.push_str("\x1B[0m");
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

pub fn line_sel_range(line_idx: usize, sel: &Pos, cursor: &Pos, active: bool, line_len: usize) -> Option<(usize, usize)> {
    if !active { return None; }
    if sel.y == cursor.y && sel.x == cursor.x { return None; }
    let (top, bot) = if sel.y < cursor.y || (sel.y == cursor.y && sel.x < cursor.x) {
        (*sel, *cursor)
    } else {
        (*cursor, *sel)
    };
    if line_idx < top.y || line_idx > bot.y { return None; }
    if top.y == bot.y { Some((top.x, bot.x)) }
    else if line_idx == top.y { Some((top.x, line_len)) }
    else if line_idx == bot.y { Some((0, bot.x)) }
    else { Some((0, line_len)) }
}

pub fn delete_selection(lines: &mut Vec<String>, sel: &Pos, cursor: &Pos) -> Pos {
    let (top, bot) = if sel.y < cursor.y || (sel.y == cursor.y && sel.x < cursor.x) {
        (*sel, *cursor)
    } else {
        (*cursor, *sel)
    };
    if top.y == bot.y {
        let b1 = byte_idx(&lines[top.y], top.x);
        let b2 = byte_idx(&lines[top.y], bot.x);
        lines[top.y].drain(b1..b2);
        Pos { x: top.x, y: top.y }
    } else {
        let first: String = lines[top.y].chars().take(top.x).collect();
        let last: String = lines[bot.y].chars().skip(bot.x).collect();
        lines[top.y] = first + &last;
        for _ in 0..(bot.y - top.y) { lines.remove(top.y + 1); }
        Pos { x: top.x, y: top.y }
    }
}

pub fn collect_selection_text(lines: &[String], sel: &Pos, cursor: &Pos) -> Vec<String> {
    let (top, bot) = if sel.y < cursor.y || (sel.y == cursor.y && sel.x < cursor.x) {
        (*sel, *cursor)
    } else {
        (*cursor, *sel)
    };
    if top.y == bot.y {
        let s: String = lines[top.y].chars().skip(top.x).take(bot.x - top.x).collect();
        vec![s]
    } else {
        let mut result = Vec::new();
        let first: String = lines[top.y].chars().skip(top.x).collect();
        result.push(first);
        for y in top.y + 1..bot.y { result.push(lines[y].clone()); }
        let last: String = lines[bot.y].chars().take(bot.x).collect();
        result.push(last);
        result
    }
}

pub fn find_prev_match(lines: &[String], query: &str, from: &Pos) -> Option<Pos> {
    if query.is_empty() { return None; }
    let qc: Vec<char> = query.chars().collect();
    for y in (0..=from.y).rev() {
        let lc: Vec<char> = lines[y].chars().collect();
        if qc.len() > lc.len() { continue; }
        let mut max_x = lc.len() - qc.len();
        if y == from.y { max_x = max_x.min(from.x.saturating_sub(qc.len())); }
        for x in (0..=max_x).rev() {
            if lc[x..x + qc.len()] == qc[..] { return Some(Pos { x, y }); }
        }
    }
    for y in (from.y + 1..lines.len()).rev() {
        let lc: Vec<char> = lines[y].chars().collect();
        if qc.len() > lc.len() { continue; }
        for x in (0..=lc.len() - qc.len()).rev() {
            if lc[x..x + qc.len()] == qc[..] { return Some(Pos { x, y }); }
        }
    }
    None
}

pub fn find_next_match(lines: &[String], query: &str, from: &Pos) -> Option<Pos> {
    if query.is_empty() { return None; }
    let qc: Vec<char> = query.chars().collect();
    for y in from.y..lines.len() {
        let lc: Vec<char> = lines[y].chars().collect();
        if qc.len() > lc.len() { continue; }
        let sx = if y == from.y { from.x } else { 0 };
        for x in sx..=lc.len() - qc.len() {
            if lc[x..x + qc.len()] == qc[..] { return Some(Pos { x, y }); }
        }
    }
    for y in 0..=from.y {
        let lc: Vec<char> = lines[y].chars().collect();
        if qc.len() > lc.len() { continue; }
        let ex = if y == from.y { from.x.saturating_sub(1) } else { lc.len() };
        if qc.len() > ex { continue; }
        for x in 0..=ex.saturating_sub(qc.len()) {
            if lc[x..x + qc.len()] == qc[..] { return Some(Pos { x, y }); }
        }
        if y == from.y { break; }
    }
    None
}

pub fn scan_dir(dir: &PathBuf) -> io::Result<Vec<(String, bool)>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type()?.is_dir();
        if !name.starts_with('.') { entries.push((name, is_dir)); }
    }
    entries.sort_by(|a, b| {
        if a.1 != b.1 { b.1.cmp(&a.1) } else { a.0.cmp(&b.0) }
    });
    Ok(entries)
}

pub fn find_all_matches(lines: &[String], query: &str) -> Vec<Pos> {
    if query.is_empty() { return vec![]; }
    let qc: Vec<char> = query.chars().collect();
    let mut res = vec![];
    for (y, line) in lines.iter().enumerate() {
        let lc: Vec<char> = line.chars().collect();
        if qc.len() > lc.len() { continue; }
        for x in 0..=lc.len() - qc.len() {
            if lc[x..x + qc.len()] == qc[..] {
                res.push(Pos { x, y });
            }
        }
    }
    res
}

pub fn render_browser_line(name: &str, is_dir: bool, active: bool, width: usize) -> String {
    let mut out = String::new();
    if active { out.push_str("\x1B[7m"); }
    let icon = if is_dir { 'd' } else { 'f' };
    let truncated: String = format!(" {}  {}", icon, name).chars().take(width).collect();
    out.push_str(&truncated);
    if active { out.push_str("\x1B[0m"); }
    out
}

pub fn collect_candidates(lines: &[String], prefix: &str) -> Vec<String> {
    if prefix.len() < 2 {
        return vec![];
    }
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for line in lines {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_alphanumeric() || chars[i] == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if word != prefix && word.len() >= prefix.len()
                    && word[..prefix.len()].eq_ignore_ascii_case(prefix)
                    && seen.insert(word.clone())
                {
                    candidates.push(word);
                }
            } else {
                i += 1;
            }
        }
    }
    candidates.sort();
    candidates.truncate(20);
    candidates
}

pub struct FileState {
    pub path: Option<String>,
    pub lines: Vec<String>,
    pub cursor: Pos,
    pub dirty: bool,
    pub snapshot: String,
    pub undo: Vec<Vec<String>>,
    pub redo: Vec<Vec<String>>,
    pub selecting: bool,
    pub sel_start: Pos,
    pub search_q: String,
    pub search_mode: bool,
    pub block_comment: bool,
    pub block_string: bool,
    pub row_off: usize,
    pub col_off: usize,
    pub cached_search_q: String,
    pub cached_total_matches: usize,
    pub cached_current_match: usize,
    pub pasting: bool,
    pub max_undo: usize,
    pub completion_candidates: Vec<String>,
    pub completion_idx: usize,
    pub completion_active: bool,
    pub completion_prefix: String,
}

impl FileState {
    pub fn new(path: Option<String>, lines: Vec<String>, snapshot: String) -> Self {
        FileState {
            path, lines, snapshot,
            cursor: Pos { x: 0, y: 0 },
            dirty: false,
            undo: Vec::new(), redo: Vec::new(),
            selecting: false, sel_start: Pos { x: 0, y: 0 },
            search_q: String::new(), search_mode: false,
            block_comment: false, block_string: false,
            row_off: 0, col_off: 0,
            cached_search_q: String::new(), cached_total_matches: 0, cached_current_match: 0,
            pasting: false, max_undo: 500,
            completion_candidates: Vec::new(), completion_idx: 0, completion_active: false, completion_prefix: String::new(),
        }
    }

    pub fn save_undo(&mut self, last_action: &mut Action, expected: Action) {
        let same = *last_action == expected && *last_action != Action::None;
        if !same {
            if self.undo.len() >= self.max_undo { self.undo.remove(0); }
            self.undo.push(self.lines.clone());
        }
        *last_action = expected;
        self.redo.clear();
    }

    pub fn clamp_cursor(&mut self) {
        if self.cursor.y >= self.lines.len() {
            self.cursor.y = self.lines.len().saturating_sub(1);
        }
        let cc = self.lines[self.cursor.y].chars().count();
        if self.cursor.x > cc { self.cursor.x = cc; }
    }

    pub fn save_to_disk(&mut self) {
        if let Some(ref path) = self.path {
            if let Some(parent) = std::path::Path::new(path).parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(mut f) = File::create(path) {
                let _ = write!(f, "{}", self.lines.join("\n"));
                self.dirty = false;
                self.snapshot = self.lines.join("\n");
            }
        }
    }

    pub fn update_completion(&mut self) {
        let prefix: String = self.lines[self.cursor.y].chars().take(self.cursor.x)
            .collect::<String>().chars().rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>().into_iter().rev().collect();
        if prefix.len() >= 2 {
            self.completion_prefix = prefix;
            self.completion_candidates = collect_candidates(&self.lines, &self.completion_prefix);
            self.completion_idx = 0;
            self.completion_active = !self.completion_candidates.is_empty();
        } else {
            self.completion_active = false;
            self.completion_candidates.clear();
        }
    }
}

pub struct BrowserState {
    pub dir: PathBuf,
    pub entries: Vec<(String, bool)>,
    pub cursor: usize,
    pub scroll: usize,
    pub history: Vec<PathBuf>,
}

pub enum Tab {
    File(FileState),
    Browser(BrowserState),
}

#[derive(PartialEq)]
pub enum Action { None, Insert, Delete, Other }

pub fn nav_cursor(mods: KeyModifiers, last_action: &mut Action, file: &mut FileState) {
    *last_action = Action::None;
    if mods.contains(KeyModifiers::SHIFT) {
        if !file.selecting { file.sel_start = file.cursor; file.selecting = true; }
    } else {
        file.selecting = false;
    }
}

pub fn prescan_block_state(lines: &[String], up_to: usize, start_comment: bool, start_string: bool) -> (bool, bool) {
    let mut bc = start_comment;
    let mut bs = start_string;
    let end = up_to.min(lines.len());
    for li in 0..end {
        let s = &lines[li];
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if bc {
                if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '/' {
                    bc = false;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            if bs {
                if i + 2 < chars.len() && chars[i] == '"' && chars[i + 1] == '"' && chars[i + 2] == '"' {
                    bs = false;
                    i += 3;
                    continue;
                }
                i += 1;
                continue;
            }
            if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '/' {
                break;
            }
            if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '*' {
                bc = true;
                i += 2;
                continue;
            }
            if i + 2 < chars.len() && chars[i] == '"' && chars[i + 1] == '"' && chars[i + 2] == '"' {
                bs = true;
                i += 3;
                continue;
            }
            if chars[i] == '"' {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() { i += 1; }
                    i += 1;
                }
                if i < chars.len() { i += 1; }
                continue;
            }
            if chars[i] == '\'' {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    if chars[i] == '\\' && i + 1 < chars.len() { i += 1; }
                    i += 1;
                }
                if i < chars.len() { i += 1; }
                continue;
            }
            i += 1;
        }
    }
    (bc, bs)
}
