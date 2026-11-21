//! 极简 JSON（无第三方依赖）：语言服务器只需要解析 JSON-RPC 报文。

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) => Some(*n as i64),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn s(v: &str) -> Value {
        Value::Str(v.to_string())
    }
    pub fn n(v: i64) -> Value {
        Value::Num(v as f64)
    }
}

pub fn parse(src: &str) -> Option<Value> {
    let b: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let v = parse_value(&b, &mut i)?;
    skip_ws(&b, &mut i);
    if i != b.len() {
        return None;
    }
    Some(v)
}

fn skip_ws(b: &[char], i: &mut usize) {
    while *i < b.len() && (b[*i] == ' ' || b[*i] == '\n' || b[*i] == '\r' || b[*i] == '\t') {
        *i += 1;
    }
}

fn parse_value(b: &[char], i: &mut usize) -> Option<Value> {
    skip_ws(b, i);
    if *i >= b.len() {
        return None;
    }
    match b[*i] {
        '{' => {
            *i += 1;
            let mut kv = Vec::new();
            skip_ws(b, i);
            if *i < b.len() && b[*i] == '}' {
                *i += 1;
                return Some(Value::Obj(kv));
            }
            loop {
                skip_ws(b, i);
                let k = match parse_value(b, i)? {
                    Value::Str(s) => s,
                    _ => return None,
                };
                skip_ws(b, i);
                if *i >= b.len() || b[*i] != ':' {
                    return None;
                }
                *i += 1;
                let v = parse_value(b, i)?;
                kv.push((k, v));
                skip_ws(b, i);
                if *i < b.len() && b[*i] == ',' {
                    *i += 1;
                    continue;
                }
                if *i < b.len() && b[*i] == '}' {
                    *i += 1;
                    return Some(Value::Obj(kv));
                }
                return None;
            }
        }
        '[' => {
            *i += 1;
            let mut items = Vec::new();
            skip_ws(b, i);
            if *i < b.len() && b[*i] == ']' {
                *i += 1;
                return Some(Value::Arr(items));
            }
            loop {
                let v = parse_value(b, i)?;
                items.push(v);
                skip_ws(b, i);
                if *i < b.len() && b[*i] == ',' {
                    *i += 1;
                    continue;
                }
                if *i < b.len() && b[*i] == ']' {
                    *i += 1;
                    return Some(Value::Arr(items));
                }
                return None;
            }
        }
        '"' => parse_string(b, i).map(Value::Str),
        't' => {
            if b.len() >= *i + 4 && b[*i..*i + 4] == ['t', 'r', 'u', 'e'] {
                *i += 4;
                Some(Value::Bool(true))
            } else {
                None
            }
        }
        'f' => {
            if b.len() >= *i + 5 && b[*i..*i + 5] == ['f', 'a', 'l', 's', 'e'] {
                *i += 5;
                Some(Value::Bool(false))
            } else {
                None
            }
        }
        'n' => {
            if b.len() >= *i + 4 && b[*i..*i + 4] == ['n', 'u', 'l', 'l'] {
                *i += 4;
                Some(Value::Null)
            } else {
                None
            }
        }
        _ => {
            let start = *i;
            if b[*i] == '-' {
                *i += 1;
            }
            while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == '.' || b[*i] == 'e' || b[*i] == 'E' || b[*i] == '+' || b[*i] == '-') {
                *i += 1;
            }
            let s: String = b[start..*i].iter().collect();
            s.parse::<f64>().ok().map(Value::Num)
        }
    }
}

fn parse_string(b: &[char], i: &mut usize) -> Option<String> {
    if *i >= b.len() || b[*i] != '"' {
        return None;
    }
    *i += 1;
    let mut s = String::new();
    while *i < b.len() {
        let c = b[*i];
        if c == '"' {
            *i += 1;
            return Some(s);
        }
        if c == '\\' {
            *i += 1;
            if *i >= b.len() {
                return None;
            }
            match b[*i] {
                'n' => s.push('\n'),
                't' => s.push('\t'),
                'r' => s.push('\r'),
                'b' => s.push('\u{8}'),
                'f' => s.push('\u{c}'),
                '"' => s.push('"'),
                '\\' => s.push('\\'),
                '/' => s.push('/'),
                'u' => {
                    let mut code = 0u32;
                    for _ in 0..4 {
                        *i += 1;
                        if *i >= b.len() {
                            return None;
                        }
                        code = code * 16 + b[*i].to_digit(16)?;
                    }
                    s.push(std::char::from_u32(code)?);
                }
                _ => return None,
            }
            *i += 1;
            continue;
        }
        s.push(c);
        *i += 1;
    }
    None
}

pub fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn write(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&format!("{}", *n as i64));
            } else {
                out.push_str(&format!("{}", n));
            }
        }
        Value::Str(s) => escape(s, out),
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(x, out);
            }
            out.push(']');
        }
        Value::Obj(kv) => {
            out.push('{');
            for (i, (k, x)) in kv.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                escape(k, out);
                out.push(':');
                write(x, out);
            }
            out.push('}');
        }
    }
}
