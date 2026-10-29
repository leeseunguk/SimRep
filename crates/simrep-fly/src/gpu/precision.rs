use bytemuck::{Pod, Zeroable};

use super::MAX_BASES;

pub(super) fn study_is(name: &str) -> bool {
    cfg!(feature = "precision-study") && std::env::var("SIMREP_FLY_ROUND").is_ok_and(|v| v == name)
}

// Explicit padding gives both native double and portable float buffers the
// WGSL vec4 layout. Only these two concrete scalar types are Pod.
#[repr(C)]
#[derive(Clone, Copy, Zeroable)]
pub(super) struct Parameters<F = f64> {
    pub dims: [u32; 4],
    pub cells: [u32; 4],
    pub degrees: [u32; 4],
    pub flags: [u32; 4],
    pub spacing: [F; 4],
    pub axes: [[F; 4]; 3],
    pub bounds: [[F; 4]; 2],
    pub detector: [F; 4],
    pub stop_plane: [F; 4],
    pub integration: [F; 4],
    pub tolerance: [F; 4],
    pub counts: [u32; 4],
    pub padding: [u32; 4],
    pub voltages: [[F; 4]; MAX_BASES],
    pub origin: [F; 4],
    pub magnetic: [F; 4],
    pub collision: [F; 4],
    pub gas: [F; 4],
    pub collision_limits: [F; 4],
    pub step_limits: [F; 4],
    pub physics: [u32; 4],
    pub physics_padding: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Zeroable)]
pub(super) struct Ion<F = f64> {
    pub cell: [i32; 4],
    pub padding0: [u32; 4],
    pub offset: [F; 4],
    pub velocity: [F; 4],
    pub metadata: [F; 4],
    pub outcome: [u32; 4],
    pub padding1: [u32; 4],
    pub time: [F; 4],
    pub progress: [F; 4],
}

/// Only terminal output: offset.w and velocity.w carry the split TOF.
#[repr(C)]
#[derive(Clone, Copy, Zeroable)]
pub(super) struct EndState<F = f64> {
    pub cell: [i32; 4],
    pub outcome: [u32; 4],
    pub offset: [F; 4],
    pub velocity: [F; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Zeroable)]
pub(super) struct Sample<F = f64> {
    pub position: [F; 4],
    pub velocity: [F; 4],
}

// SAFETY: All fields are Pod arrays; the explicit padding above covers every
// alignment gap for f32 and f64. The layout sizes are checked below.
unsafe impl Pod for Parameters<f32> {}
unsafe impl Pod for Parameters<f64> {}
unsafe impl Pod for Ion<f32> {}
unsafe impl Pod for Ion<f64> {}
unsafe impl Pod for EndState<f32> {}
unsafe impl Pod for EndState<f64> {}
unsafe impl Pod for Sample<f32> {}
unsafe impl Pod for Sample<f64> {}

impl<F: Copy> Parameters<F> {
    pub fn map<T>(self, f: impl Fn(F) -> T + Copy) -> Parameters<T> {
        Parameters {
            dims: self.dims,
            cells: self.cells,
            degrees: self.degrees,
            flags: self.flags,
            spacing: self.spacing.map(f),
            axes: self.axes.map(|a| a.map(f)),
            bounds: self.bounds.map(|a| a.map(f)),
            detector: self.detector.map(f),
            stop_plane: self.stop_plane.map(f),
            integration: self.integration.map(f),
            tolerance: self.tolerance.map(f),
            counts: self.counts,
            padding: self.padding,
            voltages: self.voltages.map(|a| a.map(f)),
            origin: self.origin.map(f),
            magnetic: self.magnetic.map(f),
            collision: self.collision.map(f),
            gas: self.gas.map(f),
            collision_limits: self.collision_limits.map(f),
            step_limits: self.step_limits.map(f),
            physics: self.physics,
            physics_padding: self.physics_padding,
        }
    }
}

