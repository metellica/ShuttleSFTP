<script setup lang="ts">
import { ref, watch, computed } from 'vue'
import { useTabsStore } from '@/stores/tabs'
import { listDir, uploadFiles } from '@/composables/useTauri'
import type { FileEntry } from '@/types/filesystem'

const tabsStore = useTabsStore()
const files = ref<FileEntry[]>([])
const loading = ref(false)
const dragOver = ref(false)

const currentPath = computed(() => tabsStore.activeTab?.currentPath || '/')
const sessionId = computed(() => tabsStore.activeTab?.sessionId || '')

async function fetchDir(path: string) {
  if (!sessionId.value) return
  loading.value = true
  try {
    files.value = await listDir(sessionId.value, path)
    // Sort: directories first, then by name
    files.value.sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
      return a.name.localeCompare(b.name)
    })
  } catch (e) {
    console.error('Failed to list directory:', e)
  } finally {
    loading.value = false
  }
}

function navigateTo(path: string) {
  if (tabsStore.activeTab) {
    tabsStore.updateTab(tabsStore.activeTab.id, { currentPath: path })
  }
}

function navigateUp() {
  const parts = currentPath.value.split('/').filter(Boolean)
  parts.pop()
  navigateTo('/' + parts.join('/'))
}

function onItemClick(entry: FileEntry) {
  if (entry.isDir) {
    navigateTo(entry.path)
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

// Drag & drop from OS (upload)
function onDragOver(e: DragEvent) {
  e.preventDefault()
  dragOver.value = true
}

function onDragLeave() {
  dragOver.value = false
}

async function onDrop(e: DragEvent) {
  e.preventDefault()
  dragOver.value = false

  const droppedFiles = e.dataTransfer?.files
  if (!droppedFiles || !sessionId.value) return

  const paths: string[] = []
  for (let i = 0; i < droppedFiles.length; i++) {
    // In Tauri webview, File.path gives the real filesystem path
    const file = droppedFiles[i] as any
    if (file.path) paths.push(file.path)
  }

  if (paths.length > 0) {
    try {
      await uploadFiles(sessionId.value, paths, currentPath.value)
      await fetchDir(currentPath.value)
    } catch (e) {
      console.error('Upload failed:', e)
    }
  }
}

// Watch path changes
watch(currentPath, (newPath) => fetchDir(newPath), { immediate: true })
</script>

<template>
  <div
    class="remote-panel"
    :class="{ 'drag-over': dragOver }"
    @dragover="onDragOver"
    @dragleave="onDragLeave"
    @drop="onDrop"
  >
    <!-- Path bar -->
    <div class="path-bar">
      <button class="nav-btn" @click="navigateUp" :disabled="currentPath === '/'">⬆</button>
      <span class="path-text">{{ currentPath }}</span>
    </div>

    <!-- File list -->
    <div class="file-list" v-if="!loading">
      <div class="file-header">
        <span class="col-name">Name</span>
        <span class="col-size">Size</span>
        <span class="col-perm">Permissions</span>
        <span class="col-date">Modified</span>
      </div>
      <div
        v-for="entry in files"
        :key="entry.path"
        class="file-row"
        @dblclick="onItemClick(entry)"
      >
        <span class="col-name">
          <span class="icon">{{ entry.isDir ? '📁' : '📄' }}</span>
          {{ entry.name }}
        </span>
        <span class="col-size">{{ entry.isDir ? '-' : formatSize(entry.size) }}</span>
        <span class="col-perm">{{ entry.permissions || '-' }}</span>
        <span class="col-date">{{ formatDate(entry.modified) }}</span>
      </div>
      <div v-if="files.length === 0" class="empty">Empty directory</div>
    </div>

    <div v-else class="loading">Loading...</div>

    <!-- Drop overlay -->
    <div v-if="dragOver" class="drop-overlay">
      <p>Drop files here to upload</p>
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
  gap: 8px;
  padding: 8px 12px;
  background: #181825;
  border-bottom: 1px solid #313244;
}

.nav-btn {
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  color: #cdd6f4;
  cursor: pointer;
  padding: 2px 8px;
  font-size: 12px;
}

.nav-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.path-text {
  font-size: 13px;
  color: #89b4fa;
  font-family: monospace;
}

.file-list {
  flex: 1;
  overflow-y: auto;
}

.file-header,
.file-row {
  display: grid;
  grid-template-columns: 1fr 100px 120px 180px;
  padding: 6px 12px;
  font-size: 12px;
  align-items: center;
}

.file-header {
  background: #181825;
  color: #a6adc8;
  border-bottom: 1px solid #313244;
  position: sticky;
  top: 0;
  font-weight: 600;
}

.file-row {
  cursor: pointer;
  border-bottom: 1px solid #1e1e2e;
}

.file-row:hover {
  background: #313244;
}

.col-name {
  display: flex;
  align-items: center;
  gap: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.col-size,
.col-perm,
.col-date {
  color: #a6adc8;
}

.col-perm {
  font-family: monospace;
}

.icon {
  font-size: 14px;
}

.loading,
.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100px;
  color: #6c7086;
  font-size: 13px;
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
}

.drop-overlay p {
  font-size: 16px;
  color: #89b4fa;
  font-weight: 600;
}

.remote-panel.drag-over {
  border: 2px solid #89b4fa;
}
</style>
