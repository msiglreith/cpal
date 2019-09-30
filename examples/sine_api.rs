extern crate cpal;

use cpal::api::{PhysicalDevice, Instance, Device, OutputStream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = cpal::wasapi::new::Instance::create("cpal - sine");

    let output_device = instance.default_physical_output_device()?.unwrap();

    let device = instance.create_device(&output_device, cpal::Format {
        channels: 2,
        data_type: cpal::SampleFormat::F32,
        sample_rate: cpal::SampleRate(48_000),
    })?;

    let frequency = 440.0;
    let sample_rate = 48_000 as f32;
    let num_channels = 2;
    let cycle_step = frequency / sample_rate;
    let mut cycle = 0.0;


    let mut stream = device.output_stream()?;
    stream.start();

    loop {
        let (raw_buffer, num_frames) = stream.acquire_buffer(!0);
        let buffer = unsafe {
            std::slice::from_raw_parts_mut(
                raw_buffer as *mut f32,
                num_frames as usize * num_channels,
            )
        };

        for dt in 0..num_frames {
            let phase = 2.0 * std::f32::consts::PI * cycle;
            let sample = phase.sin() * 0.5;

            buffer[num_channels * dt as usize] = sample;
            buffer[num_channels * dt as usize + 1] = sample;

            cycle = (cycle + cycle_step) % 1.0;
        }

        stream.release_buffer(num_frames);
    }

    Ok(())
}