//! JSON as the tools that write it are read: by the gate that reads `package.json` as npm does
//! (`tests/gates.rs`), and by `mn evidence`, which reads the list of tests nextest writes. One
//! reader, defined once.

/// A JSON value as the guard reads package.json: every key of an object in order, duplicates kept,
/// so that a second `"scripts"` key cannot hide behind the first (npm's `JSON.parse` keeps the
/// last), and every escape decoded as `JSON.parse` decodes it, so an escaped key still names `scripts`.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Reads a whole JSON text, refusing anything after its value.
pub fn parse(text: &str) -> Result<Json, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    let value = json_value(&chars, &mut at)?;
    json_space(&chars, &mut at);
    if at != chars.len() {
        return Err(format!("text after the value at {at}"));
    }
    Ok(value)
}

fn json_space(chars: &[char], at: &mut usize) {
    while chars
        .get(*at)
        .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
    {
        *at += 1;
    }
}

fn json_value(chars: &[char], at: &mut usize) -> Result<Json, String> {
    json_space(chars, at);
    match chars.get(*at) {
        Some('{') => {
            *at += 1;
            let mut entries = Vec::new();
            json_space(chars, at);
            if chars.get(*at) == Some(&'}') {
                *at += 1;
                return Ok(Json::Object(entries));
            }
            loop {
                json_space(chars, at);
                let key = json_string(chars, at)?;
                json_space(chars, at);
                if chars.get(*at) != Some(&':') {
                    return Err(format!("no `:` after a key at {at}"));
                }
                *at += 1;
                entries.push((key, json_value(chars, at)?));
                json_space(chars, at);
                match chars.get(*at) {
                    Some(',') => *at += 1,
                    Some('}') => {
                        *at += 1;
                        return Ok(Json::Object(entries));
                    }
                    _ => return Err(format!("no `,` or `}}` at {at}")),
                }
            }
        }
        Some('[') => {
            *at += 1;
            let mut items = Vec::new();
            json_space(chars, at);
            if chars.get(*at) == Some(&']') {
                *at += 1;
                return Ok(Json::Array(items));
            }
            loop {
                items.push(json_value(chars, at)?);
                json_space(chars, at);
                match chars.get(*at) {
                    Some(',') => *at += 1,
                    Some(']') => {
                        *at += 1;
                        return Ok(Json::Array(items));
                    }
                    _ => return Err(format!("no `,` or `]` at {at}")),
                }
            }
        }
        Some('"') => Ok(Json::Str(json_string(chars, at)?)),
        Some(_) => {
            let start = *at;
            while chars
                .get(*at)
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.'))
            {
                *at += 1;
            }
            let word: String = chars[start..*at].iter().collect();
            match word.as_str() {
                "null" => Ok(Json::Null),
                "true" => Ok(Json::Bool(true)),
                "false" => Ok(Json::Bool(false)),
                _ if word.parse::<f64>().is_ok() => Ok(Json::Number(word)),
                _ => Err(format!("not a JSON value at {start}: {word:?}")),
            }
        }
        None => Err("the text ends before a value".to_string()),
    }
}

fn json_string(chars: &[char], at: &mut usize) -> Result<String, String> {
    if chars.get(*at) != Some(&'"') {
        return Err(format!("no string at {at}"));
    }
    *at += 1;
    let mut out = String::new();
    let hex = |at: usize| -> Result<u32, String> {
        let digits: String = chars
            .get(at..at + 4)
            .ok_or("a short \\u escape")?
            .iter()
            .collect();
        u32::from_str_radix(&digits, 16).map_err(|_| format!("a bad \\u escape at {at}"))
    };
    loop {
        match chars.get(*at) {
            None => return Err("a string that does not end".to_string()),
            Some('"') => {
                *at += 1;
                return Ok(out);
            }
            Some('\\') => {
                let escaped = chars.get(*at + 1).ok_or("an escape at the end")?;
                *at += 2;
                match escaped {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let mut code = hex(*at)?;
                        *at += 4;
                        // A high surrogate and its low one make one character.
                        if (0xD800..0xDC00).contains(&code)
                            && chars.get(*at) == Some(&'\\')
                            && chars.get(*at + 1) == Some(&'u')
                        {
                            let low = hex(*at + 2)?;
                            if (0xDC00..0xE000).contains(&low) {
                                code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                *at += 6;
                            }
                        }
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    c => return Err(format!("an unknown escape \\{c}")),
                }
            }
            Some(c) => {
                out.push(*c);
                *at += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, Json};

    fn string(text: &str) -> String {
        match parse(text) {
            Ok(Json::Str(s)) => s,
            other => panic!("{text}: {other:?}"),
        }
    }

    /// A `\u` escape of a high surrogate joins the low one escaped right after it into one
    /// character, as `JSON.parse` does. Any other escape stands alone, even one a low surrogate
    /// follows, and so does a high surrogate that no `\u` escape follows at once; a lone surrogate
    /// is no character and reads as U+FFFD.
    #[test]
    fn test_only_a_high_surrogate_and_the_low_one_after_it_make_one_character() {
        assert_eq!(string(r#""\uD83D\uDE00""#), "\u{1F600}");
        assert_eq!(string(r#""\u0041\uDC00""#), "A\u{FFFD}");
        assert_eq!(string(r#""\uD800\u0041""#), "\u{FFFD}A");
        assert_eq!(string(r#""\uD800x\uDC00""#), "\u{FFFD}x\u{FFFD}");
        assert_eq!(string(r#""\u0041xuDC00""#), "AxuDC00");
    }
}
