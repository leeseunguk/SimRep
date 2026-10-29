//! Portable flight partitions and lossless, provenance-checked CSV assembly.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use simrep_solve::gpu::GpuResult;

pub const CSV_HEADER: &str = "ion,tof_us,x_mm,y_mm,z_mm,ke_ev,reason,electrode_number,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us";
pub const TRAJECTORY_HEADER: &str =
    "trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Shard {
    pub index: usize,
    pub count: usize,
}

impl FromStr for Shard {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || "--shard requires i/N with 1 <= i <= N".to_owned();
        let (i, n) = text.split_once('/').ok_or_else(invalid)?;
        let index = i.parse().map_err(|_| invalid())?;
        let count = n.parse().map_err(|_| invalid())?;
        if count == 0 || index == 0 || index > count {
            return Err(invalid());
        }
        Ok(Self { index, count })
    }
}

impl Shard {
    pub fn contains(self, global_index: usize) -> bool {
        global_index % self.count == self.index - 1
    }
}

pub fn debug_units<S: serde::Serializer>(
    value: &Option<(simrep_io::bfield::LengthUnit, simrep_io::bfield::FluxUnit)>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format!("{value:?}"))
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".json");
    PathBuf::from(name)
}

// Hash bytes, not timestamps or absolute paths, so copied inputs work on another PC.
fn fingerprint(path: &Path) -> GpuResult<Value> {
    let bytes = fs::read(path)?;
    let mut value = json!({"sha256": sha256(&bytes)});
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pa0"))
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let stem = path.file_stem().ok_or("PA has no file stem")?;
        let mut bases = BTreeMap::new();
        for entry in fs::read_dir(parent)? {
            let entry = entry?.path();
            if entry.file_stem() != Some(stem) {
                continue;
            }
            let Some(ext) = entry.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let ext = ext.to_ascii_lowercase();
            if let Some(number) = ext.strip_prefix("pa")
                && (number == "_"
                    || !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit()))
                && ext != "pa0"
            {
                bases.insert(ext, sha256(&fs::read(entry)?));
            }
        }
        value["bases"] = json!(bases);
    }
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.to_ascii_lowercase().starts_with("pa"))
    {
        let magnetic = path.with_extension("mag.json");
        if magnetic.is_file() {
            value["magnetic_sidecar_sha256"] = json!(sha256(&fs::read(magnetic)?));
        }
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        let mut parsed: Value = serde_json::from_slice(&bytes)?;
        // Run-file magnetic paths are cwd-relative, whereas exported field and
        // adaptive sidecars resolve their own components next to the sidecar.
        let parent = if parsed.get("physics").is_some() {
            Path::new(".")
        } else {
            path.parent().unwrap_or(Path::new("."))
        };
        snapshot_paths(&mut parsed, parent)?;
        value["dependencies"] = parsed;
    }
    Ok(value)
}

fn snapshot_paths(value: &mut Value, parent: &Path) -> GpuResult<()> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "pa" | "cuts"
                        | "particles"
                        | "run_file"
                        | "bfield_map"
                        | "magnetic_vector_potential"
                        | "magnetic_pa"
                        | "bfield_coil"
                        | "mbmr"
                        | "path"
                        | "potential"
                        | "components"
                        | "base"
                        | "file"
                ) {
                    match value {
                        Value::String(path) => *value = fingerprint(&parent.join(path))?,
                        Value::Array(paths) => {
                            for value in paths {
                                if let Some(path) = value.as_str() {
                                    *value = fingerprint(&parent.join(path))?;
                                }
                            }
                        }
                        _ => {}
                    }
                } else {
                    snapshot_paths(value, parent)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                snapshot_paths(value, parent)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn run_identity(mut settings: Value) -> GpuResult<Value> {
    if settings["cuts"].is_null()
        && (settings["no_cuts"] == false || !settings["surface_collisions"].is_null())
        && let Some(pa) = settings["pa"].as_str()
    {
        let mut pa = PathBuf::from(pa);
        if pa
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            let manifest: Value = serde_json::from_slice(&fs::read(&pa)?)?;
            if let Some(base) = manifest["base"].as_str() {
                pa = pa.parent().unwrap_or(Path::new(".")).join(base);
            }
        }
        let cuts = pa.with_extension("cuts.json");
        if cuts.is_file() {
            settings["automatic_cuts"] = fingerprint(&cuts)?;
        }
    }
    let object = settings.as_object_mut().ok_or("invalid flight settings")?;
    for key in ["shard", "output", "trajectories", "gpu_adapter"] {
        object.remove(key);
    }
    snapshot_paths(&mut settings, Path::new("."))?;
    Ok(json!({"simrep_version": env!("CARGO_PKG_VERSION"), "settings": settings}))
}

