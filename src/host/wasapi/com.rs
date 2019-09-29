//! Handles COM initialization and cleanup.

use super::check_result;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::ptr;

use super::winapi::ctypes::c_void;
use super::winapi::um::combaseapi::{CoInitializeEx, CoUninitialize};
use super::winapi::um::objbase::COINIT_MULTITHREADED;

thread_local!(static COM_INITIALIZED: ComInitialized = {
    unsafe {
        // this call can fail if another library initialized COM in single-threaded mode
        // handling this situation properly would make the API more annoying, so we just don't care
        check_result(CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED)).unwrap();
        ComInitialized(ptr::null_mut())
    }
});

/// RAII object that guards the fact that COM is initialized.
///
// We store a raw pointer because it's the only way at the moment to remove `Send`/`Sync` from the
// object.
struct ComInitialized(*mut ());

impl Drop for ComInitialized {
    #[inline]
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// Ensures that COM is initialized in this thread.
#[inline]
pub fn com_initialized() {
    COM_INITIALIZED.with(|_| {});
}

///
#[repr(transparent)]
pub struct WeakPtr<T>(*mut T);

impl<T> WeakPtr<T> {
    pub fn null() -> Self {
        WeakPtr(ptr::null_mut())
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    pub unsafe fn mut_void(&mut self) -> *mut *mut c_void {
        &mut self.0 as *mut *mut _ as *mut *mut _
    }
}

impl<T> Clone for WeakPtr<T> {
    fn clone(&self) -> Self {
        WeakPtr(self.0)
    }
}

impl<T> Copy for WeakPtr<T> {}

impl<T> Deref for WeakPtr<T> {
    type Target = T;
    fn deref(&self) -> &T {
        debug_assert!(!self.is_null());
        unsafe { &*self.0 }
    }
}

impl<T> fmt::Debug for WeakPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "WeakPtr( ptr: {:?} )", self.0)
    }
}

impl<T> PartialEq<*mut T> for WeakPtr<T> {
    fn eq(&self, other: &*mut T) -> bool {
        self.0 == *other
    }
}

impl<T> PartialEq for WeakPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T> Hash for WeakPtr<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
