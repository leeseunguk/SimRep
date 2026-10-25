//! Lua 5.1 GEM execution. Only geometry, pure libraries and virtual text IO
//! are exposed; Lua never receives filesystem, process or module access.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mlua::{Function, Lua, MultiValue, Value as LuaValue};

use crate::parser::{Node, Parser, Value};
use crate::{Error, Result};

fn error(e: mlua::Error) -> Error {
    Error(format!("GEM Lua: {e}"))
}

#[derive(Clone, Debug)]
pub struct Callback {
    // Function handles in mlua hold only a weak reference to their interpreter.
    _lua: Lua,
    function: Function,
    error: Arc<Mutex<Option<String>>>,
}

impl Callback {
    pub(crate) fn call(&self, p: [f64; 3]) -> LuaValue {
        match self.function.call((p[0], p[1], p[2])) {
            Ok(v) => v,
            Err(e) => {
                *self.error.lock().unwrap() = Some(e.to_string());
                LuaValue::Nil
            }
        }
    }

    pub(crate) fn check(&self) -> Result<()> {
        if let Some(e) = self.error.lock().unwrap().as_ref() {
            return Err(Error(format!("GEM Lua callback: {e}")));
        }
        Ok(())
    }

    pub(crate) fn potential(&self, p: [f64; 3]) -> f64 {
        match self.call(p) {
            LuaValue::Number(v) => v,
            LuaValue::Integer(v) => v as f64,
            _ => {
                self.error
                    .lock()
                    .unwrap()
                    .get_or_insert_with(|| "potential callback must return a number".into());
                0.
            }
        }
    }
}

