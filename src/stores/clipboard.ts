import { defineStore } from 'pinia'
import { ref } from 'vue'

/** Files marked for copy, pasteable into any session's directory. */
export const useClipboardStore = defineStore('clipboard', () => {
  const sessionId = ref<string | null>(null)
  const paths = ref<string[]>([])
  /** Label of the tab the files were copied from, for menu display. */
  const sourceLabel = ref('')

  function set(fromSessionId: string, filePaths: string[], label: string) {
    sessionId.value = fromSessionId
    paths.value = filePaths
    sourceLabel.value = label
  }

  function clear() {
    sessionId.value = null
    paths.value = []
    sourceLabel.value = ''
  }

  return { sessionId, paths, sourceLabel, set, clear }
})
