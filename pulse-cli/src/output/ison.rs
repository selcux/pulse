//! Encodes `Report::tables()` as ISON v1.0 (<https://www.ison.dev/spec.html>).
//!
//! Only `table` and `object` blocks are written: no type annotations, summary rows, references
//! or column alignment.

use anyhow::{Context, bail};
use serde_json::{Map, Value};

/// Encodes a `Report::tables()` value as an ISON v1.0 document.
///
/// The root must be an object whose keys are block names, written in order:
///
/// - a non-empty array of flat objects becomes a `table.<key>` block;
/// - a flat object becomes an `object.<key>` block with one row;
/// - a group (a non-empty object whose values are all arrays) becomes one
///   `table.<key>.<member>` block per non-empty member;
/// - `null`, an empty array, an empty object and an empty group member are left out.
///
/// Blocks are separated by one empty line, with no trailing newline. When every table is left
/// out the document is the empty string.
///
/// # Errors
///
/// Fails, with a message starting `ISON:`, when the value is outside that contract: a nested
/// cell, rows with different fields, a scalar table, a mixed object, or a name that cannot be
/// written unquoted.
pub fn encode(tables: &Value) -> anyhow::Result<String> {
    let Some(root) = tables.as_object() else {
        bail!("ISON: the tables value must be an object");
    };
    let mut blocks = Vec::new();
    for (key, value) in root {
        match value {
            Value::Null => {}
            Value::Array(rows) => blocks.extend(table_block(&[key], rows)?),
            Value::Object(fields) if fields.is_empty() => {}
            Value::Object(fields) if fields.values().all(Value::is_array) => {
                for (member, rows) in fields {
                    let rows = rows.as_array().map_or(&[][..], Vec::as_slice);
                    blocks.extend(table_block(&[key, member], rows)?);
                }
            }
            Value::Object(fields) => blocks.extend(object_block(key, fields)?),
            _ => bail!("ISON: `{key}` must be an array, an object or null"),
        }
    }
    Ok(blocks.join("\n\n"))
}

