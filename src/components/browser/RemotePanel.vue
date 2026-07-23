<script setup lang="ts">
import { ref, watch, computed, onMounted, onUnmounted } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open, save } from '@tauri-apps/plugin-dialog'
import { useTabsStore } from '@/stores/tabs'
import { useTransferStore } from '@/stores/transfer'
import { listDir, uploadFiles, downloadFiles, downloadFileAs, previewFile, saveBookmark } from '@/composables/useTauri'
import type { FileEntry, FilePreview } from '@/types/filesystem'
import type { Bookmark } from '@/types/connection'

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

type ViewMode = 'columns' | 'list'
const viewMode = ref<ViewMode>(
  (localStorage.getItem('viewMode') as ViewMode) === 'list' ? 'list' : 'columns'
)
const listEntries = ref<FileEntry[]>([])
const listLoading = ref(false)

function setViewMode(mode: ViewMode) {
  if (viewMode.value === mode) return
  viewMode.value = mode
  localStorage.setItem('viewMode', mode)
}
const ctxMenu = ref<{ visible: boolean; x: number; y: number; entry: FileEntry | null }>({
  visible: false,
  x: 0,
  y: 0,
  entry: null,
})
const preview = ref<{
  entry: FileEntry | null
  loading: boolean
  data: FilePreview | null
}>({ entry: null, loading: false, data: null })
const previewCtxMenu = ref<{ visible: boolean; x: number; y: number; hasSelection: boolean }>({
  visible: false,
  x: 0,
  y: 0,
  hasSelection: false,
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

// Selected files across the active view (for toolbar download)
const allEntries = computed(() =>
  viewMode.value === 'columns' ? columns.value.flatMap((c) => c.entries) : listEntries.value
)
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

async function loadList(path: string) {
  if (!sessionId.value) return
  listLoading.value = true
  try {
    listEntries.value = sortEntries(await listDir(sessionId.value, path))
  } catch (e) {
    console.error('Failed to list directory:', e)
  } finally {
    listLoading.value = false
  }
}

async function refresh() {
  if (viewMode.value === 'columns') {
    await buildColumns(currentPath.value)
  } else {
    await loadList(currentPath.value)
  }
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
    preview.value = { entry: null, loading: false, data: null }
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
    loadPreview(entry)
    scrollToEnd()
  }
}

const PREVIEWABLE_SIZE = 10 * 1024 * 1024 // don't preview files larger than 10MB

async function loadPreview(entry: FileEntry) {
  preview.value = { entry, loading: true, data: null }
  if (entry.size > PREVIEWABLE_SIZE || !sessionId.value) {
    preview.value = { entry, loading: false, data: null }
    return
  }
  try {
    const data = await previewFile(sessionId.value, entry.path)
    // Ignore stale responses after user clicked another file
    if (preview.value.entry?.path === entry.path) {
      preview.value = { entry, loading: false, data }
    }
  } catch (e) {
    console.error('Preview failed:', e)
    if (preview.value.entry?.path === entry.path) {
      preview.value = { entry, loading: false, data: null }
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

function formatDate(ts: number): string {
  if (!ts) return '-'
  return new Date(ts * 1000).toLocaleString()
}

// Explorer list view interactions
function onListClick(entry: FileEntry, event: MouseEvent) {
  if (event.ctrlKey || event.metaKey) {
    if (selectedPaths.value.has(entry.path)) {
      selectedPaths.value.delete(entry.path)
    } else {
      selectedPaths.value.add(entry.path)
    }
    return
  }
  selectedPaths.value.clear()
  selectedPaths.value.add(entry.path)
  if (entry.isDir) {
    preview.value = { entry: null, loading: false, data: null }
  } else {
    loadPreview(entry)
  }
}

function onListDblClick(entry: FileEntry) {
  if (entry.isDir) {
    navigateTo(entry.path)
  }
}

function onListContextMenu(entry: FileEntry, event: MouseEvent) {
  previewCtxMenu.value.visible = false
  if (!selectedPaths.value.has(entry.path)) {
    selectedPaths.value.clear()
    selectedPaths.value.add(entry.path)
  }
  ctxMenu.value = { visible: true, x: event.clientX, y: event.clientY, entry }
}

// Context menu
function onEntryContextMenu(colIndex: number, entry: FileEntry, event: MouseEvent) {
  previewCtxMenu.value.visible = false
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
  previewCtxMenu.value.visible = false
}

// Preview context menu (copy)
function onPreviewContextMenu(event: MouseEvent) {
  ctxMenu.value.visible = false
  const selection = window.getSelection()?.toString() ?? ''
  previewCtxMenu.value = {
    visible: true,
    x: event.clientX,
    y: event.clientY,
    hasSelection: selection.length > 0,
  }
}

async function copyToClipboard(text: string) {
  try {
    await navigator.clipboard.writeText(text)
  } catch (e) {
    console.error('Clipboard write failed:', e)
  }
}

async function copySelected() {
  const selection = window.getSelection()?.toString() ?? ''
  previewCtxMenu.value.visible = false
  if (selection) await copyToClipboard(selection)
}

async function copyAll() {
  previewCtxMenu.value.visible = false
  const content = preview.value.data?.content
  if (content) await copyToClipboard(content)
}

async function ctxDownload() {
  hideCtxMenu()
  const sid = sessionId.value
  const targets = selectedFiles.value.map((f) => f.path)
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
  if (!sid || !entry) return

  const target = await save({
    defaultPath: entry.name,
    title: entry.isDir ? 'Save Folder As' : 'Save As',
  })
  if (!target) return

  try {
    await downloadFileAs(sid, entry.path, target)
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Save As failed:', e)
  }
}

async function ctxAddBookmark() {
  const entry = ctxMenu.value.entry
  hideCtxMenu()
  const tab = tabsStore.activeTab
  const params = tab?.connectParams
  if (!tab || !params) return

  // Bookmark the folder itself, or the containing dir for files
  const path = entry?.isDir ? entry.path : currentPath.value
  const alias = prompt('Bookmark alias:', path)
  if (alias === null) return

  const bookmark: Bookmark = {
    id: crypto.randomUUID(),
    alias: alias.trim() || path,
    host: params.host,
    port: params.port,
    username: params.username,
    authMethod: params.auth.type,
    path,
  }
  if (params.auth.type === 'key') {
    bookmark.privateKeyPath = params.auth.key_path
    if (params.auth.passphrase) bookmark.passphrase = params.auth.passphrase
  } else if (params.auth.type === 'password') {
    bookmark.password = params.auth.password
  }

  try {
    await saveBookmark(bookmark)
  } catch (e) {
    console.error('Save bookmark failed:', e)
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
    preview.value = { entry: null, loading: false, data: null }
    if (viewMode.value === 'columns') {
      buildColumns(newPath)
    } else {
      loadList(newPath)
    }
  },
  { immediate: true }
)

watch(viewMode, () => {
  selectedPaths.value.clear()
  preview.value = { entry: null, loading: false, data: null }
  if (viewMode.value === 'columns') {
    buildColumns(currentPath.value)
  } else {
    loadList(currentPath.value)
  }
})
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
      <span class="path-spacer" />
      <div class="view-toggle">
        <button
          class="toggle-btn"
          :class="{ active: viewMode === 'columns' }"
          title="Column view (Finder)"
          @click="setViewMode('columns')"
        >
          ▦
        </button>
        <button
          class="toggle-btn"
          :class="{ active: viewMode === 'list' }"
          title="Details view (Explorer)"
          @click="setViewMode('list')"
        >
          ☰
        </button>
      </div>
    </div>

    <div class="body">
      <!-- Finder-style Miller columns -->
      <div v-if="viewMode === 'columns'" class="columns" ref="columnsEl">
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

      <!-- Windows Explorer-style details list -->
      <div v-else class="list-view">
        <div v-if="listLoading" class="col-loading">Loading...</div>
        <template v-else>
          <div class="file-header">
            <span class="col-name">Name</span>
            <span class="col-size">Size</span>
            <span class="col-perm">Permissions</span>
            <span class="col-date">Modified</span>
          </div>
          <div
            v-for="entry in listEntries"
            :key="entry.path"
            class="file-row"
            :class="{ selected: selectedPaths.has(entry.path) }"
            @click="onListClick(entry, $event)"
            @dblclick="onListDblClick(entry)"
            @contextmenu.prevent="onListContextMenu(entry, $event)"
          >
            <span class="col-name">
              <span class="entry-icon">{{ entry.isDir ? '📁' : '📄' }}</span>
              {{ entry.name }}
            </span>
            <span class="col-size">{{ entry.isDir ? '-' : formatSize(entry.size) }}</span>
            <span class="col-perm">{{ entry.permissions || '-' }}</span>
            <span class="col-date">{{ formatDate(entry.modified) }}</span>
          </div>
          <div v-if="listEntries.length === 0" class="col-empty">Empty directory</div>
        </template>
      </div>

      <!-- Preview pane for the selected file -->
      <div v-if="preview.entry" class="preview-col" @contextmenu.prevent="onPreviewContextMenu">
        <div class="preview-head">
          <span class="preview-icon">📄</span>
          <div class="preview-meta">
            <div class="preview-name" :title="preview.entry.name">{{ preview.entry.name }}</div>
            <div class="preview-info">{{ formatSize(preview.entry.size) }}</div>
          </div>
        </div>
        <div v-if="preview.loading" class="preview-status">Loading preview…</div>
        <template v-else-if="preview.data?.isText && preview.data.content !== null">
          <pre class="preview-text">{{ preview.data.content }}</pre>
          <div v-if="preview.data.truncated" class="preview-status">
            — preview truncated —
          </div>
        </template>
        <div v-else class="preview-status">No preview available (binary or large file)</div>
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
        :disabled="!ctxMenu.entry"
        @click="ctxSaveAs"
      >
        💾 Save As…
      </button>
      <button class="ctx-item" @click="ctxAddBookmark">
        ⭐ Add Bookmark{{ ctxMenu.entry && !ctxMenu.entry.isDir ? ' (folder)' : '' }}
      </button>
    </div>

    <!-- Preview copy menu -->
    <div
      v-if="previewCtxMenu.visible"
      class="ctx-menu"
      :style="{ left: previewCtxMenu.x + 'px', top: previewCtxMenu.y + 'px' }"
      @click.stop
    >
      <button class="ctx-item" :disabled="!previewCtxMenu.hasSelection" @click="copySelected">
        📋 Copy Selected
      </button>
      <button
        class="ctx-item"
        :disabled="!preview.data?.isText || !preview.data.content"
        @click="copyAll"
      >
        📄 Copy All
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

.path-spacer {
  flex: 1;
}

.view-toggle {
  display: flex;
  gap: 2px;
  flex-shrink: 0;
}

.toggle-btn {
  background: none;
  border: none;
  border-radius: 4px;
  color: #6c7086;
  cursor: pointer;
  padding: 2px 8px;
  font-size: 14px;
  line-height: 1.4;
}

.toggle-btn:hover {
  background: #313244;
  color: #cdd6f4;
}

.toggle-btn.active {
  background: #4f6ec2;
  color: #ffffff;
}

.body {
  flex: 1;
  display: flex;
  overflow: hidden;
  background: #181825;
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

/* Explorer-style details list */
.list-view {
  flex: 1;
  overflow-y: auto;
  background: #1e1e2e;
}

.file-header,
.file-row {
  display: grid;
  grid-template-columns: 1fr 90px 110px 170px;
  padding: 6px 14px;
  font-size: 13px;
  align-items: center;
}

.file-header {
  background: #181825;
  color: #6c7086;
  font-size: 12px;
  font-weight: 600;
  position: sticky;
  top: 0;
  z-index: 1;
  border-bottom: 1px solid #2a2a3d;
}

.file-row {
  cursor: pointer;
  color: #cdd6f4;
  transition: background 0.08s;
}

.file-row:hover {
  background: #28283c;
}

.file-row.selected {
  background: #4f6ec2;
  color: #ffffff;
}

.file-row.selected .col-size,
.file-row.selected .col-perm,
.file-row.selected .col-date {
  color: #c8d4f5;
}

.col-name {
  display: flex;
  align-items: center;
  gap: 7px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.col-size,
.col-perm,
.col-date {
  color: #6c7086;
  font-size: 12px;
}

.col-perm {
  font-family: 'Cascadia Code', Consolas, monospace;
}

.preview-col {
  width: 340px;
  height: 100%;
  overflow-y: auto;
  background: #1e1e2e;
  border-left: 1px solid #2a2a3d;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.preview-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 14px;
  border-bottom: 1px solid #2a2a3d;
  flex-shrink: 0;
}

.preview-icon {
  font-size: 26px;
}

.preview-meta {
  overflow: hidden;
}

.preview-name {
  font-size: 13px;
  font-weight: 600;
  color: #cdd6f4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-info {
  font-size: 11px;
  color: #6c7086;
  margin-top: 2px;
}

.preview-text {
  flex: 1;
  margin: 0;
  padding: 12px 14px;
  font-family: 'Cascadia Code', Consolas, monospace;
  font-size: 12px;
  line-height: 1.5;
  color: #cdd6f4;
  white-space: pre-wrap;
  word-break: break-all;
  overflow-y: auto;
}

.preview-status {
  padding: 14px;
  color: #6c7086;
  font-size: 12px;
  text-align: center;
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
