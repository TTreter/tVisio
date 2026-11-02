use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    #[cfg(target_os = "windows")]
    {
        // 1. Gera shim estático para o WebView2Loader a fim de eliminar qualquer dependência
        //    de WebView2Loader.dll na tabela de importação PE do tVisio.exe (100% single-file).
        if let Ok(out_dir) = env::var("OUT_DIR") {
            let out_path = PathBuf::from(&out_dir);
            let c_file = out_path.join("wv2_static_shim.c");
            let o_file = out_path.join("wv2_static_shim.o");
            let a_file = out_path.join("libWebView2Loader.dll.a");
            let lib_file = out_path.join("WebView2Loader.dll.lib");

            let c_src = r#"
#include <windows.h>

static HMODULE get_wv2_module(void) {
    static HMODULE hMod = NULL;
    if (!hMod) {
        WCHAR path[MAX_PATH * 2];
        DWORD len = GetEnvironmentVariableW(L"TVISIO_WV2_DLL_PATH", path, MAX_PATH * 2);
        if (len > 0 && len < MAX_PATH * 2) {
            hMod = LoadLibraryW(path);
        }
        if (!hMod) {
            hMod = LoadLibraryW(L"WebView2Loader.dll");
        }
    }
    return hMod;
}

typedef HRESULT (WINAPI *PFN_CompareBrowserVersions)(LPCWSTR, LPCWSTR, int*);
typedef HRESULT (WINAPI *PFN_CreateCoreWebView2Environment)(void*);
typedef HRESULT (WINAPI *PFN_CreateCoreWebView2EnvironmentWithOptions)(LPCWSTR, LPCWSTR, void*, void*);
typedef HRESULT (WINAPI *PFN_GetAvailableCoreWebView2BrowserVersionString)(LPCWSTR, LPWSTR*);

HRESULT WINAPI CompareBrowserVersions(LPCWSTR v1, LPCWSTR v2, int* res) {
    HMODULE h = get_wv2_module();
    if (!h) return E_FAIL;
    PFN_CompareBrowserVersions fn = (PFN_CompareBrowserVersions)GetProcAddress(h, "CompareBrowserVersions");
    return fn ? fn(v1, v2, res) : E_FAIL;
}

HRESULT WINAPI CreateCoreWebView2Environment(void* handler) {
    HMODULE h = get_wv2_module();
    if (!h) return E_FAIL;
    PFN_CreateCoreWebView2Environment fn = (PFN_CreateCoreWebView2Environment)GetProcAddress(h, "CreateCoreWebView2Environment");
    return fn ? fn(handler) : E_FAIL;
}

HRESULT WINAPI CreateCoreWebView2EnvironmentWithOptions(LPCWSTR browserFolder, LPCWSTR userDataFolder, void* options, void* handler) {
    HMODULE h = get_wv2_module();
    if (!h) return E_FAIL;
    PFN_CreateCoreWebView2EnvironmentWithOptions fn = (PFN_CreateCoreWebView2EnvironmentWithOptions)GetProcAddress(h, "CreateCoreWebView2EnvironmentWithOptions");
    return fn ? fn(browserFolder, userDataFolder, options, handler) : E_FAIL;
}

HRESULT WINAPI GetAvailableCoreWebView2BrowserVersionString(LPCWSTR browserFolder, LPWSTR* versionInfo) {
    HMODULE h = get_wv2_module();
    if (!h) return E_FAIL;
    PFN_GetAvailableCoreWebView2BrowserVersionString fn = (PFN_GetAvailableCoreWebView2BrowserVersionString)GetProcAddress(h, "GetAvailableCoreWebView2BrowserVersionString");
    return fn ? fn(browserFolder, versionInfo) : E_FAIL;
}
"#;
            let _ = fs::write(&c_file, c_src);
            let status_gcc = Command::new("gcc")
                .args(["-O2", "-c"])
                .arg(&c_file)
                .arg("-o")
                .arg(&o_file)
                .status();

            if status_gcc.map(|s| s.success()).unwrap_or(false) {
                let _ = Command::new("ar")
                    .args(["rcs"])
                    .arg(&a_file)
                    .arg(&o_file)
                    .status();
                let _ = fs::copy(&a_file, &lib_file);
                println!("cargo:rustc-link-search=native={}", out_path.display());

                // Substitui também o import library do webview2-com-sys no diretório build
                if let Some(build_root) = out_path.parent().and_then(|p| p.parent()) {
                    if let Ok(entries) = fs::read_dir(build_root) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with("webview2-com-sys-") {
                                let x64_dir = entry.path().join("out").join("x64");
                                if x64_dir.is_dir() {
                                    let _ = fs::copy(&a_file, x64_dir.join("WebView2Loader.dll.lib"));
                                    let _ = fs::copy(&a_file, x64_dir.join("libWebView2Loader.dll.a"));
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Embute o ícone e metadados Win32 no executável
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "tVisio — Ultra-Low Latency Virtual Webcam");
        res.set(
            "FileDescription",
            "tVisio — Motor Nativo de Webcam Virtual DirectShow de Ultra-Baixa Latência",
        );
        res.set("LegalCopyright", "Copyright (C) 2026 Tiago Treter (@TTreter) - GPLv3");
        let _ = res.compile();

        if let Ok(out_dir) = env::var("OUT_DIR") {
            let res_o = PathBuf::from(&out_dir).join("resource.o");
            if res_o.exists() {
                println!("cargo:rustc-link-arg={}", res_o.display());
            }
        }
    }
}
