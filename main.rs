use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, size},
};

#[derive(Copy, Clone)]
struct Pos {
    x: usize,
    y: usize,
}

fn byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .map(|(b, _) | b)
        .nth(char_idx)
        .unwrap_or(s.len())
}

fn is_keyword(w: &str) -> bool {
    matches!(w, "fn" | "let" | "mut" | "struct" | "enum" | "impl" | "use" | "pub"
        | "return" | "if" | "else" | "loop" | "while" | "for" | "in" | "match"
        | "break" | "continue" | "mod" | "crate" | "as" | "true" | "false")
}

fn is_type(w: &str) -> bool {
    matches!(w, "i32" | "i64" | "u32" | "u64" | "usize" | "isize" | "f32" | "f64"
        | "str" | "String" | "Vec" | "Result" | "Option" | "bool" | "char")
}

fn is_selected(sel: Option<(usize, usize)>, start: usize, end: usize) -> bool {
    sel.map_or(false, |(s, e)| end > s && start < e)
}

fn in_ranges(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
    ranges.iter().any(|&(s, e)| end > s && start < e)
}

fn find_matches(line: &str, query: &str) -> Vec<(usize, usize)> {
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

fn render_line(line: &str, sel_range: Option<(usize, usize)>, search_matches: &[(usize, usize)]) -> String {
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            let is_fn = i < chars.len() && chars[i] == '(';
            let is_path_ahead = i + 1 < chars.len() && chars[i] == ':' && chars[i + 1] == ':';
            let is_path_behind = start >= 2 && chars[start - 2] == ':' && chars[start - 1] == ':';
            let is_path = is_path_ahead || is_path_behind;

            let color = if is_fn { "\x1B[33m" }
                       else if is_path { "\x1B[36m" }
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

fn line_sel_range(line_idx: usize, sel: &Pos, cursor: &Pos, active: bool, line_len: usize) -> Option<(usize, usize)> {
    if !active { return None; }
    if sel.y == cursor.y && sel.x == cursor.x { return None; }

    let (top, bot) = if sel.y < cursor.y || (sel.y == cursor.y && sel.x < cursor.x) {
        (*sel, *cursor)
    } else {
        (*cursor, *sel)
    };

    if line_idx < top.y || line_idx > bot.y { return None; }

    if top.y == bot.y {
        Some((top.x, bot.x))
    } else if line_idx == top.y {
        Some((top.x, line_len))
    } else if line_idx == bot.y {
        Some((0, bot.x))
    } else {
        Some((0, line_len))
    }
}

fn delete_selection(lines: &mut Vec<String>, sel: &Pos, cursor: &Pos) -> Pos {
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
        for _ in 0..(bot.y - top.y) {
            lines.remove(top.y + 1);
        }
        Pos { x: top.x, y: top.y }
    }
}

fn collect_selection_text(lines: &[String], sel: &Pos, cursor: &Pos) -> Vec<String> {
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
        for y in top.y + 1..bot.y {
            result.push(lines[y].clone());
        }
        let last: String = lines[bot.y].chars().take(bot.x).collect();
        result.push(last);
        result
    }
}

