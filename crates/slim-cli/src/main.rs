//! `slim` command line interface.
//!
//! Phase 6 provides the real user-facing surface for the schema/layout engine:
//! `slim --help` and `slim schema layout <file.slimschema>` are implemented,
//! with the rest of the command surface left for the later phases.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use slimpack::schema::{Attr, FieldSpec, Layout, LayoutKind, StructSpec, Type};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = if args.is_empty() || (args.len() == 1 && (args[0] == "--help" || args[0] == "-h" || args[0] == "help")) {
        print_help();
        Ok(())
    } else if args.len() == 3 && args[0] == "schema" && args[1] == "layout" {
        parse_schema_layout(&args[2])
    } else if args.len() >= 2 && args[0] == "schema" && args[1] == "layout" {
        eprintln!("usage: slim schema layout <file.slimschema>");
        Err(2)
    } else {
        eprintln!("slim: unknown command. Try `slim --help`.");
        Err(2)
    };

    if let Err(code) = result {
        process::exit(code);
    }
}

fn print_help() {
    println!("slim: SlimPack command line interface");
    println!();
    println!("Usage:");
    println!("  slim --help");
    println!("  slim schema layout <file.slimschema>");
    println!();
    println!("Notes:");
    println!("  - `schema layout` parses a simple `.slimschema` file and prints the computed layout.");
    println!("  - This is the Phase 6 CLI foundation; the rest of the command surface arrives later.");
}

fn parse_schema_layout(path: &str) -> Result<(), i32> {
    let file = PathBuf::from(path);
    let text = fs::read_to_string(&file).map_err(|err| {
        eprintln!("slim: cannot read {}: {err}", file.display());
        2
    })?;

    let spec = parse_schema_text(&text).map_err(|err| {
        eprintln!("slim: invalid schema in {}: {err}", file.display());
        2
    })?;

    let kind = infer_layout_kind(&text).unwrap_or(LayoutKind::Aligned);
    let layout = Layout::compute(&spec, kind).map_err(|err| {
        eprintln!("slim: layout error: {err}");
        2
    })?;

    println!("struct {} ({kind:?})", spec.name);
    println!("  stride: {} bytes", layout.stride());
    for field in &layout.fields {
        let offset = field.byte_offset.map_or("n/a".to_string(), |b| b.to_string());
        let bit = field.bit_offset;
        let width = field.bit_width;
        let attr = match (field.range, field.bits) {
            (Some((min, max)), _) => format!("range({min}, {max})"),
            (None, Some(bits)) => format!("bits({bits})"),
            _ => "none".to_string(),
        };
        println!(
            "  {}: {:?} offset={} bit={} width={} attr={}",
            field.name, field.ty, offset, bit, width, attr
        );
    }
    Ok(())
}

fn infer_layout_kind(text: &str) -> Option<LayoutKind> {
    if text.contains("layout = packed") {
        Some(LayoutKind::Packed)
    } else if text.contains("layout = aligned") {
        Some(LayoutKind::Aligned)
    } else {
        None
    }
}

fn parse_schema_text(text: &str) -> Result<StructSpec, String> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("struct")
        .ok_or_else(|| "expected a `struct` declaration".to_string())?;
    let rest = body.trim_start();
    let mut name_end = 0;
    let mut seen_brace = false;
    for (idx, ch) in rest.char_indices() {
        if ch == '{' {
            name_end = idx;
            seen_brace = true;
            break;
        }
    }
    if !seen_brace {
        return Err("expected `{` after the struct name".to_string());
    }
    let name = rest[..name_end].trim();
    if name.is_empty() {
        return Err("struct name is empty".to_string());
    }
    let after_name = &rest[name_end + 1..];
    let end = after_name.rfind('}').ok_or_else(|| "expected a closing `}`".to_string())?;
    let body = &after_name[..end];
    let mut fields = Vec::new();
    let mut layout_kind = LayoutKind::Aligned;
    let mut item_cursor = 0usize;
    while item_cursor < body.len() {
        while item_cursor < body.len() && body[item_cursor..].chars().next().is_some_and(char::is_whitespace) {
            let ch = body[item_cursor..].chars().next().unwrap();
            item_cursor += ch.len_utf8();
        }
        if item_cursor >= body.len() {
            break;
        }
        let stmt_start = item_cursor;
        let mut depth = 0usize;
        let mut stmt_end = None;
        while item_cursor < body.len() {
            let ch = body[item_cursor..].chars().next().unwrap();
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ';' if depth == 0 => {
                    stmt_end = Some(item_cursor);
                    item_cursor += ch.len_utf8();
                    break;
                }
                _ => {}
            }
            item_cursor += ch.len_utf8();
        }
        let stmt_end = stmt_end.ok_or_else(|| "unterminated statement in schema body".to_string())?;
        let stmt = body[stmt_start..stmt_end].trim();
        if stmt.starts_with("layout") {
            let value = stmt.trim_start_matches("layout").trim();
            let value = value.strip_prefix('=').unwrap_or(value).trim();
            if value.starts_with("packed") {
                layout_kind = LayoutKind::Packed;
            } else if value.starts_with("aligned") {
                layout_kind = LayoutKind::Aligned;
            }
        } else if !stmt.is_empty() {
            fields.push(parse_field(stmt)?);
        }
    }

    let _ = layout_kind;
    Ok(StructSpec::new(name, fields))
}

