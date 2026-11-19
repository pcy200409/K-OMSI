//! The object editor's splines, as text: a tile's `[spline]` / `[spline_h]` records found by
//! their id and changed in place (their type, length, curve, gradients, height change, cant,
//! start), new ones written after the others, one taken away with what hangs on it, and the
//! `[spline_terrain_align]` flag after one switched. Only tiles of version 11 or later (every
//! record names the ids of the splines before and after it) are edited.
//!
//! A record, from its keyword: the detail level, the file, the id, the ids of the previous
//! and the next spline, x, height, y (tile metres), the heading, the length, the radius
//! (0: straight; right positive), the gradients at the start and the end (percent), for
//! `[spline_h]` the height it climbs, the cant at the start and the end (percent), from
//! version 14 the skew at the start and the end, and the texture offset (the length of the
//! chain before it).

/// A value of a spline record that the editor changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    X,
    Height,
    Y,
    Heading,
    Length,
    Radius,
    GradStart,
    GradEnd,
    /// `[spline_h]`'s height change; set on a plain `[spline]` it becomes one.
    DeltaH,
    CantStart,
    CantEnd,
}

pub const FIELDS: [Field; 8] = [Field::Length, Field::Radius, Field::GradStart, Field::GradEnd, Field::DeltaH, Field::CantStart, Field::CantEnd, Field::Height];

impl Field {
    /// The name shown in the editor's list (English: the key of the translations).
    pub fn label(self) -> &'static str {
        match self {
            Field::X => "x",
            Field::Y => "y",
            Field::Heading => "Heading (°)",
            Field::Height => "Start height (m)",
            Field::Length => "Length (m)",
            Field::Radius => "Radius (m, 0 straight, - left)",
            Field::GradStart => "Gradient at the start (%)",
            Field::GradEnd => "Gradient at the end (%)",
            Field::DeltaH => "Height change (m, spline_h)",
            Field::CantStart => "Cant at the start (%)",
            Field::CantEnd => "Cant at the end (%)",
        }
    }
}

/// The tile's text split in lines, endings kept.
fn split(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn body(l: &str) -> &str {
    l.trim_end_matches(['\r', '\n'])
}

fn eol(text: &str) -> &'static str {
    if text.contains("\r\n") || text.is_empty() { "\r\n" } else { "\n" }
}

fn is_spline_kw(l: &str) -> bool {
    let b = body(l).trim();
    b.eq_ignore_ascii_case("[spline]") || b.eq_ignore_ascii_case("[spline_h]")
}

fn is_keyword(l: &str) -> bool {
    let b = body(l).trim();
    b.starts_with('[') && b.ends_with(']')
}

/// The record of spline `id`: its keyword line and one past its last value line (a
/// `mirror` line included).
fn find(lines: &[&str], id: i64) -> Option<(usize, usize)> {
    for i in 0..lines.len() {
        if !is_spline_kw(lines[i]) {
            continue;
        }
        if lines.get(i + 3).and_then(|l| body(l).trim().parse::<i64>().ok()) != Some(id) {
            continue;
        }
        let mut end = i + 1;
        while end < lines.len() && !body(lines[end]).trim().is_empty() && !is_keyword(lines[end]) && !body(lines[end]).trim_start().starts_with("Object Nr.") {
            end += 1;
        }
        return Some((i, end));
    }
    None
}

/// The line of `field` in a record starting at `start`.
fn line_of(lines: &[&str], start: usize, field: Field) -> Option<usize> {
    let h = body(lines[start]).trim().eq_ignore_ascii_case("[spline_h]");
    let k = match field {
        Field::X => 6,
        Field::Height => 7,
        Field::Y => 8,
        Field::Heading => 9,
        Field::Length => 10,
        Field::Radius => 11,
        Field::GradStart => 12,
        Field::GradEnd => 13,
        Field::DeltaH if h => 14,
        Field::DeltaH => return None,
        Field::CantStart => 14 + h as usize,
        Field::CantEnd => 15 + h as usize,
    };
    Some(start + k)
}