#[derive(Default)]
struct State {
    nodes: Vec<Vec<Node>>,
    directories: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

pub(crate) struct Runtime {
    lua: Lua,
    state: Arc<Mutex<State>>,
    callback_error: Arc<Mutex<Option<String>>>,
}

const COMMANDS: &str = "pa_define electrode e p elect pole electrode_points pole_points \
non_electrode n non_e non_p non_pole non_electrode_points non_pole_points \
fill fill_volume edge_fill edge_fill_volume rotate_fill rotate_fill_volume \
rotate_edge_fill rotate_edge_fill_volume within intersect notin within_inside \
notin_inside within_inside_or_on notin_inside_or_on locate project project_it transform \
scale rotate_x rotate_y rotate_z revolve_xy revolve_yz revolve_zx extrude_xy extrude_yz \
extrude_zx box box2d box3d centered_box centered_box2d centered_box3d cent_box \
cent_box2d cent_box3d corner_box corner_box2d corner_box3d corn_box corn_box2d \
corn_box3d circle ellipse sphere ellipsoid cylinder cylinder3d polyline points \
points2d points3d hyperbola parabola half_space stl shape";

impl Runtime {
    pub(crate) fn new() -> Result<Self> {
        let lua = simrep_lua::sandbox("GEM").map_err(error)?;
        lua.load(include_str!("sandbox.lua"))
            .exec()
            .map_err(error)?;
        for name in [
            "planar",
            "cylindrical",
            "electrostatic",
            "electric",
            "magnetic",
            "electric_",
            "electrostatic_",
            "magnetic_",
            "x",
            "y",
            "z",
            "xy",
            "xz",
            "yz",
            "xyz",
            "gu",
            "none",
            "auto",
            "fractional",
        ] {
            lua.globals().set(name, name).map_err(error)?;
        }
        let state = Arc::new(Mutex::new(State::default()));
        let callback_error = Arc::new(Mutex::new(None));
        for name in COMMANDS.split_whitespace() {
            let state = state.clone();
            let callback_error = callback_error.clone();
            lua.globals()
                .set(
                    name,
                    lua.create_function(move |lua, args: MultiValue| {
                        let table_call =
                            args.len() == 1 && matches!(args.front(), Some(LuaValue::Table(_)));
                        let mut node = Node {
                            name: name.into(),
                            args: Vec::new(),
                            named: BTreeMap::new(),
                            children: Vec::new(),
                            line: 1,
                            table: table_call && name == "pa_define",
                        };
                        let values: Vec<LuaValue> = if table_call {
                            let LuaValue::Table(table) = &args[0] else {
                                unreachable!()
                            };
                            let n = table.get::<Option<usize>>("n")?.unwrap_or(table.raw_len());
                            for pair in table.clone().pairs::<LuaValue, LuaValue>() {
                                if let (LuaValue::String(key), value) = pair? {
                                    let key = key.to_str()?.to_string();
                                    if key != "n" {
                                        node.named.insert(
                                            key,
                                            value_from_lua(lua, value, &callback_error)?,
                                        );
                                    }
                                }
                            }
                            (1..=n).map(|i| table.get(i)).collect::<mlua::Result<_>>()?
                        } else {
                            args.into_iter().collect()
                        };
                        for v in values {
                            if name == "pa_define"
                                && let LuaValue::Function(f) = &v
                            {
                                let alias = ["p", "n", "e"].into_iter().find(|alias| {
                                    lua.globals()
                                        .get::<Function>(*alias)
                                        .is_ok_and(|g| g.to_pointer() == f.to_pointer())
                                });
                                if let Some(alias) = alias {
                                    node.args.push(Value::Text(alias.into()));
                                    continue;
                                }
                            }
                            node.args.push(value_from_lua(lua, v, &callback_error)?);
                        }
                        if name == "stl"
                            && let Some(Value::Text(filename)) = node.args.first_mut()
                        {
                            let directory = state
                                .lock()
                                .unwrap()
                                .directories
                                .last()
                                .cloned()
                                .unwrap_or_default();
                            *filename = directory
                                .join(filename.replace('\\', "/"))
                                .to_string_lossy()
                                .into_owned();
                        }
                        let structural = crate::electrode(name).is_some()
                            || crate::is_fill(name)
                            || crate::within(name).is_some()
                            || matches!(
                                name,
                                "locate"
                                    | "project"
                                    | "project_it"
                                    | "transform"
                                    | "scale"
                                    | "rotate_x"
                                    | "rotate_y"
                                    | "rotate_z"
                                    | "revolve_xy"
                                    | "revolve_yz"
                                    | "revolve_zx"
                                    | "extrude_xy"
                                    | "extrude_yz"
                                    | "extrude_zx"
                            );
                        if structural {
                            let state = state.clone();
                            return lua
                                .create_function(move |_, body: Function| {
                                    state.lock().unwrap().nodes.push(Vec::new());
                                    let result = body.call::<()>(());
                                    let children = state.lock().unwrap().nodes.pop().unwrap();
                                    result?;
                                    let mut node = node.clone();
                                    node.children = children;
                                    state.lock().unwrap().nodes.last_mut().unwrap().push(node);
                                    Ok(())
                                })
                                .map(LuaValue::Function);
                        }
                        state.lock().unwrap().nodes.last_mut().unwrap().push(node);
                        Ok(LuaValue::Nil)
                    })
                    .map_err(error)?,
                )
                .map_err(error)?;
        }
        let sink = state.clone();
        lua.globals()
            .set(
                "_put",
                lua.create_function(move |_, text: String| {
                    let directory = sink
                        .lock()
                        .unwrap()
                        .directories
                        .last()
                        .cloned()
                        .unwrap_or_default();
                    let nodes = Parser::new()
                        .expanded_source(&text, &directory)
                        .map_err(mlua::Error::external)?;
                    sink.lock().unwrap().nodes.last_mut().unwrap().extend(nodes);
                    Ok(())
                })
                .map_err(error)?,
            )
            .map_err(error)?;
        let includes = state.clone();
        let errors = callback_error.clone();
        lua.globals()
            .set(
                "include",
                lua.create_function(move |lua, filename: String| {
                    let runtime = Runtime {
                        lua: lua.clone(),
                        state: includes.clone(),
                        callback_error: errors.clone(),
                    };
                    let directory = includes
                        .lock()
                        .unwrap()
                        .directories
                        .last()
                        .cloned()
                        .unwrap_or_default();
                    let mut path = directory.join(filename.replace('\\', "/"));
                    if path.extension().is_none() {
                        path.set_extension("gem");
                    }
                    let path = path.canonicalize().map_err(mlua::Error::external)?;
                    {
                        let mut state = includes.lock().unwrap();
                        if state.files.len() >= 15 || state.files.contains(&path) {
                            return Err(mlua::Error::RuntimeError(
                                "recursive include or include nesting exceeds 15".into(),
                            ));
                        }
                        state.files.push(path.clone());
                    }
                    let result = std::fs::read(&path)
                        .map_err(|e| Error(format!("{}: {e}", path.display())))
                        .and_then(|source| {
                            runtime
                                .source(&String::from_utf8_lossy(&source), path.parent().unwrap())
                        });
                    includes.lock().unwrap().files.pop();
                    let nodes = result.map_err(mlua::Error::external)?;
                    includes
                        .lock()
                        .unwrap()
                        .nodes
                        .last_mut()
                        .unwrap()
                        .extend(nodes);
                    Ok(())
                })
                .map_err(error)?,
            )
            .map_err(error)?;
        Ok(Self {
            lua,
            state,
            callback_error,
        })
    }

