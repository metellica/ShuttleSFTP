<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { terminalOpen, terminalInput, terminalResize, terminalClose } from '@/composables/useTauri'

const props = defineProps<{
  sessionId: string
  path: string
  /** Kept mounted while hidden so the shell survives tab switches. */
  visible: boolean
}>()

const emit = defineEmits<{ exited: [] }>()

const termEl = ref<HTMLDivElement | null>(null)
const error = ref('')
const exited = ref(false)

let term: Terminal | null = null
let fit: FitAddon | null = null
let terminalId: string | null = null
let resizeObserver: ResizeObserver | null = null
const unlisteners: UnlistenFn[] = []

function b64encode(data: string): string {
  const bytes = new TextEncoder().encode(data)
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin)
}

function b64decode(data: string): Uint8Array {
  const bin = atob(data)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return bytes
}

onMounted(async () => {
  if (!termEl.value) return
  term = new Terminal({
    fontSize: 13,
    fontFamily: 'Consolas, "Cascadia Mono", Menlo, monospace',
    cursorBlink: true,
    theme: {
      background: '#181825',
      foreground: '#cdd6f4',
      cursor: '#cdd6f4',
      selectionBackground: '#45475a',
    },
  })
  fit = new FitAddon()
  term.loadAddon(fit)
  term.open(termEl.value)
  fit.fit()

  try {
    terminalId = await terminalOpen(props.sessionId, props.path, term.cols, term.rows)
  } catch (e: any) {
    error.value = e?.toString() || 'Cannot open terminal'
    return
  }
  const id = terminalId

  unlisteners.push(
    await listen<{ id: string; data: string }>('terminal:data', (e) => {
      if (e.payload.id === id) term?.write(b64decode(e.payload.data))
    })
  )
  unlisteners.push(
    await listen<{ id: string }>('terminal:exit', (e) => {
      if (e.payload.id === id) {
        exited.value = true
        term?.write('\r\n\x1b[90m[process exited]\x1b[0m\r\n')
        emit('exited')
      }
    })
  )

  term.onData((data) => {
    if (terminalId && !exited.value) {
      terminalInput(terminalId, b64encode(data)).catch(() => {})
    }
  })
  term.onResize(({ cols, rows }) => {
    if (terminalId && !exited.value) {
      terminalResize(terminalId, cols, rows).catch(() => {})
    }
  })

  // Refit on panel size changes (debounced by rAF).
  let raf = 0
  resizeObserver = new ResizeObserver(() => {
    cancelAnimationFrame(raf)
    raf = requestAnimationFrame(() => {
      if (props.visible) fit?.fit()
    })
  })
  resizeObserver.observe(termEl.value)
  if (props.visible) term.focus()
})

// Hidden terminals have zero size: refit and refocus once shown again.
watch(
  () => props.visible,
  async (vis) => {
    if (vis) {
      await nextTick()
      fit?.fit()
      term?.focus()
    }
  }
)

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  unlisteners.forEach((u) => u())
  if (terminalId) terminalClose(terminalId).catch(() => {})
  term?.dispose()
})
</script>

<template>
  <div class="term-view" v-show="visible">
    <div v-if="error" class="term-error">{{ error }}</div>
    <div v-else ref="termEl" class="term-host" />
  </div>
</template>

<style scoped>
.term-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.term-error {
  padding: 12px;
  color: #f38ba8;
  font-size: 12px;
}

.term-host {
  flex: 1;
  min-height: 0;
  padding: 4px 0 0 6px;
}

.term-host :deep(.xterm) {
  height: 100%;
}
</style>