impl<F: Copy> Ion<F> {
    pub fn map<T>(self, f: impl Fn(F) -> T + Copy) -> Ion<T> {
        Ion {
            cell: self.cell,
            padding0: self.padding0,
            offset: self.offset.map(f),
            velocity: self.velocity.map(f),
            metadata: self.metadata.map(f),
            outcome: self.outcome,
            padding1: self.padding1,
            time: self.time.map(f),
            progress: self.progress.map(f),
        }
    }
}

impl<F: Copy> EndState<F> {
    pub fn map<T>(self, f: impl Fn(F) -> T + Copy) -> EndState<T> {
        EndState {
            cell: self.cell,
            outcome: self.outcome,
            offset: self.offset.map(f),
            velocity: self.velocity.map(f),
        }
    }
}

impl<F: Copy> Sample<F> {
    pub fn map<T>(self, f: impl Fn(F) -> T + Copy) -> Sample<T> {
        Sample {
            position: self.position.map(f),
            velocity: self.velocity.map(f),
        }
    }
}

pub(super) fn parameters_bytes(values: &[Parameters], wide: bool) -> Vec<u8> {
    if wide {
        bytemuck::cast_slice(values).to_vec()
    } else {
        bytemuck::cast_slice(
            &values
                .iter()
                .map(|p| p.map(|v| v as f32))
                .collect::<Vec<_>>(),
        )
        .to_vec()
    }
}

fn resident_parameters(source: &str) -> String {
    let workbench = source.contains("const WORKBENCH: bool = true;");
    if !workbench {
        return source
            .replace("var<private> p: Parameters;", "")
            .replace("p = root;", "")
            .replace("p=root;", "")
            .replace("p = instances[i];", "")
            .replace("p=instances[i];", "")
            .replace("p.", "root.");
    }
    // Avoid copying the whole 960-byte parameter block when selecting an
    // instance. Load only the requested vector.
    let mut source = source
        .replace(
            "var<private> p: Parameters;",
            "var<private> current_instance: u32;",
        )
        .replace("p = root;", "current_instance = 0xffffffffu;")
        .replace("p=root;", "current_instance = 0xffffffffu;")
        .replace("p = instances[i];", "current_instance = i;")
        .replace("p=instances[i];", "current_instance = i;");
    for (field, ty) in [
        ("dims", "u32"),
        ("cells", "u32"),
        ("degrees", "u32"),
        ("flags", "u32"),
        ("counts", "u32"),
        ("padding", "u32"),
        ("spacing", "f32"),
        ("origin", "f32"),
        ("magnetic", "f32"),
        ("detector", "f32"),
        ("stop_plane", "f32"),
        ("integration", "f32"),
        ("tolerance", "f32"),
        ("axes[0]", "f32"),
        ("axes[1]", "f32"),
        ("axes[2]", "f32"),
        ("bounds[0]", "f32"),
        ("bounds[1]", "f32"),
        ("voltages[b]", "f32"),
    ] {
        let ty = if ty == "f32" && source.contains("spacing: vec4<f64>") {
            "f64"
        } else {
            ty
        };
        let name = field.replace('[', "_").replace(']', "");
        let arg = if field == "voltages[b]" { "b: u32" } else { "" };
        let call = if arg.is_empty() { "" } else { "b" };
        source = source.replace(&format!("p.{field}"), &format!("load_{name}({call})"));
        source.push_str(&format!(
            "\nfn load_{name}({arg}) -> vec4<{ty}> {{
                if current_instance == 0xffffffffu {{ return root.{field}; }}
                return instances[current_instance].{field};
            }}\n"
        ));
    }
    source
}

fn unroll_horner(source: &str) -> String {
    // The general loop remains for lower-degree cells. Keep the same Horner
    // operations and order; only remove the six-coefficient loop's indexing
    // and control overhead. Both precisions preserve their arithmetic order.
    let original = "                for (var x = i32(effective.x) - 1; x >= 0; x--) {
                    d = d * t.x + vx;
                    vx = vx * t.x + coefficient(start + u32((z * i32(degrees.y) + y) * i32(degrees.x) + x));
                }
