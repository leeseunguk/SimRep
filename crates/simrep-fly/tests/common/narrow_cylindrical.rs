use simrep_core::{PotentialArray, Symmetry};
use simrep_fly::{Vector, surface::SurfaceModel, surface_sdf::SdfVolumes};

/// Bilinear meridian f(x,r) = -A0 - A1*(x-.1) - (x-.99)*r.
/// Along the segment, r^2 = q0 + .07*t + t^2 and the first entry is t=.05,
/// followed by a narrow exit at .06 and a later entry at .7.
pub fn fixture() -> (SdfVolumes, Vector, Vector) {
    let a0 = 1.531986471620668;
    let a1 = -1.6804385595255453;
    let q0 = 2.96537375234029_f64;
    let q1 = 0.07_f64;
    let values: Vec<f64> = [0.0, 3.0]
        .into_iter()
        .flat_map(|r| [0.0, 3.0].map(|x| -a0 - a1 * (x - 0.1) - ((x - 0.1) - 0.89) * r))
        .collect();
    let sidecar = serde_json::json!({
        "version": 2, "dims": [2, 2, 1], "origin_mm": [0.0, 0.0, 0.0],
        "spacing_mm": [3.0, 3.0, 3.0], "symmetry": "cylindrical",
        "electrode_numbers": [1], "node_electrodes": [null, null, null, null],
        "cut_fractions": vec![[Option::<f64>::None; 6]; 4],
        "cut_electrodes": vec![[Option::<usize>::None; 6]; 4],
        "sampled_sdfs": [{"number": 1, "dims": [2, 2, 1], "origin_mm": [0.0, 0.0, 0.0],
            "spacing_mm": [3.0, 3.0, 3.0], "values": values}]
    });
    let mut pa = PotentialArray::new(2, 2, 1);
    pa.dx_mm = 3.0;
    pa.dy_mm = 3.0;
    pa.dz_mm = 3.0;
    pa.symmetry = Symmetry::Cylindrical;
    let surface = SurfaceModel::from_reader(sidecar.to_string().as_bytes()).unwrap();
    let sdf = SdfVolumes::build(&surface, &pa, 1).unwrap();
    let z = (q0 - (q1 / 2.0).powi(2)).sqrt();
    (sdf, [0.1, q1 / 2.0, z], [1.1, 1.0 + q1 / 2.0, z])
}
