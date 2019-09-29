use std::{slice, ptr, mem};
use std::ops::{Deref, Drop};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

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
use {traits, DevicesError, Format, SampleFormat};

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
    unsafe fn enumerate_physical_devices(
        &self,
        ty: EDataFlow,
    ) -> Result<Vec<PhysicalDevice>, DevicesError> {
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
}

impl traits::Instance for Instance {
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

    fn enumerate_physical_input_devices(&self) -> Result<Vec<Self::PhysicalDevice>, DevicesError> {
        unsafe { self.enumerate_physical_devices(eCapture) }
    }

    fn enumerate_physical_output_devices(&self) -> Result<Vec<Self::PhysicalDevice>, DevicesError> {
        unsafe { self.enumerate_physical_devices(eRender) }
    }

    fn create_device(
        &self,
        physical_device: &Self::PhysicalDevice,
        format: Format,
    ) -> Self::Device {
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
            check_result(hresult).unwrap(); // TODO: error
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
                panic!("device not available"); // TODO: error
                                                // return Err(BuildStreamError::DeviceNotAvailable);
            }
            Err(e) => {
                unsafe { audio_client.Release() };
                panic!("{}", e); // TODO: error

                // let description = format!("{}", e);
                // let err = BackendSpecificError { description };
                // return Err(err.into());
            }
            Ok(()) => (),
        };

        let fence = unsafe { Fence::create(false, false) };

        let hresult = unsafe { audio_client.SetEventHandle(fence.0) }; // TODO: error

        Device {
            audio_client,
            fence,
        }
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

impl traits::PhysicalDevice for PhysicalDevice {
    fn properties(&self) -> traits::PhysicalDeviceProperties {
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

        traits::PhysicalDeviceProperties {
            device_name,
        }
    }
}

pub struct Device {
    audio_client: WeakPtr<IAudioClient>,
    fence: Fence,
}

impl traits::Device for Device {
    type OutputStream = OutputStream;
    type InputStream = InputStream;

    fn properties(&self) -> traits::DeviceProperties {
        unimplemented!()
    }

    fn output_stream(&self) -> Self::OutputStream {
        unimplemented!()
    }

    fn async_output_stream(&self) -> Self::OutputStream {
        unimplemented!()
    }

    fn input_stream(&self) -> Self::InputStream {
        unimplemented!()
    }

    fn async_input_stream(&self) -> Self::InputStream {
        unimplemented!()
    }
}

pub struct OutputStream {
    audio_client: WeakPtr<IAudioClient>,
    audio_render_client: WeakPtr<IAudioRenderClient>,
    fence: Fence,
}

impl traits::OutputStream for OutputStream {
    fn start(&self) {
        unimplemented!()
    }

    fn stop(&self) {
        unimplemented!()
    }

    fn acquire_buffer(&self, timeout_ms: u32) -> (*mut (), traits::Frames) {
        unimplemented!()
    }

    fn release_buffer(&self, num_frames: traits::Frames) {
        unimplemented!()
    }
}

pub struct InputStream {
    audio_capture_client: WeakPtr<IAudioCaptureClient>,
    fence: Fence,
}

impl traits::InputStream for InputStream {}

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