fn find_next_match(lines: &[String], query: &str, from: &Pos) -> Option<Pos> {
    if query.is_empty() { return None; }
    let qc: Vec<char> = query.chars().collect();

    for y in from.y..lines.len() {
        let lc: Vec<char> = lines[y].chars().collect();
        if qc.len() > lc.len() { continue; }
        let sx = if y == from.y { from.x } else { 0 };
        for x in sx..=lc.len() - qc.len() {
            if lc[x..x + qc.len()] == qc[..] {
                return Some(Pos { x, y });
            }
        }
    }

    for y in 0..=from.y {
        let lc: Vec<char> = lines[y].chars().collect();
        if qc.len() > lc.len() { continue; }
        let ex = if y == from.y { from.x.saturating_sub(1) } else { lc.len() };
        if qc.len() > ex { continue; }
        for x in 0..=ex.saturating_sub(qc.len()) {
            if lc[x..x + qc.len()] == qc[..] {
                return Some(Pos { x, y });
            }
        }
        if y == from.y { break; }
    }
    None
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let filename = if args.len() > 1 { Some(args[1].clone()) } else { None };
    let mut lines: Vec<String> = Vec::new();

    if let Some(ref path) = filename {
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            for line in reader.lines() {
                lines.push(line?);
            }
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }

    enable_raw_mode()?;
    let mut pos = Pos { x: 0, y: 0 };
    let mut row_off = 0;
    let mut col_off = 0;
    let mut dirty = false;
    let mut clipboard: Vec<String> = Vec::new();
    let mut undo: Vec<Vec<String>> = Vec::new();
    let mut redo: Vec<Vec<String>> = Vec::new();

    let mut selecting = false;
    let mut sel_start = Pos { x: 0, y: 0 };

    let mut search_q = String::new();
    let mut search_mode = false;

    #[derive(PartialEq)]
    enum Action { None, Insert, Delete, Other }
    let mut last_action = Action::None;

    macro_rules! push_undo {
        ($act:expr) => {
            if last_action != $act || last_action == Action::None {
                undo.push(lines.clone());
            }
            last_action = $act;
        };
    }

    loop {
        let (tw, th) = size()?;
        let vw = tw as usize;
        let vh = (th as usize).saturating_sub(1);

        if pos.y < row_off { row_off = pos.y; }
        else if pos.y >= row_off + vh { row_off = pos.y - vh + 1; }
        if pos.x < col_off { col_off = pos.x; }
        else if pos.x >= col_off + vw { col_off = pos.x - vw + 1; }

        let sel_for_line = |li: usize, ll: usize| -> Option<(usize, usize)> {
            if selecting && sel_start.y == pos.y && sel_start.x == pos.x { return None; }
            line_sel_range(li, &sel_start, &pos, selecting, ll)
        };

        let mut search_line_matches: Vec<Vec<(usize, usize)>> = Vec::new();
        if !search_q.is_empty() {
            for r in 0..vh {
                let li = r + row_off;
                if li < lines.len() {
                    search_line_matches.push(find_matches(&lines[li], &search_q));
                } else {
                    search_line_matches.push(vec![]);
                }
            }
        }

        print!("\x1B[H");
        for r in 0..vh {
            let li = r + row_off;
            if li < lines.len() {
                let full_line = &lines[li];
                let full_len = full_line.chars().count();
                let vis_start = col_off.min(full_len);
                let vis_end = (col_off + vw).min(full_len);

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

                let rendered = render_line(&plain_line, adj_sel, &adj_sm);
                print!("{}\x1B[K\r\n", rendered);
            } else {
                print!("\x1B[K\r\n");
            }
        }

        let finfo = filename.as_deref().unwrap_or("[No Name]");
        let dm = if dirty { " [+]" } else { "" };

        let status = if search_mode {
            format!("/ {}", search_q)
        } else {
            let sm = if selecting { " [Sel]" } else { "" };
            format!("{}{}{} | Ln {} Col {}", finfo, dm, sm, pos.y + 1, pos.x + 1)
        };

        let sp = vw.saturating_sub(status.chars().count());
        print!("\x1B[{};1H\x1B[7m{}{}\x1B[0m\x1B[K", th, " ".repeat(sp), status);

        let vr = pos.y - row_off;
        let vc = pos.x - col_off;
        print!("\x1B[{};{}H", vr + 1, vc + 1);

        io::stdout().flush()?;

        if let Event::Key(k) = event::read()? {
            let ctrl = KeyModifiers::CONTROL;
            let shift = KeyModifiers::SHIFT;
            let mods = k.modifiers;

            if search_mode {
                match k.code {
                    KeyCode::Char(c) => {
                        search_q.push(c);
                        if let Some(m) = find_next_match(&lines, &search_q, &pos) {
                            pos = m;
                        }
                    }
                    KeyCode::Backspace => {
                        search_q.pop();
                        if !search_q.is_empty() {
                            if let Some(m) = find_next_match(&lines, &search_q, &pos) {
                                pos = m;
                            }
                        }
                    }
                    KeyCode::Enter | KeyCode::Esc if mods == KeyModifiers::NONE => {
                        if k.code == KeyCode::Esc { search_q.clear(); }
                        search_mode = false;
                    }
                    _ => {}
                }
                continue;
            }

            match k.code {
                KeyCode::Char('q') if mods.contains(ctrl) => break,

                KeyCode::Char('s') if mods.contains(ctrl) => {
                    if let Some(ref path) = filename {
                        if let Ok(mut f) = File::create(path) {
                            if write!(f, "{}", lines.join("\n")).is_ok() {
                                dirty = false;
                            }
                        }
                    }
                }

                KeyCode::Char('z') if mods.contains(ctrl) => {
                    if !undo.is_empty() {
                        selecting = false;
                        redo.push(lines.clone());
                        lines = undo.pop().unwrap();
                        if pos.y >= lines.len() { pos.y = lines.len().saturating_sub(1); }
                        let cc = lines[pos.y].chars().count();
                        if pos.x > cc { pos.x = cc; }
                        dirty = true;
                        last_action = Action::None;
                    }
                }

                KeyCode::Char('y') if mods.contains(ctrl) => {
                    if !redo.is_empty() {
                        selecting = false;
                        undo.push(lines.clone());
                        lines = redo.pop().unwrap();
                        if pos.y >= lines.len() { pos.y = lines.len().saturating_sub(1); }
                        let cc = lines[pos.y].chars().count();
                        if pos.x > cc { pos.x = cc; }
                        dirty = true;
                        last_action = Action::None;
                    }
                }

                KeyCode::Char('f') if mods.contains(ctrl) => {
                    search_mode = true;
                    search_q.clear();
                }

                KeyCode::Char('c') if mods.contains(ctrl) => {
                    if selecting {
                        clipboard = collect_selection_text(&lines, &sel_start, &pos);
                    } else {
                        clipboard = vec![lines[pos.y].clone()];
                    }
                }

                KeyCode::Char('x') if mods.contains(ctrl) => {
                    push_undo!(Action::Other);
                    redo.clear();
                    if selecting {
                        clipboard = collect_selection_text(&lines, &sel_start, &pos);
                        pos = delete_selection(&mut lines, &sel_start, &pos);
                        selecting = false;
                    } else {
                        clipboard = vec![lines[pos.y].clone()];
                        if lines.len() > 1 {
                            lines.remove(pos.y);
                            if pos.y >= lines.len() { pos.y = lines.len() - 1; }
                        } else {
                            lines[0].clear();
                        }
                        pos.x = 0;
                    }
                    dirty = true;
                }

                KeyCode::Char('v') if mods.contains(ctrl) && !clipboard.is_empty() => {
                    push_undo!(Action::Other);
                    redo.clear();
                    if selecting {
                        pos = delete_selection(&mut lines, &sel_start, &pos);
                        selecting = false;
                    }
                    if clipboard.len() == 1 {
                        let b = byte_idx(&lines[pos.y], pos.x);
                        lines[pos.y].insert_str(b, &clipboard[0]);
                        pos.x += clipboard[0].chars().count();
                    } else {
                        let b = byte_idx(&lines[pos.y], pos.x);
                        let rest = lines[pos.y].split_off(b);
                        lines[pos.y].push_str(&clipboard[0]);
                        for i in 1..clipboard.len() {
                            lines.insert(pos.y + i, clipboard[i].clone());
                        }
                        let last_line_idx = pos.y + clipboard.len() - 1;
                        lines[last_line_idx].push_str(&rest);
                        pos.y = last_line_idx;
                        pos.x = clipboard.last().unwrap().chars().count();
                    }
                    dirty = true;
                }

                KeyCode::Char('a') if mods.contains(ctrl) => {
                    selecting = true;
                    sel_start = Pos { x: 0, y: 0 };
                    pos.y = lines.len() - 1;
                    pos.x = lines[pos.y].chars().count();
                }

                KeyCode::Char('d') if mods.contains(ctrl) => {
                    selecting = false;
                    push_undo!(Action::Other);
                    redo.clear();
                    if lines.len() > 1 {
                        lines.remove(pos.y);
                        if pos.y >= lines.len() { pos.y = lines.len() - 1; }
                    } else {
                        lines[0].clear();
                    }
                    pos.x = 0;
                    dirty = true;
                }

                KeyCode::Char('k') if mods.contains(ctrl) => {
                    selecting = false;
                    push_undo!(Action::Other);
                    redo.clear();
                    lines.clear();
                    lines.push(String::new());
                    pos = Pos { x: 0, y: 0 };
                    dirty = true;
                }

                KeyCode::Up => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    if pos.y > 0 { pos.y -= 1; }
                    let cc = lines[pos.y].chars().count();
                    if pos.x > cc { pos.x = cc; }
                }

                KeyCode::Down => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    if pos.y < lines.len() - 1 { pos.y += 1; }
                    let cc = lines[pos.y].chars().count();
                    if pos.x > cc { pos.x = cc; }
                }

                KeyCode::Left => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    if pos.x > 0 { pos.x -= 1; }
                    else if pos.y > 0 { pos.y -= 1; pos.x = lines[pos.y].chars().count(); }
                }

                KeyCode::Right => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    let cc = lines[pos.y].chars().count();
                    if pos.x < cc { pos.x += 1; }
                    else if pos.y < lines.len() - 1 { pos.y += 1; pos.x = 0; }
                }

                KeyCode::Home => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    pos.x = 0;
                }

                KeyCode::End => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    pos.x = lines[pos.y].chars().count();
                }

                KeyCode::PageUp => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    let page = vh.saturating_sub(1);
                    if pos.y > page { pos.y -= page; } else { pos.y = 0; }
                    let cc = lines[pos.y].chars().count();
                    if pos.x > cc { pos.x = cc; }
                }

                KeyCode::PageDown => {
                    last_action = Action::None;
                    if mods.contains(shift) {
                        if !selecting { sel_start = pos; selecting = true; }
                    } else {
                        selecting = false;
                    }
                    let page = vh.saturating_sub(1);
                    let last = lines.len() - 1;
                    if pos.y + page < last { pos.y += page; } else { pos.y = last; }
                    let cc = lines[pos.y].chars().count();
                    if pos.x > cc { pos.x = cc; }
                }

                _ => {
                    if mods.intersects(ctrl) { continue; }

                    if selecting {
                        push_undo!(Action::Other);
                        redo.clear();
                        pos = delete_selection(&mut lines, &sel_start, &pos);
                        selecting = false;
                    }

                    match k.code {
                        KeyCode::Char(ch) => {
                            push_undo!(Action::Insert);
                            redo.clear();
                            let b = byte_idx(&lines[pos.y], pos.x);
                            lines[pos.y].insert(b, ch);
                            pos.x += 1;
                            dirty = true;
                        }
                        KeyCode::Tab => {
                            push_undo!(Action::Other);
                            redo.clear();
                            let b = byte_idx(&lines[pos.y], pos.x);
                            lines[pos.y].insert_str(b, "    ");
                            pos.x = pos.x.saturating_add(4);
                            dirty = true;
                        }
                        KeyCode::Enter => {
                            push_undo!(Action::Other);
                            redo.clear();
                            let b = byte_idx(&lines[pos.y], pos.x);
                            let rest = lines[pos.y].split_off(b);
                            lines.insert(pos.y + 1, rest);
                            pos.y += 1;
                            pos.x = 0;
                            dirty = true;
                        }
                        KeyCode::Backspace => {
                            if pos.x > 0 || pos.y > 0 {
                                push_undo!(Action::Delete);
                                redo.clear();
                            }
                            if pos.x > 0 {
                                let l = &lines[pos.y].clone();
                                let chars: Vec<char> = l.chars().collect();
                                let mut n = 1;
                                if pos.x >= 4 && chars[pos.x - 4..pos.x].iter().all(|&c| c == ' ') {
                                    n = 4;
                                }
                                for _ in 0..n {
                                    let b = byte_idx(&lines[pos.y], pos.x - 1);
                                    lines[pos.y].remove(b);
                                    pos.x -= 1;
                                }
                                dirty = true;
                            } else if pos.y > 0 {
                                let cur = lines.remove(pos.y);
                                pos.y -= 1;
                                pos.x = lines[pos.y].chars().count();
                                lines[pos.y].push_str(&cur);
                                dirty = true;
                            }
                        }
                        KeyCode::Delete => {
                            let cc = lines[pos.y].chars().count();
                            if pos.x < cc || pos.y < lines.len() - 1 {
                                push_undo!(Action::Delete);
                                redo.clear();
                            }
                            if pos.x < cc {
                                let b = byte_idx(&lines[pos.y], pos.x);
                                lines[pos.y].remove(b);
                                dirty = true;
                            } else if pos.y < lines.len() - 1 {
                                let next = lines.remove(pos.y + 1);
                                lines[pos.y].push_str(&next);
                                dirty = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    Ok(())
}