    pub(crate) fn source(&self, text: &str, directory: &Path) -> Result<Vec<Node>> {
        self.state
            .lock()
            .unwrap()
            .directories
            .push(directory.into());
        let result = self.execute(text);
        self.state.lock().unwrap().directories.pop();
        result
    }

    pub(crate) fn including(&self, files: Vec<PathBuf>) {
        self.state.lock().unwrap().files = files;
    }

    fn execute(&self, text: &str) -> Result<Vec<Node>> {
        self.state.lock().unwrap().nodes.push(Vec::new());
        let modern = modern(text);
        let result = (|| {
            if modern {
                let program = translate(text)?;
                self.lua
                    .load(&program)
                    .set_name("GEM")
                    .exec()
                    .map_err(error)
            } else {
                let expanded = self.preprocess(text)?;
                let directory = self
                    .state
                    .lock()
                    .unwrap()
                    .directories
                    .last()
                    .cloned()
                    .unwrap_or_default();
                let files = self.state.lock().unwrap().files.clone();
                Parser::including(files)
                    .expanded_source(&expanded, &directory)
                    .map(|nodes| {
                        self.state
                            .lock()
                            .unwrap()
                            .nodes
                            .last_mut()
                            .unwrap()
                            .extend(nodes);
                    })
            }
        })();
        let nodes = self.state.lock().unwrap().nodes.pop().unwrap();
        result?;
        if let Some(e) = self.callback_error.lock().unwrap().as_ref() {
            return Err(Error(format!("GEM Lua: {e}")));
        }
        Ok(nodes)
    }

    pub(crate) fn preprocess(&self, text: &str) -> Result<String> {
        let output = Arc::new(Mutex::new(String::new()));
        let sink = output.clone();
        let put = self
            .lua
            .create_function(move |lua, values: MultiValue| {
                for v in values {
                    if !matches!(v, LuaValue::Nil) {
                        let s: mlua::String = lua.globals().get::<Function>("tostring")?.call(v)?;
                        sink.lock().unwrap().push_str(&s.to_string_lossy());
                    }
                }
                Ok(())
            })
            .map_err(error)?;
        self.lua.globals().set("_put", put).map_err(error)?;
        let sink = output.clone();
        self.lua
            .globals()
            .set(
                "_emit",
                self.lua
                    .create_function(move |lua, values: MultiValue| {
                        for value in values {
                            let text = match value {
                                LuaValue::Nil => continue,
                                LuaValue::Number(n) => n.to_string(),
                                LuaValue::Integer(n) => n.to_string(),
                                value => lua
                                    .globals()
                                    .get::<Function>("tostring")?
                                    .call::<mlua::String>(value)?
                                    .to_string_lossy(),
                            };
                            sink.lock().unwrap().push_str(&text);
                        }
                        Ok(())
                    })
                    .map_err(error)?,
            )
            .map_err(error)?;
        let mut program = String::new();
        let mut pos = 0;
        while pos < text.len() {
            let line_end = text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1);
            let line = &text[pos..line_end];
            if line.trim_start().starts_with('#') {
                program.push_str(line.trim_start().strip_prefix('#').unwrap());
                program.push('\n');
                pos = line_end;
                continue;
            }
            let end = text[pos..].find("$(").map_or(text.len(), |i| pos + i);
            // Don't evaluate substitutions written inside GEM comments.
            let mut literal_end = end;
            if let Some(comment) = line.find(';') {
                let comment = pos + comment;
                if comment < end {
                    literal_end = line_end;
                }
            }
            if literal_end > pos {
                // Emit only through the current line so '#' remains line-oriented.
                literal_end = literal_end.min(line_end);
                program.push_str(&format!("_put({})\n", quote(&text[pos..literal_end])));
                pos = literal_end;
                continue;
            }
            let close = matching(text, pos + 1, '(', ')')?;
            let code = &text[pos + 2..close];
            if self
                .lua
                .load(format!("return {code}"))
                .into_function()
                .is_ok()
            {
                program.push_str(&format!("_emit({code})\n"));
            } else {
                program.push_str(code);
                program.push('\n');
            }
            pos = close + 1;
        }
        self.lua
            .load(&program)
            .set_name("GEM preprocessing")
            .exec()
            .map_err(error)?;
        let expanded = output.lock().unwrap().clone();
        Ok(expanded)
    }
}

