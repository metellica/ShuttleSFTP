// Mounts the full App (tabs + toolbar + dialogs) against the mock backend.
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from '@/App.vue'
import { useTabsStore } from '@/stores/tabs'

const app = createApp(App)
const pinia = createPinia()
app.use(pinia)

// Expose the store for the test driver (before mount so it survives mount errors)
;(window as unknown as Record<string, unknown>).__tabs = useTabsStore(pinia)

try {
  app.mount('#app')
} catch (e) {
  console.error('[mock] mount failed:', e)
}
