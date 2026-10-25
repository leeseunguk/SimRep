//! Small, non-executable voltage language. Time is absolute microseconds.
use std::collections::BTreeMap;

use crate::FlyError;

#[derive(Clone, Debug)]
enum Node {
    Constant(f64),
    Time,
    Unary(Box<Node>),
    Binary(u8, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}

/// Parsed and parameter-bound voltage. Unknown names are errors, not zeroes.
/// `step(t0)` is one for t >= t0; `pulse(t0,width)` is one on [t0,t0+width).
#[derive(Clone, Debug)]
pub struct VoltageExpression(Node);

impl VoltageExpression {
    pub fn parse(source: &str, parameters: &BTreeMap<String, f64>) -> Result<Self, FlyError> {
        if source.len() > 8192 || parameters.values().any(|v| !v.is_finite()) {
            return Err(FlyError("expression too long or parameter not finite"));
        }
        if parameters.keys().any(|name| {
            matches!(name.as_str(), "t" | "pi" | "e")
                || name.is_empty()
                || !name.as_bytes()[0].is_ascii_alphabetic() && name.as_bytes()[0] != b'_'
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }) {
            return Err(FlyError(
                "parameter names must be identifiers other than t, pi and e",
            ));
        }
        let mut parser = Parser {
            source: source.as_bytes(),
            at: 0,
            parameters,
            depth: 0,
        };
        let node = parser.expression(0)?;
        parser.space();
        if parser.at != parser.source.len() {
            return Err(FlyError("unexpected token in voltage expression"));
        }
        Ok(Self(node))
    }

    pub fn constant(value: f64) -> Result<Self, FlyError> {
        if !value.is_finite() {
            return Err(FlyError("voltage constant must be finite"));
        }
        Ok(Self(Node::Constant(value)))
    }

    pub fn evaluate(&self, time_us: f64) -> f64 {
        self.0.evaluate(time_us)
    }

    /// Inverse of the fastest affine sin/cos phase or exponential rate.
    /// Explicit step/pulse edges are handled separately by `next_event`.
    pub fn time_scale_us(&self) -> Option<f64> {
        self.0.time_scale_us()
    }

    /// Expression for use in a WGSL f32 function whose argument is `t`.
    pub fn to_wgsl(&self) -> Result<String, FlyError> {
        self.0.wgsl(false)
    }

    pub(crate) fn to_wgsl_f64(&self) -> Result<String, FlyError> {
        self.0.wgsl(true)
    }

    /// Explicit step/pulse edges strictly after this absolute time.
    pub fn next_event(&self, time_us: f64) -> Option<f64> {
        self.0.next_event(time_us)
    }

