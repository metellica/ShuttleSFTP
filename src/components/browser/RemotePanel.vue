<script setup lang="ts">
import { ref, watch, computed, onMounted, onUnmounted, nextTick } from 'vue'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open, save, ask, message } from '@tauri-apps/plugin-dialog'
import { useTabsStore } from '@/stores/tabs'
import { useTransferStore } from '@/stores/transfer'
import { useClipboardStore } from '@/stores/clipboard'
import { usePrepareStore } from '@/stores/prepare'
import { useViewSettingsStore, COLUMN_KEYS, type ColumnKey } from '@/stores/viewSettings'
import DensityControl from '@/components/layout/DensityControl.vue'
import { listDir, mkDir, uploadFiles, downloadFiles, downloadFileAs, previewFile, saveFileContent, saveBookmark, removeEntry, renameEntry, transferRemote } from '@/composables/useTauri'
import { promptText } from '@/composables/usePrompt'
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

const props = defineProps<{ tabId?: string }>()

const tabsStore = useTabsStore()
const transferStore = useTransferStore()
const clipboard = useClipboardStore()
const prepareStore = usePrepareStore()
const viewSettings = useViewSettingsStore()

/** The tab this panel is rendering — independent of which pane is focused. */
const tab = computed(() => tabsStore.tabs.find((t) => t.id === props.tabId) ?? null)

const columns = ref<Column[]>([])
const dragOver = ref(false)
const selectedPaths = ref<Set<string>>(new Set())
/** Anchor for shift+click range selection (colIndex is null in list view). */
const selectionAnchor = ref<{ colIndex: number | null; path: string } | null>(null)
const bodyEl = ref<HTMLElement | null>(null)
const listViewEl = ref<HTMLElement | null>(null)

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

// --- Sorting -------------------------------------------------------------
type SortKey = 'name' | 'size' | 'permissions' | 'modified'
const SORT_KEYS: SortKey[] = ['name', 'size', 'permissions', 'modified']
const storedSortKey = localStorage.getItem('sortKey') as SortKey | null
const sortKey = ref<SortKey>(
  storedSortKey && SORT_KEYS.includes(storedSortKey) ? storedSortKey : 'name'
)
const sortAsc = ref(localStorage.getItem('sortAsc') !== 'false')

/** Click a column header: toggle direction when already sorted by it. */
function setSort(key: SortKey) {
  if (sortKey.value === key) {
    sortAsc.value = !sortAsc.value
  } else {
    sortKey.value = key
    sortAsc.value = true
  }
  localStorage.setItem('sortKey', sortKey.value)
  localStorage.setItem('sortAsc', String(sortAsc.value))
  resortLoaded()
}

function sortIndicator(key: SortKey): string {
  if (sortKey.value !== key) return ''
  return sortAsc.value ? '▲' : '▼'
}

// --- Column widths -------------------------------------------------------
/** Horizontal padding of a list row, which counts towards its total width. */
const ROW_PADDING = 14

/**
 * Widths are stored unscaled and multiplied by the row scale here, so
 * zooming the rows keeps the columns in proportion.
 */
const scaledWidths = computed(
  () =>
    Object.fromEntries(
      COLUMN_KEYS.map((key) => [key, viewSettings.columnWidths[key] * viewSettings.rowScale])
    ) as Record<ColumnKey, number>
)

/**
 * The name column fills the pane until it is dragged; after that it keeps
 * its width and the trailing filler track takes the slack instead.
 */
const gridTemplate = computed(() => {
  const w = scaledWidths.value
  const name = viewSettings.stretchName ? `minmax(${w.name}px, 1fr)` : `${w.name}px`
  const filler = viewSettings.stretchName ? '0px' : '1fr'
  return `${name} ${w.size}px ${w.permissions}px ${w.modified}px ${filler}`
})

/** Once the columns outgrow the pane the list scrolls sideways. */
const rowWidth = computed(() => {
  const w = scaledWidths.value
  const tracks = COLUMN_KEYS.reduce((sum, key) => sum + w[key], 0)
  return tracks + 2 * ROW_PADDING * viewSettings.rowScale
})

const resizing = ref<ColumnKey | null>(null)
const headNameEl = ref<HTMLElement | null>(null)

/** Header and rows share the track sizes, so they always line up. */
const rowStyle = computed(() => ({
  gridTemplateColumns: gridTemplate.value,
  width: `max(100%, ${rowWidth.value}px)`,
}))

/** Drag a header divider; the width it writes is what the next start restores. */
function startColumnResize(key: ColumnKey, event: MouseEvent) {
  const startX = event.clientX
  // Undo the row scale so a pixel of pointer travel is a pixel on screen.
  const scale = viewSettings.rowScale || 1
  // A stretched name column is wider than its stored width, so the drag has
  // to start from what is actually on screen or the first pixels do nothing.
  const startWidth =
    key === 'name' && headNameEl.value
      ? headNameEl.value.getBoundingClientRect().width / scale
      : viewSettings.columnWidths[key]
  resizing.value = key

  const move = (e: MouseEvent) =>
    viewSettings.setColumnWidth(key, startWidth + (e.clientX - startX) / scale)
  const stop = () => {
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', stop)
    document.body.classList.remove('col-resizing')
    resizing.value = null
  }
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', stop)
  document.body.classList.add('col-resizing')
}

onUnmounted(() => document.body.classList.remove('col-resizing'))

/** Re-sort already loaded entries in place, without hitting the server again. */
function resortLoaded() {
  listEntries.value = sortEntries([...listEntries.value])
  columns.value = columns.value.map((c) => ({ ...c, entries: sortEntries([...c.entries]) }))
}

