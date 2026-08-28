//! Voicemeeter Remote API wrapper (FFI over VoicemeeterRemote64.dll).
//!
//! Symbols are resolved once at construction and stored as plain function
//! pointers; the `Library` is kept alive in the struct so they stay valid.
//! Intended for single-threaded use from the engine thread.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use libloading::Library;

use super::config::Rule;
use super::ducker::StripControl;
use super::layout::{strip_channels, strip_layout, to_db, DLL_NAME_64};

type LoginFn = unsafe extern "C" fn() -> i32;
type LogoutFn = unsafe extern "C" fn() -> i32;
type TypeFn = unsafe extern "C" fn(*mut i32) -> i32;
type DirtyFn = unsafe extern "C" fn() -> i32;
type GetFloatFn = unsafe extern "C" fn(*const c_char, *mut f32) -> i32;
type SetFloatFn = unsafe extern "C" fn(*const c_char, f32) -> i32;
type GetStringFn = unsafe extern "C" fn(*const c_char, *mut c_char) -> i32;
type SetParamsFn = unsafe extern "C" fn(*const c_char) -> i32;
type GetLevelFn = unsafe extern "C" fn(i32, i32, *mut f32) -> i32;

/// Default install locations for the 64-bit remote DLL.
pub const DEFAULT_DLL_PATHS: &[&str] = &[
    r"C:\Program Files (x86)\VB\Voicemeeter\VoicemeeterRemote64.dll",
    r"C:\Program Files\VB\Voicemeeter\VoicemeeterRemote64.dll",
];

pub struct VoicemeeterApi {
    _lib: Library,
    login: LoginFn,
    logout: LogoutFn,
    get_type: TypeFn,
    dirty: DirtyFn,
    get_float: GetFloatFn,
    set_float: SetFloatFn,
    get_string: GetStringFn,
    set_params: SetParamsFn,
    get_level: GetLevelFn,
}

impl VoicemeeterApi {
    /// Load the DLL from the first path that exists (or the plain name via the
    /// system search path).
    pub fn load() -> Result<Self, String> {
        for p in DEFAULT_DLL_PATHS {
            if Path::new(p).exists() {
                return Self::load_from(p);
            }
        }
        Self::load_from(DLL_NAME_64)
    }

    pub fn load_from(path: &str) -> Result<Self, String> {
        unsafe {
            let lib = Library::new(path).map_err(|e| format!("cannot load {path}: {e}"))?;
            macro_rules! sym {
                ($name:literal, $ty:ty) => {{
                    let s: libloading::Symbol<$ty> = lib
                        .get($name)
                        .map_err(|e| format!("missing symbol {}: {e}", String::from_utf8_lossy($name)))?;
                    *s
                }};
            }
            Ok(VoicemeeterApi {
                login: sym!(b"VBVMR_Login", LoginFn),
                logout: sym!(b"VBVMR_Logout", LogoutFn),
                get_type: sym!(b"VBVMR_GetVoicemeeterType", TypeFn),
                dirty: sym!(b"VBVMR_IsParametersDirty", DirtyFn),
                get_float: sym!(b"VBVMR_GetParameterFloat", GetFloatFn),
                set_float: sym!(b"VBVMR_SetParameterFloat", SetFloatFn),
                get_string: sym!(b"VBVMR_GetParameterStringA", GetStringFn),
                set_params: sym!(b"VBVMR_SetParameters", SetParamsFn),
                get_level: sym!(b"VBVMR_GetLevel", GetLevelFn),
                _lib: lib,
            })
        }
    }

    pub fn login(&self) -> i32 {
        unsafe { (self.login)() }
    }

    /// SetParameter writes are asynchronous; a logout right after one drops it.
    /// Pump the dirty poll so queued writes land, then log out.
    pub fn logout(&self) {
        self.flush();
        unsafe {
            (self.logout)();
        }
    }

    pub fn flush(&self) {
        for _ in 0..20 {
            self.is_dirty();
            sleep(Duration::from_millis(10));
        }
    }

    /// >= 0 while Voicemeeter is up (1 = params changed), < 0 when it is gone.
    /// Reads only refresh after this is called, so poll it before every read.
    pub fn is_dirty(&self) -> i32 {
        unsafe { (self.dirty)() }
    }

    pub fn kind(&self) -> i32 {
        let mut t: i32 = 0;
        unsafe {
            if (self.get_type)(&mut t) == 0 {
                t
            } else {
                0
            }
        }
    }

