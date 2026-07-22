<script setup lang="ts">
import { ref, watch, computed, onMounted, onUnmounted } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open, save } from '@tauri-apps/plugin-dialog'
import { useTabsStore } from '@/stores/tabs'
import { useTransferStore } from '@/stores/transfer'
import { listDir, uploadFiles, downloadFiles, downloadFileAs } from '@/composables/useTauri'
import type { FileEntry } from '@/types/filesystem'

interface Column {
  /** Directory this column lists. */
  path: string
  entries: FileEntry[]
  loading: boolean
  /** Path of the entry selected inside this column (drives the next column). */
  selectedPath: string | null
}

const tabsStore = useTabsStore()
const transferStore = useTransferStore()
const columns = ref<Column[]>([])
const dragOver = ref(false)
const selectedPaths = ref<Set<string>>(new Set())
const columnsEl = ref<HTMLElement | null>(null)
const ctxMenu = ref<{ visible: boolean; x: number; y: number; entry: FileEntry | null }>({
  visible: false,
  x: 0,
  y: 0,
  entry: null,
})

const currentPath = computed(() => tabsStore.activeTab?.currentPath || '/')
const sessionId = computed(() => tabsStore.activeTab?.sessionId || '')

// Breadcrumb segments for the path bar
const breadcrumbs = computed(() => {
  const parts = currentPath.value.split('/').filter(Boolean)
  const crumbs = [{ name: '/', path: '/' }]
  let acc = ''
  for (const part of parts) {
    acc += '/' + part
    crumbs.push({ name: part, path: acc })
  }
  return crumbs
})

// Selected files across all columns (for toolbar download)
const allEntries = computed(() => columns.value.flatMap((c) => c.entries))
const selectedFiles = computed(() =>
  allEntries.value.filter((f) => selectedPaths.value.has(f.path))
)
defineExpose({ selectedFiles, refresh })

function sortEntries(entries: FileEntry[]): FileEntry[] {
  return entries.sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
    return a.name.localeCompare(b.name)
  })
}

async function loadColumn(path: string): Promise<Column> {
  const col: Column = { path, entries: [], loading: true, selectedPath: null }
  try {
    col.entries = sortEntries(await listDir(sessionId.value, path))
  } catch (e) {
    console.error('Failed to list directory:', e)
  } finally {
    col.loading = false
  }
  return col
}

/** Rebuild all columns for the given path: one column per ancestor directory. */
async function buildColumns(path: string) {
  if (!sessionId.value) return
  const parts = path.split('/').filter(Boolean)
  const dirs = ['/']
  let acc = ''
  for (const part of parts) {
    acc += '/' + part
    dirs.push(acc)
  }

  const loaded = await Promise.all(dirs.map((d) => loadColumn(d)))
  // Mark each column's selected entry to point at the next directory
  for (let i = 0; i < loaded.length - 1; i++) {
    const col = loaded[i]
    const next = dirs[i + 1]
    if (col && next) col.selectedPath = next
  }
  columns.value = loaded
  scrollToEnd()
}

function scrollToEnd() {
  requestAnimationFrame(() => {
    columnsEl.value?.scrollTo({ left: columnsEl.value.scrollWidth, behavior: 'smooth' })
  })
}

async function refresh() {
  await buildColumns(currentPath.value)
}

function navigateTo(path: string) {
  if (tabsStore.activeTab) {
    tabsStore.updateTab(tabsStore.activeTab.id, { currentPath: path })
  }
}

async function onEntryClick(colIndex: number, entry: FileEntry, event: MouseEvent) {
  const col = columns.value[colIndex]
  if (!col) return

  if (event.ctrlKey || event.metaKey) {
    // Multi-select toggle without changing columns
    if (selectedPaths.value.has(entry.path)) {
      selectedPaths.value.delete(entry.path)
    } else {
      selectedPaths.value.add(entry.path)
    }
    return
  }

  selectedPaths.value.clear()
  selectedPaths.value.add(entry.path)
  col.selectedPath = entry.path

  if (entry.isDir) {
    // Trim deeper columns and append the new directory column
    columns.value = columns.value.slice(0, colIndex + 1)
    const newCol = await loadColumn(entry.path)
    columns.value.push(newCol)
    if (tabsStore.activeTab) {
      // Update path without triggering a full rebuild
      suppressWatch = true
      tabsStore.updateTab(tabsStore.activeTab.id, { currentPath: entry.path })
    }
    scrollToEnd()
  } else {
    // Selecting a file: current dir is the column's dir
    columns.value = columns.value.slice(0, colIndex + 1)
    if (tabsStore.activeTab && tabsStore.activeTab.currentPath !== col.path) {
      suppressWatch = true
      tabsStore.updateTab(tabsStore.activeTab.id, { currentPath: col.path })
    }
  }
}