/// A number as a tile file writes it.
pub(crate) fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// The ids of the splines in the tile, in file order (the order spline attachments count).
pub fn spline_ids(text: &str) -> Vec<i64> {
    let lines = split(text);
    (0..lines.len()).filter(|i| is_spline_kw(lines[*i])).filter_map(|i| lines.get(i + 3).and_then(|l| body(l).trim().parse().ok())).collect()
}

/// A value of spline `id`'s record (`DeltaH` of a plain spline: 0).
pub fn field(text: &str, id: i64, f: Field) -> Option<f64> {
    let lines = split(text);
    let (start, end) = find(&lines, id)?;
    match line_of(&lines, start, f) {
        Some(k) if k < end => body(lines[k]).trim().parse().ok(),
        None if f == Field::DeltaH => Some(0.0),
        _ => None,
    }
}

/// The file of spline `id`, its previous and next ids.
pub fn links(text: &str, id: i64) -> Option<(String, i64, i64)> {
    let lines = split(text);
    let (start, _) = find(&lines, id)?;
    let n = |k: usize| lines.get(start + k).and_then(|l| body(l).trim().parse::<i64>().ok());
    Some((body(lines.get(start + 2)?).trim().to_string(), n(4)?, n(5)?))
}

/// The tile with one line of spline `id`'s record replaced.
fn replace_line(text: &str, at: usize, value: &str) -> String {
    let lines = split(text);
    let mut out = String::with_capacity(text.len() + 16);
    for (i, l) in lines.iter().enumerate() {
        if i == at {
            out.push_str(value);
            out.push_str(&l[body(l).len()..]);
        } else {
            out.push_str(l);
        }
    }
    out
}

/// The tile with a value of spline `id` set. Setting the height change of a plain spline
/// makes it a `[spline_h]` (a line inserted after the end gradient); 0 on a `[spline_h]`
/// leaves it one.
pub fn set_field(text: &str, id: i64, f: Field, v: f64) -> Option<String> {
    let lines = split(text);
    let (start, end) = find(&lines, id)?;
    if let Some(k) = line_of(&lines, start, f).filter(|k| *k < end) {
        return Some(replace_line(text, k, &num(v)));
    }
    if f != Field::DeltaH {
        return None;
    }
    // [spline] → [spline_h]: the keyword, and the height change after the end gradient
    let e = eol(text);
    let mut out = String::with_capacity(text.len() + 32);
    for (i, l) in lines.iter().enumerate() {
        if i == start {
            out.push_str(&body(l).replace("[spline]", "[spline_h]").replace("[SPLINE]", "[spline_h]"));
            out.push_str(&l[body(l).len()..]);
            continue;
        }
        out.push_str(l);
        if i == start + 13 {
            if !l.ends_with('\n') {
                out.push_str(e);
            }
            out.push_str(&num(v));
            out.push_str(e);
        }
    }
    Some(out)
}

/// The tile with spline `id`'s file (its type) changed.
pub fn set_file(text: &str, id: i64, file: &str) -> Option<String> {
    let lines = split(text);
    let (start, _) = find(&lines, id)?;
    Some(replace_line(text, start + 2, file))
}

/// The tile with spline `id`'s previous (`Some(prev)`) and next spline set.
pub fn set_links(text: &str, id: i64, prev: Option<i64>, next: Option<i64>) -> Option<String> {
    let lines = split(text);
    let (start, _) = find(&lines, id)?;
    let mut t = text.to_string();
    if let Some(p) = prev {
        t = replace_line(&t, start + 4, &p.to_string());
    }
    if let Some(n) = next {
        t = replace_line(&t, start + 5, &n.to_string());
    }
    Some(t)
}

