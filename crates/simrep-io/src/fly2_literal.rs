//! Literal FLY2 input for builds that intentionally do not embed Lua.
//!
//! Supports particles/standard_beam tables, scalar fields and vector literals.
//! Executable Lua and distribution constructors fail rather than being sampled
//! differently from the native sandbox.
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::particles::{ParticleDefinition, invalid};

#[derive(Clone, Debug)]
pub struct Fly2Instance {
    pub origin_mm: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub scale: f64,
    pub spacing_mm: [f64; 3],
    pub dimensions: [usize; 3],
    pub working_origin_mm: [f64; 3],
    pub electrodes: Option<Arc<[bool]>>,
}

impl Default for Fly2Instance {
    fn default() -> Self {
        Self {
            origin_mm: [0.; 3],
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            scale: 1.,
            spacing_mm: [1.; 3],
            dimensions: [1; 3],
            working_origin_mm: [0.; 3],
            electrodes: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Fly2Field {
    pub potential_v: f64,
    pub electric_v_per_mm: [f64; 3],
    pub magnetic_gauss: [f64; 3],
    pub electrode: bool,
}

pub type Fly2FieldProvider<'a> =
    dyn Fn(usize, [f64; 3]) -> io::Result<Fly2Field> + Send + Sync + 'a;

#[derive(Default)]
pub struct Fly2Context<'a> {
    pub instances: Vec<Fly2Instance>,
    pub field: Option<&'a Fly2FieldProvider<'a>>,
    pub seed: u64,
    pub lua_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Number(f64),
    Symbol(u8),
}

fn tokens(source: &str) -> io::Result<Vec<Token>> {
    let bytes = source.trim_start_matches('\u{feff}').as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b if b.is_ascii_whitespace() => i += 1,
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                if bytes.get(i..i + 2) == Some(b"[[") {
                    i += 2;
                    while bytes.get(i..i + 2) != Some(b"]]") {
                        if i >= bytes.len() {
                            return Err(invalid("unterminated FLY2 comment"));
                        }
                        i += 1;
                    }
                    i += 2;
                } else {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
            }
            b'{' | b'}' | b'(' | b')' | b'=' | b',' | b';' => {
                out.push(Token::Symbol(bytes[i]));
                i += 1;
            }
            b if b.is_ascii_alphabetic() || b == b'_' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                out.push(Token::Word(
                    String::from_utf8_lossy(&bytes[start..i]).into_owned(),
                ));
            }
            b if b.is_ascii_digit() || b".+-".contains(&b) => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || b".eE+-".contains(&bytes[i]))
                {
                    i += 1;
                }
                let n = std::str::from_utf8(&bytes[start..i])
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| {
                        invalid("FLY2 requires finite numeric literals; Lua is unsupported")
                    })?;
                out.push(Token::Number(n));
            }
            _ => return Err(invalid("Lua is unsupported; expected literal FLY2 tables")),
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn symbol(&mut self, symbol: u8) -> bool {
        if self.tokens.get(self.at) == Some(&Token::Symbol(symbol)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, symbol: u8) -> io::Result<()> {
        if self.symbol(symbol) {
            Ok(())
        } else {
            Err(invalid(format!("FLY2 expected '{}'", symbol as char)))
        }
    }

    fn word(&mut self) -> io::Result<String> {
        if let Some(Token::Word(word)) = self.tokens.get(self.at) {
            let word = word.clone();
            self.at += 1;
            Ok(word)
        } else {
            Err(invalid(
                "FLY2 expected a literal table field; Lua is unsupported",
            ))
        }
    }

    fn number(&mut self) -> io::Result<f64> {
        if let Some(Token::Number(number)) = self.tokens.get(self.at) {
            let number = *number;
            self.at += 1;
            Ok(number)
        } else {
            Err(invalid(
                "FLY2 expected a numeric literal; Lua is unsupported",
            ))
        }
    }

    fn separator(&mut self, end: u8) -> io::Result<bool> {
        if self.symbol(end) {
            return Ok(true);
        }
        if !self.symbol(b',') && !self.symbol(b';') {
            return Err(invalid("FLY2 fields require a comma or semicolon"));
        }
        Ok(self.symbol(end))
    }

    fn vector(&mut self) -> io::Result<[f64; 3]> {
        if self.word()? != "vector" {
            return Err(invalid("Lua is unsupported; expected vector(x,y,z)"));
        }
        self.expect(b'(')?;
        let x = self.number()?;
        self.expect(b',')?;
        let y = self.number()?;
        self.expect(b',')?;
        let z = self.number()?;
        self.expect(b')')?;
        Ok([x, y, z])
    }

    fn beam(&mut self) -> io::Result<Vec<ParticleDefinition>> {
        self.expect(b'{')?;
        let mut scalars = BTreeMap::new();
        let mut vectors = BTreeMap::new();
        if !self.symbol(b'}') {
            loop {
                let key = self.word()?;
                self.expect(b'=')?;
                if [
                    "position",
                    "direction",
                    "velocity",
                    "pupil",
                    "pupil_position",
                    "window",
                ]
                .contains(&key.as_str())
                {
                    if vectors.insert(key, self.vector()?).is_some() {
                        return Err(invalid("duplicate FLY2 field"));
                    }
                } else if [
                    "n", "tob", "mass", "charge", "x", "y", "z", "az", "el", "ke", "cwf", "color",
                    "speed",
                ]
                .contains(&key.as_str())
                {
                    if scalars.insert(key, self.number()?).is_some() {
                        return Err(invalid("duplicate FLY2 field"));
                    }
                } else {
                    return Err(invalid(format!("unsupported literal FLY2 field {key}")));
                }
                if self.separator(b'}')? {
                    break;
                }
            }
        }
        let value = |key: &str, default| scalars.get(key).copied().unwrap_or(default);
        let count = value("n", 1.);
        if !(1. ..=10_000_000.).contains(&count) || count.fract() != 0. {
            return Err(invalid(
                "FLY2 particle count must be an integer in 1..10000000",
            ));
        }
        let position = vectors.get("position").copied().unwrap_or([
            value("x", 0.),
            value("y", 0.),
            value("z", 0.),
        ]);
        let mut direction = vectors.get("direction").copied().unwrap_or([1., 0., 0.]);
        if let Some(window) = vectors.get("window") {
            direction = std::array::from_fn(|a| position[a] - window[a]);
        } else if let Some(pupil) = vectors
            .get("pupil")
            .or_else(|| vectors.get("pupil_position"))
        {
            direction = std::array::from_fn(|a| pupil[a] - position[a]);
        } else if let Some(velocity) = vectors.get("velocity") {
            direction = *velocity;
        } else if scalars.contains_key("az") || scalars.contains_key("el") {
            let az = value("az", 0.).to_radians();
            let el = value("el", 0.).to_radians();
            direction = [el.cos() * az.cos(), el.sin(), -el.cos() * az.sin()];
        }
        // Native FLY2 normalizes the zero vector to +X before validation.
        let norm = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
        direction = if norm == 0. {
            [1., 0., 0.]
        } else {
            direction.map(|v| v / norm)
        };
        let mass = value("mass", 0.00054857990946);
        let speed = vectors
            .get("velocity")
            .map(|v| v.iter().map(|x| x * x).sum::<f64>().sqrt())
            .or_else(|| scalars.get("speed").copied());
        let energy = value(
            "ke",
            speed.map_or(0.1, |v| {
                (0.5 * 1.660538921e-27 / 1.602176565e-19 * 1e6) * mass * v * v
            }),
        );
        let mut particle = ParticleDefinition {
            birth_time_us: value("tob", 0.),
            mass_amu: mass,
            charge_e: value("charge", -1.),
            position_mm: position,
            direction,
            energy_ev: energy,
            charge_weight: value("cwf", 1.),
            color: value("color", 0.).trunc().rem_euclid(16.),
        };
        particle.validate()?;
        Ok(vec![particle; count as usize])
    }
}

