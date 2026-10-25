//! SIMION binary PA, PA#, and PA0 files.
//!
//! Based on SIMION-2024's shipped `lib/{python,cpp,perl}` PA implementations,
//! plus the supplemental documentation `anisotropically_scaled_pas.html` and
//! `writing_pa0_files.html` and `antimirroring.html`. No SIMION executable
//! code is used.
//! `potential_array.html` documents the non-refinable bit (mirror bit 28).
//!
//! The 32-byte mode -1 header and 56-byte mode -2 header use little-endian
//! i32/f64 values. Data is x-fastest f64, with electrode points encoded as
//! `potential + 2 * max_voltage` and identified by `raw > max_voltage`.
//! That offset can round an electrode potential to the precision available
//! near `2 * max_voltage`, as in the reference implementations.
//! PA# uses the same layout; its electrode potentials are electrode numbers.
//! PA0 appends adjustable-electrode metadata, including optional scalable
//! voltages and 64-bit offsets for arrays with at least 2^31 points.

use simrep_core::{FieldType, MirrorFlags, PotentialArray, Symmetry};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

/// Binary header version supported by the SIMION reference implementations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaMode {
    /// SIMION 7/8.0: 32-byte header, implicit 1 mm grid spacing.
    Standard,
    /// SIMION 8.1+: 56-byte header, explicit dx/dy/dz in millimeters.
    Extended,
}

impl PaMode {
    pub fn value(self) -> i32 {
        match self {
            Self::Standard => -1,
            Self::Extended => -2,
        }
    }

    pub fn header_bytes(self) -> usize {
        match self {
            Self::Standard => 32,
            Self::Extended => 56,
        }
    }
}

/// Optional PA0 extended fast-scalable flag and its current voltages.
#[derive(Clone, Debug, PartialEq)]
pub enum FastScalable {
    /// An explicit zero flag. Older files may omit the flag altogether.
    Disabled,
    /// A one flag followed by one voltage per electrode-offset entry.
    Voltages(Vec<f64>),
}

/// Adjustable-electrode trailer stored after the grid in a PA0 file.
#[derive(Clone, Debug, PartialEq)]
pub struct Pa0Metadata {
    /// Reference potential used in PA1, PA2, ... files, normally 10000 V.
    pub reference_voltage: f64,
    /// First flat grid index for each electrode, or -1 if absent.
    ///
    /// Entry zero is the fast-scalable electrode. There are at least 32
    /// entries, ending after the last represented electrode (or at 32).
    pub electrode_indices: Vec<i64>,
    /// `None` preserves the older layout with no scalable flag.
    pub scalable: Option<FastScalable>,
}

/// Potential array together with format-specific metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct PaFile {
    pub array: PotentialArray,
    /// Preferred header mode; non-unit spacing upgrades Standard to Extended.
    pub mode: PaMode,
    /// Electrode-encoding threshold, not the measured maximum potential.
    pub max_voltage: f64,
    /// Magnetic field scaling factor, packed into mirror bits 4 through 20.
    pub ng: u32,
    /// Present for a PA0 with adjustable-electrode metadata.
    pub pa0: Option<Pa0Metadata>,
}

impl PaFile {
    /// Wrap an array using the reference defaults (max_voltage=100000, ng=100).
    pub fn new(array: PotentialArray) -> Self {
        let mode = if unit_spacing(&array) {
            PaMode::Standard
        } else {
            PaMode::Extended
        };
        Self {
            array,
            mode,
            max_voltage: 100_000.0,
            ng: 100,
            pa0: None,
        }
    }

    /// Header mode that will be written, including automatic spacing upgrade.
    pub fn effective_mode(&self) -> PaMode {
        if !unit_spacing(&self.array) {
            PaMode::Extended
        } else {
            self.mode
        }
    }