/// A new spline record of a version-11-or-later tile: `file` from `pos` (tile metres, its
/// height absolute) heading `heading`, `length` long with `radius`, the gradient `grad`
/// throughout, after spline `prev` (0: none), its textures going on from `tex_offset`.
#[allow(clippy::too_many_arguments)]
pub fn new_record(version: i32, file: &str, id: i64, prev: i64, pos: [f64; 3], heading: f64, length: f64, radius: f64, grad: f64, tex_offset: f64, eol: &str) -> String {
    let mut lines = vec![
        "[spline]".to_string(),
        "0".into(),
        file.to_string(),
        id.to_string(),
        prev.to_string(),
        "0".into(),
        num(pos[0]),
        num(pos[2]),
        num(pos[1]),
        num(heading.rem_euclid(360.0)),
        num(length),
        num(radius),
        num(grad),
        num(grad),
        "0".into(),
        "0".into(),
    ];
    if version >= 14 {
        lines.push("0".into());
        lines.push("0".into());
    }
    lines.push(num(tex_offset));
    lines.push(String::new());
    let mut s = lines.join(eol);
    s.push_str(eol);
    s
}

/// The tile with `record` written after its last spline (else at its end): a new spline
/// comes after the others, so the spline attachments' count of them stays right.
pub fn add_record(text: &str, record: &str) -> String {
    let lines = split(text);
    let last = (0..lines.len()).rev().find(|i| is_spline_kw(lines[*i]));
    let e = eol(text);
    let at = match last {
        // after it and what belongs to it (its rules, its terrain flag), before the next
        // record of another kind
        Some(i) => {
            let mut k = i + 1;
            while k < lines.len() {
                let b = body(lines[k]).trim().to_ascii_lowercase();
                let own = ["[rule]", "[kill_rule]", "[spline_terrain_align]", "[spline_terrain_align_2]"].contains(&b.as_str());
                if is_keyword(lines[k]) && !own {
                    break;
                }
                k += 1;
            }
            k
        }
        None => lines.len(),
    };
    let mut out = String::with_capacity(text.len() + record.len() + 4);
    for l in &lines[..at] {
        out.push_str(l);
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(e);
    }
    out.push_str(record);
    for l in &lines[at..] {
        out.push_str(l);
    }
    out
}

/// One past the last line that belongs to the record ending before `end`: its `[rule]`s,
/// `[kill_rule]`s and terrain flags (with their values), and the blank lines between.
fn trailing_end(lines: &[&str], end: usize) -> usize {
    let mut k = end;
    loop {
        while k < lines.len() && body(lines[k]).trim().is_empty() {
            k += 1;
        }
        let Some(l) = lines.get(k) else { return k };
        let b = body(l).trim().to_ascii_lowercase();
        let values = match b.as_str() {
            "[rule]" | "[kill_rule]" => 4,
            "[spline_terrain_align]" => 0,
            "[spline_terrain_align_2]" => 1,
            _ => return k,
        };
        k += 1;
        // (a rule's fourth line is missing in very old files: stop at a keyword)
        for _ in 0..values {
            if k < lines.len() && !is_keyword(lines[k]) {
                k += 1;
            }
        }
    }
}

/// Whether spline `id` carries `[spline_terrain_align]` (or its `_2` form).
pub fn terrain_aligned(text: &str, id: i64) -> Option<bool> {
    let lines = split(text);
    let (_, end) = find(&lines, id)?;
    let stop = trailing_end(&lines, end);
    Some(lines[end..stop].iter().any(|l| body(l).trim().to_ascii_lowercase().starts_with("[spline_terrain_align")))
}