// --- Fuzzy filter (current folder only) ----------------------------------
const filterActive = ref(false)
const filterQuery = ref('')
const filterInputEl = ref<HTMLInputElement | null>(null)
const filterText = computed(() => filterQuery.value.trim())

/** Case-insensitive substring match, falling back to a subsequence match. */
function fuzzyMatch(name: string, query: string): boolean {
  if (!query) return true
  const n = name.toLowerCase()
  const q = query.toLowerCase()
  if (n.includes(q)) return true
  let i = 0
  for (const ch of q) {
    if (ch === ' ') continue
    i = n.indexOf(ch, i)
    if (i === -1) return false
    i++
  }
  return true
}

function applyFilter(entries: FileEntry[]): FileEntry[] {
  const q = filterText.value
  return q ? entries.filter((e) => fuzzyMatch(e.name, q)) : entries
}

const visibleListEntries = computed(() => applyFilter(listEntries.value))

/** The filter only narrows the folder the user is currently in. */
function visibleEntries(col: Column): FileEntry[] {
  return col.path === currentPath.value ? applyFilter(col.entries) : col.entries
}

function openFilter() {
  filterActive.value = true
  nextTick(() => {
    filterInputEl.value?.focus()
    filterInputEl.value?.select()
  })
}

function closeFilter() {
  filterActive.value = false
  filterQuery.value = ''
}

function toggleFilter() {
  if (filterActive.value) closeFilter()
  else openFilter()
}
const ctxMenu = ref<{
  visible: boolean
  x: number
  y: number
  entry: FileEntry | null
  /** Directory a blank-area menu targets (Paste / New Folder). */
  dir: string | null
}>({
  visible: false,
  x: 0,
  y: 0,
  entry: null,
  dir: null,
})
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

const currentPath = computed(() => tab.value?.currentPath || '/')
const sessionId = computed(() => tab.value?.sessionId || '')

/** Parent directory path, or null when at root. */
const parentPath = computed(() => {
  const p = currentPath.value
  if (p === '/') return null
  const idx = p.lastIndexOf('/')
  return idx === 0 ? '/' : p.slice(0, idx)
})

/** True when `parent` is the immediate parent directory of `child`. */
function isDirectChildOf(parent: string, child: string): boolean {
  const np = parent.replace(/\/+$/, '') || '/'
  const nc = child.replace(/\/+$/, '') || '/'
  const sep = nc.lastIndexOf('/')
  if (sep === 0) return np === '/'
  return nc.slice(0, sep) === np
}

/** Synthetic ".." entry for the list view, so the user can double-click up. */
const parentEntry = computed<FileEntry | null>(() => {
  if (parentPath.value === null) return null
  return {
    name: '..',
    path: parentPath.value,
    isDir: true,
    size: 0,
    modified: 0,
    permissions: null,
  }
})

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
  viewMode.value === 'columns'
    ? columns.value.flatMap((c) => visibleEntries(c))
    : visibleListEntries.value
)
const selectedFiles = computed(() =>
  allEntries.value.filter((f) => selectedPaths.value.has(f.path))
)
defineExpose({ selectedFiles, refresh })

function compareByName(a: FileEntry, b: FileEntry): number {
  return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })
}

function compareEntries(a: FileEntry, b: FileEntry): number {
  // Folders always stay on top, regardless of the sort direction
  if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
  let r = 0
  switch (sortKey.value) {
    case 'size':
      r = a.size - b.size
      break
    case 'modified':
      r = a.modified - b.modified
      break
    case 'permissions':
      r = (a.permissions || '').localeCompare(b.permissions || '')
      break
    default:
      r = compareByName(a, b)
  }
  if (r === 0) r = compareByName(a, b)
  return sortAsc.value ? r : -r
}

function sortEntries(entries: FileEntry[]): FileEntry[] {
  return entries.sort(compareEntries)
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
      if (tab.value && tab.value.currentPath !== parent.path) {
        suppressWatch = true
        tabsStore.updateTab(tab.value.id, { currentPath: parent.path })
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
      if (tab.value && tab.value.currentPath !== parentDir) {
        suppressWatch = true
        tabsStore.updateTab(tab.value.id, { currentPath: parentDir })
      }
      loadPreview(fileEntry)
    } catch {
      console.error('Failed to list directory:', e)
    }
  } finally {
    listLoading.value = false
  }
}

/**
 * Auto-select the ".." row or the child folder we navigated up from.
 * Only applies to list view; in column view the parent is always visible.
 *
 * Called from a watch on listEntries — the same pattern ShuttleFiles uses
 * (watch props.entries → auto-select) — so the DOM is already showing the
 * new rows by the time `nextTick` fires.
 */
function autoSelectInList() {
  if (viewMode.value !== 'list' || listLoading.value) return
  selectedPaths.value.clear()
  selectionAnchor.value = null

  if (focusPath.value && listEntries.value.some((e) => e.path === focusPath.value)) {
    selectedPaths.value = new Set([focusPath.value])
    selectionAnchor.value = { colIndex: null, path: focusPath.value }
  } else if (parentPath.value) {
    selectedPaths.value = new Set([parentPath.value])
    selectionAnchor.value = { colIndex: null, path: parentPath.value }
  }

  nextTick(() => {
    const container = listViewEl.value
    const row = container?.querySelector('.file-row.selected') as HTMLElement | null
    if (!container || !row) return
    const cRect = container.getBoundingClientRect()
    const rRect = row.getBoundingClientRect()
    const offset = rRect.top - cRect.top + container.scrollTop
    const center = offset - cRect.height / 2 + rRect.height / 2
    container.scrollTo({ top: Math.max(0, center), behavior: 'auto' })
  })
}

