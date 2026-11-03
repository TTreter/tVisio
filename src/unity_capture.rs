//! tVisio — Cliente Nativo de Memória Compartilhada DirectShow (UnityCapture Zero-Copy)
//! ====================================================================================
//! Implementa o protocolo binário nativo de Shared Memory Win32 do driver DirectShow
//! `UnityCapture` (`UnityCapture_Mutx0`, `UnityCapture_Want0`, `UnityCapture_Sent0`,
//! `UnityCapture_Data0`), eliminando dependências C++/Python (`pyvirtualcam`).

use std::ffi::c_void;
use std::ptr;
use winreg::enums::*;
use winreg::RegKey;

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Memory::{MapViewOfFile, OpenFileMappingA, UnmapViewOfFile, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS},
        Threading::{
            CreateEventA, OpenEventA, OpenMutexW, ReleaseMutex, ResetEvent, SetEvent, WaitForSingleObject,
            EVENT_MODIFY_STATE,
        },
    },
};

const SYNCHRONIZE_RIGHT: u32 = 0x0010_0000 | 0x0001; // SYNCHRONIZE | MUTEX_MODIFY_STATE
const UNITY_CAPTURE_CLSID_64: &str = "{5C2CD55C-92AD-4999-8666-912BD3E70010}";
const UNITY_CAPTURE_CLSID_32: &str = "{5C2CD55C-92AD-4999-8666-912BD3E70020}";

