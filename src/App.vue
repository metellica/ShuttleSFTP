<script setup lang="ts">
import { useTabsStore } from '@/stores/tabs'
import TabBar from '@/components/layout/TabBar.vue'
import Toolbar from '@/components/layout/Toolbar.vue'
import ConnectDialog from '@/components/connection/ConnectDialog.vue'
import BookmarksDialog from '@/components/connection/BookmarksDialog.vue'
import RemotePanel from '@/components/browser/RemotePanel.vue'
import TransferQueue from '@/components/transfer/TransferQueue.vue'
import { ref, onMounted, onUnmounted } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { uploadFiles, downloadFiles, mkDir } from '@/composables/useTauri'
import { useTransferStore } from '@/stores/transfer'
import type { TransferProgress, TransferTask } from '@/types/transfer'
import type { ConnectedMeta } from '@/types/connection'
import type { Tab } from '@/stores/tabs'

const tabsStore = useTabsStore()
const transferStore = useTransferStore()
const showConnectDialog = ref(false)
const showBookmarksDialog = ref(false)
const connectDialogMode = ref<'ssh' | 'container' | 'pod'>('ssh')
const connectDialogVia = ref<string | undefined>(undefined)
const remotePanelRef = ref<InstanceType<typeof RemotePanel> | null>(null)
const unlisteners: UnlistenFn[] = []

function preventDefaultContextMenu(e: MouseEvent) {
  e.preventDefault()
}

onMounted(async () => {
  document.addEventListener('contextmenu', preventDefaultContextMenu)
  if (tabsStore.tabs.length === 0) {
    tabsStore.addTab()
    showConnectDialog.value = true
  }

  // Restore persisted transfers (interrupted ones come back as paused)
  transferStore.syncTasks().catch((e) => console.error('Cannot load transfers:', e))

  unlisteners.push(
    await listen<TransferProgress>('transfer:progress', (e) => {
      transferStore.updateTask(e.payload.taskId, {
        transferredBytes: e.payload.transferredBytes,
        totalBytes: e.payload.totalBytes,
        speed: e.payload.speed,
      })
    })
  )
  unlisteners.push(
    await listen<{ taskId: string; status: TransferTask['status'] }>(
      'transfer:status',
      (e) => {
        transferStore.updateTask(e.payload.taskId, { status: e.payload.status })
        if (e.payload.status === 'completed') {
          const task = transferStore.tasks.find((t) => t.id === e.payload.taskId)
          // Refresh unless we know it was a download (remote dir unchanged)
          if (!task || task.direction === 'upload') {
            remotePanelRef.value?.refresh()
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
  connectDialogMode.value = 'ssh'
  connectDialogVia.value = undefined
  showConnectDialog.value = true
}

function onShowConnect() {
  connectDialogMode.value = 'ssh'
  connectDialogVia.value = undefined
  showConnectDialog.value = true
}

/** Tab context menu: browse containers running on an SSH host. */
function onBrowseContainers(tab: Tab) {
  if (!tab.sessionId) return
  connectDialogMode.value = 'container'
  connectDialogVia.value = tab.sessionId
  tabsStore.addTab()
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
      containerSpec: meta.containerSpec ?? null,
      podSpec: meta.podSpec ?? null,
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
  // Reuse the active tab if it's idle, otherwise open a new one
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
    containerSpec: meta.containerSpec ?? null,
    podSpec: meta.podSpec ?? null,
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
    await uploadFiles(tab.sessionId, paths, tab.currentPath)
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
    await uploadFiles(tab.sessionId, paths, tab.currentPath)
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Upload folder failed:', e)
  }
}

async function onDownload() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return

  const selectedFiles = remotePanelRef.value?.selectedFiles
  if (!selectedFiles || selectedFiles.length === 0) return

  const localDir = await open({
    directory: true,
    title: 'Choose download location',
  })
  if (!localDir) return

  const remotePaths = selectedFiles.map((f) => f.path)
  try {
    await downloadFiles(tab.sessionId, remotePaths, localDir as string)
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Download failed:', e)
  }
}

function onRefresh() {
  remotePanelRef.value?.refresh()
}

async function onNewFolder() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return

  const name = prompt('Enter folder name:')
  if (!name) return

  const path = tab.currentPath === '/'
    ? `/${name}`
    : `${tab.currentPath}/${name}`
  try {
    await mkDir(tab.sessionId, path)
    remotePanelRef.value?.refresh()
  } catch (e) {
    console.error('Create folder failed:', e)
  }
}
</script>

<template>
  <div class="app-container">
    <TabBar @new-tab="onNewTab" @browse-containers="onBrowseContainers" />
    <Toolbar
      @connect="onShowConnect"
      @bookmarks="showBookmarksDialog = true"
      @upload="onUpload"
      @upload-folder="onUploadFolder"
      @download="onDownload"
      @refresh="onRefresh"
      @new-folder="onNewFolder"
    />
    <main class="main-content">
      <RemotePanel v-if="tabsStore.activeTab?.status === 'connected'" ref="remotePanelRef" />
      <div v-else class="empty-state">
        <p>Click "Connect" or press the + tab to start a new SFTP session</p>
      </div>
    </main>
    <TransferQueue />
    <ConnectDialog
      v-if="showConnectDialog"
      :initial-mode="connectDialogMode"
      :via-session-id="connectDialogVia"
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

/* Flat themed scrollbars (WebView2 / Chromium) */
::-webkit-scrollbar {
  width: 12px;
  height: 12px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: #4f6ec2;
  border-radius: 6px;
}

::-webkit-scrollbar-thumb:hover {
  background: #6b8ae0;
}

::-webkit-scrollbar-thumb:active {
  background: #89b4fa;
}

::-webkit-scrollbar-corner {
  background: transparent;
}
</style>

<style scoped>
.app-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #1e1e2e;
  color: #cdd6f4;
}

.main-content {
  flex: 1;
  overflow: hidden;
}

.empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #6c7086;
  font-size: 14px;
}
</style>
