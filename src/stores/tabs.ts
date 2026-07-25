import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import type { ConnectParams, SessionKind } from '@/types/connection'

export interface Tab {
  id: string
  sessionId: string | null
  label: string
  status: 'disconnected' | 'connecting' | 'connected' | 'error'
  currentPath: string
  /** Endpoint type of the session shown in this tab. */
  kind: SessionKind
  /** Params used to establish the SSH leg; needed for bookmarking. */
  connectParams: ConnectParams | null
  /** Visited directories, for back/forward navigation. */
  history: string[]
  historyIndex: number
}

export const useTabsStore = defineStore('tabs', () => {
  const tabs = ref<Tab[]>([])
  const activeTabId = ref<string | null>(null)

  const activeTab = computed(() =>
    tabs.value.find((t) => t.id === activeTabId.value) ?? null
  )

  function addTab(): Tab {
    const id = crypto.randomUUID()
    const tab: Tab = {
      id,
      sessionId: null,
      label: 'New Connection',
      status: 'disconnected',
      currentPath: '/',
      kind: 'ssh',
      connectParams: null,
      history: ['/'],
      historyIndex: 0,
    }
    tabs.value.push(tab)
    activeTabId.value = id
    return tab
  }

  function closeTab(tabId: string) {
    const index = tabs.value.findIndex((t) => t.id === tabId)
    if (index === -1) return
    tabs.value.splice(index, 1)
    if (activeTabId.value === tabId) {
      activeTabId.value = tabs.value[Math.min(index, tabs.value.length - 1)]?.id ?? null
    }
  }

  function setActiveTab(tabId: string) {
    activeTabId.value = tabId
  }

  function updateTab(tabId: string, updates: Partial<Tab>) {
    const tab = tabs.value.find((t) => t.id === tabId)
    if (!tab) return
    if (updates.sessionId !== undefined && updates.sessionId !== tab.sessionId) {
      // New session in this tab: old history is meaningless
      const start = updates.currentPath ?? tab.currentPath
      tab.history = [start]
      tab.historyIndex = 0
    } else if (
      updates.currentPath !== undefined &&
      updates.currentPath !== tab.currentPath
    ) {
      // Normal navigation: drop the forward stack, push the new path
      tab.history = tab.history.slice(0, tab.historyIndex + 1)
      tab.history.push(updates.currentPath)
      tab.historyIndex = tab.history.length - 1
    }
    Object.assign(tab, updates)
  }

  const canGoBack = computed(() => (activeTab.value?.historyIndex ?? 0) > 0)
  const canGoForward = computed(() => {
    const t = activeTab.value
    return !!t && t.historyIndex < t.history.length - 1
  })

  function goBack() {
    const tab = activeTab.value
    if (!tab || tab.historyIndex <= 0) return
    tab.historyIndex--
    // Set directly: history moves must not re-push
    tab.currentPath = tab.history[tab.historyIndex] ?? '/'
  }

  function goForward() {
    const tab = activeTab.value
    if (!tab || tab.historyIndex >= tab.history.length - 1) return
    tab.historyIndex++
    tab.currentPath = tab.history[tab.historyIndex] ?? '/'
  }

  return {
    tabs,
    activeTabId,
    activeTab,
    canGoBack,
    canGoForward,
    addTab,
    closeTab,
    setActiveTab,
    updateTab,
    goBack,
    goForward,
  }
})
