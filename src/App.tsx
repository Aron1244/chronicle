import { useState, useCallback, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-dialog"
import { Button } from "@/components/ui/button"
import "./App.css"

interface StreamFormat {
  id: string
  ext: string
  resolution: string
  fps: number | null
  bitrate: number | null
  video_codec: string | null
  audio_codec: string | null
}

function App() {
  const [streamUrl, setStreamUrl] = useState("")
  const [fileName, setFileName] = useState("")
  const [outputFolder, setOutputFolder] = useState(() => localStorage.getItem("chronicle_output") || "")
  const [recording, setRecording] = useState(false)
  const [formats, setFormats] = useState<StreamFormat[]>([])
  const [selectedFormat, setSelectedFormat] = useState<string>("")
  const [showFormatPicker, setShowFormatPicker] = useState(false)
  const [fetching, setFetching] = useState(false)

  const saveFolder = useCallback((folder: string) => {
    setOutputFolder(folder)
    localStorage.setItem("chronicle_output", folder)
  }, [])
  const [status, setStatus] = useState("")
  const [elapsed, setElapsed] = useState(0)
  const startTime = useRef(0)
  const timerRef = useRef<ReturnType<typeof setInterval>>(undefined)
  const urlTimer = useRef<ReturnType<typeof setTimeout>>(undefined)

  useEffect(() => {
    if (recording) {
      startTime.current = Date.now()
      timerRef.current = setInterval(() => {
        setElapsed(Math.floor((Date.now() - startTime.current) / 1000))
      }, 1000)
    } else {
      clearInterval(timerRef.current)
      setElapsed(0)
    }
    return () => clearInterval(timerRef.current)
  }, [recording])

  const isDirectUrl = (url: string) =>
    url.endsWith(".m3u8") || url.endsWith(".mp4") || url.endsWith(".ts") ||
    url.startsWith("rtmp://") || url.startsWith("rtsp://")

  const doFetchFormats = useCallback(async (url: string) => {
    if (!url || isDirectUrl(url)) return
    setFetching(true)
    setStatus("Obteniendo formatos...")
    try {
      const result = await invoke<StreamFormat[]>("fetch_formats", { streamUrl: url })
      setFormats(result)
      if (result.length > 0) {
        setSelectedFormat(result[0].id)
        setShowFormatPicker(true)
        setStatus(`${result.length} calidad(es) disponible(s)`)
      } else {
        setFormats([])
        setSelectedFormat("")
        setStatus("No se encontraron formatos")
      }
    } catch (e) {
      setFormats([])
      setSelectedFormat("")
      const msg = String(e)
      if (msg.includes("not currently live") || msg.includes("UserNotLive")) {
        setStatus("El stream no está en vivo en este momento")
      } else {
        setStatus(`Error: ${msg}`)
      }
    } finally {
      setFetching(false)
    }
  }, [])

  const handleUrlChange = useCallback((url: string) => {
    setStreamUrl(url)
    clearTimeout(urlTimer.current)
    urlTimer.current = setTimeout(() => {
      if (url) doFetchFormats(url)
    }, 800)
  }, [doFetchFormats])

  const pickFolder = useCallback(async () => {
    const selected = await open({ directory: true, multiple: false })
    if (selected) saveFolder(selected)
  }, [saveFolder])

  const startRecording = useCallback(async () => {
    if (!streamUrl || !fileName || !outputFolder) {
      setStatus("Completa todos los campos")
      return
    }
    const outputPath = `${outputFolder}\\${fileName}.mp4`
    try {
      setStatus("Iniciando...")
      const msg = await invoke("start_recording", {
        streamUrl,
        outputPath,
        formatId: selectedFormat || null,
      })
      setRecording(true)
      setShowFormatPicker(false)
      setStatus(msg as string)
    } catch (e) {
      setStatus(`Error: ${e}`)
    }
  }, [streamUrl, fileName, outputFolder, selectedFormat])

  // Poll for errors while recording
  const errorTimerRef = useRef<ReturnType<typeof setInterval>>(undefined)

  const friendlyError = (raw: string): string => {
    if (raw.includes("ffmpeg exited with code")) return "Error de descarga: el stream no está disponible"
    if (raw.includes("404")) return "Error de conexión: no se pudo acceder al stream"
    if (raw.includes("HTTP error")) return "Error de conexión con el servidor de stream"
    if (raw.includes("Error opening input")) return "El stream no está accesible"
    return raw
  }

  useEffect(() => {
    if (recording) {
      errorTimerRef.current = setInterval(async () => {
        try {
          const err = await invoke<string | null>("get_recording_error")
          if (err) {
            setStatus(`Error: ${friendlyError(err)}`)
            setRecording(false)
            setElapsed(0)
          }
        } catch {
          // ignore poll errors
        }
      }, 2000)
    } else {
      clearInterval(errorTimerRef.current)
    }
    return () => clearInterval(errorTimerRef.current)
  }, [recording])

  const stopRecording = useCallback(async () => {
    try {
      setStatus("Deteniendo...")
      const msg = await invoke("stop_recording")
      setRecording(false)
      setStatus(msg as string)
    } catch (e) {
      setStatus(`Error: ${e}`)
    }
  }, [])

  const fmt = (s: number) => {
    const h = String(Math.floor(s / 3600)).padStart(2, "0")
    const m = String(Math.floor((s % 3600) / 60)).padStart(2, "0")
    const sec = String(s % 60).padStart(2, "0")
    return `${h}:${m}:${sec}`
  }

  const labelForFormat = (f: StreamFormat) => {
    if (f.resolution) {
      const fps = f.fps ? ` ${f.fps}fps` : ""
      const br = f.bitrate ? ` ${Math.round(f.bitrate)}kbps` : ""
      return `${f.id.padEnd(6)} ${f.resolution}${fps}${br}`
    }
    return f.id
  }

  return (
    <div className="min-h-screen bg-background text-foreground flex items-center justify-center p-4">
      <div className="w-full max-w-md space-y-6">
        <h1 className="text-2xl font-bold text-center">Chronicle</h1>

        <div className="space-y-4">
          <div>
            <label className="text-sm font-medium mb-1 block">URL del Stream</label>
            <input
              className="flex h-9 w-full rounded-lg border border-input bg-background px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              placeholder="https://...  (URL directa .m3u8 o link de sitio)"
              value={streamUrl}
              onChange={(e) => handleUrlChange(e.target.value)}
              disabled={recording}
            />
          </div>

          <div>
            <label className="text-sm font-medium mb-1 block">Nombre de la grabación</label>
            <input
              className="flex h-9 w-full rounded-lg border border-input bg-background px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              placeholder="Mi Stream"
              value={fileName}
              onChange={(e) => setFileName(e.target.value)}
              disabled={recording}
            />
          </div>

          <div>
            <label className="text-sm font-medium mb-1 block">Carpeta de destino</label>
            <div className="flex gap-2">
              <input
                className="flex h-9 w-full rounded-lg border border-input bg-background px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                placeholder="C:\Grabaciones"
                value={outputFolder}
                onChange={(e) => saveFolder(e.target.value)}
                disabled={recording}
              />
              <Button variant="outline" onClick={pickFolder} disabled={recording}>
                Examinar
              </Button>
            </div>
          </div>
        </div>

        {recording && (
          <div className="text-center space-y-1">
            <div className="text-3xl font-mono font-bold tabular-nums text-green-600">
              {fmt(elapsed)}
            </div>
            <div className="text-xs text-muted-foreground">grabando...</div>
          </div>
        )}

        {status && (
          <div className="text-sm text-center text-muted-foreground bg-muted rounded-lg px-3 py-2">
            {status}
          </div>
        )}

        <div className="flex gap-2">
          {!recording ? (
            <Button className="flex-1" onClick={startRecording} disabled={fetching}>
              Iniciar Grabación
            </Button>
          ) : (
            <Button className="flex-1" variant="destructive" onClick={stopRecording}>
              Detener Grabación
            </Button>
          )}
        </div>
      </div>

      {showFormatPicker && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowFormatPicker(false)}>
          <div className="bg-background border rounded-xl shadow-lg w-full max-w-sm p-6 space-y-4" onClick={(e) => e.stopPropagation()}>
            <h2 className="text-lg font-semibold">Seleccionar calidad</h2>
            <div className="space-y-2 max-h-64 overflow-y-auto">
              {formats.map((f) => (
                <label
                  key={f.id}
                  className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                    selectedFormat === f.id
                      ? "border-primary bg-primary/5"
                      : "border-border hover:bg-muted"
                  }`}
                >
                  <input
                    type="radio"
                    name="format"
                    value={f.id}
                    checked={selectedFormat === f.id}
                    onChange={() => setSelectedFormat(f.id)}
                    className="accent-primary"
                  />
                  <div className="flex-1 text-sm">
                    <span className="font-mono">{labelForFormat(f)}</span>
                  </div>
                </label>
              ))}
            </div>
            <div className="flex gap-2 pt-2">
              <Button variant="outline" className="flex-1" onClick={() => setShowFormatPicker(false)}>
                Cancelar
              </Button>
              <Button className="flex-1" onClick={() => setShowFormatPicker(false)}>
                Confirmar
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default App