pub fn parse_fly2(input: &str) -> io::Result<Vec<ParticleDefinition>> {
    parse_fly2_in(input, &Fly2Context::default())
}

/// Parse bytes already loaded by the caller; no filesystem or Lua access.
pub fn parse_fly2_in(
    input: &str,
    context: &Fly2Context<'_>,
) -> io::Result<Vec<ParticleDefinition>> {
    let mut parser = Parser {
        tokens: tokens(input)?,
        at: 0,
    };
    if parser.word()? != "particles" {
        return Err(invalid("Lua is unsupported; expected particles {...}"));
    }
    parser.expect(b'{')?;
    let mut particles = Vec::new();
    let mut coordinates = 0.;
    let mut have_coordinates = false;
    if !parser.symbol(b'}') {
        loop {
            match parser.word()?.as_str() {
                "coordinates" => {
                    if have_coordinates {
                        return Err(invalid("duplicate FLY2 coordinates"));
                    }
                    have_coordinates = true;
                    parser.expect(b'=')?;
                    coordinates = parser.number()?;
                }
                "standard_beam" | "standard_beam_define" => {
                    let beam = parser.beam()?;
                    if particles.len() + beam.len() > 10_000_000 {
                        return Err(invalid("FLY2 particle limit exceeded"));
                    }
                    particles.extend(beam);
                }
                _ => return Err(invalid("Lua is unsupported; expected standard_beam {...}")),
            }
            if parser.separator(b'}')? {
                break;
            }
        }
    }
    if parser.at != parser.tokens.len() {
        return Err(invalid("Lua is unsupported; trailing FLY2 statements"));
    }
    if coordinates < 0. || coordinates.fract() != 0. || coordinates > context.instances.len() as f64
    {
        return Err(invalid("missing or invalid FLY2 coordinates instance"));
    }
    if coordinates > 0. {
        let instance = &context.instances[coordinates as usize - 1];
        if !instance.scale.is_finite()
            || instance.scale <= 0.
            || instance
                .origin_mm
                .iter()
                .chain(instance.spacing_mm.iter())
                .chain(instance.axes.iter().flatten())
                .any(|v| !v.is_finite())
            || instance.spacing_mm.iter().any(|v| *v <= 0.)
            || (0..3).any(|a| {
                (0..3).any(|b| {
                    ((0..3)
                        .map(|k| instance.axes[a][k] * instance.axes[b][k])
                        .sum::<f64>()
                        - f64::from(a == b))
                    .abs()
                        > 1e-10
                })
            })
        {
            return Err(invalid("invalid FLY2 instance placement"));
        }
        for particle in &mut particles {
            particle.position_mm = std::array::from_fn(|a| {
                instance.origin_mm[a]
                    + (0..3)
                        .map(|b| {
                            instance.axes[b][a]
                                * instance.scale
                                * particle.position_mm[b]
                                * instance.spacing_mm[b]
                        })
                        .sum::<f64>()
            });
            particle.direction = std::array::from_fn(|a| {
                (0..3)
                    .map(|b| instance.axes[b][a] * particle.direction[b])
                    .sum()
            });
            particle.validate()?;
        }
    }
    Ok(particles)
}

