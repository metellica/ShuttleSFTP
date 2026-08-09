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

/**
 * One side of a split window: its own tabs, its own front tab.
 *
 * Two panes side by side so you can browse two servers, or two folders
 * on the same server, and transfer between them.
 */
export interface Pane {
  id: string
  tabs: Tab[]
  activeTabId: string
}

/** The window is never paneless; two is as many as the split allows. */
export const MAX_PANES = 2

export const useTabsStore = defineStore('tabs', () => {
  const panes = ref<Pane[]>([])
  const activePaneId = ref('')

  const split = computed(() => panes.value.length > 1)
  const activePane = computed(
    () => panes.value.find((p) => p.id === activePaneId.value) ?? panes.value[0] ?? null
  )

  /** The focused pane's tabs — what the toolbar and shortcuts act on. */
  const tabs = computed(() => activePane.value?.tabs ?? [])
  const activeTabId = computed(() => activePane.value?.activeTabId ?? null)
  const activeTab = computed(() => findTab(activeTabId.value))

  const canGoBack = computed(() => (activeTab.value?.historyIndex ?? 0) > 0)
  const canGoForward = computed(() => {
    const t = activeTab.value
    return !!t && t.historyIndex < t.history.length - 1
  })

  /** Tab ids are unique across the window, so a lookup need not say where. */
  function findTab(id: string | null): Tab | null {
    if (!id) return null
    for (const pane of panes.value) {
      const tab = pane.tabs.find((t) => t.id === id)
      if (tab) return tab
    }
    return null
  }

  function paneOf(tabId: string): Pane | null {
    return panes.value.find((p) => p.tabs.some((t) => t.id === tabId)) ?? null
  }

  function paneById(id: string): Pane | null {
    return panes.value.find((p) => p.id === id) ?? null
  }

  /** The window always has a pane to put a tab in, even at startup. */
  function ensurePane(): Pane {
    if (panes.value.length === 0) {
      panes.value.push({ id: crypto.randomUUID(), tabs: [], activeTabId: '' })
    }
    if (!paneById(activePaneId.value)) activePaneId.value = panes.value[0]!.id
    return paneById(activePaneId.value)!
  }

  function addTab(): Tab {
    return addTabIn(ensurePane().id)
  }

  function addTabIn(paneId: string): Tab {
    const pane = paneById(paneId) ?? ensurePane()
    const tab: Tab = {
      id: crypto.randomUUID(),
      sessionId: null,
      label: 'New Connection',
      status: 'disconnected',
      currentPath: '/',
      kind: 'ssh',
      connectParams: null,
      history: ['/'],
      historyIndex: 0,
    }
    pane.tabs.push(tab)
    pane.activeTabId = tab.id
    activePaneId.value = pane.id
    return tab
  }

  function closeTab(tabId: string) {
    const pane = paneOf(tabId)
    if (!pane) return
    const index = pane.tabs.findIndex((t) => t.id === tabId)
    if (index === -1) return
    pane.tabs.splice(index, 1)
    if (pane.tabs.length === 0) {
      if (split.value) closePane(pane.id)
      else addTabIn(pane.id)
      return
    }
    if (pane.activeTabId === tabId) {
      pane.activeTabId = pane.tabs[Math.max(0, index - 1)]!.id
    }
  }

  function setActiveTab(tabId: string) {
    const pane = paneOf(tabId)
    if (!pane) return
    pane.activeTabId = tabId
    activePaneId.value = pane.id
  }

  function setActivePane(id: string) {
    if (paneById(id)) activePaneId.value = id
  }

  function focusOtherPane() {
    if (!split.value) return
    const index = panes.value.findIndex((p) => p.id === activePaneId.value)
    activePaneId.value = panes.value[(index + 1) % panes.value.length]!.id
  }

  function closePane(id: string) {
    if (!split.value) return
    const closing = paneById(id)
    const keep = panes.value.find((p) => p.id !== id)
    if (!closing || !keep) return
    keep.tabs.push(...closing.tabs)
    panes.value = panes.value.filter((p) => p.id !== id)
    if (!paneById(activePaneId.value)) activePaneId.value = keep.id
  }

  function toggleSplit() {
    if (split.value) {
      closePane(panes.value.find((p) => p.id !== activePaneId.value)!.id)
      return
    }
    const source = ensurePane()
    const currentTab = findTab(source.activeTabId)
    const tab: Tab = {
      id: crypto.randomUUID(),
      sessionId: null,
      label: 'New Connection',
      status: 'disconnected',
      currentPath: currentTab?.currentPath ?? '/',
      kind: currentTab?.kind ?? 'ssh',
      connectParams: null,
      history: ['/'],
      historyIndex: 0,
    }
    const pane: Pane = { id: crypto.randomUUID(), tabs: [tab], activeTabId: tab.id }
    panes.value.push(pane)
    activePaneId.value = pane.id
  }

  function updateTab(tabId: string, updates: Partial<Tab>) {
    const tab = findTab(tabId)
    if (!tab) return
    if (updates.sessionId !== undefined && updates.sessionId !== tab.sessionId) {
      const start = updates.currentPath ?? tab.currentPath
      tab.history = [start]
      tab.historyIndex = 0
    } else if (
      updates.currentPath !== undefined &&
      updates.currentPath !== tab.currentPath
    ) {
      tab.history = tab.history.slice(0, tab.historyIndex + 1)
      tab.history.push(updates.currentPath)
      tab.historyIndex = tab.history.length - 1
    }
    Object.assign(tab, updates)
  }

  function goBack() {
    const tab = activeTab.value
    if (!tab || tab.historyIndex <= 0) return
    tab.historyIndex--
    tab.currentPath = tab.history[tab.historyIndex] ?? '/'
  }

  function goForward() {
    const tab = activeTab.value
    if (!tab || tab.historyIndex >= tab.history.length - 1) return
    tab.historyIndex++
    tab.currentPath = tab.history[tab.historyIndex] ?? '/'
  }

  return {
    panes,
    activePaneId,
    activePane,
    split,
    tabs,
    activeTabId,
    activeTab,
    canGoBack,
    canGoForward,
    addTab,
    addTabIn,
    closeTab,
    setActiveTab,
    setActivePane,
    focusOtherPane,
    toggleSplit,
    updateTab,
    goBack,
    goForward,
  }
})
