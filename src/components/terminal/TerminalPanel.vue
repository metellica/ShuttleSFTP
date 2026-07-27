<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useTabsStore } from '@/stores/tabs'
import { useTerminalsStore } from '@/stores/terminals'
import TerminalView from '@/components/terminal/TerminalView.vue'

const tabsStore = useTabsStore()
const terminalsStore = useTerminalsStore()

/** Terminals of the currently shown browser tab. */
const activeTabTerms = computed(() => {
  const tabId = tabsStore.activeTabId
  if (!tabId) return []
  return terminalsStore.byTab.get(tabId) ?? []
})

const activeKey = computed(() => {
  const tabId = tabsStore.activeTabId
  return tabId ? terminalsStore.activeByTab[tabId] : undefined
})

// Terminals die with their browser tab.
watch(
  () => tabsStore.tabs.map((t) => t.id),
  (ids) => {
    const alive = new Set(ids)
    for (const t of [...terminalsStore.terms]) {
      if (!alive.has(t.tabId)) terminalsStore.closeForTab(t.tabId)
    }
  }
)

/** New terminal in the active tab's current directory. */
function addTerminal() {
  const tab = tabsStore.activeTab
  if (!tab?.sessionId) return
  terminalsStore.open(tab.id, tab.sessionId, tab.currentPath)
}

// --- Drag-resizable panel height ---
const height = ref(280)
function onResizeStart(e: MouseEvent) {
  e.preventDefault()
  const startY = e.clientY
  const startH = height.value
  const max = Math.round(window.innerHeight * 0.8)
  function onMove(ev: MouseEvent) {
    height.value = Math.min(max, Math.max(120, startH + (startY - ev.clientY)))
  }
  function onUp() {
    document.removeEventListener('mousemove', onMove)
    document.removeEventListener('mouseup', onUp)
    document.body.style.cursor = ''
    document.body.style.userSelect = ''
  }
  document.addEventListener('mousemove', onMove)
  document.addEventListener('mouseup', onUp)
  document.body.style.cursor = 'ns-resize'
  document.body.style.userSelect = 'none'
}
</script>

<template>
  <div class="terminal-panel" v-show="activeTabTerms.length > 0" :style="{ height: height + 'px' }">
    <div class="resize-handle" title="Drag to resize" @mousedown="onResizeStart" />
    <div class="term-header">
      <span class="term-icon">🖥</span>
      <div class="term-tabs">
        <div
          v-for="t in activeTabTerms"
          :key="t.key"
          class="term-tab"
          :class="{ active: t.key === activeKey }"
          :title="t.path"
          @click="terminalsStore.setActive(t.tabId, t.key)"
        >
          <span class="term-tab-title">{{ t.title }}</span>
          <button
            class="term-tab-close"
            title="Close terminal"
            @click.stop="terminalsStore.close(t.key)"
          >
            ✕
          </button>
        </div>
        <button class="term-add" title="New terminal in current directory" @click="addTerminal">+</button>
      </div>
    </div>
    <!-- All terminals stay mounted; only the active tab's active one shows -->
    <TerminalView
      v-for="t in terminalsStore.terms"
      :key="t.key"
      :session-id="t.sessionId"
      :path="t.path"
      :visible="t.tabId === tabsStore.activeTabId && t.key === activeKey"
    />
  </div>
</template>

<style scoped>
.terminal-panel {
  display: flex;
  flex-direction: column;
  min-height: 120px;
  background: #181825;
  border-top: 1px solid #313244;
  flex-shrink: 0;
  position: relative;
}

.resize-handle {
  position: absolute;
  top: -3px;
  left: 0;
  right: 0;
  height: 6px;
  cursor: ns-resize;
  z-index: 10;
}

.resize-handle:hover {
  background: rgba(137, 180, 250, 0.3);
}

.term-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 10px;
  background: #1e1e2e;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}

.term-icon {
  font-size: 11px;
  flex-shrink: 0;
}

.term-tabs {
  display: flex;
  gap: 4px;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
}

.term-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 4px 2px 10px;
  background: #24243a;
  border: 1px solid #313244;
  border-radius: 4px;
  font-size: 12px;
  color: #a6adc8;
  cursor: pointer;
  flex-shrink: 0;
  max-width: 220px;
  user-select: none;
}

.term-tab.active {
  background: #313244;
  color: #cdd6f4;
  border-color: #45475a;
}

.term-tab-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: monospace;
}

.term-tab-close {
  background: transparent;
  border: none;
  color: #6c7086;
  cursor: pointer;
  font-size: 10px;
  padding: 2px 4px;
  border-radius: 3px;
  flex-shrink: 0;
}

.term-tab-close:hover {
  background: #45475a;
  color: #f38ba8;
}

.term-add {
  background: transparent;
  border: 1px solid #313244;
  border-radius: 4px;
  color: #a6adc8;
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
  padding: 1px 8px;
  flex-shrink: 0;
}

.term-add:hover {
  background: #313244;
  color: #cdd6f4;
}
</style>
