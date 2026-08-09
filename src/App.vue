<script setup lang="ts">
import { useTabsStore, type Pane } from '@/stores/tabs'
import { useViewSettingsStore } from '@/stores/viewSettings'
import TabBar from '@/components/layout/TabBar.vue'
import Toolbar from '@/components/layout/Toolbar.vue'
import ConnectDialog from '@/components/connection/ConnectDialog.vue'
import BookmarksDialog from '@/components/connection/BookmarksDialog.vue'
import RemotePanel from '@/components/browser/RemotePanel.vue'
import TransferQueue from '@/components/transfer/TransferQueue.vue'
import PrepareOverlay from '@/components/layout/PrepareOverlay.vue'
import TerminalPanel from '@/components/terminal/TerminalPanel.vue'
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { uploadFiles, downloadFiles, mkDir, listBookmarks } from '@/composables/useTauri'
import { promptText } from '@/composables/usePrompt'
import { useTransferStore } from '@/stores/transfer'
import { usePrepareStore, type PrepareProgressEvent } from '@/stores/prepare'
import { useTerminalsStore } from '@/stores/terminals'
import type { TransferProgress, TransferTask } from '@/types/transfer'
import type { ConnectedMeta } from '@/types/connection'

const tabsStore = useTabsStore()
const transferStore = useTransferStore()
const prepareStore = usePrepareStore()
const view = useViewSettingsStore()
const showConnectDialog = ref(false)
const showBookmarksDialog = ref(false)
const unlisteners: UnlistenFn[] = []
const terminalsStore = useTerminalsStore()
const contentRef = ref<HTMLElement | null>(null)

/** One RemotePanel per pane, keyed by pane id. */
const panels = new Map<string, InstanceType<typeof RemotePanel>>()

function setPanel(paneId: string, el: unknown) {
  if (el) panels.set(paneId, el as InstanceType<typeof RemotePanel>)
}

function activePanel(): InstanceType<typeof RemotePanel> | null {
  return panels.get(tabsStore.activePaneId) ?? null
}

/** The active tab for a given pane — independent of which pane is focused. */
function paneTab(pane: Pane) {
  return pane.tabs.find((t) => t.id === pane.activeTabId) ?? null
}

/** Both rows of the split — the tab bars and the panes — share a grid. */
const columns = computed(() =>
  tabsStore.split ? `${view.splitRatio}fr 5px ${1 - view.splitRatio}fr` : '1fr'
)

function onOpenTerminal() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return
  terminalsStore.open(tab.id, tab.sessionId, tab.currentPath)
}

// --- Transfer event coalescing -------------------------------------------

const pendingProgress = new Map<string, TransferProgress>()
let progressTimer: ReturnType<typeof setTimeout> | null = null
function queueProgress(p: TransferProgress) {
  pendingProgress.set(p.taskId, p)
  progressTimer ??= setTimeout(() => {
    progressTimer = null
    for (const [taskId, prog] of pendingProgress) {
      transferStore.updateTask(taskId, {
        transferredBytes: prog.transferredBytes,
        totalBytes: prog.totalBytes,
        speed: prog.speed,
      })
    }
    pendingProgress.clear()
  }, 100)
}

let syncTimer: ReturnType<typeof setTimeout> | null = null
function queueSync() {
  syncTimer ??= setTimeout(() => {
    syncTimer = null
    transferStore.syncTasks().catch((e) => console.error('Cannot sync transfers:', e))
  }, 100)
}

let refreshTimer: ReturnType<typeof setTimeout> | null = null
function queueRefresh() {
  refreshTimer ??= setTimeout(() => {
    refreshTimer = null
    activePanel()?.refresh()
  }, 500)
}

function preventDefaultContextMenu(e: MouseEvent) {
  e.preventDefault()
}

