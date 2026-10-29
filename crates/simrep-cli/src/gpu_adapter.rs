//! Explicit adapter selection without environment mutation or an MPI runtime.
use simrep_fly::gpu::{Backends, Gpu};
use simrep_solve::gpu::GpuResult;

pub async fn selected(backends: Backends, selector: &str) -> GpuResult<Option<Gpu>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = instance.enumerate_adapters(backends).await;
    let adapter = if let Ok(index) = selector.parse::<usize>() {
        adapters
            .into_iter()
            .nth(index)
            .ok_or_else(|| format!("GPU adapter index {index} not found for {backends:?}"))?
    } else {
        let name = selector.to_lowercase();
        let mut matches = adapters
            .into_iter()
            .filter(|a| a.get_info().name.to_lowercase().contains(&name));
        let adapter = matches
            .next()
            .ok_or_else(|| format!("GPU adapter name {selector:?} not found for {backends:?}"))?;
        if matches.next().is_some() {
            return Err(format!("GPU adapter name {selector:?} is ambiguous; use an index").into());
        }
        adapter
    };
    let info = adapter.get_info();
    crate::require_hardware_gpu(&info)?;
    let features = adapter.features();
    let limits = adapter.limits();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("simrep selected flyer"),
            required_features: features
                & (wgpu::Features::SHADER_F64 | wgpu::Features::TIMESTAMP_QUERY),
            required_limits: wgpu::Limits {
                max_storage_buffers_per_shader_stage: limits
                    .max_storage_buffers_per_shader_stage
                    .min(10),
                max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
                max_buffer_size: limits.max_buffer_size,
                max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
                ..Default::default()
            },
            ..Default::default()
        })
        .await?;
    Ok(Some(Gpu::from_device(device, queue, info).await?))
}
