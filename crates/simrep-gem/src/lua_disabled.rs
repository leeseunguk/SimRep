//! Literal GEM support without an executable Lua runtime.
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct Callback(std::convert::Infallible);

impl Callback {
    pub(crate) fn contains(&self, _p: [f64; 3]) -> bool {
        match self.0 {}
    }
    pub(crate) fn check(&self) -> Result<()> {
        match self.0 {}
    }
    pub(crate) fn potential(&self, _p: [f64; 3]) -> f64 {
        match self.0 {}
    }
}

pub(crate) fn preprocess(text: &str) -> Result<String> {
    // Do not interpret words in strings or comments as executable syntax.
    let mut chars = text.chars().peekable();
    let mut plain = String::new();
    while let Some(c) = chars.next() {
        if c == '\'' || c == '"' {
            let quote = c;
            while let Some(c) = chars.next() {
                if c == '\\' {
                    chars.next();
                } else if c == quote {
                    break;
                }
            }
            plain.push(' ');
        } else if c == ';' || c == '-' && chars.peek() == Some(&'-') {
            let mut long = false;
            if c == '-' {
                chars.next();
                if chars.peek() == Some(&'[') {
                    chars.next();
                    if chars.peek() == Some(&'[') {
                        chars.next();
                        long = true;
                    }
                }
            }
            if long {
                let mut previous = '\0';
                for c in chars.by_ref() {
                    if previous == ']' && c == ']' {
                        break;
                    }
                    previous = c;
                }
            } else {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            plain.push('\n');
        } else {
            plain.push(c);
        }
    }
    if plain.contains('#')
        || plain.contains("$(")
        || plain
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
            .any(|s| {
                matches!(
                    s,
                    "local"
                        | "function"
                        | "for"
                        | "while"
                        | "repeat"
                        | "if"
                        | "return"
                        | "end"
                        | "print"
                        | "require"
                        | "dofile"
                        | "loadfile"
                        | "load"
                        | "setmetatable"
                        | "collectgarbage"
                ) || s.starts_with("os.")
                    || s.starts_with("io.")
            })
    {
        return Err(Error("unsupported in browser: GEM Lua".into()));
    }
    Ok(text.to_owned())
}