fn parse_field(statement: &str) -> Result<FieldSpec, String> {
    let (name, remainder) = statement
        .split_once(':')
        .ok_or_else(|| format!("field missing a `:` in `{statement}`"))?;
    let field_name = name.trim();
    if field_name.is_empty() {
        return Err(format!("empty field name in `{statement}`"));
    }

    let mut rest = remainder.trim();
    let ty = parse_type(&mut rest)?;
    let mut field = FieldSpec::new(field_name, ty);
    while !rest.trim().is_empty() {
        let trimmed = rest.trim_start();
        if trimmed.starts_with("range") {
            let call = parse_attr_call(trimmed, "range")?;
            let args = split_top_level_args(&call.args, ',')?;
            if args.len() != 2 {
                return Err(format!("range expects 2 arguments in `{statement}`"));
            }
            let min = parse_int_literal(args[0].trim())?;
            let max = parse_int_literal(args[1].trim())?;
            field = field.with(Attr::Range { min, max });
            rest = &trimmed[call.end..];
        } else if trimmed.starts_with("bits") {
            let call = parse_attr_call(trimmed, "bits")?;
            let args = split_top_level_args(&call.args, ',')?;
            if args.len() != 1 {
                return Err(format!("bits expects 1 argument in `{statement}`"));
            }
            let bits = parse_int_literal(args[0].trim())? as u8;
            field = field.with(Attr::Bits(bits));
            rest = &trimmed[call.end..];
        } else if trimmed.starts_with("quantize") {
            let call = parse_attr_call(trimmed, "quantize")?;
            let args = split_top_level_args(&call.args, ',')?;
            if args.len() != 1 {
                return Err(format!("quantize expects 1 argument in `{statement}`"));
            }
            let value = parse_float_literal(args[0].trim())?;
            field = field.with(Attr::Quantize(value));
            rest = &trimmed[call.end..];
        } else if trimmed.starts_with("optional") {
            field = field.with(Attr::Optional);
            rest = &trimmed["optional".len()..];
        } else {
            return Err(format!("unknown field attribute in `{statement}`"));
        }
    }
    Ok(field)
}

fn parse_type(rest: &mut &str) -> Result<Type, String> {
    let trimmed = rest.trim_start();
    let mut end = None;
    let mut depth = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 && ch.is_whitespace() => {
                end = Some(idx);
                break;
            }
            _ => {}
        }
    }
    let first = match end {
        Some(idx) => &trimmed[..idx],
        None => trimmed,
    };
    let ty = match first {
        "bool" => Type::Bool,
        "i8" => Type::I8,
        "u8" => Type::U8,
        "i16" => Type::I16,
        "u16" => Type::U16,
        "i32" => Type::I32,
        "u32" => Type::U32,
        "i64" => Type::I64,
        "u64" => Type::U64,
        "f32" => Type::F32,
        "f64" => Type::F64,
        "str" => Type::Str,
        "bytes" => Type::Bytes,
        "Vec" => {
            let inner = trimmed.strip_prefix("Vec").and_then(|rest| rest.strip_prefix('<'))
                .and_then(|rest| rest.rsplit_once('>'))
                .map(|(inside, _)| inside.trim())
                .ok_or_else(|| "Vec requires `<T>` syntax".to_string())?;
            let mut inner_rest = inner;
            let inner_ty = parse_type(&mut inner_rest)?;
            Type::Vec(Box::new(inner_ty))
        }
        other => Type::Struct(other.to_string()),
    };
    if let Some(idx) = end {
        *rest = &trimmed[idx..];
    } else {
        *rest = "";
    }
    Ok(ty)
}

struct AttrCall<'a> {
    args: &'a str,
    end: usize,
}

fn parse_attr_call<'a>(text: &'a str, name: &str) -> Result<AttrCall<'a>, String> {
    let after_name = text
        .strip_prefix(name)
        .ok_or_else(|| format!("expected `{name}`"))?;
    let open = after_name
        .find('(')
        .ok_or_else(|| format!("expected `(` after `{name}`"))?;
    let mut depth = 0usize;
    let mut close = None;
    for (offset, ch) in after_name[open + 1..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    close = Some(open + 1 + offset);
                    break;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    let close = close.ok_or_else(|| format!("missing closing `)` in `{name}`"))?;
    let args = &after_name[open + 1..close];
    let pos = close + 1;
    Ok(AttrCall {
        args,
        end: name.len() + pos,
    })
}

fn split_top_level_args(text: &str, delim: char) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c == delim && depth == 0 => {
                parts.push(&text[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    if !text[start..].trim().is_empty() {
        parts.push(&text[start..]);
    }
    Ok(parts.iter().map(|v| v.trim()).filter(|v| !v.is_empty()).collect())
}

fn parse_int_literal(token: &str) -> Result<i128, String> {
    token.trim().parse::<i128>().map_err(|_| format!("invalid integer `{token}`"))
}

fn parse_float_literal(token: &str) -> Result<f64, String> {
    token.trim().parse::<f64>().map_err(|_| format!("invalid float `{token}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_schema_and_layout() {
        let schema = parse_schema_text(
            "struct Telemetry {\n  rsrp: i16 range(-140, -44);\n  band: u8;\n  speed: f32;\n}\n",
        )
        .unwrap();

        assert_eq!(schema.name, "Telemetry");
        assert_eq!(schema.fields.len(), 3);
        assert_eq!(schema.fields[0].ty, Type::I16);
        assert_eq!(schema.fields[0].attrs[0], Attr::Range { min: -140, max: -44 });

        let layout = Layout::compute(&schema, LayoutKind::Aligned).unwrap();
        assert_eq!(layout.stride(), 7);
    }
}
