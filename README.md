<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://img.shields.io/badge/Chronicle-FF6B6B?style=for-the-badge&logo=tauri&logoColor=white">
    <img alt="Chronicle" src="https://img.shields.io/badge/Chronicle-FF6B6B?style=for-the-badge&logo=tauri&logoColor=white">
  </picture>
</p>

<p align="center">
  <strong>Desktop app para grabar streams en vivo</strong><br>
  <sub>Portátil · Sin instalación · FFmpeg + yt-dlp + Streamlink incluidos</sub>
</p>

<p align="center">
  <img alt="GitHub release" src="https://img.shields.io/badge/version-0.1.0-8A2BE2">
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows-blue">
  <img alt="Built with" src="https://img.shields.io/badge/built%20with-Tauri%20%2B%20React-61DAFB">
</p>

---

## ✨ Características

- **Grabación en vivo** — Soporta URLs directas (`.m3u8`, `.mp4`, `rtmp://`) y páginas de streaming.
- **Selector de calidad** — Elige resolución antes de grabar.
- **Timer en vivo** — Muestra HH:MM:SS transcurrido.
- **Monitoreo de errores** — Detecta fallos de FFmpeg y los muestra en la UI.
- **Portátil** — FFmpeg, yt-dlp y Streamlink empaquetados como `.exe`. No requiere instalación.
- **Plugin personalizado** — yt-dlp plugin que normaliza URLs y extrae HLS de páginas compatibles.

## 🚀 Cómo usar

1. Descarga la última versión desde [Releases](https://github.com/Aron1244/chronicle/releases).
2. Ejecuta `Chronicle.exe`.
3. Pega la URL del stream y elige calidad.
4. Presiona **Start**.

## 🧰 Stack

| Capa | Tecnología |
|------|-----------|
| Frontend | React 19 + TypeScript + Tailwind CSS v4 + shadcn/ui |
| Backend | Rust + Tauri v2 |
| Grabación | yt-dlp + FFmpeg + Streamlink |

## 🏗️ Desarrollo

```bash
# Clonar
git clone https://github.com/Aron1244/chronicle.git
cd chronicle

# Instalar dependencias
pnpm install

# Iniciar en modo desarrollo
pnpm tauri dev

# Build para producción
pnpm tauri build
```

> **Nota:** Los binarios (yt-dlp.exe, ffmpeg.exe, streamlink.exe) se trackean con Git LFS.
> ```bash
> git lfs pull
> ```

## 📁 Estructura

```
chronicle/
├── src/                    # UI (React + Tailwind)
│   ├── App.tsx             # Componente principal
│   └── components/ui/      # Componentes shadcn
├── src-tauri/              # Backend Rust
│   └── src/
│       ├── ffmpeg/         # Recorder + format detection
│       ├── commands/       # Tauri commands
│       └── state/          # App state
├── yt-dlp-plugins/         # Plugin personalizado yt-dlp
└── streamlink-*/           # Streamlink portable + FFmpeg
```

## 📄 Licencia

MIT