onMounted(async () => {
  document.addEventListener('contextmenu', preventDefaultContextMenu)
  if (tabsStore.tabs.length === 0) {
    tabsStore.addTab()
    try {
      const bookmarks = await listBookmarks()
      if (bookmarks.length > 0) {
        showBookmarksDialog.value = true
      } else {
        showConnectDialog.value = true
      }
    } catch {
      showConnectDialog.value = true
    }
  }

  transferStore.syncTasks().catch((e) => console.error('Cannot load transfers:', e))

  unlisteners.push(
    await listen<PrepareProgressEvent>('prepare:progress', (e) => {
      prepareStore.onProgress(e.payload)
    })
  )
  unlisteners.push(
    await listen('transfer:bulk-update', () => {
      queueSync()
    })
  )
  unlisteners.push(
    await listen<TransferTask>('transfer:queued', (e) => {
      transferStore.addTask(e.payload)
    })
  )
  unlisteners.push(
    await listen<TransferProgress>('transfer:progress', (e) => {
      queueProgress(e.payload)
    })
  )
  unlisteners.push(
    await listen<{ taskId: string; status: TransferTask['status'] }>(
      'transfer:status',
      (e) => {
        transferStore.updateTask(e.payload.taskId, { status: e.payload.status })
        if (e.payload.status === 'completed') {
          const task = transferStore.getTask(e.payload.taskId)
          if (!task || task.direction === 'upload' || task.direction === 'remote') {
            queueRefresh()
          }
        }
      }
    )
  )
})

onUnmounted(() => {
  document.removeEventListener('contextmenu', preventDefaultContextMenu)
  unlisteners.forEach((u) => u())
})

function onNewTab() {
  tabsStore.addTab()
  showConnectDialog.value = true
}

function onShowConnect() {
  showConnectDialog.value = true
}

function onConnected(sessionId: string, label: string, meta: ConnectedMeta) {
  if (tabsStore.activeTab) {
    tabsStore.updateTab(tabsStore.activeTab.id, {
      sessionId,
      label,
      status: 'connected',
      currentPath: meta.initialPath ?? '/',
      kind: meta.kind,
      connectParams: meta.params,
    })
  }
  showConnectDialog.value = false
}

function onBookmarkConnected(
  sessionId: string,
  label: string,
  path: string,
  meta: ConnectedMeta
) {
  const tab =
    tabsStore.activeTab && tabsStore.activeTab.status === 'disconnected'
      ? tabsStore.activeTab
      : tabsStore.addTab()
  tabsStore.updateTab(tab.id, {
    sessionId,
    label,
    status: 'connected',
    currentPath: path,
    kind: meta.kind,
    connectParams: meta.params,
  })
  showBookmarksDialog.value = false
}

async function onUpload() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return

  const selected = await open({
    multiple: true,
    directory: false,
    title: 'Select files to upload',
  })
  if (!selected) return

  const paths = Array.isArray(selected) ? selected : [selected]
  try {
    await prepareStore.run('Preparing upload', (pid) =>
      uploadFiles(tab.sessionId!, paths, tab.currentPath, pid)
    )
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Upload failed:', e)
  }
}

async function onUploadFolder() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return

  const selected = await open({
    multiple: true,
    directory: true,
    title: 'Select folders to upload',
  })
  if (!selected) return

  const paths = Array.isArray(selected) ? selected : [selected]
  try {
    await prepareStore.run('Preparing upload', (pid) =>
      uploadFiles(tab.sessionId!, paths, tab.currentPath, pid)
    )
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Upload folder failed:', e)
  }
}

async function onDownload() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return

  const selectedFiles = activePanel()?.selectedFiles
  if (!selectedFiles || selectedFiles.length === 0) return

  const localDir = await open({
    directory: true,
    title: 'Choose download location',
  })
  if (!localDir) return

  const remotePaths = selectedFiles.map((f) => f.path)
  try {
    await prepareStore.run('Preparing download', (pid) =>
      downloadFiles(tab.sessionId!, remotePaths, localDir as string, pid)
    )
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Download failed:', e)
  }
}

function onRefresh() {
  activePanel()?.refresh()
}

async function onNewFolder() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return

  const name = await promptText('Enter folder name:')
  if (!name) return

  const path = tab.currentPath === '/'
    ? `/${name}`
    : `${tab.currentPath}/${name}`
  try {
    await mkDir(tab.sessionId, path)
    activePanel()?.refresh()
  } catch (e) {
    console.error('Create folder failed:', e)
  }
}

// --- Splitter drag -------------------------------------------------------

