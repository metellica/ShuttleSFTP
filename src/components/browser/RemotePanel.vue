<script setup lang="ts">
import { ref, watch, computed, onMounted, onUnmounted, nextTick } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open, save, ask, message } from '@tauri-apps/plugin-dialog'
import { useTabsStore } from '@/stores/tabs'
import { useTransferStore } from '@/stores/transfer'
import { listDir, uploadFiles, downloadFiles, downloadFileAs, previewFile, saveFileContent, saveBookmark, removeEntry, transferRemote } from '@/composables/useTauri'
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
/** Anchor for shift+click range selection (colIndex is null in list view). */
const selectionAnchor = ref<{ colIndex: number | null; path: string } | null>(null)
const bodyEl = ref<HTMLElement | null>(null)

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
/** Expanded state of the "Copy to" submenu inside the context menu. */
const ctxCopyToOpen = ref(false)
const preview = ref<{
  entry: FileEntry | null
  loading: boolean
  data: FilePreview | null
}>({ entry: null, loading: false, data: null })
const previewMaximized = ref(false)
const previewEdit = ref<{ active: boolean; text: string; saving: boolean }>({
  active: false,
  text: '',
  saving: false,
})
const previewDirty = computed(
  () => previewEdit.value.active && previewEdit.value.text !== (preview.value.data?.content ?? '')
)

// Line numbers for preview and editor
const previewLines = computed(() => (preview.value.data?.content ?? '').split('\n'))
const editorEl = ref<HTMLTextAreaElement | null>(null)
const gutterEl = ref<HTMLElement | null>(null)
const mirrorEl = ref<HTMLElement | null>(null)
const editLineHeights = ref<number[]>([])

const gutterWidth = computed(() => {
  const count = previewEdit.value.active
    ? editLineHeights.value.length
    : previewLines.value.length
  const digits = Math.max(2, String(Math.max(1, count)).length)
  return `calc(${digits}ch + 18px)`
})

/** Measure the rendered height of each logical line (soft wrap aware) via a hidden mirror. */
function recomputeLineHeights() {
  const ta = editorEl.value
  const mirror = mirrorEl.value
  if (!ta || !mirror) return
  mirror.style.width = ta.clientWidth + 'px'
  mirror.textContent = ''
  const lines = previewEdit.value.text.split('\n')
  const frag = document.createDocumentFragment()
  for (const line of lines) {
    const div = document.createElement('div')
    div.textContent = line.length > 0 ? line : ' '
    frag.appendChild(div)
  }
  mirror.appendChild(frag)
  const heights: number[] = []
  for (const child of Array.from(mirror.children)) {
    heights.push((child as HTMLElement).offsetHeight)
  }
  editLineHeights.value = heights
  mirror.textContent = ''
  syncGutterScroll()
}

let recomputeTimer: number | undefined
function scheduleRecompute() {
  if (recomputeTimer !== undefined) clearTimeout(recomputeTimer)
  recomputeTimer = window.setTimeout(recomputeLineHeights, 60)
}

function syncGutterScroll() {
  if (gutterEl.value && editorEl.value) {
    gutterEl.value.scrollTop = editorEl.value.scrollTop
  }
}

watch(
  () => previewEdit.value.text,
  () => {
    if (previewEdit.value.active) scheduleRecompute()
  }
)
watch(
  () => previewEdit.value.active,
  (active) => {
    if (active) {
      nextTick(recomputeLineHeights)
    } else {
      editLineHeights.value = []
    }
  }
)
watch(previewMaximized, () => {
  if (previewEdit.value.active) nextTick(recomputeLineHeights)
})
const previewCtxMenu = ref<{ visible: boolean; x: number; y: number; hasSelection: boolean }>({
  visible: false,
  x: 0,
  y: 0,
  hasSelection: false,
})

const currentPath = computed(() => tabsStore.activeTab?.currentPath || '/')
const sessionId = computed(() => tabsStore.activeTab?.sessionId || '')

// Editable path bar state
const pathEdit = ref<{ active: boolean; value: string }>({ active: false, value: '' })
const pathInputEl = ref<HTMLInputElement | null>(null)
const pathCtxMenu = ref<{ visible: boolean; x: number; y: number }>({ visible: false, x: 0, y: 0 })

function normalizePath(p: string): string {
  let path = p.trim().replace(/\\/g, '/')
  if (!path.startsWith('/')) path = '/' + path
  path = path.replace(/\/+/g, '/')
  if (path.length > 1 && path.endsWith('/')) path = path.slice(0, -1)
  return path
}