/// The tile with spline `id`'s ground alignment switched: `[spline_terrain_align]` added
/// after the record (and its rules), or every form of it taken away.
pub fn toggle_terrain_align(text: &str, id: i64) -> Option<String> {
    let lines = split(text);
    let (_, end) = find(&lines, id)?;
    let stop = trailing_end(&lines, end);
    let e = eol(text);
    let on = terrain_aligned(text, id)?;
    let mut out = String::with_capacity(text.len() + 32);
    let mut i = 0;
    while i < lines.len() {
        if (end..stop).contains(&i) && on && body(lines[i]).trim().to_ascii_lowercase().starts_with("[spline_terrain_align") {
            // (and the reach of the `_2` form)
            let two = body(lines[i]).trim().eq_ignore_ascii_case("[spline_terrain_align_2]");
            i += 1 + two as usize;
            continue;
        }
        if i == stop && !on {
            if !out.ends_with('\n') {
                out.push_str(e);
            }
            out.push_str("[spline_terrain_align]");
            out.push_str(e);
            out.push_str(e);
        }
        out.push_str(lines[i]);
        i += 1;
    }
    if stop >= lines.len() && !on {
        if !out.ends_with('\n') {
            out.push_str(e);
        }
        out.push_str("[spline_terrain_align]");
        out.push_str(e);
    }
    Some(out)
}

/// The top-level records of a tile (what ends a spline attachment's record).
const RECORDS: [&str; 10] = ["[object]", "[object_h]", "[spline]", "[spline_h]", "[splineattachement]", "[splineattachement_repeater]", "[attachobj]", "[varparent]", "[selobject]", "[selspline]"];

fn is_record(l: &str) -> bool {
    let b = body(l).trim().to_ascii_lowercase();
    RECORDS.contains(&b.as_str())
}

/// The tile without spline `id`: its record, its rules and terrain flag, and the rows of
/// objects along it (`[splineAttachement]` records naming it) go; the rows on later splines
/// count one spline fewer before theirs; a spline of the tile before or after it no longer
/// names it. Returns the text and the ids it was linked to (to unlink in other tiles).
pub fn delete(text: &str, id: i64) -> Option<(String, i64, i64)> {
    let ordinal = spline_ids(text).iter().position(|s| *s == id)?;
    let (_, prev, next) = links(text, id)?;
    let lines = split(text);
    let (start, end) = find(&lines, id)?;
    let stop = trailing_end(&lines, end);
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < lines.len() {
        if i == start {
            i = stop;
            continue;
        }
        let b = body(lines[i]).trim().to_ascii_lowercase();
        let ord_line = match b.as_str() {
            "[splineattachement]" => Some(i + 4),
            "[splineattachement_repeater]" => Some(i + 6),
            _ => None,
        };
        if let Some(k) = ord_line {
            let ord = lines.get(k).and_then(|l| body(l).trim().parse::<usize>().ok());
            if ord == Some(ordinal) {
                // the row goes, up to the next record
                i += 1;
                while i < lines.len() && !is_record(lines[i]) {
                    i += 1;
                }
                continue;
            }
            if let Some(o) = ord.filter(|o| *o > ordinal) {
                for l in &lines[i..k] {
                    out.push_str(l);
                }
                out.push_str(&(o - 1).to_string());
                out.push_str(&lines[k][body(lines[k]).len()..]);
                i = k + 1;
                continue;
            }
        }
        out.push_str(lines[i]);
        i += 1;
    }
    Some((unlink(&out, id), prev, next))
}

/// The tile with every spline that names `id` as its previous or next naming none.
pub fn unlink(text: &str, id: i64) -> String {
    let mut t = text.to_string();
    for s in spline_ids(text) {
        let Some((_, p, n)) = links(&t, s) else { continue };
        if p == id {
            t = set_links(&t, s, Some(0), None).unwrap_or(t);
        }
        if n == id {
            t = set_links(&t, s, None, Some(0)).unwrap_or(t);
        }
    }
    t
}

