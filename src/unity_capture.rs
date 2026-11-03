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
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Memory::{
            CreateFileMappingA, MapViewOfFile, OpenFileMappingA, UnmapViewOfFile,
            FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
        },
        Threading::{
            CreateEventA, CreateMutexA, OpenEventA, OpenMutexW, ReleaseMutex, ResetEvent,
            SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        },
    },
};

const SYNCHRONIZE_RIGHT: u32 = 0x0010_0000 | 0x0001; // SYNCHRONIZE | MUTEX_MODIFY_STATE
const UNITY_CAPTURE_CLSID_64: &str = "{5C2CD55C-92AD-4999-8666-912BD3E70010}";
const UNITY_CAPTURE_CLSID_32: &str = "{5C2CD55C-92AD-4999-8666-912BD3E70020}";

/// Tamanho máximo da imagem no UnityCapture: 3840 x 2160 x 4 x sizeof(short) = 66.355.200 bytes (4K RGBA)
const MAX_SHARED_IMAGE_SIZE: u32 = 3840 * 2160 * 4 * 2;
/// Tamanho do cabeçalho binário (32 bytes): maxSize(4), width(4), height(4), stride(4), format(4), resizemode(4), mirrormode(4), timeout(4)
const SHM_HEADER_SIZE: u32 = 32;
/// Tamanho total do mapeamento compartilhado Win32 (66.355.232 bytes)
const TOTAL_SHM_SIZE: u32 = SHM_HEADER_SIZE + MAX_SHARED_IMAGE_SIZE;

