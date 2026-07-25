<script setup lang="ts">
import { ref } from 'vue'
import { useTabsStore, type Tab } from '@/stores/tabs'
import { useTransferStore } from '@/stores/transfer'
import { transferRemote } from '@/composables/useTauri'

const emit = defineEmits<{ 'new-tab': [] }>()
const tabsStore = useTabsStore()
const transferStore = useTransferStore()

const KIND_ICONS: Record<string, string> = { ssh: '⌁', local: '💻' }

const tabCtxMenu = ref<{ visible: boolean; x: number; y: number; tab: Tab | null }>({
  visible: false,
  x: 0,
  y: 0,
  tab: null,
})

function onTabContextMenu(tab: Tab, event: MouseEvent) {
  tabCtxMenu.value = { visible: true, x: event.clientX, y: event.clientY, tab }
}

function hideTabCtxMenu() {
  tabCtxMenu.value.visible = false
}

function ctxCloseTab() {
  const tab = tabCtxMenu.value.tab
  hideTabCtxMenu()
  if (tab) tabsStore.closeTab(tab.id)
}

// --- Drag files from a panel onto another tab -------------------------------

const dragTabId = ref<string | null>(null)
let hoverSwitchTimer: number | undefined

function onTabDragOver(tab: Tab, event: DragEvent) {
  if (!event.dataTransfer?.types.includes('application/x-shuttle-files')) return
  if (!tab.sessionId || tab.status !== 'connected') return
  event.preventDefault()
  event.dataTransfer.dropEffect = 'copy'
  if (dragTabId.value !== tab.id) {
    dragTabId.value = tab.id
    // Hovering a tab briefly switches to it, like OS docks do
    if (hoverSwitchTimer !== undefined) clearTimeout(hoverSwitchTimer)
    hoverSwitchTimer = window.setTimeout(() => {
      if (dragTabId.value === tab.id) tabsStore.setActiveTab(tab.id)
    }, 600)
  }
}

function onTabDragLeave(tab: Tab) {
  if (dragTabId.value === tab.id) dragTabId.value = null
}

async function onTabDrop(tab: Tab, event: DragEvent) {
  dragTabId.value = null
  if (hoverSwitchTimer !== undefined) clearTimeout(hoverSwitchTimer)
  const raw = event.dataTransfer?.getData('application/x-shuttle-files')
  if (!raw || !tab.sessionId) return
  event.preventDefault()
  try {
    const payload = JSON.parse(raw) as { sessionId: string; paths: string[] }
    if (!payload.sessionId || payload.sessionId === tab.sessionId) return
    await transferRemote(payload.sessionId, payload.paths, tab.sessionId, tab.currentPath)
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Cross-tab copy failed:', e)
  }
}
</script>

<template>
  <div class="tab-bar" @click="hideTabCtxMenu">
    <div
      v-for="tab in tabsStore.tabs"
      :key="tab.id"
      class="tab"
      :class="{ active: tab.id === tabsStore.activeTabId, 'drop-target': tab.id === dragTabId }"
      @click="tabsStore.setActiveTab(tab.id)"
      @contextmenu.prevent="onTabContextMenu(tab, $event)"
      @dragover="onTabDragOver(tab, $event)"
      @dragleave="onTabDragLeave(tab)"
      @drop="onTabDrop(tab, $event)"
    >
      <span class="tab-status" :class="tab.status" />
      <span class="tab-kind" :title="tab.kind">{{ KIND_ICONS[tab.kind] ?? '⌁' }}</span>
      <span class="tab-label">{{ tab.label }}</span>
      <button class="tab-close" @click.stop="tabsStore.closeTab(tab.id)">×</button>
    </div>
    <button class="tab-add" @click="emit('new-tab')">+</button>

    <!-- Tab context menu -->
    <div
      v-if="tabCtxMenu.visible"
      class="ctx-menu"
      :style="{ left: tabCtxMenu.x + 'px', top: tabCtxMenu.y + 'px' }"
      @click.stop
    >
      <button class="ctx-item" @click="ctxCloseTab">× Close Tab</button>
    </div>
  </div>
</template>

<style scoped>
.tab-bar {
  display: flex;
  background: #181825;
  border-bottom: 1px solid #313244;
  height: 36px;
  align-items: stretch;
  overflow-x: auto;
  user-select: none;
}

.tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  cursor: pointer;
  border-right: 1px solid #313244;
  font-size: 12px;
  color: #a6adc8;
  min-width: 120px;
  max-width: 200px;
}

.tab.active {
  background: #1e1e2e;
  color: #cdd6f4;
}

.tab.drop-target {
  background: #2c3a5c;
  outline: 1px dashed #89b4fa;
  outline-offset: -2px;
}

.tab-kind {
  color: #89b4fa;
  font-size: 11px;
  flex-shrink: 0;
}

.tab:hover {
  background: #242438;
}

.tab-status {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #6c7086;
}

.tab-status.connected { background: #a6e3a1; }
.tab-status.connecting { background: #f9e2af; }
.tab-status.error { background: #f38ba8; }

.tab-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tab-close {
  background: none;
  border: none;
  color: #6c7086;
  cursor: pointer;
  font-size: 16px;
  line-height: 1;
  padding: 2px;
}

.tab-close:hover { color: #f38ba8; }

.tab-add {
  background: none;
  border: none;
  color: #a6adc8;
  cursor: pointer;
  font-size: 18px;
  padding: 0 12px;
}

.tab-add:hover { color: #cdd6f4; }

.ctx-menu {
  position: fixed;
  z-index: 200;
  min-width: 200px;
  background: #24243a;
  border: 1px solid #45475a;
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  padding: 4px;
  display: flex;
  flex-direction: column;
}

.ctx-item {
  background: none;
  border: none;
  color: #cdd6f4;
  text-align: left;
  padding: 6px 10px;
  font-size: 12px;
  cursor: pointer;
  border-radius: 4px;
}

.ctx-item:hover {
  background: #45475a;
}
</style>