fn value_from_lua(
    lua: &Lua,
    value: LuaValue,
    errors: &Arc<Mutex<Option<String>>>,
) -> mlua::Result<Value> {
    Ok(match value {
        LuaValue::Nil => Value::Default,
        LuaValue::Integer(n) => Value::Number(n as f64, false),
        LuaValue::Number(n) => Value::Number(n, false),
        LuaValue::String(s) => Value::Text(s.to_str()?.to_string()),
        LuaValue::Boolean(b) => Value::Text(b.to_string()),
        LuaValue::Table(t) => Value::Number(t.get("__mm")?, true),
        LuaValue::Function(function) => Value::Callback(Callback {
            _lua: lua.clone(),
            function,
            error: errors.clone(),
        }),
        _ => return Err(mlua::Error::RuntimeError("invalid GEM parameter".into())),
    })
}

fn quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\r', "\\r")
            .replace('\n', "\\n")
    )
}

pub(crate) fn modern(text: &str) -> bool {
    let mut pos = 0;
    while pos < text.len() {
        if let Some(end) = quoted_end(text, pos) {
            pos = end;
            continue;
        }
        if text[pos..].starts_with("$(") {
            let Ok(end) = matching(text, pos + 1, '(', ')') else {
                return false;
            };
            pos = end + 1;
            continue;
        }
        let c = text.as_bytes()[pos] as char;
        if c == ';' || c == '#' && text[..pos].rsplit('\n').next().unwrap().trim().is_empty() {
            pos += text[pos..].find('\n').unwrap_or(text.len() - pos);
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = pos;
            pos += 1;
            while pos < text.len()
                && (text.as_bytes()[pos].is_ascii_alphanumeric() || text.as_bytes()[pos] == b'_')
            {
                pos += 1;
            }
            if matches!(
                &text[start..pos],
                "local"
                    | "function"
                    | "for"
                    | "if"
                    | "while"
                    | "repeat"
                    | "print"
                    | "_put"
                    | "assert"
            ) {
                return true;
            }
        } else {
            pos += 1;
        }
    }
    false
}

/// Find balanced delimiters, respecting Lua strings and comments.
fn matching(text: &str, start: usize, open: char, close: char) -> Result<usize> {
    let mut depth = 0;
    let mut pos = start;
    while pos < text.len() {
        if let Some(end) = quoted_end(text, pos) {
            pos = end;
            continue;
        }
        let c = text.as_bytes()[pos] as char;
        if c == open {
            depth += 1;
        }
        if c == close {
            depth -= 1;
            if depth == 0 {
                return Ok(pos);
            }
        }
        pos += 1;
    }
    Err(Error(format!("GEM Lua: unclosed '{open}'")))
}