/// One arc from `start` heading `heading` (degrees clockwise from north) through `target`
/// (both horizontal): its length and radius (right positive, 0 straight) and the heading it
/// arrives with.
pub fn arc_to(start: glam::DVec2, heading: f64, target: glam::DVec2) -> Option<(f64, f64, f64)> {
    let d = target - start;
    let chord = d.length();
    if chord < 0.1 {
        return None;
    }
    // the chord's heading, and how far it turns off the start's
    let chord_h = d.x.atan2(d.y).to_degrees();
    let theta = (chord_h - heading + 540.0).rem_euclid(360.0) - 180.0;
    if theta.abs() >= 89.0 {
        return None;
    }
    if theta.abs() < 0.01 {
        return Some((chord, 0.0, heading));
    }
    let t = theta.to_radians();
    let r = chord / (2.0 * t.sin().abs());
    let length = r * 2.0 * t.abs();
    Some((length, r * t.signum(), heading + 2.0 * theta))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Version 14: two splines linked, a row of lamps along the second, and an object.
    const TILE: &str = "[version]\r\n14\r\n\r\n[spline]\r\n0\r\nSplines\\road.sli\r\n10\r\n0\r\n11\r\n5\r\n0.5\r\n6\r\n0\r\n50\r\n0\r\n0\r\n0\r\n0\r\n0\r\n0\r\n0\r\n0\r\n\r\n[spline]\r\n0\r\nSplines\\road.sli\r\n11\r\n10\r\n0\r\n5\r\n0.5\r\n56\r\n0\r\n30\r\n100\r\n0\r\n2\r\n0\r\n0\r\n0\r\n0\r\n50\r\n\r\n[rule]\r\n0\r\nspeedlimit\r\n50\r\n0\r\n\r\nObject Nr. 1\r\n[splineAttachement]\r\n0\r\nSceneryobjects\\lamp.sco\r\n20\r\n1\r\n3\r\n0\r\n0\r\n0\r\n0\r\n0\r\n10\r\n30\r\n0\r\n0\r\n\r\n[object]\r\n0\r\nSceneryobjects\\a.sco\r\n30\r\n1\r\n2\r\n0\r\n0\r\n0\r\n0\r\n0\r\n\r\n";

    fn parsed(text: &str) -> omsi_map::Tile {
        omsi_map::Tile::parse(&omsi_cfg::CfgFile::from_str("tile_0_0.map", text))
    }

    #[test]
    fn values_are_read_and_set_in_their_record() {
        assert_eq!(spline_ids(TILE), vec![10, 11]);
        assert_eq!(field(TILE, 11, Field::Radius), Some(100.0));
        assert_eq!(field(TILE, 11, Field::GradEnd), Some(2.0));
        let t = set_field(TILE, 11, Field::Length, 42.5).unwrap();
        let t = set_field(&t, 11, Field::CantEnd, -3.0).unwrap();
        let t = set_field(&t, 10, Field::Height, 1.25).unwrap();
        let p = parsed(&t);
        assert_eq!((p.splines[1].length, p.splines[1].cant_end), (42.5, -3.0));
        assert_eq!(p.splines[0].pos[2], 1.25);
        assert_eq!(p.splines[0].length, 50.0);
        // the rule stays with its spline
        assert_eq!(p.splines[1].rules.len(), 1);
    }

    #[test]
    fn a_height_change_makes_a_spline_h() {
        let t = set_field(TILE, 10, Field::DeltaH, 2.5).unwrap();
        let p = parsed(&t);
        assert_eq!(p.splines[0].delta_h, Some(2.5));
        assert!(p.splines[0].is_h);
        assert_eq!((p.splines[0].cant_start, p.splines[0].tex_offset), (0.0, 0.0));
        assert_eq!(field(&t, 10, Field::DeltaH), Some(2.5));
        let t = set_field(&t, 10, Field::DeltaH, -1.0).unwrap();
        assert_eq!(parsed(&t).splines[0].delta_h, Some(-1.0));
    }

    #[test]
    fn a_new_spline_comes_after_the_others_and_links() {
        let rec = new_record(14, "Splines\\road.sli", 12, 11, [5.0, 86.0, 0.5], 10.0, 20.0, 0.0, 1.0, 80.0, "\r\n");
        let t = add_record(TILE, &rec);
        let t = set_links(&t, 11, None, Some(12)).unwrap();
        let p = parsed(&t);
        assert_eq!(p.splines.len(), 3);
        let s = &p.splines[2];
        assert_eq!((s.id, s.prev_id, s.next_id, s.pos, s.heading, s.length, s.tex_offset), (12, 11, 0, [5.0, 86.0, 0.5], 10.0, 20.0, 80.0));
        assert_eq!(p.splines[1].next_id, 12);
        // the rows and the rule still belong where they did
        assert_eq!(p.spline_attachments[0].spline_index, 1);
        assert_eq!(p.splines[1].rules.len(), 1);
        assert!(p.splines[2].rules.is_empty());
        assert_eq!(p.objects.len(), 1);
    }

    #[test]
    fn a_deleted_spline_takes_its_row_and_unlinks() {
        let (t, prev, next) = delete(TILE, 11).unwrap();
        assert_eq!((prev, next), (10, 0));
        let p = parsed(&t);
        assert_eq!(p.splines.iter().map(|s| s.id).collect::<Vec<_>>(), vec![10]);
        assert_eq!(p.splines[0].next_id, 0);
        assert!(p.spline_attachments.is_empty());
        assert_eq!(p.objects.len(), 1);
        assert!(!t.contains("speedlimit"));
        // the first: the row on the second now counts from 0
        let (t, _, _) = delete(TILE, 10).unwrap();
        let p = parsed(&t);
        assert_eq!(p.spline_attachments[0].spline_index, 0);
        assert_eq!(p.splines[0].prev_id, 0);
    }

    #[test]
    fn the_terrain_flag_switches() {
        assert_eq!(terrain_aligned(TILE, 11), Some(false));
        let t = toggle_terrain_align(TILE, 11).unwrap();
        let p = parsed(&t);
        assert!(p.splines[1].terrain_align_flag && !p.splines[0].terrain_align_flag);
        assert_eq!(p.splines[1].rules.len(), 1);
        assert_eq!(terrain_aligned(&t, 11), Some(true));
        let t = toggle_terrain_align(&t, 11).unwrap();
        assert!(!parsed(&t).splines[1].terrain_align_flag);
        assert_eq!(t.replace("\r\n\r\n\r\n", "\r\n\r\n"), TILE.replace("\r\n\r\n\r\n", "\r\n\r\n"));
        // the first spline (no rules after it)
        let t = toggle_terrain_align(TILE, 10).unwrap();
        assert!(parsed(&t).splines[0].terrain_align_flag);
    }

    #[test]
    fn the_type_changes() {
        let t = set_file(TILE, 10, "Splines\\rail.sli").unwrap();
        assert_eq!(parsed(&t).splines[0].file, "Splines\\rail.sli");
        assert_eq!(parsed(&t).splines[1].file, "Splines\\road.sli");
    }

    #[test]
    fn an_arc_reaches_its_target() {
        use glam::DVec2;
        // straight ahead
        let (l, r, h) = arc_to(DVec2::ZERO, 0.0, DVec2::new(0.0, 30.0)).unwrap();
        assert!((l - 30.0).abs() < 1e-9 && r == 0.0 && h == 0.0);
        // a quarter circle to the right, radius 20
        let (l, r, h) = arc_to(DVec2::ZERO, 0.0, DVec2::new(20.0, 20.0)).unwrap();
        assert!((r - 20.0).abs() < 1e-6, "{r}");
        assert!((l - 20.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((h - 90.0).abs() < 1e-6);
        let c = omsi_geometry::SplineCurve { start: glam::DVec3::ZERO, heading_deg: 0.0, length: l, radius: r, grad_start: 0.0, grad_end: 0.0, delta_h: None, cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: 10.0 };
        assert!((c.end_point().truncate() - DVec2::new(20.0, 20.0)).length() < 1e-6);
        // to the left
        let (_, r, h) = arc_to(DVec2::ZERO, 90.0, DVec2::new(20.0, 20.0)).unwrap();
        assert!(r < 0.0 && (h - 0.0).abs() < 1e-6, "{r} {h}");
    }
}
