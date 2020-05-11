use std::convert::TryInto;

use futures::executor::block_on;
use zerocopy::AsBytes;

fn main() {
    let (device, queue) = block_on(gpu());

    // iterations and entries were chosen fairly arbitrarily and aren't meant to be at the boundary between
    // working and not working.

    let (iterations, entries) = {
        // debug: works consistently
        // release: works consistently
        (10, 200_000)

        // debug: works consistently
        // release: fails consistently
        // (100, 200_000)

        // debug: works inconsistently
        // release: fails consistently
        // (1000, 200_000)

        // debug: works consistently
        // release: works consistently
        // (10, 1_000_000)

        // debug: works consistently
        // release: fails consistently
        // (75, 1_000_000)

        // debug: fails consistently
        // release: fails consistently
        // (100, 1_000_000)

    };

    let data = {
        let mut v = Vec::with_capacity(entries);
        for _ in 0..entries {
            v.push(0);
        }
        v
    };
    let data_size = (std::mem::size_of::<u32>() * entries) as wgpu::BufferAddress;

    let (staging, storage) = make_buffer(&device, &data);

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        bindings: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStage::COMPUTE,
            ty: wgpu::BindingType::StorageBuffer {
                dynamic: false,
                readonly: false,
            },
        }],
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        bindings: &[wgpu::Binding {
            binding: 0,
            resource: wgpu::BindingResource::Buffer {
                buffer: &storage,
                range: 0..data_size,
            },
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        bind_group_layouts: &[&bind_group_layout],
    });

    let cs_module = device.create_shader_module(&shader());

    let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        layout: &pipeline_layout,
        compute_stage: wgpu::ProgrammableStageDescriptor {
            module: &cs_module,
            entry_point: "main",
        },
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

    encoder.copy_buffer_to_buffer(&staging, 0, &storage, 0, data_size);

    queue.submit(&[encoder.finish()]);

    for _ in 0..iterations {
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let mut cpass = encoder.begin_compute_pass();
            cpass.set_pipeline(&compute_pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            cpass.dispatch(data.len() as u32, 1, 1);
        }

        queue.submit(&[encoder.finish()]);
    }

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

    encoder.copy_buffer_to_buffer(&storage, 0, &staging, 0, data_size);

    queue.submit(&[encoder.finish()]);

    println!("reading data back, {} bytes", data_size);
    let read_future = staging.map_read(0, data_size);

    println!("waiting");
    device.poll(wgpu::Maintain::Wait);

    if let Ok(mapping) = block_on(read_future) {
        let output: Vec<u32> = mapping
            .as_slice()
            .chunks_exact(4)
            .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
            .collect();
        println!(
            "received data, {} bytes",
            output.len() * std::mem::size_of::<u32>()
        );
    }
}

async fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let adapter: wgpu::Adapter = wgpu::Adapter::request(
        &wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::Default,
            compatible_surface: None,
        },
        wgpu::BackendBit::PRIMARY,
    )
    .await
    .expect("error creating adapter");

    adapter
        .request_device(&wgpu::DeviceDescriptor {
            extensions: wgpu::Extensions {
                anisotropic_filtering: false,
            },
            limits: wgpu::Limits::default(),
        })
        .await
}

fn make_buffer(device: &wgpu::Device, data: &[u32]) -> (wgpu::Buffer, wgpu::Buffer) {
    let staging_buffer = device.create_buffer_with_data(
        data.as_bytes(),
        wgpu::BufferUsage::MAP_READ | wgpu::BufferUsage::COPY_DST | wgpu::BufferUsage::COPY_SRC,
    );

    let storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (data.len() * std::mem::size_of::<u32>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsage::STORAGE
            | wgpu::BufferUsage::COPY_DST
            | wgpu::BufferUsage::COPY_SRC,
    });

    (staging_buffer, storage_buffer)
}

fn shader() -> Vec<u32> {
    let cs = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shader/", "shader.comp.spv"));
    wgpu::read_spirv(std::io::Cursor::new(&cs[..])).expect("error reading shader")
}