// Mirror ShuttleFiles: react to the data that drives the template, not a
// manual call inside loadList.  listLoading guards against the first
// (empty) fire from the currentPath → clear → loadList cascade.
watch(listEntries, () => autoSelectInList())

async function refresh() {
  if (viewMode.value === 'columns') {
    await buildColumns(currentPath.value)
  } else {
    await loadList(currentPath.value)
  }
}

function navigateTo(path: string) {
  if (tab.value) {
    tabsStore.updateTab(tab.value.id, { currentPath: path })
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
    selectRange(visibleEntries(col), entry, anchorPath, event.ctrlKey || event.metaKey)
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
    if (tab.value) {
      // Update path without triggering a full rebuild
      suppressWatch = true
      tabsStore.updateTab(tab.value.id, { currentPath: entry.path })
    }
    scrollToEnd()
  } else {
    // Selecting a file: current dir is the column's dir
    columns.value = columns.value.slice(0, colIndex + 1)
    if (tab.value && tab.value.currentPath !== col.path) {
      suppressWatch = true
      tabsStore.updateTab(tab.value.id, { currentPath: col.path })
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

async function cancelPreviewEdit() {
  if (previewDirty.value && !(await ask('Discard unsaved changes?', { title: 'Edit File', kind: 'warning' }))) return
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
    await message('Failed to save file: ' + e, { title: 'Edit File', kind: 'error' })
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
    selectRange(visibleListEntries.value, entry, anchorPath, event.ctrlKey || event.metaKey)
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
  ctxMenu.value = { visible: true, x: event.clientX, y: event.clientY, entry, dir: null }
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
  ctxMenu.value = { visible: true, x: event.clientX, y: event.clientY, entry, dir: null }
}

/** Right-click on the blank area of a column / the list: dir-level menu. */
function onBlankContextMenu(dir: string, event: MouseEvent) {
  previewCtxMenu.value.visible = false
  ctxMenu.value = { visible: true, x: event.clientX, y: event.clientY, entry: null, dir }
}

/** Left-click on blank space clears the selection. */
function onBlankClick() {
  selectedPaths.value.clear()
  selectionAnchor.value = null
}

function hideCtxMenu() {
  ctxMenu.value.visible = false
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
    await prepareStore.run('Preparing download', (pid) =>
      downloadFiles(sid, targets, dir as string, pid)
    )
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Download failed:', e)
  }
}

/** Icons for virtual container/pod dirs, falling back to file/folder. */
function entryIcon(e: FileEntry): string {
  if (e.name === '@containers') return '▣'
  if (e.name === '@pods') return '⎈'
  if (e.path.startsWith('/@containers/') && e.path.split('/').filter(Boolean).length === 2)
    return '▣'
  if (e.path.startsWith('/@pods/') && e.path.split('/').filter(Boolean).length <= 4 && e.isDir)
    return '⎈'
  return e.isDir ? '📁' : '📄'
}

/** Mark the selection for a later Paste (in any session). */
function copyFilesToClipboard() {
  const sid = sessionId.value
  const targets = selectedFiles.value.map((f) => f.path)
  if (!sid || targets.length === 0) return
  clipboard.set(sid, targets, tab.value?.label ?? '')
}

function ctxCopyFiles() {
  hideCtxMenu()
  copyFilesToClipboard()
}

async function pasteClipboardInto(destDir: string) {
  const sid = sessionId.value
  if (!sid || !clipboard.sessionId || clipboard.paths.length === 0) return
  try {
    await prepareStore.run('Preparing copy', (pid) =>
      transferRemote(clipboard.sessionId!, [...clipboard.paths], sid, destDir, pid)
    )
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Paste failed:', e)
    await message(`Paste failed: ${e}`, { title: 'Paste', kind: 'error' })
  }
}

/** Paste always targets the currently browsed directory, regardless of
 *  what's selected or right-clicked — matching standard file manager UX. */
async function ctxPasteFiles() {
  hideCtxMenu()
  await pasteClipboardInto(currentPath.value)
}

/** Name of the folder Paste would target, for the menu label. */
const pasteTargetName = computed(() =>
  currentPath.value.split('/').filter(Boolean).pop() ?? '/'
)

/** New folder inside the blank-clicked directory. */
async function ctxNewFolder() {
  const dir = ctxMenu.value.dir ?? currentPath.value
  hideCtxMenu()
  const sid = sessionId.value
  if (!sid) return
  const name = await promptText('New folder name:')
  if (!name?.trim()) return
  try {
    await mkDir(sid, dir === '/' ? `/${name.trim()}` : `${dir}/${name.trim()}`)
    await refresh()
  } catch (e) {
    console.error('Create folder failed:', e)
    await message(`Create folder failed: ${e}`, { title: 'New Folder', kind: 'error' })
  }
}

function ctxRefresh() {
  hideCtxMenu()
  refresh()
}

/** Rename a single entry in place (works for both local and remote sessions). */
async function renameFile(entry: FileEntry) {
  const sid = sessionId.value
  if (!sid) return

  const name = await promptText('Rename to:', { defaultValue: entry.name })
  const trimmed = name?.trim()
  if (!trimmed || trimmed === entry.name) return

  const parentPath = entry.path.slice(0, entry.path.length - entry.name.length).replace(/\/+$/, '')
  const newPath = parentPath ? `${parentPath}/${trimmed}` : `/${trimmed}`

  try {
    await renameEntry(sid, entry.path, newPath)
    if (preview.value.entry?.path === entry.path) {
      preview.value = { entry: null, loading: false, data: null }
      previewEdit.value = { active: false, text: '', saving: false }
    }
    selectedPaths.value.clear()
    selectedPaths.value.add(newPath)
    selectionAnchor.value = null
  } catch (e) {
    console.error('Rename failed:', e)
    await message(`Rename failed: ${e}`, { title: 'Rename', kind: 'error' })
  }
  await refresh()
}

function ctxRename() {
  const entry = ctxMenu.value.entry
  hideCtxMenu()
  if (entry) renameFile(entry)
}

/** F2 renames the single selected entry, matching desktop file manager conventions. */
function onRenameKeydown(e: KeyboardEvent) {
  if (e.key !== 'F2') return
  const t = e.target as HTMLElement | null
  if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
  const sel = selectedFiles.value
  const only = sel.length === 1 ? sel[0] : undefined
  if (!only) return
  e.preventDefault()
  renameFile(only)
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

// --- Drop dragged files onto a folder entry (copy into it) -----------------

const dropTargetPath = ref<string | null>(null)

function onEntryDragOver(entry: FileEntry, event: DragEvent) {
  if (!entry.isDir) return
  if (!event.dataTransfer?.types.includes('application/x-shuttle-files')) return
  event.preventDefault()
  event.dataTransfer.dropEffect = 'copy'
  dropTargetPath.value = entry.path
}

function onEntryDragLeave(entry: FileEntry) {
  if (dropTargetPath.value === entry.path) dropTargetPath.value = null
}

async function onEntryDrop(entry: FileEntry, event: DragEvent) {
  dropTargetPath.value = null
  if (!entry.isDir) return
  const raw = event.dataTransfer?.getData('application/x-shuttle-files')
  if (!raw || !sessionId.value) return
  event.preventDefault()
  event.stopPropagation()
  try {
    const payload = JSON.parse(raw) as { sessionId: string; paths: string[] }
    if (!payload.sessionId || payload.paths.length === 0) return
    // Ignore dropping something onto itself
    if (payload.sessionId === sessionId.value && payload.paths.includes(entry.path)) return
    await prepareStore.run('Preparing copy', (pid) =>
      transferRemote(payload.sessionId, payload.paths, sessionId.value, entry.path, pid)
    )
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Drop copy failed:', e)
    await message(`Copy failed: ${e}`, { title: 'Copy', kind: 'error' })
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
    await prepareStore.run('Preparing download', (pid) =>
      downloadFileAs(sid, entry.path, target, pid)
    )
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
    await prepareStore.run('Deleting', async (pid) => {
      for (const f of targets) {
        await removeEntry(sid, f.path, f.isDir, pid)
        if (preview.value.entry?.path === f.path) {
          preview.value = { entry: null, loading: false, data: null }
          previewEdit.value = { active: false, text: '', saving: false }
        }
      }
    })
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
  const activeTab = tab.value
  if (!activeTab) return
  const params = activeTab.connectParams
  if (activeTab.kind === 'ssh' && !params) return

  // Bookmark the folder itself, or the containing dir for files
  const path = entry?.isDir ? entry.path : currentPath.value
  const alias = await promptText('Bookmark alias:', { defaultValue: path })
  if (alias === null) return

  // Connection alias from the tab label ("user@alias"), for display
  const prefix = `${params?.username ?? ''}@`
  let hostAlias = activeTab.label.startsWith(prefix)
    ? activeTab.label.slice(prefix.length)
    : undefined
  // Label was the bare host/IP, not a real alias: keep the ip:port fallback
  if (hostAlias === params?.host) hostAlias = undefined

  const bookmark: Bookmark = {
    id: crypto.randomUUID(),
    alias: alias.trim() || path,
    host: params?.host ?? 'local',
    port: params?.port ?? 0,
    username: params?.username ?? '',
    ...(hostAlias ? { hostAlias } : {}),
    authMethod: params ? params.auth.type : 'agent',
    path,
    kind: activeTab.kind,
  }
  if (params?.auth.type === 'key') {
    bookmark.privateKeyPath = params.auth.key_path
    if (params.auth.passphrase) bookmark.passphrase = params.auth.passphrase
  } else if (params?.auth.type === 'password') {
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

/** Previous path, used to detect "go up" navigation for auto-focus. */
const prevPath = ref<string | null>(null)
/** Path to auto-select after loading — the child folder when going up. */
const focusPath = ref<string | null>(null)

/** Alt+←/→ and mouse back/forward buttons navigate history. */
function onNavKeydown(e: KeyboardEvent) {
  if (!e.altKey) return
  if (e.key === 'ArrowLeft') {
    e.preventDefault()
    tabsStore.goBack()
  } else if (e.key === 'ArrowRight') {
    e.preventDefault()
    tabsStore.goForward()
  }
}

function onNavMouseUp(e: MouseEvent) {
  if (e.button === 3) {
    e.preventDefault()
    tabsStore.goBack()
  } else if (e.button === 4) {
    e.preventDefault()
    tabsStore.goForward()
  }
}

/** Ctrl/Cmd+C copies the file selection, Ctrl/Cmd+V pastes into the current dir. */
function onClipboardKeydown(e: KeyboardEvent) {
  if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return
  const key = e.key.toLowerCase()
  if (key !== 'c' && key !== 'v') return
  // Leave native copy/paste alone inside inputs, textareas and editables
  const t = e.target as HTMLElement | null
  if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
  // Ctrl+C with a text selection (e.g. in the preview pane) stays native
  if (key === 'c' && (window.getSelection()?.toString() ?? '') !== '') return
  if (key === 'c') {
    if (selectedFiles.value.length === 0) return
    e.preventDefault()
    copyFilesToClipboard()
  } else {
    if (!clipboard.sessionId || clipboard.paths.length === 0) return
    e.preventDefault()
    pasteClipboardInto(currentPath.value)
  }
}

/** Ctrl/Cmd+F opens the folder filter, Esc closes it. */
function onFilterKeydown(e: KeyboardEvent) {
  const t = e.target as HTMLElement | null
  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'f') {
    if (t && (t.tagName === 'TEXTAREA' || t.isContentEditable)) return
    e.preventDefault()
    openFilter()
  } else if (e.key === 'Escape' && filterActive.value) {
    if (t && (t.tagName === 'TEXTAREA' || t.isContentEditable)) return
    closeFilter()
  }
}

/** Ctrl+wheel zooms the file rows instead of scrolling, as in browsers. */
function onZoomWheel(e: WheelEvent) {
  if (!e.ctrlKey && !e.metaKey) return
  // The preview pane keeps native scrolling/zooming.
  if ((e.target as HTMLElement | null)?.closest('.preview-col')) return
  e.preventDefault()
  viewSettings.nudge(e.deltaY < 0 ? 1 : -1)
}

/** Row zoom, using the keys browsers already train users on. */
function onZoomKeydown(e: KeyboardEvent) {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return
  // The terminal and the in-place editor own their own key handling.
  const t = e.target as HTMLElement | null
  if (t && (t.tagName === 'TEXTAREA' || t.isContentEditable)) return
  if (e.key === '=' || e.key === '+') {
    e.preventDefault()
    viewSettings.nudge(1)
  } else if (e.key === '-') {
    e.preventDefault()
    viewSettings.nudge(-1)
  } else if (e.key === '0') {
    e.preventDefault()
    viewSettings.reset()
  }
}

onMounted(async () => {
  window.addEventListener('click', hideCtxMenu)
  window.addEventListener('resize', scheduleRecompute)
  window.addEventListener('keydown', onNavKeydown)
  window.addEventListener('keydown', onClipboardKeydown)
  window.addEventListener('keydown', onFilterKeydown)
  window.addEventListener('keydown', onZoomKeydown)
  window.addEventListener('keydown', onRenameKeydown)
  window.addEventListener('mouseup', onNavMouseUp)
  // Registered by hand: a passive listener could not call preventDefault,
  // and the browser would zoom the whole page instead.
  bodyEl.value?.addEventListener('wheel', onZoomWheel, { passive: false })
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
          await prepareStore.run('Preparing upload', (pid) =>
            uploadFiles(sessionId.value, paths, currentPath.value, pid)
          )
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
  window.removeEventListener('keydown', onNavKeydown)
  window.removeEventListener('keydown', onClipboardKeydown)
  window.removeEventListener('keydown', onFilterKeydown)
  window.removeEventListener('keydown', onZoomKeydown)
  window.removeEventListener('keydown', onRenameKeydown)
  window.removeEventListener('mouseup', onNavMouseUp)
  bodyEl.value?.removeEventListener('wheel', onZoomWheel)
  unlistenDragDrop?.()
})

watch(
  currentPath,
  (newPath, oldPath) => {
    if (suppressWatch) {
      suppressWatch = false
      return
    }
    // Detect "go up" navigation: auto-select the child folder we left.
    if (oldPath && isDirectChildOf(newPath, oldPath)) {
      focusPath.value = oldPath
    } else {
      focusPath.value = null
    }
    prevPath.value = newPath

    selectedPaths.value.clear()
    selectionAnchor.value = null
    preview.value = { entry: null, loading: false, data: null }
    filterQuery.value = ''
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
      <div class="nav-btns">
        <button
          class="toggle-btn nav-btn"
          title="Back (Alt+←)"
          :disabled="!tabsStore.canGoBack"
          @click="tabsStore.goBack()"
        >
          ←
        </button>
        <button
          class="toggle-btn nav-btn"
          title="Forward (Alt+→)"
          :disabled="!tabsStore.canGoForward"
          @click="tabsStore.goForward()"
        >
          →
        </button>
        <button
          class="toggle-btn nav-btn"
          title="Up one level"
          :disabled="currentPath === '/'"
          @click="navigateTo(currentPath.slice(0, currentPath.lastIndexOf('/')) || '/')"
        >
          ↑
        </button>
      </div>
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
      <div class="filter-box">
        <input
          v-if="filterActive"
          ref="filterInputEl"
          v-model="filterQuery"
          class="filter-input"
          placeholder="Filter files…"
          spellcheck="false"
          @keydown.esc.stop.prevent="closeFilter"
        />
        <button
          class="toggle-btn"
          :class="{ active: filterActive }"
          title="Filter files in this folder (Ctrl+F)"
          @click="toggleFilter"
        >
          🔍
        </button>
      </div>
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
      <DensityControl />
    </div>

    <div class="body" ref="bodyEl" :style="{ '--row-scale': viewSettings.rowScale }">
      <!-- Finder-style Miller columns -->
      <div
        v-if="viewMode === 'columns'"
        v-show="!previewMaximized"
        class="columns"
        @click.self="onBlankClick"
        @contextmenu.self.prevent="onBlankContextMenu(currentPath, $event)"
      >
        <div
          v-for="(col, colIndex) in columns"
          :key="col.path"
          class="column"
          @click.self="onBlankClick"
          @contextmenu.self.prevent="onBlankContextMenu(col.path, $event)"
        >
          <div v-if="col.loading" class="col-loading">Loading...</div>
          <template v-else>
            <div
              v-for="entry in visibleEntries(col)"
              :key="entry.path"
              class="entry"
              :class="{
                selected: selectedPaths.has(entry.path),
                opened: col.selectedPath === entry.path,
                'drop-target': dropTargetPath === entry.path,
              }"
              draggable="true"
              @dragstart="onEntryDragStart(entry, $event)"
              @dragover="onEntryDragOver(entry, $event)"
              @dragleave="onEntryDragLeave(entry)"
              @drop="onEntryDrop(entry, $event)"
              @click="onEntryClick(colIndex, entry, $event)"
              @contextmenu.prevent="onEntryContextMenu(colIndex, entry, $event)"
            >
              <span class="entry-icon">{{ entryIcon(entry) }}</span>
              <span class="entry-name" :title="entry.name">{{ entry.name }}</span>
              <span v-if="!entry.isDir" class="entry-size">{{ formatSize(entry.size) }}</span>
              <span v-else class="entry-arrow">›</span>
            </div>
            <div v-if="visibleEntries(col).length === 0" class="col-empty">
              {{ col.entries.length > 0 ? 'No matches' : 'Empty' }}
            </div>
          </template>
        </div>
      </div>

      <!-- Windows Explorer-style details list -->
      <div
        v-else
        v-show="!previewMaximized"
        class="list-view"
        ref="listViewEl"
        @click.self="onBlankClick"
        @contextmenu.self.prevent="onBlankContextMenu(currentPath, $event)"
      >
        <div v-if="listLoading" class="col-loading">Loading...</div>
        <template v-else>
          <div class="file-header" :style="rowStyle">
            <div ref="headNameEl" class="head-cell col-name">
              <button
                class="sort-btn"
                :class="{ active: sortKey === 'name' }"
                @click="setSort('name')"
              >
                Name<span class="sort-arrow">{{ sortIndicator('name') }}</span>
              </button>
              <span
                class="grip"
                :class="{ active: resizing === 'name' }"
                title="Drag to resize, double-click to reset"
                @mousedown.prevent.stop="startColumnResize('name', $event)"
                @dblclick.stop="viewSettings.resetColumnWidth('name')"
              />
            </div>
            <div class="head-cell col-size">
              <button
                class="sort-btn"
                :class="{ active: sortKey === 'size' }"
                @click="setSort('size')"
              >
                Size<span class="sort-arrow">{{ sortIndicator('size') }}</span>
              </button>
              <span
                class="grip"
                :class="{ active: resizing === 'size' }"
                title="Drag to resize, double-click to reset"
                @mousedown.prevent.stop="startColumnResize('size', $event)"
                @dblclick.stop="viewSettings.resetColumnWidth('size')"
              />
            </div>
            <div class="head-cell col-perm">
              <button
                class="sort-btn"
                :class="{ active: sortKey === 'permissions' }"
                @click="setSort('permissions')"
              >
                Permissions<span class="sort-arrow">{{ sortIndicator('permissions') }}</span>
              </button>
              <span
                class="grip"
                :class="{ active: resizing === 'permissions' }"
                title="Drag to resize, double-click to reset"
                @mousedown.prevent.stop="startColumnResize('permissions', $event)"
                @dblclick.stop="viewSettings.resetColumnWidth('permissions')"
              />
            </div>
            <div class="head-cell col-date">
              <button
                class="sort-btn"
                :class="{ active: sortKey === 'modified' }"
                @click="setSort('modified')"
              >
                Modified<span class="sort-arrow">{{ sortIndicator('modified') }}</span>
              </button>
              <span
                class="grip"
                :class="{ active: resizing === 'modified' }"
                title="Drag to resize, double-click to reset"
                @mousedown.prevent.stop="startColumnResize('modified', $event)"
                @dblclick.stop="viewSettings.resetColumnWidth('modified')"
              />
            </div>
          </div>
          <!-- ".." parent row for quick navigation back up -->
          <div
            v-if="parentEntry && !filterActive"
            class="file-row parent-row"
            :style="rowStyle"
            :class="{ selected: selectedPaths.has(parentEntry.path) }"
            @dblclick="navigateTo(parentEntry.path)"
          >
            <span class="col-name">
              <span class="entry-icon">📁</span>
              ..
            </span>
            <span class="col-size" />
            <span class="col-perm" />
            <span class="col-date" />
          </div>
          <div
            v-for="entry in visibleListEntries"
            :key="entry.path"
            class="file-row"
            :style="rowStyle"
            :class="{
              selected: selectedPaths.has(entry.path),
              'drop-target': dropTargetPath === entry.path,
            }"
            draggable="true"
            @dragstart="onEntryDragStart(entry, $event)"
            @dragover="onEntryDragOver(entry, $event)"
            @dragleave="onEntryDragLeave(entry)"
            @drop="onEntryDrop(entry, $event)"
            @click="onListClick(entry, $event)"
            @dblclick="onListDblClick(entry)"
            @contextmenu.prevent="onListContextMenu(entry, $event)"
          >
            <span class="col-name">
              <span class="entry-icon">{{ entryIcon(entry) }}</span>
              {{ entry.name }}
            </span>
            <span class="col-size">{{ entry.isDir ? '-' : formatSize(entry.size) }}</span>
            <span class="col-perm">{{ entry.permissions || '-' }}</span>
            <span class="col-date">{{ formatDate(entry.modified) }}</span>
          </div>
          <div v-if="visibleListEntries.length === 0" class="col-empty">
            {{ listEntries.length > 0 ? 'No files match the filter' : 'Empty directory' }}
          </div>
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
      <template v-if="ctxMenu.entry">
        <button class="ctx-item" @click="ctxDownload">⬇ Download…</button>
        <button class="ctx-item" @click="ctxSaveAs">💾 Save As…</button>
        <button class="ctx-item" @click="ctxCopyFiles">
          📋 Copy{{ selectedFiles.length > 1 ? ` (${selectedFiles.length} items)` : '' }}
        </button>
      </template>
      <button
        class="ctx-item"
        :disabled="!clipboard.sessionId || clipboard.paths.length === 0"
        :title="clipboard.sourceLabel ? `From ${clipboard.sourceLabel}` : ''"
        @click="ctxPasteFiles"
      >
        📥 Paste{{ clipboard.paths.length ? ` (${clipboard.paths.length})` : '' }} into “{{ pasteTargetName }}”
      </button>
      <template v-if="!ctxMenu.entry">
        <button class="ctx-item" @click="ctxNewFolder">📁 New Folder…</button>
        <button class="ctx-item" @click="ctxRefresh">🔄 Refresh</button>
      </template>
      <template v-if="ctxMenu.entry">
      <button class="ctx-item" :disabled="selectedFiles.length > 1" @click="ctxRename">
        ✏️ Rename…
      </button>
      <button class="ctx-item" @click="ctxAddBookmark">
        ⭐ Add Bookmark{{ ctxMenu.entry && !ctxMenu.entry.isDir ? ' (folder)' : '' }}
      </button>
      <button class="ctx-item ctx-danger" @click="ctxDelete">
        🗑 Delete{{ selectedFiles.length > 1 ? ` (${selectedFiles.length} items)` : '' }}…
      </button>
      </template>
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
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border);
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
  color: var(--accent);
  cursor: pointer;
  padding: 2px 6px;
  font-size: 13px;
  font-family: monospace;
}

.crumb:hover {
  background: var(--surface);
}

.crumb.current {
  color: var(--text-primary);
  font-weight: 600;
}

.crumb-sep {
  color: var(--text-muted);
  font-size: 12px;
}

.path-spacer {
  flex: 1;
  align-self: stretch;
  cursor: text;
}

.path-input {
  flex: 1;
  background: var(--bg-secondary);
  border: 1px solid var(--scrollbar-thumb);
  border-radius: 5px;
  color: var(--text-primary);
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

.filter-box {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
  margin-left: 4px;
}

.filter-input {
  width: 150px;
  background: var(--bg-secondary);
  border: 1px solid var(--scrollbar-thumb);
  border-radius: 5px;
  color: var(--text-primary);
  font-size: 12px;
  padding: 3px 8px;
  outline: none;
}

.filter-input::placeholder {
  color: var(--text-muted);
}

.toggle-btn {
  background: none;
  border: none;
  border-radius: 4px;
  color: var(--text-muted);
  cursor: pointer;
  padding: 2px 8px;
  font-size: 14px;
  line-height: 1.4;
}

.toggle-btn:hover {
  background: var(--surface);
  color: var(--text-primary);
}

.toggle-btn.active {
  background: var(--scrollbar-thumb);
  color: var(--accent-text);
}

.nav-btns {
  display: flex;
  gap: 2px;
  margin-right: 6px;
  flex-shrink: 0;
}

.nav-btn {
  font-weight: 700;
}

.nav-btn:disabled {
  opacity: 0.35;
  cursor: default;
}

.nav-btn:disabled:hover {
  background: none;
  color: var(--text-muted);
}

/*
 * File-row metrics below derive from --row-scale so the listing can be
 * tuned for eyesight or display DPI. Column widths scale too, otherwise
 * the larger text clips at the bigger settings.
 */
.body {
  --row-scale: 1;
  flex: 1;
  display: flex;
  overflow-x: auto;
  overflow-y: hidden;
  background: var(--bg-secondary);
}

.columns {
  display: flex;
  flex-shrink: 0;
  background: var(--bg-secondary);
}

.column {
  min-width: calc(230px * var(--row-scale));
  max-width: calc(280px * var(--row-scale));
  height: 100%;
  overflow-y: auto;
  overflow-x: hidden;
  border-right: 1px solid var(--border);
  flex-shrink: 0;
  padding: 6px 0;
  background: var(--bg-primary);
}

.entry {
  display: flex;
  align-items: center;
  gap: calc(7px * var(--row-scale));
  padding: calc(5px * var(--row-scale)) calc(12px * var(--row-scale));
  font-size: calc(13px * var(--row-scale));
  cursor: pointer;
  border-radius: 5px;
  margin: 1px 6px;
  color: var(--text-primary);
  transition: background 0.08s;
  user-select: none;
}

.entry:hover {
  background: var(--bg-hover);
}

/* Active selection: Finder-style accent */
.entry.selected {
  background: var(--scrollbar-thumb);
  color: var(--accent-text);
}

.entry.selected .entry-size,
.entry.selected .entry-arrow {
  color: var(--text-secondary);
}

.entry.drop-target,
.file-row.drop-target {
  background: var(--bg-selected);
  outline: 1px dashed var(--accent);
  outline-offset: -2px;
}

/* Ancestor columns on the opened path: muted highlight */
.entry.opened:not(.selected) {
  background: var(--surface);
  color: var(--text-primary);
}

.entry.opened:not(.selected) .entry-arrow {
  color: var(--accent);
}

.entry-icon {
  font-size: calc(14px * var(--row-scale));
  flex-shrink: 0;
}

.entry-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.entry-size {
  color: var(--text-muted);
  font-size: calc(11px * var(--row-scale));
  flex-shrink: 0;
}

.entry-arrow {
  color: var(--text-muted);
  font-size: calc(12px * var(--row-scale));
  flex-shrink: 0;
}

.col-loading,
.col-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: calc(60px * var(--row-scale));
  color: var(--text-muted);
  font-size: calc(12px * var(--row-scale));
}

/* Explorer-style details list */
.list-view {
  flex: 1;
  min-width: 0;
  overflow: auto;
  background: var(--bg-primary);
  position: relative;
}

.file-header,
.file-row {
  display: grid;
  /* Track sizes come from the store so the dividers can be dragged. */
  padding: calc(6px * var(--row-scale)) calc(14px * var(--row-scale));
  font-size: calc(13px * var(--row-scale));
  align-items: center;
  box-sizing: border-box;
}

.file-header {
  background: var(--bg-secondary);
  color: var(--text-muted);
  font-size: calc(12px * var(--row-scale));
  font-weight: 600;
  position: sticky;
  top: 0;
  z-index: 1;
  border-bottom: 1px solid var(--border);
  user-select: none;
}

.file-header .sort-btn {
  flex: 1;
  min-width: 0;
  background: none;
  border: none;
  padding: 0;
  margin: 0;
  font: inherit;
  color: inherit;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 4px;
  text-align: left;
  overflow: hidden;
  white-space: nowrap;
}

.head-cell {
  position: relative;
  display: flex;
  align-items: center;
  min-width: 0;
}

/*
 * The name cell clips its text with overflow:hidden, which would swallow the
 * grip that hangs over the column edge. Only the sort button needs clipping.
 */
.file-header .head-cell {
  overflow: visible;
}

/* Kept inside the cell so the last column's handle cannot fall off the pane. */
.grip {
  position: absolute;
  top: calc(-6px * var(--row-scale));
  right: 0;
  width: 10px;
  height: calc(100% + 12px * var(--row-scale));
  cursor: col-resize;
  z-index: 2;
}

.grip::after {
  content: '';
  position: absolute;
  top: 15%;
  right: 0;
  width: 1px;
  height: 70%;
  background: var(--border);
}

.grip:hover::after,
.grip.active::after {
  background: var(--accent);
  width: 2px;
}

.file-header .sort-btn:hover {
  color: var(--text-primary);
}

.file-header .sort-btn.active {
  color: var(--accent);
}

.sort-arrow {
  font-size: calc(9px * var(--row-scale));
  line-height: 1;
}

.file-row {
  cursor: pointer;
  color: var(--text-primary);
  transition: background 0.08s;
  user-select: none;
}

.file-row:hover {
  background: var(--bg-hover);
}

.file-row.selected {
  background: var(--scrollbar-thumb);
  color: var(--accent-text);
}

.file-row.selected .col-size,
.file-row.selected .col-perm,
.file-row.selected .col-date {
  color: var(--text-secondary);
}

.col-name {
  display: flex;
  align-items: center;
  gap: calc(7px * var(--row-scale));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.col-size,
.col-perm,
.col-date {
  color: var(--text-muted);
  font-size: calc(12px * var(--row-scale));
}

.col-perm {
  font-family: 'Cascadia Code', Consolas, monospace;
}

.preview-col {
  width: 340px;
  height: 100%;
  overflow-y: auto;
  background: var(--bg-primary);
  border-left: 1px solid var(--border);
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
  background: var(--bg-secondary);
}

.editor-gutter {
  flex-shrink: 0;
  overflow: hidden;
  padding: 12px 0 12px 0;
  background: var(--bg-secondary);
  border-right: 1px solid var(--border);
}

.code-ln {
  flex-shrink: 0;
  box-sizing: border-box;
  padding: 0 10px 0 8px;
  text-align: right;
  color: var(--text-muted);
  user-select: none;
}

.preview-editor {
  flex: 1;
  margin: 0;
  padding: 12px 14px 12px 10px;
  color: var(--text-primary);
  background: var(--bg-secondary);
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
  border-bottom: 1px solid var(--border);
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
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-info {
  font-size: 11px;
  color: var(--text-muted);
  margin-top: 2px;
}

.code-view {
  flex: 1;
  margin: 0;
  padding: 12px 0;
  color: var(--text-primary);
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
  color: var(--text-muted);
  font-size: 12px;
  text-align: center;
}

.preview-load-full {
  margin-left: 8px;
  padding: 3px 10px;
  background: var(--surface);
  color: var(--text-primary);
  border: 1px solid var(--text-disabled);
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
}

.preview-load-full:hover {
  background: var(--text-disabled);
}

.drop-overlay {
  position: absolute;
  inset: 0;
  background: var(--accent-alpha);
  border: 2px dashed var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 10;
  pointer-events: none;
}

.drop-overlay p {
  font-size: 16px;
  color: var(--accent);
  font-weight: 600;
}

.remote-panel.drag-over {
  border: 2px solid var(--accent);
}

.ctx-menu {
  position: fixed;
  z-index: 100;
  min-width: 160px;
  background: var(--bg-panel);
  border: 1px solid var(--text-disabled);
  border-radius: 6px;
  padding: 4px;
  box-shadow: 0 4px 16px var(--shadow-sm);
  display: flex;
  flex-direction: column;
}

.ctx-item {
  display: block;
  width: 100%;
  text-align: left;
  background: none;
  border: none;
  color: var(--text-primary);
  font-size: 13px;
  padding: 6px 10px;
  border-radius: 4px;
  cursor: pointer;
}

.ctx-item:hover:not(:disabled) {
  background: var(--text-disabled);
}

.ctx-item.ctx-danger {
  color: var(--error);
}

.ctx-item:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
