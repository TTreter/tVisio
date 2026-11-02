fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "tVisio — Ultra-Low Latency Virtual Webcam");
        res.set(
            "FileDescription",
            "tVisio — Motor Nativo de Webcam Virtual DirectShow de Ultra-Baixa Latência",
        );
        res.set("LegalCopyright", "Copyright (C) 2026 Tiago Treter (@TTreter) - GPLv3");
        let _ = res.compile();
    }
}
