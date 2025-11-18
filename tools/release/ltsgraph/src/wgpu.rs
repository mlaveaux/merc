use merc_utilities::MCRL3Error;
use wgpu::Instance;

pub async fn init_wgpu() -> Result<(wgpu::Device, wgpu::Queue), MCRL3Error> {
    let instance = Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|e| MCRL3Error::from(format!("Cannot find a suitable adapter: {e}")))?;

    adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .map_err(|e| MCRL3Error::from(format!("Failed to create device: {e}")))
}
