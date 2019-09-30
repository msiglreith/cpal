use std::ffi::OsString;
use std::ops::{Deref, Drop};
use std::os::windows::ffi::OsStringExt;
use std::{mem, ptr, slice};

use super::com::{self, WeakPtr};
use super::winapi::shared::devpkey::*;
use super::winapi::shared::ksmedia;
use super::winapi::shared::mmreg::*;
use super::winapi::um::audioclient::*;
use super::winapi::um::audiosessiontypes::*;
use super::winapi::um::combaseapi::*;
use super::winapi::um::coml2api::STGM_READ;
use super::winapi::um::mmdeviceapi::*;
use super::winapi::um::propsys::*;
use super::winapi::um::synchapi;
use super::winapi::um::winnt;
use super::winapi::Interface;
use super::{check_result, check_result_backend_specific};
use api::{Error, Result};
use {api, Format, SampleFormat};

type InstanceRaw = WeakPtr<IMMDeviceEnumerator>;
pub struct Instance(InstanceRaw);

impl Deref for Instance {
    type Target = InstanceRaw;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { self.Release() };
    }
}

impl Instance {
    unsafe fn enumerate_physical_devices(&self, ty: EDataFlow) -> Result<Vec<PhysicalDevice>> {
        type DeviceCollection = WeakPtr<IMMDeviceCollection>;

        let collection = {
            let mut collection = DeviceCollection::null();
            // can fail because of wrong parameters (should never happen) or out of memory
            check_result_backend_specific(self.EnumAudioEndpoints(
                ty,
                DEVICE_STATE_ACTIVE,
                collection.mut_void() as *mut _,
            ))?;
            collection
        };

        let num_items = {
            let mut num = 0;
            // can fail if the parameter is null, which should never happen
            check_result_backend_specific(collection.GetCount(&mut num))?;
            num
        };

        let physical_devices = (0..num_items)
            .map(|i| {
                let mut device = PhysicalDeviceRaw::null();
                collection.Item(i, device.mut_void() as *mut _);
                PhysicalDevice(device)
            })
            .collect();

        // cleanup
        collection.Release();

        Ok(physical_devices)
    }

    unsafe fn default_physical_device(&self, ty: EDataFlow) -> Result<Option<PhysicalDevice>> {
        let mut device = PhysicalDeviceRaw::null();
        let hresult = self.GetDefaultAudioEndpoint(ty, eConsole, device.mut_void() as _);
        if let Err(_err) = check_result(hresult) {
            return Ok(None); // TODO: check specifically for `E_NOTFOUND`, and panic otherwise
        }
        Ok(Some(PhysicalDevice(device)))
    }
}

impl api::Instance for Instance {
    type PhysicalDevice = PhysicalDevice;
    type Device = Device;

    fn create(_: &str) -> Self {
        // COM initialization is thread local, but we only need to have COM initialized in the
        // thread we create the objects in
        com::com_initialized();

        let mut instance = InstanceRaw::null();
        let hresult = unsafe {
            CoCreateInstance(
                &CLSID_MMDeviceEnumerator,
                ptr::null_mut(),
                CLSCTX_ALL,
                &IMMDeviceEnumerator::uuidof(),
                instance.mut_void(),
            )
        };
        check_result(hresult).unwrap();

        Instance(instance)
    }

    fn enumerate_physical_input_devices(&self) -> Result<Vec<Self::PhysicalDevice>> {
        unsafe { self.enumerate_physical_devices(eCapture) }
    }

    fn enumerate_physical_output_devices(&self) -> Result<Vec<Self::PhysicalDevice>> {
        unsafe { self.enumerate_physical_devices(eRender) }
    }

    fn default_physical_input_device(&self) -> Result<Option<Self::PhysicalDevice>> {
        unsafe { self.default_physical_device(eCapture) }
    }

    fn default_physical_output_device(&self) -> Result<Option<Self::PhysicalDevice>> {
        unsafe { self.default_physical_device(eRender) }
    }