    /// Decode a binary PA stream, including any PA0 trailer.
    pub fn read(mut reader: impl Read) -> io::Result<Self> {
        let mode = match read_i32(&mut reader)? {
            -1 => PaMode::Standard,
            -2 => PaMode::Extended,
            mode => return Err(invalid(format!("unsupported PA header mode {mode}"))),
        };
        let symmetry = match read_i32(&mut reader)? {
            0 => Symmetry::Cylindrical,
            1 => Symmetry::Planar,
            value => return Err(invalid(format!("invalid PA symmetry {value}"))),
        };
        let max_voltage = read_f64(&mut reader)?;
        let nx = read_dimension(&mut reader)?;
        let ny = read_dimension(&mut reader)?;
        let nz = read_dimension(&mut reader)?;
        let mirror = read_i32(&mut reader)? as u32;
        let (dx_mm, dy_mm, dz_mm) = match mode {
            PaMode::Standard => (1.0, 1.0, 1.0),
            PaMode::Extended => (
                read_f64(&mut reader)?,
                read_f64(&mut reader)?,
                read_f64(&mut reader)?,
            ),
        };
        let mut file = Self {
            array: PotentialArray {
                nx,
                ny,
                nz,
                symmetry,
                mirrors: MirrorFlags {
                    x: mirror & 1 != 0,
                    y: mirror & 2 != 0,
                    z: mirror & 4 != 0,
                },
                anti_mirrors: MirrorFlags {
                    x: mirror & (1 << 29) != 0,
                    y: mirror & (1 << 30) != 0,
                    z: mirror & (1 << 31) != 0,
                },
                field_type: if mirror & 8 != 0 {
                    FieldType::Magnetic
                } else {
                    FieldType::Electrostatic
                },
                refinable: mirror & (1 << 28) == 0,
                dx_mm,
                dy_mm,
                dz_mm,
                potentials: Vec::new(),
                electrodes: Vec::new(),
            },
            mode,
            max_voltage,
            ng: (mirror >> 4) & ((1 << 17) - 1),
            pa0: None,
        };
        let count = file.validate_header()?;
        file.array
            .potentials
            .try_reserve_exact(count)
            .map_err(io::Error::other)?;
        file.array
            .electrodes
            .try_reserve_exact(count)
            .map_err(io::Error::other)?;
        for _ in 0..count {
            let raw = read_f64(&mut reader)?;
            if !raw.is_finite() {
                return Err(invalid("non-finite PA potential"));
            }
            let electrode = raw > max_voltage;
            file.array.potentials.push(if electrode {
                raw - 2.0 * max_voltage
            } else {
                raw
            });
            file.array.electrodes.push(electrode);
        }
        file.pa0 = read_pa0(&mut reader, count)?;
        Ok(file)
    }