fn quoted_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if text[start..].starts_with("--") {
        if let Some(end) = long_end(text, start + 2) {
            return Some(end);
        }
        return Some(text[start..].find('\n').map_or(text.len(), |i| start + i));
    }
    if let Some(end) = long_end(text, start) {
        return Some(end);
    }
    let quote = bytes[start];
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let mut pos = start + 1;
    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            pos += 2;
        } else if bytes[pos] == quote {
            return Some(pos + 1);
        } else {
            pos += 1;
        }
    }
    Some(text.len())
}

fn long_end(text: &str, start: usize) -> Option<usize> {
    if text.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let mut pos = start + 1;
    while text.as_bytes().get(pos) == Some(&b'=') {
        pos += 1;
    }
    if text.as_bytes().get(pos) != Some(&b'[') {
        return None;
    }
    let close = format!("]{}]", &text[start + 1..pos]);
    Some(
        text[pos + 1..]
            .find(&close)
            .map_or(text.len(), |i| pos + 1 + i + close.len()),
    )
}

/// GEM scopes are Lua calls whose final argument is a statement block.
/// Tables, closures, method calls and ordinary Lua expressions remain Lua.
fn translate(text: &str) -> Result<String> {
    let mut out = String::new();
    let mut pos = 0;
    while pos < text.len() {
        if let Some(end) = quoted_end(text, pos) {
            out.push_str(&text[pos..end]);
            pos = end;
            continue;
        }
        if text[pos..].starts_with("$(") {
            let end = matching(text, pos + 1, '(', ')')?;
            out.push_str(&translate(&text[pos + 2..end])?);
            pos = end + 1;
            continue;
        }
        let c = text.as_bytes()[pos] as char;
        if c == '#' && text[..pos].rsplit('\n').next().unwrap().trim().is_empty() {
            pos += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = pos;
            pos += 1;
            while pos < text.len()
                && (text.as_bytes()[pos].is_ascii_alphanumeric() || text.as_bytes()[pos] == b'_')
            {
                pos += 1;
            }
            let word = &text[start..pos];
            let gap = text[pos..]
                .find(|c: char| !c.is_whitespace())
                .map_or(text.len(), |i| pos + i);
            // Declarations and calls without a scope stay ordinary Lua.
            if text.as_bytes().get(gap) == Some(&b'(') {
                let end = matching(text, gap, '(', ')')?;
                let mut next = end + 1;
                loop {
                    next += text[next..]
                        .find(|c: char| !c.is_whitespace())
                        .unwrap_or(text.len() - next);
                    if text[next..].starts_with("--") {
                        next = quoted_end(text, next).unwrap();
                    } else {
                        break;
                    }
                }
                if text.as_bytes().get(next) == Some(&b'{') {
                    let close = matching(text, next, '{', '}')?;
                    out.push_str(&format!(
                        "{word}({})(function()\n{} end)\n",
                        translate(&text[gap + 1..end])?,
                        translate(&text[next + 1..close])?
                    ));
                    pos = close + 1;
                    continue;
                }
                // Statement-bearing parameter lists (e.g. polygon_regular).
                let params = &text[gap + 1..end];
                if params.trim_start().starts_with("for ") {
                    let mut body = String::new();
                    for line in params.lines() {
                        let line = line.trim().trim_end_matches(',');
                        if line.is_empty() || line.starts_with("--") {
                            continue;
                        }
                        if ["for ", "local ", "if ", "end", "else"]
                            .iter()
                            .any(|p| line.starts_with(p))
                        {
                            body.push_str(line);
                        } else {
                            body.push_str(&format!("_append({line})"));
                        }
                        body.push('\n');
                    }
                    out.push_str(&format!(
                        "{word}(_arguments(function(_append)\n{body} end))"
                    ));
                    pos = end + 1;
                    continue;
                }
            } else if text.as_bytes().get(gap) == Some(&b'{') && word != "pa_define" {
                let close = matching(text, gap, '{', '}')?;
                if crate::within(word).is_some() || crate::is_fill(word) {
                    out.push_str(&format!(
                        "{word}()(function()\n{} end)\n",
                        translate(&text[gap + 1..close])?
                    ));
                    pos = close + 1;
                    continue;
                }
            }
            out.push_str(word);
            continue;
        }
        out.push(c);
        pos += 1;
    }
    Ok(out)
}
