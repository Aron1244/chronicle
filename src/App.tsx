import { useState, useCallback, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Card, CardHeader, CardTitle, CardContent, CardFooter,
} from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription,
} from "@/components/ui/dialog"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Skeleton } from "@/components/ui/skeleton"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import {
  Monitor, Moon, Sun, Video, Square, Folder, Loader2, Radio,
} from "lucide-react"
import "./App.css"

const MAX_SLOTS = 3

interface StreamFormat {
  id: string
  ext: string
  resolution: string
  fps: number | null
  bitrate: number | null
  video_codec: string | null
  audio_codec: string | null
  url: string | null
}

interface SlotStatus {
  slot: number
  recording: boolean
  optimizing: boolean
  error: string | null
  url: string | null
  label: string | null
  elapsed_secs: number
}

interface CompressionConfig { crf: number; preset: string }

type Theme = "system" | "light" | "dark"

function App() {
  const [theme, setTheme] = useState<Theme>(() => {
    const saved = localStorage.getItem("chronicle_theme") as Theme | null
    if (saved) return saved
    return "system"
  })
  const [statuses, setStatuses] = useState<SlotStatus[]>(
    Array.from({ length: MAX_SLOTS }, (_, i) => ({
      slot: i, recording: false, optimizing: false, error: null, url: null, label: null, elapsed_secs: 0,
    }))
  )
  const [globalMsg, setGlobalMsg] = useState("")

  // Modal state
  const [modalSlot, setModalSlot] = useState<number | null>(null)
  const [streamUrl, setStreamUrl] = useState("")
  const [fileName, setFileName] = useState("")
  const [outputFolder, setOutputFolder] = useState(() => localStorage.getItem("chronicle_output") || "")
  const [formats, setFormats] = useState<StreamFormat[]>([])
  const [selectedFormat, setSelectedFormat] = useState("")
  const [selectedFormatUrl, setSelectedFormatUrl] = useState<string | null>(null)
  const [fetching, setFetching] = useState(false)
  const [previewData, setPreviewData] = useState<string | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [starting, setStarting] = useState(false)
  const [compressEnabled, setCompressEnabled] = useState(false)
  const [compressCrf, setCompressCrf] = useState(23)
  const [compressPreset, setCompressPreset] = useState("medium")
  const urlTimer = useRef<ReturnType<typeof setTimeout>>(undefined)

  // Dark mode
  useEffect(() => {
    const root = document.documentElement
    if (theme === "dark") {
      root.classList.add("dark")
    } else if (theme === "light") {
      root.classList.remove("dark")
    } else {
      const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches
      root.classList.toggle("dark", prefersDark)
    }
    localStorage.setItem("chronicle_theme", theme)
  }, [theme])

  const cycleTheme = useCallback(() => {
    setTheme(t => t === "system" ? "light" : t === "light" ? "dark" : "system")
  }, [])

  const ThemeIcon = theme === "dark" ? Moon : theme === "light" ? Sun : Monitor

  // Poll all slot statuses
  useEffect(() => {
    const poll = async () => {
      try {
        const result = await invoke<SlotStatus[]>("get_all_statuses")
        setStatuses(result)
      } catch { /* ignore */ }
    }
    poll()
    const interval = setInterval(poll, 1000)
    return () => clearInterval(interval)
  }, [])

  // Capture errors from stopped slots
  const prevStatuses = useRef(statuses)
  useEffect(() => {
    for (let i = 0; i < MAX_SLOTS; i++) {
      const prev = prevStatuses.current[i]
      const curr = statuses[i]
      if (prev?.recording === true && curr?.recording === false && curr?.error) {
        setGlobalMsg(`Slot ${i + 1}: ${friendlyError(curr.error)}`)
      }
    }
    prevStatuses.current = statuses
  }, [statuses])

  const isDirectUrl = (url: string) =>
    url.endsWith(".m3u8") || url.endsWith(".mp4") || url.endsWith(".ts") ||
    url.startsWith("rtmp://") || url.startsWith("rtsp://")

  const generateDefaultName = useCallback((url: string) => {
    try {
      const u = new URL(url)
      const path = u.pathname.replace(/\/$/, "")
      const last = path.split("/").pop() || u.hostname.split(".").slice(-2, -1)[0] || "stream"
      const clean = last.replace(/[<>:"/\\|?*]/g, "_").slice(0, 50)
      const now = new Date()
      const pad = (n: number) => String(n).padStart(2, "0")
      const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
      const time = `${pad(now.getHours())}-${pad(now.getMinutes())}`
      return `${clean} ${date} ${time}`
    } catch {
      return ""
    }
  }, [])

  const doFetchFormats = useCallback(async (url: string) => {
    if (!url || isDirectUrl(url)) return
    setFetching(true)
    try {
      const result = await invoke<StreamFormat[]>("fetch_formats", { streamUrl: url })
      setFormats(result)
      if (result.length > 0) {
        setSelectedFormat(result[0].id)
        setSelectedFormatUrl(result[0].url)
        if (result[0].url) loadPreview(result[0].url)
      } else {
        setFormats([])
        setSelectedFormat("")
        setSelectedFormatUrl(null)
      }
    } catch {
      setFormats([])
      setSelectedFormat("")
      setSelectedFormatUrl(null)
    } finally {
      setFetching(false)
    }
  }, [])

  const handleUrlChange = useCallback((url: string) => {
    setStreamUrl(url)
    if (url) setFileName(generateDefaultName(url))
    clearTimeout(urlTimer.current)
    urlTimer.current = setTimeout(() => {
      if (url) doFetchFormats(url)
    }, 800)
  }, [doFetchFormats, generateDefaultName])

  const loadPreview = useCallback(async (url: string) => {
    if (!url) return
    setPreviewLoading(true)
    setPreviewData(null)
    try {
      const data = await invoke<string>("preview_stream", { url })
      setPreviewData(data)
    } catch {
      setPreviewData(null)
    } finally {
      setPreviewLoading(false)
    }
  }, [])

  const openModal = useCallback((slot: number) => {
    const existing = statuses[slot]
    if (existing.recording) return
    setModalSlot(slot)
    setStreamUrl(existing.url || "")
    setFileName("")
    setOutputFolder(localStorage.getItem("chronicle_output") || "")
    setFormats([])
    setSelectedFormat("")
    setSelectedFormatUrl(null)
    setPreviewData(null)
    setStarting(false)
    setCompressEnabled(false)
    setCompressCrf(23)
    setCompressPreset("medium")
  }, [statuses])

  const pickFolder = useCallback(async () => {
    const selected = await open({ directory: true, multiple: false })
    if (selected) {
      setOutputFolder(selected)
      localStorage.setItem("chronicle_output", selected)
    }
  }, [])

  const startRecording = useCallback(async () => {
    if (modalSlot === null || !streamUrl || !fileName || !outputFolder) {
      setGlobalMsg("Completa todos los campos")
      return
    }
    setStarting(true)
    const outputPath = `${outputFolder}\\${fileName}.mkv`
    const compress: CompressionConfig | null = compressEnabled ? { crf: compressCrf, preset: compressPreset } : null
    try {
      await invoke("start_recording", {
        slot: modalSlot,
        streamUrl,
        outputPath,
        formatId: selectedFormat || null,
        formatUrl: selectedFormatUrl || null,
        compress,
      })
      setModalSlot(null)
      setGlobalMsg(`Slot ${modalSlot + 1}: grabando`)
    } catch (e) {
      setGlobalMsg(`Error: ${e}`)
    } finally {
      setStarting(false)
    }
  }, [modalSlot, streamUrl, fileName, outputFolder, selectedFormat, selectedFormatUrl])

  const stopSlot = useCallback(async (slot: number) => {
    try {
      await invoke("stop_recording", { slot })
      setGlobalMsg(`Slot ${slot + 1}: detenido`)
    } catch (e) {
      setGlobalMsg(`Error: ${e}`)
    }
  }, [])

  const stopAll = useCallback(async () => {
    try {
      const msgs = await invoke<string[]>("stop_all_recordings")
      setGlobalMsg(msgs.join("; ") || "Todos detenidos")
    } catch (e) {
      setGlobalMsg(`Error: ${e}`)
    }
  }, [])

  const friendlyError = (raw: string): string => {
    if (raw.includes("ffmpeg exited with code")) return "Error de descarga"
    if (raw.includes("404") || raw.includes("HTTP error")) return "Error de conexión"
    if (raw.includes("Error opening input")) return "Stream no accesible"
    return raw.length > 80 ? raw.slice(0, 80) + "…" : raw
  }

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

  const anyRecording = statuses.some(s => s.recording)

  return (
    <TooltipProvider>
      <div className="min-h-screen bg-background text-foreground">
        {/* Top bar */}
        <header className="border-b border-border/40 bg-card/80 backdrop-blur-sm sticky top-0 z-40">
          <div className="max-w-5xl mx-auto flex items-center justify-between px-4 h-14">
            <div className="flex items-center gap-2.5">
              <Radio className="size-5 text-primary" />
              <h1 className="text-lg font-semibold tracking-tight">Chronicle</h1>
            </div>
            <div className="flex items-center gap-1">
              {anyRecording && (
                <Badge variant="destructive" className="gap-1.5 animate-pulse">
                  <span className="size-1.5 rounded-full bg-destructive" />
                  {statuses.filter(s => s.recording).length} activo(s)
                </Badge>
              )}
              <Tooltip>
                <TooltipTrigger>
                  <Button variant="ghost" size="icon-sm" onClick={cycleTheme}>
                    <ThemeIcon className="size-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {theme === "dark" ? "Modo oscuro" : theme === "light" ? "Modo claro" : "Sistema"}
                </TooltipContent>
              </Tooltip>
            </div>
          </div>
        </header>

        {/* Main content */}
        <main className="max-w-5xl mx-auto px-4 py-6 space-y-6">
          {/* Slot cards */}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            {statuses.map((s, i) => (
              <Card key={i} size="sm" className={s.recording ? "ring-green-500/40" : s.error ? "ring-destructive/30" : ""}>
                <CardHeader>
                  <div className="flex items-center justify-between w-full">
                    <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground">
                      Slot {i + 1}
                    </CardTitle>
                    {s.optimizing && (
                      <Badge variant="secondary" className="gap-1.5">
                        <Loader2 className="size-3 animate-spin" />
                        Optimizando
                      </Badge>
                    )}
                    {s.recording && !s.optimizing && (
                      <Badge variant="default" className="bg-green-600 hover:bg-green-600 gap-1">
                        <span className="size-1.5 rounded-full bg-white animate-pulse" />
                        Grabando
                      </Badge>
                    )}
                    {!s.recording && !s.optimizing && s.error && (
                      <Badge variant="destructive">Error</Badge>
                    )}
                    {!s.recording && !s.optimizing && !s.error && (
                      <Badge variant="outline" className="text-muted-foreground">Inactivo</Badge>
                    )}
                  </div>
                </CardHeader>
                <CardContent className="space-y-3">
                  {s.optimizing ? (
                    <div className="flex flex-col items-center gap-2 py-3">
                      <Loader2 className="size-8 text-muted-foreground/40 animate-spin" />
                      <span className="text-xs text-muted-foreground/60">Comprimiendo video…</span>
                    </div>
                  ) : s.recording ? (
                    <>
                      <div className="text-3xl font-mono font-bold tabular-nums text-center text-green-600 tracking-tight">
                        {fmt(s.elapsed_secs)}
                      </div>
                      {s.label && (
                        <div className="text-xs text-muted-foreground text-center truncate" title={s.url ?? undefined}>
                          {s.label}
                        </div>
                      )}
                    </>
                  ) : s.error ? (
                    <div className="text-xs text-destructive bg-destructive/10 rounded-lg p-2.5 leading-relaxed">
                      {friendlyError(s.error)}
                    </div>
                  ) : (
                    <div className="flex flex-col items-center gap-2 py-3">
                      <Video className="size-8 text-muted-foreground/40" />
                      <span className="text-xs text-muted-foreground/60">Sin grabación</span>
                    </div>
                  )}
                </CardContent>
                <CardFooter>
                  {s.optimizing ? (
                    <Button variant="secondary" size="sm" className="w-full gap-1.5" disabled>
                      <Loader2 className="size-3.5 animate-spin" />
                      Comprimiendo…
                    </Button>
                  ) : s.recording ? (
                    <Button variant="destructive" size="sm" className="w-full gap-1.5" onClick={() => stopSlot(i)}>
                      <Square className="size-3.5" />
                      Detener
                    </Button>
                  ) : (
                    <Button variant="outline" size="sm" className="w-full gap-1.5" onClick={() => openModal(i)}>
                      <Video className="size-3.5" />
                      {s.error ? "Reintentar" : "Configurar"}
                    </Button>
                  )}
                </CardFooter>
              </Card>
            ))}
          </div>

          {/* Stop All */}
          {anyRecording && (
            <Button variant="destructive" className="w-full gap-2" onClick={stopAll}>
              <Square className="size-4" />
              Detener Todas las Grabaciones
            </Button>
          )}

          {/* Status message */}
          {globalMsg && (
            <div className="text-sm text-center text-muted-foreground bg-muted/60 rounded-xl px-4 py-2.5 border border-border/50">
              {globalMsg}
            </div>
          )}
        </main>

        {/* Config dialog */}
        <Dialog open={modalSlot !== null} onOpenChange={(open) => { if (!open) { setModalSlot(null); setPreviewData(null) } }}>
          <DialogContent className="sm:max-w-md">
            <DialogHeader>
              <DialogTitle>Configurar Slot {modalSlot !== null ? modalSlot + 1 : ""}</DialogTitle>
              <DialogDescription>
                Ingresá la URL del stream y configurá la salida.
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-4 py-1 overflow-y-auto max-h-[55vh]">
              {/* URL */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground">URL del Stream</label>
                <Input
                  placeholder="https://twitch.tv/..."
                  value={streamUrl}
                  onChange={(e) => handleUrlChange(e.target.value)}
                />
              </div>

              {/* File name */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground">Nombre del archivo</label>
                <Input
                  placeholder="Mi Stream"
                  value={fileName}
                  onChange={(e) => setFileName(e.target.value)}
                />
              </div>

              {/* Output folder */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground">Carpeta de destino</label>
                <div className="flex gap-2">
                  <Input
                    placeholder="C:\Grabaciones"
                    value={outputFolder}
                    onChange={(e) => setOutputFolder(e.target.value)}
                    className="flex-1"
                  />
                  <Button variant="outline" size="icon-sm" onClick={pickFolder}>
                    <Folder className="size-4" />
                  </Button>
                </div>
              </div>

              {/* Compresión post-proceso */}
              <div className="space-y-2">
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={compressEnabled}
                    onChange={(e) => setCompressEnabled(e.target.checked)}
                    className="accent-primary"
                  />
                  <span className="text-xs font-medium text-muted-foreground">
                    Comprimir al finalizar (sin pérdida visible)
                  </span>
                </label>
                {compressEnabled && (
                  <div className="pl-5 space-y-2 pt-1">
                    <div className="flex items-center gap-3">
                      <span className="text-xs text-muted-foreground w-8">CRF</span>
                      <input
                        type="range"
                        min="18"
                        max="28"
                        value={compressCrf}
                        onChange={(e) => setCompressCrf(Number(e.target.value))}
                        className="flex-1 accent-primary"
                      />
                      <span className="text-xs font-mono text-muted-foreground w-6 text-right">{compressCrf}</span>
                    </div>
                    <div className="flex text-[11px] text-muted-foreground/60 justify-between px-1">
                      <span>Mejor calidad</span>
                      <span>Más compresión</span>
                    </div>
                    <div className="flex items-center gap-3">
                      <span className="text-xs text-muted-foreground w-8">Preset</span>
                      <select
                        value={compressPreset}
                        onChange={(e) => setCompressPreset(e.target.value)}
                        className="flex-1 h-7 rounded-md border border-input bg-background px-2 text-xs"
                      >
                        <option value="ultrafast">Ultra rápido</option>
                        <option value="superfast">Super rápido</option>
                        <option value="veryfast">Muy rápido</option>
                        <option value="faster">Más rápido</option>
                        <option value="fast">Rápido</option>
                        <option value="medium">Medio</option>
                        <option value="slow">Lento</option>
                        <option value="slower">Más lento</option>
                        <option value="veryslow">Muy lento</option>
                      </select>
                    </div>
                    <p className="text-[11px] text-muted-foreground/50 leading-relaxed">
                      CRF 18 = casi sin pérdida, 23 = buena calidad, 28 = archivo pequeño.
                      Preset más lento = mejor compresión a costa de CPU.
                    </p>
                  </div>
                )}
              </div>

              {/* Formatos */}
              {fetching && (
                <div className="space-y-2">
                  <Skeleton className="h-4 w-24" />
                  {[1, 2, 3].map(j => <Skeleton key={j} className="h-10 w-full rounded-lg" />)}
                </div>
              )}

              {previewData && (
                <div className="rounded-lg overflow-hidden border border-border/60">
                  <img src={`data:image/png;base64,${previewData}`} alt="Preview" className="w-full h-36 object-cover" />
                </div>
              )}
              {previewLoading && (
                <div className="text-xs text-center text-muted-foreground py-2">Capturando preview…</div>
              )}

              {formats.length > 0 && !fetching && (
                <div className="space-y-1.5">
                  <label className="text-xs font-medium text-muted-foreground">Calidad</label>
                  <ScrollArea className="max-h-48">
                    <div className="space-y-1 pr-1">
                      {formats.map((f) => (
                        <label
                          key={f.id}
                          className={`flex items-center gap-3 p-2.5 rounded-lg border cursor-pointer transition-colors text-sm ${
                            selectedFormat === f.id
                              ? "border-primary/50 bg-primary/5"
                              : "border-border hover:bg-muted/60"
                          }`}
                        >
                          <input
                            type="radio"
                            name="format"
                            value={f.id}
                            checked={selectedFormat === f.id}
                            onChange={() => { setSelectedFormat(f.id); setSelectedFormatUrl(f.url); if (f.url) loadPreview(f.url) }}
                            className="accent-primary"
                          />
                          <span className="font-mono text-xs">{labelForFormat(f)}</span>
                        </label>
                      ))}
                    </div>
                  </ScrollArea>
                </div>
              )}
            </div>

            <DialogFooter showCloseButton>
              <Button className="gap-1.5" onClick={startRecording} disabled={starting || !streamUrl || !fileName || !outputFolder}>
                {starting ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <Video className="size-3.5" />
                )}
                {starting ? "Iniciando…" : "Iniciar Grabación"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    </TooltipProvider>
  )
}

export default App