    pub fn get_float_by(&self, name: &str) -> Result<f32, String> {
        let cname = CString::new(name).map_err(|e| e.to_string())?;
        let mut v: f32 = 0.0;
        let rc = unsafe { (self.get_float)(cname.as_ptr(), &mut v) };
        if rc == 0 {
            Ok(v)
        } else {
            Err(format!("get {name} failed ({rc})"))
        }
    }

    pub fn set_float_by(&self, name: &str, value: f32) -> Result<(), String> {
        let cname = CString::new(name).map_err(|e| e.to_string())?;
        let rc = unsafe { (self.set_float)(cname.as_ptr(), value) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("set {name}={value} failed ({rc})"))
        }
    }

    pub fn get_string_by(&self, name: &str) -> Result<String, String> {
        let cname = CString::new(name).map_err(|e| e.to_string())?;
        let mut buf = [0i8; 512];
        let rc = unsafe { (self.get_string)(cname.as_ptr(), buf.as_mut_ptr()) };
        if rc != 0 {
            return Err(format!("get string {name} failed ({rc})"));
        }
        let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn script(&self, text: &str) -> Result<(), String> {
        let ctext = CString::new(text).map_err(|e| e.to_string())?;
        let rc = unsafe { (self.set_params)(ctext.as_ptr()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("script {text:?} failed ({rc})"))
        }
    }

    pub fn level(&self, channel: i32) -> f32 {
        let mut v: f32 = 0.0;
        unsafe {
            if (self.get_level)(0, channel, &mut v) == 0 {
                v
            } else {
                0.0
            }
        }
    }

    pub fn is_muted(&self, strip: u32) -> bool {
        self.get_float_by(&format!("Strip[{strip}].mute")).map(|v| v >= 0.5).unwrap_or(false)
    }

    /// Pre-fader peak of a strip in dB.
    pub fn strip_peak_db(&self, kind: i32, strip: u32) -> f32 {
        let mut peak = 0.0f32;
        for ch in strip_channels(kind, strip) {
            let v = self.level(ch);
            if v > peak {
                peak = v;
            }
        }
        to_db(peak)
    }

    /// A friendly label for a strip (its Voicemeeter label, else device name).
    /// Local display only.
    pub fn strip_label(&self, strip: u32) -> String {
        let label = self.get_string_by(&format!("Strip[{strip}].label")).unwrap_or_default();
        if !label.trim().is_empty() {
            return label;
        }
        self.get_string_by(&format!("Strip[{strip}].device.name")).unwrap_or_default()
    }
}

impl StripControl for VoicemeeterApi {
    fn get_gain(&self, strip: u32) -> f32 {
        self.get_float_by(&format!("Strip[{strip}].gain")).unwrap_or_else(|e| {
            log::warn!("{e}");
            0.0
        })
    }
    fn set_gain(&self, strip: u32, gain: f32) {
        if let Err(e) = self.set_float_by(&format!("Strip[{strip}].gain"), gain) {
            log::warn!("{e}");
        }
    }
    fn fade_gain(&self, strip: u32, gain: f32, ms: f64) {
        if let Err(e) = self.script(&format!("Strip[{strip}].FadeTo = ({gain:.2}, {});", ms as i64)) {
            log::warn!("{e}");
        }
    }
    fn get_color(&self, strip: u32) -> (f32, f32) {
        let x = self.get_float_by(&format!("Strip[{strip}].Color_x")).unwrap_or(0.0);
        let y = self.get_float_by(&format!("Strip[{strip}].Color_y")).unwrap_or(0.0);
        (x, y)
    }
    fn set_color(&self, strip: u32, x: f32, y: f32) {
        let _ = self.set_float_by(&format!("Strip[{strip}].Color_x"), x);
        let _ = self.set_float_by(&format!("Strip[{strip}].Color_y"), y);
    }
}

/// Which of a rule's targets actually have a Color pad on this Voicemeeter kind.
pub fn color_strips_for(rule: &Rule, kind: i32) -> std::collections::HashSet<u32> {
    let hw = strip_layout(kind).map(|(h, _)| h).unwrap_or(0);
    rule.targets.iter().copied().filter(|t| *t < hw).collect()
}

/// Read each source's pre-fader peak. Muted sources are still metered (pre-fader),
/// so a muted mic keeps triggering.
pub fn read_levels(api: &VoicemeeterApi, kind: i32, rule: &Rule) -> HashMap<u32, Option<f32>> {
    rule.sources.iter().map(|s| (s.strip, Some(api.strip_peak_db(kind, s.strip)))).collect()
}
