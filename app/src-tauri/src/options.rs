//! The option values the UI may send to the commands, as one list the UI is checked against.
//!
//! `options_match_the_commands` proves every value listed here is accepted by the command that
//! takes it. It does not prove every enum variant is listed: a new variant must be added to the
//! name functions below (they are exhaustive matches), but the test still passes if `OPTIONS` and
//! the length constants are left unchanged. `options_json_is_current` keeps
//! `src/scene/engineOptions.json` equal to this list; the UI's `src/scene/options.test.ts`
//! compares its selectable values with that file.
//! Regenerate the file after a change with `SIMREP_WRITE_OPTIONS=1 cargo test options`.

use serde::Serialize;

use crate::fly::{Collision, Precision, Shape};
use crate::geometry::{CadSurface, Method};
use crate::refine::DeviceChoice;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineOptions {
    /// `refine_start` request.device.
    pub refine_devices: &'static [&'static str],
    /// `fly` request.precision.
    pub fly_precisions: &'static [&'static str],
    /// `fly` request.collision.
    pub collisions: &'static [&'static str],
    /// `fly` request.source.shape (beam sources).
    pub beam_shapes: &'static [&'static str],
    /// `export_array` what.format of a SIMION array export.
    pub pa_export_formats: &'static [&'static str],
    /// `create_template` kind.
    pub templates: &'static [&'static str],
    /// `build_geometry` request.method.
    pub build_methods: &'static [&'static str],
    /// `build_geometry` request.cadSurface.
    pub cad_surfaces: &'static [&'static str],
}

pub const OPTIONS: EngineOptions = EngineOptions {
    refine_devices: &["auto", "gpu", "cpu"],
    fly_precisions: &["live", "precise"],
    collisions: &["allCorners", "nearestNode"],
    beam_shapes: &["point", "line", "disk"],
    pa_export_formats: &["pa", "paHash", "pa0"],
    templates: &["einzel", "quadrupole", "reflectron", "trap"],
    build_methods: &["aligned", "cutcell"],
    cad_surfaces: &["tessellated", "exact"],
};

fn device_name(d: DeviceChoice) -> &'static str {
    match d {
        DeviceChoice::Auto => "auto",
        DeviceChoice::Gpu => "gpu",
        DeviceChoice::Cpu => "cpu",
    }
}

fn precision_name(p: Precision) -> &'static str {
    match p {
        Precision::Live => "live",
        Precision::Precise => "precise",
    }
}

fn collision_name(c: Collision) -> &'static str {
    match c {
        Collision::AllCorners => "allCorners",
        Collision::NearestNode => "nearestNode",
    }
}

fn shape_name(s: Shape) -> &'static str {
    match s {
        Shape::Point => "point",
        Shape::Line => "line",
        Shape::Disk => "disk",
    }
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::Aligned => "aligned",
        Method::Cutcell => "cutcell",
    }
}

fn cad_surface_name(s: CadSurface) -> &'static str {
    match s {
        CadSurface::Tessellated => "tessellated",
        CadSurface::Exact => "exact",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;

    /// Every listed value parses into `T` and names itself back; an unlisted value is refused.
    fn check<T: DeserializeOwned>(list: &[&str], name: fn(T) -> &'static str) {
        for &value in list {
            let parsed: T = serde_json::from_value(serde_json::json!(value))
                .unwrap_or_else(|e| panic!("{value}: {e}"));
            assert_eq!(name(parsed), value);
        }
        assert!(serde_json::from_value::<T>(serde_json::json!("notAnOption")).is_err());
    }

    #[test]
    fn options_match_the_commands() {
        check::<DeviceChoice>(OPTIONS.refine_devices, device_name);
        check::<Precision>(OPTIONS.fly_precisions, precision_name);
        check::<Collision>(OPTIONS.collisions, collision_name);
        check::<Shape>(OPTIONS.beam_shapes, shape_name);
        check::<Method>(OPTIONS.build_methods, method_name);
        check::<CadSurface>(OPTIONS.cad_surfaces, cad_surface_name);
        // Variant counts: serde would reject any list value that is not a variant, so the lists
        // hold no extras. These literal counts do not detect a variant missing from a list.
        assert_eq!(OPTIONS.refine_devices.len(), 3);
        assert_eq!(OPTIONS.fly_precisions.len(), 2);
        assert_eq!(OPTIONS.collisions.len(), 2);
        assert_eq!(OPTIONS.beam_shapes.len(), 3);
        assert_eq!(OPTIONS.build_methods.len(), 2);
        assert_eq!(OPTIONS.cad_surfaces.len(), 2);

        for kind in OPTIONS.templates {
            assert!(
                crate::templates::template(kind).is_some(),
                "template {kind}"
            );
        }
        assert!(crate::templates::template("notAnOption").is_none());

        let dir = std::env::temp_dir().join(format!("simrep-options-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for format in OPTIONS
            .pa_export_formats
            .iter()
            .copied()
            .chain(["notAnOption"])
        {
            let session = crate::session::Session::new(
                "options-test.pa".into(),
                simrep_io::PaFile::new(simrep_core::PotentialArray::new(2, 2, 1)),
                None,
            );
            let unknown =
                match crate::export::write_simion(&session, &dir.join("out.pa"), format, &[]) {
                    Ok(_) => false,
                    // "pa0" needs a fast-adjust set this array lacks: a coded refusal, not an unknown format.
                    Err(e) => serde_json::to_value(&e).unwrap()["detail"]
                        .as_str()
                        .unwrap()
                        .starts_with("unknown PA format"),
                };
            assert_eq!(unknown, format == "notAnOption", "{format}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_json_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/scene/engineOptions.json"
        );
        let expected = serde_json::to_value(&OPTIONS).unwrap();
        if std::env::var_os("SIMREP_WRITE_OPTIONS").is_some() {
            std::fs::write(
                path,
                serde_json::to_string_pretty(&expected).unwrap() + "\n",
            )
            .unwrap();
        }
        let file: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            file, expected,
            "{path} is stale: run SIMREP_WRITE_OPTIONS=1 cargo test options"
        );
    }
}