    pub(crate) fn events(&self) -> Vec<f64> {
        let mut edges = Vec::new();
        self.0.events(&mut edges);
        edges.sort_by(f64::total_cmp);
        edges.dedup();
        edges
    }
}

impl Node {
    fn affine(&self) -> Option<[f64; 2]> {
        if !self.time_dependent() {
            return Some([self.evaluate(0.0), 0.0]);
        }
        match self {
            Self::Time => Some([0.0, 1.0]),
            Self::Unary(n) => n.affine().map(|v| v.map(|x| -x)),
            Self::Binary(op, a, b) => {
                let [a, da] = a.affine()?;
                let [b, db] = b.affine()?;
                match op {
                    b'+' => Some([a + b, da + db]),
                    b'-' => Some([a - b, da - db]),
                    b'*' if da == 0.0 => Some([a * b, a * db]),
                    b'*' if db == 0.0 => Some([a * b, da * b]),
                    b'/' if db == 0.0 => Some([a / b, da / b]),
                    b'^' if b == 1.0 && db == 0.0 => Some([a, da]),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn time_scale_us(&self) -> Option<f64> {
        match self {
            Self::Unary(n) => n.time_scale_us(),
            Self::Binary(_, a, b) => a
                .time_scale_us()
                .into_iter()
                .chain(b.time_scale_us())
                .min_by(f64::total_cmp),
            Self::Call(name, args) => {
                let own = matches!(name.as_str(), "sin" | "cos" | "exp")
                    .then(|| args[0].affine())
                    .flatten()
                    .map(|v| v[1].abs())
                    .filter(|rate| rate.is_finite() && *rate > 0.0)
                    .map(|rate| 1.0 / rate);
                args.iter()
                    .filter_map(Self::time_scale_us)
                    .chain(own)
                    .min_by(f64::total_cmp)
            }
            _ => None,
        }
    }

    fn events(&self, edges: &mut Vec<f64>) {
        match self {
            Self::Unary(n) => n.events(edges),
            Self::Binary(_, a, b) => {
                a.events(edges);
                b.events(edges);
            }
            Self::Call(name, args) => {
                for a in args {
                    a.events(edges);
                }
                if (name == "step" || name == "pulse") && !args.iter().any(Self::time_dependent) {
                    let start = args[0].evaluate(0.0);
                    if start.is_finite() {
                        edges.push(start);
                    }
                    if name == "pulse" {
                        let end = start + args[1].evaluate(0.0);
                        if end.is_finite() {
                            edges.push(end);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn evaluate(&self, t: f64) -> f64 {
        match self {
            Self::Constant(v) => *v,
            Self::Time => t,
            Self::Unary(x) => -x.evaluate(t),
            Self::Binary(op, a, b) => {
                let (a, b) = (a.evaluate(t), b.evaluate(t));
                match op {
                    b'+' => a + b,
                    b'-' => a - b,
                    b'*' => a * b,
                    b'/' => a / b,
                    _ => a.powf(b),
                }
            }
            Self::Call(name, args) => {
                let x = args[0].evaluate(t);
                match name.as_str() {
                    "sin" => x.sin(),
                    "cos" => x.cos(),
                    "exp" => x.exp(),
                    "sqrt" => x.sqrt(),
                    "abs" => x.abs(),
                    "min" => x.min(args[1].evaluate(t)),
                    "max" => x.max(args[1].evaluate(t)),
                    "step" => f64::from(t >= x),
                    "pulse" => f64::from(t >= x && t < x + args[1].evaluate(t)),
                    _ => unreachable!(),
                }
            }
        }
    }

    fn wgsl(&self, wide: bool) -> Result<String, FlyError> {
        Ok(match self {
            Self::Constant(v) => {
                if !wide && !(*v as f32).is_finite() {
                    return Err(FlyError("expression constant exceeds GPU f32 range"));
                }
                if wide {
                    format!("{v:.17e}")
                } else {
                    format!("{:.9e}", *v as f32)
                }
            }
            Self::Time => "t".into(),
            Self::Unary(x) => format!("(-{})", x.wgsl(wide)?),
            Self::Binary(op, a, b) => {
                let (a, b) = (a.wgsl(wide)?, b.wgsl(wide)?);
                if *op == b'^' {
                    format!("voltage_pow({a},{b})")
                } else {
                    format!("({a}{}{b})", char::from(*op))
                }
            }
            Self::Call(name, args) => {
                let x = args[0].wgsl(wide)?;
                match name.as_str() {
                    "step" => format!("select(0.0,1.0,t >= {x})"),
                    "pulse" => format!(
                        "select(0.0,1.0,t >= {x} && t < ({x}+{}))",
                        args[1].wgsl(wide)?
                    ),
                    _ => format!(
                        "{name}({})",
                        args.iter()
                            .map(|a| a.wgsl(wide))
                            .collect::<Result<Vec<_>, _>>()?
                            .join(",")
                    ),
                }
            }
        })
    }

    fn time_dependent(&self) -> bool {
        match self {
            Self::Constant(_) => false,
            Self::Time => true,
            Self::Unary(n) => n.time_dependent(),
            Self::Binary(_, a, b) => a.time_dependent() || b.time_dependent(),
            Self::Call(name, args) => {
                name == "step" || name == "pulse" || args.iter().any(Self::time_dependent)
            }
        }
    }

    fn next_event(&self, t: f64) -> Option<f64> {
        let first = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match self {
            Self::Unary(n) => n.next_event(t),
            Self::Binary(_, a, b) => first(a.next_event(t), b.next_event(t)),
            Self::Call(name, args) => {
                let mut edge = args
                    .iter()
                    .filter_map(|a| a.next_event(t))
                    .min_by(f64::total_cmp);
                if (name == "step" || name == "pulse") && !args.iter().any(Self::time_dependent) {
                    let start = args[0].evaluate(0.0);
                    if start > t {
                        edge = first(edge, Some(start));
                    }
                    if name == "pulse" {
                        let end = start + args[1].evaluate(0.0);
                        if end > t {
                            edge = first(edge, Some(end));
                        }
                    }
                }
                edge
            }
            _ => None,
        }
    }
}

struct Parser<'a> {
    source: &'a [u8],
    at: usize,
    parameters: &'a BTreeMap<String, f64>,
    depth: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self
            .source
            .get(self.at)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.at += 1;
        }
    }

    fn take(&mut self, token: u8) -> bool {
        self.space();
        if self.source.get(self.at) == Some(&token) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expression(&mut self, minimum: u8) -> Result<Node, FlyError> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(FlyError("voltage expression nesting exceeds 64"));
        }
        self.space();
        let mut left = if self.take(b'-') {
            Node::Unary(Box::new(self.expression(5)?))
        } else if self.take(b'+') {
            self.expression(5)?
        } else if self.take(b'(') {
            let n = self.expression(0)?;
            if !self.take(b')') {
                return Err(FlyError("missing closing parenthesis"));
            }
            n
        } else {
            self.atom()?
        };
        loop {
            self.space();
            let Some(&op) = self.source.get(self.at) else {
                break;
            };
            let (l, r) = match op {
                b'+' | b'-' => (1, 2),
                b'*' | b'/' => (3, 4),
                b'^' => (7, 6),
                _ => break,
            };
            if l < minimum {
                break;
            }
            self.at += 1;
            left = Node::Binary(op, Box::new(left), Box::new(self.expression(r)?));
        }
        self.depth -= 1;
        Ok(left)
    }

    fn atom(&mut self) -> Result<Node, FlyError> {
        self.space();
        let start = self.at;
        if self
            .source
            .get(self.at)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
        {
            while self
                .source
                .get(self.at)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
            {
                self.at += 1;
            }
            if self
                .source
                .get(self.at)
                .is_some_and(|c| *c == b'e' || *c == b'E')
            {
                self.at += 1;
                if self
                    .source
                    .get(self.at)
                    .is_some_and(|c| *c == b'+' || *c == b'-')
                {
                    self.at += 1;
                }
                while self.source.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
            }
            let number = std::str::from_utf8(&self.source[start..self.at])
                .map_err(|_| FlyError("invalid numeric literal"))?;
            let value: f64 = number
                .parse()
                .map_err(|_| FlyError("invalid numeric literal"))?;
            if !value.is_finite() {
                return Err(FlyError("numeric literal must be finite"));
            }
            return Ok(Node::Constant(value));
        }
        while self
            .source
            .get(self.at)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            self.at += 1;
        }
        let name = std::str::from_utf8(&self.source[start..self.at])
            .map_err(|_| FlyError("invalid name"))?;
        if name.is_empty() {
            return Err(FlyError("expected number, name or function"));
        }
        if self.take(b'(') {
            let count = match name {
                "sin" | "cos" | "exp" | "sqrt" | "abs" | "step" => 1,
                "min" | "max" | "pulse" => 2,
                _ => return Err(FlyError("unknown voltage function")),
            };
            let mut args = vec![self.expression(0)?];
            if count == 2 {
                if !self.take(b',') {
                    return Err(FlyError("function needs two arguments"));
                }
                args.push(self.expression(0)?);
            }
            if !self.take(b')') {
                return Err(FlyError("wrong function arity or missing parenthesis"));
            }
            if (name == "step" || name == "pulse")
                && (args.iter().any(Node::time_dependent)
                    || args.iter().any(|a| !a.evaluate(0.0).is_finite())
                    || name == "pulse"
                        && (args[1].evaluate(0.0) < 0.0
                            || !(args[0].evaluate(0.0) + args[1].evaluate(0.0)).is_finite()))
            {
                return Err(FlyError(
                    "step/pulse times must be finite constants or parameters; width must be nonnegative",
                ));
            }
            return Ok(Node::Call(name.to_owned(), args));
        }
        Ok(match name {
            "t" => Node::Time,
            "pi" => Node::Constant(std::f64::consts::PI),
            "e" => Node::Constant(std::f64::consts::E),
            _ => Node::Constant(
                *self
                    .parameters
                    .get(name)
                    .ok_or(FlyError("unknown voltage parameter"))?,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic_functions_and_bound_parameters() {
        let p = BTreeMap::from([("amplitude".into(), 100.0), ("frequency".into(), 0.14)]);
        let e = VoltageExpression::parse("amplitude*cos(2*pi*frequency*t)+5", &p).unwrap();
        assert!((e.evaluate(0.0) - 105.0).abs() < 1e-12);
        assert!((e.evaluate(0.5 / 0.14) + 95.0).abs() < 1e-12);
        for (s, v) in [
            ("-2^2", -4.0),
            ("2^3^2", 512.0),
            ("2^-2", 0.25),
            ("max(abs(-2),min(3,sqrt(16)))+exp(0)+sin(0)", 4.0),
        ] {
            assert_eq!(VoltageExpression::parse(s, &p).unwrap().evaluate(0.0), v);
        }
        assert!(e.to_wgsl().unwrap().contains("cos("));
    }
    #[test]
    fn thresholds_and_invalid_inputs() {
        let p = BTreeMap::new();
        let e = VoltageExpression::parse("step(2)+pulse(4,0.5)", &p).unwrap();
        assert_eq!(e.evaluate(1.9), 0.0);
        assert_eq!(e.evaluate(2.0), 1.0);
        assert_eq!(e.evaluate(4.0), 2.0);
        assert_eq!(e.evaluate(4.5), 1.0);
        assert_eq!(e.next_event(2.0), Some(4.0));
        assert_eq!(e.next_event(4.0), Some(4.5));
        for s in [
            "",
            "unknown",
            "sin(1,2)",
            "min(1)",
            "1e",
            "1e999",
            "1;exit",
            "sin(1",
            "2 3",
            "step(t)",
            "pulse(1,-1)",
            "step(1/0)",
        ] {
            assert!(VoltageExpression::parse(s, &p).is_err(), "{s}");
        }
    }
}