    /// Encode a binary PA stream. Provide `pa0` metadata to write a PA0.
    ///
    /// Invalid metadata or unencodable potentials are rejected before writing.
    /// For file output, use [`write_pa`] to validate before creating the file.
    pub fn write(&self, mut writer: impl Write) -> io::Result<()> {
        self.validate()?;
        let a = &self.array;
        writer.write_all(&self.effective_mode().value().to_le_bytes())?;
        writer.write_all(&(i32::from(a.symmetry == Symmetry::Planar)).to_le_bytes())?;
        writer.write_all(&self.max_voltage.to_le_bytes())?;
        for dim in [a.nx, a.ny, a.nz] {
            writer.write_all(&(dim as i32).to_le_bytes())?;
        }
        let mirror = u32::from(a.mirrors.x)
            | (u32::from(a.mirrors.y) << 1)
            | (u32::from(a.mirrors.z) << 2)
            | (u32::from(a.field_type == FieldType::Magnetic) << 3)
            | (self.ng << 4)
            | (u32::from(!a.refinable) << 28)
            | (u32::from(a.anti_mirrors.x) << 29)
            | (u32::from(a.anti_mirrors.y) << 30)
            | (u32::from(a.anti_mirrors.z) << 31);
        writer.write_all(&mirror.to_le_bytes())?;
        if self.effective_mode() == PaMode::Extended {
            for spacing in [a.dx_mm, a.dy_mm, a.dz_mm] {
                writer.write_all(&spacing.to_le_bytes())?;
            }
        }
        for (&potential, &electrode) in a.potentials.iter().zip(&a.electrodes) {
            let raw = if electrode {
                potential + 2.0 * self.max_voltage
            } else {
                potential
            };
            writer.write_all(&raw.to_le_bytes())?;
        }
        if let Some(pa0) = &self.pa0 {
            let count = pa0.electrode_indices.iter().filter(|&&i| i != -1).count();
            writer.write_all(&(count as i32).to_le_bytes())?;
            writer.write_all(&pa0.reference_voltage.to_le_bytes())?;
            for &index in &pa0.electrode_indices {
                if a.potentials.len() >= (1 << 31) {
                    writer.write_all(&index.to_le_bytes())?;
                } else {
                    writer.write_all(&(index as i32).to_le_bytes())?;
                }
            }
            match &pa0.scalable {
                None => {}
                Some(FastScalable::Disabled) => writer.write_all(&0_i32.to_le_bytes())?,
                Some(FastScalable::Voltages(values)) => {
                    writer.write_all(&1_i32.to_le_bytes())?;
                    for value in values {
                        writer.write_all(&value.to_le_bytes())?;
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_header(&self) -> io::Result<usize> {
        let a = &self.array;
        if !(3..=90_000).contains(&a.nx)
            || !(3..=90_000).contains(&a.ny)
            || !(1..=90_000).contains(&a.nz)
        {
            return Err(invalid(
                "PA dimensions require nx,ny in 3..=90000 and nz in 1..=90000",
            ));
        }
        if a.symmetry == Symmetry::Cylindrical && (a.nz != 1 || !a.mirrors.y) {
            return Err(invalid("cylindrical PA requires nz=1 and y mirroring"));
        }
        if a.nz == 1 && a.mirrors.z {
            return Err(invalid("z mirroring requires a 3D PA"));
        }
        for spacing in [a.dx_mm, a.dy_mm, a.dz_mm] {
            if !spacing.is_finite() || !(1e-6..=900.0).contains(&spacing) {
                return Err(invalid(
                    "PA grid spacing must be finite and in 1e-6..=900 mm",
                ));
            }
        }
        if !self.max_voltage.is_finite()
            || self.max_voltage <= 0.0
            || !(2.0 * self.max_voltage).is_finite()
        {
            return Err(invalid(
                "PA max_voltage must be positive, finite, and safe to double",
            ));
        }
        if self.ng > 90_000 {
            return Err(invalid("PA ng must be in 0..=90000"));
        }
        a.nx.checked_mul(a.ny)
            .and_then(|n| n.checked_mul(a.nz))
            .and_then(|n| n.checked_mul(8).map(|_| n))
            .ok_or_else(|| invalid("PA point count overflows addressable storage"))
    }

    fn validate(&self) -> io::Result<()> {
        let count = self.validate_header()?;
        let a = &self.array;
        if a.potentials.len() != count || a.electrodes.len() != count {
            return Err(invalid("PA vector lengths do not match dimensions"));
        }
        for (&potential, &electrode) in a.potentials.iter().zip(&a.electrodes) {
            let raw = if electrode {
                potential + 2.0 * self.max_voltage
            } else {
                potential
            };
            if !potential.is_finite() || !raw.is_finite() || (raw > self.max_voltage) != electrode {
                return Err(invalid(
                    "potential cannot be encoded with this PA max_voltage",
                ));
            }
        }
        if let Some(pa0) = &self.pa0 {
            validate_pa0(pa0, count)?;
        }
        Ok(())
    }
}

/// Read a PA, PA#, or PA0 file. The binary layout, not the suffix, is decoded.
pub fn read_pa(path: impl AsRef<Path>) -> io::Result<PaFile> {
    PaFile::read(BufReader::new(File::open(path)?))
}

/// Write a PA, PA#, or PA0 file, preserving the supplied format metadata.
pub fn write_pa(path: impl AsRef<Path>, file: &PaFile) -> io::Result<()> {
    file.validate()?;
    let mut writer = BufWriter::new(File::create(path)?);
    file.write(&mut writer)?;
    writer.flush()
}

fn unit_spacing(a: &PotentialArray) -> bool {
    a.dx_mm == 1.0 && a.dy_mm == 1.0 && a.dz_mm == 1.0
}

fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read_bytes<const N: usize>(reader: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_i32(reader: &mut impl Read) -> io::Result<i32> {
    Ok(i32::from_le_bytes(read_bytes(reader)?))
}

fn read_f64(reader: &mut impl Read) -> io::Result<f64> {
    Ok(f64::from_le_bytes(read_bytes(reader)?))
}

fn read_dimension(reader: &mut impl Read) -> io::Result<usize> {
    usize::try_from(read_i32(reader)?).map_err(|_| invalid("negative PA dimension"))
}

fn optional_i32(reader: &mut impl Read) -> io::Result<Option<i32>> {
    let mut bytes = [0; 4];
    if reader.read(&mut bytes[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut bytes[1..])?;
    Ok(Some(i32::from_le_bytes(bytes)))
}

fn read_pa0(reader: &mut impl Read, points: usize) -> io::Result<Option<Pa0Metadata>> {
    let Some(adjustable) = optional_i32(reader)? else {
        return Ok(None);
    };
    if !(1..=1001).contains(&adjustable) {
        return Err(invalid(
            "PA0 adjustable-electrode count must be in 1..=1001",
        ));
    }
    let reference_voltage = read_f64(reader)?;
    let mut electrode_indices = Vec::new();
    let mut found = 0;
    while electrode_indices.len() < 32 || found < adjustable {
        if electrode_indices.len() >= 1001 {
            return Err(invalid("PA0 electrode table exceeds 1001 entries"));
        }
        let index = if points >= (1 << 31) {
            i64::from_le_bytes(read_bytes(reader)?)
        } else {
            i64::from(read_i32(reader)?)
        };
        if index != -1 {
            found += 1;
        }
        electrode_indices.push(index);
    }
    if found != adjustable {
        return Err(invalid(
            "PA0 adjustable-electrode count does not match table",
        ));
    }
    let scalable = match optional_i32(reader)? {
        None => None,
        Some(0) => Some(FastScalable::Disabled),
        Some(1) => {
            let values = (0..electrode_indices.len())
                .map(|_| read_f64(reader))
                .collect::<io::Result<Vec<_>>>()?;
            Some(FastScalable::Voltages(values))
        }
        Some(flag) => return Err(invalid(format!("unsupported PA0 scalable flag {flag}"))),
    };
    if optional_i32(reader)?.is_some() {
        return Err(invalid("unexpected data after PA0 trailer"));
    }
    let pa0 = Pa0Metadata {
        reference_voltage,
        electrode_indices,
        scalable,
    };
    validate_pa0(&pa0, points)?;
    Ok(Some(pa0))
}

fn validate_pa0(pa0: &Pa0Metadata, points: usize) -> io::Result<()> {
    let indices = &pa0.electrode_indices;
    let count = indices.iter().filter(|&&i| i != -1).count();
    let nread = indices
        .iter()
        .rposition(|&i| i != -1)
        .map_or(32, |i| (i + 1).max(32));
    if !(1..=1001).contains(&count) || indices.len() != nread || indices.len() > 1001 {
        return Err(invalid("invalid PA0 electrode-table length or count"));
    }
    if indices
        .iter()
        .any(|&i| i < -1 || (i >= 0 && i as u64 >= points as u64))
    {
        return Err(invalid("PA0 electrode offset outside the array"));
    }
    if !pa0.reference_voltage.is_finite() {
        return Err(invalid("non-finite PA0 reference voltage"));
    }
    if let Some(FastScalable::Voltages(values)) = &pa0.scalable {
        // Entries for absent electrodes are undefined by the format.
        if values.len() != indices.len()
            || values
                .iter()
                .zip(indices)
                .any(|(v, &i)| i != -1 && !v.is_finite())
        {
            return Err(invalid("invalid PA0 scalable voltage table"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pa0_uses_64_bit_offsets_at_two_billion_points() {
        let mut bytes = Vec::new();
        bytes.extend(1_i32.to_le_bytes());
        bytes.extend(10_000_f64.to_le_bytes());
        for n in 0..32 {
            bytes.extend(if n == 1 { (1_i64 << 31) - 1 } else { -1 }.to_le_bytes());
        }
        let pa0 = read_pa0(&mut bytes.as_slice(), 1_usize << 31)
            .unwrap()
            .unwrap();
        assert_eq!(pa0.electrode_indices[1], (1_i64 << 31) - 1);
        assert_eq!(pa0.electrode_indices.len(), 32);
        assert!(pa0.scalable.is_none());
    }
}