function startPathEdit() {
  pathEdit.value = { active: true, value: currentPath.value }
  nextTick(() => {
    pathInputEl.value?.focus()
    pathInputEl.value?.select()
  })
}

function commitPathEdit() {
  const target = pathEdit.value.value.trim()
  pathEdit.value.active = false
  if (target && normalizePath(target) !== currentPath.value) {
    navigateTo(normalizePath(target))
  }
}

function cancelPathEdit() {
  pathEdit.value.active = false
}

function onPathBarContextMenu(event: MouseEvent) {
  ctxMenu.value.visible = false
  previewCtxMenu.value.visible = false
  pathCtxMenu.value = { visible: true, x: event.clientX, y: event.clientY }
}

async function copyPath() {
  pathCtxMenu.value.visible = false
  await copyToClipboard(currentPath.value)
}

async function pastePathAndGo() {
  pathCtxMenu.value.visible = false
  try {
    const text = (await navigator.clipboard.readText()).trim()
    if (text) navigateTo(normalizePath(text))
  } catch (e) {
    console.error('Clipboard read failed:', e)
  }
}

function ctxEditPath() {
  pathCtxMenu.value.visible = false
  startPathEdit()
}

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

  let loaded = await Promise.all(dirs.map((d) => loadColumn(d)))

  // If the last segment is actually a file (e.g. a file path typed/pasted in
  // the path bar), don't show a broken empty column for it: select it in its
  // parent column and open the preview instead.
  if (loaded.length >= 2) {
    const parent = loaded[loaded.length - 2]
    const lastPath = dirs[dirs.length - 1]
    const fileEntry = parent?.entries.find((e) => e.path === lastPath && !e.isDir)
    if (parent && fileEntry) {
      loaded = loaded.slice(0, -1)
      dirs.pop()
      parent.selectedPath = fileEntry.path
      selectedPaths.value.clear()
      selectedPaths.value.add(fileEntry.path)
      selectionAnchor.value = { colIndex: loaded.length - 1, path: fileEntry.path }
      if (tabsStore.activeTab && tabsStore.activeTab.currentPath !== parent.path) {
        suppressWatch = true
        tabsStore.updateTab(tabsStore.activeTab.id, { currentPath: parent.path })
      }
      loadPreview(fileEntry)
    }
  }

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
    bodyEl.value?.scrollTo({ left: bodyEl.value.scrollWidth, behavior: 'smooth' })
  })
}

