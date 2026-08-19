//! Format-level helpers shared by the workspace build scripts that scrape LSB
//! (vendor/server) SQL dumps, lua enums, and C++ headers into compile-time
//! Rust tables. Source-file-specific orchestration stays in each crate's
//! build.rs; only generic-signature walkers and table writers live here.

use std::fs;

use anyhow::{bail, Context, Result};

pub fn parse_int_lit(s: &str) -> Option<u16> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

pub fn parse_cpp_enum_class(src: &str, enum_name: &str) -> Result<Vec<(u32, String)>> {
    parse_cpp_enum_body(src, &format!("enum class {enum_name}"))
}

pub fn parse_cpp_plain_enum(src: &str, enum_name: &str) -> Result<Vec<(u32, String)>> {
    parse_cpp_enum_body(src, &format!("enum {enum_name}"))
}

fn parse_cpp_enum_body(src: &str, needle: &str) -> Result<Vec<(u32, String)>> {
    let header = src
        .find(needle)
        .with_context(|| format!("could not locate `{needle}` in source"))?;
    let body_start = src[header..]
        .find('{')
        .with_context(|| format!("no opening `{{` after `{needle}`"))?
        + header
        + 1;
    let body_end = src[body_start..]
        .find('}')
        .with_context(|| format!("no closing `}}` after `{needle}`"))?
        + body_start;

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in src[body_start..body_end].lines() {
        let line = line.trim();
        let Some(eq) = line.find('=') else { continue };
        let ident = line[..eq].trim();
        if ident.is_empty() || !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let value = line[eq + 1..].split([',', '/']).next().unwrap_or("").trim();
        let Some(id) = parse_int_lit(value) else {
            continue;
        };
        if seen.insert(id) {
            out.push((id as u32, ident.to_string()));
        }
    }
    if out.is_empty() {
        bail!("parsed zero entries for `{needle}` — header format may have changed");
    }
    out.sort_by_key(|(id, _)| *id);
    Ok(out)
}

pub fn parse_packet_enum(src: &str, prefix: &str) -> Result<Vec<(u32, String)>> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        let Some(eq) = rest.find('=') else { continue };
        let name = rest[..eq].trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let num_str = rest[eq + 1..].trim().trim_end_matches(',').trim();
        let Some(id) = parse_int_lit(num_str) else {
            continue;
        };
        if seen.insert(id) {
            out.push((id as u32, name.to_string()));
        }
    }
    if out.is_empty() {
        bail!("parsed zero `{prefix}*` packet ids — packet enum format may have changed");
    }
    Ok(out)
}

pub fn parse_xi_ident_table(src: &str, needle_prefix: &str) -> Result<Vec<(u32, String)>> {
    let needle = format!("{needle_prefix} =");
    let header = src
        .find(&needle)
        .with_context(|| format!("could not locate `{needle}` in source"))?;
    let body_start = src[header..]
        .find('{')
        .with_context(|| format!("no opening `{{` after `{needle}`"))?
        + header
        + 1;

    let body_end = src[body_start..]
        .find('}')
        .with_context(|| format!("no closing `}}` after `{needle}`"))?
        + body_start;
    let body = &src[body_start..body_end];

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in body.lines() {
        let line = line.trim();
        let Some(eq) = line.find('=') else { continue };
        let ident = line[..eq].trim();
        if ident.is_empty()
            || !ident
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            continue;
        }
        if !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let tail = line[eq + 1..].trim_start();
        let num_str: String = tail
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        let Ok(id) = num_str.parse::<u32>() else {
            continue;
        };
        let pretty = prettify_snake_case(ident);
        if seen.insert(id) {
            out.push((id, pretty));
        }
    }
    if out.is_empty() {
        bail!("parsed zero entries for `{needle_prefix}` — source format may have changed");
    }
    Ok(out)
}

// `field` selects which quoted string in each `[id] = { 'ABBR', 'Full Name' }`
// row to keep: 1 = abbreviation, 3 = display name (split-by-`'` part index).
pub fn parse_lua_indexed_pair_table(
    src: &str,
    needle_prefix: &str,
    field: usize,
) -> Result<Vec<(u32, String)>> {
    let needle = format!("{needle_prefix} =");
    let header = src
        .find(&needle)
        .with_context(|| format!("could not locate `{needle}` in source"))?;
    let body_start = src[header..]
        .find('{')
        .with_context(|| format!("no opening `{{` after `{needle}`"))?
        + header
        + 1;
    let body = &src[body_start..];

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in body.lines() {
        let line = line.trim();

        let Some(open) = line.find('[') else { continue };
        let Some(close) = line[open + 1..].find(']') else {
            continue;
        };
        let id_str = line[open + 1..open + 1 + close].trim();
        let Ok(id) = id_str.parse::<u32>() else {
            continue;
        };

        let rest = &line[open + 1 + close..];
        let parts: Vec<&str> = rest.split('\'').collect();

        if parts.len() <= field {
            continue;
        }
        let display = parts[field].trim();
        if display.is_empty() {
            continue;
        }
        if seen.insert(id) {
            out.push((id, display.to_string()));
        }
    }
    if out.is_empty() {
        bail!("parsed zero entries for `{needle_prefix}` — source format may have changed");
    }
    Ok(out)
}

