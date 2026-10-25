//! Fast adjustment of an existing PA0 using its sibling solution arrays.

use crate::{FastScalable, PaFile, read_pa};
use simrep_core::PotentialArray;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Load a PA0 and apply `(adjustable_number, voltage)` overrides.
///
/// Unspecified adjustable voltages and any fixed/background potential already
/// in PA0 are preserved. Each represented sibling PA1, PA2, ... is checked for
/// matching geometry, including the electrode mask. Number 0 selects PA_;
/// its voltage refers to the signed electrode potential with greatest absolute
/// value, not a multiplier. Ordinary numbered solutions are normalized by
/// PA0's `reference_voltage`. Extended fast-scalable solutions use their
/// maximum-absolute electrode voltage and the PA0 stored voltage table.
///
/// Voltage differences are superimposed on PA0 rather than rebuilding from
/// zero, so PA_ and other existing background contributions are not lost or
/// added twice. No files are modified.
///
/// Sibling extension case follows the PA0 extension (`.PA0` uses `.PA1`).
/// Extended scalable solutions without electrode points (cropped arrays)
/// cannot be normalized and are rejected when overridden.
pub fn read_fast_adjust(
    path: impl AsRef<Path>,
    overrides: &[(usize, f64)],
) -> io::Result<PotentialArray> {
    let path = path.as_ref();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| s.eq_ignore_ascii_case("pa0"))
        .ok_or_else(|| invalid("fast adjustment requires a .pa0 file"))?;
    let base = read_pa(path)?;
    let metadata = base
        .pa0
        .as_ref()
        .ok_or_else(|| invalid("fast adjustment requires PA0 electrode metadata"))?;
    if metadata.reference_voltage <= 0.0 {
        return Err(invalid("PA0 reference_voltage must be positive"));
    }
    let mut requested = BTreeMap::new();
    for &(number, voltage) in overrides {
        if !voltage.is_finite() {
            return Err(invalid("fast-adjust override voltage must be finite"));
        }
        if metadata
            .electrode_indices
            .get(number)
            .is_none_or(|&index| index == -1)
        {
            let accepted: Vec<_> = metadata
                .electrode_indices
                .iter()
                .enumerate()
                .filter_map(|(number, &index)| (index != -1).then_some(number))
                .collect();
            return Err(invalid(format!(
                "adjustable electrode {number} is not represented in PA0; accepted electrode numbers: {accepted:?}"
            )));
        }
        if requested.insert(number, voltage).is_some() {
            return Err(invalid(format!(
                "duplicate voltage override for adjustable electrode {number}"
            )));
        }
    }
    let mut result = base.array.clone();
    for (number, &index) in metadata.electrode_indices.iter().enumerate() {
        if index == -1 {
            continue;
        }
        let suffix = if number == 0 {
            "_".to_owned()
        } else {
            number.to_string()
        };
        let sibling = path.with_extension(format!("{}{suffix}", &extension[..2]));
        let solution = read_pa(&sibling).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", sibling.display()))
        })?;
        if !same_geometry(&base, &solution) {
            return Err(invalid(format!(
                "{}: solution geometry differs from PA0",
                sibling.display()
            )));
        }
        let Some(&voltage) = requested.get(&number) else {
            continue;
        };
        let (current, reference) = if let Some(FastScalable::Voltages(values)) = &metadata.scalable
        {
            let (_, reference) = peak_electrode(&solution.array)?;
            (values[number], reference)
        } else if number == 0 {
            let (peak, reference) = peak_electrode(&solution.array)?;
            (base.array.potentials[peak], reference)
        } else {
            let index = index as usize;
            if !base.array.electrodes[index] {
                return Err(invalid(format!(
                    "PA0 electrode {number} offset is not an electrode point"
                )));
            }
            (base.array.potentials[index], metadata.reference_voltage)
        };
        let delta = (voltage - current) / reference;
        if !delta.is_finite() {
            return Err(invalid("non-finite fast-adjust scale"));
        }
        for (potential, &basis) in result.potentials.iter_mut().zip(&solution.array.potentials) {
            *potential += delta * basis;
            if !potential.is_finite() {
                return Err(invalid("non-finite superimposed potential"));
            }
        }
    }
    Ok(result)
}

fn peak_electrode(array: &PotentialArray) -> io::Result<(usize, f64)> {
    array
        .potentials
        .iter()
        .enumerate()
        .filter(|&(i, _)| array.electrodes[i])
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        .filter(|&(_, &value)| value != 0.0)
        .map(|(index, &value)| (index, value))
        .ok_or_else(|| {
            invalid("cannot normalize scalable solution without a nonzero electrode point")
        })
}

fn same_geometry(a: &PaFile, b: &PaFile) -> bool {
    let x = &a.array;
    let y = &b.array;
    x.nx == y.nx
        && x.ny == y.ny
        && x.nz == y.nz
        && x.symmetry == y.symmetry
        && x.mirrors == y.mirrors
        && x.anti_mirrors == y.anti_mirrors
        && x.field_type == y.field_type
        && x.refinable == y.refinable
        && x.dx_mm == y.dx_mm
        && x.dy_mm == y.dy_mm
        && x.dz_mm == y.dz_mm
        && x.electrodes == y.electrodes
        && a.ng == b.ng
}

fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