pub fn read_fly2(path: impl AsRef<Path>) -> io::Result<Vec<ParticleDefinition>> {
    read_fly2_in(path, &Fly2Context::default())
}

pub fn read_fly2_in(
    path: impl AsRef<Path>,
    context: &Fly2Context<'_>,
) -> io::Result<Vec<ParticleDefinition>> {
    parse_fly2_in(&std::fs::read_to_string(path)?, context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_beams_and_units_match_the_native_contract() {
        let particles = parse_fly2("particles { coordinates=0, standard_beam {n=2, tob=1.5, mass=4, charge=-2, ke=2e3, position=vector(1,-2,.3), direction=vector(0,3,4), cwf=2, color=15}, standard_beam {} }").unwrap();
        assert_eq!(particles.len(), 3);
        assert_eq!(particles[0].position_mm, [1., -2., 0.3]);
        assert_eq!(particles[0].direction, [0., 0.6, 0.8]);
        assert_eq!(particles[0].energy_ev, 2000.);
        assert_eq!(particles[2].mass_amu, 0.00054857990946);
        assert_eq!(particles[2].charge_e, -1.);
    }

    #[test]
    fn executable_lua_and_malformed_literals_are_rejected() {
        for source in [
            "for i=1,2 do end",
            "particles {standard_beam {ke=distribution(fn)}}",
            "particles {standard_beam {n=0}}",
            "particles {standard_beam {mass=0}}",
            "particles {standard_beam {position=vector(1,2)}}",
            "particles {standard_beam {ke=1 mass=2}}",
            "particles {coordinates=1,standard_beam {}}",
            "particles {} particles {}",
            "particles {standard_beam {ke=0/0}}",
        ] {
            assert!(parse_fly2(source).is_err(), "{source}");
        }
        assert!(parse_fly2("particles {}").unwrap().is_empty());
    }
}