    fn create_device(
        &self,
        physical_device: &Self::PhysicalDevice,
        format: Format,
    ) -> Result<Self::Device> {
        let audio_client = {
            let mut audio_client = WeakPtr::<IAudioClient>::null();
            let hresult = unsafe {
                physical_device.Activate(
                    &IAudioClient::uuidof(),
                    CLSCTX_ALL,
                    ptr::null_mut(),
                    audio_client.mut_void() as *mut _,
                )
            };
            // can fail if the device has been disconnected since we enumerated it.
            check_result(hresult)?;
            assert!(!audio_client.is_null());

            audio_client
        };

        let mix_format = map_format(&format).unwrap(); // TODO: error

        // TODO: check format support
        //
        // Ensure the format is supported.
        // match super::device::is_format_supported(audio_client, &format_attempt.Format) {
        //     Ok(false) => return Err(BuildStreamError::FormatNotSupported),
        //     Err(_) => return Err(BuildStreamError::DeviceNotAvailable),
        //     _ => (),
        // }

        let share_mode = AUDCLNT_SHAREMODE_SHARED;
        let hresult = unsafe {
            audio_client.Initialize(
                share_mode,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                0,
                0,
                &mix_format as *const _ as _,
                ptr::null(),
            )
        };

        match check_result(hresult) {
            Err(ref e) if e.raw_os_error() == Some(AUDCLNT_E_DEVICE_INVALIDATED) => {
                unsafe { audio_client.Release() };
                return Err(Error::DeviceLost);
            }
            Err(e) => {
                unsafe { audio_client.Release() };
                let description = format!("{}", e);
                return Err(Error::BackendSpecific { description });
            }
            Ok(()) => (),
        };

        let fence = unsafe { Fence::create(false, false) };
        let hresult = unsafe { audio_client.SetEventHandle(fence.0) };
        if let Err(e) = check_result(hresult) {
            unsafe {
                audio_client.Release();
            }
            let description = format!("failed to call SetEventHandle: {}", e);
            return Err(Error::BackendSpecific { description });
        }

        Ok(Device {
            audio_client,
            fence,
        })
    }
}

type PhysicalDeviceRaw = WeakPtr<IMMDevice>;
pub struct PhysicalDevice(PhysicalDeviceRaw);

impl Deref for PhysicalDevice {
    type Target = PhysicalDeviceRaw;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for PhysicalDevice {
    fn drop(&mut self) {
        unsafe { self.Release() };
    }
}

impl api::PhysicalDevice for PhysicalDevice {
    fn properties(&self) -> api::PhysicalDeviceProperties {
        type PropertyStore = WeakPtr<IPropertyStore>;

        let mut store = PropertyStore::null();
        unsafe { self.OpenPropertyStore(STGM_READ, store.mut_void() as *mut _) };

        let device_name = unsafe {
            let mut value = mem::MaybeUninit::uninit();
            store.GetValue(
                &DEVPKEY_Device_FriendlyName as *const _ as *const _,
                value.as_mut_ptr(),
            );
            let value = value.assume_init();
            let os_str = *value.data.pwszVal();
            let mut len = 0;
            while *os_str.offset(len) != 0 {
                len += 1;
            }
            let name: OsString = OsStringExt::from_wide(slice::from_raw_parts(os_str, len as _));
            name.into_string().unwrap()
        };

        api::PhysicalDeviceProperties { device_name }
    }
}

pub struct Device {
    audio_client: WeakPtr<IAudioClient>,
    fence: Fence,
}

impl api::Device for Device {
    type OutputStream = OutputStream;
    type InputStream = InputStream;

    fn properties(&self) -> api::DeviceProperties {
        api::DeviceProperties {}
    }