function startSplitDrag() {
  const host = contentRef.value
  if (!host) return
  const rect = host.getBoundingClientRect()
  const move = (e: MouseEvent) => view.setSplitRatio((e.clientX - rect.left) / rect.width)
  const stop = () => {
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', stop)
    document.body.classList.remove('splitting')
  }
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', stop)
  document.body.classList.add('splitting')
}

// --- Keyboard shortcuts --------------------------------------------------

function onKeyDown(e: KeyboardEvent) {
  const mod = e.ctrlKey || e.metaKey
  // Ctrl+\ toggles the split
  if (mod && e.key === '\\') {
    e.preventDefault()
    tabsStore.toggleSplit()
  }
}

onMounted(() => window.addEventListener('keydown', onKeyDown))
onUnmounted(() => window.removeEventListener('keydown', onKeyDown))
</script>

<template>
  <div class="app-container">
    <div class="tab-bars" :style="{ gridTemplateColumns: columns }">
      <template v-for="(pane, index) in tabsStore.panes" :key="pane.id">
        <div v-if="index > 0" class="tab-bars-gap" />
        <TabBar :pane="pane" @new-tab="tabsStore.addTabIn(pane.id)" />
      </template>
    </div>
    <Toolbar
      :split="tabsStore.split"
      @connect="onShowConnect"
      @bookmarks="showBookmarksDialog = true"
      @upload="onUpload"
      @upload-folder="onUploadFolder"
      @download="onDownload"
      @refresh="onRefresh"
      @new-folder="onNewFolder"
      @terminal="onOpenTerminal"
      @toggle-split="tabsStore.toggleSplit()"
    />
    <main ref="contentRef" class="main-content" :style="{ gridTemplateColumns: columns }">
      <template v-for="(pane, index) in tabsStore.panes" :key="pane.id">
        <div
          v-if="index > 0"
          class="splitter"
          title="Drag to resize, double-click to even out"
          @mousedown.prevent="startSplitDrag"
          @dblclick="view.setSplitRatio(0.5)"
        />
        <section
          class="pane"
          :class="{ focused: tabsStore.split && tabsStore.activePaneId === pane.id }"
          @mousedown.capture="tabsStore.setActivePane(pane.id)"
        >
          <RemotePanel
            v-if="paneTab(pane)?.status === 'connected'"
            :ref="(el) => setPanel(pane.id, el)"
            :tab-id="paneTab(pane)?.id"
          />
          <div v-else class="empty-state">
            <p>Click "Connect" or press the + tab to start a new SFTP session</p>
          </div>
        </section>
      </template>
    </main>
    <TerminalPanel />
    <TransferQueue />
    <PrepareOverlay />
    <ConnectDialog
      v-if="showConnectDialog"
      @close="showConnectDialog = false"
      @connected="onConnected"
    />
    <BookmarksDialog
      v-if="showBookmarksDialog"
      @close="showBookmarksDialog = false"
      @connected="onBookmarkConnected"
    />
  </div>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  overflow: hidden;
}

::-webkit-scrollbar {
  width: 12px;
  height: 12px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: var(--scrollbar-thumb);
  border-radius: 6px;
}

::-webkit-scrollbar-thumb:hover {
  background: var(--scrollbar-thumb-hover);
}

::-webkit-scrollbar-thumb:active {
  background: var(--scrollbar-thumb-active);
}

::-webkit-scrollbar-corner {
  background: transparent;
}

body.col-resizing {
  cursor: col-resize;
  user-select: none;
}

body.splitting {
  cursor: col-resize;
  user-select: none;
}
</style>

<style scoped>
.app-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: var(--bg-primary);
  color: var(--text-primary);
}

.tab-bars {
  display: grid;
  min-width: 0;
}

.tab-bars-gap {
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border);
  border-left: 1px solid var(--border);
}

.main-content {
  flex: 1;
  overflow: hidden;
  display: grid;
  min-height: 0;
}

.pane {
  position: relative;
  overflow: hidden;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.pane.focused::after {
  content: '';
  position: absolute;
  inset: 0;
  border-top: 2px solid var(--accent);
  pointer-events: none;
}

.splitter {
  background: var(--border);
  cursor: col-resize;
}

.splitter:hover {
  background: var(--accent);
}

.empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-muted);
  font-size: 14px;
}
</style>