";
    let mut unrolled = String::from("                if effective.x == 6u {\n");
    for x in (0..6).rev() {
        unrolled.push_str(&format!(
            "                    d = d * t.x + vx;\n\
             vx = vx * t.x + coefficient(start + u32((z * i32(degrees.y) + y) * i32(degrees.x) + {x}));\n"
        ));
    }
    unrolled.push_str("                } else {\n");
    unrolled.push_str(original);
    unrolled.push_str("                }\n");
    source.replace(original, &unrolled)
}

pub(super) fn shader(source: &str, wide: bool) -> String {
    let source = if source.contains("const SURFACE_SDF: bool = true;") {
        source.replace("// SDF_FUNCTIONS", include_str!("sdf_common.wgsl"))
    } else {
        // Naga validates resource usage even behind a constant-false branch.
        // Keep the default pipeline's original nine-binding contract intact.
        let start = source.find("// SDF coordinates").unwrap();
        let end = source
            .find("// WGSL permits floating-point reassociation;")
            .unwrap();
        format!("{}{}", &source[..start], &source[end..])
            .replace("const SURFACE_SDF: bool = false;\nconst SURFACE_SDF_REPLACE: bool = false;\n", "")
            .replace("// SDF_FUNCTIONS\n@group(0) @binding(9) var<storage, read> sdf_headers: array<sdf_Volume>;\n@group(0) @binding(10) var<storage, read> sdf_samples: array<f32>;\n\n", "")
            .replace("return Event(0.0,select((packed>>12u)&1u,0u,SURFACE_SDF_REPLACE),index);", "return Event(0.0,(packed>>12u)&1u,index);")
            .replace("return Event(0.0, select(metal[index + select(0u, p.counts.x, WORKBENCH)],0u,SURFACE_SDF_REPLACE), index);", "return Event(0.0, metal[index + select(0u, p.counts.x, WORKBENCH)], index);")
            .replace("fn base_status(", "fn status(")
            .replace("fn base_intersection(", "fn intersection(")
    };
    let mut physics = include_str!("physics.wgsl").to_owned();
    if !source.contains("const IMPORTED_MAPS: bool = true;") {
        let start = physics.find("fn imported_local(").unwrap();
        let end = physics.find("fn coil_value(").unwrap();
        physics.replace_range(
            start..end,
            "fn imported_status(point: vec3<f32>) -> Event { return Event(0.0,0u,0u); }\n\
             fn imported_intersection(point: vec3<f32>, delta: vec3<f32>) -> Event { return Event(0.0,0u,0u); }\n\
             fn imported_field(point: vec3<f32>) -> vec3<f32> { return vec3<f32>(0.0); }\n",
        );
    }
    let source = source.replace("// PHYSICS_FUNCTIONS", &physics);
    let source = if study_is("constant") {
        source
    } else {
        source.replace(
            "96.48533215665327",
            &format!("{:.17e}", crate::ELECTRIC_ACCELERATION),
        )
    };
    let source = source.replace(
        "0.009648533215665327",
        &format!("{:.17e}", crate::MAGNETIC_FREQUENCY),
    );
    let source = unroll_horner(&source);
    if !wide {
        // A one-float remaining interval can round above the two-float clock
        // budget. Round that last step downward, then integrate its residual on
        // the next iteration instead of advancing beyond the prescribed time.
        let source = source.replace(
            "h = min(h, remaining);",
            "h = min(h, remaining);
        if h == remaining && h > 0.0 {
            let end = time_add(time, h);
            let after = (p.integration.x - end.x) + (p.tolerance.w - end.y);
            if after < 0.0 { h = bitcast<f32>(bitcast<u32>(h) - 1u); }
        }",
        );
        return resident_parameters(&source);
    }
    let start = source.find("fn close_difference(").unwrap();
    let end = source[start..].find("fn finite(").unwrap() + start;
    let source = format!(
        "{}fn time_add(time: vec2<f64>, h: f64) -> vec2<f64> {{
            return vec2<f64>(time.x + h, 0.0);
        }}\n{}\n{}",
        &source[..start],
        &source[end..],
        include_str!("native_math.wgsl"),
    );
    let source = source
        .replace(
            "return (f32(c.x>>9u)+0.5)/8388608.0;",
            "return (f32(c.x>>6u)*67108864.0+f32(c.y>>6u)+0.5)/4503599627370496.0;",
        )
        .replace(
            "if end_time == 0.0 { end_time = bitcast<f32>(0x80000001u); }\n        else { end_time = bitcast<f32>(bitcast<u32>(end_time) - select(1u, 0xffffffffu, end_time < 0.0)); }",
            "end_time -= max(abs(end_time) * 2.220446049250313e-16, 2.2250738585072014e-308);",
        )
        .replace(
            "fn finite(v: vec3<f32>) -> bool { return all((bitcast<vec3<u32>>(v)&vec3<u32>(0x7f800000u)) != vec3<u32>(0x7f800000u)); }",
            "fn finite(v: vec3<f64>) -> bool {
                let narrow = vec3<NARROW>(v);
                return all((bitcast<vec3<u32>>(narrow)&vec3<u32>(0x7f800000u)) != vec3<u32>(0x7f800000u));
            }",
        )
        .replace("bitcast<f32>(0x7fc00000u)", "f64(bitcast<NARROW>(0x7fc00000u))")
        .replace("f32", "f64")
        .replace("NARROW", "f32")
        .replace(
            "localization = 27u;",
            if study_is("localization") {
                "localization = 49u;"
            } else {
                "localization = localization_steps(length(trial.phase.d), p.tolerance.y);"
            },
        );
    // A WGSL `var x = 0.0` otherwise materializes an f32 even in a
    // double-valued function. Native Naga accepts the `lf` double suffix.
    let mut result = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit()
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
        {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || (bytes[i] == b'+' || bytes[i] == b'-') && i > start && bytes[i - 1] == b'e')
            {
                i += 1;
            }
            let token = &source[start..i];
            result.push_str(token);
            if !token.starts_with("0x") && (token.contains('.') || token.contains('e')) {
                result.push_str("lf");
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    // SPIR-V's GLSL Pow instruction is only defined for 16/32-bit scalars.
    // The adaptive controller needs only a few digits, not double arithmetic.
    let mut result = result
        .replace("pow(trial.error, -0.2lf)", "controller(trial.error)")
        .replace("cos(", "native_cos(")
        .replace("sin(", "native_sin(")
        .replace("exp(", "native_exp(")
        .replace("pow(", "native_pow(")
        .replace("log(", "native_log(")
        .replace("atan2(", "native_atan2(")
        .replace("native_native_", "native_")
        .replace("ldnative_exp(", "ldexp(")
        + "\nfn controller(error: f64) -> f64 { return f64(pow(f32(error), -0.2)); }\n\
fn localization_steps(distance: f64, tolerance: f64) -> u32 {\n\
    let bits = ceil(log2(max(f32(distance/tolerance),1.0)))+4.0;\n\
    return u32(clamp(bits,27.0,49.0));\n\
}\n";
    if study_is("position") {
        result = result.replace(
            "q = moved(q, trial.phase.d);",
            "q = moved(q, trial.phase.d); q.offset = vec3<f64>(vec3<f32>(q.offset));",
        );
    }
    if study_is("velocity") {
        result = result.replace(
            "u = trial.phase.u;",
            "u = vec3<f64>(vec3<f32>(trial.phase.u));",
        );
    }
    if study_is("phase") {
        result = result.replace(
            "let absolute_time = ion.metadata.y + (ion.metadata.z + time.x + time.y);",
            "let absolute_time = f64(f32(ion.metadata.y + (ion.metadata.z + time.x + time.y)));",
        );
    }
    if study_is("transforms") {
        let start = result.find("fn root_world(").unwrap();
        let end = result[start..].find("fn electric(").unwrap() + start;
        result.replace_range(
            start..end,
            "
fn root_world(q: Coordinate) -> vec3<f64> {
    let point = (vec3<f32>(q.cell) + vec3<f32>(q.offset)) * vec3<f32>(root.spacing.xyz);
    return vec3<f64>(vec3<f32>(root.axes[0].xyz)*point.x
        + vec3<f32>(root.axes[1].xyz)*point.y + vec3<f32>(root.axes[2].xyz)*point.z);
}
fn to_instance(point: vec3<f64>) -> Coordinate {
    let delta = vec3<f32>(point) - vec3<f32>(p.origin.xyz);
    let q = vec3<f32>(dot(vec3<f32>(p.axes[0].xyz),delta),
        dot(vec3<f32>(p.axes[1].xyz),delta),dot(vec3<f32>(p.axes[2].xyz),delta))
        / vec3<f32>(p.spacing.xyz);
    let cell = vec3<i32>(floor(q));
    return Coordinate(cell, vec3<f64>(q-vec3<f32>(cell)));
}
",
        );
    }
    if study_is("evaluation") {
        let original = include_str!("fly.wgsl");
        let start = original
            .find("    let t = vec3<f32>(q.cell - cell)")
            .unwrap();
        let end = original[start..].find("    var reflection =").unwrap() + start;
        let body = original[start..end]
            .replace(" + q.offset", " + vec3<f32>(q.offset)")
            .replace("coefficient(", "coefficient32(")
            .replace(
                "nodal_gradient(vec3<u32>(cell),t)",
                "vec3<f32>(nodal_gradient(vec3<u32>(cell),vec3<f64>(t)))",
            );
        let start = result.find("    let t = vec3<f64>(q.cell - cell)").unwrap();
        let end = result[start..].find("    var reflection =").unwrap() + start;
        result.replace_range(start..end, &body);
        result = result.replace("* gradient /", "* vec3<f64>(gradient) /");
        result.push_str("\nfn coefficient32(i: u32) -> f32 { return f32(coefficient(i)); }\n");
    }
    if study_is("tof") {
        let original = include_str!("fly.wgsl");
        let start = original.find("fn close_difference(").unwrap();
        let end = original[start..].find("fn finite(").unwrap() + start;
        result.push_str(&original[start..end].replace("fn time_add(", "fn narrow_time_add("));
        result = result.replace(
            "return vec2<f64>(time.x + h, 0.0lf);",
            "return vec2<f64>(narrow_time_add(vec2<f32>(time), f32(h)));",
        );
        result = result.replace("h = min(h, remaining);", "h = f64(f32(min(h, remaining)));");
    }
    resident_parameters(&result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_layout_has_no_implicit_padding() {
        assert_eq!(std::mem::size_of::<Parameters<f32>>(), 640);
        assert_eq!(std::mem::size_of::<Parameters<f64>>(), 1152);
        assert_eq!(std::mem::size_of::<Ion<f32>>(), 144);
        assert_eq!(std::mem::size_of::<Ion<f64>>(), 224);
        assert_eq!(std::mem::size_of::<EndState<f32>>(), 64);
        assert_eq!(std::mem::size_of::<EndState<f64>>(), 96);
        assert_eq!(std::mem::offset_of!(Ion<f64>, time), 160);
        assert_eq!(std::mem::offset_of!(Parameters<f64>, voltages), 416);
    }

    #[test]
    fn coil_staging_stride_matches_workgroup_lanes() {
        for wide in [false, true] {
            let source = shader(include_str!("fly.wgsl"), wide);
            let stride = source
                .split("const COIL_THREADS: u32 = ")
                .nth(1)
                .unwrap()
                .split('u')
                .next()
                .unwrap()
                .parse::<u32>()
                .unwrap();
            let lanes = source
                .rsplit("@compute @workgroup_size(")
                .next()
                .unwrap()
                .split(')')
                .next()
                .unwrap()
                .parse::<u32>()
                .unwrap();
            assert_eq!(stride, lanes);
        }
    }
}