    fn output_stream(&self) -> Result<Self::OutputStream> {
        // TODO: check device type

        let mut audio_render_client = WeakPtr::<IAudioRenderClient>::null();
        let hresult = unsafe {
            self.audio_client.GetService(
                &IAudioRenderClient::uuidof(),
                audio_render_client.mut_void() as *mut _,
            )
        };

        // TODO: error

        let buffer_size = {
            let mut size = 0;
            let hresult = unsafe { self.audio_client.GetBufferSize(&mut size) }; // TODO: error
            size
        };

        Ok(OutputStream {
            audio_client: self.audio_client,
            audio_render_client,
            buffer_size,
            fence: self.fence,
        })
    }

    fn async_output_stream(&self) -> Result<Self::OutputStream> {
        Err(Error::Unsupported)
    }

    fn input_stream(&self) -> Result<Self::InputStream> {
        unimplemented!()
    }

    fn async_input_stream(&self) -> Result<Self::InputStream> {
        Err(Error::Unsupported)
    }
}

pub struct OutputStream {
    audio_client: WeakPtr<IAudioClient>,
    audio_render_client: WeakPtr<IAudioRenderClient>,
    buffer_size: u32,
    fence: Fence,
}

impl api::OutputStream for OutputStream {
    fn start(&self) {
        unsafe {
            self.audio_client.Start();
        }
    }

    fn stop(&self) {
        unsafe {
            self.audio_client.Stop();
        }
    }

    fn acquire_buffer(&self, timeout_ms: u32) -> (*mut (), api::Frames) {
        unsafe {
            self.fence.wait(timeout_ms);
        }

        let mut data = ptr::null_mut();
        let mut padding = 0;

        let hresult = unsafe { self.audio_client.GetCurrentPadding(&mut padding) }; // TODO: error

        let len = self.buffer_size - padding;
        let hresult = unsafe { self.audio_render_client.GetBuffer(len, &mut data) }; // TODO: error

        (data as _, len as _)
    }

    fn release_buffer(&self, num_frames: api::Frames) {
        unsafe {
            self.audio_render_client.ReleaseBuffer(num_frames as _, 0);
        }
    }
}

pub struct InputStream {
    audio_capture_client: WeakPtr<IAudioCaptureClient>,
    fence: Fence,
}

impl api::InputStream for InputStream {}

#[derive(Copy, Clone)]
struct Fence(pub winnt::HANDLE);
impl Fence {
    unsafe fn create(manual_reset: bool, initial_state: bool) -> Self {
        Fence(synchapi::CreateEventA(
            ptr::null_mut(),
            manual_reset as _,
            initial_state as _,
            ptr::null(),
        ))
    }

    unsafe fn wait(&self, timeout_ms: u32) -> u32 {
        synchapi::WaitForSingleObject(self.0, timeout_ms)
    }
}

fn map_format(format: &Format) -> Option<WAVEFORMATEXTENSIBLE> {
    let (format_tag, sub_format, bytes_per_sample) = match format.data_type {
        SampleFormat::F32 => (
            WAVE_FORMAT_EXTENSIBLE,
            ksmedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
            4,
        ),
        _ => unimplemented!(),
    };

    let bits_per_sample = 8 * bytes_per_sample;

    let wave_format = WAVEFORMATEX {
        wFormatTag: format_tag,
        nChannels: format.channels as _,
        nSamplesPerSec: format.sample_rate.0 as _,
        nAvgBytesPerSec: (format.channels as u32 * format.sample_rate.0 * bytes_per_sample as u32)
            as _,
        nBlockAlign: (format.channels * bytes_per_sample) as _,
        wBitsPerSample: bits_per_sample as _,
        cbSize: (mem::size_of::<WAVEFORMATEXTENSIBLE>() - mem::size_of::<WAVEFORMATEX>()) as _,
    };

    Some(WAVEFORMATEXTENSIBLE {
        Format: wave_format,
        Samples: bits_per_sample as _,
        dwChannelMask: 0, // TODO
        SubFormat: sub_format,
    })
}