// Nomes IPC nativos do filtro DirectShow UnityCapture para o Dispositivo 0 (CapNum 0).
// OBS/UnityCapture substitui o '0' final por '\0' para CapNum 0 ("UnityCapture_Mutx", "UnityCapture_Data", etc.)
const IPC_MUTEX_PRIMARY: &[u8] = b"UnityCapture_Mutx\0";
const IPC_MUTEX_FALLBACK: &[u8] = b"UnityCapture_Mutx0\0";
const IPC_WANT_PRIMARY: &[u8] = b"UnityCapture_Want\0";
const IPC_WANT_FALLBACK: &[u8] = b"UnityCapture_Want0\0";
const IPC_SENT_PRIMARY: &[u8] = b"UnityCapture_Sent\0";
const IPC_SENT_FALLBACK: &[u8] = b"UnityCapture_Sent0\0";
const IPC_DATA_PRIMARY: &[u8] = b"UnityCapture_Data\0";
const IPC_DATA_FALLBACK: &[u8] = b"UnityCapture_Data0\0";

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

    /// Abre ou cria os objetos de sincronização e memória compartilhada Win32
    /// utilizados pelo driver DirectShow `UnityCapture` (nome padrão "UnityCapture_...").
    /// Funciona de forma bidirecional:
    /// - Se o OBS/Discord abrir primeiro, o tVisio conecta no mapeamento existente.
    /// - Se o tVisio abrir antes do OBS, o tVisio pré-aloca a memória compartilhada com o
    ///   cabeçalho oficial pré-configurado, evitando a tela verde de "Unity has not started sending image data".
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

            // 1. Mutex de Sincronização: tenta abrir existente ou cria para garantir que o objeto exista
            if self.h_mutex.is_null() {
                let mutex_w: Vec<u16> = "UnityCapture_Mutx\0".encode_utf16().collect();
                let mut h = OpenMutexW(SYNCHRONIZE_RIGHT, 0, mutex_w.as_ptr());
                if h.is_null() {
                    h = CreateMutexA(ptr::null(), 0, IPC_MUTEX_PRIMARY.as_ptr());
                }
                if h.is_null() {
                    let mutex_fallback_w: Vec<u16> = "UnityCapture_Mutx0\0".encode_utf16().collect();
                    h = OpenMutexW(SYNCHRONIZE_RIGHT, 0, mutex_fallback_w.as_ptr());
                    if h.is_null() {
                        h = CreateMutexA(ptr::null(), 0, IPC_MUTEX_FALLBACK.as_ptr());
                    }
                }
                self.h_mutex = h;
                if self.h_mutex.is_null() {
                    return false;
                }
            }

            // 2. Evento "Want": disparado pelo OBS quando solicita um novo quadro
            if self.h_want_event.is_null() {
                let mut h = OpenEventA(EVENT_MODIFY_STATE, 0, IPC_WANT_PRIMARY.as_ptr());
                if h.is_null() {
                    h = CreateEventA(ptr::null(), 0, 0, IPC_WANT_PRIMARY.as_ptr());
                }
                if h.is_null() {
                    h = OpenEventA(EVENT_MODIFY_STATE, 0, IPC_WANT_FALLBACK.as_ptr());
                    if h.is_null() {
                        h = CreateEventA(ptr::null(), 0, 0, IPC_WANT_FALLBACK.as_ptr());
                    }
                }
                self.h_want_event = h;
            }

            // 3. Evento "Sent": disparado pelo tVisio para avisar ao OBS que um novo quadro foi escrito
            if self.h_sent_event.is_null() {
                let mut h = OpenEventA(EVENT_MODIFY_STATE, 0, IPC_SENT_PRIMARY.as_ptr());
                if h.is_null() {
                    h = CreateEventA(ptr::null(), 0, 0, IPC_SENT_PRIMARY.as_ptr());
                }
                if h.is_null() {
                    h = OpenEventA(EVENT_MODIFY_STATE, 0, IPC_SENT_FALLBACK.as_ptr());
                    if h.is_null() {
                        h = CreateEventA(ptr::null(), 0, 0, IPC_SENT_FALLBACK.as_ptr());
                    }
                }
                self.h_sent_event = h;
            }

            // 4. Arquivo de Memória Compartilhada (Shared Memory MMF)
            if self.h_map_file.is_null() {
                let mut h = OpenFileMappingA(FILE_MAP_WRITE, 0, IPC_DATA_PRIMARY.as_ptr());
                if h.is_null() {
                    h = CreateFileMappingA(
                        INVALID_HANDLE_VALUE,
                        ptr::null(),
                        PAGE_READWRITE,
                        0,
                        TOTAL_SHM_SIZE,
                        IPC_DATA_PRIMARY.as_ptr(),
                    );
                }
                if h.is_null() {
                    h = OpenFileMappingA(FILE_MAP_WRITE, 0, IPC_DATA_FALLBACK.as_ptr());
                    if h.is_null() {
                        h = CreateFileMappingA(
                            INVALID_HANDLE_VALUE,
                            ptr::null(),
                            PAGE_READWRITE,
                            0,
                            TOTAL_SHM_SIZE,
                            IPC_DATA_FALLBACK.as_ptr(),
                        );
                    }
                }
                self.h_map_file = h;
            }

            if !self.h_map_file.is_null() && self.p_data.is_null() {
                let view: MEMORY_MAPPED_VIEW_ADDRESS =
                    MapViewOfFile(self.h_map_file, FILE_MAP_WRITE, 0, 0, 0);
                if !view.Value.is_null() {
                    self.p_data = view.Value as *mut u8;
                    self.consumer_connected = true;
                    self.driver_registered = true;

                    // Inicializa cabeçalho com maxSize e dimensões válidas para que o filtro DirectShow
                    // ao conectar nunca veja width == 0 (causa da tela verde RECEIVERES_CAPTUREINACTIVE)
                    let header_u32 = self.p_data as *mut u32;
                    let current_max = ptr::read_unaligned(header_u32);
                    if current_max == 0 {
                        ptr::write_unaligned(header_u32, MAX_SHARED_IMAGE_SIZE);
                    }

                    let header_i32 = self.p_data as *mut i32;
                    ptr::write_unaligned(header_i32.add(1), 1280i32);       // width
                    ptr::write_unaligned(header_i32.add(2), 720i32);        // height
                    ptr::write_unaligned(header_i32.add(3), 1280i32);       // stride (em pixels = width)
                    ptr::write_unaligned(header_i32.add(4), 0i32);          // format = FORMAT_UINT8 (RGBA 8-bit)
                    ptr::write_unaligned(header_i32.add(5), 1i32);          // resizemode = RESIZEMODE_LINEAR
                    ptr::write_unaligned(header_i32.add(6), 0i32);          // mirrormode = MIRRORMODE_DISABLED
                    ptr::write_unaligned(header_i32.add(7), 0x7FFF_FF37i32); // timeout = 1000ms
                }
            }

            return !self.p_data.is_null();
        }

        #[cfg(not(target_os = "windows"))]
        false
    }

    /// Envia um quadro RGBA8 diretamente na memória compartilhada DirectShow do UnityCapture.
    /// Como o padrão do DirectShow para DIBs (Device-Independent Bitmaps) é bottom-up
    /// (linha 0 da memória é exibida na parte inferior da tela no OBS/Discord), este método
    /// converte a imagem top-down de origem invertendo a ordem das linhas verticalmente,
    /// garantindo que a imagem seja exibida perfeitamente orientada no OBS/Discord.
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
            if max_size == 0 {
                ptr::write_unaligned(self.p_data as *mut u32, MAX_SHARED_IMAGE_SIZE);
            } else if max_size < data_size {
                return false;
            }

            // Garante que o handle de Sent esteja conectado caso o consumidor tenha aberto recentemente
            if self.h_sent_event.is_null() {
                let mut h = OpenEventA(EVENT_MODIFY_STATE, 0, IPC_SENT_PRIMARY.as_ptr());
                if h.is_null() {
                    h = OpenEventA(EVENT_MODIFY_STATE, 0, IPC_SENT_FALLBACK.as_ptr());
                }
                self.h_sent_event = h;
            }

            // Timeout de 25ms para sincronização com o consumidor
            if WaitForSingleObject(self.h_mutex, 25) != 0 {
                return false;
            }

            let header = self.p_data as *mut i32;
            // Layout exato de 32 bytes do cabeçalho SharedImageMemory do UnityCapture:
            // [0x00] uint32 maxSize
            // [0x04] int32 width
            // [0x08] int32 height
            // [0x0C] int32 stride (em pixels = width)
            // [0x10] int32 format (0 = FORMAT_UINT8)
            // [0x14] int32 resizemode (1 = RESIZEMODE_LINEAR)
            // [0x18] int32 mirrormode (0 = MIRRORMODE_DISABLED)
            // [0x1C] int32 timeout (0x7FFFFF37 / 1000ms)
            ptr::write_unaligned(header.add(1), width as i32);
            ptr::write_unaligned(header.add(2), height as i32);
            ptr::write_unaligned(header.add(3), width as i32);
            ptr::write_unaligned(header.add(4), 0i32);
            ptr::write_unaligned(header.add(5), 1i32);
            ptr::write_unaligned(header.add(6), 0i32);
            ptr::write_unaligned(header.add(7), 0x7FFF_FF37i32);

            let row_stride = (width * 4) as usize;
            let pixel_dst = self.p_data.add(0x20);
            let h = height as usize;

            // Inversão vertical de linhas (Top-Down -> DirectShow Bottom-Up DIB)
            // Copia linha a linha: linha 0 (topo) vai para linha (h - 1) no buffer DirectShow
            for y in 0..h {
                let src_offset = y * row_stride;
                let dst_offset = (h - 1 - y) * row_stride;
                ptr::copy_nonoverlapping(
                    rgba_data.as_ptr().add(src_offset),
                    pixel_dst.add(dst_offset),
                    row_stride,
                );
            }

            ReleaseMutex(self.h_mutex);

            if !self.h_sent_event.is_null() {
                SetEvent(self.h_sent_event);
            }
            if !self.h_want_event.is_null() {
                ResetEvent(self.h_want_event);
            }
            self.consumer_connected = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn test_unity_capture_shm_protocol() {
        let mut shm = UnityCaptureSharedMem::new();
        assert!(shm.try_connect_shm(), "Deveria conectar ou criar a memória compartilhada");

        // Cria frame dummy 1280x720 RGBA
        // Primeira linha preenchida com valor 200 (topo) e última linha com valor 50 (base)
        let mut dummy_frame = vec![0u8; 1280 * 720 * 4];
        dummy_frame[0..1280 * 4].fill(200);
        let last_row_start = (720 - 1) * 1280 * 4;
        dummy_frame[last_row_start..last_row_start + 1280 * 4].fill(50);

        let sent = shm.send_frame_rgba_bottom_up(1280, 720, &dummy_frame);
        assert!(sent, "O envio de quadro RGBA para a memória compartilhada falhou");

        unsafe {
            assert!(!shm.p_data.is_null());
            let header_u32 = shm.p_data as *const u32;
            let header_i32 = shm.p_data as *const i32;

            assert_eq!(*header_u32, MAX_SHARED_IMAGE_SIZE, "maxSize incorreto");
            assert_eq!(*header_i32.add(1), 1280, "width incorreto");
            assert_eq!(*header_i32.add(2), 720, "height incorreto");
            assert_eq!(*header_i32.add(3), 1280, "stride incorreto");
            assert_eq!(*header_i32.add(4), 0, "format deve ser 0 (FORMAT_UINT8)");
            assert_eq!(*header_i32.add(5), 1, "resizemode deve ser 1 (LINEAR)");
            assert_eq!(*header_i32.add(6), 0, "mirrormode deve ser 0 (DISABLED)");

            // No DirectShow (bottom-up DIB), a linha 0 na memória corresponde à base da imagem (valor 50)
            let pixel_ptr = shm.p_data.add(0x20);
            assert_eq!(*pixel_ptr, 50, "Linha 0 na memória deve ser a base da imagem (bottom-up DIB)");

            // A última linha na memória corresponde ao topo da imagem (valor 200)
            let last_ptr = pixel_ptr.add(last_row_start);
            assert_eq!(*last_ptr, 200, "Última linha na memória deve ser o topo da imagem");
        }
    }
}
