//! Opt-in SDF selection, shared by the CLI and Python flight frontends.
use simrep_fly::gpu::{DeviceType, Gpu};
use simrep_solve::gpu::GpuResult;

// None means never select GPU. Any number added here must be justified by a
// compute-only record in docs/dev/sdf-auto-crossover.md (precision, divisor,
// hardware), following docs/dev/verification.md. No crossover is shipped yet.
const CROSSOVERS: [(&str, u32, Option<usize>); 4] = [
    ("f64", 2, None),
    ("f64", 4, None),
    ("f32", 2, None),
    ("f32", 4, None),
];

#[derive(Clone, Copy)]
pub(crate) struct Policy<'a> {
    pub device: &'a str,
    pub vulkan: bool,
    pub surfaces: bool,
    pub physics_supported: bool,
    pub ions: usize,
    pub divisor: u32,
}

impl Policy<'_> {
    fn cpu_reason(self, override_ions: Option<usize>) -> Option<&'static str> {
        if self.device == "cpu" {
            Some("explicit --device cpu")
        } else if !self.surfaces {
            Some("no collision surfaces")
        } else if !self.physics_supported {
            Some("physics unsupported on GPU SDF")
        } else if !self.vulkan {
            Some("GPU SDF requires Vulkan")
        } else if self.crossover("f64", override_ions).is_none()
            && self.crossover("f32", override_ions).is_none()
        {
            Some("no measured GPU SDF crossover")
        } else {
            None
        }
    }

    fn crossover(self, precision: &str, override_ions: Option<usize>) -> Option<usize> {
        CROSSOVERS.iter().find_map(|&(p, divisor, measured)| {
            (p == precision && divisor == self.divisor)
                .then_some(override_ions.or(measured))
                .flatten()
        })
    }

    fn resolve(
        self,
        precision: &str,
        hardware_vulkan: bool,
        override_ions: Option<usize>,
    ) -> (bool, String) {
        if let Some(reason) = self.cpu_reason(override_ions) {
            return (false, cpu(reason));
        }
        if !hardware_vulkan {
            return (
                false,
                cpu("no hardware Vulkan adapter (software adapters refused)"),
            );
        }
        let Some(threshold) = self.crossover(precision, override_ions) else {
            return (false, cpu("no measured GPU SDF crossover"));
        };
        let use_gpu = self.ions >= threshold;
        (
            use_gpu,
            format!(
                "auto: {} ({} ions {} crossover {threshold} for {precision} N={})",
                if use_gpu { "GPU sdf" } else { "CPU exact" },
                self.ions,
                if use_gpu { ">=" } else { "<" },
                self.divisor,
            ),
        )
    }
}

fn cpu(reason: &str) -> String {
    format!("auto: CPU exact ({reason})")
}

/// Probe only after the user opts in and a usable crossover exists. In
/// particular, the default all-None table never initializes a GPU.
pub(crate) fn select(
    policy: Policy<'_>,
    probe: impl FnOnce() -> GpuResult<Option<Gpu>>,
) -> GpuResult<(Option<Gpu>, String)> {
    let override_ions = std::env::var("SIMREP_SDF_GPU_CROSSOVER")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "SIMREP_SDF_GPU_CROSSOVER must be a nonnegative ion count")
        })
        .transpose()?;
    if let Some(reason) = policy.cpu_reason(override_ions) {
        return Ok((None, cpu(reason)));
    }
    let Some(gpu) = probe()? else {
        return Ok((None, cpu("no hardware Vulkan adapter available")));
    };
    let (use_gpu, reason) = policy.resolve(
        if gpu.has_f64() { "f64" } else { "f32" },
        gpu.info.device_type != DeviceType::Cpu && gpu.info.backend == wgpu::Backend::Vulkan,
        override_ions,
    );
    if use_gpu && gpu.device().limits().max_storage_buffers_per_shader_stage < 10 {
        return Ok((
            None,
            cpu("GPU SDF requires 10 storage buffers per shader stage"),
        ));
    }
    Ok((use_gpu.then_some(gpu), reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy<'static> {
        Policy {
            device: "auto",
            vulkan: true,
            surfaces: true,
            physics_supported: true,
            ions: 240_000,
            divisor: 2,
        }
    }

    #[test]
    fn every_precision_and_divisor_defaults_to_no_crossover() {
        for precision in ["f64", "f32"] {
            for divisor in [2, 4] {
                let policy = Policy {
                    divisor,
                    ..policy()
                };
                assert_eq!(policy.crossover(precision, None), None);
                let (gpu, reason) = policy.resolve(precision, true, None);
                assert!(!gpu);
                assert_eq!(reason, "auto: CPU exact (no measured GPU SDF crossover)");
            }
        }
    }

    #[test]
    fn every_selection_gate_is_required_even_with_an_override() {
        for policy in [
            Policy {
                device: "cpu",
                ..policy()
            },
            Policy {
                surfaces: false,
                ..policy()
            },
            Policy {
                physics_supported: false,
                ..policy()
            },
            Policy {
                vulkan: false,
                ..policy()
            },
            Policy {
                divisor: 1,
                ..policy()
            },
        ] {
            assert!(!policy.resolve("f64", true, Some(1)).0);
        }
        assert!(!policy().resolve("f64", false, Some(1)).0);
    }

    #[test]
    fn precision_divisor_threshold_and_device_overrides() {
        for device in ["auto", "gpu"] {
            for precision in ["f64", "f32"] {
                for divisor in [2, 4] {
                    let p = Policy {
                        device,
                        divisor,
                        ions: 119_999,
                        ..policy()
                    };
                    assert!(!p.resolve(precision, true, Some(120_000)).0);
                    assert!(
                        Policy { ions: 120_000, ..p }
                            .resolve(precision, true, Some(120_000))
                            .0
                    );
                    assert!(
                        Policy { ions: 120_001, ..p }
                            .resolve(precision, true, Some(120_000))
                            .0
                    );
                }
            }
        }
        assert_eq!(
            policy().resolve("f32", true, Some(120_000)).1,
            "auto: GPU sdf (240000 ions >= crossover 120000 for f32 N=2)"
        );
    }
}
