<script setup lang="ts">
import { ref } from 'vue'
import { useTabsStore, type Tab } from '@/stores/tabs'
import { useTransferStore } from '@/stores/transfer'
import { usePrepareStore } from '@/stores/prepare'
import { transferRemote } from '@/composables/useTauri'

const props = defineProps<{ pane: import('@/stores/tabs').Pane }>()
const emit = defineEmits<{ 'new-tab': [] }>()
const tabsStore = useTabsStore()
const transferStore = useTransferStore()
const prepareStore = usePrepareStore()

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
    await prepareStore.run('Preparing copy', (pid) =>
      transferRemote(payload.sessionId, payload.paths, tab.sessionId!, tab.currentPath, pid)
    )
    await transferStore.syncTasks()
  } catch (e) {
    console.error('Cross-tab copy failed:', e)
  }
}
</script>

<template>
  <div class="tab-bar" @click="hideTabCtxMenu">
    <div
      v-for="tab in props.pane.tabs"
      :key="tab.id"
      class="tab"
      :class="{ active: tab.id === props.pane.activeTabId, 'drop-target': tab.id === dragTabId }"
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
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border);
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
  border-right: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-secondary);
  min-width: 120px;
  max-width: 200px;
}

/* The accent bar is what makes the current tab findable at a glance:
   the background alone is a shade apart from the bar's, which is easy
   to lose on a row of connected sessions. Matches ShuttleFiles. */
.tab.active {
  background: var(--bg-primary);
  color: var(--text-primary);
  box-shadow: inset 0 2px 0 var(--accent);
}

.tab.drop-target {
  background: var(--bg-selected);
  outline: 1px dashed var(--accent);
  outline-offset: -2px;
}

.tab-kind {
  color: var(--accent);
  font-size: 11px;
  flex-shrink: 0;
}

.tab:hover {
  background: var(--bg-hover);
}

.tab-status {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-muted);
}

.tab-status.connected { background: var(--success); }
.tab-status.connecting { background: var(--warning); }
.tab-status.error { background: var(--error); }

.tab-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tab-close {
  background: none;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 16px;
  line-height: 1;
  padding: 2px;
}

.tab-close:hover { color: var(--error); }

.tab-add {
  background: none;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 18px;
  padding: 0 12px;
}

.tab-add:hover { color: var(--text-primary); }

.ctx-menu {
  position: fixed;
  z-index: 200;
  min-width: 200px;
  background: var(--bg-panel);
  border: 1px solid var(--text-disabled);
  border-radius: 6px;
  box-shadow: 0 4px 16px var(--shadow-sm);
  padding: 4px;
  display: flex;
  flex-direction: column;
}

.ctx-item {
  background: none;
  border: none;
  color: var(--text-primary);
  text-align: left;
  padding: 6px 10px;
  font-size: 12px;
  cursor: pointer;
  border-radius: 4px;
}

.ctx-item:hover {
  background: var(--text-disabled);
}
</style>
