
use api::{Error, Result};
use {Format, api, SampleFormat};
use super::alsa;
use super::check_errors;
use std::ffi::CString;
use std::{mem, ptr};

pub struct Instance;

impl api::Instance for Instance {
    type PhysicalDevice = PhysicalDevice;
    type Device = Device;

    fn create(name: &str) -> Self {
        Instance
    }

    fn enumerate_physical_input_devices(&self) -> Result<Vec<Self::PhysicalDevice>> {
        unimplemented!()
    }

    fn enumerate_physical_output_devices(&self) -> Result<Vec<Self::PhysicalDevice>> {
        let mut hints = unsafe {
            // TODO: check in which situation this can fail.
            let card = -1; // -1 means all cards.
            let iface = b"pcm\0"; // Interface identification.
            let mut hints = mem::uninitialized(); // Array of device name hints.
            let res = alsa::snd_device_name_hint(card, iface.as_ptr() as *const _, &mut hints);
            if let Err(description) = check_errors(res) {
                return Err(Error::BackendSpecific { description });
            }
            hints
        };

        let mut devices = Vec::new();

        loop {
            unsafe {
                if (*hints).is_null() {
                    break;
                }

                let name = {
                    let n_ptr = alsa::snd_device_name_get_hint(*hints as *const _,
                                                               b"NAME\0".as_ptr() as *const _);
                    if !n_ptr.is_null() {
                        let bytes = CString::from_raw(n_ptr).into_bytes();
                        let string = String::from_utf8(bytes).unwrap();
                        Some(string)
                    } else {
                        None
                    }
                };

                let io = {
                    let n_ptr = alsa::snd_device_name_get_hint(*hints as *const _,
                                                               b"IOID\0".as_ptr() as *const _);
                    if !n_ptr.is_null() {
                        let bytes = CString::from_raw(n_ptr).into_bytes();
                        let string = String::from_utf8(bytes).unwrap();
                        Some(string)
                    } else {
                        None
                    }
                };

                hints = hints.offset(1);

                if let Some(io) = io {
                    if io != "Output" {
                        continue;
                    }
                }

                let name = match name {
                    Some(name) => {
                        // Ignoring the `null` device.
                        if name == "null" {
                            continue;
                        }
                        name
                    },
                    _ => continue,
                };

                // trying to open the PCM device to see if it can be opened
                let name_zeroed = CString::new(&name[..]).unwrap();

                // See if the device has an available output stream.
                let mut playback_handle = mem::uninitialized();
                let has_available_output = alsa::snd_pcm_open(
                    &mut playback_handle,
                    name_zeroed.as_ptr() as *const _,
                    alsa::SND_PCM_STREAM_PLAYBACK,
                    alsa::SND_PCM_NONBLOCK,
                ) == 0;
                if has_available_output {
                    alsa::snd_pcm_close(playback_handle);
                }

                if has_available_output {
                    devices.push(PhysicalDevice {
                        device_name: name,
                        stream: alsa::SND_PCM_STREAM_PLAYBACK,
                    });
                }
            }
        }

        Ok(devices)
    }

    fn default_physical_input_device(&self) -> Result<Option<Self::PhysicalDevice>> {
        unimplemented!()
    }

    fn default_physical_output_device(&self) -> Result<Option<Self::PhysicalDevice>> {
        Ok(Some(PhysicalDevice {
            device_name: "default".into(),
            stream: alsa::SND_PCM_STREAM_PLAYBACK,
        }))
    }

    fn create_device(
        &self,
        physical_device: &Self::PhysicalDevice,
        format: Format,
    ) -> Result<Self::Device> {
        unsafe {
            let name = CString::new(physical_device.device_name.clone()).expect("unable to clone device");

            let mut pcm = ptr::null_mut();
            match alsa::snd_pcm_open(
                &mut pcm,
                name.as_ptr(),
                physical_device.stream,
                alsa::SND_PCM_NONBLOCK,
            ) {
                -16 /* determined empirically */ => return Err(Error::DeviceLost),
                e => if let Err(description) = check_errors(e) {
                    return Err(Error::BackendSpecific { description });
                }
            }

            let hw_params = HwParams::alloc();
            set_hw_params_from_format(pcm, &hw_params, &format)
                .map_err(|description| Error::BackendSpecific { description })?;

            set_sw_params_from_format(pcm, &format)
                .map_err(|description| Error::BackendSpecific { description })?;

            let format_layout = match format.data_type {
                SampleFormat::I16 => std::alloc::Layout::new::<i16>(),
                SampleFormat::U16 => std::alloc::Layout::new::<u16>(),
                SampleFormat::F32 => std::alloc::Layout::new::<f32>(),
            };
            let buffer_frames = 1024; // TODO
            let buffer_layout = std::alloc::Layout::from_size_align(
                format.channels as usize * buffer_frames * format_layout.size(),
                format_layout.align()
            ).unwrap(); // aligned and size within bounds

            Ok(Device { pcm, buffer_layout, buffer_frames })
        }
    }
}