function formatSize(bytes: number): string {
  if (bytes === 0) return '-'
  const units = ['B', 'KB', 'MB', 'GB']
  let i = 0
  let size = bytes
  while (size >= 1024 && i < units.length - 1) {
    size /= 1024
    i++
  }
  return `${size.toFixed(i === 0 ? 0 : 1)} ${units[i]}`
}

// Context menu
function onEntryContextMenu(colIndex: number, entry: FileEntry, event: MouseEvent) {
  if (!selectedPaths.value.has(entry.path)) {
    selectedPaths.value.clear()
    selectedPaths.value.add(entry.path)
    const col = columns.value[colIndex]
    if (col) col.selectedPath = entry.path
  }
  ctxMenu.value = { visible: true, x: event.clientX, y: event.clientY, entry }
}

function hideCtxMenu() {
  ctxMenu.value.visible = false
}

async function ctxDownload() {
  hideCtxMenu()
  const sid = sessionId.value
  const targets = selectedFiles.value.filter((f) => !f.isDir).map((f) => f.path)
  if (!sid || targets.length === 0) return

  const dir = await open({ directory: true, title: 'Choose download location' })
  if (!dir) return

  try {
    await downloadFiles(sid, targets, dir as string)
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Download failed:', e)
  }
}

async function ctxSaveAs() {
  hideCtxMenu()
  const entry = ctxMenu.value.entry
  const sid = sessionId.value
  if (!sid || !entry || entry.isDir) return

  const target = await save({ defaultPath: entry.name, title: 'Save As' })
  if (!target) return

  try {
    await downloadFileAs(sid, entry.path, target)
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Save As failed:', e)
  }
}

// Drag & drop upload using Tauri 2 native API
let unlistenDragDrop: (() => void) | null = null
let suppressWatch = false

onMounted(async () => {
  window.addEventListener('click', hideCtxMenu)
  unlistenDragDrop = await getCurrentWebview().onDragDropEvent(async (event) => {
    if (event.payload.type === 'enter' || event.payload.type === 'over') {
      dragOver.value = true
    } else if (event.payload.type === 'leave') {
      dragOver.value = false
    } else if (event.payload.type === 'drop') {
      dragOver.value = false
      const paths = event.payload.paths
      if (paths.length > 0 && sessionId.value) {
        try {
          await uploadFiles(sessionId.value, paths, currentPath.value)
          await transferStore.syncTasks()
        } catch (e) {
          console.error('Upload failed:', e)
        }
      }
    }
  })
})

onUnmounted(() => {
  window.removeEventListener('click', hideCtxMenu)
  unlistenDragDrop?.()
})

watch(
  currentPath,
  (newPath) => {
    if (suppressWatch) {
      suppressWatch = false
      return
    }
    selectedPaths.value.clear()
    buildColumns(newPath)
  },
  { immediate: true }
)
</script>

<template>
  <div class="remote-panel" :class="{ 'drag-over': dragOver }">
    <!-- Breadcrumb path bar -->
    <div class="path-bar">
      <template v-for="(crumb, i) in breadcrumbs" :key="crumb.path">
        <span v-if="i > 0" class="crumb-sep">›</span>
        <button
          class="crumb"
          :class="{ current: i === breadcrumbs.length - 1 }"
          @click="navigateTo(crumb.path)"
        >
          {{ crumb.name }}
        </button>
      </template>
    </div>

    <!-- Finder-style Miller columns -->
    <div class="columns" ref="columnsEl">
      <div v-for="(col, colIndex) in columns" :key="col.path" class="column">
        <div v-if="col.loading" class="col-loading">Loading...</div>
        <template v-else>
          <div
            v-for="entry in col.entries"
            :key="entry.path"
            class="entry"
            :class="{
              selected: selectedPaths.has(entry.path),
              opened: col.selectedPath === entry.path,
            }"
            @click="onEntryClick(colIndex, entry, $event)"
            @contextmenu.prevent="onEntryContextMenu(colIndex, entry, $event)"
          >
            <span class="entry-icon">{{ entry.isDir ? '📁' : '📄' }}</span>
            <span class="entry-name" :title="entry.name">{{ entry.name }}</span>
            <span v-if="!entry.isDir" class="entry-size">{{ formatSize(entry.size) }}</span>
            <span v-else class="entry-arrow">›</span>
          </div>
          <div v-if="col.entries.length === 0" class="col-empty">Empty</div>
        </template>
      </div>
    </div>

    <!-- Drop overlay -->
    <div v-if="dragOver" class="drop-overlay">
      <p>Drop files here to upload</p>
    </div>

    <!-- Context menu -->
    <div
      v-if="ctxMenu.visible"
      class="ctx-menu"
      :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
      @click.stop
    >
      <button class="ctx-item" @click="ctxDownload">⬇ Download…</button>
      <button
        class="ctx-item"
        :disabled="!ctxMenu.entry || ctxMenu.entry.isDir"
        @click="ctxSaveAs"
      >
        💾 Save As…
      </button>
    </div>
  </div>
