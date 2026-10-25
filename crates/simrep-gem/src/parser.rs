use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

#[derive(Clone, Debug)]
pub(crate) enum Value {
    Number(f64, bool),
    Text(String),
    Default,
    Callback(crate::lua::Callback),
}

impl Value {
    pub fn number(&self) -> Result<f64> {
        match self {
            Self::Number(n, _) => Ok(*n),
            Self::Text(s) => Err(Error(format!("expected a number, got '{s}'"))),
            Self::Default => Err(Error("missing numeric parameter".into())),
            Self::Callback(_) => Err(Error("expected a number, got a Lua callback".into())),
        }
    }
    pub fn text(&self) -> Result<&str> {
        match self {
            Self::Text(s) => Ok(s),
            _ => Err(Error("expected a string parameter".into())),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub name: String,
    pub args: Vec<Value>,
    pub named: BTreeMap<String, Value>,
    pub children: Vec<Node>,
    pub line: usize,
    pub table: bool,
}

impl Node {
    pub fn num(&self, i: usize, default: f64) -> Result<f64> {
        match self.args.get(i) {
            None | Some(Value::Default) => Ok(default),
            Some(v) => v.number(),
        }
    }
    pub fn string<'a>(&'a self, i: usize, default: &'a str) -> Result<&'a str> {
        match self.args.get(i) {
            None | Some(Value::Default) => Ok(default),
            Some(v) => v.text(),
        }
    }
    pub fn unsupported(&self) -> Error {
        Error(format!(
            "line {}: unsupported GEM keyword or feature '{}'",
            self.line, self.name
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Word(String),
    String(String),
    Number(f64),
    Symbol(char),
    End,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    line: usize,
}

fn lex(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    let mut line = 1;
    while let Some(c) = chars.next() {
        match c {
            '\n' => line += 1,
            c if c.is_whitespace() => {}
            ';' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        line += 1;
                        break;
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                if chars.peek() == Some(&'[') {
                    chars.next();
                    if chars.peek() != Some(&'[') {
                        return Err(Error(format!("line {line}: unsupported Lua long comment")));
                    }
                    chars.next();
                    let mut prev = '\0';
                    let mut closed = false;
                    for c in chars.by_ref() {
                        if c == '\n' {
                            line += 1;
                        }
                        if prev == ']' && c == ']' {
                            closed = true;
                            break;
                        }
                        prev = c;
                    }
                    if !closed {
                        return Err(Error(format!("line {line}: unterminated comment")));
                    }
                } else {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            line += 1;
                            break;
                        }
                    }
                }
            }
            '\'' | '"' => {
                let mut s = String::new();
                let mut closed = false;
                while let Some(ch) = chars.next() {
                    if ch == c {
                        closed = true;
                        break;
                    }
                    if ch == '\\' {
                        let next = chars
                            .next()
                            .ok_or_else(|| Error("unterminated string".into()))?;
                        if next != '\\' && next != '\'' && next != '"' {
                            s.push('\\');
                        }
                        s.push(next);
                    } else {
                        if ch == '\n' {
                            line += 1;
                        }
                        s.push(ch);
                    }
                }
                if !closed {
                    return Err(Error(format!("line {line}: unterminated string")));
                }
                tokens.push(Token {
                    kind: Kind::String(s),
                    line,
                });
            }
            c if c.is_ascii_digit()
                || c == '.' && chars.peek().is_some_and(char::is_ascii_digit) =>
            {
                let mut n = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() || next == '.' {
                        n.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                if chars.peek().is_some_and(|c| *c == 'e' || *c == 'E') {
                    n.push(chars.next().unwrap());
                    if chars.peek().is_some_and(|c| *c == '+' || *c == '-') {
                        n.push(chars.next().unwrap());
                    }
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        n.push(chars.next().unwrap());
                    }
                }
                let n = n
                    .parse()
                    .map_err(|_| Error(format!("line {line}: invalid number '{n}'")))?;
                tokens.push(Token {
                    kind: Kind::Number(n),
                    line,
                });
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut word = String::from(c);
                while chars
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                {
                    word.push(chars.next().unwrap());
                }
                // Traditional unquoted "non-mirrored" is a single string.
                if word.eq_ignore_ascii_case("non") && chars.peek() == Some(&'-') {
                    word.push(chars.next().unwrap());
                    while chars
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                    {
                        word.push(chars.next().unwrap());
                    }
                }
                tokens.push(Token {
                    kind: Kind::Word(word),
                    line,
                });
            }
            c if "(){},=+-*/%^#$".contains(c) => tokens.push(Token {
                kind: Kind::Symbol(c),
                line,
            }),
            _ => return Err(Error(format!("line {line}: unsupported character '{c}'"))),
        }
    }
    tokens.push(Token {
        kind: Kind::End,
        line,
    });
    Ok(tokens)
}

pub(crate) struct Parser {
    variables: BTreeMap<String, Value>,
    stack: Vec<PathBuf>,
}

impl Parser {
    pub fn new() -> Self {
        Self::including(Vec::new())
    }

    pub(crate) fn including(stack: Vec<PathBuf>) -> Self {
        Self {
            variables: BTreeMap::new(),
            stack,
        }
    }

    pub fn file(&mut self, path: &Path) -> Result<Vec<Node>> {
        let path = path
            .canonicalize()
            .map_err(|e| Error(format!("{}: {e}", path.display())))?;
        if self.stack.len() >= 15 || self.stack.contains(&path) {
            return Err(Error(format!(
                "recursive include or include nesting exceeds 15: {}",
                path.display()
            )));
        }
        let text = std::fs::read(&path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        // Legacy files use Windows-1252 in comments; geometry tokens are ASCII.
        let text = String::from_utf8_lossy(&text);
        self.stack.push(path.clone());
        let result = self
            .source(&text, path.parent().unwrap())
            .map_err(|e| Error(format!("{}: {e}", path.display())));
        self.stack.pop();
        result
    }

    pub fn source(&mut self, text: &str, directory: &Path) -> Result<Vec<Node>> {
        let lua = crate::lua::Runtime::new()?;
        if crate::lua::modern(text) {
            lua.including(self.stack.clone());
            lua.source(text, directory)
        } else {
            self.expanded_source(&lua.preprocess(text)?, directory)
        }
    }

    pub(crate) fn expanded_source(&mut self, text: &str, directory: &Path) -> Result<Vec<Node>> {
        let tokens = lex(text)?;
        let mut cursor = Cursor {
            tokens,
            pos: 0,
            parser: self,
            directory,
        };
        cursor.nodes(false)
    }
}

struct Cursor<'a> {
    tokens: Vec<Token>,
    pos: usize,
    parser: &'a mut Parser,
    directory: &'a Path,
}

impl Cursor<'_> {
    fn peek(&self) -> &Kind {
        &self.tokens[self.pos].kind
    }
    fn symbol(&mut self, c: char) -> bool {
        if self.peek() == &Kind::Symbol(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: char) -> Result<()> {
        if self.symbol(c) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{c}'")))
        }
    }
    fn error(&self, msg: impl AsRef<str>) -> Error {
        Error(format!(
            "line {}: {}",
            self.tokens[self.pos].line,
            msg.as_ref()
        ))
    }
    fn word(&mut self) -> Result<String> {
        if let Kind::Word(s) = self.peek().clone() {
            self.pos += 1;
            Ok(s)
        } else {
            Err(self.error("expected GEM instruction or variable"))
        }
    }

    fn nodes(&mut self, nested: bool) -> Result<Vec<Node>> {
        let mut out = Vec::new();
        loop {
            if self.symbol('}') {
                return if nested {
                    Ok(out)
                } else {
                    Err(self.error("unexpected '}'"))
                };
            }
            if self.peek() == &Kind::End {
                return if nested {
                    Err(self.error("unclosed block"))
                } else {
                    Ok(out)
                };
            }
            let macro_line = self.symbol('#');
            // Empty preprocessor lines (including "# -- comment") are legal.
            if macro_line
                && (self.peek() == &Kind::End
                    || self.tokens[self.pos].line > self.tokens[self.pos - 1].line)
            {
                continue;
            }
            let line = self.tokens[self.pos].line;
            let mut name = self.word()?;
            if name == "local" {
                name = self.word()?;
            }
            if matches!(
                name.as_str(),
                "function" | "for" | "if" | "while" | "repeat" | "return" | "end"
            ) {
                return Err(self.error(format!("unsupported Lua feature '{name}'; GEM supports numeric expressions, not Lua execution")));
            }
            if self.symbol('=') {
                let value = self.expression(0)?;
                self.parser.variables.insert(name, value);
                continue;
            }
            if macro_line {
                return Err(self.error(format!("unsupported Lua macro '{name}'")));
            }
            let name = name.to_ascii_lowercase();
            let mut node = Node {
                name,
                args: Vec::new(),
                named: BTreeMap::new(),
                children: Vec::new(),
                line,
                table: false,
            };
            if node.name == "pa_define" && self.symbol('{') {
                node.table = true;
                self.arguments(&mut node, '}')?;
            } else {
                if self.symbol('(') {
                    self.arguments(&mut node, ')')?;
                }
                if self.symbol('{') {
                    node.children = self.nodes(true)?;
                }
            }
            if matches!(node.name.as_str(), "include" | "include_file") {
                if node.args.len() != 1 {
                    return Err(self.error(
                        "include requires one filename; Lua include arguments are unsupported",
                    ));
                }
                let mut path = PathBuf::from(node.args[0].text()?.replace('\\', "/"));
                if path.extension().is_none() {
                    path.set_extension("gem");
                }
                out.extend(self.parser.file(&self.directory.join(path))?);
            } else {
                if node.name == "stl" {
                    let filename = node
                        .args
                        .first()
                        .ok_or_else(|| self.error("stl requires a filename"))?
                        .text()?;
                    let path = self.directory.join(filename.replace('\\', "/"));
                    node.args[0] = Value::Text(path.to_string_lossy().into_owned());
                }
                if node.name != "pa_define" && !node.named.is_empty() {
                    return Err(
                        self.error(format!("unsupported named parameters on '{}'", node.name))
                    );
                }
                out.push(node);
            }
        }
    }

    fn arguments(&mut self, node: &mut Node, end: char) -> Result<()> {
        let mut after_value = false;
        loop {
            if self.symbol(end) {
                break;
            }
            if self.peek() == &Kind::End {
                return Err(self.error("unclosed argument list"));
            }
            if self.symbol(',') {
                if !after_value {
                    node.args.push(Value::Default);
                }
                after_value = false;
                continue;
            }
            let named = if let Kind::Word(key) = self.peek() {
                if self
                    .tokens
                    .get(self.pos + 1)
                    .is_some_and(|t| t.kind == Kind::Symbol('='))
                {
                    Some(key.to_ascii_lowercase())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(key) = named {
                self.pos += 2;
                let value = self.expression(0)?;
                if node.named.insert(key.clone(), value).is_some() {
                    return Err(self.error(format!("duplicate named parameter '{key}'")));
                }
            } else {
                node.args.push(self.expression(0)?);
            }
            after_value = true;
        }
        Ok(())
    }

    fn expression(&mut self, min: u8) -> Result<Value> {
        let mut lhs = if self.symbol('$') {
            self.expect('(')?;
            let value = self.expression(0)?;
            self.expect(')')?;
            value
        } else if self.symbol('(') {
            let value = self.expression(0)?;
            self.expect(')')?;
            value
        } else if self.symbol('-') {
            let v = self.expression(5)?;
            Value::Number(-v.number()?, matches!(v, Value::Number(_, true)))
        } else if self.symbol('+') {
            self.expression(5)?
        } else {
            let token = self.peek().clone();
            self.pos += 1;
            match token {
                Kind::Number(n) => Value::Number(n, false),
                Kind::String(s) => Value::Text(s),
                Kind::Word(s) => {
                    if s == "function" {
                        return Err(self.error(
                            "unsupported Lua function/callback; use analytic GEM primitives",
                        ));
                    }
                    if self.symbol('(') {
                        let mut values = Vec::new();
                        if !self.symbol(')') {
                            loop {
                                values.push(self.expression(0)?.number()?);
                                if self.symbol(')') {
                                    break;
                                }
                                self.expect(',')?;
                            }
                        }
                        Value::Number(function(&s, &values).map_err(|e| self.error(e.0))?, false)
                    } else if let Some(v) = self.parser.variables.get(&s) {
                        v.clone()
                    } else {
                        match s.as_str() {
                            "pi" | "math.pi" => Value::Number(std::f64::consts::PI, false),
                            "inf" | "math.huge" => Value::Number(f64::INFINITY, false),
                            "mm" => Value::Number(1.0, true),
                            _ => Value::Text(s),
                        }
                    }
                }
                _ => return Err(self.error("expected a numeric expression or string")),
            }
        };
        while let Kind::Symbol(c @ ('+' | '-' | '*' | '/' | '%' | '^')) = self.peek() {
            let op = *c;
            let (left, right) = match op {
                '+' | '-' => (1, 2),
                '*' | '/' | '%' => (3, 4),
                '^' => (6, 5),
                _ => unreachable!(),
            };
            if left < min {
                break;
            }
            self.pos += 1;
            let rhs = self.expression(right)?;
            let a = lhs.number()?;
            let b = rhs.number()?;
            let unit =
                matches!(lhs, Value::Number(_, true)) || matches!(rhs, Value::Number(_, true));
            let value = match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => a / b,
                '%' => a.rem_euclid(b),
                '^' => a.powf(b),
                _ => unreachable!(),
            };
            if value.is_nan() {
                return Err(self.error("numeric expression produces NaN"));
            }
            lhs = Value::Number(value, unit);
        }
        Ok(lhs)
    }
}

fn function(name: &str, args: &[f64]) -> Result<f64> {
    let name = name.strip_prefix("math.").unwrap_or(name);
    match (name, args) {
        ("sin", [x]) => Ok(x.sin()),
        ("cos", [x]) => Ok(x.cos()),
        ("tan", [x]) => Ok(x.tan()),
        ("asin", [x]) => Ok(x.asin()),
        ("acos", [x]) => Ok(x.acos()),
        ("atan", [x]) => Ok(x.atan()),
        ("atan2", [y, x]) => Ok(y.atan2(*x)),
        ("sqrt", [x]) => Ok(x.sqrt()),
        ("abs", [x]) => Ok(x.abs()),
        ("floor", [x]) => Ok(x.floor()),
        ("ceil", [x]) => Ok(x.ceil()),
        ("exp", [x]) => Ok(x.exp()),
        ("log", [x]) => Ok(x.ln()),
        ("rad", [x]) => Ok(x.to_radians()),
        ("deg", [x]) => Ok(x.to_degrees()),
        ("min", [_, ..]) => Ok(args.iter().copied().fold(f64::INFINITY, f64::min)),
        ("max", [_, ..]) => Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        _ => Err(Error(format!(
            "unsupported numeric function or argument count '{name}'"
        ))),
    }
}
