//! Static magnetic export readers. Coordinates become mm and flux density G.
//! Point rows may be unordered, but must cover a complete, uniform tensor grid.
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Auto,
    Comsol,
    Opera,
    Delimited,
    Axisymmetric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthUnit {
    Mm,
    M,
    Cm,
    Inch,
}

impl LengthUnit {
    pub fn millimeters(self) -> f64 {
        match self {
            Self::Mm => 1.0,
            Self::M => 1000.0,
            Self::Cm => 10.0,
            Self::Inch => 25.4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluxUnit {
    Gauss,
    Tesla,
    Millitesla,
}

impl FluxUnit {
    pub fn gauss(self) -> f64 {
        match self {
            Self::Gauss => 1.0,
            Self::Tesla => 1e4,
            Self::Millitesla => 10.0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ReadOptions {
    pub format: Format,
    /// None detects recognized header units, otherwise mm/G.
    pub length_unit: Option<LengthUnit>,
    pub flux_unit: Option<FluxUnit>,
    /// Source column indices, zero-based: x,y,z,Bx,By,Bz or z,r,Bz,Br.
    pub columns: Option<Vec<usize>>,
    /// None accepts comma, semicolon, tab and whitespace.
    pub delimiter: Option<char>,
}

#[derive(Clone, Debug)]
pub struct MagneticGrid {
    pub dims: [usize; 3],
    /// Cartesian xyz, or axial z / radius / zero for axisymmetric data.
    pub origin_mm: [f64; 3],
    pub spacing_mm: [f64; 3],
    /// x-fast vectors (axial B, radial B, zero for axisymmetric data).
    pub values_gauss: Vec<[f64; 3]>,
    pub axisymmetric: bool,
    pub length_unit: LengthUnit,
    pub flux_unit: FluxUnit,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn read(path: impl AsRef<Path>, options: &ReadOptions) -> io::Result<MagneticGrid> {
    let text = std::fs::read_to_string(path.as_ref())?;
    let mut options = options.clone();
    if options.format == Format::Auto
        && path
            .as_ref()
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("table"))
    {
        options.format = Format::Opera;
    }
    parse(&text, &options)
}

fn numbers(line: &str, delimiter: Option<char>) -> Result<Vec<f64>, std::num::ParseFloatError> {
    let delimiter = delimiter
        .or_else(|| line.contains(',').then_some(','))
        .or_else(|| line.contains(';').then_some(';'));
    let columns: Vec<_> = match delimiter {
        Some(d) if !d.is_whitespace() => line.split(d).collect(),
        _ => line.split_whitespace().collect(),
    };
    columns
        .into_iter()
        .map(|v| v.trim().replace(['D', 'd'], "E").parse::<f64>())
        .collect()
}

fn units(header: &str) -> (Option<LengthUnit>, Option<FluxUnit>) {
    let header = header
        .lines()
        .filter(|line| {
            let label = line
                .trim()
                .trim_start_matches(['%', '#', '!', '/'])
                .trim_start()
                .to_ascii_lowercase();
            ![
                "model:",
                "model name:",
                "version:",
                "date:",
                "file:",
                "filename:",
                "project:",
            ]
            .iter()
            .any(|prefix| label.starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let tokens: Vec<_> = header
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let has = |s| tokens.iter().any(|t| t == s);
    let length = if has("mm") {
        Some(LengthUnit::Mm)
    } else if has("cm") {
        Some(LengthUnit::Cm)
    } else if has("inch")
        || header.to_ascii_lowercase().contains("[in]")
        || header.to_ascii_lowercase().contains("(in)")
    {
        Some(LengthUnit::Inch)
    } else if has("m") {
        Some(LengthUnit::M)
    } else {
        None
    };
    let flux = if has("mt") {
        Some(FluxUnit::Millitesla)
    } else if has("t") || has("tesla") {
        Some(FluxUnit::Tesla)
    } else if has("g") || has("gauss") {
        Some(FluxUnit::Gauss)
    } else {
        None
    };
    (length, flux)
}

pub fn parse(text: &str, options: &ReadOptions) -> io::Result<MagneticGrid> {
    let axisymmetric = options.format == Format::Axisymmetric;
    let width = if axisymmetric { 4 } else { 6 };
    let columns = options
        .columns
        .clone()
        .unwrap_or_else(|| (0..width).collect());
    if columns.len() != width {
        return Err(invalid(format!("expected {width} column indices")));
    }
    let mut unique = columns.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != columns.len() {
        return Err(invalid("magnetic data columns must be distinct"));
    }
    let mut header = String::new();
    let mut rows = Vec::new();
    let mut grid_axes = Vec::new();
    let mut blocks: Vec<Vec<f64>> = Vec::new();
    let mut grid_mode = false;
    let mut opera_body = options.format != Format::Opera;
    let mut started = false;
    for (i, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if !opera_body {
            if line == "0" {
                opera_body = true;
            } else {
                header.push_str(line);
                header.push('\n');
            }
            continue;
        }
        if line.starts_with(['%', '#', '!', '/']) {
            header.push_str(line);
            header.push('\n');
            let tag = line.trim_start_matches('%').trim().to_ascii_lowercase();
            if tag == "grid" {
                grid_mode = true;
            } else if grid_mode && tag.starts_with("data") {
                blocks.push(Vec::new());
            }
            continue;
        }
        let values = match numbers(line, options.delimiter) {
            Ok(v) => v,
            Err(_)
                if !started
                    && !grid_mode
                    && !line
                        .trim_start_matches(['+', '-'])
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ';')) =>
            {
                header.push_str(line);
                header.push('\n');
                continue;
            }
            Err(_) => {
                return Err(invalid(format!(
                    "invalid magnetic number on line {}",
                    i + 1
                )));
            }
        };
        if values.iter().any(|v| !v.is_finite()) {
            return Err(invalid(format!(
                "nonfinite magnetic data on line {}",
                i + 1
            )));
        }
        started = true;
        if grid_mode {
            if let Some(block) = blocks.last_mut() {
                block.extend(values);
            } else {
                grid_axes.push(values);
            }
        } else {
            if columns.iter().any(|&c| c >= values.len()) {
                return Err(invalid(format!(
                    "missing magnetic column on line {}",
                    i + 1
                )));
            }
            rows.push(columns.iter().map(|&c| values[c]).collect::<Vec<_>>());
        }
    }
    if !opera_body {
        return Err(invalid("Opera table header requires a terminating 0 line"));
    }
    if grid_mode {
        if grid_axes.is_empty() || grid_axes.len() > 3 || blocks.len() != 3 || axisymmetric {
            return Err(invalid(
                "COMSOL Grid requires 1..3 coordinate rows and three % Data blocks (Bx,By,Bz)",
            ));
        }
        while grid_axes.len() < 3 {
            grid_axes.push(vec![0.0]);
        }
        let count = grid_axes
            .iter()
            .try_fold(1usize, |n, a| n.checked_mul(a.len()))
            .ok_or_else(|| invalid("magnetic grid dimensions overflow"))?;
        if blocks.iter().any(|b| b.len() != count) {
            return Err(invalid(
                "COMSOL Grid data block size does not match coordinates",
            ));
        }
        for (k, &z) in grid_axes[2].iter().enumerate() {
            for (j, &y) in grid_axes[1].iter().enumerate() {
                for (i, &x) in grid_axes[0].iter().enumerate() {
                    let index = (k * grid_axes[1].len() + j) * grid_axes[0].len() + i;
                    rows.push(vec![
                        x,
                        y,
                        z,
                        blocks[0][index],
                        blocks[1][index],
                        blocks[2][index],
                    ]);
                }
            }
        }
    }
    if rows.is_empty() {
        return Err(invalid("magnetic map contains no data"));
    }
    let (detected_length, detected_flux) = units(&header);
    let length_unit = options
        .length_unit
        .or(detected_length)
        .unwrap_or(LengthUnit::Mm);
    let flux_unit = options
        .flux_unit
        .or(detected_flux)
        .unwrap_or(FluxUnit::Gauss);
    let mut points = Vec::with_capacity(rows.len());
    for row in rows {
        let (p, b) = if axisymmetric {
            ([row[0], row[1], 0.0], [row[2], row[3], 0.0])
        } else {
            ([row[0], row[1], row[2]], [row[3], row[4], row[5]])
        };
        let p = p.map(|v| {
            if v == 0.0 {
                0.0
            } else {
                v * length_unit.millimeters()
            }
        });
        let b = b.map(|v| v * flux_unit.gauss());
        if p.iter().chain(&b).any(|v| !v.is_finite()) {
            return Err(invalid("magnetic unit conversion overflow"));
        }
        points.push((p, b));
    }
    let axes: [Vec<f64>; 3] = std::array::from_fn(|a| {
        let mut values: Vec<_> = points.iter().map(|(p, _)| p[a]).collect();
        values.sort_by(f64::total_cmp);
        values.dedup_by(|a, b| *a == *b);
        values
    });
    let dims = axes.each_ref().map(Vec::len);
    let count = dims
        .into_iter()
        .try_fold(1usize, |n, d| n.checked_mul(d))
        .ok_or_else(|| invalid("magnetic grid dimensions overflow"))?;
    if count != points.len() {
        return Err(invalid(
            "scattered/incomplete or duplicate magnetic points: re-export a complete regular grid; no implicit scattered resampling",
        ));
    }
    let origin_mm = std::array::from_fn(|a| axes[a][0]);
    let spacing_mm = std::array::from_fn(|a| {
        if dims[a] == 1 {
            1.0
        } else {
            (axes[a][dims[a] - 1] - axes[a][0]) / (dims[a] - 1) as f64
        }
    });
    if spacing_mm.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err(invalid("magnetic grid spacing overflow or collapse"));
    }
    for a in 0..3 {
        if axes[a].iter().enumerate().any(|(i, &v)| {
            (v - (origin_mm[a] + i as f64 * spacing_mm[a])).abs()
                > 1e-7 * spacing_mm[a] + 32.0 * f64::EPSILON * v.abs()
        }) {
            return Err(invalid(
                "nonuniform/scattered magnetic coordinates: re-export a uniform grid",
            ));
        }
    }
    if axisymmetric && (origin_mm[1] != 0.0 || dims[1] < 2) {
        return Err(invalid(
            "axisymmetric magnetic grid must include r=0 and at least two radii",
        ));
    }
    let mut values_gauss = vec![[0.0; 3]; count];
    let mut seen = vec![false; count];
    for (p, b) in points {
        let q: [usize; 3] =
            std::array::from_fn(|a| axes[a].binary_search_by(|v| v.total_cmp(&p[a])).unwrap());
        let index = (q[2] * dims[1] + q[1]) * dims[0] + q[0];
        if std::mem::replace(&mut seen[index], true) {
            return Err(invalid("duplicate magnetic grid point"));
        }
        if axisymmetric && q[1] == 0 && b[1].abs() > 1e-12 * b[0].abs().max(1.0) {
            return Err(invalid("axisymmetric Br must vanish at r=0"));
        }
        values_gauss[index] = b;
    }
    Ok(MagneticGrid {
        dims,
        origin_mm,
        spacing_mm,
        values_gauss,
        axisymmetric,
        length_unit,
        flux_unit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_headers_units_column_order_and_fortran_numbers() {
        for (format, header) in [
            (
                Format::Comsol,
                "% x (cm),y (cm),z (cm),Bx (mT),By (mT),Bz (mT)\n",
            ),
            (
                Format::Opera,
                "2 1 1\n1 X [CM]\n2 Y [CM]\n3 Z [CM]\n4 BX [MT]\n5 BY [MT]\n6 BZ [MT]\n0\n",
            ),
            (
                Format::Delimited,
                "x[cm];y[cm];z[cm];Bx[mT];By[mT];Bz[mT]\n",
            ),
        ] {
            let grid = parse(
                &format!("{header}1;0;0;1D0;2;3\n0;0;0;4;5;6\n"),
                &ReadOptions {
                    format,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(grid.dims, [2, 1, 1]);
            assert_eq!(grid.spacing_mm[0], 10.0);
            assert_eq!(grid.values_gauss, [[40.0, 50.0, 60.0], [10.0, 20.0, 30.0]]);
        }
        let grid = parse(
            "3,2,1,0,0,0\n",
            &ReadOptions {
                columns: Some(vec![5, 4, 3, 2, 1, 0]),
                length_unit: Some(LengthUnit::Inch),
                flux_unit: Some(FluxUnit::Tesla),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(grid.values_gauss[0], [1e4, 2e4, 3e4]);
        let grid = parse("0 -0 0 1 2 3\n1 0 -0 1 2 3", &ReadOptions::default()).unwrap();
        assert_eq!(grid.dims, [2, 1, 1]);
        let grid = parse(
            "% Model: coil_mm_mt.mph\n% Length unit: m\n% x (m) y (m) z (m) Bx (T) By (T) Bz (T)\n0 0 0 0.1 0.2 0.3\n0.001 0 0 0.1 0.2 0.3",
            &ReadOptions::default(),
        ).unwrap();
        assert_eq!(grid.length_unit, LengthUnit::M);
        assert_eq!(grid.flux_unit, FluxUnit::Tesla);
        assert_eq!(grid.spacing_mm[0], 1.0);
        assert_eq!(grid.values_gauss[0], [1000.0, 2000.0, 3000.0]);
    }

    #[test]
    fn comsol_grid_axisym_and_invalid_clouds() {
        let grid = parse("% Grid\n0 1\n0 1\n% Data (Bx)\n1 2\n3 4\n% Data (By)\n0 0\n0 0\n% Data (Bz)\n5 6\n7 8\n", &ReadOptions::default()).unwrap();
        assert_eq!(grid.values_gauss[3], [4.0, 0.0, 8.0]);
        let axisym = ReadOptions {
            format: Format::Axisymmetric,
            ..Default::default()
        };
        assert!(
            parse("0 0 1 0\n1 0 1 0\n0 1 1 2\n1 1 1 2", &axisym)
                .unwrap()
                .axisymmetric
        );
        for text in [
            "0 0 0 1 2 3\n1 1 0 1 2 3",
            "0 0 0 1 2 3\n1 0 0 1 2 3\n3 0 0 1 2 3",
            "0 0 0 NaN 0 0",
            "0 0 0 1 2 3\n0 0 0 1 2 3",
            "0,0,0,1,2,3\n1,,0,1,2,3,4",
            "0,,0,1,2,3,4\n1,0,0,1,2,3",
        ] {
            assert!(parse(text, &ReadOptions::default()).is_err(), "{text}");
        }
    }
}