pub struct PhysicalDevice {
    device_name: String,
    stream: alsa::snd_pcm_stream_t,
}

impl api::PhysicalDevice for PhysicalDevice {
    fn properties(&self) -> api::PhysicalDeviceProperties {
        api::PhysicalDeviceProperties {
            device_name: self.device_name.clone(),
        }
    }
}

pub struct Device {
    pcm: *mut alsa::snd_pcm_t,
    buffer_layout: std::alloc::Layout,
    buffer_frames: api::Frames,
}

impl api::Device for Device {
    type OutputStream = OutputStream;
    type InputStream = InputStream;

    fn properties(&self) -> api::DeviceProperties {
        unimplemented!()
    }

    fn output_stream(&self) -> Result<Self::OutputStream> {
        unsafe {
            if let Err(desc) = check_errors(alsa::snd_pcm_prepare(self.pcm)) {
                let description = format!("could not get playback handle: {}", desc);
                return Err(Error::BackendSpecific { description });
            }
        }

        Ok(OutputStream {
            pcm: self.pcm,
            buffer: unsafe { std::alloc::alloc(self.buffer_layout) as _ },
            buffer_frames: self.buffer_frames,
        })
    }

    fn async_output_stream(&self) -> Result<Self::OutputStream> {
        unimplemented!()
    }

    fn input_stream(&self) -> Result<Self::InputStream> {
        unimplemented!()
    }

    fn async_input_stream(&self) -> Result<Self::InputStream> {
        unimplemented!()
    }
}

pub struct OutputStream {
    pcm: *mut alsa::snd_pcm_t,
    buffer: *mut u8, // aligned to the format
    buffer_frames: api::Frames,
}

impl api::OutputStream for OutputStream {
    fn start(&self) {
        // TODO:
    }

    fn stop(&self) {
        // TODO:
    }

    fn acquire_buffer(&self, timeout_ms: u32) -> (*mut (), api::Frames) {
        unsafe {
            let _ = alsa::snd_pcm_wait(self.pcm, timeout_ms as _); // TODO: return value
            (self.buffer as _, self.buffer_frames)
        }
    }

    fn release_buffer(&self, num_frames: api::Frames) {
        unsafe {
            let _ = alsa::snd_pcm_writei(self.pcm, self.buffer as _, num_frames as _);
        }
    }
}

pub struct InputStream;

impl api::InputStream for InputStream {

}


