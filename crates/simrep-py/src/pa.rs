use numpy::{IntoPyArray, PyArray3, PyReadonlyArray3, PyUntypedArrayMethods, ndarray::Array3};
use pyo3::{exceptions::PyValueError, prelude::*};
use serde::{Deserialize, Serialize};
use simrep_core::{FieldType, MirrorFlags, PotentialArray, Symmetry};
use simrep_io::{FastScalable, Pa0Metadata, PaFile, PaMode, read_fast_adjust, read_pa, write_pa};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub type Arrays<'py> = (
    Bound<'py, PyArray3<f64>>,
    Bound<'py, PyArray3<bool>>,
    String,
);

pub fn error(error: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(error.to_string())
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    pub mode: i32,
    pub max_voltage: f64,
    pub ng: u32,
    pub symmetry: String,
    pub mirrors: [bool; 3],
    pub anti_mirrors: [bool; 3],
    pub field_type: String,
    pub refinable: bool,
    pub spacing_mm: [f64; 3],
    pub pa0: Option<Trailer>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trailer {
    pub reference_voltage: f64,
    pub electrode_indices: Vec<i64>,
    // null = omitted flag, false = disabled flag, list = scalable voltages.
    pub scalable: serde_json::Value,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            mode: -1,
            max_voltage: 100_000.0,
            ng: 100,
            symmetry: "planar".into(),
            mirrors: [false; 3],
            anti_mirrors: [false; 3],
            field_type: "electrostatic".into(),
            refinable: true,
            spacing_mm: [1.0; 3],
            pa0: None,
        }
    }
}

fn flags(f: [bool; 3]) -> MirrorFlags {
    MirrorFlags {
        x: f[0],
        y: f[1],
        z: f[2],
    }
}

pub fn owned(
    potentials: PyReadonlyArray3<'_, f64>,
    electrodes: PyReadonlyArray3<'_, bool>,
    metadata: &str,
) -> PyResult<PaFile> {
    let m: Metadata = serde_json::from_str(metadata).map_err(error)?;
    let shape = potentials.shape();
    if shape != electrodes.shape() || shape.contains(&0) {
        return Err(error(
            "potentials and electrodes must have the same nonempty (nz, ny, nx) shape",
        ));
    }
    // Copy while attached: another Python thread may change these buffers once
    // the GIL is released. No borrowed Python memory enters native computation.
    let mut a = PotentialArray::new(shape[2], shape[1], shape[0]);
    a.potentials = potentials.as_array().iter().copied().collect();
    a.electrodes = electrodes.as_array().iter().copied().collect();
    if a.potentials.iter().any(|v| !v.is_finite())
        || m.spacing_mm.iter().any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err(error(
            "potentials must be finite and spacing_mm positive and finite",
        ));
    }
    a.symmetry = match m.symmetry.as_str() {
        "planar" => Symmetry::Planar,
        "cylindrical" if a.nz == 1 => Symmetry::Cylindrical,
        _ => return Err(error("symmetry must be planar or cylindrical (with nz=1)")),
    };
    a.field_type = match m.field_type.as_str() {
        "electrostatic" => FieldType::Electrostatic,
        "magnetic" => FieldType::Magnetic,
        _ => return Err(error("field_type must be electrostatic or magnetic")),
    };
    a.refinable = m.refinable;
    a.mirrors = flags(m.mirrors);
    a.anti_mirrors = flags(m.anti_mirrors);
    [a.dx_mm, a.dy_mm, a.dz_mm] = m.spacing_mm;
    let mut file = PaFile::new(a);
    file.mode = match m.mode {
        -1 => PaMode::Standard,
        -2 => PaMode::Extended,
        _ => return Err(error("PA mode must be -1 or -2")),
    };
    file.max_voltage = m.max_voltage;
    file.ng = m.ng;
    file.pa0 = m
        .pa0
        .map(|t| {
            let scalable = match t.scalable {
                serde_json::Value::Null => None,
                serde_json::Value::Bool(false) => Some(FastScalable::Disabled),
                v => Some(FastScalable::Voltages(serde_json::from_value(v)?)),
            };
            Ok::<_, serde_json::Error>(Pa0Metadata {
                reference_voltage: t.reference_voltage,
                electrode_indices: t.electrode_indices,
                scalable,
            })
        })
        .transpose()
        .map_err(error)?;
    Ok(file)
}

pub fn arrays(py: Python<'_>, file: PaFile) -> PyResult<Arrays<'_>> {
    let a = file.array;
    let m = Metadata {
        mode: file.mode.value(),
        max_voltage: file.max_voltage,
        ng: file.ng,
        symmetry: if a.symmetry == Symmetry::Cylindrical {
            "cylindrical"
        } else {
            "planar"
        }
        .into(),
        mirrors: [a.mirrors.x, a.mirrors.y, a.mirrors.z],
        anti_mirrors: [a.anti_mirrors.x, a.anti_mirrors.y, a.anti_mirrors.z],
        field_type: if a.field_type == FieldType::Magnetic {
            "magnetic"
        } else {
            "electrostatic"
        }
        .into(),
        refinable: a.refinable,
        spacing_mm: [a.dx_mm, a.dy_mm, a.dz_mm],
        pa0: file.pa0.map(|t| Trailer {
            reference_voltage: t.reference_voltage,
            electrode_indices: t.electrode_indices,
            scalable: match t.scalable {
                None => serde_json::Value::Null,
                Some(FastScalable::Disabled) => serde_json::Value::Bool(false),
                Some(FastScalable::Voltages(v)) => serde_json::json!(v),
            },
        }),
    };
    let shape = (a.nz, a.ny, a.nx);
    Ok((
        Array3::from_shape_vec(shape, a.potentials)
            .map_err(error)?
            .into_pyarray(py),
        Array3::from_shape_vec(shape, a.electrodes)
            .map_err(error)?
            .into_pyarray(py),
        serde_json::to_string(&m).map_err(error)?,
    ))
}

#[pyfunction]
pub fn read(py: Python<'_>, path: String) -> PyResult<Arrays<'_>> {
    let file = py
        .detach(|| read_pa(&path))
        .map_err(|e| error(format!("{path}: {e}")))?;
    arrays(py, file)
}

#[pyfunction]
pub fn write(
    py: Python<'_>,
    path: String,
    potentials: PyReadonlyArray3<'_, f64>,
    electrodes: PyReadonlyArray3<'_, bool>,
    metadata: &str,
) -> PyResult<()> {
    let file = owned(potentials, electrodes, metadata)?;
    py.detach(|| write_pa(&path, &file))
        .map_err(|e| error(format!("{path}: {e}")))
}

#[pyfunction]
pub fn adjust(py: Python<'_>, path: String, voltages: Vec<(usize, f64)>) -> PyResult<Arrays<'_>> {
    let file = py
        .detach(|| {
            let mut file = read_pa(&path)?;
            file.array = read_fast_adjust(&path, &voltages)?;
            Ok::<_, std::io::Error>(file)
        })
        .map_err(|e| error(format!("{path}: {e}")))?;
    arrays(py, file)
}