/// Configura as chaves de registro do dispositivo DirectShow no Windows
/// para que o dispositivo apareça com o nome customizado no OBS, Discord, Meet, Zoom e Teams.
pub fn configure_tvisio_camera_name(custom_name: Option<&str>) {
    let name = match custom_name {
        Some(n) if !n.trim().is_empty() => n.trim(),
        _ => "tVisio Virtual Camera",
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(r"Software\UnityCapture\Device 0") {
        let _ = key.set_value("FriendlyName", &name);
        let _ = key.set_value("Timeout", &1000u32);
        let _ = key.set_value("ResizeMode", &1u32);
        let _ = key.set_value("MirrorMode", &0u32);
    }

    // Registra FriendlyName diretamente na categoria DirectShow do Windows (64-bit e 32-bit)
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let ds_path_64 = format!(
        r"SOFTWARE\Classes\CLSID\{{860BB310-5D01-11d0-BD3B-00A0C911CE86}}\Instance\{}",
        UNITY_CAPTURE_CLSID_64
    );
    if let Ok((key64, _)) = hklm.create_subkey(&ds_path_64) {
        let _ = key64.set_value("FriendlyName", &name);
    }

    let ds_path_32 = format!(
        r"SOFTWARE\WOW6432Node\Classes\CLSID\{{860BB310-5D01-11d0-BD3B-00A0C911CE86}}\Instance\{}",
        UNITY_CAPTURE_CLSID_32
    );
    if let Ok((key32, _)) = hklm.create_subkey(&ds_path_32) {
        let _ = key32.set_value("FriendlyName", &name);
    }
}

/// Verifica no Registro do Windows se o filtro DirectShow UnityCapture está registrado.
pub fn is_unity_capture_registered() -> bool {
    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
    if hkcr.open_subkey(format!(r"CLSID\{UNITY_CAPTURE_CLSID_64}")).is_ok()
        || hkcr.open_subkey(format!(r"CLSID\{UNITY_CAPTURE_CLSID_32}")).is_ok()
    {
        return true;
    }
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if hklm.open_subkey(format!(r"SOFTWARE\Classes\CLSID\{UNITY_CAPTURE_CLSID_64}")).is_ok()
        || hklm.open_subkey(format!(r"SOFTWARE\Classes\CLSID\{UNITY_CAPTURE_CLSID_32}")).is_ok()
        || hklm.open_subkey(format!(r"SOFTWARE\WOW6432Node\Classes\CLSID\{UNITY_CAPTURE_CLSID_32}")).is_ok()
    {
        return true;
    }
    false
}

/// Cliente de Memória Compartilhada Win32 para envio direto de quadros RGBA ao filtro DirectShow.
pub struct UnityCaptureSharedMem {
    #[cfg(target_os = "windows")]
    h_mutex: HANDLE,
    #[cfg(target_os = "windows")]
    h_want_event: HANDLE,
    #[cfg(target_os = "windows")]
    h_sent_event: HANDLE,
    #[cfg(target_os = "windows")]
    h_map_file: HANDLE,
    #[cfg(target_os = "windows")]
    p_data: *mut u8,
    pub driver_registered: bool,
    pub consumer_connected: bool,
}

unsafe impl Send for UnityCaptureSharedMem {}
unsafe impl Sync for UnityCaptureSharedMem {}

impl Default for UnityCaptureSharedMem {
    fn default() -> Self {
        Self::new()
    }
}

impl UnityCaptureSharedMem {
    pub fn new() -> Self {
        Self::with_name("tVisio Virtual Camera")
    }

    pub fn with_name(custom_name: &str) -> Self {
        configure_tvisio_camera_name(Some(custom_name));
        let registered = is_unity_capture_registered();
        let mut instance = Self {
            #[cfg(target_os = "windows")]
            h_mutex: ptr::null_mut(),
            #[cfg(target_os = "windows")]
            h_want_event: ptr::null_mut(),
            #[cfg(target_os = "windows")]
            h_sent_event: ptr::null_mut(),
            #[cfg(target_os = "windows")]
            h_map_file: ptr::null_mut(),
            #[cfg(target_os = "windows")]
            p_data: ptr::null_mut(),
            driver_registered: registered,
            consumer_connected: false,
        };
        instance.try_connect_shm();
        instance
    }

    /// Fecha handles e limpa ponteiros quando o consumidor desconecta
    pub fn reset_shm(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            if !self.p_data.is_null() {
                let view = MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.p_data as *mut c_void,
                };
                UnmapViewOfFile(view);
                self.p_data = ptr::null_mut();
            }
            if !self.h_map_file.is_null() {
                CloseHandle(self.h_map_file);
                self.h_map_file = ptr::null_mut();
            }
            if !self.h_sent_event.is_null() {
                CloseHandle(self.h_sent_event);
                self.h_sent_event = ptr::null_mut();
            }
            if !self.h_want_event.is_null() {
                CloseHandle(self.h_want_event);
                self.h_want_event = ptr::null_mut();
            }
            if !self.h_mutex.is_null() {
                CloseHandle(self.h_mutex);
                self.h_mutex = ptr::null_mut();
            }
            self.consumer_connected = false;
        }
    }

    /// Atualiza o estado de registro do driver DirectShow no Windows.
    pub fn refresh_registration_status(&mut self) -> bool {
        configure_tvisio_camera_name(None);
        self.driver_registered = is_unity_capture_registered();
        if self.driver_registered && !self.consumer_connected {
            self.try_connect_shm();
        }
        self.driver_registered
    }

    /// Tenta abrir os objetos de memória compartilhada criados pelo filtro DirectShow quando um consumidor
    /// (OBS Studio, Discord, Meet, Zoom) abre a câmera virtual.
    pub fn try_connect_shm(&mut self) -> bool {
        #[cfg(target_os = "windows")]
        unsafe {
            if !self.p_data.is_null() {
                // Valida se o mutex ainda responde; se foi fechado pelo consumidor, reinicia handles
                if !self.h_mutex.is_null() && WaitForSingleObject(self.h_mutex, 0) == 0 {
                    ReleaseMutex(self.h_mutex);
                    self.consumer_connected = true;
                    return true;
                } else {
                    self.reset_shm();
                }
            }

            if self.h_mutex.is_null() {
                let mutex_w: Vec<u16> = "UnityCapture_Mutx0\0".encode_utf16().collect();
                self.h_mutex = OpenMutexW(SYNCHRONIZE_RIGHT, 0, mutex_w.as_ptr());
                if self.h_mutex.is_null() {
                    return false;
                }
            }

            if WaitForSingleObject(self.h_mutex, 40) != 0 {
                return false;
            }

            if self.h_want_event.is_null() {
                self.h_want_event = CreateEventA(
                    ptr::null(),
                    0,
                    0,
                    b"UnityCapture_Want0\0".as_ptr(),
                );
            }

            if self.h_sent_event.is_null() {
                self.h_sent_event = OpenEventA(
                    EVENT_MODIFY_STATE,
                    0,
                    b"UnityCapture_Sent0\0".as_ptr(),
                );
            }

            if self.h_map_file.is_null() {
                self.h_map_file = OpenFileMappingA(
                    FILE_MAP_WRITE,
                    0,
                    b"UnityCapture_Data0\0".as_ptr(),
                );
            }

            if !self.h_map_file.is_null() && self.p_data.is_null() {
                let view: MEMORY_MAPPED_VIEW_ADDRESS =
                    MapViewOfFile(self.h_map_file, FILE_MAP_WRITE, 0, 0, 0);
                if !view.Value.is_null() {
                    self.p_data = view.Value as *mut u8;
                    self.consumer_connected = true;
                    self.driver_registered = true;
                }
            }

            ReleaseMutex(self.h_mutex);
            return self.consumer_connected;
        }

        #[cfg(not(target_os = "windows"))]
        false
    }

    /// Envia um quadro RGBA8 diretamente na memória compartilhada `UnityCapture_Data0`.
    pub fn send_frame_rgba_bottom_up(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> bool {
        #[cfg(target_os = "windows")]
        unsafe {
            if self.p_data.is_null() && !self.try_connect_shm() {
                return false;
            }

            let data_size = (width * height * 4) as usize;
            if rgba_data.len() < data_size {
                return false;
            }

            let max_size = ptr::read_unaligned(self.p_data as *const u32) as usize;
            if max_size > 0 && max_size < data_size {
                return false;
            }

            // Garante que o handle de Sent0 esteja conectado se o filtro iniciou após o tVisio
            if self.h_sent_event.is_null() {
                self.h_sent_event = OpenEventA(
                    EVENT_MODIFY_STATE,
                    0,
                    b"UnityCapture_Sent0\0".as_ptr(),
                );
            }

            // Timeout curto de 25ms para sincronização
            if WaitForSingleObject(self.h_mutex, 25) != 0 {
                return false;
            }

            let header = self.p_data as *mut i32;
            // Layout exato de 32 bytes do cabeçalho SharedImageMemory do UnityCapture:
            // [0x00] uint32 maxSize
            // [0x04] int32 width
            // [0x08] int32 height
            // [0x0C] int32 stride (em pixels = width)
            // [0x10] int32 format (0 = CAP_FORMAT_RGBA8)
            // [0x14] int32 resizemode (1 = Linear)
            // [0x18] int32 mirrormode (0 = Disabled)
            // [0x1C] int32 timeout (1000ms / 0x7FFFFF37)
            ptr::write_unaligned(header.add(1), width as i32);
            ptr::write_unaligned(header.add(2), height as i32);
            ptr::write_unaligned(header.add(3), width as i32);
            ptr::write_unaligned(header.add(4), 0i32);
            ptr::write_unaligned(header.add(5), 1i32);
            ptr::write_unaligned(header.add(6), 0i32);
            ptr::write_unaligned(header.add(7), 0x7FFF_FF37i32);

            let pixel_dst = self.p_data.add(0x20);
            ptr::copy_nonoverlapping(rgba_data.as_ptr(), pixel_dst, data_size);

            ReleaseMutex(self.h_mutex);

            if !self.h_sent_event.is_null() {
                SetEvent(self.h_sent_event);
            }
            if !self.h_want_event.is_null() {
                ResetEvent(self.h_want_event);
            }
            return true;
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (width, height, rgba_data);
            false
        }
    }

    pub fn backend_status_label(&self) -> String {
        if self.consumer_connected {
            "tVisio Virtual Camera (DirectShow Ativo)".to_string()
        } else if self.driver_registered {
            "tVisio Virtual Camera (Pronto no OBS/Discord)".to_string()
        } else {
            "Driver Virtual Pendente (Ative em 1-Clique)".to_string()
        }
    }
}

impl Drop for UnityCaptureSharedMem {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            if !self.p_data.is_null() {
                let view = MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.p_data as *mut c_void,
                };
                UnmapViewOfFile(view);
                self.p_data = ptr::null_mut();
            }
            if !self.h_map_file.is_null() {
                CloseHandle(self.h_map_file);
                self.h_map_file = ptr::null_mut();
            }
            if !self.h_sent_event.is_null() {
                CloseHandle(self.h_sent_event);
                self.h_sent_event = ptr::null_mut();
            }
            if !self.h_want_event.is_null() {
                CloseHandle(self.h_want_event);
                self.h_want_event = ptr::null_mut();
            }
            if !self.h_mutex.is_null() {
                CloseHandle(self.h_mutex);
                self.h_mutex = ptr::null_mut();
            }
        }
    }
}
