import { createApp } from 'vue'
import { createPinia } from 'pinia'
import RemotePanel from '@/components/browser/RemotePanel.vue'
import { useTabsStore } from '@/stores/tabs'

const app = createApp(RemotePanel)
app.use(createPinia())

const tabs = useTabsStore()
const tab = tabs.addTab()
tabs.updateTab(tab.id, {
  sessionId: 'mock-session',
  label: 'mock',
  status: 'connected',
  currentPath: '/',
})

app.mount('#app')

// Expose store for the test driver
;(window as unknown as Record<string, unknown>).__tabs = tabs
