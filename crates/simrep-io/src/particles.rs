//! Physical initial conditions from SIMION particle definition files.

use std::io;
use std::path::Path;

/// One particle in physical units, independent of a particular flight engine.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleDefinition {
    /// Time of birth in microseconds.
    pub birth_time_us: f64,
    /// Mass in unified atomic mass units.
    pub mass_amu: f64,
    /// Charge in elementary-charge units (negative and neutral are allowed).
    pub charge_e: f64,
    /// Cartesian source position in millimeters.
    pub position_mm: [f64; 3],
    /// Unit direction vector. SIMION azimuth +90 points toward -Z.
    pub direction: [f64; 3],
    /// Initial kinetic energy in electronvolts.
    pub energy_ev: f64,
    /// Source-charge weighting factor (CWF); does not alter physical q/m.
    pub charge_weight: f64,
    /// SIMION display color (retained even in headless loading).
    pub color: f64,
}

impl ParticleDefinition {
    pub(crate) fn validate(&mut self) -> io::Result<()> {
        if !self.birth_time_us.is_finite()
            || !self.mass_amu.is_finite()
            || self.mass_amu <= 0.0
            || !self.charge_e.is_finite()
            || !self.energy_ev.is_finite()
            || self.energy_ev < 0.0
            || !self.charge_weight.is_finite()
            || !self.color.is_finite()
            || self.position_mm.iter().any(|x| !x.is_finite())
            || self.direction.iter().any(|x| !x.is_finite())
        {
            return Err(invalid(
                "particle values must be finite, with positive mass and nonnegative KE",
            ));
        }
        let magnitude = self.direction[0]
            .hypot(self.direction[1])
            .hypot(self.direction[2]);
        if magnitude == 0.0 || !magnitude.is_finite() {
            return Err(invalid(
                "particle direction must have a finite nonzero magnitude",
            ));
        }
        for component in &mut self.direction {
            *component /= magnitude;
        }
        Ok(())
    }
}

/// Parse SIMION ION CSV: TOB, mass, charge, X, Y, Z, AZ, EL, KE, CWF, color.
///
/// AZ and EL are in degrees, position is in millimeters, and energy is in eV.
/// CWF and color may be omitted; when supplied they must be finite numbers.
/// Signed CWF is retained, as in SIMION 8.1.2.20 and later. Empty lines and `;` comments (including inline comments) are
/// accepted. Headers, quoted CSV, missing physical fields, and extra fields
/// are rejected. All errors identify the one-based input line.
pub fn parse_ion(input: &str) -> io::Result<Vec<ParticleDefinition>> {
    let mut particles = Vec::new();
    for (line_number, line) in input.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parse_row = || -> io::Result<ParticleDefinition> {
            let fields = line.split(',').collect::<Vec<_>>();
            if !(9..=11).contains(&fields.len()) {
                return Err(invalid(
                    "ION row requires 9 physical fields and optional CWF/color",
                ));
            }
            let values = fields
                .iter()
                .map(|field| finite_number(field.trim()))
                .collect::<io::Result<Vec<_>>>()?;
            let azimuth = values[6].to_radians();
            let elevation = values[7].to_radians();
            let mut particle = ParticleDefinition {
                birth_time_us: values[0],
                mass_amu: values[1],
                charge_e: values[2],
                position_mm: [values[3], values[4], values[5]],
                direction: [
                    elevation.cos() * azimuth.cos(),
                    elevation.sin(),
                    -elevation.cos() * azimuth.sin(),
                ],
                energy_ev: values[8],
                charge_weight: values.get(9).copied().unwrap_or(1.0),
                color: values.get(10).copied().unwrap_or(0.0),
            };
            particle.validate()?;
            Ok(particle)
        };
        particles.push(
            parse_row()
                .map_err(|error| invalid(format!("ION line {}: {error}", line_number + 1)))?,
        );
    }
    Ok(particles)
}

/// Read an ION text file into physical initial conditions.
pub fn read_ion(path: impl AsRef<Path>) -> io::Result<Vec<ParticleDefinition>> {
    parse_ion(&std::fs::read_to_string(path)?)
}

/// Read an ION or context-free FLY2 file, selected by its extension.
pub fn read_particles(path: impl AsRef<Path>) -> io::Result<Vec<ParticleDefinition>> {
    read_particles_in(path, &crate::fly2::Fly2Context::default())
}

/// Read particles with explicit workbench placement and field-query context.
pub fn read_particles_in(
    path: impl AsRef<Path>,
    context: &crate::fly2::Fly2Context<'_>,
) -> io::Result<Vec<ParticleDefinition>> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("ion") => read_ion(path),
        Some(extension) if extension.eq_ignore_ascii_case("fly2") => {
            crate::fly2::read_fly2_in(path, context)
        }
        _ => Err(invalid(
            "unsupported particle format; expected .ion or .fly2",
        )),
    }
}

pub(crate) fn finite_number(input: &str) -> io::Result<f64> {
    input
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid(format!("expected finite numeric literal, got {input:?}")))
}

pub(crate) fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