pub fn parse_sql_insert_rows(
    src: &str,
    table: &str,
    id_field: usize,
    name_field: usize,
) -> Result<Vec<(u32, String)>> {
    let needle = format!("INSERT INTO `{table}` VALUES ");
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(needle.as_str()) else {
            continue;
        };

        let mut cursor = rest;
        while let Some(open) = cursor.find('(') {
            cursor = &cursor[open + 1..];
            let Some((tuple, after)) = split_sql_tuple(cursor) else {
                break;
            };
            cursor = after;
            let fields = split_sql_fields(tuple);
            let id_str = fields.get(id_field).map(|s| s.trim()).unwrap_or("");
            let name_raw = fields.get(name_field).map(|s| s.trim()).unwrap_or("");
            let Ok(id) = id_str.parse::<u32>() else {
                continue;
            };
            let Some(name) = strip_sql_string(name_raw) else {
                continue;
            };

            if name.is_empty()
                || name.chars().all(|c| c == '_')
                || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            {
                continue;
            }
            out.push((id, prettify_snake_case(&name)));
        }
    }
    if out.is_empty() {
        bail!("parsed zero rows from `INSERT INTO {table}` — SQL format may have changed");
    }
    Ok(out)
}

pub fn parse_u16_pair_rows(src: &str, table: &str, value_field: usize) -> Result<Vec<(u16, u16)>> {
    let needle = format!("INSERT INTO `{table}` VALUES ");
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(needle.as_str()) else {
            continue;
        };
        let mut cursor = rest;
        while let Some(open) = cursor.find('(') {
            cursor = &cursor[open + 1..];
            let Some((tuple, after)) = split_sql_tuple(cursor) else {
                break;
            };
            cursor = after;
            let fields = split_sql_fields(tuple);
            let Some(Ok(id)) = fields.first().map(|s| s.trim().parse::<u16>()) else {
                continue;
            };
            let value = fields
                .get(value_field)
                .and_then(|s| s.trim().parse::<u16>().ok())
                .unwrap_or(0);
            out.push((id, value));
        }
    }
    if out.is_empty() {
        bail!("parsed zero rows from `INSERT INTO {table}` — SQL format may have changed");
    }
    Ok(out)
}

pub fn parse_u32_pair_rows(src: &str, table: &str, value_field: usize) -> Result<Vec<(u16, u32)>> {
    let needle = format!("INSERT INTO `{table}` VALUES ");
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(needle.as_str()) else {
            continue;
        };
        let mut cursor = rest;
        while let Some(open) = cursor.find('(') {
            cursor = &cursor[open + 1..];
            let Some((tuple, after)) = split_sql_tuple(cursor) else {
                break;
            };
            cursor = after;
            let fields = split_sql_fields(tuple);
            let Some(Ok(id)) = fields.first().map(|s| s.trim().parse::<u16>()) else {
                continue;
            };
            let value = fields
                .get(value_field)
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(0);
            out.push((id, value));
        }
    }
    if out.is_empty() {
        bail!("parsed zero rows from `INSERT INTO {table}` — SQL format may have changed");
    }
    Ok(out)
}

pub fn split_sql_tuple(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_string = false;
            }
            i += 1;
        } else {
            if c == b'\'' {
                in_string = true;
                i += 1;
                continue;
            }
            if c == b')' {
                return Some((&s[..i], &s[i + 1..]));
            }
            i += 1;
        }
    }
    None
}

pub fn split_sql_fields(tuple: &str) -> Vec<&str> {
    let bytes = tuple.as_bytes();
    let mut fields = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_string = false;
            }
            i += 1;
        } else if c == b'\'' {
            in_string = true;
            i += 1;
        } else if c == b',' {
            fields.push(&tuple[start..i]);
            start = i + 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    fields.push(&tuple[start..]);
    fields
}

pub fn strip_sql_string(field: &str) -> Option<String> {
    let f = field.trim();
    let stripped = f.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))?;
    Some(stripped.replace("''", "'"))
}