</template>

<style scoped>
.remote-panel {
  height: 100%;
  display: flex;
  flex-direction: column;
  position: relative;
}

.path-bar {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 8px 12px;
  background: #181825;
  border-bottom: 1px solid #2a2a3d;
  overflow-x: auto;
  white-space: nowrap;
  flex-shrink: 0;
  scrollbar-width: none;
}

.path-bar::-webkit-scrollbar {
  display: none;
}

.crumb {
  background: none;
  border: none;
  border-radius: 4px;
  color: #89b4fa;
  cursor: pointer;
  padding: 2px 6px;
  font-size: 13px;
  font-family: monospace;
}

.crumb:hover {
  background: #313244;
}

.crumb.current {
  color: #cdd6f4;
  font-weight: 600;
}

.crumb-sep {
  color: #6c7086;
  font-size: 12px;
}

.columns {
  flex: 1;
  display: flex;
  overflow-x: auto;
  overflow-y: hidden;
  background: #181825;
}

.column {
  min-width: 230px;
  max-width: 280px;
  height: 100%;
  overflow-y: auto;
  overflow-x: hidden;
  border-right: 1px solid #2a2a3d;
  flex-shrink: 0;
  padding: 6px 0;
  background: #1e1e2e;
}

.entry {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 5px 12px;
  font-size: 13px;
  cursor: pointer;
  border-radius: 5px;
  margin: 1px 6px;
  color: #cdd6f4;
  transition: background 0.08s;
}

.entry:hover {
  background: #28283c;
}

/* Active selection: Finder-style accent */
.entry.selected {
  background: #4f6ec2;
  color: #ffffff;
}

.entry.selected .entry-size,
.entry.selected .entry-arrow {
  color: #c8d4f5;
}

/* Ancestor columns on the opened path: muted highlight */
.entry.opened:not(.selected) {
  background: #313244;
  color: #cdd6f4;
}

.entry.opened:not(.selected) .entry-arrow {
  color: #89b4fa;
}

.entry-icon {
  font-size: 14px;
  flex-shrink: 0;
}

.entry-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.entry-size {
  color: #6c7086;
  font-size: 11px;
  flex-shrink: 0;
}

.entry-arrow {
  color: #6c7086;
  font-size: 12px;
  flex-shrink: 0;
}

.col-loading,
.col-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 60px;
  color: #6c7086;
  font-size: 12px;
}

.drop-overlay {
  position: absolute;
  inset: 0;
  background: rgba(137, 180, 250, 0.1);
  border: 2px dashed #89b4fa;
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 10;
  pointer-events: none;
}

.drop-overlay p {
  font-size: 16px;
  color: #89b4fa;
  font-weight: 600;
}

.remote-panel.drag-over {
  border: 2px solid #89b4fa;
}

.ctx-menu {
  position: fixed;
  z-index: 100;
  min-width: 160px;
  background: #24243a;
  border: 1px solid #45475a;
  border-radius: 6px;
  padding: 4px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  display: flex;
  flex-direction: column;
}

.ctx-item {
  display: block;
  width: 100%;
  text-align: left;
  background: none;
  border: none;
  color: #cdd6f4;
  font-size: 13px;
  padding: 6px 10px;
  border-radius: 4px;
  cursor: pointer;
}

.ctx-item:hover:not(:disabled) {
  background: #45475a;
}

.ctx-item:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
