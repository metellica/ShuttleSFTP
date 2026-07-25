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
    if (tab) Object.assign(tab, updates)
  }

  return { tabs, activeTabId, activeTab, addTab, closeTab, setActiveTab, updateTab }
})