pub fn prettify_snake_case(s: &str) -> String {
    const ROMAN: &[&str] = &[
        "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv",
        "xv",
    ];
    const CONNECTORS: &[&str] = &["of", "the", "a", "an", "in", "on", "and", "to"];
    let words: Vec<&str> = s.split('_').filter(|w| !w.is_empty()).collect();
    let mut out = String::with_capacity(s.len() + words.len());
    for (idx, word) in words.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        let lower = word.to_ascii_lowercase();
        if ROMAN.iter().any(|r| *r == lower) {
            out.push_str(&lower.to_ascii_uppercase());
        } else if idx > 0 && CONNECTORS.iter().any(|c| *c == lower) {
            out.push_str(&lower);
        } else {
            let mut chars = lower.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    out
}

pub fn rust_string_literal(s: &str) -> String {
    if !s.contains('"') && !s.contains('\\') {
        format!("\"{s}\"")
    } else {
        let mut esc = String::with_capacity(s.len() + 2);
        esc.push('"');
        for c in s.chars() {
            match c {
                '"' => esc.push_str("\\\""),
                '\\' => esc.push_str("\\\\"),
                _ => esc.push(c),
            }
        }
        esc.push('"');
        esc
    }
}

pub fn write_u16_table(
    out_path: &std::path::Path,
    const_name: &str,
    source_path: &str,
    entries: &[(u32, String)],
) -> Result<()> {
    let mut entries: Vec<(u32, String)> = entries.to_vec();
    entries.sort_by_key(|(id, _)| *id);
    entries.dedup_by_key(|(id, _)| *id);

    for (id, name) in &entries {
        if *id > u16::MAX as u32 {
            bail!("entry id {id} ({name:?}) overflows u16 — table needs widening");
        }
    }
    let mut out = String::new();
    out.push_str(&format!(
        "// AUTO-GENERATED by ffxi-proto/build.rs from {source_path}.\n"
    ));
    out.push_str("// Do not edit by hand.\n");
    out.push_str(&format!("pub const {const_name}: &[(u16, &str)] = &[\n"));
    for (id, text) in &entries {
        out.push_str(&format!("    ({id}, {}),\n", rust_string_literal(text)));
    }
    out.push_str("];\n");
    fs::write(out_path, &out)?;
    Ok(())
}

pub fn write_u16_u8_table(
    out_path: &std::path::Path,
    const_name: &str,
    source_path: &str,
    entries: &[(u16, u8)],
) -> Result<()> {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|(id, _)| *id);
    entries.dedup_by_key(|(id, _)| *id);

    let mut out = String::new();
    out.push_str(&format!(
        "// AUTO-GENERATED by ffxi-proto/build.rs from {source_path}.\n"
    ));
    out.push_str("// Do not edit by hand.\n");
    out.push_str(&format!("pub const {const_name}: &[(u16, u8)] = &[\n"));
    for (id, skill) in &entries {
        out.push_str(&format!("    ({id}, {skill}),\n"));
    }
    out.push_str("];\n");
    fs::write(out_path, &out)?;
    Ok(())
}

pub fn write_u16_u16_table(
    out_path: &std::path::Path,
    const_name: &str,
    source_path: &str,
    entries: &[(u16, u16)],
) -> Result<()> {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|(id, _)| *id);
    entries.dedup_by_key(|(id, _)| *id);

    let mut out = String::new();
    out.push_str(&format!(
        "// AUTO-GENERATED by ffxi-proto/build.rs from {source_path}.\n"
    ));
    out.push_str("// Do not edit by hand.\n");
    out.push_str(&format!("pub const {const_name}: &[(u16, u16)] = &[\n"));
    for (id, value) in &entries {
        out.push_str(&format!("    ({id}, {value}),\n"));
    }
    out.push_str("];\n");
    fs::write(out_path, &out)?;
    Ok(())
}

pub fn write_u16_u32_table(
    out_path: &std::path::Path,
    const_name: &str,
    source_path: &str,
    entries: &[(u16, u32)],
) -> Result<()> {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|(id, _)| *id);
    entries.dedup_by_key(|(id, _)| *id);

    let mut out = String::new();
    out.push_str(&format!(
        "// AUTO-GENERATED by ffxi-proto/build.rs from {source_path}.\n"
    ));
    out.push_str("// Do not edit by hand.\n");
    out.push_str(&format!("pub const {const_name}: &[(u16, u32)] = &[\n"));
    for (id, value) in &entries {
        out.push_str(&format!("    ({id}, {value:#x}),\n"));
    }
    out.push_str("];\n");
    fs::write(out_path, &out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prettify_handles_roman_numerals_and_connectors() {
        assert_eq!(prettify_snake_case("cure"), "Cure");
        assert_eq!(prettify_snake_case("cure_iv"), "Cure IV");
        assert_eq!(prettify_snake_case("cure_vi"), "Cure VI");
        assert_eq!(
            prettify_snake_case("pile_of_chocobo_bedding"),
            "Pile of Chocobo Bedding"
        );
        assert_eq!(prettify_snake_case("BLAZE_SPIKES"), "Blaze Spikes");
        assert_eq!(prettify_snake_case("mighty_strikes"), "Mighty Strikes");
    }

    #[test]
    fn sql_tuple_handles_escaped_quotes() {
        let (body, after) = split_sql_tuple("1,'it''s',2);extra").unwrap();
        assert_eq!(body, "1,'it''s',2");
        assert_eq!(after, ";extra");
        let fields = split_sql_fields(body);
        assert_eq!(fields.len(), 3);
        assert_eq!(strip_sql_string(fields[1]).unwrap(), "it's");
    }
}