/// A `table` block named by `path` (joined with dots), or `None` when there are no rows.
fn table_block(path: &[&str], rows: &[Value]) -> anyhow::Result<Option<String>> {
    let rows = rows
        .iter()
        .map(|row| {
            row.as_object()
                .with_context(|| format!("ISON: a row of `{}` is not an object", path.join(".")))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    write_block("table", path, &rows)
}

/// An `object` block with a single row.
fn object_block(name: &str, fields: &Map<String, Value>) -> anyhow::Result<Option<String>> {
    if fields
        .values()
        .any(|value| value.is_array() || value.is_object())
    {
        bail!("ISON: `{name}` mixes nested values with plain fields");
    }
    write_block("object", &[name], &[fields])
}

/// `kind.name`, the field line, then one line per row; `None` when there are no rows.
fn write_block(
    kind: &str,
    path: &[&str],
    rows: &[&Map<String, Value>],
) -> anyhow::Result<Option<String>> {
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let name = path.join(".");
    // Each part is checked on its own: an empty member would still make a non-empty `sets.`.
    for part in path {
        if let Some(problem) = name_problem(part) {
            bail!("ISON: block name `{name}`: `{part}` {problem}");
        }
    }
    let fields: Vec<&str> = first.keys().map(String::as_str).collect();
    if fields.is_empty() {
        bail!("ISON: `{name}` has rows without fields");
    }
    for field in &fields {
        if let Some(problem) = field_problem(field) {
            bail!("ISON: field `{field}` of `{name}` {problem}");
        }
    }

    let mut lines = vec![format!("{kind}.{name}"), fields.join(" ")];
    for row in rows {
        if !row.keys().map(String::as_str).eq(fields.iter().copied()) {
            bail!("ISON: the rows of `{name}` do not all have the same fields");
        }
        let cells = row
            .iter()
            .map(|(field, value)| {
                cell(value).with_context(|| format!("ISON: field `{field}` of `{name}`"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        lines.push(cells.join(" "));
    }
    Ok(Some(lines.join("\n")))
}

/// Why `name` cannot be written bare in a block header, if it cannot.
fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("is empty")
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Some("contains whitespace or a control character")
    } else {
        None
    }
}

/// Why `name` cannot be written bare on a field line, if it cannot.
fn field_problem(name: &str) -> Option<&'static str> {
    name_problem(name).or_else(|| {
        (name.contains(':') || name.starts_with('#')).then_some("contains `:` or starts with `#`")
    })
}

fn cell(value: &Value) -> anyhow::Result<String> {
    Ok(match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(x) if n.is_f64() => float_cell(x),
            _ => n.to_string(),
        },
        Value::String(s) => string_cell(s),
        Value::Array(_) | Value::Object(_) => bail!("a cell cannot hold an array or an object"),
    })
}

/// A float always carries a decimal point: ISON reads `48` as an integer.
fn float_cell(x: f64) -> String {
    let text = x.to_string();
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// `s` bare when a reader would take it for the same string, otherwise quoted and escaped.
fn string_cell(s: &str) -> String {
    if !needs_quotes(s) {
        return s.to_string();
    }
    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('"');
    for c in s.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn needs_quotes(s: &str) -> bool {
    s.is_empty()
        || matches!(s, "true" | "false" | "null" | "~")
        // Covers every integer and float ISON reads, and the forms (`1e5`, `inf`, `+1`) that
        // only some parsers read as numbers.
        || s.parse::<f64>().is_ok()
        // A reference, a comment, and (ison-py) a summary separator as the first cell. The
        // reference serializers leave `---` bare; we are stricter on purpose.
        || s.starts_with([':', '#'])
        || s.starts_with("---")
        || s.contains(['"', '\\'])
        // Unicode whitespace at a line end is trimmed by the reference parsers, and v1.0 has
        // no escape for control characters. Stricter than the reference serializers on purpose.
        || s.chars().any(|c| c.is_whitespace() || c.is_control())
        || looks_like_block_header(s)
}

/// `ident.ident` alone on a line reads as a block header.
fn looks_like_block_header(s: &str) -> bool {
    let is_ident = |part: &str| {
        let mut chars = part.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    s.split_once('.')
        .is_some_and(|(kind, name)| is_ident(kind) && is_ident(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// (input, cell). Normal literals where the cell holds a raw control or non-ASCII character.
    const CELLS: &[(&str, &str)] = &[
        // bare
        ("garmin", "garmin"),
        ("a,b", "a,b"),
        ("-lead", "-lead"),
        ("--x", "--x"),
        ("[x]", "[x]"),
        ("pull-up", "pull-up"),
        ("2026-01-02", "2026-01-02"),
        ("2026-01-02T07:30:00", "2026-01-02T07:30:00"),
        ("i101", "i101"),
        ("a.b.c", "a.b.c"),
        ("v1.2", "v1.2"),
        ("a#b", "a#b"),
        ("1_000", "1_000"),
        // quoted: empty, keywords, anything that parses as a number
        ("", r#""""#),
        ("true", r#""true""#),
        ("false", r#""false""#),
        ("null", r#""null""#),
        ("~", r#""~""#),
        ("123", r#""123""#),
        ("-7", r#""-7""#),
        ("-2.5", r#""-2.5""#),
        ("21234567890", r#""21234567890""#),
        ("1e5", r#""1e5""#),
        ("inf", r#""inf""#),
        ("NaN", r#""NaN""#),
        ("+1", r#""+1""#),
        (".5", r#"".5""#),
        // quoted: meaning at the start of a token or line
        (":ref", r#"":ref""#),
        ("#tag", r##""#tag""##),
        ("---", r#""---""#),
        ("table.users", r#""table.users""#),
        // quoted: whitespace, quote, backslash, control characters
        ("plain text", r#""plain text""#),
        ("Bench Press", r#""Bench Press""#),
        (" pad ", r#"" pad ""#),
        ("a: b", r#""a: b""#),
        ("caf\u{e9} \u{2014} ok", "\"caf\u{e9} \u{2014} ok\""),
        ("say \"hi\"", r#""say \"hi\"""#),
        ("back\\slash", r#""back\\slash""#),
        ("line\nbreak", r#""line\nbreak""#),
        ("tab\there", r#""tab\there""#),
        ("cr\rhere", r#""cr\rhere""#),
        ("nbsp\u{a0}end", "\"nbsp\u{a0}end\""),
        ("bell\u{7}", "\"bell\u{7}\""),
    ];

    const FLOATS: &[(f64, &str)] = &[
        (48.0, "48.0"),
        (51.5, "51.5"),
        (100.0, "100.0"),
        (0.0, "0.0"),
        (-0.0, "-0.0"),
        (-3.5, "-3.5"),
        (1.05, "1.05"),
        (90.20833333333333, "90.20833333333333"),
        (1e21, "1000000000000000000000.0"),
        (1e-7, "0.0000001"),
    ];

    #[test]
    fn string_cells_quote_only_what_the_spec_requires() {
        for (input, cell) in CELLS {
            assert_eq!(string_cell(input), *cell, "{input:?}");
        }
    }

    #[test]
    fn float_cells_keep_a_decimal_point() {
        for (x, cell) in FLOATS {
            assert_eq!(float_cell(*x), *cell, "{x:?}");
        }
    }

    #[test]
    fn encode_prints_integers_and_booleans() {
        let tables = json!({"t": [
            {"b": true, "i": -5, "n": 27000},
            {"b": false, "i": 0, "n": i64::MAX},
        ]});
        assert_eq!(
            encode(&tables).unwrap(),
            "table.t\nb i n\ntrue -5 27000\nfalse 0 9223372036854775807"
        );
    }

    #[test]
    fn encode_writes_a_flat_object_as_one_object_row() {
        let tables = json!({"pace": {"trend": "steady", "multiplier": 1.05}});
        assert_eq!(
            encode(&tables).unwrap(),
            "object.pace\ntrend multiplier\nsteady 1.05"
        );
    }

    #[test]
    fn encode_writes_a_block_per_group_member() {
        let tables = json!({"sets": {"w1": [{"n": 1}], "w2": [], "w3": [{"n": 2}, {"n": 3}]}});
        assert_eq!(
            encode(&tables).unwrap(),
            "table.sets.w1\nn\n1\n\ntable.sets.w3\nn\n2\n3"
        );
    }

    #[test]
    fn encode_leaves_out_null_and_empty_tables() {
        assert_eq!(encode(&json!({"a": null, "b": [], "c": {}})).unwrap(), "");
    }

    #[test]
    fn encode_rejects_a_nested_cell() {
        assert!(encode(&json!({"t": [{"a": [1]}]})).is_err());
        assert!(encode(&json!({"t": [{"a": {"b": 1}}]})).is_err());
    }

    #[test]
    fn encode_rejects_rows_with_different_fields() {
        let inconsistent = [
            json!({"t": [{"a": 1}, {"b": 1}]}),
            json!({"t": [{"a": 1, "b": 2}, {"b": 2, "a": 1}]}),
            json!({"t": [{"a": 1}, {"a": 1, "b": 2}]}),
        ];
        for tables in inconsistent {
            assert!(encode(&tables).is_err(), "{tables}");
        }
    }

    #[test]
    fn encode_rejects_an_unwritable_field_name() {
        for name in ["", "first name", "a:b", "#x", "tab\tname"] {
            let mut row = serde_json::Map::new();
            row.insert(name.to_string(), json!(1));
            let tables = json!({"t": [Value::Object(row)]});
            assert!(encode(&tables).is_err(), "{name:?}");
        }
    }

    #[test]
    fn encode_rejects_rows_without_fields() {
        assert!(encode(&json!({"t": [{}]})).is_err());
    }

    #[test]
    fn encode_rejects_a_non_object_row() {
        assert!(encode(&json!({"t": [1]})).is_err());
    }

    #[test]
    fn encode_rejects_an_unwritable_block_name() {
        assert!(encode(&json!({"w 1": [{"n": 1}]})).is_err());
        assert!(encode(&json!({"sets": {"w 1": [{"n": 1}]}})).is_err());
        assert!(encode(&json!({"sets": {"": [{"n": 1}]}})).is_err());
    }

    #[test]
    fn encode_rejects_shapes_outside_the_tables_contract() {
        let outside = [
            json!([]),
            json!(null),
            json!({"t": 5}),
            json!({"t": "x"}),
            json!({"t": {"a": [{"n": 1}], "b": 2}}),
        ];
        for tables in outside {
            assert!(encode(&tables).is_err(), "{tables}");
        }
    }
}