unsafe fn set_hw_params_from_format(
    pcm_handle: *mut alsa::snd_pcm_t,
    hw_params: &HwParams,
    format: &Format,
) -> std::result::Result<(), String> {
    if let Err(e) = check_errors(alsa::snd_pcm_hw_params_any(pcm_handle, hw_params.0)) {
        return Err(format!("errors on pcm handle: {}", e));
    }
    if let Err(e) = check_errors(alsa::snd_pcm_hw_params_set_access(pcm_handle,
                                                    hw_params.0,
                                                    alsa::SND_PCM_ACCESS_RW_INTERLEAVED)) {
        return Err(format!("handle not acessible: {}", e));
    }

    let data_type = if cfg!(target_endian = "big") {
        match format.data_type {
            SampleFormat::I16 => alsa::SND_PCM_FORMAT_S16_BE,
            SampleFormat::U16 => alsa::SND_PCM_FORMAT_U16_BE,
            SampleFormat::F32 => alsa::SND_PCM_FORMAT_FLOAT_BE,
        }
    } else {
        match format.data_type {
            SampleFormat::I16 => alsa::SND_PCM_FORMAT_S16_LE,
            SampleFormat::U16 => alsa::SND_PCM_FORMAT_U16_LE,
            SampleFormat::F32 => alsa::SND_PCM_FORMAT_FLOAT_LE,
        }
    };

    if let Err(e) = check_errors(alsa::snd_pcm_hw_params_set_format(pcm_handle,
                                                    hw_params.0,
                                                    data_type)) {
        return Err(format!("format could not be set: {}", e));
    }
    if let Err(e) = check_errors(alsa::snd_pcm_hw_params_set_rate(pcm_handle,
                                                  hw_params.0,
                                                  format.sample_rate.0 as libc::c_uint,
                                                  0)) {
        return Err(format!("sample rate could not be set: {}", e));
    }
    if let Err(e) = check_errors(alsa::snd_pcm_hw_params_set_channels(pcm_handle,
                                                      hw_params.0,
                                                      format.channels as
                                                                      libc::c_uint)) {
        return Err(format!("channel count could not be set: {}", e));
    }

    // TODO: Review this. 200ms seems arbitrary...
    let mut max_buffer_size = format.sample_rate.0 as alsa::snd_pcm_uframes_t /
        format.channels as alsa::snd_pcm_uframes_t /
        5; // 200ms of buffer
    if let Err(e) = check_errors(alsa::snd_pcm_hw_params_set_buffer_size_max(pcm_handle,
                                                             hw_params.0,
                                                             &mut max_buffer_size))
    {
        return Err(format!("max buffer size could not be set: {}", e));
    }

    if let Err(e) = check_errors(alsa::snd_pcm_hw_params(pcm_handle, hw_params.0)) {
        return Err(format!("hardware params could not be set: {}", e));
    }

    Ok(())
}

unsafe fn set_sw_params_from_format(
    pcm_handle: *mut alsa::snd_pcm_t,
    format: &Format,
) -> std::result::Result<(usize, usize), String>
{
    let mut sw_params = mem::uninitialized(); // TODO: RAII
    if let Err(e) = check_errors(alsa::snd_pcm_sw_params_malloc(&mut sw_params)) {
        return Err(format!("snd_pcm_sw_params_malloc failed: {}", e));
    }
    if let Err(e) = check_errors(alsa::snd_pcm_sw_params_current(pcm_handle, sw_params)) {
        return Err(format!("snd_pcm_sw_params_current failed: {}", e));
    }
    if let Err(e) = check_errors(alsa::snd_pcm_sw_params_set_start_threshold(pcm_handle, sw_params, 0)) {
        return Err(format!("snd_pcm_sw_params_set_start_threshold failed: {}", e));
    }

    let (buffer_len, period_len) = {
        let mut buffer = mem::uninitialized();
        let mut period = mem::uninitialized();
        if let Err(e) = check_errors(alsa::snd_pcm_get_params(pcm_handle, &mut buffer, &mut period)) {
            return Err(format!("failed to initialize buffer: {}", e));
        }
        if buffer == 0 {
            return Err(format!("initialization resulted in a null buffer"));
        }
        if let Err(e) = check_errors(alsa::snd_pcm_sw_params_set_avail_min(pcm_handle, sw_params, period)) {
            return Err(format!("snd_pcm_sw_params_set_avail_min failed: {}", e));
        }
        let buffer = buffer as usize * format.channels as usize;
        let period = period as usize * format.channels as usize;
        (buffer, period)
    };

    if let Err(e) = check_errors(alsa::snd_pcm_sw_params(pcm_handle, sw_params)) {
        return Err(format!("snd_pcm_sw_params failed: {}", e));
    }

    alsa::snd_pcm_sw_params_free(sw_params);
    Ok((buffer_len, period_len))
}

/// Wrapper around `hw_params`.
struct HwParams(*mut alsa::snd_pcm_hw_params_t);

impl HwParams {
    pub fn alloc() -> HwParams {
        unsafe {
            let mut hw_params = mem::uninitialized();
            check_errors(alsa::snd_pcm_hw_params_malloc(&mut hw_params))
                .expect("unable to get hardware parameters");
            HwParams(hw_params)
        }
    }
}

impl Drop for HwParams {
    fn drop(&mut self) {
        unsafe {
            alsa::snd_pcm_hw_params_free(self.0);
        }
    }
}