pub fn tof_stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"mean": null, "std": null, "min": null, "max": null});
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    json!({
        "mean": mean,
        "std": (values.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt(),
        "min": values.iter().copied().fold(f64::INFINITY, f64::min),
        "max": values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightFile {
    pub schema_version: u32,
    pub run_hash: String,
    pub total_ions: usize,
    pub shard: Option<Shard>,
    pub csv_sha256: String,
    // Full precision values, in CSV row order. Re-parsing the printed CSV
    // would lose the final bit and change recomputed TOF statistics.
    pub tof_us: Vec<f64>,
    pub summary: Value,
    pub trajectories: Option<TrajectoryFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrajectoryFile {
    pub file: PathBuf,
    pub sha256: String,
}

pub fn write_metadata(path: &Path, metadata: &FlightFile) -> GpuResult<()> {
    fs::write(sidecar(path), serde_json::to_vec_pretty(metadata)?)?;
    Ok(())
}

#[derive(Args)]
pub struct MergeOptions {
    /// Destination flight CSV (also writes OUT.csv.json).
    pub output: PathBuf,
    /// Complementary shard CSVs, each with its .csv.json sidecar.
    #[arg(required = true, num_args = 1..)]
    pub inputs: Vec<PathBuf>,
}

pub fn merge(options: MergeOptions) -> GpuResult<()> {
    let mut rows = BTreeMap::new();
    let mut trajectories = BTreeMap::<usize, Vec<String>>::new();
    let mut partition = None;
    let mut hash = None;
    let mut total = None;
    let mut shards = BTreeSet::new();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut summary = Value::Null;
    let mut compute_sum = 0.0;
    let mut compute_max = 0.0_f64;
    let mut prep_sum = 0.0;
    let mut has_trajectories = None;
    for path in &options.inputs {
        if *path == options.output {
            return Err("merge-flights output must not overwrite an input".into());
        }
        let metadata: FlightFile = serde_json::from_slice(&fs::read(sidecar(path))?)?;
        if metadata.schema_version != 1 {
            return Err("unsupported flight sidecar schema".into());
        }
        let shard = metadata
            .shard
            .ok_or("merge-flights requires shard CSV sidecars")?;
        if shard.count == 0 || shard.index == 0 || shard.index > shard.count {
            return Err("invalid shard metadata".into());
        }
        if partition.is_some_and(|n| n != shard.count)
            || total.is_some_and(|n| n != metadata.total_ions)
            || hash.as_ref().is_some_and(|h| h != &metadata.run_hash)
        {
            return Err("merge-flights run hash or shard layout mismatch".into());
        }
        if !shards.insert(shard.index) {
            return Err("merge-flights duplicate shard".into());
        }
        partition = Some(shard.count);
        total = Some(metadata.total_ions);
        hash = Some(metadata.run_hash);
        let bytes = fs::read(path)?;
        if sha256(&bytes) != metadata.csv_sha256 {
            return Err("merge-flights CSV checksum mismatch".into());
        }
        let csv = std::str::from_utf8(&bytes)?;
        if !csv.ends_with('\n') {
            return Err("merge-flights CSV must end with a newline".into());
        }
        let mut lines = csv.lines();
        if lines.next() != Some(CSV_HEADER) {
            return Err("merge-flights incompatible CSV header".into());
        }
        let lines: Vec<_> = lines.collect();
        if lines.len() != metadata.tof_us.len() {
            return Err("merge-flights TOF metadata length mismatch".into());
        }
        let mut owned = BTreeSet::new();
        for (line, tof) in lines.into_iter().zip(metadata.tof_us) {
            let columns: Vec<_> = line.split(',').collect();
            if columns.len() != 11 || !tof.is_finite() {
                return Err("merge-flights invalid flight record".into());
            }
            let index: usize = columns[0].parse()?;
            if index == 0 || index > metadata.total_ions || !shard.contains(index - 1) {
                return Err("merge-flights ion index does not belong to shard".into());
            }
            if rows
                .insert(index, (line.to_owned(), tof, columns[6].to_owned()))
                .is_some()
            {
                return Err("merge-flights duplicate ion index".into());
            }
            owned.insert(index);
        }
        if has_trajectories.is_some_and(|present| present != metadata.trajectories.is_some()) {
            return Err("merge-flights inconsistent trajectory exports".into());
        }
        has_trajectories = Some(metadata.trajectories.is_some());
        if let Some(trajectory) = metadata.trajectories {
            let trajectory_path = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(trajectory.file);
            let bytes = fs::read(trajectory_path)?;
            if sha256(&bytes) != trajectory.sha256 {
                return Err("merge-flights trajectory checksum mismatch".into());
            }
            let text = std::str::from_utf8(&bytes)?;
            let mut lines = text.lines();
            if !text.ends_with('\n') || lines.next() != Some(TRAJECTORY_HEADER) {
                return Err("merge-flights incompatible trajectory CSV".into());
            }
            let mut sampled = BTreeSet::new();
            for line in lines {
                let columns: Vec<_> = line.split(',').collect();
                if columns.len() != 8 {
                    return Err("merge-flights invalid trajectory record".into());
                }
                let index = columns[0].parse::<usize>()?;
                if !owned.contains(&index) {
                    return Err("merge-flights trajectory ion does not belong to shard".into());
                }
                trajectories.entry(index).or_default().push(line.to_owned());
                sampled.insert(index);
            }
            if owned != sampled {
                return Err("merge-flights missing ion trajectories".into());
            }
        }
        compute_sum += metadata.summary["compute_ms"]
            .as_f64()
            .ok_or("missing compute_ms")?;
        compute_max = compute_max.max(
            metadata.summary["compute_ms"]
                .as_f64()
                .ok_or("missing compute_ms")?,
        );
        prep_sum += metadata.summary["prep_ms"]
            .as_f64()
            .ok_or("missing prep_ms")?;
        for reason in metadata.summary["splat_counts"]
            .as_object()
            .ok_or("missing splat_counts")?
            .keys()
        {
            counts.entry(reason.clone()).or_default();
        }
        summary = metadata.summary;
    }
    let count = partition.ok_or("merge-flights requires input shards")?;
    let total = total.unwrap();
    if shards.len() != count || rows.len() != total || !rows.keys().copied().eq(1..=total) {
        return Err("merge-flights missing shard or ion index".into());
    }
    let mut csv = format!("{CSV_HEADER}\n");
    let mut arrivals = Vec::new();
    let mut tofs = Vec::new();
    for (line, tof, reason) in rows.values() {
        csv.push_str(line);
        csv.push('\n');
        *counts
            .get_mut(reason)
            .ok_or("merge-flights invalid termination reason")? += 1;
        if reason == "plane" {
            arrivals.push(*tof);
        }
        tofs.push(*tof);
    }
    summary["ion_count"] = json!(total);
    summary["total_ions"] = json!(total);
    summary["shard"] = Value::Null;
    summary["merged_shards"] = json!(count);
    summary["splat_counts"] = json!(counts);
    summary["arrivals"] = json!(arrivals.len());
    summary["arrival_tof_us"] = tof_stats(&arrivals);
    summary["compute_ms"] = json!(compute_sum);
    summary["compute_ms_sum"] = json!(compute_sum);
    summary["compute_ms_max"] = json!(compute_max);
    summary["prep_ms"] = json!(prep_sum);
    summary["output_paths"] = json!([options.output, sidecar(&options.output)]);
    let trajectory = if has_trajectories == Some(true) {
        let output = options.output.with_extension("trajectories.csv");
        if options.inputs.contains(&output) {
            return Err("merge-flights trajectory output must not overwrite an input".into());
        }
        let mut csv = format!("{TRAJECTORY_HEADER}\n");
        for lines in trajectories.values() {
            for line in lines {
                csv.push_str(line);
                csv.push('\n');
            }
        }
        Some((output, csv))
    } else {
        None
    };
    // All validation finishes before opening any output.
    fs::write(&options.output, &csv)?;
    let trajectory = if let Some((path, csv)) = trajectory {
        fs::write(&path, &csv)?;
        summary["output_paths"]
            .as_array_mut()
            .unwrap()
            .push(json!(path));
        Some(TrajectoryFile {
            file: PathBuf::from(
                path.file_name()
                    .ok_or("trajectory output has no filename")?,
            ),
            sha256: sha256(csv.as_bytes()),
        })
    } else {
        None
    };
    summary["schema_version"] = json!(1);
    summary["simrep_version"] = json!(env!("CARGO_PKG_VERSION"));
    write_metadata(
        &options.output,
        &FlightFile {
            schema_version: 1,
            run_hash: hash.unwrap(),
            total_ions: total,
            shard: None,
            csv_sha256: sha256(csv.as_bytes()),
            tof_us: tofs,
            summary: summary.clone(),
            trajectories: trajectory,
        },
    )?;
    crate::json::set_summary(summary);
    eprintln!(
        "merged {count} shards, {total} ions: {}",
        options.output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_uses_bytes_not_paths_and_covers_bases_sources_and_active_cuts() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let settings = |dir: &Path| {
            json!({
                "pa": dir.join("lens.pa0"), "particles": dir.join("ions.ion"),
                "no_cuts": false, "cuts": null, "surface_collisions": null,
            })
        };
        for dir in [a.path(), b.path()] {
            fs::write(dir.join("lens.pa0"), b"potential bytes").unwrap();
            fs::write(dir.join("lens.pa1"), b"basis bytes").unwrap();
            fs::write(dir.join("lens.cuts.json"), b"{}").unwrap();
            fs::write(dir.join("ions.ion"), b"particle bytes").unwrap();
        }
        let original = run_identity(settings(a.path())).unwrap();
        let mut copied = settings(b.path());
        copied["shard"] = json!({"index": 2, "count": 7});
        copied["output"] = json!("other.csv");
        copied["gpu_adapter"] = json!("other GPU");
        assert_eq!(original, run_identity(copied).unwrap());
        for file in ["lens.pa0", "lens.pa1", "ions.ion", "lens.cuts.json"] {
            let path = a.path().join(file);
            let previous = fs::read(&path).unwrap();
            fs::write(
                &path,
                if file.ends_with("json") {
                    b"{\"changed\":true}".as_slice()
                } else {
                    b"changed".as_slice()
                },
            )
            .unwrap();
            assert_ne!(
                original,
                run_identity(settings(a.path())).unwrap(),
                "{file}"
            );
            fs::write(path, previous).unwrap();
        }
        let lower = a.path().join("lens.pa1");
        let upper = a.path().join("lens.PA1");
        fs::rename(&lower, &upper).unwrap();
        assert_eq!(original, run_identity(settings(a.path())).unwrap());
        fs::rename(upper, lower).unwrap();
        let mut staircase = settings(a.path());
        staircase["no_cuts"] = json!(true);
        let before = run_identity(staircase.clone()).unwrap();
        fs::write(a.path().join("lens.cuts.json"), b"not even valid JSON").unwrap();
        assert_eq!(before, run_identity(staircase).unwrap());
    }

    #[test]
    fn identity_covers_adaptive_sibling_surfaces_and_scalar_magnetic_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("base.pa0"), b"potential").unwrap();
        fs::write(dir.path().join("base.cuts.json"), b"{}").unwrap();
        let manifest = dir.path().join("lens.adaptive.json");
        fs::write(&manifest, b"{\"base\":\"base.pa0\"}").unwrap();
        let settings =
            json!({"pa": manifest, "no_cuts": false, "cuts": null, "surface_collisions": null});
        let before = run_identity(settings.clone()).unwrap();
        fs::write(dir.path().join("base.cuts.json"), b"{\"changed\":true}").unwrap();
        assert_ne!(before, run_identity(settings).unwrap());
        let magnetic = dir.path().join("field.pa");
        fs::write(&magnetic, b"magnetic potential").unwrap();
        let sidecar = dir.path().join("field.mag.json");
        fs::write(&sidecar, b"{\"potential\":\"field.pa\",\"solid_cells\":[]}").unwrap();
        let settings = json!({"physics": {"magnetic_pa": [magnetic]}});
        let before = run_identity(settings.clone()).unwrap();
        fs::write(sidecar, b"{\"potential\":\"field.pa\",\"solid_cells\":[1]}").unwrap();
        assert_ne!(before, run_identity(settings).unwrap());
    }
}