async function loadList(path: string) {
  if (!sessionId.value) return
  listLoading.value = true
  try {
    listEntries.value = sortEntries(await listDir(sessionId.value, path))
  } catch (e) {
    // The path may point at a file: fall back to its parent and preview it
    const parentDir = path.slice(0, path.lastIndexOf('/')) || '/'
    try {
      const entries = sortEntries(await listDir(sessionId.value, parentDir))
      const fileEntry = entries.find((en) => en.path === path && !en.isDir)
      if (!fileEntry) throw e
      listEntries.value = entries
      selectedPaths.value.clear()
      selectedPaths.value.add(fileEntry.path)
      selectionAnchor.value = { colIndex: null, path: fileEntry.path }
      if (tabsStore.activeTab && tabsStore.activeTab.currentPath !== parentDir) {
        suppressWatch = true
        tabsStore.updateTab(tabsStore.activeTab.id, { currentPath: parentDir })
      }
      loadPreview(fileEntry)
    } catch {
      console.error('Failed to list directory:', e)
    }
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

/** Select the range between the anchor and the clicked entry within one entry list. */
function selectRange(
  entries: FileEntry[],
  entry: FileEntry,
  anchorPath: string | null,
  additive: boolean
) {
  const clickedIdx = entries.findIndex((e) => e.path === entry.path)
  let anchorIdx = anchorPath ? entries.findIndex((e) => e.path === anchorPath) : -1
  if (anchorIdx === -1) anchorIdx = clickedIdx
  if (!additive) selectedPaths.value.clear()
  const lo = Math.min(anchorIdx, clickedIdx)
  const hi = Math.max(anchorIdx, clickedIdx)
  for (let i = lo; i <= hi; i++) {
    const e = entries[i]
    if (e) selectedPaths.value.add(e.path)
  }
}

async function onEntryClick(colIndex: number, entry: FileEntry, event: MouseEvent) {
  const col = columns.value[colIndex]
  if (!col) return

  if (event.shiftKey) {
    // Range select within this column, anchored at the last non-shift click
    const anchor = selectionAnchor.value
    const anchorPath = anchor && anchor.colIndex === colIndex ? anchor.path : null
    selectRange(col.entries, entry, anchorPath, event.ctrlKey || event.metaKey)
    return
  }

  if (event.ctrlKey || event.metaKey) {
    // Multi-select toggle without changing columns
    if (selectedPaths.value.has(entry.path)) {
      selectedPaths.value.delete(entry.path)
    } else {
      selectedPaths.value.add(entry.path)
      selectionAnchor.value = { colIndex, path: entry.path }
    }
    return
  }

  selectedPaths.value.clear()
  selectedPaths.value.add(entry.path)
  selectionAnchor.value = { colIndex, path: entry.path }
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
  previewEdit.value = { active: false, text: '', saving: false }
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

async function loadFullPreview() {
  const entry = preview.value.entry
  if (!entry || !sessionId.value || preview.value.loading) return
  preview.value = { entry, loading: true, data: preview.value.data }
  try {
    const data = await previewFile(sessionId.value, entry.path, true)
    if (preview.value.entry?.path === entry.path) {
      preview.value = { entry, loading: false, data: { ...data, truncated: false } }
    }
  } catch (e) {
    console.error('Full preview failed:', e)
    if (preview.value.entry?.path === entry.path) {
      preview.value = { entry, loading: false, data: preview.value.data }
    }
  }
}

const EDITABLE_SIZE = 2 * 1024 * 1024 // don't edit files larger than 2MB

const previewEditable = computed(() => {
  const p = preview.value
  return (
    !!p.entry &&
    !p.loading &&
    !!p.data?.isText &&
    p.data.content !== null &&
    p.entry.size <= EDITABLE_SIZE
  )
})

async function startPreviewEdit() {
  if (!previewEditable.value) return
  if (preview.value.data?.truncated) {
    await loadFullPreview()
  }
  const content = preview.value.data?.content
  if (content === null || content === undefined) return
  previewEdit.value = { active: true, text: content, saving: false }
}

function cancelPreviewEdit() {
  if (previewDirty.value && !confirm('Discard unsaved changes?')) return
  previewEdit.value = { active: false, text: '', saving: false }
}

async function savePreviewEdit() {
  const entry = preview.value.entry
  if (!entry || !sessionId.value || !previewEdit.value.active || previewEdit.value.saving) return
  previewEdit.value.saving = true
  try {
    await saveFileContent(sessionId.value, entry.path, previewEdit.value.text)
    if (preview.value.data) {
      preview.value.data = {
        ...preview.value.data,
        content: previewEdit.value.text,
        truncated: false,
      }
    }
    previewEdit.value = { active: false, text: '', saving: false }
    await refresh() // pick up the new file size in listings
  } catch (e) {
    console.error('Save failed:', e)
    alert('Failed to save file: ' + e)
    previewEdit.value.saving = false
  }
}

function togglePreviewMaximized() {
  previewMaximized.value = !previewMaximized.value
}

// Reset maximize/edit state whenever the preview pane closes
watch(
  () => preview.value.entry,
  (entry) => {
    if (!entry) {
      previewMaximized.value = false
      previewEdit.value = { active: false, text: '', saving: false }
    }
  }
)

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
  if (event.shiftKey) {
    const anchor = selectionAnchor.value
    const anchorPath = anchor && anchor.colIndex === null ? anchor.path : null
    selectRange(listEntries.value, entry, anchorPath, event.ctrlKey || event.metaKey)
    return
  }
  if (event.ctrlKey || event.metaKey) {
    if (selectedPaths.value.has(entry.path)) {
      selectedPaths.value.delete(entry.path)
    } else {
      selectedPaths.value.add(entry.path)
      selectionAnchor.value = { colIndex: null, path: entry.path }
    }
    return
  }
  selectedPaths.value.clear()
  selectedPaths.value.add(entry.path)
  selectionAnchor.value = { colIndex: null, path: entry.path }
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
  ctxCopyToOpen.value = false
  previewCtxMenu.value.visible = false
  pathCtxMenu.value.visible = false
}

// Preview context menu (copy)
function onPreviewContextMenu(event: MouseEvent) {
  // Keep the native menu (cut/copy/paste/undo) inside the editor textarea
  if ((event.target as HTMLElement | null)?.tagName === 'TEXTAREA') return
  event.preventDefault()
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

/** Other connected tabs that can receive a cross-endpoint copy. */
const copyToTargets = computed(() =>
  tabsStore.tabs.filter(
    (t) => t.status === 'connected' && t.sessionId && t.sessionId !== sessionId.value
  )
)

const KIND_ICONS: Record<string, string> = { ssh: '⌁', container: '▣', pod: '⎈' }

async function ctxCopyToTab(dstSessionId: string, dstDir: string) {
  hideCtxMenu()
  const sid = sessionId.value
  const targets = selectedFiles.value.map((f) => f.path)
  if (!sid || targets.length === 0) return
  try {
    await transferRemote(sid, targets, dstSessionId, dstDir)
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Copy to session failed:', e)
    await message(`Copy failed: ${e}`, { title: 'Copy to', kind: 'error' })
  }
}

/** Drag files out of the panel: payload consumed by tabs (cross-session copy). */
function onEntryDragStart(entry: FileEntry, event: DragEvent) {
  if (!selectedPaths.value.has(entry.path)) {
    selectedPaths.value.clear()
    selectedPaths.value.add(entry.path)
  }
  const payload = JSON.stringify({
    sessionId: sessionId.value,
    paths: selectedFiles.value.map((f) => f.path),
  })
  event.dataTransfer?.setData('application/x-shuttle-files', payload)
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copy'
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

async function ctxDelete() {
  hideCtxMenu()
  const sid = sessionId.value
  const targets = selectedFiles.value
  if (!sid || targets.length === 0) return

  const first = targets[0]
  const what =
    targets.length === 1 && first
      ? `"${first.name}"`
      : `${targets.length} selected items`
  const confirmed = await ask(`Delete ${what}? This cannot be undone.`, {
    title: 'Confirm Delete',
    kind: 'warning',
    okLabel: 'Delete',
    cancelLabel: 'Cancel',
  })
  if (!confirmed) return

  try {
    for (const f of targets) {
      await removeEntry(sid, f.path, f.isDir)
      if (preview.value.entry?.path === f.path) {
        preview.value = { entry: null, loading: false, data: null }
        previewEdit.value = { active: false, text: '', saving: false }
      }
    }
  } catch (e) {
    console.error('Delete failed:', e)
    await message(`Delete failed: ${e}`, { title: 'Delete', kind: 'error' })
  }
  selectedPaths.value.clear()
  selectionAnchor.value = null
  await refresh()
}

async function ctxAddBookmark() {
  const entry = ctxMenu.value.entry
  hideCtxMenu()
  const tab = tabsStore.activeTab
  if (!tab) return
  const params = tab.connectParams
  if (tab.kind === 'ssh' && !params) return

  // Bookmark the folder itself, or the containing dir for files
  const path = entry?.isDir ? entry.path : currentPath.value
  const alias = prompt('Bookmark alias:', path)
  if (alias === null) return

  const bookmark: Bookmark = {
    id: crypto.randomUUID(),
    alias: alias.trim() || path,
    host: params?.host ?? 'local',
    port: params?.port ?? 0,
    username: params?.username ?? '',
    authMethod: params ? params.auth.type : 'agent',
    path,
    kind: tab.kind,
  }
  if (params?.auth.type === 'key') {
    bookmark.privateKeyPath = params.auth.key_path
    if (params.auth.passphrase) bookmark.passphrase = params.auth.passphrase
  } else if (params?.auth.type === 'password') {
    bookmark.password = params.auth.password
  }
  if (tab.kind === 'container' && tab.containerSpec) {
    bookmark.container = {
      runtime: tab.containerSpec.runtime,
      containerId: tab.containerSpec.containerId,
      name: tab.containerSpec.name,
    }
  } else if (tab.kind === 'pod' && tab.podSpec) {
    bookmark.pod = {
      context: tab.podSpec.context,
      namespace: tab.podSpec.namespace,
      pod: tab.podSpec.pod,
      container: tab.podSpec.container,
    }
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
  window.addEventListener('resize', scheduleRecompute)
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
  window.removeEventListener('resize', scheduleRecompute)
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
    selectionAnchor.value = null
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
  selectionAnchor.value = null
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
    <div class="path-bar" @contextmenu.prevent="onPathBarContextMenu">
      <template v-if="pathEdit.active">
        <input
          ref="pathInputEl"
          v-model="pathEdit.value"
          class="path-input"
          spellcheck="false"
          @keydown.enter="commitPathEdit"
          @keydown.esc="cancelPathEdit"
          @blur="cancelPathEdit"
        />
      </template>
      <template v-else>
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
        <span class="path-spacer" title="Click to edit path" @click="startPathEdit" />
        <button class="toggle-btn" title="Copy path" @click="copyPath">📋</button>
        <button class="toggle-btn" title="Edit path" @click="startPathEdit">✏️</button>
      </template>
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

    <div class="body" ref="bodyEl">
      <!-- Finder-style Miller columns -->
      <div v-if="viewMode === 'columns'" v-show="!previewMaximized" class="columns">
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
              draggable="true"
              @dragstart="onEntryDragStart(entry, $event)"
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
      <div v-else v-show="!previewMaximized" class="list-view">
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
            draggable="true"
            @dragstart="onEntryDragStart(entry, $event)"
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
      <div
        v-if="preview.entry"
        class="preview-col"
        :class="{ maximized: previewMaximized }"
        @contextmenu="onPreviewContextMenu"
      >
        <div class="preview-head">
          <span class="preview-icon">📄</span>
          <div class="preview-meta">
            <div class="preview-name" :title="preview.entry.name">{{ preview.entry.name }}</div>
            <div class="preview-info">
              {{ formatSize(preview.entry.size) }}<span v-if="previewDirty"> · modified</span>
            </div>
          </div>
          <span class="preview-spacer" />
          <div class="preview-actions">
            <template v-if="previewEdit.active">
              <button
                class="toggle-btn"
                :disabled="previewEdit.saving || !previewDirty"
                title="Save (Ctrl+S)"
                @click="savePreviewEdit"
              >
                💾
              </button>
              <button class="toggle-btn" title="Cancel editing" @click="cancelPreviewEdit">✕</button>
            </template>
            <button
              v-else
              class="toggle-btn"
              :disabled="!previewEditable"
              title="Edit file"
              @click="startPreviewEdit"
            >
              ✏️
            </button>
            <button
              class="toggle-btn"
              :title="previewMaximized ? 'Restore' : 'Maximize'"
              @click="togglePreviewMaximized"
            >
              {{ previewMaximized ? '🗗' : '🗖' }}
            </button>
          </div>
        </div>
        <div v-if="preview.loading" class="preview-status">Loading preview…</div>
        <template v-else-if="previewEdit.active">
          <div class="editor-wrap">
            <div ref="gutterEl" class="editor-gutter" :style="{ width: gutterWidth }">
              <div
                v-for="(h, i) in editLineHeights"
                :key="i"
                class="code-ln"
                :style="{ height: h + 'px' }"
              >
                {{ i + 1 }}
              </div>
            </div>
            <textarea
              ref="editorEl"
              v-model="previewEdit.text"
              class="preview-editor"
              spellcheck="false"
              :disabled="previewEdit.saving"
              @scroll="syncGutterScroll"
              @keydown.ctrl.s.prevent="savePreviewEdit"
              @keydown.meta.s.prevent="savePreviewEdit"
            />
            <div ref="mirrorEl" class="editor-mirror" aria-hidden="true"></div>
          </div>
          <div v-if="previewEdit.saving" class="preview-status">Saving…</div>
        </template>
        <template v-else-if="preview.data?.isText && preview.data.content !== null">
          <div class="code-view">
            <div v-for="(line, i) in previewLines" :key="i" class="code-line">
              <span class="code-ln" :style="{ width: gutterWidth }">{{ i + 1 }}</span>
              <span class="code-text">{{ line }}</span>
            </div>
          </div>
          <div v-if="preview.data.truncated" class="preview-status">
            — preview truncated —
            <button class="preview-load-full" @click="loadFullPreview">Load full content</button>
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
      <button
        class="ctx-item"
        @click.stop="ctxCopyToOpen = !ctxCopyToOpen"
      >
        📤 Copy to{{ selectedFiles.length > 1 ? ` (${selectedFiles.length} items)` : '' }} ▸
      </button>
      <template v-if="ctxCopyToOpen">
        <button class="ctx-item ctx-sub" @click="ctxDownload">
          💻 Local…
        </button>
        <button
          v-for="t in copyToTargets"
          :key="t.id"
          class="ctx-item ctx-sub"
          :title="`${t.label} — ${t.currentPath}`"
          @click="ctxCopyToTab(t.sessionId!, t.currentPath)"
        >
          {{ KIND_ICONS[t.kind] ?? '⌁' }} {{ t.label }}
          <span class="ctx-sub-path">{{ t.currentPath }}</span>
        </button>
      </template>
      <button class="ctx-item" @click="ctxAddBookmark">
        ⭐ Add Bookmark{{ ctxMenu.entry && !ctxMenu.entry.isDir ? ' (folder)' : '' }}
      </button>
      <button class="ctx-item ctx-danger" @click="ctxDelete">
        🗑 Delete{{ selectedFiles.length > 1 ? ` (${selectedFiles.length} items)` : '' }}…
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

    <!-- Path bar menu -->
    <div
      v-if="pathCtxMenu.visible"
      class="ctx-menu"
      :style="{ left: pathCtxMenu.x + 'px', top: pathCtxMenu.y + 'px' }"
      @click.stop
    >
      <button class="ctx-item" @click="copyPath">📋 Copy Path</button>
      <button class="ctx-item" @click="pastePathAndGo">📥 Paste &amp; Go</button>
      <button class="ctx-item" @click="ctxEditPath">✏️ Edit Path</button>
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
  align-self: stretch;
  cursor: text;
}

.path-input {
  flex: 1;
  background: #11111b;
  border: 1px solid #4f6ec2;
  border-radius: 5px;
  color: #cdd6f4;
  font-size: 13px;
  font-family: monospace;
  padding: 3px 8px;
  outline: none;
  min-width: 0;
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
  overflow-x: auto;
  overflow-y: hidden;
  background: #181825;
}

.columns {
  display: flex;
  flex-shrink: 0;
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
  user-select: none;
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
  user-select: none;
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

.preview-col.maximized {
  width: 100%;
  flex: 1;
  border-left: none;
}

.preview-spacer {
  flex: 1;
}

.preview-actions {
  display: flex;
  gap: 2px;
  flex-shrink: 0;
}

/* Shared code font for preview and editor */
.code-view,
.editor-gutter,
.editor-mirror,
.preview-editor {
  font-family: 'Cascadia Code', Consolas, monospace;
  font-size: 12px;
  line-height: 1.5;
}

.editor-wrap {
  flex: 1;
  display: flex;
  overflow: hidden;
  position: relative;
  background: #11111b;
}

.editor-gutter {
  flex-shrink: 0;
  overflow: hidden;
  padding: 12px 0 12px 0;
  background: #11111b;
  border-right: 1px solid #2a2a3d;
}

.code-ln {
  flex-shrink: 0;
  box-sizing: border-box;
  padding: 0 10px 0 8px;
  text-align: right;
  color: #6c7086;
  user-select: none;
}

.preview-editor {
  flex: 1;
  margin: 0;
  padding: 12px 14px 12px 10px;
  color: #cdd6f4;
  background: #11111b;
  border: none;
  outline: none;
  resize: none;
  white-space: pre-wrap;
  overflow-wrap: break-word;
  overflow-y: auto;
  overflow-x: hidden;
  min-width: 0;
}

/* Hidden mirror used to measure wrapped line heights */
.editor-mirror {
  position: absolute;
  top: 0;
  left: -99999px;
  visibility: hidden;
  box-sizing: border-box;
  padding: 0 14px 0 10px;
  white-space: pre-wrap;
  overflow-wrap: break-word;
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

.code-view {
  flex: 1;
  margin: 0;
  padding: 12px 0;
  color: #cdd6f4;
  overflow-y: auto;
}

.code-line {
  display: flex;
  min-height: 1.5em;
}

.code-text {
  flex: 1;
  min-width: 0;
  padding-right: 14px;
  white-space: pre-wrap;
  overflow-wrap: break-word;
}

.preview-status {
  padding: 14px;
  color: #6c7086;
  font-size: 12px;
  text-align: center;
}

.preview-load-full {
  margin-left: 8px;
  padding: 3px 10px;
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
}

.preview-load-full:hover {
  background: #45475a;
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

.ctx-item.ctx-danger {
  color: #f38ba8;
}

.ctx-item:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.ctx-item.ctx-sub {
  padding-left: 24px;
  display: flex;
  align-items: center;
  gap: 6px;
  overflow: hidden;
}

.ctx-sub-path {
  color: #6c7086;
  font-size: 11px;
  font-family: monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
