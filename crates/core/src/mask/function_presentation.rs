//! Lossless editor spans and construction for the native text-function subset.
use super::{parse_call, parse_until, Mask, Segment};

#[derive(Debug, Clone)]
pub struct TextFunction {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub args: &'static [&'static str],
}
pub fn text_functions() -> Vec<TextFunction> {
    [
        ("upper", "Uppercase", "Make text uppercase.", &["Value"][..]),
        ("lower", "Lowercase", "Make text lowercase.", &["Value"][..]),
        ("trim", "Trim", "Remove outer spaces.", &["Value"][..]),
        (
            "strpos",
            "Find",
            "Find a text position.",
            &["Value", "Find"][..],
        ),
        (
            "replace",
            "Replace",
            "Replace matching text.",
            &["Value", "Find", "Replace with"][..],
        ),
    ]
    .into_iter()
    .map(|(key, label, description, args)| TextFunction {
        key,
        label,
        description,
        args,
    })
    .collect()
}
#[derive(Debug, Clone)]
pub struct FunctionSpan {
    pub start: usize,
    pub end: usize,
    pub key: String,
    pub args: Vec<String>,
    pub display_args: Vec<String>,
    pub raw: String,
}
fn bounded(value: &str) -> Result<(), String> {
    if value.len() > 65536 {
        Err("Pattern exceeds the editor size limit".into())
    } else {
        Ok(())
    }
}
fn call_at(chars: &[char], start: usize) -> Option<(usize, String, Vec<String>)> {
    let mut end = start;
    let Segment::Call(function, _) = parse_call(chars, &mut end, 0).ok()?? else {
        return None;
    };
    let mut cursor = start + 1;
    while chars.get(cursor) != Some(&'(') {
        cursor += 1;
    }
    cursor += 1;
    let mut args = Vec::new();
    loop {
        let begin = cursor;
        let (_, closed) = parse_until(chars, &mut cursor, &[',', ')'], 1).ok()?;
        args.push(chars[begin..cursor.checked_sub(1)?].iter().collect());
        if closed == Some(')') {
            break;
        }
        if closed.is_none() {
            return None;
        }
    }
    Some((end, function.name().to_string(), args))
}
fn display_argument(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '$' {
            if let Some((end, _, _)) = call_at(&chars, at) {
                out.extend(chars[at..end].iter());
                at = end;
                continue;
            }
        }
        if chars[at] == '\'' {
            if let Some(offset) = chars[at + 1..].iter().position(|c| *c == '\'') {
                let end = at + 1 + offset;
                let quoted: String = chars[at + 1..end].iter().collect();
                if quoted.contains(['%', '$']) {
                    out.extend(chars[at..=end].iter());
                } else if quoted.is_empty() {
                    out.push('\'');
                } else {
                    out.push_str(&quoted);
                }
                at = end + 1;
                continue;
            }
        }
        out.push(chars[at]);
        at += 1;
    }
    out
}
pub fn function_spans(pattern: &str) -> Result<Vec<FunctionSpan>, String> {
    bounded(pattern)?;
    let chars: Vec<char> = pattern.chars().collect();
    let utf16: Vec<usize> = std::iter::once(0)
        .chain(chars.iter().scan(0, |offset, ch| {
            *offset += ch.len_utf16();
            Some(*offset)
        }))
        .collect();
    let mut spans = Vec::new();
    let mut at = 0;
    let mut quoted = false;
    while at < chars.len() {
        if chars[at] == '\'' {
            quoted = !quoted;
            at += 1;
            continue;
        }
        if !quoted && chars[at] == '$' {
            if let Some((end, key, args)) = call_at(&chars, at) {
                if text_functions().iter().any(|function| function.key == key) {
                    spans.push(FunctionSpan {
                        start: utf16[at],
                        end: utf16[end],
                        key,
                        display_args: args.iter().map(|arg| display_argument(arg)).collect(),
                        args,
                        raw: chars[at..end].iter().collect(),
                    });
                }
                at = end;
                continue;
            }
        }
        at += 1;
    }
    Ok(spans)
}
fn encode_argument(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '$' {
            if let Some((end, _, _)) = call_at(&chars, at) {
                out.extend(chars[at..end].iter());
                at = end;
                continue;
            }
        }
        if chars[at] == '%' {
            if let Some(offset) = chars[at + 1..].iter().position(|c| *c == '%') {
                let end = at + offset + 2;
                let token: String = chars[at..end].iter().collect();
                if Mask::parse(&token).is_ok() {
                    out.push_str(&token);
                    at = end;
                    continue;
                }
            }
        }
        let ch = chars[at];
        at += 1;
        match ch {
            '\'' => out.push_str("''"),
            '(' | ')' | ',' | '[' | ']' | '%' | '$' => {
                out.push('\'');
                out.push(ch);
                out.push('\'');
            }
            _ => out.push(ch),
        }
    }
    out
}
pub fn build_text_function(
    key: &str,
    args: &[String],
    original: Option<&str>,
) -> Result<String, String> {
    let functions = text_functions();
    let function = functions
        .iter()
        .find(|function| function.key == key)
        .ok_or("Unsupported text function")?;
    if function.args.len() != args.len() {
        return Err("Incorrect argument count".into());
    }
    for arg in args {
        bounded(arg)?;
    }
    let prior = original
        .map(function_spans)
        .transpose()?
        .and_then(|spans| spans.into_iter().next());
    if let Some(prior) = &prior {
        if prior.key == key && prior.display_args == args {
            return Ok(prior.raw.clone());
        }
    }
    let encoded: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(index, value)| {
            prior
                .as_ref()
                .filter(|prior| prior.display_args.get(index) == Some(value))
                .and_then(|prior| prior.args.get(index))
                .cloned()
                .unwrap_or_else(|| encode_argument(value))
        })
        .collect();
    let out = format!("${key}({})", encoded.join(","));
    bounded(&out)?;
    Mask::parse(&out).map_err(|error| error.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TagField, TagMap};
    #[test]
    fn spans_use_native_grammar_and_utf16_offsets() {
        let raw = "🎵 $replace($upper(%artist%),',','(')";
        let spans = function_spans(raw).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, 3);
        assert_eq!(spans[0].args, ["$upper(%artist%)", "','", "'('"]);
        assert_eq!(spans[0].display_args[1], ",");
        assert!(function_spans("$if2(%artist%,%title%)").unwrap().is_empty());
        assert!(function_spans("$upper(%artist%").unwrap().is_empty());
        // A literal opening parenthesis is not a nested native function.
        assert_eq!(
            function_spans("$upper(a(b))").unwrap()[0].raw,
            "$upper(a(b)"
        );
    }
    #[test]
    fn literal_arguments_and_nested_functions_render_correctly() {
        let raw = build_text_function(
            "replace",
            &["%title%".into(), ",".into(), "it's John's".into()],
            None,
        )
        .unwrap();
        let mut tags = TagMap::new();
        tags.insert(TagField::Title, "A,B".into());
        assert_eq!(
            Mask::parse(&raw)
                .unwrap()
                .render_value_with(&tags, &Default::default())
                .unwrap(),
            "Ait's John'sB"
        );
        let raw = build_text_function("upper", &["$trim(%title%)".into()], None).unwrap();
        assert_eq!(raw, "$upper($trim(%title%))");
    }
    #[test]
    fn editing_preserves_untouched_raw_arguments_and_deletion_replacement() {
        let original = "$REPLACE(%title%,'a,b',)";
        let args = vec!["%title%".into(), "a,b".into(), "".into()];
        assert_eq!(
            build_text_function("replace", &args, Some(original)).unwrap(),
            original
        );
        let updated = build_text_function(
            "replace",
            &["%artist%".into(), "a,b".into(), "".into()],
            Some(original),
        )
        .unwrap();
        assert_eq!(updated, "$replace(%artist%,'a,b',)");
        assert!(build_text_function("if2", &args, None).is_err());
    }
}
